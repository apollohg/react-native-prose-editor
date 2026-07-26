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
    private var leases: [FabricLeaseKey: PreparedProseLayout] = [:]
    private var leaseKeyBySurface: [FabricSurfaceToken: FabricLeaseKey] = [:]
    private var leaseAccessOrder: [FabricLeaseKey] = []
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
        if let layout = completed[key] {
            touch(key)
            if let fabricSurface { leaseLocked(layout, for: key, surface: fabricSurface) }
            condition.unlock()
            PreparedProseInstrumentation.cacheLookup(lookupStarted, hit: true)
            return layout
        }
        if let preparation = inFlight[key] {
            while preparation.result == nil { condition.wait() }
            let result = preparation.result!
            if case let .success(layout) = result, let fabricSurface {
                leaseLocked(layout, for: key, surface: fabricSurface)
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
                leaseLocked(layout, for: key, surface: fabricSurface)
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
        if let leaseKey = leaseKeyBySurface[surface],
           leaseKey.layout.generationIdentity == generationIdentity,
           leaseKey.layout.widthPixels == widthPixels,
           leaseKey.layout.displayScaleBits == Double(displayScale).bitPattern,
           let layout = leases[leaseKey] {
            return layout
        }
        guard let key = mountIndex[mountKey], let layout = completed[key] else { return nil }
        touch(key)
        return layout
    }

    func releaseLease(for surface: FabricSurfaceToken, generationIdentity: String? = nil) {
        condition.lock()
        if generationIdentity == nil || leaseKeyBySurface[surface]?.layout.generationIdentity == generationIdentity {
            releaseLeaseLocked(for: surface)
        }
        condition.unlock()
    }

    func registerDirectMount(_ owner: String, layout: PreparedProseLayout) {
        condition.lock(); directMounted[owner] = layout; publishOwnerBytesLocked(); condition.unlock()
    }

    func releaseDirectMount(_ owner: String) {
        condition.lock(); directMounted.removeValue(forKey: owner); retireUnownedPublicationKeysLocked(); publishOwnerBytesLocked(); condition.unlock()
    }

    func removeAllUnmounted() {
        condition.lock()
        completed.removeAll()
        accessOrder.removeAll()
        mountIndex.removeAll()
        // Fabric/direct mounted owners survive pressure. Registry clears
        // compiled documents only after this unmounted cache step.
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
        return leases.values.filter { $0.retainedBytes > byteBudget }.count
    }

    private func leaseLocked(
        _ layout: PreparedProseLayout,
        for key: ProseLayoutKey,
        surface: FabricSurfaceToken
    ) {
        releaseLeaseLocked(for: surface)
        let leaseKey = FabricLeaseKey(surface: surface, layout: key)
        leases[leaseKey] = layout
        leaseKeyBySurface[surface] = leaseKey
        touchLease(leaseKey)
        enforceBudgetLocked(preferredLease: leaseKey)
    }

    private func releaseLeaseLocked(for surface: FabricSurfaceToken) {
        guard let leaseKey = leaseKeyBySurface.removeValue(forKey: surface) else { return }
        leases.removeValue(forKey: leaseKey)
        leaseAccessOrder.removeAll { $0 == leaseKey }
    }

    private func touch(_ key: ProseLayoutKey) {
        accessOrder.removeAll { $0 == key }
        accessOrder.append(key)
    }

    private func touchLease(_ key: FabricLeaseKey) {
        leaseAccessOrder.removeAll { $0 == key }
        leaseAccessOrder.append(key)
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

    private func removeLeaseLocked(_ key: FabricLeaseKey) {
        leases.removeValue(forKey: key)
        leaseAccessOrder.removeAll { $0 == key }
        if leaseKeyBySurface[key.surface] == key {
            leaseKeyBySurface.removeValue(forKey: key.surface)
        }
    }

    private func enforceBudgetLocked(preferredLease: FabricLeaseKey? = nil) {
        // Leases are handoffs to a mounted owner, not unmounted cache entries.
        // The cache budget therefore evicts only completed LRU artifacts; a
        // mounted artifact is never evicted by pressure or entry churn.
        while completedRetainedBytesLocked() > byteBudget {
            if let oldest = accessOrder.first {
                removeCompletedLocked(oldest)
                continue
            }
            break
        }
        retireUnownedPublicationKeysLocked()
        publishOwnerBytesLocked()
    }

    /// Cache and lease references commonly point at the same immutable layout.
    /// Count each artifact once to reflect retained memory, not reference count.
    private func retainedBytesLocked() -> Int {
        var uniqueLayouts: [ObjectIdentifier: PreparedProseLayout] = [:]
        for layout in completed.values { uniqueLayouts[ObjectIdentifier(layout)] = layout }
        for layout in leases.values { uniqueLayouts[ObjectIdentifier(layout)] = layout }
        for layout in directMounted.values { uniqueLayouts[ObjectIdentifier(layout)] = layout }
        return uniqueLayouts.values.reduce(0) { $0 + $1.retainedBytes }
    }

    private func completedRetainedBytesLocked() -> Int {
        var uniqueLayouts: [ObjectIdentifier: PreparedProseLayout] = [:]
        let mountedKeys = Set(leases.values.map(\.key)).union(directMounted.values.map(\.key))
        for layout in completed.values where !mountedKeys.contains(layout.key) {
            uniqueLayouts[ObjectIdentifier(layout)] = layout
        }
        return uniqueLayouts.values.reduce(0) { $0 + $1.retainedBytes }
    }

    private func retireUnownedPublicationKeysLocked() {
#if DEBUG
        let live = Set(completed.keys).union(leases.values.map(\.key)).union(directMounted.values.map(\.key))
        publishedKeys.formIntersection(live)
#endif
    }

    private func publishOwnerBytesLocked() {
        PreparedProseInstrumentation.retained(.unmountedLayout, scope: "cache", bytes: completedRetainedBytesLocked())
        PreparedProseInstrumentation.retained(.fabricLeaseHandoff, scope: "leases", bytes: uniqueBytes(leases.values))
        PreparedProseInstrumentation.retained(.directMounted, scope: "views", bytes: uniqueBytes(directMounted.values))
    }

    private func uniqueBytes(_ layouts: Dictionary<FabricLeaseKey, PreparedProseLayout>.Values) -> Int {
        Dictionary(uniqueKeysWithValues: layouts.map { (ObjectIdentifier($0), $0) }).values.reduce(0) { $0 + $1.retainedBytes }
    }

    private func uniqueBytes(_ layouts: Dictionary<String, PreparedProseLayout>.Values) -> Int {
        Dictionary(uniqueKeysWithValues: layouts.map { (ObjectIdentifier($0), $0) }).values.reduce(0) { $0 + $1.retainedBytes }
    }
}
