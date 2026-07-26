package com.apollohg.editor.viewer

import com.apollohg.editor.BuildConfig
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap

/** Byte-bounded unmounted LRU plus non-evictable Fabric/direct owners. */
internal class PreparedProseLayoutCache(private val byteBudget: Long = 32L * 1024L * 1024L) {
    private val lock = Any()
    private val inFlight = ConcurrentHashMap<ProseLayoutKey, CompletableFuture<PreparedProseLayout>>()
    private val completed = LinkedHashMap<ProseLayoutKey, PreparedProseLayout>(16, 0.75f, true)
    private val leases = LinkedHashMap<FabricGenerationToken, PreparedProseLayout>(16, 0.75f, true)
    private val directMounted = mutableMapOf<String, PreparedProseLayout>()
    private val leaseBySurface = mutableMapOf<FabricSurfaceToken, FabricGenerationToken>()
    private val mountIndex = mutableMapOf<ProseMountKey, ProseLayoutKey>()
    private val publishedKeys = mutableSetOf<ProseLayoutKey>()

    fun value(key: ProseLayoutKey, surface: FabricSurfaceToken? = null, build: () -> PreparedProseLayout): PreparedProseLayout {
        val started = PreparedProseInstrumentation.now()
        synchronized(lock) { completed[key]?.let { if (surface != null) leaseLocked(it, surface); PreparedProseInstrumentation.cacheLookup(started, true); return it } }
        val fresh = CompletableFuture<PreparedProseLayout>(); val existing = inFlight.putIfAbsent(key, fresh)
        if (existing != null) { val layout = existing.join(); if (surface != null) synchronized(lock) { leaseLocked(layout, surface) }; PreparedProseInstrumentation.cacheLookup(started, true, true); return layout }
        PreparedProseInstrumentation.cacheLookup(started, false)
        val layout = try { build() } catch (error: Throwable) { fresh.completeExceptionally(error); inFlight.remove(key, fresh); throw error }
        synchronized(lock) {
            if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION && !publishedKeys.add(key)) { PreparedProseInstrumentation.duplicatePublication(); check(false) { "Prepared prose layout published twice for a live semantic/width/revision key." } }
            // An oversize artifact bypasses only the unmounted cache; a caller may
            // still retain it through a Fabric lease or direct mounted owner.
            if (layout.retainedBytes <= byteBudget) { completed[key] = layout; mountIndex[mountKey(key)] = key; enforceBudgetLocked() }
            if (surface != null) leaseLocked(layout, surface)
            publishOwnersLocked()
        }
        fresh.complete(layout); inFlight.remove(key, fresh); return layout
    }
    fun registerDirectMount(owner: String, layout: PreparedProseLayout) = synchronized(lock) { directMounted[owner] = layout; publishOwnersLocked() }
    fun releaseDirectMount(owner: String) = synchronized(lock) { directMounted.remove(owner); retireUnownedPublicationsLocked(); publishOwnersLocked() }
    /** Acquisition does not consume the lease; it remains live until Fabric release. */
    fun acquireForFabricMount(surface: FabricSurfaceToken, generationIdentity: String, widthPx: Int, densityBits: Long): PreparedProseLayout? = synchronized(lock) {
        leaseBySurface[surface]?.takeIf { it.generationIdentity == generationIdentity }?.let { leases[it]?.takeIf { layout -> layout.key.widthPx == widthPx && layout.key.densityBits == densityBits }?.let { return@synchronized it } }
        completed[mountIndex[ProseMountKey(generationIdentity, widthPx, densityBits)]]
    }
    fun releaseLease(surface: FabricSurfaceToken, generationIdentity: String? = null) = synchronized(lock) { val lease = leaseBySurface[surface] ?: return@synchronized; if (generationIdentity == null || lease.generationIdentity == generationIdentity) { leases.remove(lease); leaseBySurface.remove(surface); retireUnownedPublicationsLocked(); publishOwnersLocked() } }
    fun releaseSurfaceId(surfaceId: Int) = synchronized(lock) { leaseBySurface.filterKeys { it.surfaceId == surfaceId }.keys.toList().forEach { surface -> val lease = leaseBySurface.remove(surface) ?: return@forEach; leases.remove(lease) }; retireUnownedPublicationsLocked(); publishOwnersLocked() }
    /** Memory pressure trims unmounted layouts before registry compiled documents, never mounted ownership. */
    fun removeAllUnmounted() = synchronized(lock) { completed.clear(); mountIndex.clear(); retireUnownedPublicationsLocked(); publishOwnersLocked() }
    internal val completedCountForTesting: Int get() = synchronized(lock) { completed.size }
    internal val retainedBytesForTesting: Long get() = synchronized(lock) { unmountedBytesLocked() }
    internal val leaseCountForTesting: Int get() = synchronized(lock) { leases.size }
    private fun leaseLocked(layout: PreparedProseLayout, surface: FabricSurfaceToken) { val old = leaseBySurface.remove(surface); if (old != null) leases.remove(old); val token = FabricGenerationToken(surface, layout.key.generationIdentity); leases[token] = layout; leaseBySurface[surface] = token }
    private fun enforceBudgetLocked() { while (unmountedBytesLocked() > byteBudget) { val oldest = completed.entries.firstOrNull() ?: break; completed.remove(oldest.key); if (mountIndex[mountKey(oldest.key)] == oldest.key) mountIndex.remove(mountKey(oldest.key)) }; retireUnownedPublicationsLocked() }
    private fun retireUnownedPublicationsLocked() { publishedKeys.retainAll((completed.keys + leases.values.map { it.key } + directMounted.values.map { it.key }).toSet()) }
    private fun publishOwnersLocked() { PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.UNMOUNTED_LAYOUT, "cache", unmountedBytesLocked()); PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.FABRIC_LEASE_HANDOFF, "leases", uniqueBytes(leases.values)); PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.DIRECT_MOUNTED, "views", uniqueBytes(directMounted.values)) }
    /** A shared completed reference is charged to its live mount/lease, never twice. */
    private fun unmountedBytesLocked(): Long {
        val mountedKeys = leases.values.map { it.key }.toSet() + directMounted.values.map { it.key }.toSet()
        return uniqueBytes(completed.filterValues { it.key !in mountedKeys }.values)
    }
    private fun uniqueBytes(layouts: Collection<PreparedProseLayout>) = layouts.associateBy { it.key }.values.sumOf { it.retainedBytes }
    private fun mountKey(key: ProseLayoutKey) = ProseMountKey(key.generationIdentity, key.widthPx, key.densityBits)
}
