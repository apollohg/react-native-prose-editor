package com.apollohg.editor.viewer

/**
 * Yoga prepares Fabric artifacts before a View exists. Keep its publication
 * sidecar by exact state incarnation so a new semantic identity is cleared before intrinsic
 * fallback lookup, then mount binds the same state without a second reset.
 */
internal object FabricAttachmentSidecars {
    private val lock = Any()
    private val states = mutableMapOf<FabricGenerationToken, ViewerAttachmentRevisionState>()
    private val measurementState = ThreadLocal<ViewerAttachmentRevisionState>()

    /** Only preparation's explicitly bound owner may satisfy an LRU miss. */
    val currentMeasurementState: ViewerAttachmentRevisionState?
        get() = measurementState.get()

    fun beginIfActive(
        generation: FabricGenerationToken,
        semanticIdentity: String,
        isActive: () -> Boolean,
    ): ViewerAttachmentRevisionState? = synchronized(lock) {
        if (!isActive()) return@synchronized null
        states.getOrPut(generation, ::ViewerAttachmentRevisionState).also {
            it.beginSemanticGeneration(semanticIdentity)
        }
    }

    fun begin(generation: FabricGenerationToken, semanticIdentity: String): ViewerAttachmentRevisionState =
        beginIfActive(generation, semanticIdentity) { true }
            ?: error("Unconditionally active sidecar unexpectedly rejected.")

    fun state(generation: FabricGenerationToken): ViewerAttachmentRevisionState? = synchronized(lock) { states[generation] }

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

    fun remove(generation: FabricGenerationToken) = synchronized(lock) { states.remove(generation)?.reset() }

    fun remove(surface: FabricSurfaceToken) = synchronized(lock) {
        states.keys.filter { it.surface == surface }.forEach { states.remove(it)?.reset() }
    }

    fun removeSurface(surfaceId: Int) = synchronized(lock) {
        states.keys.filter { it.surface.surfaceId == surfaceId }.forEach { token -> states.remove(token)?.reset() }
    }

    fun remove(owner: FabricLeaseOwner) = synchronized(lock) {
        states.keys
            .filter { it.surface == owner.surface && it.leaseHandle == owner.leaseHandle }
            .forEach { states.remove(it)?.reset() }
    }

    /** Commit keeps the incoming generation's in-flight sidecar but drops every stale revision for this family. */
    fun removeOtherGenerations(owner: FabricLeaseOwner, keeping: FabricGenerationToken) = synchronized(lock) {
        states.keys
            .filter { it.surface == owner.surface && it.leaseHandle == owner.leaseHandle && it != keeping }
            .forEach { states.remove(it)?.reset() }
    }
}
