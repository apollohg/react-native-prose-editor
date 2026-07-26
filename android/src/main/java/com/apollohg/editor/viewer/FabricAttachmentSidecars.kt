package com.apollohg.editor.viewer

/**
 * Yoga prepares Fabric artifacts before a View exists. Keep its publication
 * sidecar by surface so a new semantic identity is cleared before intrinsic
 * fallback lookup, then mount binds the same state without a second reset.
 */
internal object FabricAttachmentSidecars {
    private val lock = Any()
    private val states = mutableMapOf<FabricSurfaceToken, ViewerAttachmentRevisionState>()

    fun begin(surface: FabricSurfaceToken, semanticIdentity: String): ViewerAttachmentRevisionState = synchronized(lock) {
        states.getOrPut(surface, ::ViewerAttachmentRevisionState).also {
            it.beginSemanticGeneration(semanticIdentity)
        }
    }

    fun state(surface: FabricSurfaceToken): ViewerAttachmentRevisionState? = synchronized(lock) { states[surface] }

    fun remove(surface: FabricSurfaceToken) = synchronized(lock) { states.remove(surface)?.reset() }

    fun removeSurface(surfaceId: Int) = synchronized(lock) {
        states.keys.filter { it.surfaceId == surfaceId }.forEach { token -> states.remove(token)?.reset() }
    }
}
