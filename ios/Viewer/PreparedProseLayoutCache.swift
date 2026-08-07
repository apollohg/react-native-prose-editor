import Foundation

/// Owns unmounted layouts and exact Yoga-to-Fabric handoffs. A lease shares
/// its immutable artifact with the LRU; lifecycle release owns a live
/// handoff's retirement so its eventual mount cannot fall back to a cache.
final class PreparedProseLayoutCache {
    static let pixelGridRoundingSlackPixels = 1

    private final class Preparation {
        var result: Result<PreparedProseLayout, Error>?
    }

    private let condition = NSCondition()
    private var completed: [ProseLayoutKey: PreparedProseLayout] = [:]
    /// Accesses append a generation token instead of moving an existing array
    /// element. Stale tokens are ignored at eviction time and periodically
    /// compacted, so ordinary cache hits remain O(1) while eviction retains
    /// exact LRU order.
    private var accessOrder: [(key: ProseLayoutKey, generation: UInt64)] = []
    private var accessGenerations: [ProseLayoutKey: UInt64] = [:]
    private var accessOrderHead = 0
    private var nextAccessGeneration: UInt64 = 0
    private var inFlight: [ProseLayoutKey: Preparation] = [:]
    /// Yoga measurement hands an immutable artifact to Fabric before there is
    /// a component view. A pending handoff is therefore distinct from a
    /// mounted owner: mount consumes it exactly once. Both are live exact
    /// owners and remain retained until their lifecycle releases them.
    private var pendingLeases: [FabricLeaseKey: PreparedProseLayout] = [:]
    private var mountedLeases: [FabricLeaseKey: PreparedProseLayout] = [:]
    private var directMounted: [String: PreparedProseLayout] = [:]
    /// Secondary ownership indexes make mount/release lookup proportional to
    /// the affected owners, not every retained artifact.
    private var pendingLeaseKeysByOwner: [FabricLeaseOwner: Set<FabricLeaseKey>] = [:]
    private var mountedLeaseKeysByOwner: [FabricLeaseOwner: Set<FabricLeaseKey>] = [:]
    private var pendingLeaseKeysBySurface: [FabricSurfaceToken: Set<FabricLeaseKey>] = [:]
    private var mountedLeaseKeysBySurface: [FabricSurfaceToken: Set<FabricLeaseKey>] = [:]
    private var pendingLeaseKeysByLayout: [ProseLayoutKey: Set<FabricLeaseKey>] = [:]
    private var mountedLeaseKeysByLayout: [ProseLayoutKey: Set<FabricLeaseKey>] = [:]
    private var directOwnersByLayout: [ProseLayoutKey: Set<String>] = [:]

    private struct LayoutOwnership {
        let layout: PreparedProseLayout
        var completedReferences = 0
        var pendingReferences = 0
        var mountedReferences = 0
        var directReferences = 0
    }

    /// All retention totals are maintained on identity transitions. A layout
    /// may be referenced by several roles, but is counted once in each metric
    /// with the same exclusions as the former set-based calculations.
    private var ownershipByIdentifier: [ObjectIdentifier: LayoutOwnership] = [:]
    private var retainedBytes = 0
    private var budgetedRetainedBytes = 0
    private var unmountedRetainedBytes = 0
    private var fabricLeaseRetainedBytes = 0
    private var directMountedRetainedBytes = 0
    private var oversizedLeaseCount = 0
    private var benchmarkCensusKeys: Set<ProseLayoutKey>?
    private var mountIndex: [ProseMountKey: ProseLayoutKey] = [:]
#if DEBUG
    /// A live generation must publish an artifact once for its complete
    /// semantic/physical-width/revision key. Eviction retires only unmounted
    /// entries, so a mounted owner can never be republished accidentally.
    private var publishedKeys: Set<ProseLayoutKey> = []
    private var livePublicationCounts: [ProseLayoutKey: Int] = [:]
    private var publicationRetirementCandidates: Set<ProseLayoutKey> = []
#endif
    private let byteBudget: Int
    init(byteBudget: Int = 32 * 1024 * 1024) {
        self.byteBudget = byteBudget
    }

    func value(
        for key: ProseLayoutKey,
        fabricSurface: FabricSurfaceToken? = nil,
        fabricLeaseHandle: UInt64? = nil,
        shouldCreateFabricLease: (() -> Bool)? = nil,
        build: () throws -> PreparedProseLayout
    ) throws -> PreparedProseLayout {
        precondition(fabricSurface == nil || fabricLeaseHandle != nil)
        let lookupStarted = PreparedProseInstrumentation.now()
        condition.lock()
        benchmarkCensusKeys?.insert(key)
        // Mounted ownership is the most authoritative exact-key cache entry:
        // it survives unmounted-cache eviction and memory warnings. Consult it
        // before completed entries, because creating a same-width pending lease
        // would otherwise prune a different-width replacement already pending
        // for this Fabric surface and generation.
        if let layout = mountedLayoutLocked(
            for: key,
            fabricSurface: fabricSurface,
            fabricLeaseHandle: fabricLeaseHandle
        ) {
            condition.unlock()
            PreparedProseInstrumentation.cacheLookup(lookupStarted, hit: true)
            return layout
        }
        if let layout = completed[key] {
            touch(key)
            if let fabricSurface, let fabricLeaseHandle, (shouldCreateFabricLease?() ?? true) {
                createPendingLeaseLocked(layout, for: key, surface: fabricSurface, leaseHandle: fabricLeaseHandle)
            }
            condition.unlock()
            PreparedProseInstrumentation.cacheLookup(lookupStarted, hit: true)
            return layout
        }
        // Immutable prepared layouts are global to their complete layout key,
        // while Fabric ownership is intentionally surface-scoped. Any live
        // owner can therefore satisfy another caller without preparing a
        // duplicate artifact. Fabric receives its own exact-once pending
        // lease; UIKit simply reuses the immutable value without inventing a
        // Fabric owner.
        if let layout = liveLayoutLocked(for: key) {
            if let fabricSurface, let fabricLeaseHandle, (shouldCreateFabricLease?() ?? true) {
                createPendingLeaseLocked(layout, for: key, surface: fabricSurface, leaseHandle: fabricLeaseHandle)
            }
            condition.unlock()
            PreparedProseInstrumentation.cacheLookup(lookupStarted, hit: true)
            return layout
        }
        if let preparation = inFlight[key] {
            while preparation.result == nil { condition.wait() }
            let result = preparation.result!
            if case let .success(layout) = result, let fabricSurface, let fabricLeaseHandle,
               (shouldCreateFabricLease?() ?? true) {
                createPendingLeaseLocked(layout, for: key, surface: fabricSurface, leaseHandle: fabricLeaseHandle)
            }
            condition.unlock()
            PreparedProseInstrumentation.cacheLookup(lookupStarted, hit: true, waited: true)
            return try result.get()
        }
        let preparation = Preparation()
        inFlight[key] = preparation
        condition.unlock()
        PreparedProseInstrumentation.cacheLookup(lookupStarted, hit: false)

        let result = Result(catching: build)

        condition.lock()
        if case let .success(layout) = result {
#if DEBUG
            if !publishedKeys.insert(key).inserted {
                PreparedProseInstrumentation.duplicatePublication()
                preconditionFailure("Prepared prose layout published twice for a live semantic/width/revision key.")
            }
            // A build that is neither completed nor leased must retire before
            // a later publication of the same key. Role insertion removes this
            // candidate, so handoff transfers never transiently retire live
            // publication state.
            publicationRetirementCandidates.insert(key)
#endif
            if layout.retainedBytes <= byteBudget {
                insertCompletedLocked(layout, for: key)
            }
            if let fabricSurface, let fabricLeaseHandle, (shouldCreateFabricLease?() ?? true) {
                createPendingLeaseLocked(layout, for: key, surface: fabricSurface, leaseHandle: fabricLeaseHandle)
            } else {
                enforceBudgetLocked()
            }
        }
        preparation.result = result
        inFlight.removeValue(forKey: key)
        condition.broadcast()
        condition.unlock()
        return try result.get()
    }

    /// Used only by Fabric mount. It never builds, compiles, or prepares.
    func acquireForFabricMount(
        surface: FabricSurfaceToken,
        generationIdentity: String,
        widthPixels: Int,
        displayScale: CGFloat,
        leaseHandle: UInt64
    ) -> PreparedProseLayout? {
        condition.lock()
        defer { condition.unlock() }

        let requestedMountKey = ProseMountKey(
            generationIdentity: generationIdentity,
            widthPixels: widthPixels,
            displayScale: displayScale
        )
        // UIKit-only callers use the documented zero token because they have
        // no Fabric owner. They retain the ordinary completed-cache lookup;
        // every real Fabric surface must consume its own pending handoff.
        if surface.surfaceId == 0, surface.componentTag == 0,
           let key = mountIndex[requestedMountKey], let layout = completed[key] {
            touch(key)
            return layout
        }
        let owner = FabricLeaseOwner(surface: surface, leaseHandle: leaseHandle)
        guard let leaseKey = (pendingLeaseKeysByOwner[owner] ?? [])
            .filter({
                $0.layout.generationIdentity == requestedMountKey.generationIdentity
                    && $0.layout.displayScaleBits == requestedMountKey.displayScaleBits
                    && abs($0.layout.widthPixels - widthPixels) <= Self.pixelGridRoundingSlackPixels
            })
            .min(by: {
                (abs($0.layout.widthPixels - widthPixels), $0.layout.widthPixels)
                    < (abs($1.layout.widthPixels - widthPixels), $1.layout.widthPixels)
            }),
            let layout = pendingLeases[leaseKey]
        else { return nil }

        // A Fabric mount can only consume the Yoga handoff once. Do not fall
        // back to completed here: that would let a second/recycled component
        // mount an artifact that was never measured for its own owner. Move
        // the pending handoff to mounted ownership under this lock, retiring
        // only other widths for the same surface/generation/handle in the same
        // critical section. A replacement that never reaches this point
        // therefore cannot disturb the currently mounted artifact.
        removePendingLeaseLocked(leaseKey)
        for stale in mountedLeaseKeysByOwner[owner] ?? [] where stale != leaseKey {
            removeMountedLeaseLocked(stale)
        }
        insertMountedLeaseLocked(layout, for: leaseKey)
        // Replacing the mounted owner can make the stale artifact's completed
        // entry budgeted again. Reclaim it before reporting cache ownership.
        enforceBudgetLocked()
        return layout
    }

    func releaseLease(
        for surface: FabricSurfaceToken,
        generationIdentity: String? = nil,
        leaseHandle: UInt64? = nil
    ) {
        condition.lock()
        let pending = matchingLeaseKeysLocked(
            in: pendingLeaseKeysBySurface[surface] ?? [],
            generationIdentity: generationIdentity,
            leaseHandle: leaseHandle
        )
        let mounted = matchingLeaseKeysLocked(
            in: mountedLeaseKeysBySurface[surface] ?? [],
            generationIdentity: generationIdentity,
            leaseHandle: leaseHandle
        )
        pending.forEach(removePendingLeaseLocked)
        mounted.forEach(removeMountedLeaseLocked)
        // Released mounted artifacts may still have completed entries. Those
        // entries re-enter the disposable budget on this transition.
        enforceBudgetLocked()
        condition.unlock()
    }

    /// Commits a replacement generation for one state-family handle. Pending
    /// G1 handoffs are stale immediately; mounted G1 remains visible until
    /// G2's exact pending handoff is acquired by the normal mount path.
    func activateFabricGeneration(
        surface: FabricSurfaceToken,
        generationIdentity: String,
        leaseHandle: UInt64
    ) {
        condition.lock()
        let owner = FabricLeaseOwner(surface: surface, leaseHandle: leaseHandle)
        for stale in pendingLeaseKeysByOwner[owner] ?? [] where stale.layout.generationIdentity != generationIdentity {
            removePendingLeaseLocked(stale)
        }
        retireUnownedPublicationKeysLocked()
        publishOwnerBytesLocked()
        condition.unlock()
    }

    /// Removes only the unmounted handoff requested by a stale Fabric mount
    /// callback. Mounted ownership is deliberately preserved: it may be the
    /// currently displayed width while another width is pending.
    ///
    /// Returns whether another lease for the same surface/generation remains,
    /// so the registry can retain its generation-scoped compiler pin.
    func releasePendingLease(
        for surface: FabricSurfaceToken,
        generationIdentity: String,
        widthPixels: Int,
        displayScale: CGFloat,
        leaseHandle: UInt64
    ) -> Bool {
        condition.lock()
        defer { condition.unlock() }

        let requested = ProseMountKey(
            generationIdentity: generationIdentity,
            widthPixels: widthPixels,
            displayScale: displayScale
        )
        let owner = FabricLeaseOwner(surface: surface, leaseHandle: leaseHandle)
        if let leaseKey = pendingLeaseKeysByOwner[owner]?.first,
           mountKey(for: leaseKey.layout) == requested {
            removePendingLeaseLocked(leaseKey)
            retireUnownedPublicationKeysLocked()
            publishOwnerBytesLocked()
        }
        return (pendingLeaseKeysByOwner[owner] ?? []).contains { $0.layout.generationIdentity == generationIdentity }
            || (mountedLeaseKeysByOwner[owner] ?? []).contains { $0.layout.generationIdentity == generationIdentity }
    }

    /// Surface shutdown must release mounted artifacts even after memory
    /// pressure has already cleared registry compiler ownership. Return the
    /// exact lease identities before mutating this bounded cache.
    func fabricGenerations(for surface: FabricSurfaceToken) -> Set<FabricGenerationToken> {
        condition.lock()
        defer { condition.unlock() }
        let leaseKeys = (pendingLeaseKeysBySurface[surface] ?? [])
            .union(mountedLeaseKeysBySurface[surface] ?? [])
        return Set(leaseKeys.map {
                FabricGenerationToken(
                    surface: $0.surface,
                    generationIdentity: $0.layout.generationIdentity,
                    leaseHandle: $0.leaseHandle
                )
            })
    }

    func registerDirectMount(_ owner: String, layout: PreparedProseLayout) {
        condition.lock()
        insertDirectMountLocked(layout, for: owner)
        // Replacing an owner can return its prior completed entry to the
        // disposable budget even as the new artifact becomes direct-mounted.
        enforceBudgetLocked()
        condition.unlock()
    }

    func releaseDirectMount(_ owner: String) {
        condition.lock()
        removeDirectMountLocked(owner)
        // A direct mount is excluded from the disposable-cache budget while
        // it owns the artifact. Once released, its completed entry becomes
        // unmounted again and must be evicted before telemetry observes it.
        enforceBudgetLocked()
        condition.unlock()
    }

    func removeAllUnmounted() {
        condition.lock()
        while let key = completed.keys.first { removeCompletedLocked(key) }
        accessOrder.removeAll()
        accessGenerations.removeAll()
        accessOrderHead = 0
        mountIndex.removeAll()
        while let key = pendingLeases.keys.first { removePendingLeaseLocked(key) }
        // Pending handoffs are unmounted owners, so memory pressure retires
        // them with the completed cache. Fabric/direct mounted owners survive
        // until their explicit release paths run. Registry clears compiled
        // documents only after this unmounted cache step.
        retireUnownedPublicationKeysLocked()
        publishOwnerBytesLocked()
        condition.unlock()
    }

    var countForTesting: Int {
        condition.lock()
        defer { condition.unlock() }
        return completed.count
    }

    /// Benchmark reverse passes may not rebind a one-item or already-visible
    /// cell. Seed only those passes with the exact live census from the
    /// completed forward pass, then retain the normal end-of-pass liveness
    /// intersection below.
    func beginBenchmarkCensus(seeding keys: [ProseLayoutKey] = []) {
        condition.lock()
        benchmarkCensusKeys = Set(keys)
        condition.unlock()
    }

    func endBenchmarkCensus() -> [ProseLayoutKey] {
        condition.lock()
        defer { condition.unlock() }
        // A census records keys observed during a benchmark pass, but it is
        // evidence of resident artifacts only. A lookup can fail, produce an
        // oversized unowned artifact, or evict an earlier completed entry;
        // none of those keys is resident when the pass ends. Pending and
        // mounted Fabric leases plus direct UIKit mounts are genuine live
        // owners even when the disposable completed LRU no longer has them.
        let liveKeys = Set(completed.keys)
            .union(pendingLeaseKeysByLayout.keys)
            .union(mountedLeaseKeysByLayout.keys)
            .union(directOwnersByLayout.keys)
        let keys = benchmarkCensusKeys.map { Array($0.intersection(liveKeys)) } ?? []
        benchmarkCensusKeys = nil
        return keys
    }

    var retainedBytesForTesting: Int {
        condition.lock()
        defer { condition.unlock() }
        return retainedBytes
    }

    var oversizedLeaseCountForTesting: Int {
        condition.lock()
        defer { condition.unlock() }
        return oversizedLeaseCount
    }

    var pendingLeaseCountForTesting: Int {
        condition.lock()
        defer { condition.unlock() }
        return pendingLeases.count
    }

    var mountedLeaseCountForTesting: Int {
        condition.lock()
        defer { condition.unlock() }
        return mountedLeases.count
    }

    var leaseCountForTesting: Int {
        condition.lock()
        defer { condition.unlock() }
        return pendingLeases.count + mountedLeases.count
    }

    var accessOrderTokenCountForTesting: Int {
        condition.lock()
        defer { condition.unlock() }
        return accessOrder.count
    }

    private func createPendingLeaseLocked(
        _ layout: PreparedProseLayout,
        for key: ProseLayoutKey,
        surface: FabricSurfaceToken,
        leaseHandle: UInt64
    ) {
        let leaseKey = FabricLeaseKey(surface: surface, layout: key, leaseHandle: leaseHandle)
        // A new measurement supersedes only unmounted handoffs for this
        // Fabric surface and generation. Keep a mounted artifact alive until
        // the replacement is actually acquired by a component view.
        let owner = FabricLeaseOwner(surface: surface, leaseHandle: leaseHandle)
        for stale in pendingLeaseKeysByOwner[owner] ?? [] where stale != leaseKey {
            removePendingLeaseLocked(stale)
        }

        // Repeated Yoga measurements for an already mounted identity neither
        // replace that mounted artifact nor manufacture another handoff.
        guard mountedLeases[leaseKey] == nil else {
            retireUnownedPublicationKeysLocked()
            publishOwnerBytesLocked()
            return
        }
        insertPendingLeaseLocked(layout, for: leaseKey)
        enforceBudgetLocked()
    }

    private func removePendingLeaseLocked(_ key: FabricLeaseKey) {
        guard let layout = pendingLeases.removeValue(forKey: key) else { return }
        removeLeaseKey(key, from: &pendingLeaseKeysByOwner, key: FabricLeaseOwner(surface: key.surface, leaseHandle: key.leaseHandle))
        removeLeaseKey(key, from: &pendingLeaseKeysBySurface, key: key.surface)
        removeLeaseKey(key, from: &pendingLeaseKeysByLayout, key: key.layout)
        if layout.retainedBytes > byteBudget { oversizedLeaseCount -= 1 }
        updateOwnershipLocked(layout, pendingDelta: -1, publicationKey: key.layout)
    }

    private func touch(_ key: ProseLayoutKey) {
        guard completed[key] != nil else { return }
        precondition(nextAccessGeneration < UInt64.max, "Prepared prose LRU generation overflowed.")
        nextAccessGeneration += 1
        accessGenerations[key] = nextAccessGeneration
        accessOrder.append((key, nextAccessGeneration))
        compactAccessOrderIfNeededLocked()
    }

    private func mountedLayoutLocked(
        for key: ProseLayoutKey,
        fabricSurface: FabricSurfaceToken?,
        fabricLeaseHandle: UInt64?
    ) -> PreparedProseLayout? {
        if let fabricSurface {
            // Fabric handoffs are owner-isolated: an artifact mounted by one
            // surface must never satisfy another surface's measurement.
            guard let fabricLeaseHandle else { return nil }
            return mountedLeases[FabricLeaseKey(
                surface: fabricSurface,
                layout: key,
                leaseHandle: fabricLeaseHandle
            )]
        }
        // UIKit has no Fabric lease owner. A direct mounted view is still an
        // exact immutable artifact for this semantic/physical key and remains
        // valid after unmounted cache eviction.
        guard let owner = directOwnersByLayout[key]?.first else { return nil }
        return directMounted[owner]
    }

    /// Lookup after same-owner mounted and completed entries. Pending, mounted,
    /// and direct owners all retain the same immutable artifact contract even
    /// if the completed cache was evicted or cleared under memory pressure.
    private func liveLayoutLocked(for key: ProseLayoutKey) -> PreparedProseLayout? {
        if let leaseKey = pendingLeaseKeysByLayout[key]?.first, let layout = pendingLeases[leaseKey] {
            return layout
        }
        if let leaseKey = mountedLeaseKeysByLayout[key]?.first, let layout = mountedLeases[leaseKey] {
            return layout
        }
        if let owner = directOwnersByLayout[key]?.first {
            return directMounted[owner]
        }
        return nil
    }

    private func mountKey(for layoutKey: ProseLayoutKey) -> ProseMountKey {
        ProseMountKey(generationIdentity: layoutKey.generationIdentity, widthPixels: layoutKey.widthPixels, displayScaleBits: layoutKey.displayScaleBits)
    }

    private func removeCompletedLocked(_ key: ProseLayoutKey) {
        guard let layout = completed.removeValue(forKey: key) else { return }
        accessGenerations.removeValue(forKey: key)
        let mountKey = mountKey(for: key)
        if mountIndex[mountKey] == key { mountIndex.removeValue(forKey: mountKey) }
        updateOwnershipLocked(layout, completedDelta: -1, publicationKey: key)
    }

    private func enforceBudgetLocked() {
        // Completed entries are a disposable LRU. A pending Yoga-to-Fabric
        // handoff is an exact active owner: evicting it would make Fabric's
        // later mount miss without any completed-cache fallback. Its lifecycle
        // release, activation replacement, or explicit memory warning owns
        // retirement instead of ordinary byte pressure.
        while budgetedRetainedBytes > byteBudget {
            if let oldest = oldestCompletedKeyLocked() {
                removeCompletedLocked(oldest)
                continue
            }
            break
        }
        retireUnownedPublicationKeysLocked()
        publishOwnerBytesLocked()
    }

    private func retireUnownedPublicationKeysLocked() {
#if DEBUG
        for key in publicationRetirementCandidates where livePublicationCounts[key, default: 0] == 0 {
            publishedKeys.remove(key)
        }
        publicationRetirementCandidates.removeAll()
#endif
    }

    private func publishOwnerBytesLocked() {
        PreparedProseInstrumentation.cacheUpdated(
            unmountedBytes: unmountedRetainedBytes,
            unmountedResidentCount: completed.count
        )
        PreparedProseInstrumentation.retained(.unmountedLayout, scope: "cache", bytes: unmountedRetainedBytes)
        PreparedProseInstrumentation.retained(.fabricLeaseHandoff, scope: "leases", bytes: fabricLeaseRetainedBytes)
        PreparedProseInstrumentation.retained(.directMounted, scope: "views", bytes: directMountedRetainedBytes)
    }

    private func insertCompletedLocked(_ layout: PreparedProseLayout, for key: ProseLayoutKey) {
        if completed[key] != nil {
            removeCompletedLocked(key)
        }
        completed[key] = layout
        mountIndex[mountKey(for: key)] = key
        updateOwnershipLocked(layout, completedDelta: 1, publicationKey: key)
        touch(key)
    }

    private func insertPendingLeaseLocked(_ layout: PreparedProseLayout, for key: FabricLeaseKey) {
        removePendingLeaseLocked(key)
        pendingLeases[key] = layout
        insertLeaseKey(key, into: &pendingLeaseKeysByOwner, key: FabricLeaseOwner(surface: key.surface, leaseHandle: key.leaseHandle))
        insertLeaseKey(key, into: &pendingLeaseKeysBySurface, key: key.surface)
        insertLeaseKey(key, into: &pendingLeaseKeysByLayout, key: key.layout)
        if layout.retainedBytes > byteBudget { oversizedLeaseCount += 1 }
        updateOwnershipLocked(layout, pendingDelta: 1, publicationKey: key.layout)
    }

    private func insertMountedLeaseLocked(_ layout: PreparedProseLayout, for key: FabricLeaseKey) {
        removeMountedLeaseLocked(key)
        mountedLeases[key] = layout
        insertLeaseKey(key, into: &mountedLeaseKeysByOwner, key: FabricLeaseOwner(surface: key.surface, leaseHandle: key.leaseHandle))
        insertLeaseKey(key, into: &mountedLeaseKeysBySurface, key: key.surface)
        insertLeaseKey(key, into: &mountedLeaseKeysByLayout, key: key.layout)
        if layout.retainedBytes > byteBudget { oversizedLeaseCount += 1 }
        updateOwnershipLocked(layout, mountedDelta: 1, publicationKey: key.layout)
    }

    private func removeMountedLeaseLocked(_ key: FabricLeaseKey) {
        guard let layout = mountedLeases.removeValue(forKey: key) else { return }
        removeLeaseKey(key, from: &mountedLeaseKeysByOwner, key: FabricLeaseOwner(surface: key.surface, leaseHandle: key.leaseHandle))
        removeLeaseKey(key, from: &mountedLeaseKeysBySurface, key: key.surface)
        removeLeaseKey(key, from: &mountedLeaseKeysByLayout, key: key.layout)
        if layout.retainedBytes > byteBudget { oversizedLeaseCount -= 1 }
        updateOwnershipLocked(layout, mountedDelta: -1, publicationKey: key.layout)
    }

    private func insertDirectMountLocked(_ layout: PreparedProseLayout, for owner: String) {
        removeDirectMountLocked(owner)
        directMounted[owner] = layout
        directOwnersByLayout[layout.key, default: []].insert(owner)
        updateOwnershipLocked(layout, directDelta: 1, publicationKey: layout.key)
    }

    private func removeDirectMountLocked(_ owner: String) {
        guard let layout = directMounted.removeValue(forKey: owner) else { return }
        directOwnersByLayout[layout.key]?.remove(owner)
        if directOwnersByLayout[layout.key]?.isEmpty == true { directOwnersByLayout.removeValue(forKey: layout.key) }
        updateOwnershipLocked(layout, directDelta: -1, publicationKey: layout.key)
    }

    private func insertLeaseKey<Index: Hashable>(
        _ key: FabricLeaseKey,
        into table: inout [Index: Set<FabricLeaseKey>],
        key indexKey: Index
    ) {
        table[indexKey, default: []].insert(key)
    }

    private func removeLeaseKey<Index: Hashable>(
        _ key: FabricLeaseKey,
        from table: inout [Index: Set<FabricLeaseKey>],
        key indexKey: Index
    ) {
        table[indexKey]?.remove(key)
        if table[indexKey]?.isEmpty == true { table.removeValue(forKey: indexKey) }
    }

    private func matchingLeaseKeysLocked(
        in keys: Set<FabricLeaseKey>,
        generationIdentity: String?,
        leaseHandle: UInt64?
    ) -> [FabricLeaseKey] {
        keys.filter {
            (generationIdentity == nil || $0.layout.generationIdentity == generationIdentity) &&
                (leaseHandle == nil || $0.leaseHandle == leaseHandle)
        }
    }

    private func oldestCompletedKeyLocked() -> ProseLayoutKey? {
        var skippedPinnedEntry = false
        while accessOrderHead < accessOrder.count {
            let token = accessOrder[accessOrderHead]
            accessOrderHead += 1
            guard accessGenerations[token.key] == token.generation,
                  let layout = completed[token.key],
                  let ownership = ownershipByIdentifier[ObjectIdentifier(layout)]
            else { continue }

            if ownership.pendingReferences == 0,
               ownership.mountedReferences == 0,
               ownership.directReferences == 0 {
                // A pinned entry ahead of this candidate still needs its LRU
                // token when it later returns to the budget. Rebuild only on
                // this cold eviction path to preserve that ordering.
                if skippedPinnedEntry {
                    rebuildAccessOrderLocked(excluding: token)
                } else {
                    compactAccessOrderIfNeededLocked()
                }
                return token.key
            }

            skippedPinnedEntry = true
        }
        // Every remaining completed entry is pinned. Keep their current LRU
        // tokens so a later release can make one evictable without a new hit.
        rebuildAccessOrderLocked()
        return nil
    }

    private func rebuildAccessOrderLocked(
        excluding excluded: (key: ProseLayoutKey, generation: UInt64)? = nil
    ) {
        accessOrder = accessOrder.filter { token in
            guard accessGenerations[token.key] == token.generation, completed[token.key] != nil else {
                return false
            }
            return excluded.map { $0.key != token.key || $0.generation != token.generation } ?? true
        }
        accessOrderHead = 0
    }

    private func compactAccessOrderIfNeededLocked() {
        let liveTokenCount = accessOrder.count - accessOrderHead
        let compactionThreshold = max(64, completed.count * 3)
        guard liveTokenCount > compactionThreshold || accessOrderHead > compactionThreshold else { return }
        accessOrder = accessOrder[accessOrderHead...].filter { token in
            accessGenerations[token.key] == token.generation && completed[token.key] != nil
        }
        accessOrderHead = 0
    }

    private func updateOwnershipLocked(
        _ layout: PreparedProseLayout,
        completedDelta: Int = 0,
        pendingDelta: Int = 0,
        mountedDelta: Int = 0,
        directDelta: Int = 0,
        publicationKey: ProseLayoutKey
    ) {
        let identifier = ObjectIdentifier(layout)
        var ownership = ownershipByIdentifier[identifier] ?? LayoutOwnership(layout: layout)
        let previous = ownership
        ownership.completedReferences += completedDelta
        ownership.pendingReferences += pendingDelta
        ownership.mountedReferences += mountedDelta
        ownership.directReferences += directDelta
        precondition(
            ownership.completedReferences >= 0 && ownership.pendingReferences >= 0 &&
                ownership.mountedReferences >= 0 && ownership.directReferences >= 0,
            "Prepared prose ownership references must not underflow."
        )
        applyOwnershipContributionDelta(from: previous, to: ownership)
        if ownership.completedReferences + ownership.pendingReferences + ownership.mountedReferences + ownership.directReferences == 0 {
            ownershipByIdentifier.removeValue(forKey: identifier)
        } else {
            ownershipByIdentifier[identifier] = ownership
        }
#if DEBUG
        let previousLiveCount = livePublicationCounts[publicationKey, default: 0]
        let nextLiveCount = previousLiveCount + completedDelta + pendingDelta + mountedDelta + directDelta
        precondition(nextLiveCount >= 0, "Prepared prose publication references must not underflow.")
        if nextLiveCount == 0 {
            livePublicationCounts.removeValue(forKey: publicationKey)
            publicationRetirementCandidates.insert(publicationKey)
        } else {
            livePublicationCounts[publicationKey] = nextLiveCount
            publicationRetirementCandidates.remove(publicationKey)
        }
#endif
    }

    private func applyOwnershipContributionDelta(from previous: LayoutOwnership, to next: LayoutOwnership) {
        let bytes = next.layout.retainedBytes
        retainedBytes += contribution(next, bytes: bytes, kind: .retained) - contribution(previous, bytes: bytes, kind: .retained)
        budgetedRetainedBytes += contribution(next, bytes: bytes, kind: .budgeted) - contribution(previous, bytes: bytes, kind: .budgeted)
        unmountedRetainedBytes += contribution(next, bytes: bytes, kind: .unmounted) - contribution(previous, bytes: bytes, kind: .unmounted)
        fabricLeaseRetainedBytes += contribution(next, bytes: bytes, kind: .fabricLease) - contribution(previous, bytes: bytes, kind: .fabricLease)
        directMountedRetainedBytes += contribution(next, bytes: bytes, kind: .directMounted) - contribution(previous, bytes: bytes, kind: .directMounted)
    }

    private enum OwnershipContribution {
        case retained
        case budgeted
        case unmounted
        case fabricLease
        case directMounted
    }

    private func contribution(_ ownership: LayoutOwnership, bytes: Int, kind: OwnershipContribution) -> Int {
        switch kind {
        case .retained:
            return ownership.completedReferences + ownership.pendingReferences + ownership.mountedReferences + ownership.directReferences > 0 ? bytes : 0
        case .budgeted:
            return ownership.completedReferences + ownership.pendingReferences > 0 &&
                ownership.mountedReferences + ownership.directReferences == 0 ? bytes : 0
        case .unmounted:
            return ownership.completedReferences > 0 && ownership.pendingReferences == 0 &&
                ownership.mountedReferences == 0 && ownership.directReferences == 0 ? bytes : 0
        case .fabricLease:
            return ownership.pendingReferences + ownership.mountedReferences > 0 && ownership.directReferences == 0 ? bytes : 0
        case .directMounted:
            return ownership.directReferences > 0 ? bytes : 0
        }
    }
}
