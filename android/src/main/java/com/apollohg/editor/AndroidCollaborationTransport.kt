package com.apollohg.editor

import android.os.SystemClock
import okhttp3.Request
import okio.ByteString.Companion.toByteString
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI
import java.util.concurrent.Future
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.ScheduledThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

internal data class NativeCollaborationTransportConfig(
    val url: String,
    val connect: Boolean,
    val connectionInitJwt: String?,
    val diagnosticEndpoint: String,
) {
    companion object {
        const val MAXIMUM_URL_BYTES = 4_096
        const val MAXIMUM_JWT_BYTES = 16_384

        fun parse(
            url: String,
            connect: Boolean,
            connectionInitJwt: String?,
        ): NativeCollaborationTransportConfig? {
            if (url.toByteArray(Charsets.UTF_8).size !in 1..MAXIMUM_URL_BYTES) return null
            if (
                connectionInitJwt != null &&
                (
                    connectionInitJwt.toByteArray(Charsets.UTF_8).size !in 1..MAXIMUM_JWT_BYTES ||
                        connectionInitJwt.any { it == '\u0000' || it == '\r' || it == '\n' }
                )
            ) return null
            val uri = runCatching { URI(url) }.getOrNull() ?: return null
            if (uri.scheme?.lowercase() !in setOf("ws", "wss")) return null
            if (uri.host.isNullOrEmpty() || uri.rawUserInfo != null || uri.rawFragment != null) return null
            if (runCatching { Request.Builder().url(url).build() }.isFailure) return null
            val endpoint = runCatching {
                URI(uri.scheme.lowercase(), null, uri.host, uri.port, uri.rawPath, null, null)
                    .toASCIIString()
            }.getOrNull() ?: return null
            return NativeCollaborationTransportConfig(url, connect, connectionInitJwt, endpoint)
        }
    }
}

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
    private var connectionAckDeadline: ScheduledFuture<*>? = null
    private var inFlightLease: EditorV2OutboundLease? = null
    private var networkSocketOpened = false
    private var socketOpened = false
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
        val token = ++socketToken
        val newSocket = socketFactory.makeSocket(
            activeConfig.url,
            CollaborationSocketCallbacks(
                onOpen = { enqueue { socketDidOpen(token, newGeneration) } },
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

    private fun socketDidOpen(token: Long, callbackGeneration: String) {
        if (!isCurrent(token, callbackGeneration) || networkSocketOpened) return
        networkSocketOpened = true
        val jwt = config?.connectionInitJwt
        if (jwt == null) {
            activateYjs(token, callbackGeneration)
            return
        }
        val message = JSONObject()
            .put("type", "connection_init")
            .put("payload", JSONObject().put("Authorization", "JWT $jwt"))
            .toString()
        if (socket?.send(message) != true) {
            failCurrentSocket(token, callbackGeneration, null)
            return
        }
        scheduleConnectionAckTimeout(token, callbackGeneration)
    }

    private fun activateYjs(token: Long, callbackGeneration: String) {
        if (
            !isCurrent(token, callbackGeneration) ||
            !networkSocketOpened ||
            socketOpened
        ) return
        connectionAckDeadline?.cancel(false)
        connectionAckDeadline = null
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
            failCurrentSocket(token, callbackGeneration, 1008)
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
        val acknowledged = !socketOpened &&
            text.toByteArray(Charsets.UTF_8).size <= 8_192 &&
            runCatching { JSONObject(text).optString("type") == "connection_ack" }
                .getOrDefault(false)
        if (acknowledged) {
            activateYjs(token, callbackGeneration)
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
        reportCurrentSocketClose(callbackGeneration, code)
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
        connectionAckDeadline?.cancel(false)
        connectionAckDeadline = null
        socketToken += 1
        socket = null
        networkSocketOpened = false
        socketOpened = false
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

    private fun scheduleConnectionAckTimeout(token: Long, callbackGeneration: String) {
        connectionAckDeadline?.cancel(false)
        connectionAckDeadline = executor.schedule(
            {
                connectionAckDeadline = null
                failCurrentSocket(token, callbackGeneration, 1008)
            },
            10,
            TimeUnit.SECONDS,
        )
    }

    private fun retireNativeResources() {
        deadline?.cancel(false)
        deadline = null
        connectionAckDeadline?.cancel(false)
        connectionAckDeadline = null
        socketToken += 1
        socket?.cancel()
        socket = null
        generation = null
        inFlightLease = null
        networkSocketOpened = false
        socketOpened = false
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
