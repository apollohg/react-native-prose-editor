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
    private var mountIndex: [ProseMountKey: ProseLayoutKey] = [:]
    private let byteBudget: Int
    private let entryBudget: Int
    private let leaseBudget: Int

    init(
        byteBudget: Int = 32 * 1024 * 1024,
        entryBudget: Int = 512,
        leaseBudget: Int = 32
    ) {
        self.byteBudget = byteBudget
        self.entryBudget = entryBudget
        self.leaseBudget = leaseBudget
    }

    func value(
        for key: ProseLayoutKey,
        fabricSurface: FabricSurfaceToken? = nil,
        build: () throws -> PreparedProseLayout
    ) throws -> PreparedProseLayout {
        condition.lock()
        if let layout = completed[key] {
            touch(key)
            if let fabricSurface { leaseLocked(layout, for: key, surface: fabricSurface) }
            condition.unlock()
            return layout
        }
        if let preparation = inFlight[key] {
            while preparation.result == nil { condition.wait() }
            let result = preparation.result!
            if case let .success(layout) = result, let fabricSurface {
                leaseLocked(layout, for: key, surface: fabricSurface)
            }
            condition.unlock()
            return try result.get()
        }
        let preparation = Preparation()
        inFlight[key] = preparation
        condition.unlock()

        let result = Result(catching: build)

        condition.lock()
        if case let .success(layout) = result {
            completed[key] = layout
            mountIndex[mountKey(for: key)] = key
            touch(key)
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
           let layout = leases.removeValue(forKey: leaseKey) {
            leaseKeyBySurface.removeValue(forKey: surface)
            leaseAccessOrder.removeAll { $0 == leaseKey }
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

    func removeAllUnmounted() {
        condition.lock()
        completed.removeAll()
        accessOrder.removeAll()
        leases.removeAll()
        leaseKeyBySurface.removeAll()
        leaseAccessOrder.removeAll()
        mountIndex.removeAll()
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
    }

    private func removeLeaseLocked(_ key: FabricLeaseKey) {
        leases.removeValue(forKey: key)
        leaseAccessOrder.removeAll { $0 == key }
        if leaseKeyBySurface[key.surface] == key {
            leaseKeyBySurface.removeValue(forKey: key.surface)
        }
    }

    private func enforceBudgetLocked(preferredLease: FabricLeaseKey? = nil) {
        while completed.count > entryBudget, let oldest = accessOrder.first {
            removeCompletedLocked(oldest)
        }

        // An oversize handoff is permitted only for the newest measurement and
        // only after every older lease has been deterministically evicted.
        if let preferredLease,
           let layout = leases[preferredLease],
           layout.retainedBytes > byteBudget {
            for key in leaseAccessOrder where key != preferredLease {
                removeLeaseLocked(key)
            }
            for key in Array(completed.keys) where key != preferredLease.layout {
                removeCompletedLocked(key)
            }
            removeCompletedLocked(preferredLease.layout)
            return
        }

        while leases.count > leaseBudget, let oldest = leaseAccessOrder.first {
            removeLeaseLocked(oldest)
        }
        while retainedBytesLocked() > byteBudget {
            if let oldest = accessOrder.first {
                removeCompletedLocked(oldest)
                continue
            }
            guard let oldestLease = leaseAccessOrder.first else { break }
            removeLeaseLocked(oldestLease)
        }
    }

    /// Cache and lease references commonly point at the same immutable layout.
    /// Count each artifact once to reflect retained memory, not reference count.
    private func retainedBytesLocked() -> Int {
        var uniqueLayouts: [ObjectIdentifier: PreparedProseLayout] = [:]
        for layout in completed.values { uniqueLayouts[ObjectIdentifier(layout)] = layout }
        for layout in leases.values { uniqueLayouts[ObjectIdentifier(layout)] = layout }
        return uniqueLayouts.values.reduce(0) { $0 + $1.retainedBytes }
    }
}
