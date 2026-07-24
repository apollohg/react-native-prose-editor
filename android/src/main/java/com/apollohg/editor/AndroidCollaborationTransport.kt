package com.apollohg.editor

import android.os.SystemClock
import okhttp3.Request
import okio.ByteString.Companion.toByteString
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.util.ArrayDeque
import java.util.concurrent.Future
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.ScheduledThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

internal data class NativeCollaborationTransportConfig(
    val url: String,
    val connect: Boolean,
    val protocolAdapter: NativeCollaborationProtocolAdapterConfig?,
    val diagnosticEndpoint: String,
) {
    companion object {
        const val MAXIMUM_URL_BYTES = 4_096

        fun parse(
            url: String,
            connect: Boolean,
            protocolAdapter: NativeCollaborationProtocolAdapterConfig?,
        ): NativeCollaborationTransportConfig? {
            if (url.toByteArray(Charsets.UTF_8).size !in 1..MAXIMUM_URL_BYTES) return null
            val uri = runCatching { URI(url) }.getOrNull() ?: return null
            if (uri.scheme?.lowercase() !in setOf("ws", "wss")) return null
            if (uri.host.isNullOrEmpty() || uri.rawUserInfo != null || uri.rawFragment != null) return null
            if (runCatching { Request.Builder().url(url).build() }.isFailure) return null
            val endpoint = runCatching {
                URI(uri.scheme.lowercase(), null, uri.host, uri.port, uri.rawPath, null, null)
                    .toASCIIString()
            }.getOrNull() ?: return null
            return NativeCollaborationTransportConfig(url, connect, protocolAdapter, endpoint)
        }
    }
}

internal data class NativeCollaborationProtocolAdapterConfig(
    val protocols: List<String>,
    val timeoutMillis: Long,
    val terminalCloseCodes: Set<Int>,
) {
    companion object {
        const val MAXIMUM_FRAME_BYTES = 64 * 1_024
        const val MAXIMUM_PRE_READY_FRAMES = 16
    }
}

internal sealed interface NativeCollaborationProtocolFrame {
    data class Text(val data: String) : NativeCollaborationProtocolFrame
    data class Binary(val data: ByteArray) : NativeCollaborationProtocolFrame
}

internal enum class NativeCollaborationProtocolAdapterAction {
    CONTINUE,
    READY,
    REJECT,
}

internal data class NativeCollaborationProtocolAdapterResponse(
    val action: NativeCollaborationProtocolAdapterAction,
    val frames: List<NativeCollaborationProtocolFrame>,
)

internal sealed interface NativeCollaborationProtocolAdapterPhase {
    object Open : NativeCollaborationProtocolAdapterPhase
    data class Message(val frame: NativeCollaborationProtocolFrame) :
        NativeCollaborationProtocolAdapterPhase
}

internal data class NativeCollaborationProtocolAdapterEvent(
    val attemptId: String,
    val eventId: String,
    val generation: String,
    val negotiatedProtocol: String?,
    val phase: NativeCollaborationProtocolAdapterPhase,
)

internal enum class CollaborationWakeReason(val wireValue: String) {
    LOCAL_MUTATION("localMutation"),
    MODULE_MUTATION("moduleMutation"),
    RECEIVE("receive"),
    TIMER("timer"),
    OPEN("open"),
    REATTACH("reattach"),
    AWARENESS("awareness"),
}

internal data class AndroidCollaborationDirective(
    val transportState: String,
    val generationToOpen: String?,
    val nextDeadlineMillis: String?,
    val remoteCommitApplied: Boolean,
    val peersChanged: Boolean,
    val renewedLocal: Boolean,
    val expiredPeers: List<String>,
)

internal sealed interface AndroidCollaborationTransportEvent {
    data class Directive(
        val directive: AndroidCollaborationDirective,
        val generation: String?,
        val wakeReason: CollaborationWakeReason,
    ) : AndroidCollaborationTransportEvent

    data class Error(
        val error: EditorV2Error,
        val generation: String?,
    ) : AndroidCollaborationTransportEvent

    data class ProtocolAdapter(
        val event: NativeCollaborationProtocolAdapterEvent,
    ) : AndroidCollaborationTransportEvent
}

internal fun interface CollaborationMonotonicClock {
    fun nowMillis(): Long
}

internal class AndroidCollaborationTransport(
    private val editorId: String,
    private val backend: EditorV2Backend = UniffiEditorV2Backend,
    private val socketFactory: CollaborationSocketFactory = OkHttpCollaborationSocketFactory(),
    private val clock: CollaborationMonotonicClock =
        CollaborationMonotonicClock(SystemClock::elapsedRealtime),
    private val eventSink: (AndroidCollaborationTransportEvent) -> Unit = {},
) {
    private val workerThread = AtomicReference<Thread?>()
    private val executor = ScheduledThreadPoolExecutor(1) { runnable ->
        Thread(runnable, "native-editor-collaboration-$editorId").also {
            it.isDaemon = true
            workerThread.set(it)
        }
    }.apply {
        removeOnCancelPolicy = true
    }

    private var config: NativeCollaborationTransportConfig? = null
    private var socket: CollaborationSocket? = null
    private var generation: String? = null
    private var socketToken = 0L
    private var deadline: ScheduledFuture<*>? = null
    private var protocolAdapterDeadline: ScheduledFuture<*>? = null
    private var inFlightLease: EditorV2OutboundLease? = null
    private var networkSocketOpened = false
    private var socketOpened = false
    private var protocolAttemptId: String? = null
    private var protocolEventSequence = 0uL
    private var pendingProtocolEventId: String? = null
    private var negotiatedProtocol: String? = null
    private val bufferedProtocolFrames = ArrayDeque<NativeCollaborationProtocolFrame>()
    private var closeReported = false
    private var backgrounded = false
    private var destroyed = false

    fun configure(newConfig: NativeCollaborationTransportConfig?): EditorV2Error? = onWorker {
        if (destroyed) return@onWorker lifecycleError("collaboration transport is destroyed")
        if (config == newConfig) {
            if (newConfig?.connect == true && !backgrounded) drive(CollaborationWakeReason.REATTACH)
            return@onWorker null
        }
        retireNativeResources()
        backend.collaborationDetach(editorId)?.let {
            emit(it)
            return@onWorker it
        }
        config = newConfig
        if (newConfig?.connect != true || backgrounded) return@onWorker null
        backend.collaborationReattach(editorId)?.let {
            emit(it)
            return@onWorker it
        }
        drive(CollaborationWakeReason.REATTACH)
        null
    }

    fun notifyOutboundAvailable(reason: CollaborationWakeReason) {
        enqueue {
            if (canDrive()) drive(reason)
        }
    }

    fun resolveProtocolAdapter(
        attemptId: String,
        eventId: String,
        response: NativeCollaborationProtocolAdapterResponse,
    ): EditorV2Error? = onWorker {
        if (destroyed) return@onWorker lifecycleError("collaboration transport is destroyed")
        val activeGeneration = generation
        val activeSocket = socket
        if (
            protocolAttemptId != attemptId ||
            pendingProtocolEventId != eventId ||
            activeGeneration == null ||
            activeSocket == null
        ) {
            return@onWorker null
        }
        pendingProtocolEventId = null
        if (!sendProtocolAdapterFrames(activeSocket, response.frames)) {
            failCurrentSocket(socketToken, activeGeneration, null)
            return@onWorker null
        }
        when (response.action) {
            NativeCollaborationProtocolAdapterAction.CONTINUE ->
                emitNextBufferedProtocolFrame(socketToken, activeGeneration)
            NativeCollaborationProtocolAdapterAction.READY -> {
                activateYjs(socketToken, activeGeneration)
                drainBufferedFramesAfterReady(socketToken, activeGeneration)
            }
            NativeCollaborationProtocolAdapterAction.REJECT ->
                failCurrentSocket(socketToken, activeGeneration, 1008)
        }
        null
    }

    fun enterBackground() = onWorker {
        if (!destroyed && !backgrounded) {
            backgrounded = true
            retireNativeResources()
            backend.collaborationDetach(editorId)?.let(::emit)
        }
    }

    fun enterForeground() = onWorker {
        if (!destroyed && backgrounded) {
            backgrounded = false
            if (config?.connect == true) {
                val error = backend.collaborationReattach(editorId)
                if (error != null) emit(error) else drive(CollaborationWakeReason.REATTACH)
            }
        }
    }

    fun destroy() {
        onWorker {
            if (!destroyed) {
                destroyed = true
                retireNativeResources()
                backend.collaborationDetach(editorId)?.let(::emit)
                config = null
            }
        }
        executor.shutdownNow()
    }

    private fun canDrive(): Boolean = !destroyed && !backgrounded && config?.connect == true

    private fun drive(reason: CollaborationWakeReason) {
        if (!canDrive()) return
        consumeDirective(
            backend.collaborationDrive(editorId, clock.nowMillis().toString()),
            generation,
            reason,
        )
    }

    private fun consumeDirective(
        result: EditorV2CallResult<String>,
        eventGeneration: String?,
        reason: CollaborationWakeReason,
    ): Boolean = when (result) {
        is EditorV2CallResult.Err -> {
            emit(result.error, eventGeneration)
            false
        }
        is EditorV2CallResult.Ok -> {
            val directive = parseDirective(result.value)
            if (directive == null) {
                emit(contractError("collaboration directive violates the frozen shape"), eventGeneration)
                false
            } else {
                eventSink(AndroidCollaborationTransportEvent.Directive(directive, eventGeneration, reason))
                scheduleDeadline(directive.nextDeadlineMillis)
                if (directive.generationToOpen != null) {
                    openSocket(directive.generationToOpen)
                } else {
                    driveOutboundIfPossible()
                }
                true
            }
        }
    }

    private fun openSocket(newGeneration: String) {
        val activeConfig = config ?: return
        if (!canDrive()) return
        retireNativeResources()
        generation = newGeneration
        closeReported = false
        networkSocketOpened = false
        socketOpened = false
        protocolAttemptId = if (activeConfig.protocolAdapter == null) null else java.util.UUID.randomUUID().toString()
        protocolEventSequence = 0uL
        pendingProtocolEventId = null
        negotiatedProtocol = null
        bufferedProtocolFrames.clear()
        val token = ++socketToken
        val newSocket = socketFactory.makeSocket(
            activeConfig.url,
            activeConfig.protocolAdapter?.protocols ?: emptyList(),
            CollaborationSocketCallbacks(
                onOpen = { selectedProtocol ->
                    enqueue { socketDidOpen(token, newGeneration, selectedProtocol) }
                },
                onBinaryMessage = { bytes ->
                    enqueue { socketDidReceive(token, newGeneration, bytes) }
                },
                onTextMessage = { text ->
                    enqueue { socketDidReceiveText(token, newGeneration, text) }
                },
                onClose = { code ->
                    enqueue { socketDidClose(token, newGeneration, code) }
                },
                onFailure = {
                    enqueue { failCurrentSocket(token, newGeneration, null) }
                },
            ),
        )
        socket = newSocket
        newSocket.connect()
    }

    private fun socketDidOpen(
        token: Long,
        callbackGeneration: String,
        selectedProtocol: String?,
    ) {
        if (!isCurrent(token, callbackGeneration) || networkSocketOpened) return
        networkSocketOpened = true
        negotiatedProtocol = selectedProtocol
        if (config?.protocolAdapter == null) {
            activateYjs(token, callbackGeneration)
            return
        }
        scheduleProtocolAdapterTimeout(token, callbackGeneration)
        emitProtocolAdapterEvent(
            NativeCollaborationProtocolAdapterPhase.Open,
            token,
            callbackGeneration,
        )
    }

    private fun activateYjs(token: Long, callbackGeneration: String) {
        if (
            !isCurrent(token, callbackGeneration) ||
            !networkSocketOpened ||
            socketOpened
        ) return
        protocolAdapterDeadline?.cancel(false)
        protocolAdapterDeadline = null
        pendingProtocolEventId = null
        socketOpened = true
        val accepted = consumeDirective(
            backend.collaborationSocketOpen(
                editorId,
                callbackGeneration,
                clock.nowMillis().toString(),
            ),
            callbackGeneration,
            CollaborationWakeReason.OPEN,
        )
        if (!accepted) failCurrentSocket(token, callbackGeneration, 1008)
    }

    private fun socketDidReceive(token: Long, callbackGeneration: String, bytes: ByteArray) {
        if (!isCurrent(token, callbackGeneration)) return
        if (!socketOpened) {
            receiveProtocolAdapterFrame(
                NativeCollaborationProtocolFrame.Binary(bytes),
                token,
                callbackGeneration,
            )
            return
        }
        val accepted = consumeDirective(
            backend.collaborationReceive(
                editorId,
                callbackGeneration,
                bytes,
                clock.nowMillis().toString(),
            ),
            callbackGeneration,
            CollaborationWakeReason.RECEIVE,
        )
        if (!accepted) failCurrentSocket(token, callbackGeneration, 1008)
    }

    private fun socketDidReceiveText(
        token: Long,
        callbackGeneration: String,
        text: String,
    ) {
        if (!isCurrent(token, callbackGeneration)) return
        if (!socketOpened) {
            receiveProtocolAdapterFrame(
                NativeCollaborationProtocolFrame.Text(text),
                token,
                callbackGeneration,
            )
        } else {
            failCurrentSocket(token, callbackGeneration, 1008)
        }
    }

    private fun driveOutboundIfPossible() {
        val activeGeneration = generation ?: return
        val activeSocket = socket ?: return
        if (!canDrive() || !socketOpened || inFlightLease != null) return

        when (val result = backend.collaborationLeaseOutbound(editorId, activeGeneration)) {
            EditorV2LeaseResult.Empty -> Unit
            is EditorV2LeaseResult.Err -> {
                emit(result.error, activeGeneration)
                failCurrentSocket(socketToken, activeGeneration, 1008)
            }
            is EditorV2LeaseResult.Value -> {
                val lease = result.lease
                inFlightLease = lease
                if (activeSocket.send(lease.frame.toByteString())) {
                    inFlightLease = null
                    when (
                        val ack = backend.collaborationAckOutbound(
                            editorId,
                            activeGeneration,
                            lease.leaseId,
                        )
                    ) {
                        is EditorV2CallResult.Err -> {
                            emit(ack.error, activeGeneration)
                            failCurrentSocket(socketToken, activeGeneration, 1008)
                        }
                        is EditorV2CallResult.Ok -> enqueue {
                            if (generation == activeGeneration) drive(CollaborationWakeReason.LOCAL_MUTATION)
                        }
                    }
                } else {
                    inFlightLease = null
                    when (
                        val nack = backend.collaborationNackOutbound(
                            editorId,
                            activeGeneration,
                            lease.leaseId,
                        )
                    ) {
                        is EditorV2CallResult.Err -> emit(nack.error, activeGeneration)
                        is EditorV2CallResult.Ok -> Unit
                    }
                    failCurrentSocket(socketToken, activeGeneration, null)
                }
            }
        }
    }

    private fun socketDidClose(token: Long, callbackGeneration: String, code: Int) {
        if (!isCurrent(token, callbackGeneration)) return
        val backendCode =
            if (config?.protocolAdapter?.terminalCloseCodes?.contains(code) == true) 1008 else code
        reportCurrentSocketClose(callbackGeneration, backendCode)
    }

    private fun failCurrentSocket(token: Long, callbackGeneration: String, code: Int?) {
        if (!isCurrent(token, callbackGeneration)) return
        socket?.cancel()
        reportCurrentSocketClose(callbackGeneration, code)
    }

    private fun reportCurrentSocketClose(callbackGeneration: String, code: Int?) {
        if (closeReported) return
        closeReported = true
        deadline?.cancel(false)
        deadline = null
        protocolAdapterDeadline?.cancel(false)
        protocolAdapterDeadline = null
        socketToken += 1
        socket = null
        networkSocketOpened = false
        socketOpened = false
        protocolAttemptId = null
        pendingProtocolEventId = null
        negotiatedProtocol = null
        bufferedProtocolFrames.clear()
        inFlightLease = null
        generation = null
        consumeDirective(
            backend.collaborationSocketClose(
                editorId,
                callbackGeneration,
                code?.takeIf { it >= 0 }?.toUInt(),
                null,
                clock.nowMillis().toString(),
            ),
            callbackGeneration,
            CollaborationWakeReason.TIMER,
        )
    }

    private fun scheduleDeadline(rawDeadline: String?) {
        deadline?.cancel(false)
        deadline = null
        val target = rawDeadline?.let(::canonicalULong) ?: return
        if (!canDrive()) return
        val now = clock.nowMillis().coerceAtLeast(0).toULong()
        val delay = if (target > now) target - now else 0uL
        val boundedDelay = minOf(delay, Long.MAX_VALUE.toULong()).toLong()
        deadline = executor.schedule(
            {
                deadline = null
                drive(CollaborationWakeReason.TIMER)
            },
            boundedDelay,
            TimeUnit.MILLISECONDS,
        )
    }

    private fun scheduleProtocolAdapterTimeout(token: Long, callbackGeneration: String) {
        protocolAdapterDeadline?.cancel(false)
        val timeoutMillis = config?.protocolAdapter?.timeoutMillis ?: return
        protocolAdapterDeadline = executor.schedule(
            {
                protocolAdapterDeadline = null
                failCurrentSocket(token, callbackGeneration, 1008)
            },
            timeoutMillis,
            TimeUnit.MILLISECONDS,
        )
    }

    private fun receiveProtocolAdapterFrame(
        frame: NativeCollaborationProtocolFrame,
        token: Long,
        callbackGeneration: String,
    ) {
        if (config?.protocolAdapter == null) {
            failCurrentSocket(token, callbackGeneration, 1008)
            return
        }
        val frameSize = when (frame) {
            is NativeCollaborationProtocolFrame.Text ->
                frame.data.toByteArray(Charsets.UTF_8).size
            is NativeCollaborationProtocolFrame.Binary -> frame.data.size
        }
        if (frameSize > NativeCollaborationProtocolAdapterConfig.MAXIMUM_FRAME_BYTES) {
            failCurrentSocket(token, callbackGeneration, 1008)
            return
        }
        if (pendingProtocolEventId != null) {
            if (
                bufferedProtocolFrames.size >=
                NativeCollaborationProtocolAdapterConfig.MAXIMUM_PRE_READY_FRAMES - 1
            ) {
                failCurrentSocket(token, callbackGeneration, 1008)
            } else {
                bufferedProtocolFrames.addLast(frame)
            }
            return
        }
        emitProtocolAdapterEvent(
            NativeCollaborationProtocolAdapterPhase.Message(frame),
            token,
            callbackGeneration,
        )
    }

    private fun emitProtocolAdapterEvent(
        phase: NativeCollaborationProtocolAdapterPhase,
        token: Long,
        callbackGeneration: String,
    ) {
        val attemptId = protocolAttemptId
        if (
            !isCurrent(token, callbackGeneration) ||
            config?.protocolAdapter == null ||
            attemptId == null ||
            pendingProtocolEventId != null ||
            protocolEventSequence == ULong.MAX_VALUE
        ) {
            failCurrentSocket(token, callbackGeneration, 1008)
            return
        }
        protocolEventSequence += 1uL
        val eventId = protocolEventSequence.toString()
        pendingProtocolEventId = eventId
        eventSink(
            AndroidCollaborationTransportEvent.ProtocolAdapter(
                NativeCollaborationProtocolAdapterEvent(
                    attemptId = attemptId,
                    eventId = eventId,
                    generation = callbackGeneration,
                    negotiatedProtocol = negotiatedProtocol,
                    phase = phase,
                ),
            ),
        )
    }

    private fun emitNextBufferedProtocolFrame(token: Long, callbackGeneration: String) {
        if (bufferedProtocolFrames.isEmpty()) return
        val next = bufferedProtocolFrames.removeFirst()
        emitProtocolAdapterEvent(
            NativeCollaborationProtocolAdapterPhase.Message(next),
            token,
            callbackGeneration,
        )
    }

    private fun drainBufferedFramesAfterReady(token: Long, callbackGeneration: String) {
        while (
            bufferedProtocolFrames.isNotEmpty() &&
            isCurrent(token, callbackGeneration) &&
            socketOpened
        ) {
            when (val frame = bufferedProtocolFrames.removeFirst()) {
                is NativeCollaborationProtocolFrame.Text -> {
                    failCurrentSocket(token, callbackGeneration, 1008)
                }
                is NativeCollaborationProtocolFrame.Binary -> {
                    socketDidReceive(token, callbackGeneration, frame.data)
                }
            }
        }
    }

    private fun sendProtocolAdapterFrames(
        activeSocket: CollaborationSocket,
        frames: List<NativeCollaborationProtocolFrame>,
    ): Boolean = frames.all { frame ->
        when (frame) {
            is NativeCollaborationProtocolFrame.Text -> activeSocket.send(frame.data)
            is NativeCollaborationProtocolFrame.Binary ->
                activeSocket.send(frame.data.toByteString())
        }
    }

    private fun retireNativeResources() {
        deadline?.cancel(false)
        deadline = null
        protocolAdapterDeadline?.cancel(false)
        protocolAdapterDeadline = null
        socketToken += 1
        socket?.cancel()
        socket = null
        generation = null
        inFlightLease = null
        networkSocketOpened = false
        socketOpened = false
        protocolAttemptId = null
        pendingProtocolEventId = null
        negotiatedProtocol = null
        bufferedProtocolFrames.clear()
        closeReported = true
    }

    private fun isCurrent(token: Long, callbackGeneration: String): Boolean =
        !destroyed && token == socketToken && generation == callbackGeneration

    private fun parseDirective(json: String): AndroidCollaborationDirective? {
        val objectValue = runCatching { JSONObject(json) }.getOrNull() ?: return null
        val generationToOpen = nullableCanonicalString(objectValue, "generationToOpen") ?: return null
        val nextDeadlineMillis = nullableCanonicalString(objectValue, "nextDeadlineMillis") ?: return null
        val expired = objectValue.optJSONArray("expiredPeers") ?: return null
        val expiredPeers = buildList {
            for (index in 0 until expired.length()) {
                val value = expired.opt(index) as? String ?: return null
                if (canonicalULong(value) == null) return null
                add(value)
            }
        }
        return runCatching {
            AndroidCollaborationDirective(
                transportState = objectValue.getString("transportState"),
                generationToOpen = generationToOpen.value,
                nextDeadlineMillis = nextDeadlineMillis.value,
                remoteCommitApplied = objectValue.getBoolean("remoteCommitApplied"),
                peersChanged = objectValue.getBoolean("peersChanged"),
                renewedLocal = objectValue.getBoolean("renewedLocal"),
                expiredPeers = expiredPeers,
            )
        }.getOrNull()
    }

    private data class NullableCanonicalString(val value: String?)

    private fun nullableCanonicalString(
        value: JSONObject,
        key: String,
    ): NullableCanonicalString? {
        if (!value.has(key)) return null
        if (value.isNull(key)) return NullableCanonicalString(null)
        val raw = value.opt(key) as? String ?: return null
        if (canonicalULong(raw) == null) return null
        return NullableCanonicalString(raw)
    }

    private fun canonicalULong(raw: String): ULong? {
        if (raw.isEmpty() || raw.any { it !in '0'..'9' }) return null
        if (raw.length > 1 && raw.first() == '0') return null
        return raw.toULongOrNull()
    }

    private fun contractError(message: String) =
        EditorV2Error("boundary", "FFI_RESULT_INVALID", message)

    private fun lifecycleError(message: String) =
        EditorV2Error("lifecycle", "ENGINE_DESTROYED", message)

    private fun emit(error: EditorV2Error, eventGeneration: String? = generation) {
        eventSink(AndroidCollaborationTransportEvent.Error(error, eventGeneration))
    }

    private fun enqueue(operation: () -> Unit) {
        if (executor.isShutdown) return
        runCatching { executor.execute(operation) }
    }

    private fun <T> onWorker(operation: () -> T): T {
        if (Thread.currentThread() === workerThread.get()) return operation()
        val future: Future<T> = executor.submit<T> { operation() }
        return future.get()
    }
}
