package com.apollohg.editor

import java.util.concurrent.ConcurrentHashMap

/**
 * The v2 pairing registry: maps the public (module-visible) editor id to
 * the v2 adapter backing it. Views bound to a paired id route every
 * interaction through the adapter; an unpaired id has no engine traffic.
 */
internal object EditorV2Registry {
    private val pairings = ConcurrentHashMap<Long, EditorV2Adapter>()

    fun register(adapter: EditorV2Adapter, publicId: Long) {
        pairings[publicId] = adapter
    }

    fun adapterFor(publicId: Long): EditorV2Adapter? = pairings[publicId]

    fun remove(publicId: Long): EditorV2Adapter? = pairings.remove(publicId)

    /** Destroy the v2 session backing a pairing and drop the pairing. */
    fun destroyPair(publicId: Long) {
        remove(publicId)?.destroy()
    }

}
