package com.apollohg.editor.viewer

import com.apollohg.editor.BuildConfig
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
    private val publishedKeys = mutableSetOf<ProseLayoutKey>()

    fun value(
        key: ProseLayoutKey,
        surface: FabricSurfaceToken? = null,
        build: () -> PreparedProseLayout,
    ): PreparedProseLayout {
        val lookupStarted = PreparedProseInstrumentation.now()
        synchronized(lock) {
            completed[key]?.let { layout ->
                if (surface != null) leaseLocked(layout, surface)
                PreparedProseInstrumentation.cacheLookup(lookupStarted, hit = true)
                return layout
            }
        }
        val fresh = CompletableFuture<PreparedProseLayout>()
        val existing = inFlight.putIfAbsent(key, fresh)
        if (existing != null) {
            val layout = existing.join()
            if (surface != null && isRetainable(layout)) synchronized(lock) { leaseLocked(layout, surface) }
            PreparedProseInstrumentation.cacheLookup(lookupStarted, hit = true, waited = true)
            return layout
        }
        PreparedProseInstrumentation.cacheLookup(lookupStarted, hit = false)
        val layout = try {
            build()
        } catch (throwable: Throwable) {
            fresh.completeExceptionally(throwable)
            inFlight.remove(key, fresh)
            throw throwable
        }
        synchronized(lock) {
            // An artifact larger than the whole budget is useful to the caller that
            // measured it, but retaining it as either an LRU entry or a Fabric lease
            // would make the advertised byte bound false.
            if (isRetainable(layout)) {
                if (BuildConfig.DEBUG) {
                    check(publishedKeys.add(key)) {
                        PreparedProseInstrumentation.duplicatePublication()
                        "Prepared prose layout published twice for a live semantic/width/revision key."
                    }
                }
                completed[key] = layout
                mountIndex[mountKey(key)] = key
                if (surface != null) leaseLocked(layout, surface) else enforceBudgetLocked()
            }
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

    fun releaseSurfaceId(surfaceId: Int) = synchronized(lock) {
        val released = leases.keys.filter { it.surface.surfaceId == surfaceId }
        released.forEach { leases.remove(it) }
        leaseBySurface.entries.removeAll { it.key.surfaceId == surfaceId }
    }

    fun removeAllUnmounted() = synchronized(lock) {
        completed.clear()
        leases.clear()
        leaseBySurface.clear()
        mountIndex.clear()
        publishedKeys.clear()
        PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.LAYOUT, "unmounted-cache", 0L)
    }

    internal val completedCountForTesting: Int get() = synchronized(lock) { completed.size }
    internal val retainedBytesForTesting: Long get() = synchronized(lock) { retainedBytesLocked() }
    internal val leaseCountForTesting: Int get() = synchronized(lock) { leases.size }

    private fun leaseLocked(layout: PreparedProseLayout, surface: FabricSurfaceToken) {
        releaseLease(surface)
        val generation = FabricGenerationToken(surface, layout.key.generationIdentity)
        leases[generation] = layout
        leaseBySurface[surface] = generation
        enforceBudgetLocked(preferredLease = generation)
    }

    private fun isRetainable(layout: PreparedProseLayout): Boolean =
        layout.retainedBytes <= byteBudget

    private fun enforceBudgetLocked(preferredLease: FabricGenerationToken? = null) {
        while (completed.size > entryBudget || completedRetainedBytesLocked() > byteBudget) {
            val oldest = completed.entries.firstOrNull() ?: break
            completed.remove(oldest.key)
            if (leases.values.none { it.key == oldest.key }) publishedKeys.remove(oldest.key)
            val mountKey = mountKey(oldest.key)
            if (mountIndex[mountKey] == oldest.key) mountIndex.remove(mountKey)
        }
        // Handoff leases represent mounted ownership and are intentionally not
        // eligible for the unmounted-cache LRU or its byte budget.
        PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.LAYOUT, "unmounted-cache", completedRetainedBytesLocked())
    }

    private fun retainedBytesLocked(): Long {
        val layouts = LinkedHashMap<ProseLayoutKey, PreparedProseLayout>()
        completed.values.forEach { layouts[it.key] = it }
        leases.values.forEach { layouts[it.key] = it }
        return layouts.values.sumOf { it.retainedBytes }
    }

    private fun completedRetainedBytesLocked(): Long = completed.values
        .associateBy { it.key }
        .values
        .sumOf { it.retainedBytes }

    private fun mountKey(key: ProseLayoutKey) =
        ProseMountKey(key.generationIdentity, key.widthPx, key.densityBits)
}
