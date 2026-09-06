import XCTest
import UIKit

extension NativePerformanceTests {
    /// iPhone 13 release gate. This is intentionally a device-only benchmark:
    /// Task 14 supplies the one authorized execution and records the export.
    func testPerformance_preparedProseCorpusGates_iPhone13() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["PREPARED_PROSE_DEVICE_BENCHMARK"] == "1",
            "Runs only on the iPhone 13 device benchmark lane."
        )
        let corpus = try PreparedProseBenchmarkCorpus.load()
        let configuration = try PreparedProseBenchmarkConfiguration.load()
        let harness = PreparedProseCollectionHarness(corpus: corpus, configuration: configuration, imagesEnabled: true)
        PreparedProseInstrumentation.beginBenchmark()
        _ = try traversePreparedProseWindows(harness, windows: corpus.warmWindows, phase: .cold, imagesEnabled: true)
        _ = try traversePreparedProseWindows(harness, windows: corpus.warmWindows, phase: .imagesDisabled, imagesEnabled: false)
        PreparedProseInstrumentation.capturePreResetSnapshot()
        XCTAssertTrue(harness.hasMountedPreparedViewer)
        harness.resetCache()
        XCTAssertTrue(harness.hasMountedPreparedViewer)
        PreparedProseInstrumentation.capturePostResetSnapshot()
        let benchmarkExport = PreparedProseInstrumentation.exportJSON()
        print("[PreparedProseBenchmarkExport]\(benchmarkExport)")
        try PreparedProsePerformanceGates.assertPasses(
            exportJSON: benchmarkExport,
            expectedDocuments: corpus.documents.count
        )
    }

    /// Fixture-only device contract for Task 11 integration. It is separately
    /// gated so routine suites do not execute preparation or scroll work.
    func testPreparedProseHarnessStaticFixtures() throws {
        try XCTSkipUnless(
            ProcessInfo.processInfo.environment["PREPARED_PROSE_STATIC_HARNESS_FIXTURES"] == "1",
            "Runs only when Task 14 explicitly requests static harness fixtures."
        )
        let corpus = try PreparedProseBenchmarkCorpus.load()
        let configuration = try PreparedProseBenchmarkConfiguration.load()
        let fixtures = try PreparedProseHarnessStaticFixtures.load()
        let harness = PreparedProseCollectionHarness(corpus: corpus, configuration: configuration, imagesEnabled: true)
        PreparedProseInstrumentation.beginBenchmark()
        let preparedHeight = try attachedPreparedProseHeight(harness, entryID: fixtures.preparation.entryId)
        let shortHeight = try attachedPreparedProseHeight(harness, entryID: fixtures.differingHeights.shortEntryId)
        let longHeight = try attachedPreparedProseHeight(harness, entryID: fixtures.differingHeights.longEntryId)
        XCTAssertGreaterThan(preparedHeight, 0)
        XCTAssertGreaterThan(longHeight, shortHeight)
        let export = try JSONSerialization.jsonObject(with: Data(PreparedProseInstrumentation.exportJSON().utf8)) as? [String: Any]
        let warm = ((export?["phaseSamples"] as? [String: Any])?[fixtures.drawEvidence.phase] as? [String: Any])
        XCTAssertGreaterThan(warm?["drawCount"] as? Int ?? 0, 0)
    }

    func testPreparedProseInstrumentationContract() throws {
        let fixture = try PreparedProseHarnessStaticFixtures.load().frameClassification
        for delta in fixture.oneTickDeltasNanos {
            XCTAssertEqual(
                PreparedProseInstrumentation.classifyFrame(
                    rawDeltaNanos: delta,
                    nominalFramePeriodNanos: fixture.nominalFramePeriodNanos,
                    singleTickToleranceNanos: fixture.singleTickToleranceNanos
                ),
                .init(nominalFrameCount: 1, isDelayed: false)
            )
        }
        XCTAssertEqual(
            PreparedProseInstrumentation.classifyFrame(
                rawDeltaNanos: fixture.delayedDeltaNanos,
                nominalFramePeriodNanos: fixture.nominalFramePeriodNanos,
                singleTickToleranceNanos: fixture.singleTickToleranceNanos
            ),
            .init(nominalFrameCount: 3, isDelayed: true)
        )

        let delayedEnd = fixture.delayedDeltaNanos
        XCTAssertFalse(
            PreparedProseInstrumentation.viewerCaused(
                0,
                24_000_000,
                [
                    .init(startNanos: 10_000_000, endNanos: 20_000_000, kind: .draw),
                    .init(startNanos: 10_000_000, endNanos: 20_000_000, kind: .layout),
                ],
                rawDeltaNanos: delayedEnd,
                nominalFramePeriodNanos: fixture.nominalFramePeriodNanos
            ),
            "causal attribution must use the exported raw frame delta and union overlapping spans"
        )
        XCTAssertTrue(
            PreparedProseInstrumentation.viewerCaused(
                0,
                delayedEnd,
                [
                    .init(startNanos: 0, endNanos: 12_000_000, kind: .layout),
                    .init(startNanos: 12_000_000, endNanos: 24_000_000, kind: .draw),
                ],
                rawDeltaNanos: delayedEnd,
                nominalFramePeriodNanos: fixture.nominalFramePeriodNanos
            )
        )

        PreparedProseInstrumentation.beginBenchmark()
        for phase in [
            PreparedProseInstrumentation.TraversalPhase.cold,
            .warm,
            .imagesDisabled,
        ] {
            PreparedProseInstrumentation.beginPhase(phase)
            PreparedProseInstrumentation.endPhase()
        }
        let export = try JSONDecoder().decode(
            PreparedProseBenchmarkExportContract.self,
            from: Data(PreparedProseInstrumentation.exportJSON().utf8)
        )
        XCTAssertEqual(export.schemaVersion, 3)
        XCTAssertEqual(export.nominalFramePeriodNanos, fixture.nominalFramePeriodNanos)
        XCTAssertEqual(export.singleTickToleranceNanos, fixture.singleTickToleranceNanos)
        for snapshot in [export.preResetSnapshot, export.postResetSnapshot] {
            XCTAssertEqual(snapshot.unmountedCurrentBytes, 0)
            XCTAssertEqual(snapshot.unmountedHighWaterBytes, 0)
            XCTAssertEqual(snapshot.unmountedCurrentResidentCount, 0)
            XCTAssertEqual(snapshot.unmountedHighWaterResidentCount, 0)
            XCTAssertEqual(snapshot.compiledCurrentBytes, 0)
            XCTAssertEqual(snapshot.compiledCurrentResidentCount, 0)
        }
        for phase in [export.phaseSamples.cold, export.phaseSamples.warm, export.phaseSamples.imagesDisabled] {
            XCTAssertEqual(phase.imageRequestCount, 0)
            XCTAssertEqual(phase.imageMetadataCount, 0)
            XCTAssertEqual(phase.imageDecodeCount, 0)
        }
    }

    func testPreparedProseInstrumentationSamplesEveryTickAndPreservesTransitionBaseline() throws {
        let nominal = PreparedProseInstrumentation.nominalFramePeriodNanos
        let baselineTimestamp: UInt64 = 1_000_000_000
        let baselineMonotonic: UInt64 = 2_000_000_000

        PreparedProseInstrumentation.beginBenchmark()
        PreparedProseInstrumentation.beginPhase(.cold)
        PreparedProseInstrumentation.recordDisplayLinkTick(
            callbackTimestampNanos: baselineTimestamp,
            observedMonotonicNanos: baselineMonotonic,
            callbackDurationNanos: nominal,
            targetLeadNanos: Int64(nominal)
        )
        PreparedProseInstrumentation.recordDisplayLinkTick(
            callbackTimestampNanos: baselineTimestamp + nominal,
            observedMonotonicNanos: baselineMonotonic + nominal,
            callbackDurationNanos: nominal,
            targetLeadNanos: Int64(nominal)
        )
        PreparedProseInstrumentation.transitionPhase(.warm)
        PreparedProseInstrumentation.recordDisplayLinkTick(
            callbackTimestampNanos: baselineTimestamp + (2 * nominal),
            observedMonotonicNanos: baselineMonotonic + (2 * nominal),
            callbackDurationNanos: nominal,
            targetLeadNanos: Int64(nominal)
        )
        PreparedProseInstrumentation.endPhase()

        let export = try JSONDecoder().decode(
            PreparedProseBenchmarkExportContract.self,
            from: Data(PreparedProseInstrumentation.exportJSON().utf8)
        )
        XCTAssertEqual(export.phaseSamples.cold.rawFrameDeltasNanos, [nominal])
        XCTAssertEqual(export.phaseSamples.warm.rawFrameDeltasNanos, [nominal])
        XCTAssertEqual(export.phaseSamples.cold.frameCallbackSamples.count, 2)
        XCTAssertEqual(export.phaseSamples.warm.frameCallbackSamples.count, 1)
        XCTAssertEqual(export.phaseSamples.cold.frameCallbackSamples.last?.callbackDurationNanos, nominal)
        XCTAssertEqual(export.phaseSamples.warm.frameCallbackSamples[0].targetLeadNanos, Int64(nominal))
    }

    func testPreparedProseInstrumentationPreservesBaselineAcrossWindowPasses() throws {
        let nominal = PreparedProseInstrumentation.nominalFramePeriodNanos
        let interWindowGap = nominal * 3
        let baselineTimestamp: UInt64 = 1_000_000_000
        let baselineMonotonic: UInt64 = 2_000_000_000

        PreparedProseInstrumentation.beginBenchmark()
        PreparedProseInstrumentation.beginPhase(.cold)
        PreparedProseInstrumentation.recordDisplayLinkTick(
            callbackTimestampNanos: baselineTimestamp,
            observedMonotonicNanos: baselineMonotonic,
            callbackDurationNanos: nominal,
            targetLeadNanos: Int64(nominal)
        )
        PreparedProseInstrumentation.recordDisplayLinkTick(
            callbackTimestampNanos: baselineTimestamp + nominal,
            observedMonotonicNanos: baselineMonotonic + nominal,
            callbackDurationNanos: nominal,
            targetLeadNanos: Int64(nominal)
        )

        PreparedProseInstrumentation.beginPhase(.cold, preservingDisplayLinkBaseline: true)
        PreparedProseInstrumentation.recordDisplayLinkTick(
            callbackTimestampNanos: baselineTimestamp + nominal + interWindowGap,
            observedMonotonicNanos: baselineMonotonic + nominal + interWindowGap,
            callbackDurationNanos: nominal,
            targetLeadNanos: Int64(nominal)
        )
        PreparedProseInstrumentation.endPhase()

        let export = try JSONDecoder().decode(
            PreparedProseBenchmarkExportContract.self,
            from: Data(PreparedProseInstrumentation.exportJSON().utf8)
        )
        XCTAssertEqual(export.phaseSamples.cold.rawFrameDeltasNanos, [nominal, interWindowGap])
    }

    func testPreparedProseInstrumentationLifecycleWorkUsesSingleCausalUnion() {
        let spans: [PreparedProseInstrumentation.ViewerWorkSpan] = [
            .init(startNanos: 0, endNanos: 12_000_000, kind: .layout),
            .init(startNanos: 8_000_000, endNanos: 20_000_000, kind: .draw),
            .init(startNanos: 18_000_000, endNanos: 30_000_000, kind: .lifecycle),
        ]
        XCTAssertEqual(PreparedProseInstrumentation.viewerWorkNanos(0, 30_000_000, spans), 30_000_000)
        XCTAssertTrue(
            PreparedProseInstrumentation.viewerCaused(
                0,
                30_000_000,
                spans,
                rawDeltaNanos: PreparedProseInstrumentation.nominalFramePeriodNanos + 25_000_000,
                nominalFramePeriodNanos: PreparedProseInstrumentation.nominalFramePeriodNanos
            )
        )
    }

    func testPreparedProseInstrumentationCadenceRatioUsesInferredNominalSlots() {
        let nominal = PreparedProseInstrumentation.nominalFramePeriodNanos
        let ratio = PreparedProseInstrumentation.cadencePassRatio(
            rawFrameDeltasNanos: [
                nominal + PreparedProseInstrumentation.singleTickToleranceNanos,
                nominal * 3,
            ]
        )
        XCTAssertEqual(ratio, 0.25, accuracy: 0.000_001)
    }

    func testPreparedProseBenchmarkDisplayLinkRequestsFixed60HzCadence() {
        let displayLink = CADisplayLink(target: self, selector: #selector(displayLinkProbe(_:)))
        defer { displayLink.invalidate() }

        PreparedProseInstrumentation.configureBenchmarkCadence(displayLink)

        XCTAssertEqual(displayLink.preferredFrameRateRange.minimum, 60)
        XCTAssertEqual(displayLink.preferredFrameRateRange.maximum, 60)
        XCTAssertEqual(displayLink.preferredFrameRateRange.preferred, 60)
    }

    func testPreparedProseCollectionSelfSizingLifecycle() throws {
        let corpus = try PreparedProseBenchmarkCorpus.load()
        let configuration = try PreparedProseBenchmarkConfiguration.load()
        let shortWindow = try XCTUnwrap(corpus.warmWindows.first { $0.id == "short-01" })
        let harness = PreparedProseCollectionHarness(
            corpus: corpus,
            configuration: configuration,
            imagesEnabled: true
        )
        let completion = expectation(description: "attached self-sizing short window")
        var result: Result<[PreparedProseCollectionHarness.WindowTraversalResult], Error>?

        PreparedProseInstrumentation.beginBenchmark()
        harness.traverseWindows([shortWindow], phase: .cold, imagesEnabled: true) { traversal in
            result = traversal
            completion.fulfill()
        }
        wait(for: [completion], timeout: Self.preparedProseWindowTraversalTimeout)

        let traversals = try XCTUnwrap(result).get()
        let traversal = try XCTUnwrap(traversals.first)
        XCTAssertEqual(traversal.prime.residentKeyCount, 60)
        XCTAssertEqual(traversal.warm.compileCount, 0)
        XCTAssertEqual(traversal.warm.layoutCount, 0)
        XCTAssertEqual(traversal.warm.cacheMisses, 0)
        XCTAssertEqual(traversal.renderedHeight, traversal.preparedArtifactHeight, accuracy: 0.5)

        let imagesDisabledCompletion = expectation(description: "images-disabled short window revisits its leading cell")
        var imagesDisabledResult: Result<[PreparedProseCollectionHarness.WindowTraversalResult], Error>?
        harness.traverseWindows([shortWindow], phase: .imagesDisabled, imagesEnabled: false) { traversal in
            imagesDisabledResult = traversal
            imagesDisabledCompletion.fulfill()
        }
        wait(for: [imagesDisabledCompletion], timeout: Self.preparedProseWindowTraversalTimeout)

        let imagesDisabledTraversals = try XCTUnwrap(imagesDisabledResult).get()
        let imagesDisabledTraversal = try XCTUnwrap(imagesDisabledTraversals.first)
        XCTAssertEqual(imagesDisabledTraversal.renderedHeight, imagesDisabledTraversal.preparedArtifactHeight, accuracy: 0.5)

        let export = try JSONDecoder().decode(
            PreparedProseBenchmarkExportContract.self,
            from: Data(PreparedProseInstrumentation.exportJSON().utf8)
        )
        let shortWindowEvidence = export.windowEvidence.filter { $0.windowId == shortWindow.id }
        XCTAssertEqual(shortWindowEvidence.filter { $0.phase == "cold" }.count, 1)
        XCTAssertEqual(shortWindowEvidence.filter { $0.phase == "warm" }.count, 1)
        XCTAssertEqual(shortWindowEvidence.filter { $0.phase == "imagesDisabled" }.count, 2)

        let source = try String(contentsOf: URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("NativePerformanceTests+CollectionHarness.swift"), encoding: .utf8)
        let start = try XCTUnwrap(source.range(of: "final class PreparedProseCollectionHarness"))
        let end = try XCTUnwrap(source.range(of: "enum PreparedProsePerformanceGates"))
        let harnessSource = String(source[start.lowerBound..<end.lowerBound])
        for forbidden in ["measurementView", "prepareAndMeasure", "RunLoop.main.run"] {
            XCTAssertFalse(harnessSource.contains(forbidden), "harness must not use \(forbidden)")
        }
        XCTAssertFalse(
            harnessSource.contains("scrollToItem(at:"),
            "harness must not use a per-item UICollectionView jump loop"
        )
    }

    func testPreparedProseTraversalTimeoutScalesWithWindowCount() {
        XCTAssertEqual(preparedProseTraversalTimeout(forWindowCount: 0), Self.preparedProseWindowTraversalTimeout)
        XCTAssertEqual(preparedProseTraversalTimeout(forWindowCount: 1), Self.preparedProseWindowTraversalTimeout)
        XCTAssertEqual(preparedProseTraversalTimeout(forWindowCount: 27), 270)
    }

}
