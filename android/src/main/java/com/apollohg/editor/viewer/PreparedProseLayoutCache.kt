package com.apollohg.editor.viewer

import com.apollohg.editor.BuildConfig
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap

/** Byte-bounded unmounted LRU plus non-evictable Fabric/direct owners. */
internal class PreparedProseLayoutCache(private val byteBudget: Long = 32L * 1024L * 1024L) {
    private val lock = Any()
    private val inFlight = ConcurrentHashMap<ProseLayoutKey, CompletableFuture<PreparedProseLayout>>()
    private val completed = LinkedHashMap<ProseLayoutKey, PreparedProseLayout>(16, 0.75f, true)
    private val pendingLeases = LinkedHashMap<FabricLeaseKey, PreparedProseLayout>(16, 0.75f, true)
    private val mountedLeases = LinkedHashMap<FabricLeaseKey, PreparedProseLayout>(16, 0.75f, true)
    private val directMounted = mutableMapOf<String, PreparedProseLayout>()
    private val mountIndex = mutableMapOf<ProseMountKey, ProseLayoutKey>()
    private val publishedKeys = mutableSetOf<ProseLayoutKey>()

    fun value(key: ProseLayoutKey, fabricGeneration: FabricGenerationToken? = null, build: () -> PreparedProseLayout): PreparedProseLayout {
        val started = PreparedProseInstrumentation.now()
        synchronized(lock) { completed[key]?.let { if (fabricGeneration != null) createPendingLeaseLocked(it, fabricGeneration); PreparedProseInstrumentation.cacheLookup(started, true); return it } }
        val fresh = CompletableFuture<PreparedProseLayout>(); val existing = inFlight.putIfAbsent(key, fresh)
        if (existing != null) { val layout = existing.join(); if (fabricGeneration != null) synchronized(lock) { createPendingLeaseLocked(layout, fabricGeneration) }; PreparedProseInstrumentation.cacheLookup(started, true, true); return layout }
        PreparedProseInstrumentation.cacheLookup(started, false)
        val layout = try { build() } catch (error: Throwable) { fresh.completeExceptionally(error); inFlight.remove(key, fresh); throw error }
        synchronized(lock) {
            if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION && !publishedKeys.add(key)) { PreparedProseInstrumentation.duplicatePublication(); check(false) { "Prepared prose layout published twice for a live semantic/width/revision key." } }
            // An oversize artifact bypasses only the unmounted cache; a caller may
            // still retain it through a Fabric lease or direct mounted owner.
            if (layout.retainedBytes <= byteBudget) { completed[key] = layout; mountIndex[mountKey(key)] = key; enforceBudgetLocked() }
            if (fabricGeneration != null) createPendingLeaseLocked(layout, fabricGeneration)
            publishOwnersLocked()
        }
        fresh.complete(layout); inFlight.remove(key, fresh); return layout
    }
    fun registerDirectMount(owner: String, layout: PreparedProseLayout) = synchronized(lock) { directMounted[owner] = layout; retireUnownedPublicationsLocked(); publishOwnersLocked() }
    fun releaseDirectMount(owner: String) = synchronized(lock) { directMounted.remove(owner); retireUnownedPublicationsLocked(); publishOwnersLocked() }
    /** Fabric consumes only the exact Yoga-created pending handoff. */
    fun acquireForFabricMount(generation: FabricGenerationToken, widthPx: Int, densityBits: Long): PreparedProseLayout? = synchronized(lock) {
        val lease = pendingLeases.entries.firstOrNull { (key, layout) ->
            key.generation == generation && layout.key.widthPx == widthPx && layout.key.densityBits == densityBits
        } ?: return@synchronized null
        pendingLeases.remove(lease.key)
        mountedLeases.keys.filter { it.generation == generation && it != lease.key }.forEach(mountedLeases::remove)
        mountedLeases[lease.key] = lease.value
        retireUnownedPublicationsLocked(); publishOwnersLocked()
        lease.value
    }
    fun releaseLease(generation: FabricGenerationToken) = synchronized(lock) {
        pendingLeases.keys.filter { it.generation == generation }.forEach(pendingLeases::remove)
        mountedLeases.keys.filter { it.generation == generation }.forEach(mountedLeases::remove)
        retireUnownedPublicationsLocked(); publishOwnersLocked()
    }
    fun releasePendingLease(generation: FabricGenerationToken, widthPx: Int? = null, densityBits: Long? = null) = synchronized(lock) {
        pendingLeases.keys.filter { key ->
            key.generation == generation && (widthPx == null || key.layout.widthPx == widthPx) &&
                (densityBits == null || key.layout.densityBits == densityBits)
        }.forEach(pendingLeases::remove)
        retireUnownedPublicationsLocked(); publishOwnersLocked()
    }
    fun releaseSurface(surface: FabricSurfaceToken) = synchronized(lock) {
        pendingLeases.keys.filter { it.generation.surface == surface }.forEach(pendingLeases::remove)
        mountedLeases.keys.filter { it.generation.surface == surface }.forEach(mountedLeases::remove)
        retireUnownedPublicationsLocked(); publishOwnersLocked()
    }
    fun releaseSurfaceId(surfaceId: Int) = synchronized(lock) {
        pendingLeases.keys.filter { it.generation.surface.surfaceId == surfaceId }.forEach(pendingLeases::remove)
        mountedLeases.keys.filter { it.generation.surface.surfaceId == surfaceId }.forEach(mountedLeases::remove)
        retireUnownedPublicationsLocked(); publishOwnersLocked()
    }
    /** Memory pressure trims unmounted layouts before registry compiled documents, never mounted ownership. */
    fun removeAllUnmounted() = synchronized(lock) { completed.clear(); mountIndex.clear(); pendingLeases.clear(); retireUnownedPublicationsLocked(); publishOwnersLocked() }
    internal val completedCountForTesting: Int get() = synchronized(lock) { completed.size }
    internal val retainedBytesForTesting: Long get() = synchronized(lock) { unmountedBytesLocked() }
    internal val leaseCountForTesting: Int get() = synchronized(lock) { pendingLeases.size + mountedLeases.size }
    internal fun hasLease(generation: FabricGenerationToken): Boolean = synchronized(lock) {
        pendingLeases.keys.any { it.generation == generation } || mountedLeases.keys.any { it.generation == generation }
    }
    private fun createPendingLeaseLocked(layout: PreparedProseLayout, generation: FabricGenerationToken) {
        val lease = FabricLeaseKey(generation, layout.key)
        pendingLeases.keys.filter { it.generation == generation && it != lease }.forEach(pendingLeases::remove)
        if (mountedLeases[lease] == null) pendingLeases[lease] = layout
        retireUnownedPublicationsLocked(); publishOwnersLocked()
    }
    private fun enforceBudgetLocked() { while (unmountedBytesLocked() > byteBudget) { val oldest = completed.entries.firstOrNull() ?: break; completed.remove(oldest.key); if (mountIndex[mountKey(oldest.key)] == oldest.key) mountIndex.remove(mountKey(oldest.key)) }; retireUnownedPublicationsLocked() }
    private fun retireUnownedPublicationsLocked() { publishedKeys.retainAll((completed.keys + pendingLeases.values.map { it.key } + mountedLeases.values.map { it.key } + directMounted.values.map { it.key }).toSet()) }
    private fun publishOwnersLocked() { PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.UNMOUNTED_LAYOUT, "cache", unmountedBytesLocked()); PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.FABRIC_LEASE_HANDOFF, "leases", uniqueBytes(pendingLeases.values + mountedLeases.values)); PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.DIRECT_MOUNTED, "views", uniqueBytes(directMounted.values)) }
    /** A shared completed reference is charged to its live mount/lease, never twice. */
    private fun unmountedBytesLocked(): Long {
        val mountedKeys = (pendingLeases.values + mountedLeases.values).map { it.key }.toSet() + directMounted.values.map { it.key }.toSet()
        return uniqueBytes(completed.filterValues { it.key !in mountedKeys }.values)
    }
    private fun uniqueBytes(layouts: Collection<PreparedProseLayout>) = layouts.associateBy { it.key }.values.sumOf { it.retainedBytes }
    private fun mountKey(key: ProseLayoutKey) = ProseMountKey(key.generationIdentity, key.widthPx, key.densityBits)
}
