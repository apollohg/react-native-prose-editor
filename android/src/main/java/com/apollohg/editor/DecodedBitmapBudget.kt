package com.apollohg.editor

import android.app.ActivityManager
import android.content.Context
import android.graphics.Bitmap
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

internal enum class DecodedBitmapPriority { VISIBLE, PREFETCH }

internal class DecodedBitmapBudget(private val processLimitBytes: Long) {
    internal class Allocation(
        var bytes: Long,
        var bitmap: Bitmap? = null,
        var references: Int = 1,
        val ownerReferences: MutableMap<Long, Int> = mutableMapOf(),
        val ownerPriorityReferences: MutableMap<Pair<Long, DecodedBitmapPriority>, Int> = mutableMapOf(),
    )

    private val lock = Any()
    private var retainedProcessBytes = 0L
    private val retainedOwnerBytes = mutableMapOf<Long, Long>()
    private val ownerPressureHandlers = mutableMapOf<Long, () -> Unit>()
    @Volatile private var pressureHandler: (() -> Unit)? = null

    fun reserve(bytes: Long, priority: DecodedBitmapPriority): DecodedBitmapReservation? {
        if (bytes <= 0L || bytes > processLimitBytes) return null
        fun attempt(): Allocation? = synchronized(lock) {
            val next = checkedAdd(retainedProcessBytes, bytes) ?: return@synchronized null
            if (next > processLimitBytes) return@synchronized null
            retainedProcessBytes = next
            Allocation(bytes)
        }
        var allocation = attempt()
        if (allocation == null && priority == DecodedBitmapPriority.VISIBLE) {
            pressureHandler?.invoke()
            allocation = attempt()
            if (allocation == null) {
                synchronized(lock) { ownerPressureHandlers.values.toList() }.forEach { it() }
                pressureHandler?.invoke()
                allocation = attempt()
            }
        }
        allocation ?: return null
        return DecodedBitmapReservation(this, allocation)
    }

    fun setPressureHandler(handler: (() -> Unit)?) {
        pressureHandler = handler
    }

    fun setOwnerPressureHandler(ownerId: Long, handler: (() -> Unit)?) = synchronized(lock) {
        if (handler == null) ownerPressureHandlers.remove(ownerId)
        else ownerPressureHandlers[ownerId] = handler
    }

    private fun reconcile(allocation: Allocation, actualBytes: Long): Boolean = synchronized(lock) {
        if (actualBytes <= 0L || allocation.references <= 0) return@synchronized false
        val withoutAllocation = retainedProcessBytes - allocation.bytes
        val next = checkedAdd(withoutAllocation, actualBytes) ?: return@synchronized false
        if (next > processLimitBytes) return@synchronized false
        retainedProcessBytes = next
        allocation.bytes = actualBytes
        true
    }

    private fun fork(
        allocation: Allocation,
        ownerId: Long,
        ownerLimitBytes: Long,
        priority: DecodedBitmapPriority,
    ): DecodedBitmapLease? {
        if (ownerLimitBytes <= 0L || allocation.bytes > ownerLimitBytes) return null
        fun attempt(): DecodedBitmapLease? = synchronized(lock) {
            if (allocation.references <= 0 || allocation.bitmap == null) {
                return@synchronized null
            }
            val ownerReferences = allocation.ownerReferences[ownerId] ?: 0
            if (ownerReferences == 0) {
                val retained = retainedOwnerBytes[ownerId] ?: 0L
                val next = checkedAdd(retained, allocation.bytes) ?: return@synchronized null
                if (next > ownerLimitBytes) return@synchronized null
                retainedOwnerBytes[ownerId] = next
            }
            val priorityKey = ownerId to priority
            val priorityReferences = allocation.ownerPriorityReferences[priorityKey] ?: 0
            if (
                allocation.references == Int.MAX_VALUE ||
                ownerReferences == Int.MAX_VALUE ||
                priorityReferences == Int.MAX_VALUE
            ) return@synchronized null
            allocation.references += 1
            allocation.ownerReferences[ownerId] = ownerReferences + 1
            allocation.ownerPriorityReferences[priorityKey] = priorityReferences + 1
            DecodedBitmapLease(this, allocation, ownerId, priority)
        }
        var lease = attempt()
        if (lease == null && priority == DecodedBitmapPriority.VISIBLE) {
            synchronized(lock) { ownerPressureHandlers[ownerId] }?.invoke()
            lease = attempt()
        }
        return lease
    }

    private fun forkUnowned(allocation: Allocation): DecodedBitmapLease? = synchronized(lock) {
        if (allocation.references <= 0 || allocation.bitmap == null || allocation.references == Int.MAX_VALUE) {
            return@synchronized null
        }
        allocation.references += 1
        DecodedBitmapLease(this, allocation, null, null)
    }

    private fun release(
        allocation: Allocation,
        ownerId: Long?,
        priority: DecodedBitmapPriority?,
    ) = synchronized(lock) {
        if (allocation.references <= 0) return@synchronized
        if (ownerId != null) {
            val references = allocation.ownerReferences[ownerId] ?: return@synchronized
            val priorityKey = ownerId to checkNotNull(priority)
            val priorityReferences = allocation.ownerPriorityReferences[priorityKey]
                ?: return@synchronized
            if (priorityReferences == 1) allocation.ownerPriorityReferences.remove(priorityKey)
            else allocation.ownerPriorityReferences[priorityKey] = priorityReferences - 1
            if (references == 1) {
                allocation.ownerReferences.remove(ownerId)
                val remaining = (retainedOwnerBytes[ownerId] ?: 0L) - allocation.bytes
                if (remaining <= 0L) retainedOwnerBytes.remove(ownerId)
                else retainedOwnerBytes[ownerId] = remaining
            } else {
                allocation.ownerReferences[ownerId] = references - 1
            }
        }
        allocation.references -= 1
        if (allocation.references == 0) {
            retainedProcessBytes = (retainedProcessBytes - allocation.bytes).coerceAtLeast(0L)
            allocation.bitmap = null
            allocation.ownerReferences.clear()
            allocation.ownerPriorityReferences.clear()
        }
    }

    internal fun retainedProcessBytesForTesting(): Long = synchronized(lock) { retainedProcessBytes }

    internal fun retainedOwnerBytesForTesting(ownerId: Long): Long =
        synchronized(lock) { retainedOwnerBytes[ownerId] ?: 0L }

    internal companion object {
        private const val MIB = 1024L * 1024L
        private val ownerIds = AtomicLong()
        private val sharedLock = Any()
        @Volatile private var sharedBudget: DecodedBitmapBudget? = null

        fun processLimitBytes(memoryClassMib: Int): Long =
            (memoryClassMib.coerceAtLeast(0).toLong() * MIB / 8L)
                .coerceIn(16L * MIB, 128L * MIB)

        fun shared(context: Context? = null): DecodedBitmapBudget {
            sharedBudget?.let { return it }
            return synchronized(sharedLock) {
                sharedBudget ?: DecodedBitmapBudget(
                    processLimitBytes(
                        context?.applicationContext
                            ?.getSystemService(Context.ACTIVITY_SERVICE)
                            ?.let { it as? ActivityManager }
                            ?.memoryClass
                            ?: (Runtime.getRuntime().maxMemory() / MIB)
                                .coerceAtMost(Int.MAX_VALUE.toLong())
                                .toInt()
                    )
                ).also { sharedBudget = it }
            }
        }

        fun nextOwnerId(): Long = ownerIds.incrementAndGet()

        private fun checkedAdd(left: Long, right: Long): Long? =
            if (left < 0L || right < 0L || left > Long.MAX_VALUE - right) null else left + right
    }

    internal class DecodedBitmapReservation internal constructor(
        private val budget: DecodedBitmapBudget,
        private val allocation: Allocation,
    ) : Closeable {
        private val finished = AtomicBoolean(false)

        fun commit(bitmap: Bitmap, actualBytes: Long): DecodedBitmapLease? {
            if (!finished.compareAndSet(false, true)) return null
            if (!budget.reconcile(allocation, actualBytes)) {
                budget.release(allocation, null, null)
                return null
            }
            allocation.bitmap = bitmap
            return DecodedBitmapLease(budget, allocation, null, null)
        }

        override fun close() {
            if (finished.compareAndSet(false, true)) budget.release(allocation, null, null)
        }
    }

    internal class DecodedBitmapLease internal constructor(
        private val budget: DecodedBitmapBudget,
        private val allocation: Allocation,
        private val ownerId: Long?,
        private val priority: DecodedBitmapPriority?,
    ) : Closeable {
        private val closed = AtomicBoolean(false)
        val bitmap: Bitmap get() = checkNotNull(allocation.bitmap)
        val byteCount: Long get() = allocation.bytes

        fun fork(
            ownerId: Long,
            ownerLimitBytes: Long,
            priority: DecodedBitmapPriority,
        ): DecodedBitmapLease? {
            if (closed.get()) return null
            return budget.fork(allocation, ownerId, ownerLimitBytes, priority)
        }

        internal fun forkUnowned(): DecodedBitmapLease? {
            if (closed.get()) return null
            return budget.forkUnowned(allocation)
        }

        override fun close() {
            if (closed.compareAndSet(false, true)) budget.release(allocation, ownerId, priority)
        }
    }
}

internal typealias DecodedBitmapReservation = DecodedBitmapBudget.DecodedBitmapReservation
internal typealias DecodedBitmapLease = DecodedBitmapBudget.DecodedBitmapLease
