package com.apollohg.editor

import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/**
 * The v2 pairing registry: maps the public (module-visible) editor id to
 * the v2 adapter backing it. Views bound to a paired id route every
 * interaction through the adapter; an unpaired id has no engine traffic.
 */
internal object EditorV2Registry {
    private val pairings = ConcurrentHashMap<Long, EditorV2Adapter>()
    private val nextSyntheticId = AtomicLong(Long.MAX_VALUE)

    fun register(adapter: EditorV2Adapter, publicId: Long) {
        pairings[publicId] = adapter
    }

    fun adapterFor(publicId: Long): EditorV2Adapter? = pairings[publicId]

    fun registerSyntheticPairing(adapter: EditorV2Adapter): Long {
        val id = nextSyntheticId.getAndDecrement()
        pairings[id] = adapter
        return id
    }

    fun remove(publicId: Long): EditorV2Adapter? = pairings.remove(publicId)

    /** Destroy the v2 session backing a pairing and drop the pairing. */
    fun destroyPair(publicId: Long) {
        remove(publicId)?.destroy()
    }

    /**
     * Create-and-pair: one v2 session per editor; the public id IS the v2
     * session handle (decimal id as Long).
     */
    fun createPair(backend: EditorV2Backend, legacyConfigJson: String): EditorV2CallResult<Long> {
        return when (val created = EditorV2Adapter.create(backend, legacyConfigJson)) {
            is EditorV2CallResult.Err -> EditorV2CallResult.Err(created.error)
            is EditorV2CallResult.Ok -> {
                val publicId = created.value.editorId.toLongOrNull()
                if (publicId == null) {
                    created.value.destroy()
                    EditorV2CallResult.Err(
                        EditorV2Adapter.contractError("v2 editor handle is not a u64")
                    )
                } else {
                    register(created.value, publicId)
                    EditorV2CallResult.Ok(publicId)
                }
            }
        }
    }
}
