package com.apollohg.editor.viewer

import com.apollohg.editor.BuildConfig
import java.util.Collections
import java.util.IdentityHashMap
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap
import kotlin.math.abs

internal const val PIXEL_GRID_ROUNDING_SLACK_PX = 1

/** Byte-bounded unmounted LRU plus exact Fabric and direct immutable owners. */
internal class PreparedProseLayoutCache(
    private val byteBudget: Long = 32L * 1024L * 1024L,
    private val pendingLeaseBudget: Int = 256,
) {
    private val lock = Any()
    private val inFlight = ConcurrentHashMap<ProseLayoutKey, CompletableFuture<PreparedProseLayout>>()
    private val completed = LinkedHashMap<ProseLayoutKey, PreparedProseLayout>(16, 0.75f, true)
    // Access-order makes oldest pending work the first evictable candidate.
    private val pendingLeases = LinkedHashMap<FabricLeaseKey, PreparedProseLayout>(16, 0.75f, true)
    private val mountedLeases = LinkedHashMap<FabricLeaseKey, PreparedProseLayout>(16, 0.75f, true)
    private val directMounted = mutableMapOf<String, PreparedProseLayout>()
    private val mountIndex = mutableMapOf<ProseMountKey, ProseLayoutKey>()
    private val publishedKeys = mutableSetOf<ProseLayoutKey>()
    private var benchmarkCensusKeys: MutableSet<ProseLayoutKey>? = null

    fun value(
        key: ProseLayoutKey,
        fabricGeneration: FabricGenerationToken? = null,
        shouldCreateFabricLease: () -> Boolean = { true },
        build: () -> PreparedProseLayout,
    ): PreparedProseLayout {
        val started = PreparedProseInstrumentation.now()
        synchronized(lock) {
            benchmarkCensusKeys?.add(key)
            completed[key]?.let {
                createPendingLeaseIfActiveLocked(it, fabricGeneration, shouldCreateFabricLease)
                PreparedProseInstrumentation.cacheLookup(started, true)
                return it
            }
            // Completed entries are deliberately disposable. Any live owner is
            // still an immutable complete-key source for a different surface.
            liveLayoutLocked(key)?.let {
                createPendingLeaseIfActiveLocked(it, fabricGeneration, shouldCreateFabricLease)
                PreparedProseInstrumentation.cacheLookup(started, true)
                return it
            }
        }
        val fresh = CompletableFuture<PreparedProseLayout>()
        val existing = inFlight.putIfAbsent(key, fresh)
        if (existing != null) {
            val layout = existing.join()
            synchronized(lock) {
                createPendingLeaseIfActiveLocked(layout, fabricGeneration, shouldCreateFabricLease)
            }
            PreparedProseInstrumentation.cacheLookup(started, true, true)
            return layout
        }
        PreparedProseInstrumentation.cacheLookup(started, false)
        val layout = try {
            build()
        } catch (error: Throwable) {
            fresh.completeExceptionally(error)
            inFlight.remove(key, fresh)
            throw error
        }
        synchronized(lock) {
            // A rival direct/live owner may have published while this caller
            // built. Reuse it and discard the duplicate before publication.
            val canonical = liveLayoutLocked(key) ?: completed[key] ?: layout
            if (canonical === layout && BuildConfig.PREPARED_PROSE_INSTRUMENTATION && !publishedKeys.add(key)) {
                PreparedProseInstrumentation.duplicatePublication()
                check(false) { "Prepared prose layout published twice for a live semantic/width/revision key." }
            }
            if (canonical === layout && layout.retainedBytes <= byteBudget) {
                completed[key] = layout
                mountIndex[mountKey(key)] = key
            }
            createPendingLeaseIfActiveLocked(canonical, fabricGeneration, shouldCreateFabricLease)
            enforceBudgetLocked(preferredGeneration = fabricGeneration)
            retireUnownedPublicationsLocked()
            publishOwnersLocked()
            fresh.complete(canonical)
            inFlight.remove(key, fresh)
            return canonical
        }
    }

    fun registerDirectMount(owner: String, layout: PreparedProseLayout) = synchronized(lock) {
        directMounted[owner] = layout
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
    }

    fun releaseDirectMount(owner: String) = synchronized(lock) {
        directMounted.remove(owner)
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
    }

    /** Fabric consumes only the exact Yoga-created pending handoff. */
    fun acquireForFabricMount(generation: FabricGenerationToken, widthPx: Int, densityBits: Long): PreparedProseLayout? = synchronized(lock) {
        val lease = pendingLeases.entries
            .filter { (key, layout) ->
                key.generation == generation && layout.key.densityBits == densityBits &&
                    abs(layout.key.widthPx - widthPx) <= PIXEL_GRID_ROUNDING_SLACK_PX
            }
            .minByOrNull { abs(it.value.key.widthPx - widthPx) } ?: return@synchronized null
        pendingLeases.remove(lease.key)
        mountedLeases.keys
            .filter { it.owner == lease.key.owner && it != lease.key }
            .forEach(mountedLeases::remove)
        mountedLeases[lease.key] = lease.value
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
        lease.value
    }

    fun releaseLease(generation: FabricGenerationToken) = synchronized(lock) {
        pendingLeases.keys.filter { it.generation == generation }.forEach(pendingLeases::remove)
        mountedLeases.keys.filter { it.generation == generation }.forEach(mountedLeases::remove)
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
    }

    /**
     * A family handle may commit only one generation. Retire stale pending
     * handoffs now, but retain an older mounted artifact until this committed
     * generation acquires its replacement under the same owner lock.
     */
    fun activateFabricGeneration(generation: FabricGenerationToken) = synchronized(lock) {
        val owner = FabricLeaseOwner(generation.surface, generation.leaseHandle)
        pendingLeases.keys
            .filter { it.owner == owner && it.generation != generation }
            .forEach(pendingLeases::remove)
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
    }

    fun releaseOwner(owner: FabricLeaseOwner) = synchronized(lock) {
        pendingLeases.keys.filter { it.owner == owner }.forEach(pendingLeases::remove)
        mountedLeases.keys.filter { it.owner == owner }.forEach(mountedLeases::remove)
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
    }

    fun releasePendingLease(generation: FabricGenerationToken, widthPx: Int? = null, densityBits: Long? = null) = synchronized(lock) {
        pendingLeases.keys.filter { key ->
            key.generation == generation && (widthPx == null || key.layout.widthPx == widthPx) &&
                (densityBits == null || key.layout.densityBits == densityBits)
        }.forEach(pendingLeases::remove)
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
    }

    fun releaseSurface(surface: FabricSurfaceToken) = synchronized(lock) {
        pendingLeases.keys.filter { it.generation.surface == surface }.forEach(pendingLeases::remove)
        mountedLeases.keys.filter { it.generation.surface == surface }.forEach(mountedLeases::remove)
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
    }

    fun releaseSurfaceId(surfaceId: Int) = synchronized(lock) {
        pendingLeases.keys.filter { it.generation.surface.surfaceId == surfaceId }.forEach(pendingLeases::remove)
        mountedLeases.keys.filter { it.generation.surface.surfaceId == surfaceId }.forEach(mountedLeases::remove)
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
    }

    /** Memory pressure removes only unmounted cache and pending handoffs. */
    fun removeAllUnmounted() = synchronized(lock) {
        completed.clear()
        mountIndex.clear()
        pendingLeases.clear()
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
    }

    internal val completedCountForTesting: Int get() = synchronized(lock) { completed.size }
    internal val retainedBytesForTesting: Long get() = synchronized(lock) { unmountedBytesLocked() }
    internal val retainedLeaseBytesForTesting: Long get() = synchronized(lock) { uniqueBytes(pendingLeases.values + mountedLeases.values) }
    internal val leaseCountForTesting: Int get() = synchronized(lock) { pendingLeases.size + mountedLeases.size }
    internal val pendingLeaseCountForTesting: Int get() = synchronized(lock) { pendingLeases.size }
    internal fun beginBenchmarkCensus() = synchronized(lock) {
        benchmarkCensusKeys = linkedSetOf()
    }
    internal fun endBenchmarkCensus(): Set<ProseLayoutKey> = synchronized(lock) {
        val liveKeys = completed.keys + pendingLeases.values.map { it.key } +
            mountedLeases.values.map { it.key } + directMounted.values.map { it.key }
        val resident = benchmarkCensusKeys.orEmpty().intersect(liveKeys.toSet())
        benchmarkCensusKeys = null
        resident
    }
    internal fun hasLease(generation: FabricGenerationToken): Boolean = synchronized(lock) {
        pendingLeases.keys.any { it.generation == generation } || mountedLeases.keys.any { it.generation == generation }
    }

    private fun createPendingLeaseIfActiveLocked(
        layout: PreparedProseLayout,
        generation: FabricGenerationToken?,
        shouldCreateFabricLease: () -> Boolean,
    ) {
        if (generation != null && shouldCreateFabricLease()) createPendingLeaseLocked(layout, generation)
    }

    private fun createPendingLeaseLocked(layout: PreparedProseLayout, generation: FabricGenerationToken) {
        val lease = FabricLeaseKey(generation, layout.key)
        pendingLeases.keys
            .filter { it.owner == lease.owner && it != lease }
            .forEach(pendingLeases::remove)
        if (mountedLeases[lease] == null) pendingLeases[lease] = layout
        enforceBudgetLocked(preferredGeneration = generation)
        retireUnownedPublicationsLocked()
        publishOwnersLocked()
    }

    private fun enforceBudgetLocked(preferredGeneration: FabricGenerationToken?) {
        while (budgetedRetainedBytesLocked() > byteBudget) {
            val completedEntry = completed.entries.firstOrNull()
            if (completedEntry != null) {
                completed.remove(completedEntry.key)
                if (mountIndex[mountKey(completedEntry.key)] == completedEntry.key) mountIndex.remove(mountKey(completedEntry.key))
                continue
            }
            // A duplicate pending handoff can be the only exact artifact for
            // another Fabric owner even when the same immutable object is
            // already mounted/direct-mounted. Removing it frees zero bytes,
            // so it cannot satisfy either retained-byte pressure or justify
            // breaking exact-once handoff ownership.
            val pendingEntry = pendingLeases.entries.firstOrNull {
                it.key.generation != preferredGeneration && pendingRemovalLowersBudgetLocked(it)
            } ?: break
            pendingLeases.remove(pendingEntry.key)
        }
        // Shared immutable artifacts may make a pending handoff free zero
        // bytes. Its owner metadata still needs a hard cap in long feeds.
        while (pendingLeases.size > pendingLeaseBudget) {
            val pendingEntry = pendingLeases.entries.firstOrNull {
                it.key.generation != preferredGeneration
            } ?: break
            pendingLeases.remove(pendingEntry.key)
        }
    }

    private fun liveLayoutLocked(key: ProseLayoutKey): PreparedProseLayout? =
        (pendingLeases.values + mountedLeases.values + directMounted.values).firstOrNull { it.key == key }

    private fun retireUnownedPublicationsLocked() {
        publishedKeys.retainAll((completed.keys + pendingLeases.values.map { it.key } + mountedLeases.values.map { it.key } + directMounted.values.map { it.key }).toSet())
    }

    private fun publishOwnersLocked() {
        PreparedProseInstrumentation.cacheUpdated(
            unmountedBytes = unmountedBytesLocked(),
            unmountedResidentCount = completed.size.toLong(),
        )
        PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.UNMOUNTED_LAYOUT, "cache", unmountedBytesLocked())
        PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.FABRIC_LEASE_HANDOFF, "leases", uniqueBytes(pendingLeases.values + mountedLeases.values))
        PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.DIRECT_MOUNTED, "views", uniqueBytes(directMounted.values))
    }

    /** A shared completed reference is charged to its live mount/lease, never twice. */
    private fun unmountedBytesLocked(): Long {
        val live = identitySet(pendingLeases.values + mountedLeases.values + directMounted.values)
        return uniqueBytes(completed.values.filter { it !in live })
    }

    /**
     * The pre-mount budget counts each immutable artifact once, excluding
     * identities already retained by mounted Fabric or direct View owners.
     * A second pending lease for such an object is ownership metadata, not an
     * additional allocation.
     */
    private fun budgetedRetainedBytesLocked(): Long {
        val live = identitySet(mountedLeases.values + directMounted.values)
        return uniqueBytes((completed.values + pendingLeases.values).filter { it !in live })
    }

    private fun pendingRemovalLowersBudgetLocked(
        entry: Map.Entry<FabricLeaseKey, PreparedProseLayout>,
    ): Boolean {
        val layout = entry.value
        val mountedOrDirect = identitySet(mountedLeases.values + directMounted.values)
        if (layout in mountedOrDirect) return false
        if (completed.values.any { it === layout }) return false
        return pendingLeases.any { (key, candidate) -> key != entry.key && candidate === layout }.not()
    }

    private fun identitySet(layouts: Collection<PreparedProseLayout>): MutableSet<PreparedProseLayout> =
        Collections.newSetFromMap(IdentityHashMap<PreparedProseLayout, Boolean>()).apply { addAll(layouts) }

    private fun uniqueBytes(layouts: Collection<PreparedProseLayout>): Long {
        val seen = identitySet(layouts)
        return seen.sumOf { it.retainedBytes }
    }

    private val FabricLeaseKey.owner: FabricLeaseOwner
        get() = FabricLeaseOwner(generation.surface, generation.leaseHandle)

    private fun mountKey(key: ProseLayoutKey) = ProseMountKey(key.generationIdentity, key.widthPx, key.densityBits)
}
