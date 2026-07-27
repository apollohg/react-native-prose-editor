import Foundation
import QuartzCore

/// Debug/device-only accounting. Samples belong to the traversal that caused
/// them; a phase transition is serialized with every producer under `lock`.
/// Percentiles use nearest-rank: sorted[ceil(p * n) - 1].
enum PreparedProseInstrumentation {
    enum Owner: String, CaseIterable { case compiled, unmountedLayout, fabricLeaseHandoff, directMounted, image, sidecars, other }
    enum InvalidationReason: String { case content, width, attachment, font, memoryPressure, cacheReset, reuse }
    enum TraversalPhase: String, CaseIterable { case cold, warm, imagesDisabled, reset }

    struct PhaseSamples: Codable {
        var compileNanos: [UInt64] = []; var layoutNanos: [UInt64] = []; var combinedCompileLayoutNanos: [UInt64] = []
        var cacheLookupNanos: [UInt64] = []; var drawNanos: [UInt64] = []; var frameNanos: [UInt64] = []; var viewerFrameNanos: [UInt64] = []
        var compileCount = 0; var layoutCount = 0; var cacheHits = 0; var cacheMisses = 0; var cacheWaits = 0; var drawCount = 0; var visibleBlocksDrawn = 0
        var invalidations: [String: Int] = [:]
    }
    struct Snapshot: Codable {
        let percentileDefinition: String; let phaseSamples: [String: PhaseSamples]
        let duplicatePublications: Int; let retainedBytes: [String: Int]
    }

    private static let lock = NSLock(); private static let sampleLimit = 20_000
    private static var enabled = false; private static var phase: TraversalPhase?
    private static var samples: [TraversalPhase: PhaseSamples] = [:]
    /// Exports must never observe a traversal while it is still collecting
    /// draw/frame evidence. A phase becomes visible only at `endPhase()`.
    private static var completedPhases: Set<TraversalPhase> = []
    /// Compile durations are associated by immutable generation, never by a
    /// positional zip of unrelated arrays.
    private static var pendingCompileNanos: [TraversalPhase: [String: UInt64]] = [:]
    private static var retainedByScope: [String: Int] = [:]; private static var duplicatePublications = 0
    private static var previousDisplayTimestamp: CFTimeInterval = 0; private static var surfaceDrawnSinceFrame = false

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
    static func beginPhase(_ value: TraversalPhase) {
#if DEBUG
        lock.lock()
        guard enabled else { lock.unlock(); return }
        phase = value
        completedPhases.remove(value)
        previousDisplayTimestamp = 0
        surfaceDrawnSinceFrame = false
        lock.unlock()
#endif
    }
    static func endPhase() {
#if DEBUG
        lock.lock()
        if let phase { completedPhases.insert(phase) }
        phase = nil
        previousDisplayTimestamp = 0
        surfaceDrawnSinceFrame = false
        lock.unlock()
#endif
    }
    /// Native device harness compatibility; new bridges use the explicit
    /// begin/end phase names so cache reset cannot be mistaken for traversal.
    static func beginTraversal(_ value: TraversalPhase) { beginPhase(value) }
    static func endTraversal() { endPhase() }

    /// Called by CADisplayLink only while the collection traversal owns a
    /// mounted viewer surface. Frame evidence requires a preceding draw.
    static func displayLinkDidTick(_ displayLink: CADisplayLink) {
#if DEBUG
        lock.lock(); defer { lock.unlock() }
        guard let phase else { return }
        defer { previousDisplayTimestamp = displayLink.timestamp }
        guard previousDisplayTimestamp > 0, surfaceDrawnSinceFrame else { return }
        let nanos = UInt64((displayLink.timestamp - previousDisplayTimestamp) * 1_000_000_000)
        guard nanos > 0 else { return }
        mutate(phase) { samples in
            append(nanos, to: &samples.frameNanos)
            if phase == .warm { append(nanos, to: &samples.viewerFrameNanos) }
        }
        surfaceDrawnSinceFrame = false
#endif
    }

    static func exportJSON() -> String {
#if DEBUG
        lock.lock()
        let owners = Dictionary(uniqueKeysWithValues: Owner.allCases.map { owner in
            (owner.rawValue, retainedByScope.filter { $0.key.hasPrefix(owner.rawValue + ":") }.reduce(0) { $0 + $1.value })
        })
        let completedSamples = Dictionary(uniqueKeysWithValues: samples.compactMap { phase, samples in
            completedPhases.contains(phase) ? (phase.rawValue, samples) : nil
        })
        let snapshot = Snapshot(percentileDefinition: "nearest-rank: sorted[ceil(p*n)-1]", phaseSamples: completedSamples, duplicatePublications: duplicatePublications, retainedBytes: owners)
        lock.unlock()
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
    @inline(__always) static func compiled(_ start: UInt64, generation: String) { record(start) { phase, elapsed in
        mutate(phase) { samples in samples.compileCount += 1; append(elapsed, to: &samples.compileNanos) }
        pendingCompileNanos[phase, default: [:]][generation] = elapsed
    } }
    @inline(__always) static func laidOut(_ start: UInt64, generation: String) { record(start) { phase, elapsed in
        mutate(phase) { samples in
            samples.layoutCount += 1; append(elapsed, to: &samples.layoutNanos)
            if let compile = pendingCompileNanos[phase]?.removeValue(forKey: generation) { append(compile + elapsed, to: &samples.combinedCompileLayoutNanos) }
        }
    } }
    @inline(__always) static func cacheLookup(_ start: UInt64, hit: Bool, waited: Bool = false) { record(start) { phase, elapsed in mutate(phase) { samples in append(elapsed, to: &samples.cacheLookupNanos); if hit { samples.cacheHits += 1 } else { samples.cacheMisses += 1 }; if waited { samples.cacheWaits += 1 } } } }
    @inline(__always) static func drew(_ start: UInt64, visibleBlocks: Int) { record(start) { phase, elapsed in mutate(phase) { samples in append(elapsed, to: &samples.drawNanos); samples.drawCount += 1; samples.visibleBlocksDrawn += visibleBlocks }; surfaceDrawnSinceFrame = true } }
    static func retained(_ owner: Owner, scope: String, bytes: Int) {
#if DEBUG
        lock.lock(); if enabled { retainedByScope[owner.rawValue + ":" + scope] = max(0, bytes) }; lock.unlock()
#endif
    }
    static func invalidated(_ reason: InvalidationReason) {
#if DEBUG
        lock.lock(); if enabled, let phase { mutate(phase) { samples in samples.invalidations[reason.rawValue, default: 0] += 1 } }; lock.unlock()
#endif
    }
    static func duplicatePublication() {
#if DEBUG
        lock.lock(); if enabled { duplicatePublications += 1 }; lock.unlock()
#endif
    }
    private static func record(_ start: UInt64, _ body: (TraversalPhase, UInt64) -> Void) {
#if DEBUG
        guard start != 0 else { return }; lock.lock(); defer { lock.unlock() }; guard enabled, let phase else { return }; body(phase, DispatchTime.now().uptimeNanoseconds - start)
#endif
    }
    private static func mutate(_ phase: TraversalPhase, _ body: (inout PhaseSamples) -> Void) { var value = samples[phase] ?? PhaseSamples(); body(&value); samples[phase] = value }
    private static func append(_ value: UInt64, to samples: inout [UInt64]) { if samples.count < sampleLimit { samples.append(value) } }
    private static func resetLocked() { phase = nil; samples = [:]; completedPhases = []; pendingCompileNanos = [:]; retainedByScope = [:]; duplicatePublications = 0; previousDisplayTimestamp = 0; surfaceDrawnSinceFrame = false }
}
