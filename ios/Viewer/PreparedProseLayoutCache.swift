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
    private var retainedBytes = 0
    private let byteBudget: Int

    init(byteBudget: Int = 32 * 1024 * 1024) {
        self.byteBudget = byteBudget
    }

    func value(for key: ProseLayoutKey, build: () throws -> PreparedProseLayout) throws -> PreparedProseLayout {
        condition.lock()
        if let layout = completed[key] {
            touch(key)
            condition.unlock()
            return layout
        }
        if let preparation = inFlight[key] {
            while preparation.result == nil { condition.wait() }
            let result = preparation.result!
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

    func removeAllUnmounted() {
        condition.lock()
        completed.removeAll()
        accessOrder.removeAll()
        retainedBytes = 0
        condition.unlock()
    }

    private func touch(_ key: ProseLayoutKey) {
        accessOrder.removeAll { $0 == key }
        accessOrder.append(key)
    }

    private func trimToBudget() {
        while retainedBytes > byteBudget, let oldest = accessOrder.first {
            accessOrder.removeFirst()
            if let removed = completed.removeValue(forKey: oldest) {
                retainedBytes -= removed.retainedBytes
            }
        }
    }
}
