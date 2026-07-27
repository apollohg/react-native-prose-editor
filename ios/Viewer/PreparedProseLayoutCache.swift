import Foundation

/// Owns unmounted layouts and the bounded Yoga-to-Fabric handoff. A lease
/// shares its immutable artifact with the LRU; bytes are counted once across
/// both owners so handoff cannot silently bypass the cache budget.
final class PreparedProseLayoutCache {
    private final class Preparation {
        var result: Result<PreparedProseLayout, Error>?
    }

    private let condition = NSCondition()
    private var completed: [ProseLayoutKey: PreparedProseLayout] = [:]
    private var accessOrder: [ProseLayoutKey] = []
    private var inFlight: [ProseLayoutKey: Preparation] = [:]
    /// Yoga measurement hands an immutable artifact to Fabric before there is
    /// a component view. A pending handoff is therefore distinct from a
    /// mounted owner: mount consumes it exactly once and only the mounted
    /// owner is non-evictable.
    private var pendingLeases: [FabricLeaseKey: PreparedProseLayout] = [:]
    private var pendingLeaseAccessOrder: [FabricLeaseKey] = []
    private var mountedLeases: [FabricLeaseKey: PreparedProseLayout] = [:]
    private var directMounted: [String: PreparedProseLayout] = [:]
    private var mountIndex: [ProseMountKey: ProseLayoutKey] = [:]
#if DEBUG
    /// A live generation must publish an artifact once for its complete
    /// semantic/physical-width/revision key. Eviction retires only unmounted
    /// entries, so a mounted owner can never be republished accidentally.
    private var publishedKeys: Set<ProseLayoutKey> = []
#endif
    private let byteBudget: Int

    init(byteBudget: Int = 32 * 1024 * 1024) {
        self.byteBudget = byteBudget
    }

    func value(
        for key: ProseLayoutKey,
        fabricSurface: FabricSurfaceToken? = nil,
        build: () throws -> PreparedProseLayout
    ) throws -> PreparedProseLayout {
        let lookupStarted = PreparedProseInstrumentation.now()
        condition.lock()
        // Mounted ownership is the most authoritative exact-key cache entry:
        // it survives unmounted-cache eviction and memory warnings. Consult it
        // before completed entries, because creating a same-width pending lease
        // would otherwise prune a different-width replacement already pending
        // for this Fabric surface and generation.
        if let layout = mountedLayoutLocked(for: key, fabricSurface: fabricSurface) {
            condition.unlock()
            PreparedProseInstrumentation.cacheLookup(lookupStarted, hit: true)
            return layout
        }
        if let layout = completed[key] {
            touch(key)
            if let fabricSurface { createPendingLeaseLocked(layout, for: key, surface: fabricSurface) }
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
            if let fabricSurface { createPendingLeaseLocked(layout, for: key, surface: fabricSurface) }
            condition.unlock()
            PreparedProseInstrumentation.cacheLookup(lookupStarted, hit: true)
            return layout
        }
        if let preparation = inFlight[key] {
            while preparation.result == nil { condition.wait() }
            let result = preparation.result!
            if case let .success(layout) = result, let fabricSurface {
                createPendingLeaseLocked(layout, for: key, surface: fabricSurface)
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
#endif
            if layout.retainedBytes <= byteBudget {
                completed[key] = layout
                mountIndex[mountKey(for: key)] = key
                touch(key)
            }
            if let fabricSurface {
                createPendingLeaseLocked(layout, for: key, surface: fabricSurface)
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
        displayScale: CGFloat
    ) -> PreparedProseLayout? {
        condition.lock()
        defer { condition.unlock() }

        let mountKey = ProseMountKey(
            generationIdentity: generationIdentity,
            widthPixels: widthPixels,
            displayScale: displayScale
        )
        // UIKit-only callers use the documented zero token because they have
        // no Fabric owner. They retain the ordinary completed-cache lookup;
        // every real Fabric surface must consume its own pending handoff.
        if surface.surfaceId == 0, surface.componentTag == 0,
           let key = mountIndex[mountKey], let layout = completed[key] {
            touch(key)
            return layout
        }
        guard let leaseKey = pendingLeases.keys.first(where: {
            $0.surface == surface &&
                $0.layout.generationIdentity == mountKey.generationIdentity &&
                $0.layout.widthPixels == mountKey.widthPixels &&
                $0.layout.displayScaleBits == mountKey.displayScaleBits
        }), let layout = pendingLeases[leaseKey] else { return nil }

        // A Fabric mount can only consume the Yoga handoff once. Do not fall
        // back to completed here: that would let a second/recycled component
        // mount an artifact that was never measured for its own owner. Move
        // the pending handoff to mounted ownership under this lock, retiring
        // only older widths for the same surface/generation in the same
        // critical section. A replacement that never reaches this point
        // therefore cannot disturb the currently mounted artifact.
        removePendingLeaseLocked(leaseKey)
        mountedLeases.keys
            .filter { sameFabricOwner($0, as: leaseKey) && $0 != leaseKey }
            .forEach { mountedLeases.removeValue(forKey: $0) }
        mountedLeases[leaseKey] = layout
        retireUnownedPublicationKeysLocked()
        publishOwnerBytesLocked()
        return layout
    }

    func releaseLease(for surface: FabricSurfaceToken, generationIdentity: String? = nil) {
        condition.lock()
        let pending = pendingLeases.keys.filter {
            $0.surface == surface && (generationIdentity == nil || $0.layout.generationIdentity == generationIdentity)
        }
        let mounted = mountedLeases.keys.filter {
            $0.surface == surface && (generationIdentity == nil || $0.layout.generationIdentity == generationIdentity)
        }
        pending.forEach(removePendingLeaseLocked)
        mounted.forEach { mountedLeases.removeValue(forKey: $0) }
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
        displayScale: CGFloat
    ) -> Bool {
        condition.lock()
        defer { condition.unlock() }

        let requested = ProseMountKey(
            generationIdentity: generationIdentity,
            widthPixels: widthPixels,
            displayScale: displayScale
        )
        if let leaseKey = pendingLeases.keys.first(where: {
            $0.surface == surface && mountKey(for: $0.layout) == requested
        }) {
            removePendingLeaseLocked(leaseKey)
            retireUnownedPublicationKeysLocked()
            publishOwnerBytesLocked()
        }
        return pendingLeases.keys.contains { $0.surface == surface && $0.layout.generationIdentity == generationIdentity }
            || mountedLeases.keys.contains { $0.surface == surface && $0.layout.generationIdentity == generationIdentity }
    }

    /// Used by the registry while holding its compiler/theme condition to
    /// decide whether a mount-miss may retire generation-scoped ownership.
    /// This is intentionally a query only; the exact pending cleanup above is
    /// always completed first.
    func hasLease(for surface: FabricSurfaceToken, generationIdentity: String) -> Bool {
        condition.lock()
        defer { condition.unlock() }
        return pendingLeases.keys.contains { $0.surface == surface && $0.layout.generationIdentity == generationIdentity }
            || mountedLeases.keys.contains { $0.surface == surface && $0.layout.generationIdentity == generationIdentity }
    }

    func registerDirectMount(_ owner: String, layout: PreparedProseLayout) {
        condition.lock(); directMounted[owner] = layout; retireUnownedPublicationKeysLocked(); publishOwnerBytesLocked(); condition.unlock()
    }

    func releaseDirectMount(_ owner: String) {
        condition.lock(); directMounted.removeValue(forKey: owner); retireUnownedPublicationKeysLocked(); publishOwnerBytesLocked(); condition.unlock()
    }

    func removeAllUnmounted() {
        condition.lock()
        completed.removeAll()
        accessOrder.removeAll()
        mountIndex.removeAll()
        pendingLeases.removeAll()
        pendingLeaseAccessOrder.removeAll()
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

    var retainedBytesForTesting: Int {
        condition.lock()
        defer { condition.unlock() }
        return retainedBytesLocked()
    }

    var oversizedLeaseCountForTesting: Int {
        condition.lock()
        defer { condition.unlock() }
        return (Array(pendingLeases.values) + Array(mountedLeases.values))
            .filter { $0.retainedBytes > byteBudget }
            .count
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

    private func createPendingLeaseLocked(
        _ layout: PreparedProseLayout,
        for key: ProseLayoutKey,
        surface: FabricSurfaceToken
    ) {
        let leaseKey = FabricLeaseKey(surface: surface, layout: key)
        // A new measurement supersedes only unmounted handoffs for this
        // Fabric surface and generation. Keep a mounted artifact alive until
        // the replacement is actually acquired by a component view.
        pendingLeases.keys
            .filter { sameFabricOwner($0, as: leaseKey) && $0 != leaseKey }
            .forEach(removePendingLeaseLocked)

        // Repeated Yoga measurements for an already mounted identity neither
        // replace that mounted artifact nor manufacture another handoff.
        guard mountedLeases[leaseKey] == nil else {
            retireUnownedPublicationKeysLocked()
            publishOwnerBytesLocked()
            return
        }
        pendingLeases[leaseKey] = layout
        touchPendingLease(leaseKey)
        enforceBudgetLocked(preferredPendingLease: leaseKey)
        publishOwnerBytesLocked()
    }

    private func removePendingLeaseLocked(_ key: FabricLeaseKey) {
        pendingLeases.removeValue(forKey: key)
        pendingLeaseAccessOrder.removeAll { $0 == key }
    }

    private func touch(_ key: ProseLayoutKey) {
        accessOrder.removeAll { $0 == key }
        accessOrder.append(key)
    }

    private func touchPendingLease(_ key: FabricLeaseKey) {
        pendingLeaseAccessOrder.removeAll { $0 == key }
        pendingLeaseAccessOrder.append(key)
    }

    private func sameFabricOwner(_ lhs: FabricLeaseKey, as rhs: FabricLeaseKey) -> Bool {
        lhs.surface == rhs.surface && lhs.layout.generationIdentity == rhs.layout.generationIdentity
    }

    private func mountedLayoutLocked(
        for key: ProseLayoutKey,
        fabricSurface: FabricSurfaceToken?
    ) -> PreparedProseLayout? {
        if let fabricSurface {
            // Fabric handoffs are owner-isolated: an artifact mounted by one
            // surface must never satisfy another surface's measurement.
            return mountedLeases[FabricLeaseKey(surface: fabricSurface, layout: key)]
        }
        // UIKit has no Fabric lease owner. A direct mounted view is still an
        // exact immutable artifact for this semantic/physical key and remains
        // valid after unmounted cache eviction.
        return directMounted.values.first { $0.key == key }
    }

    /// Lookup after same-owner mounted and completed entries. Pending, mounted,
    /// and direct owners all retain the same immutable artifact contract even
    /// if the completed cache was evicted or cleared under memory pressure.
    private func liveLayoutLocked(for key: ProseLayoutKey) -> PreparedProseLayout? {
        pendingLeases.values.first { $0.key == key }
            ?? mountedLeases.values.first { $0.key == key }
            ?? directMounted.values.first { $0.key == key }
    }

    private func mountKey(for layoutKey: ProseLayoutKey) -> ProseMountKey {
        ProseMountKey(generationIdentity: layoutKey.generationIdentity, widthPixels: layoutKey.widthPixels, displayScaleBits: layoutKey.displayScaleBits)
    }

    private func removeCompletedLocked(_ key: ProseLayoutKey) {
        completed.removeValue(forKey: key)
        accessOrder.removeAll { $0 == key }
        let mountKey = mountKey(for: key)
        if mountIndex[mountKey] == key { mountIndex.removeValue(forKey: mountKey) }
#if DEBUG
        retireUnownedPublicationKeysLocked()
#endif
    }

    private func enforceBudgetLocked(preferredPendingLease: FabricLeaseKey? = nil) {
        // Pending Yoga-to-Fabric handoffs are unmounted cache owners. They
        // share the same budget as completed entries; mounted owners never
        // enter this calculation and survive pressure until explicit release.
        while budgetedRetainedBytesLocked() > byteBudget {
            if let oldest = accessOrder.first {
                removeCompletedLocked(oldest)
                continue
            }
            if let oldest = pendingLeaseAccessOrder.first(where: {
                $0 != preferredPendingLease && pendingLeaseRemovalLowersBudgetedBytesLocked($0)
            }) {
                removePendingLeaseLocked(oldest)
                continue
            }
            break
        }
        retireUnownedPublicationKeysLocked()
        publishOwnerBytesLocked()
    }

    /// Removing a duplicate pending reference must not evict another
    /// surface's handoff: it does not lower retained bytes. This matters for
    /// oversized artifacts, which intentionally retain one immutable object
    /// shared by every live owner rather than rebuilding it per surface.
    private func pendingLeaseRemovalLowersBudgetedBytesLocked(_ leaseKey: FabricLeaseKey) -> Bool {
        guard let layout = pendingLeases[leaseKey] else { return false }
        let identifier = ObjectIdentifier(layout)
        let mountedIdentifiers = Set(
            (Array(mountedLeases.values) + Array(directMounted.values)).map(ObjectIdentifier.init)
        )
        guard !mountedIdentifiers.contains(identifier) else { return false }
        guard !completed.values.contains(where: { ObjectIdentifier($0) == identifier }) else { return false }
        return !pendingLeases.contains { candidate, value in
            candidate != leaseKey && ObjectIdentifier(value) == identifier
        }
    }

    /// Cache and lease references commonly point at the same immutable layout.
    /// Count each artifact once to reflect retained memory, not reference count.
    private func retainedBytesLocked() -> Int {
        var uniqueLayouts: [ObjectIdentifier: PreparedProseLayout] = [:]
        for layout in completed.values { uniqueLayouts[ObjectIdentifier(layout)] = layout }
        for layout in pendingLeases.values { uniqueLayouts[ObjectIdentifier(layout)] = layout }
        for layout in mountedLeases.values { uniqueLayouts[ObjectIdentifier(layout)] = layout }
        for layout in directMounted.values { uniqueLayouts[ObjectIdentifier(layout)] = layout }
        return uniqueLayouts.values.reduce(0) { $0 + $1.retainedBytes }
    }

    private func completedRetainedBytesLocked() -> Int {
        var uniqueLayouts: [ObjectIdentifier: PreparedProseLayout] = [:]
        let ownedIdentifiers = Set(
            (Array(pendingLeases.values) + Array(mountedLeases.values) + Array(directMounted.values))
                .map(ObjectIdentifier.init)
        )
        for layout in completed.values where !ownedIdentifiers.contains(ObjectIdentifier(layout)) {
            uniqueLayouts[ObjectIdentifier(layout)] = layout
        }
        return uniqueLayouts.values.reduce(0) { $0 + $1.retainedBytes }
    }

    private func budgetedRetainedBytesLocked() -> Int {
        var uniqueLayouts: [ObjectIdentifier: PreparedProseLayout] = [:]
        let mountedIdentifiers = Set(
            (Array(mountedLeases.values) + Array(directMounted.values)).map(ObjectIdentifier.init)
        )
        for layout in completed.values where !mountedIdentifiers.contains(ObjectIdentifier(layout)) {
            uniqueLayouts[ObjectIdentifier(layout)] = layout
        }
        for layout in pendingLeases.values where !mountedIdentifiers.contains(ObjectIdentifier(layout)) {
            uniqueLayouts[ObjectIdentifier(layout)] = layout
        }
        return uniqueLayouts.values.reduce(0) { $0 + $1.retainedBytes }
    }

    private func retireUnownedPublicationKeysLocked() {
#if DEBUG
        let live = Set(completed.keys)
            .union(pendingLeases.values.map(\.key))
            .union(mountedLeases.values.map(\.key))
            .union(directMounted.values.map(\.key))
        publishedKeys.formIntersection(live)
#endif
    }

    private func publishOwnerBytesLocked() {
        PreparedProseInstrumentation.retained(.unmountedLayout, scope: "cache", bytes: completedRetainedBytesLocked())
        let directLayouts = Array(directMounted.values)
        let fabricLayouts = Array(pendingLeases.values) + Array(mountedLeases.values)
        PreparedProseInstrumentation.retained(
            .fabricLeaseHandoff,
            scope: "leases",
            bytes: uniqueBytes(fabricLayouts, excluding: directLayouts)
        )
        PreparedProseInstrumentation.retained(.directMounted, scope: "views", bytes: uniqueBytes(directLayouts))
    }

    private func uniqueBytes(_ layouts: [PreparedProseLayout]) -> Int {
        var identifiers = Set<ObjectIdentifier>()
        return layouts.reduce(0) { total, layout in
            identifiers.insert(ObjectIdentifier(layout)).inserted ? total + layout.retainedBytes : total
        }
    }

    private func uniqueBytes(
        _ layouts: [PreparedProseLayout],
        excluding excluded: [PreparedProseLayout]
    ) -> Int {
        var identifiers = Set<ObjectIdentifier>()
        excluded.forEach { identifiers.insert(ObjectIdentifier($0)) }
        return layouts.reduce(0) { total, layout in
            identifiers.insert(ObjectIdentifier(layout)).inserted ? total + layout.retainedBytes : total
        }
    }
}
