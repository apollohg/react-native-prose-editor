package com.apollohg.editor.viewer

import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap

/**
 * Owns completed unmounted layouts plus the one-shot Yoga-to-Fabric handoff.
 * A lease shares an artifact with the LRU, so byte accounting counts it once.
 */
internal class PreparedProseLayoutCache(
    private val byteBudget: Long = 32L * 1024L * 1024L,
    private val entryBudget: Int = 512,
    private val leaseBudget: Int = 32,
) {
    private val lock = Any()
    private val inFlight = ConcurrentHashMap<ProseLayoutKey, CompletableFuture<PreparedProseLayout>>()
    private val completed = LinkedHashMap<ProseLayoutKey, PreparedProseLayout>(16, 0.75f, true)
    private val leases = LinkedHashMap<FabricGenerationToken, PreparedProseLayout>(16, 0.75f, true)
    private val leaseBySurface = mutableMapOf<FabricSurfaceToken, FabricGenerationToken>()
    private val mountIndex = mutableMapOf<ProseMountKey, ProseLayoutKey>()

    fun value(
        key: ProseLayoutKey,
        surface: FabricSurfaceToken? = null,
        build: () -> PreparedProseLayout,
    ): PreparedProseLayout {
        synchronized(lock) {
            completed[key]?.let { layout ->
                if (surface != null) leaseLocked(layout, surface)
                return layout
            }
        }
        val fresh = CompletableFuture<PreparedProseLayout>()
        val existing = inFlight.putIfAbsent(key, fresh)
        if (existing != null) {
            val layout = existing.join()
            if (surface != null) synchronized(lock) { leaseLocked(layout, surface) }
            return layout
        }
        val layout = try {
            build()
        } catch (throwable: Throwable) {
            fresh.completeExceptionally(throwable)
            inFlight.remove(key, fresh)
            throw throwable
        }
        synchronized(lock) {
            completed[key] = layout
            mountIndex[mountKey(key)] = key
            if (surface != null) leaseLocked(layout, surface) else enforceBudgetLocked()
        }
        fresh.complete(layout)
        inFlight.remove(key, fresh)
        return layout
    }

    /** Fabric mount is acquisition-only: it must never compile or prepare. */
    fun acquireForFabricMount(
        surface: FabricSurfaceToken,
        generationIdentity: String,
        widthPx: Int,
        densityBits: Long,
    ): PreparedProseLayout? = synchronized(lock) {
        val lease = leaseBySurface[surface]
        if (lease != null && lease.generationIdentity == generationIdentity) {
            val layout = leases.remove(lease)
            leaseBySurface.remove(surface)
            if (layout != null && layout.key.widthPx == widthPx && layout.key.densityBits == densityBits) {
                return@synchronized layout
            }
        }
        completed[mountIndex[ProseMountKey(generationIdentity, widthPx, densityBits)]]
    }

    fun releaseLease(surface: FabricSurfaceToken, generationIdentity: String? = null) = synchronized(lock) {
        val lease = leaseBySurface[surface] ?: return@synchronized
        if (generationIdentity == null || lease.generationIdentity == generationIdentity) {
            leases.remove(lease)
            leaseBySurface.remove(surface)
        }
    }

    fun removeAllUnmounted() = synchronized(lock) {
        completed.clear()
        leases.clear()
        leaseBySurface.clear()
        mountIndex.clear()
    }

    internal val completedCountForTesting: Int get() = synchronized(lock) { completed.size }
    internal val retainedBytesForTesting: Long get() = synchronized(lock) { retainedBytesLocked() }

    private fun leaseLocked(layout: PreparedProseLayout, surface: FabricSurfaceToken) {
        releaseLease(surface)
        val generation = FabricGenerationToken(surface, layout.key.generationIdentity)
        leases[generation] = layout
        leaseBySurface[surface] = generation
        enforceBudgetLocked(preferredLease = generation)
    }

    private fun enforceBudgetLocked(preferredLease: FabricGenerationToken? = null) {
        while (completed.size > entryBudget || retainedBytesLocked() > byteBudget) {
            val oldest = completed.entries.firstOrNull() ?: break
            completed.remove(oldest.key)
            val mountKey = mountKey(oldest.key)
            if (mountIndex[mountKey] == oldest.key) mountIndex.remove(mountKey)
        }
        while (leases.size > leaseBudget || retainedBytesLocked() > byteBudget) {
            val oldest = leases.entries.firstOrNull { it.key != preferredLease } ?: break
            leases.remove(oldest.key)
            if (leaseBySurface[oldest.key.surface] == oldest.key) leaseBySurface.remove(oldest.key.surface)
        }
    }

    private fun retainedBytesLocked(): Long {
        val layouts = LinkedHashMap<ProseLayoutKey, PreparedProseLayout>()
        completed.values.forEach { layouts[it.key] = it }
        leases.values.forEach { layouts[it.key] = it }
        return layouts.values.sumOf { it.retainedBytes }
    }

    private fun mountKey(key: ProseLayoutKey) =
        ProseMountKey(key.generationIdentity, key.widthPx, key.densityBits)
}
