package com.apollohg.editor

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/**
 * The v2 pairing registry keeps the public editor handle as its canonical
 * decimal string. A separate, opaque view token exists only for the Android
 * widget lifecycle; it never crosses the JS/native or native/Rust boundary.
 */
internal object EditorV2Registry {
    private data class Pairing(
        val handle: String,
        val viewToken: Long,
        val adapter: EditorV2Adapter,
    )

    private val pairingsByHandle = ConcurrentHashMap<String, Pairing>()
    private val pairingsByViewToken = ConcurrentHashMap<Long, Pairing>()
    private val nextViewToken = AtomicLong(0)

    fun register(adapter: EditorV2Adapter): Long {
        val token = nextViewToken.incrementAndGet()
        check(token > 0L) { "native editor view token overflow" }
        val pairing = Pairing(adapter.editorId, token, adapter)
        pairingsByHandle[adapter.editorId] = pairing
        pairingsByViewToken[token] = pairing
        return token
    }

    fun viewTokenForHandle(handle: String): Long? = pairingsByHandle[handle]?.viewToken

    fun adapterForViewToken(viewToken: Long): EditorV2Adapter? = pairingsByViewToken[viewToken]?.adapter

    fun handleForViewToken(viewToken: Long): String? = pairingsByViewToken[viewToken]?.handle

    /** Cancel view-owned callbacks without releasing the pairing itself. */
    fun cancelAutonomousErrorOwner(handle: String) {
        pairingsByHandle[handle]?.adapter?.releaseAutonomousErrorOwner()
    }

    fun remove(handle: String): EditorV2Adapter? {
        val pairing = pairingsByHandle.remove(handle) ?: return null
        pairingsByViewToken.remove(pairing.viewToken)
        pairing.adapter.releaseAutonomousErrorOwner()
        return pairing.adapter
    }

    /** Drop a pairing after the public v2 destroy entry has handled its session. */
    fun dropPair(handle: String) {
        remove(handle)
    }
}
