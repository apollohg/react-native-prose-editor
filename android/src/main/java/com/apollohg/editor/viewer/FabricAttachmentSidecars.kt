package com.apollohg.editor.viewer

/**
 * Yoga prepares Fabric artifacts before a View exists. Keep its publication
 * sidecar by surface so a new semantic identity is cleared before intrinsic
 * fallback lookup, then mount binds the same state without a second reset.
 */
internal object FabricAttachmentSidecars {
    private val lock = Any()
    private val states = mutableMapOf<FabricSurfaceToken, ViewerAttachmentRevisionState>()
    private val measurementState = ThreadLocal<ViewerAttachmentRevisionState>()

    /** Only preparation's explicitly bound owner may satisfy an LRU miss. */
    val currentMeasurementState: ViewerAttachmentRevisionState?
        get() = measurementState.get()

    fun begin(surface: FabricSurfaceToken, semanticIdentity: String): ViewerAttachmentRevisionState = synchronized(lock) {
        states.getOrPut(surface, ::ViewerAttachmentRevisionState).also {
            it.beginSemanticGeneration(semanticIdentity)
        }
    }

    fun state(surface: FabricSurfaceToken): ViewerAttachmentRevisionState? = synchronized(lock) { states[surface] }

    /** Nestable and exception-safe: Yoga workers may reuse the same thread. */
    fun <T> withMeasurementState(state: ViewerAttachmentRevisionState, body: () -> T): T {
        val previous = measurementState.get()
        measurementState.set(state)
        return try {
            body()
        } finally {
            if (previous == null) measurementState.remove() else measurementState.set(previous)
        }
    }

    fun remove(surface: FabricSurfaceToken) = synchronized(lock) { states.remove(surface)?.reset() }

    fun removeSurface(surfaceId: Int) = synchronized(lock) {
        states.keys.filter { it.surfaceId == surfaceId }.forEach { token -> states.remove(token)?.reset() }
    }
}
