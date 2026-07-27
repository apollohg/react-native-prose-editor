package com.apollohg.editor.viewer

/** JNI-only handoff; handles never pass through `ReadableMap.getDouble()`. */
internal object FabricLeaseHandleBridge {
    private val current = ThreadLocal<Long>()

    @JvmStatic fun beginNativeMeasure(leaseHandle: Long) {
        if (leaseHandle > 0) current.set(leaseHandle) else current.remove()
    }

    @JvmStatic fun endNativeMeasure() = current.remove()

    fun currentHandle(): Long = current.get() ?: 0L
}
