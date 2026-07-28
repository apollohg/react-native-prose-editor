import Foundation
import QuartzCore

/// Debug/device-only accounting. Samples belong to the traversal that caused
/// them; a phase transition is serialized with every producer under `lock`.
/// Percentiles use nearest-rank: sorted[ceil(p * n) - 1].
enum PreparedProseInstrumentation {
    enum Owner: String, CaseIterable { case compiled, unmountedLayout, fabricLeaseHandoff, directMounted, image, sidecars, other }
    enum InvalidationReason: String { case content, width, attachment, font, memoryPressure, cacheReset, reuse }
    enum TraversalPhase: String, CaseIterable { case cold, warm, imagesDisabled, reset }
    enum ViewerWorkKind: String, Codable { case layout, draw }
    struct ViewerWorkSpan: Equatable {
        let startNanos: UInt64
        let endNanos: UInt64
        let kind: ViewerWorkKind
    }
    struct FrameClassification: Equatable {
        let nominalFrameCount: Int
        let isDelayed: Bool
    }
    struct CacheSnapshot: Codable, Equatable {
        var unmountedCurrentBytes = 0
        var unmountedHighWaterBytes = 0
        var unmountedCurrentResidentCount = 0
        var unmountedHighWaterResidentCount = 0
        var compiledCurrentBytes = 0
        var compiledCurrentResidentCount = 0
    }
    struct DelayedInterval: Codable {
        let startNanos: UInt64
        let endNanos: UInt64
        let rawDeltaNanos: UInt64
        let viewerLayoutNanos: UInt64
        let viewerDrawNanos: UInt64
        let viewerCaused: Bool
    }
    struct PhaseSamples: Codable {
        var compileNanos: [UInt64] = []; var layoutNanos: [UInt64] = []; var combinedCompileLayoutNanos: [UInt64] = []
        var cacheLookupNanos: [UInt64] = []; var drawNanos: [UInt64] = []; var rawFrameDeltasNanos: [UInt64] = []
        var compileCount = 0; var layoutCount = 0; var cacheHits = 0; var cacheMisses = 0; var cacheWaits = 0; var drawCount = 0; var visibleBlocksDrawn = 0
        var imageRequestCount = 0; var imageMetadataCount = 0; var imageDecodeCount = 0
        var nominalFrameCount = 0; var delayedIntervalCount = 0; var viewerCausedDelayedIntervals: [DelayedInterval] = []
        var invalidations: [String: Int] = [:]
    }
    struct WindowEvidence: Codable {
        let windowId: String
        let entryIds: [String]
        let phase: String
        let residentKeyCount: Int
        let residentKeyDigest: String
        let cache: CacheSnapshot
        let compileCount: Int
        let layoutCount: Int
        let cacheMisses: Int
    }
    struct Snapshot: Codable {
        let schemaVersion: Int
        let percentileDefinition: String
        let nominalFramePeriodNanos: UInt64
        let singleTickToleranceNanos: UInt64
        let phaseSamples: [String: PhaseSamples]
        let windowEvidence: [WindowEvidence]
        let preResetSnapshot: CacheSnapshot
        let postResetSnapshot: CacheSnapshot
        let duplicatePublications: Int
    }

    static let nominalFramePeriodNanos: UInt64 = 16_666_667
    static let singleTickToleranceNanos: UInt64 = 1_000_000
    private static let lock = NSLock(); private static let sampleLimit = 20_000
    private static var enabled = false; private static var phase: TraversalPhase?
    private static var samples: [TraversalPhase: PhaseSamples] = [:]
    private static var completedPhases: Set<TraversalPhase> = []
    private static var pendingCompileNanos: [TraversalPhase: [String: UInt64]] = [:]
    private static var viewerWorkSpans: [TraversalPhase: [ViewerWorkSpan]] = [:]
    private static var cacheSnapshot = CacheSnapshot(); private static var preResetSnapshot = CacheSnapshot(); private static var postResetSnapshot = CacheSnapshot()
    private static var windowEvidence: [WindowEvidence] = []; private static var duplicatePublications = 0
    private static var previousDisplayTimestamp: CFTimeInterval = 0; private static var previousMonotonicNanos: UInt64 = 0; private static var surfaceDrawnSinceFrame = false

    static func classifyFrame(rawDeltaNanos: UInt64, nominalFramePeriodNanos: UInt64, singleTickToleranceNanos: UInt64) -> FrameClassification {
        precondition(nominalFramePeriodNanos > 0)
        if rawDeltaNanos <= nominalFramePeriodNanos + singleTickToleranceNanos { return .init(nominalFrameCount: 1, isDelayed: false) }
        return .init(nominalFrameCount: Int((rawDeltaNanos + nominalFramePeriodNanos - 1) / nominalFramePeriodNanos), isDelayed: true)
    }

    static func viewerCaused(
        _ start: UInt64,
        _ end: UInt64,
        _ spans: [ViewerWorkSpan],
        rawDeltaNanos: UInt64,
        nominalFramePeriodNanos: UInt64
    ) -> Bool {
        let lateness = rawDeltaNanos > nominalFramePeriodNanos
            ? rawDeltaNanos - nominalFramePeriodNanos
            : 0
        let clipped = spans.compactMap { span -> Range<UInt64>? in
            let lower = max(start, span.startNanos), upper = min(end, span.endNanos)
            return lower < upper ? lower..<upper : nil
        }.sorted { $0.lowerBound < $1.lowerBound }
        let union = clipped.reduce(into: [Range<UInt64>]()) { merged, range in
            if let last = merged.last, range.lowerBound <= last.upperBound {
                merged[merged.count - 1] = last.lowerBound..<max(last.upperBound, range.upperBound)
            } else { merged.append(range) }
        }
        let work = union.reduce(0) { $0 + ($1.upperBound - $1.lowerBound) }
        return lateness > 0 && work >= lateness
    }

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
        phase = value; completedPhases.remove(value); previousDisplayTimestamp = 0; previousMonotonicNanos = 0; surfaceDrawnSinceFrame = false
        lock.unlock()
#endif
    }
    static func endPhase() {
#if DEBUG
        lock.lock()
        if let phase { completedPhases.insert(phase); viewerWorkSpans[phase] = [] }
        phase = nil; previousDisplayTimestamp = 0; previousMonotonicNanos = 0; surfaceDrawnSinceFrame = false
        lock.unlock()
#endif
    }
    static func beginTraversal(_ value: TraversalPhase) { beginPhase(value) }
    static func endTraversal() { endPhase() }

    static func displayLinkDidTick(_ displayLink: CADisplayLink) {
#if DEBUG
        lock.lock(); defer { lock.unlock() }
        guard let phase else { return }
        let monotonicNow = DispatchTime.now().uptimeNanoseconds
        defer { previousDisplayTimestamp = displayLink.timestamp; previousMonotonicNanos = monotonicNow }
        guard previousDisplayTimestamp > 0, previousMonotonicNanos > 0, surfaceDrawnSinceFrame else { return }
        let rawDeltaNanos = UInt64((displayLink.timestamp - previousDisplayTimestamp) * 1_000_000_000)
        guard rawDeltaNanos > 0 else { return }
        let intervalStart = previousMonotonicNanos
        let intervalEnd = monotonicNow
        let spans = viewerWorkSpans[phase] ?? []
        mutate(phase) { sample in
            append(rawDeltaNanos, to: &sample.rawFrameDeltasNanos)
            let classification = classifyFrame(rawDeltaNanos: rawDeltaNanos, nominalFramePeriodNanos: nominalFramePeriodNanos, singleTickToleranceNanos: singleTickToleranceNanos)
            sample.nominalFrameCount += classification.nominalFrameCount
            if classification.isDelayed {
                sample.delayedIntervalCount += 1
                let caused = viewerCaused(
                    intervalStart,
                    intervalEnd,
                    spans,
                    rawDeltaNanos: rawDeltaNanos,
                    nominalFramePeriodNanos: nominalFramePeriodNanos
                )
                let interval = DelayedInterval(
                    startNanos: intervalStart,
                    endNanos: intervalEnd,
                    rawDeltaNanos: rawDeltaNanos,
                    viewerLayoutNanos: clippedWork(intervalStart, intervalEnd, spans, kind: .layout),
                    viewerDrawNanos: clippedWork(intervalStart, intervalEnd, spans, kind: .draw),
                    viewerCaused: caused
                )
                if caused { sample.viewerCausedDelayedIntervals.append(interval) }
            }
        }
        viewerWorkSpans[phase] = spans.filter { $0.endNanos > intervalStart }
        surfaceDrawnSinceFrame = false
#endif
    }

    static func recordViewerWork(startNanos: UInt64, endNanos: UInt64, kind: ViewerWorkKind) {
#if DEBUG
        guard startNanos < endNanos else { return }
        lock.lock(); defer { lock.unlock() }
        guard enabled, let phase else { return }
        viewerWorkSpans[phase, default: []].append(.init(startNanos: startNanos, endNanos: endNanos, kind: kind))
#endif
    }
    static func snapshotCache() -> CacheSnapshot {
#if DEBUG
        lock.lock(); defer { lock.unlock() }; return cacheSnapshot
#else
        return CacheSnapshot()
#endif
    }
    static func capturePreResetSnapshot() {
#if DEBUG
        lock.lock(); if enabled { preResetSnapshot = cacheSnapshot }; lock.unlock()
#endif
    }
    static func capturePostResetSnapshot() {
#if DEBUG
        lock.lock(); if enabled { postResetSnapshot = cacheSnapshot }; lock.unlock()
#endif
    }
    static func phaseCounters() -> (compileCount: Int, layoutCount: Int, cacheMisses: Int) {
#if DEBUG
        lock.lock(); defer { lock.unlock() }
        guard let phase else { return (0, 0, 0) }
        let value = samples[phase] ?? PhaseSamples()
        return (value.compileCount, value.layoutCount, value.cacheMisses)
#else
        return (0, 0, 0)
#endif
    }
    static func recordWindow(
        windowId: String,
        entryIds: [String],
        phase: TraversalPhase,
        residentKeyCount: Int,
        residentKeyDigest: String,
        cache: CacheSnapshot,
        counters: (compileCount: Int, layoutCount: Int, cacheMisses: Int)
    ) {
#if DEBUG
        lock.lock(); defer { lock.unlock() }
        guard enabled else { return }
        windowEvidence.append(
            .init(
                windowId: windowId,
                entryIds: entryIds,
                phase: phase.rawValue,
                residentKeyCount: residentKeyCount,
                residentKeyDigest: residentKeyDigest,
                cache: cache,
                compileCount: counters.compileCount,
                layoutCount: counters.layoutCount,
                cacheMisses: counters.cacheMisses
            )
        )
#endif
    }
    static func cacheUpdated(unmountedBytes: Int? = nil, unmountedResidentCount: Int? = nil, compiledBytes: Int? = nil, compiledResidentCount: Int? = nil) {
#if DEBUG
        lock.lock(); defer { lock.unlock() }
        guard enabled else { return }
        if let unmountedBytes { cacheSnapshot.unmountedCurrentBytes = max(0, unmountedBytes); cacheSnapshot.unmountedHighWaterBytes = max(cacheSnapshot.unmountedHighWaterBytes, cacheSnapshot.unmountedCurrentBytes) }
        if let unmountedResidentCount { cacheSnapshot.unmountedCurrentResidentCount = max(0, unmountedResidentCount); cacheSnapshot.unmountedHighWaterResidentCount = max(cacheSnapshot.unmountedHighWaterResidentCount, cacheSnapshot.unmountedCurrentResidentCount) }
        if let compiledBytes { cacheSnapshot.compiledCurrentBytes = max(0, compiledBytes) }
        if let compiledResidentCount { cacheSnapshot.compiledCurrentResidentCount = max(0, compiledResidentCount) }
#endif
    }

    static func exportJSON() -> String {
#if DEBUG
        lock.lock()
        let exportedPhases = [TraversalPhase.cold, .warm, .imagesDisabled]
        let completedSamples = Dictionary(uniqueKeysWithValues: exportedPhases.map { ($0.rawValue, samples[$0] ?? PhaseSamples()) })
        let snapshot = Snapshot(schemaVersion: 2, percentileDefinition: "nearest-rank: sorted[ceil(p*n)-1]", nominalFramePeriodNanos: nominalFramePeriodNanos, singleTickToleranceNanos: singleTickToleranceNanos, phaseSamples: completedSamples, windowEvidence: windowEvidence, preResetSnapshot: preResetSnapshot, postResetSnapshot: postResetSnapshot, duplicatePublications: duplicatePublications)
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
    @inline(__always) static func compiled(_ start: UInt64, generation: String) { record(start) { phase, elapsed, _ in mutate(phase) { samples in samples.compileCount += 1; append(elapsed, to: &samples.compileNanos) }; pendingCompileNanos[phase, default: [:]][generation] = elapsed } }
    @inline(__always) static func laidOut(_ start: UInt64, generation: String) { record(start) { phase, elapsed, end in mutate(phase) { samples in samples.layoutCount += 1; append(elapsed, to: &samples.layoutNanos); if let compile = pendingCompileNanos[phase]?.removeValue(forKey: generation) { append(compile + elapsed, to: &samples.combinedCompileLayoutNanos) } }; recordViewerWorkLocked(startNanos: start, endNanos: end, kind: .layout, phase: phase) } }
    @inline(__always) static func cacheLookup(_ start: UInt64, hit: Bool, waited: Bool = false) { record(start) { phase, elapsed, _ in mutate(phase) { samples in append(elapsed, to: &samples.cacheLookupNanos); if hit { samples.cacheHits += 1 } else { samples.cacheMisses += 1 }; if waited { samples.cacheWaits += 1 } } } }
    @inline(__always) static func drew(_ start: UInt64, visibleBlocks: Int) { record(start) { phase, elapsed, end in mutate(phase) { samples in append(elapsed, to: &samples.drawNanos); samples.drawCount += 1; samples.visibleBlocksDrawn += visibleBlocks }; recordViewerWorkLocked(startNanos: start, endNanos: end, kind: .draw, phase: phase); surfaceDrawnSinceFrame = true } }
    static func imageRequested() { incrementImageCounter { $0.imageRequestCount += 1 } }
    static func imageMetadataRead() { incrementImageCounter { $0.imageMetadataCount += 1 } }
    static func imageDecoded() { incrementImageCounter { $0.imageDecodeCount += 1 } }
    static func retained(_ owner: Owner, scope: String, bytes: Int) { }
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
    private static func record(_ start: UInt64, _ body: (TraversalPhase, UInt64, UInt64) -> Void) {
#if DEBUG
        guard start != 0 else { return }; lock.lock(); defer { lock.unlock() }; guard enabled, let phase else { return }; let end = DispatchTime.now().uptimeNanoseconds; body(phase, end - start, end)
#endif
    }
    private static func incrementImageCounter(_ body: (inout PhaseSamples) -> Void) {
#if DEBUG
        lock.lock(); defer { lock.unlock() }; guard enabled, let phase else { return }; mutate(phase, body)
#endif
    }
    private static func recordViewerWorkLocked(startNanos: UInt64, endNanos: UInt64, kind: ViewerWorkKind, phase: TraversalPhase) { guard startNanos < endNanos else { return }; viewerWorkSpans[phase, default: []].append(.init(startNanos: startNanos, endNanos: endNanos, kind: kind)) }
    private static func clippedWork(_ start: UInt64, _ end: UInt64, _ spans: [ViewerWorkSpan], kind: ViewerWorkKind) -> UInt64 {
        let clipped = spans.filter { $0.kind == kind }.compactMap { span -> Range<UInt64>? in let lower = max(start, span.startNanos), upper = min(end, span.endNanos); return lower < upper ? lower..<upper : nil }.sorted { $0.lowerBound < $1.lowerBound }
        return clipped.reduce(into: [Range<UInt64>]()) { merged, range in if let last = merged.last, range.lowerBound <= last.upperBound { merged[merged.count - 1] = last.lowerBound..<max(last.upperBound, range.upperBound) } else { merged.append(range) } }.reduce(0) { $0 + ($1.upperBound - $1.lowerBound) }
    }
    private static func mutate(_ phase: TraversalPhase, _ body: (inout PhaseSamples) -> Void) { var value = samples[phase] ?? PhaseSamples(); body(&value); samples[phase] = value }
    private static func append(_ value: UInt64, to samples: inout [UInt64]) { if samples.count < sampleLimit { samples.append(value) } }
    private static func resetLocked() { phase = nil; samples = [:]; completedPhases = []; pendingCompileNanos = [:]; viewerWorkSpans = [:]; cacheSnapshot = CacheSnapshot(); preResetSnapshot = CacheSnapshot(); postResetSnapshot = CacheSnapshot(); windowEvidence = []; duplicatePublications = 0; previousDisplayTimestamp = 0; previousMonotonicNanos = 0; surfaceDrawnSinceFrame = false }
}
