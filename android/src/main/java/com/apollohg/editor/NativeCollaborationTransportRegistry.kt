package com.apollohg.editor

import android.util.Base64
import org.json.JSONObject

internal object NativeCollaborationTransportRegistry {
    private val lock = Any()
    private val transports = mutableMapOf<String, AndroidCollaborationTransport>()
    private val transportTokens = mutableMapOf<String, Any>()
    private val eventSequences = mutableMapOf<String, ULong>()
    private var eventEmitter: ((Map<String, Any?>) -> Unit)? = null

    fun setEventEmitter(emitter: ((Map<String, Any?>) -> Unit)?) {
        synchronized(lock) {
            eventEmitter = emitter
        }
    }

    fun configure(editorId: String, configJson: String?): EditorV2Error? {
        val canonical = canonicalV2U64(editorId)?.takeIf { it != "0" }
            ?: return contractError("invalid editorId")
        val config = if (configJson == null) {
            null
        } else {
            parseConfig(configJson) ?: return contractError(
                "invalid collaboration transport configuration",
            )
        }

        if (config == null) {
            val removed = synchronized(lock) {
                transportTokens.remove(canonical)
                transports.remove(canonical)
            }
            removed?.destroy()
            return null
        }
        val (transport, created) = synchronized(lock) {
            val created = !transports.containsKey(canonical)
            val transport = transports.getOrPut(canonical) {
                val token = Any()
                transportTokens[canonical] = token
                AndroidCollaborationTransport(
                    editorId = canonical,
                    eventSink = { event -> enqueueEvent(canonical, token, event) },
                )
            }
            transport to created
        }
        val error = transport.configure(config)
        if (error != null && created) {
            val removed = synchronized(lock) {
                if (transports[canonical] === transport) {
                    transportTokens.remove(canonical)
                    transports.remove(canonical)
                } else {
                    null
                }
            }
            if (removed != null) {
                transport.destroy()
            }
        }
        return error
    }

    fun notifyOutboundAvailable(editorId: String, reason: CollaborationWakeReason) {
        val canonical = canonicalV2U64(editorId) ?: return
        synchronized(lock) {
            transports[canonical]
        }?.notifyOutboundAvailable(reason)
    }

    fun resolveProtocolAdapter(
        editorId: String,
        attemptId: String,
        eventId: String,
        responseJson: String,
    ): EditorV2Error? {
        val canonical = canonicalV2U64(editorId)?.takeIf { it != "0" }
            ?: return contractError("invalid editorId")
        val response = parseProtocolAdapterResponse(responseJson)
            ?: return contractError("invalid collaboration protocol adapter response")
        if (attemptId.isEmpty() || canonicalV2U64(eventId) == null) {
            return contractError("invalid collaboration protocol adapter response")
        }
        val transport = synchronized(lock) { transports[canonical] } ?: return null
        return transport.resolveProtocolAdapter(attemptId, eventId, response)
    }

    fun destroy(editorId: String) {
        val canonical = canonicalV2U64(editorId) ?: return
        synchronized(lock) {
            transportTokens.remove(canonical)
            transports.remove(canonical)
        }?.destroy()
        synchronized(lock) {
            eventSequences.remove(canonical)
        }
    }

    fun enterBackground() {
        val owned = synchronized(lock) { transports.values.toList() }
        owned.forEach(AndroidCollaborationTransport::enterBackground)
    }

    fun enterForeground() {
        val owned = synchronized(lock) { transports.values.toList() }
        owned.forEach(AndroidCollaborationTransport::enterForeground)
    }

    fun destroyAll() {
        val owned = synchronized(lock) {
            val values = transports.values.toList()
            transports.clear()
            transportTokens.clear()
            eventSequences.clear()
            eventEmitter = null
            values
        }
        owned.forEach(AndroidCollaborationTransport::destroy)
    }

    private fun enqueueEvent(
        editorId: String,
        token: Any,
        event: AndroidCollaborationTransportEvent,
    ) {
        val sequence = synchronized(lock) {
            if (!transports.containsKey(editorId) || transportTokens[editorId] !== token) return
            val current = eventSequences[editorId] ?: 0uL
            if (current == ULong.MAX_VALUE) return
            val next = current + 1uL
            eventSequences[editorId] = next
            next
        }
        val payload = mutableMapOf<String, Any?>(
            "editorId" to editorId,
            "eventSequence" to sequence.toString(),
        )
        when (event) {
            is AndroidCollaborationTransportEvent.Directive -> {
                val state = state(editorId) ?: return
                payload["kind"] = "state"
                payload["generation"] = event.generation
                payload["state"] = state
                payload["peers"] = peers(editorId)
                payload["diagnostics"] = mapOf(
                    "wakeReason" to event.wakeReason.wireValue,
                    "transportState" to event.directive.transportState,
                    "nextDeadlineMillis" to event.directive.nextDeadlineMillis,
                    "remoteCommitApplied" to event.directive.remoteCommitApplied,
                    "peersChanged" to event.directive.peersChanged,
                    "renewedLocal" to event.directive.renewedLocal,
                    "expiredPeerCount" to event.directive.expiredPeers.size,
                )
            }
            is AndroidCollaborationTransportEvent.Error -> {
                payload["kind"] = "error"
                payload["generation"] = event.generation
                payload["error"] = event.error.toJSMap()
            }
            is AndroidCollaborationTransportEvent.ProtocolAdapter -> {
                val adapterEvent = event.event
                payload["kind"] = "protocolAdapter"
                payload["generation"] = adapterEvent.generation
                payload["attemptId"] = adapterEvent.attemptId
                payload["eventId"] = adapterEvent.eventId
                payload["negotiatedProtocol"] = adapterEvent.negotiatedProtocol
                when (val phase = adapterEvent.phase) {
                    NativeCollaborationProtocolAdapterPhase.Open -> {
                        payload["phase"] = "open"
                    }
                    is NativeCollaborationProtocolAdapterPhase.Message -> {
                        payload["phase"] = "message"
                        payload["frame"] = when (val frame = phase.frame) {
                            is NativeCollaborationProtocolFrame.Text ->
                                mapOf("type" to "text", "data" to frame.data)
                            is NativeCollaborationProtocolFrame.Binary ->
                                mapOf(
                                    "type" to "binary",
                                    "data" to Base64.encodeToString(frame.data, Base64.NO_WRAP),
                                )
                        }
                    }
                }
            }
        }
        val emitter = synchronized(lock) {
            if (!transports.containsKey(editorId) || transportTokens[editorId] !== token) return
            eventEmitter
        }
        if (event is AndroidCollaborationTransportEvent.Directive &&
            event.directive.remoteCommitApplied
        ) {
            NativeEditorViewRegistry.rebaseAfterRemoteCommit(editorId)
        }
        emitter?.invoke(payload)
    }

    private fun state(editorId: String): Map<String, Any?>? {
        val result = UniffiEditorV2Backend.getState(editorId)
        if (result !is EditorV2CallResult.Ok) return null
        return runCatching { JSONObject(result.value).toMap() }.getOrNull()
    }

    private fun peers(editorId: String): Any {
        val result = UniffiEditorV2Backend.collaborationPeers(editorId)
        if (result !is EditorV2CallResult.Ok) return emptyList<Any>()
        return runCatching {
            val objectValue = JSONObject(result.value)
            val peers = objectValue.getJSONArray("peers")
            List(peers.length()) { index ->
                val value = peers.get(index)
                if (value is JSONObject) value.toMap() else value
            }
        }.getOrDefault(emptyList<Any>())
    }

    private fun parseConfig(json: String): NativeCollaborationTransportConfig? {
        if (json.toByteArray(Charsets.UTF_8).size > 32_768) return null
        val value = runCatching { JSONObject(json) }.getOrNull() ?: return null
        val keys = value.keys().asSequence().toSet()
        if (
            !keys.containsAll(setOf("url", "connect")) ||
            !setOf("url", "connect", "protocolAdapter").containsAll(keys)
        ) return null
        val url = value.opt("url") as? String ?: return null
        val connect = value.opt("connect") as? Boolean ?: return null
        val protocolAdapter = when (val rawAdapter = value.opt("protocolAdapter")) {
            null -> null
            is JSONObject -> parseProtocolAdapterConfig(rawAdapter) ?: return null
            else -> return null
        }
        return NativeCollaborationTransportConfig.parse(url, connect, protocolAdapter)
    }

    private fun parseProtocolAdapterConfig(
        value: JSONObject,
    ): NativeCollaborationProtocolAdapterConfig? {
        val keys = value.keys().asSequence().toSet()
        if (
            !keys.contains("protocols") ||
            !setOf("protocols", "timeoutMillis", "terminalCloseCodes").containsAll(keys)
        ) return null
        val rawProtocols = value.optJSONArray("protocols") ?: return null
        if (rawProtocols.length() !in 1..16) return null
        val protocols = List(rawProtocols.length()) { index ->
            rawProtocols.opt(index) as? String ?: return null
        }
        if (protocols.toSet().size != protocols.size || protocols.any { !validWebSocketProtocol(it) }) {
            return null
        }
        val timeoutMillis = when (val rawTimeout = value.opt("timeoutMillis")) {
            null -> 10_000L
            is Number -> {
                val asDouble = rawTimeout.toDouble()
                if (
                    !asDouble.isFinite() ||
                    asDouble % 1.0 != 0.0 ||
                    asDouble < 1.0 ||
                    asDouble > 60_000.0
                ) return null
                asDouble.toLong()
            }
            else -> return null
        }
        val terminalCloseCodes = when (val rawCodes = value.optJSONArray("terminalCloseCodes")) {
            null -> emptySet()
            else -> buildSet {
                for (index in 0 until rawCodes.length()) {
                    val rawCode = rawCodes.opt(index) as? Number ?: return null
                    val asDouble = rawCode.toDouble()
                    if (
                        !asDouble.isFinite() ||
                        asDouble % 1.0 != 0.0 ||
                        asDouble < 1_000.0 ||
                        asDouble > 4_999.0 ||
                        !add(asDouble.toInt())
                    ) return null
                }
            }
        }
        return NativeCollaborationProtocolAdapterConfig(
            protocols = protocols,
            timeoutMillis = timeoutMillis,
            terminalCloseCodes = terminalCloseCodes,
        )
    }

    private fun validWebSocketProtocol(value: String): Boolean {
        if (value.toByteArray(Charsets.UTF_8).size !in 1..128) return false
        val allowed = "!#$%&'*+-.^_`|~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
            .toSet()
        return value.all { it in allowed }
    }

    private fun parseProtocolAdapterResponse(
        json: String,
    ): NativeCollaborationProtocolAdapterResponse? {
        if (json.toByteArray(Charsets.UTF_8).size > 1_500_000) return null
        val value = runCatching { JSONObject(json) }.getOrNull() ?: return null
        val keys = value.keys().asSequence().toSet()
        if (!keys.contains("action") || !setOf("action", "frames").containsAll(keys)) return null
        val action = when (value.opt("action") as? String) {
            "continue" -> NativeCollaborationProtocolAdapterAction.CONTINUE
            "ready" -> NativeCollaborationProtocolAdapterAction.READY
            "reject" -> NativeCollaborationProtocolAdapterAction.REJECT
            else -> return null
        }
        val rawFrames = value.optJSONArray("frames")
        if (rawFrames != null && rawFrames.length() > 16) return null
        val frames = buildList {
            if (rawFrames != null) {
                for (index in 0 until rawFrames.length()) {
                    val frame = rawFrames.optJSONObject(index) ?: return null
                    if (frame.keys().asSequence().toSet() != setOf("type", "data")) return null
                    val data = frame.opt("data") as? String ?: return null
                    when (frame.opt("type") as? String) {
                        "text" -> {
                            if (
                                data.toByteArray(Charsets.UTF_8).size >
                                NativeCollaborationProtocolAdapterConfig.MAXIMUM_FRAME_BYTES
                            ) return null
                            add(NativeCollaborationProtocolFrame.Text(data))
                        }
                        "binary" -> {
                            val decoded = runCatching {
                                Base64.decode(data, Base64.NO_WRAP)
                            }.getOrNull() ?: return null
                            if (
                                decoded.size >
                                NativeCollaborationProtocolAdapterConfig.MAXIMUM_FRAME_BYTES
                            ) return null
                            add(NativeCollaborationProtocolFrame.Binary(decoded))
                        }
                        else -> return null
                    }
                }
            }
        }
        return NativeCollaborationProtocolAdapterResponse(action, frames)
    }

    private fun JSONObject.toMap(): Map<String, Any?> =
        keys().asSequence().associateWith { key ->
            when (val value = get(key)) {
                JSONObject.NULL -> null
                is JSONObject -> value.toMap()
                else -> value
            }
        }

    private fun contractError(message: String) =
        EditorV2Error("boundary", "FFI_RESULT_INVALID", message)
}
