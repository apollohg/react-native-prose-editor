package com.apollohg.editor.viewer

/** JNI-only handoff; handles never pass through `ReadableMap.getDouble()`. */
internal object FabricLeaseHandleBridge {
    private val current = ThreadLocal<Long>()
    internal data class FinalLayout(
        val leaseHandle: Long,
        val contentOriginXPx: Int,
        val contentOriginYPx: Int,
    )
    private val finalLayout = ThreadLocal<FinalLayout>()

    @JvmStatic fun beginNativeMeasure(leaseHandle: Long) {
        finalLayout.remove()
        if (leaseHandle > 0) current.set(leaseHandle) else current.remove()
    }

    @JvmStatic fun beginNativeFinalLayout(
        leaseHandle: Long,
        contentOriginXPx: Int,
        contentOriginYPx: Int,
    ) {
        if (leaseHandle <= 0) {
            current.remove()
            finalLayout.remove()
            return
        }
        current.set(leaseHandle)
        finalLayout.set(FinalLayout(leaseHandle, contentOriginXPx, contentOriginYPx))
    }

    @JvmStatic fun endNativeMeasure() {
        current.remove()
        finalLayout.remove()
    }

    fun currentHandle(): Long = current.get() ?: 0L

    fun currentFinalLayout(): FinalLayout? = finalLayout.get()

    /**
     * The C++ state lifecycle registers before its first Yoga pass and invokes
     * the matching release from its final shared-state destructor. These are
     * static calls only: no View, Context, or manager is retained across JNI.
     */
    @JvmStatic fun registerNativeLease(surfaceId: Int, componentTag: Int, leaseHandle: Long) {
        if (surfaceId <= 0 || componentTag <= 0 || leaseHandle <= 0) return
        PreparedProseLayoutRegistry.shared.registerFabricLease(
            FabricSurfaceToken(surfaceId, componentTag),
            leaseHandle,
        )
    }

    /** Invoked only from the C++ state-family's final destructor callback. */
    @JvmStatic fun finalizeNativeLease(surfaceId: Int, componentTag: Int, leaseHandle: Long) {
        if (surfaceId <= 0 || componentTag <= 0 || leaseHandle <= 0) return
        PreparedProseLayoutRegistry.shared.finalizeFabricLease(
            FabricSurfaceToken(surfaceId, componentTag),
            leaseHandle,
        )
    }
}
