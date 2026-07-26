import Foundation
import QuartzCore

/// Debug/device-only accounting. All access, including `enabled`, is protected
/// by `lock`. Percentiles use nearest-rank: sorted[ceil(p * n) - 1].
enum PreparedProseInstrumentation {
    enum Owner: String, CaseIterable { case compiled, unmountedLayout, fabricLeaseHandoff, directMounted, image, sidecars, other }
    enum InvalidationReason: String { case content, width, attachment, font, memoryPressure, cacheReset, reuse }
    enum TraversalPhase: String { case cold, warm, imagesDisabled, reset }
    struct Snapshot: Codable {
        let percentileDefinition: String; let compileCount: Int; let compileNanos: [UInt64]; let layoutCount: Int; let layoutNanos: [UInt64]
        let cacheHits: Int; let cacheMisses: Int; let cacheWaits: Int; let cacheLookupNanos: [UInt64]; let drawNanos: [UInt64]
        let coldFrameNanos: [UInt64]; let warmFrameNanos: [UInt64]; let imagesDisabledFrameNanos: [UInt64]; let warmViewerFrameNanos: [UInt64]
        let visibleBlocksDrawn: Int; let invalidations: [String: Int]; let duplicatePublications: Int; let retainedBytes: [String: Int]
    }
    private static let lock = NSLock(); private static let sampleLimit = 20_000
    private static var enabled = false; private static var phase: TraversalPhase?
    private static var compileNanos: [UInt64] = []; private static var layoutNanos: [UInt64] = []; private static var cacheLookupNanos: [UInt64] = []; private static var drawNanos: [UInt64] = []
    private static var coldFrameNanos: [UInt64] = []; private static var warmFrameNanos: [UInt64] = []; private static var imagesDisabledFrameNanos: [UInt64] = []; private static var warmViewerFrameNanos: [UInt64] = []
    private static var compileCount = 0; private static var layoutCount = 0; private static var cacheHits = 0; private static var cacheMisses = 0; private static var cacheWaits = 0; private static var visibleBlocksDrawn = 0; private static var duplicatePublications = 0
    private static var invalidations: [String: Int] = [:]; private static var retainedByScope: [String: Int] = [:]; private static var previousDisplayTimestamp: CFTimeInterval = 0; private static var surfaceDrawnSinceFrame = false
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
    static func beginTraversal(_ value: TraversalPhase) {
#if DEBUG
        lock.lock(); phase = value; previousDisplayTimestamp = 0; surfaceDrawnSinceFrame = false; lock.unlock()
 #endif
    }
    static func endTraversal() {
#if DEBUG
        lock.lock(); phase = nil; previousDisplayTimestamp = 0; surfaceDrawnSinceFrame = false; lock.unlock()
 #endif
    }
    /// Called by CADisplayLink while and only while the collection traversal owns the visible surface.
    static func displayLinkDidTick(_ displayLink: CADisplayLink) {
#if DEBUG
        lock.lock(); defer { lock.unlock() }; guard let phase else { return }; defer { previousDisplayTimestamp = displayLink.timestamp }
        guard previousDisplayTimestamp > 0, surfaceDrawnSinceFrame else { return }; let nanos = UInt64((displayLink.timestamp - previousDisplayTimestamp) * 1_000_000_000)
        guard nanos > 0 else { return }; switch phase { case .cold: append(nanos, to: &coldFrameNanos); case .warm: append(nanos, to: &warmFrameNanos); append(nanos, to: &warmViewerFrameNanos); case .imagesDisabled: append(nanos, to: &imagesDisabledFrameNanos); case .reset: break }; surfaceDrawnSinceFrame = false
 #endif
    }
    static func exportJSON() -> String {
#if DEBUG
        lock.lock()
        let totals = Dictionary(uniqueKeysWithValues: Owner.allCases.map { owner in
            (owner.rawValue, retainedByScope.filter { $0.key.hasPrefix(owner.rawValue + ":") }.reduce(0) { $0 + $1.value })
        })
        let snapshot = Snapshot(percentileDefinition: "nearest-rank: sorted[ceil(p*n)-1]", compileCount: compileCount, compileNanos: compileNanos, layoutCount: layoutCount, layoutNanos: layoutNanos, cacheHits: cacheHits, cacheMisses: cacheMisses, cacheWaits: cacheWaits, cacheLookupNanos: cacheLookupNanos, drawNanos: drawNanos, coldFrameNanos: coldFrameNanos, warmFrameNanos: warmFrameNanos, imagesDisabledFrameNanos: imagesDisabledFrameNanos, warmViewerFrameNanos: warmViewerFrameNanos, visibleBlocksDrawn: visibleBlocksDrawn, invalidations: invalidations, duplicatePublications: duplicatePublications, retainedBytes: totals); lock.unlock()
        return String(data: (try? JSONEncoder().encode(snapshot)) ?? Data("{}".utf8), encoding: .utf8) ?? "{}"
        #else
        return "{}"
 #endif
    }
    @inline(__always) static func now() -> UInt64 {
#if DEBUG
        lock.lock(); let active = enabled; lock.unlock(); return active ? DispatchTime.now().uptimeNanoseconds : 0
        #else
        return 0
 #endif
    }
    @inline(__always) static func compiled(_ start: UInt64) { record(start) { compileCount += 1; append($0, to: &compileNanos) } }
    @inline(__always) static func laidOut(_ start: UInt64) { record(start) { layoutCount += 1; append($0, to: &layoutNanos) } }
    @inline(__always) static func cacheLookup(_ start: UInt64, hit: Bool, waited: Bool = false) { record(start) { append($0, to: &cacheLookupNanos); if hit { cacheHits += 1 } else { cacheMisses += 1 }; if waited { cacheWaits += 1 } } }
    @inline(__always) static func drew(_ start: UInt64, visibleBlocks: Int) { record(start) { append($0, to: &drawNanos); visibleBlocksDrawn += visibleBlocks; surfaceDrawnSinceFrame = true } }
    static func retained(_ owner: Owner, scope: String, bytes: Int) {
#if DEBUG
        lock.lock(); if enabled { retainedByScope[owner.rawValue + ":" + scope] = max(0, bytes) }; lock.unlock()
 #endif
    }
    static func invalidated(_ reason: InvalidationReason) {
#if DEBUG
        lock.lock(); if enabled { invalidations[reason.rawValue, default: 0] += 1 }; lock.unlock()
 #endif
    }
    static func duplicatePublication() {
#if DEBUG
        lock.lock(); if enabled { duplicatePublications += 1 }; lock.unlock()
 #endif
    }
    private static func record(_ start: UInt64, _ body: (UInt64) -> Void) {
#if DEBUG
        guard start != 0 else { return }; lock.lock(); guard enabled else { lock.unlock(); return }; body(DispatchTime.now().uptimeNanoseconds - start); lock.unlock()
 #endif
    }
    private static func append(_ value: UInt64, to samples: inout [UInt64]) { if samples.count < sampleLimit { samples.append(value) } }
    private static func resetLocked() { phase = nil; compileNanos = []; layoutNanos = []; cacheLookupNanos = []; drawNanos = []; coldFrameNanos = []; warmFrameNanos = []; imagesDisabledFrameNanos = []; warmViewerFrameNanos = []; compileCount = 0; layoutCount = 0; cacheHits = 0; cacheMisses = 0; cacheWaits = 0; visibleBlocksDrawn = 0; duplicatePublications = 0; invalidations = [:]; retainedByScope = [:]; previousDisplayTimestamp = 0; surfaceDrawnSinceFrame = false }
}
