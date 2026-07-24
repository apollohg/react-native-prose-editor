package com.apollohg.editor

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
        val payload = synchronized(lock) {
            if (!transports.containsKey(editorId) || transportTokens[editorId] !== token) return
            val current = eventSequences[editorId] ?: 0uL
            if (current == ULong.MAX_VALUE) return
            val next = current + 1uL
            eventSequences[editorId] = next
            val base = mutableMapOf<String, Any?>(
                "editorId" to editorId,
                "eventSequence" to next.toString(),
            )
            when (event) {
                is AndroidCollaborationTransportEvent.Directive -> {
                    val state = state(editorId) ?: return
                    base["kind"] = "state"
                    base["generation"] = event.generation
                    base["state"] = state
                    base["peers"] = peers(editorId)
                    base["diagnostics"] = mapOf(
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
                    base["kind"] = "error"
                    base["generation"] = event.generation
                    base["error"] = event.error.toJSMap()
                }
            }
            base
        }
        synchronized(lock) { eventEmitter }?.invoke(payload)
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
            !setOf("url", "connect", "connectionInit").containsAll(keys)
        ) return null
        val url = value.opt("url") as? String ?: return null
        val connect = value.opt("connect") as? Boolean ?: return null
        val connectionInitJwt = when (val rawConnectionInit = value.opt("connectionInit")) {
            null -> null
            is JSONObject -> {
                if (rawConnectionInit.keys().asSequence().toSet() != setOf("jwt")) return null
                rawConnectionInit.opt("jwt") as? String ?: return null
            }
            else -> return null
        }
        return NativeCollaborationTransportConfig.parse(url, connect, connectionInitJwt)
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
