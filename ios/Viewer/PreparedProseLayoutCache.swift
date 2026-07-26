import Foundation

/// A cache that admits exactly one completed immutable artifact for each key.
final class PreparedProseLayoutCache {
    private final class Preparation {
        var result: Result<PreparedProseLayout, Error>?
    }

    private let condition = NSCondition()
    private var completed: [ProseLayoutKey: PreparedProseLayout] = [:]
    private var accessOrder: [ProseLayoutKey] = []
    private var inFlight: [ProseLayoutKey: Preparation] = [:]
    private var leases: [ProseLayoutKey: PreparedProseLayout] = [:]
    private var leasedKeyByGeneration: [String: ProseLayoutKey] = [:]
    private var leaseAccessOrder: [ProseLayoutKey] = []
    private var retainedBytes = 0
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
        retainForFabricMount: Bool = false,
        build: () throws -> PreparedProseLayout
    ) throws -> PreparedProseLayout {
        condition.lock()
        if let layout = completed[key] {
            touch(key)
            if retainForFabricMount {
                leaseLocked(layout, for: key)
            }
            condition.unlock()
            return layout
        }
        if let preparation = inFlight[key] {
            while preparation.result == nil { condition.wait() }
            let result = preparation.result!
            if case let .success(layout) = result, retainForFabricMount {
                leaseLocked(layout, for: key)
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
            retainedBytes += layout.retainedBytes
            touch(key)
            if retainForFabricMount {
                leaseLocked(layout, for: key)
            }
            trimToBudget()
        }
        preparation.result = result
        inFlight.removeValue(forKey: key)
        condition.broadcast()
        condition.unlock()
        return try result.get()
    }

    func cachedValue(for key: ProseLayoutKey) -> PreparedProseLayout? {
        condition.lock()
        defer { condition.unlock() }
        guard let layout = completed[key] else { return nil }
        touch(key)
        return layout
    }

    /// Holds the latest Yoga result for a generation until Fabric consumes it.
    /// Leases deliberately sit outside the LRU so a single oversized artifact
    /// survives long enough to be mounted, but their count is bounded.
    private func leaseLocked(_ layout: PreparedProseLayout, for key: ProseLayoutKey) {
        releaseLease(forGeneration: key.generationIdentity)
        leases[key] = layout
        leasedKeyByGeneration[key.generationIdentity] = key
        touchLease(key)
        trimLeasesToBudget()
    }

    func acquireLeasedValue(
        generationIdentity: String,
        widthPixels: Int,
        displayScale: CGFloat
    ) -> PreparedProseLayout? {
        condition.lock()
        defer { condition.unlock() }
        guard let key = leasedKeyByGeneration[generationIdentity],
              key.widthPixels == widthPixels,
              key.displayScaleBits == Double(displayScale).bitPattern,
              let layout = leases.removeValue(forKey: key)
        else {
            return nil
        }
        leasedKeyByGeneration.removeValue(forKey: generationIdentity)
        leaseAccessOrder.removeAll { $0 == key }
        return layout
    }

    func removeAllUnmounted() {
        condition.lock()
        completed.removeAll()
        accessOrder.removeAll()
        leases.removeAll()
        leasedKeyByGeneration.removeAll()
        leaseAccessOrder.removeAll()
        retainedBytes = 0
        condition.unlock()
    }

    var countForTesting: Int {
        condition.lock()
        defer { condition.unlock() }
        return completed.count
    }

    private func touch(_ key: ProseLayoutKey) {
        accessOrder.removeAll { $0 == key }
        accessOrder.append(key)
    }

    private func trimToBudget() {
        while (retainedBytes > byteBudget || completed.count > entryBudget), let oldest = accessOrder.first {
            accessOrder.removeFirst()
            if let removed = completed.removeValue(forKey: oldest) {
                retainedBytes -= removed.retainedBytes
            }
        }
    }

    private func releaseLease(forGeneration generationIdentity: String) {
        guard let key = leasedKeyByGeneration.removeValue(forKey: generationIdentity) else { return }
        leases.removeValue(forKey: key)
        leaseAccessOrder.removeAll { $0 == key }
    }

    private func touchLease(_ key: ProseLayoutKey) {
        leaseAccessOrder.removeAll { $0 == key }
        leaseAccessOrder.append(key)
    }

    private func trimLeasesToBudget() {
        while leases.count > leaseBudget, let oldest = leaseAccessOrder.first {
            leaseAccessOrder.removeFirst()
            if let removed = leases.removeValue(forKey: oldest),
               leasedKeyByGeneration[removed.key.generationIdentity] == oldest {
                leasedKeyByGeneration.removeValue(forKey: removed.key.generationIdentity)
            }
        }
    }
}
