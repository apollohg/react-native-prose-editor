import Foundation

/// Thread-safe performance accounting for prepared viewer benchmarks. All
/// production call sites compile to no-ops in release builds; device suites
/// explicitly enable this collector before a traversal.
enum PreparedProseInstrumentation {
    enum Owner: String, CaseIterable { case compiled, layout, image, sidecars }
    enum InvalidationReason: String { case content, width, attachment, font, memoryPressure, cacheReset, reuse }

    struct Snapshot: Codable {
        let compileCount: Int
        let compileNanos: [UInt64]
        let layoutCount: Int
        let layoutNanos: [UInt64]
        let cacheHits: Int
        let cacheMisses: Int
        let cacheWaits: Int
        let cacheLookupNanos: [UInt64]
        let drawNanos: [UInt64]
        let visibleBlocksDrawn: Int
        let invalidations: [String: Int]
        let duplicatePublications: Int
        let frameNanos: [UInt64]
        let retainedBytes: [String: Int]
    }

    private static let lock = NSLock()
    private static var enabled = false
    private static var compileNanos: [UInt64] = []
    private static var layoutNanos: [UInt64] = []
    private static var cacheLookupNanos: [UInt64] = []
    private static var drawNanos: [UInt64] = []
    private static var frameNanos: [UInt64] = []
    private static var compileCount = 0
    private static var layoutCount = 0
    private static var cacheHits = 0
    private static var cacheMisses = 0
    private static var cacheWaits = 0
    private static var visibleBlocksDrawn = 0
    private static var duplicatePublications = 0
    private static var invalidations: [String: Int] = [:]
    private static var retainedByScope: [String: Int] = [:]
    private static let sampleLimit = 20_000

    static func beginBenchmark() {
#if DEBUG
        lock.lock(); enabled = true; resetLocked(); lock.unlock()
#endif
    }

    static func reset() {
#if DEBUG
        lock.lock(); resetLocked(); lock.unlock()
#endif
    }

    static func exportJSON() -> String {
#if DEBUG
        lock.lock()
        let totals = Dictionary(uniqueKeysWithValues: Owner.allCases.map { owner in
            (owner.rawValue, retainedByScope.filter { $0.key.hasPrefix(owner.rawValue + ":") }.reduce(0) { $0 + $1.value })
        })
        let snapshot = Snapshot(compileCount: compileCount, compileNanos: compileNanos, layoutCount: layoutCount, layoutNanos: layoutNanos, cacheHits: cacheHits, cacheMisses: cacheMisses, cacheWaits: cacheWaits, cacheLookupNanos: cacheLookupNanos, drawNanos: drawNanos, visibleBlocksDrawn: visibleBlocksDrawn, invalidations: invalidations, duplicatePublications: duplicatePublications, frameNanos: frameNanos, retainedBytes: totals)
        lock.unlock()
        return String(data: (try? JSONEncoder().encode(snapshot)) ?? Data("{}".utf8), encoding: .utf8) ?? "{}"
#else
        return "{}"
#endif
    }

    @inline(__always) static func now() -> UInt64 {
#if DEBUG
        return enabled ? DispatchTime.now().uptimeNanoseconds : 0
#else
        return 0
#endif
    }

    @inline(__always) static func compiled(_ startedAt: UInt64) {
#if DEBUG
        guard enabled, startedAt != 0 else { return }; lock.lock(); compileCount += 1; append(DispatchTime.now().uptimeNanoseconds - startedAt, to: &compileNanos); lock.unlock()
#endif
    }
    @inline(__always) static func laidOut(_ startedAt: UInt64) {
#if DEBUG
        guard enabled, startedAt != 0 else { return }; lock.lock(); layoutCount += 1; append(DispatchTime.now().uptimeNanoseconds - startedAt, to: &layoutNanos); lock.unlock()
#endif
    }
    @inline(__always) static func cacheLookup(_ startedAt: UInt64, hit: Bool, waited: Bool = false) {
#if DEBUG
        guard enabled else { return }
        lock.lock(); append(DispatchTime.now().uptimeNanoseconds - startedAt, to: &cacheLookupNanos); if hit { cacheHits += 1 } else { cacheMisses += 1 }; if waited { cacheWaits += 1 }; lock.unlock()
#endif
    }
    @inline(__always) static func drew(_ startedAt: UInt64, visibleBlocks: Int) {
#if DEBUG
        guard enabled else { return }
        lock.lock(); append(DispatchTime.now().uptimeNanoseconds - startedAt, to: &drawNanos); append(DispatchTime.now().uptimeNanoseconds - startedAt, to: &frameNanos); visibleBlocksDrawn += visibleBlocks; lock.unlock()
#endif
    }
    static func retained(_ owner: Owner, scope: String, bytes: Int) {
#if DEBUG
        guard enabled else { return }; lock.lock(); retainedByScope[owner.rawValue + ":" + scope] = max(0, bytes); lock.unlock()
#endif
    }
    static func invalidated(_ reason: InvalidationReason) {
#if DEBUG
        guard enabled else { return }; lock.lock(); invalidations[reason.rawValue, default: 0] += 1; lock.unlock()
#endif
    }
    static func duplicatePublication() {
#if DEBUG
        guard enabled else { return }; lock.lock(); duplicatePublications += 1; lock.unlock()
#endif
    }

    private static func append(_ value: UInt64, to samples: inout [UInt64]) { if samples.count < sampleLimit { samples.append(value) } }
    private static func resetLocked() { compileNanos = []; layoutNanos = []; cacheLookupNanos = []; drawNanos = []; frameNanos = []; compileCount = 0; layoutCount = 0; cacheHits = 0; cacheMisses = 0; cacheWaits = 0; visibleBlocksDrawn = 0; duplicatePublications = 0; invalidations = [:]; retainedByScope = [:] }
}
