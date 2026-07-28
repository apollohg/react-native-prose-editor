import XCTest
import UIKit

private struct ApplyUpdateTraceStats {
    let name: String
    let traces: [EditorTextView.ApplyUpdateTrace]

    private func average(_ selector: (EditorTextView.ApplyUpdateTrace) -> UInt64) -> Double {
        guard !traces.isEmpty else { return 0 }
        return Double(traces.map(selector).reduce(0, +)) / Double(traces.count) / 1_000_000.0
    }

    func summaryString(tag: String = "NativePerformanceTests") -> String {
        let averageReplaceUtf16 = traces.isEmpty
            ? 0
            : traces.map(\.applyRenderReplaceUtf16Length).reduce(0, +) / traces.count
        let averageReplacementUtf16 = traces.isEmpty
            ? 0
            : traces.map(\.applyRenderReplacementUtf16Length).reduce(0, +) / traces.count
        return "[\(tag)] \(name) avgMs={parse=\(String(format: "%.3f", average { $0.parseNanos })), resolveBlocks=\(String(format: "%.3f", average { $0.resolveRenderBlocksNanos })), patchEligibility=\(String(format: "%.3f", average { $0.patchEligibilityNanos })), patchTrim=\(String(format: "%.3f", average { $0.patchTrimNanos })), patchMetadata=\(String(format: "%.3f", average { $0.patchMetadataNanos })), buildRender=\(String(format: "%.3f", average { $0.buildRenderNanos })), applyRender=\(String(format: "%.3f", average { $0.applyRenderNanos })), applyRenderTextMutation=\(String(format: "%.3f", average { $0.applyRenderTextMutationNanos })), applyRenderBeginEditing=\(String(format: "%.3f", average { $0.applyRenderBeginEditingNanos })), applyRenderEndEditing=\(String(format: "%.3f", average { $0.applyRenderEndEditingNanos })), applyRenderStringMutation=\(String(format: "%.3f", average { $0.applyRenderStringMutationNanos })), applyRenderAttributeMutation=\(String(format: "%.3f", average { $0.applyRenderAttributeMutationNanos })), applyRenderAuthorizedText=\(String(format: "%.3f", average { $0.applyRenderAuthorizedTextNanos })), applyRenderCacheInvalidation=\(String(format: "%.3f", average { $0.applyRenderCacheInvalidationNanos })), selection=\(String(format: "%.3f", average { $0.selectionNanos })), selectionResolve=\(String(format: "%.3f", average { $0.selectionResolveNanos })), selectionAssignment=\(String(format: "%.3f", average { $0.selectionAssignmentNanos })), selectionChrome=\(String(format: "%.3f", average { $0.selectionChromeNanos })), postApply=\(String(format: "%.3f", average { $0.postApplyNanos })), postApplyTypingAttributes=\(String(format: "%.3f", average { $0.postApplyTypingAttributesNanos })), postApplyHeightNotify=\(String(format: "%.3f", average { $0.postApplyHeightNotifyNanos })), postApplyHeightNotifyMeasure=\(String(format: "%.3f", average { $0.postApplyHeightNotifyMeasureNanos })), postApplyHeightNotifyCallback=\(String(format: "%.3f", average { $0.postApplyHeightNotifyCallbackNanos })), postApplyHeightNotifyEnsureLayout=\(String(format: "%.3f", average { $0.postApplyHeightNotifyEnsureLayoutNanos })), postApplyHeightNotifyUsedRect=\(String(format: "%.3f", average { $0.postApplyHeightNotifyUsedRectNanos })), postApplyHeightNotifyContentSize=\(String(format: "%.3f", average { $0.postApplyHeightNotifyContentSizeNanos })), postApplyHeightNotifySizeThatFits=\(String(format: "%.3f", average { $0.postApplyHeightNotifySizeThatFitsNanos })), postApplySelectionOrContent=\(String(format: "%.3f", average { $0.postApplySelectionOrContentCallbackNanos })), total=\(String(format: "%.3f", average { $0.totalNanos }))} patchUsage=\(traces.filter { $0.usedPatch }.count)/\(traces.count) smallPatchMutationUsage=\(traces.filter { $0.usedSmallPatchTextMutation }.count)/\(traces.count) avgPatchUtf16={replace=\(averageReplaceUtf16), replacement=\(averageReplacementUtf16)}"
    }
}

private struct HostedLayoutTraceStats {
    let name: String
    let traces: [RichTextEditorView.HostedLayoutTrace]

    private func average(
        _ selector: (RichTextEditorView.HostedLayoutTrace) -> UInt64
    ) -> Double {
        guard !traces.isEmpty else { return 0 }
        return Double(traces.map(selector).reduce(0, +)) / Double(traces.count) / 1_000_000.0
    }

    private func averageCount(
        _ selector: (RichTextEditorView.HostedLayoutTrace) -> Int
    ) -> Double {
        guard !traces.isEmpty else { return 0 }
        return Double(traces.map(selector).reduce(0, +)) / Double(traces.count)
    }

    func summaryString(tag: String = "NativePerformanceTests") -> String {
        "[\(tag)] \(name) avgMs={intrinsicContentSize=\(String(format: "%.3f", average { $0.intrinsicContentSizeNanos })), measuredEditorHeight=\(String(format: "%.3f", average { $0.measuredEditorHeightNanos })), layoutSubviews=\(String(format: "%.3f", average { $0.layoutSubviewsNanos })), refreshOverlays=\(String(format: "%.3f", average { $0.refreshOverlaysNanos })), onHeightMayChange=\(String(format: "%.3f", average { $0.onHeightMayChangeNanos }))} avgCount={intrinsicContentSize=\(String(format: "%.2f", averageCount { $0.intrinsicContentSizeCount })), measuredEditorHeight=\(String(format: "%.2f", averageCount { $0.measuredEditorHeightCount })), layoutSubviews=\(String(format: "%.2f", averageCount { $0.layoutSubviewsCount })), refreshOverlays=\(String(format: "%.2f", averageCount { $0.refreshOverlaysCount })), overlayScheduleRequest=\(String(format: "%.2f", averageCount { $0.overlayScheduleRequestCount })), overlayScheduleExecute=\(String(format: "%.2f", averageCount { $0.overlayScheduleExecuteCount })), overlayScheduleSkip=\(String(format: "%.2f", averageCount { $0.overlayScheduleSkipCount })), onHeightMayChange=\(String(format: "%.2f", averageCount { $0.onHeightMayChangeCount }))}"
    }
}

@MainActor
final class NativePerformanceTests: XCTestCase {
    private let baseFont = UIFont.systemFont(ofSize: 16)
    private let textColor = UIColor.black
    private static let preparedProseWindowTraversalTimeout: TimeInterval = 10

    func testPerformance_renderBridgeLargeDocument() {
        let renderJSON = NativePerformanceFixtureFactory.largeRenderJSON()
        // The v2 update carries renderBlocks (the flat renderElements array
        // was a legacy-update shape); render through the blocks entry.
        let renderBlocks = (try? JSONSerialization.jsonObject(with: Data(renderJSON.utf8)))
            .flatMap { $0 as? [String: Any] }
            .flatMap { $0["renderBlocks"] as? [[[String: Any]]] } ?? []
        let options = measureOptions()

        measure(metrics: [XCTClockMetric()], options: options) {
            autoreleasepool {
                let attributed = RenderBridge.renderBlocks(
                    fromArray: renderBlocks,
                    baseFont: baseFont,
                    textColor: textColor
                )

                XCTAssertGreaterThan(attributed.length, 0)
                _ = attributed.string.utf16.count
                if attributed.length > 0 {
                    _ = attributed.attributes(at: min(1, attributed.length - 1), effectiveRange: nil)
                }
            }
        }
    }

    func testPerformance_applyUpdateJSONLargeDocument() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let updateJSON = NativePerformanceFixtureFactory.loadLargeDocument(into: editorId)
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(id: editorId)
        var traceSamples: [EditorTextView.ApplyUpdateTrace] = []

        // Warm the text system before measuring steady-state apply cost.
        textView.applyUpdateJSON(updateJSON, notifyDelegate: false)
        textView.layoutIfNeeded()

        let options = measureOptions()
        measure(metrics: [XCTClockMetric()], options: options) {
            autoreleasepool {
                textView.applyUpdateJSON(updateJSON, notifyDelegate: false)
                textView.layoutIfNeeded()
                if let trace = textView.lastApplyUpdateTrace() {
                    traceSamples.append(trace)
                }
                XCTAssertFalse(textView.text.isEmpty)
                _ = textView.attributedText.length
            }
        }
        print(
            ApplyUpdateTraceStats(
                name: "applyUpdateJSONLargeDocument.breakdown",
                traces: traceSamples
            ).summaryString()
        )
    }

    func testPerformance_typingRoundTripLargeDocument() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        _ = NativePerformanceFixtureFactory.loadLargeDocument(into: editorId)
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        textView.bindEditor(id: editorId)
        textView.layoutIfNeeded()

        let typingOffset = NativePerformanceFixtureFactory.typingCursorOffset(in: textView)
        setSelection(in: textView, utf16Range: NSRange(location: typingOffset, length: 0))

        let options = measureOptions()
        measure(metrics: [XCTClockMetric()], options: options) {
            autoreleasepool {
                setSelection(in: textView, utf16Range: NSRange(location: typingOffset, length: 0))
                textView.insertText("!")
                textView.deleteBackward()
                XCTAssertFalse(textView.text.isEmpty)
                XCTAssertNotNil(textView.selectedTextRange)
            }
        }
    }

    func testPerformance_paragraphSplitRoundTripLargeDocument() {
        let options = measureOptions()
        let sessions = NativePerformanceFixtureFactory.paragraphSplitSessions(
            count: max(options.iterationCount + 4, 12)
        )
        defer {
            for session in sessions {
                destroyV2Editor(id: session.editorId)
            }
        }

        var remainingSessionIndices = Array(sessions.indices)
        var traceSamples: [EditorTextView.ApplyUpdateTrace] = []

        measure(metrics: [XCTClockMetric()], options: options) {
            autoreleasepool {
                guard let sessionIndex = remainingSessionIndices.first else {
                    XCTFail("expected prebuilt paragraph split sessions")
                    return
                }
                remainingSessionIndices.removeFirst()

                let session = sessions[sessionIndex]
                setSelection(in: session.textView, utf16Range: NSRange(location: session.splitOffset, length: 0))
                session.textView.insertText("\n")
                session.textView.layoutIfNeeded()

                if let trace = session.textView.lastApplyUpdateTrace() {
                    traceSamples.append(trace)
                }
                XCTAssertGreaterThan(session.textView.attributedText.length, session.initialTextLength)
                XCTAssertTrue(
                    session.textView.lastRenderAppliedPatch(),
                    "paragraph split must use the render patch path; trace: \(String(describing: session.textView.lastApplyUpdateTrace()))"
                )
                XCTAssertNotNil(session.textView.selectedTextRange)
            }
        }
        print(
            ApplyUpdateTraceStats(
                name: "paragraphSplitRoundTripLargeDocument.breakdown",
                traces: traceSamples
            ).summaryString()
        )
    }

    func testPerformance_paragraphSplitRoundTripLargeDocument_autoGrow() {
        let options = measureOptions()
        let sessions = NativePerformanceFixtureFactory.paragraphSplitSessions(
            count: max(options.iterationCount + 4, 12),
            autoGrow: true
        )
        defer {
            for session in sessions {
                destroyV2Editor(id: session.editorId)
            }
        }

        var remainingSessionIndices = Array(sessions.indices)
        var traceSamples: [EditorTextView.ApplyUpdateTrace] = []

        measure(metrics: [XCTClockMetric()], options: options) {
            autoreleasepool {
                guard let sessionIndex = remainingSessionIndices.first else {
                    XCTFail("expected prebuilt paragraph split sessions")
                    return
                }
                remainingSessionIndices.removeFirst()

                let session = sessions[sessionIndex]
                setSelection(in: session.textView, utf16Range: NSRange(location: session.splitOffset, length: 0))
                session.textView.insertText("\n")
                session.textView.layoutIfNeeded()

                if let trace = session.textView.lastApplyUpdateTrace() {
                    traceSamples.append(trace)
                }
                XCTAssertGreaterThan(session.textView.attributedText.length, session.initialTextLength)
                XCTAssertTrue(
                    session.textView.lastRenderAppliedPatch(),
                    "auto-grow paragraph split must use the render patch path; trace: \(String(describing: session.textView.lastApplyUpdateTrace()))"
                )
                XCTAssertNotNil(session.textView.selectedTextRange)
            }
        }
        print(
            ApplyUpdateTraceStats(
                name: "paragraphSplitRoundTripLargeDocument.autoGrow.breakdown",
                traces: traceSamples
            ).summaryString()
        )
    }

    func testPerformance_paragraphSplitRoundTripLargeDocument_autoGrowHostedView() {
        let options = measureOptions()
        let sessions = NativePerformanceFixtureFactory.hostedParagraphSplitSessions(
            count: max(options.iterationCount + 4, 12)
        )
        defer {
            for session in sessions {
                session.view.removeFromSuperview()
                session.window.isHidden = true
                destroyV2Editor(id: session.editorId)
            }
        }

        var remainingSessionIndices = Array(sessions.indices)
        var traceSamples: [EditorTextView.ApplyUpdateTrace] = []
        var hostedLayoutTraceSamples: [RichTextEditorView.HostedLayoutTrace] = []

        measure(metrics: [XCTClockMetric()], options: options) {
            autoreleasepool {
                guard let sessionIndex = remainingSessionIndices.first else {
                    XCTFail("expected prebuilt hosted paragraph split sessions")
                    return
                }
                remainingSessionIndices.removeFirst()

                let session = sessions[sessionIndex]
                session.view.captureHostedLayoutTraceForTesting = true
                session.view.resetHostedLayoutTraceForTesting()
                setSelection(in: session.view.textView, utf16Range: NSRange(location: session.splitOffset, length: 0))
                session.view.textView.insertText("\n")
                flushMainQueue()

                let measuredHeight = ceil(session.view.intrinsicContentSize.height)
                session.view.frame.size.height = measuredHeight
                session.view.layoutIfNeeded()

                if let trace = session.view.textView.lastApplyUpdateTrace() {
                    traceSamples.append(trace)
                }
                hostedLayoutTraceSamples.append(session.view.lastHostedLayoutTraceForTesting())
                XCTAssertGreaterThan(session.view.textView.attributedText.length, session.initialTextLength)
                XCTAssertTrue(
                    session.view.textView.lastRenderAppliedPatch(),
                    "hosted auto-grow paragraph split must use the render patch path; trace: \(String(describing: session.view.textView.lastApplyUpdateTrace()))"
                )
                XCTAssertGreaterThan(measuredHeight, 0)
            }
        }
        print(
            ApplyUpdateTraceStats(
                name: "paragraphSplitRoundTripLargeDocument.autoGrowHostedView.breakdown",
                traces: traceSamples
            ).summaryString()
        )
        print(
            HostedLayoutTraceStats(
                name: "paragraphSplitRoundTripLargeDocument.autoGrowHostedView.hostedLayout",
                traces: hostedLayoutTraceSamples
            ).summaryString()
        )
    }

    func testPerformance_selectionScrubLargeDocument() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        _ = NativePerformanceFixtureFactory.loadLargeDocument(into: editorId)
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        textView.bindEditor(id: editorId)
        textView.layoutIfNeeded()

        let scrubOffsets = NativePerformanceFixtureFactory.selectionScrubOffsets(in: textView, points: 48)
        let options = measureOptions()

        measure(metrics: [XCTClockMetric()], options: options) {
            autoreleasepool {
                for offset in scrubOffsets {
                    setSelection(in: textView, utf16Range: NSRange(location: offset, length: 0))
                }

                let finalOffset = textView.offset(
                    from: textView.beginningOfDocument,
                    to: textView.selectedTextRange?.start ?? textView.endOfDocument
                )
                XCTAssertEqual(finalOffset, scrubOffsets.last ?? 0)
            }
        }
    }

    func testPerformance_remoteSelectionOverlayRefreshMultiPeerLargeDocument() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let updateJSON = NativePerformanceFixtureFactory.loadLargeDocument(into: editorId)
        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        view.editorId = editorId
        view.textView.applyUpdateJSON(updateJSON, notifyDelegate: false)
        view.layoutIfNeeded()

        let selections = NativePerformanceFixtureFactory.remoteSelections(
            editorId: editorId,
            peerCount: 24,
            selectionWidth: 24
        )
        view.setRemoteSelections(selections)
        view.layoutIfNeeded()

        let options = measureOptions()
        measure(metrics: [XCTClockMetric()], options: options) {
            autoreleasepool {
                view.setRemoteSelections(selections)
                view.layoutIfNeeded()
                XCTAssertFalse(view.remoteSelectionOverlaySubviewsForTesting().isEmpty)
                XCTAssertGreaterThanOrEqual(view.remoteSelectionOverlaySubviewsForTesting().count, selections.count)
            }
        }
    }

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
        XCTAssertEqual(export.schemaVersion, 2)
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

        let source = try String(contentsOfFile: #filePath, encoding: .utf8)
        let start = try XCTUnwrap(source.range(of: "private final class PreparedProseCollectionHarness"))
        let end = try XCTUnwrap(source.range(of: "private enum PreparedProsePerformanceGates"))
        let harnessSource = String(source[start.lowerBound..<end.lowerBound])
        for forbidden in ["measurementView", "prepareAndMeasure", "RunLoop.main.run"] {
            XCTAssertFalse(harnessSource.contains(forbidden), "harness must not use \(forbidden)")
        }
        XCTAssertFalse(
            harnessSource.contains("scrollToItem(at:"),
            "harness must not use a per-item UICollectionView jump loop"
        )
    }

    private func traversePreparedProseWindows(
        _ harness: PreparedProseCollectionHarness,
        windows: [PreparedProseBenchmarkCorpus.WarmWindow],
        phase: PreparedProseInstrumentation.TraversalPhase,
        imagesEnabled: Bool
    ) throws -> [PreparedProseCollectionHarness.WindowTraversalResult] {
        let completion = expectation(description: "prepared prose \(phase.rawValue) traversal")
        var result: Result<[PreparedProseCollectionHarness.WindowTraversalResult], Error>?
        harness.traverseWindows(windows, phase: phase, imagesEnabled: imagesEnabled) {
            result = $0
            completion.fulfill()
        }
        wait(for: [completion], timeout: preparedProseTraversalTimeout(forWindowCount: windows.count))
        return try XCTUnwrap(result).get()
    }

    func testPreparedProseTraversalTimeoutScalesWithWindowCount() {
        XCTAssertEqual(preparedProseTraversalTimeout(forWindowCount: 0), Self.preparedProseWindowTraversalTimeout)
        XCTAssertEqual(preparedProseTraversalTimeout(forWindowCount: 1), Self.preparedProseWindowTraversalTimeout)
        XCTAssertEqual(preparedProseTraversalTimeout(forWindowCount: 27), 270)
    }

    private func preparedProseTraversalTimeout(forWindowCount windowCount: Int) -> TimeInterval {
        TimeInterval(max(1, windowCount)) * Self.preparedProseWindowTraversalTimeout
    }

    private func attachedPreparedProseHeight(
        _ harness: PreparedProseCollectionHarness,
        entryID: String
    ) throws -> CGFloat {
        let window = PreparedProseBenchmarkCorpus.WarmWindow(
            id: "fixture-\(entryID)",
            primeIds: [entryID],
            warmIds: [entryID]
        )
        return try XCTUnwrap(
            traversePreparedProseWindows(harness, windows: [window], phase: .cold, imagesEnabled: true).first
        ).renderedHeight
    }

    private func measureOptions() -> XCTMeasureOptions {
        let options = XCTMeasureOptions()
        options.iterationCount = 5
        return options
    }
}

private struct PreparedProseBenchmarkCorpus: Decodable {
    struct Entry: Decodable { let id: String; let category: String; let contentJSON: [String: JSONValue] }
    struct WarmWindow: Decodable { let id: String; let primeIds: [String]; let warmIds: [String] }
    let documents: [Entry]
    let coldTraversal: [String]
    let warmTraversal: [String]
    let warmWindows: [WarmWindow]

    static func load() throws -> Self {
        guard let url = Bundle(for: NativePerformanceTests.self).url(
            forResource: "viewer-performance-corpus", withExtension: "json"
        ) else { throw NSError(domain: "PreparedProseBenchmarkCorpus", code: 1, userInfo: [NSLocalizedDescriptionKey: "Bundled viewer performance corpus is missing."]) }
        let data = try Data(contentsOf: url)
        let corpus = try JSONDecoder().decode(Self.self, from: data)
        XCTAssertEqual(corpus.documents.count, 1_000)
        XCTAssertEqual(Set(corpus.documents.map(\.id)).count, 1_000)
        XCTAssertEqual(corpus.coldTraversal.count, 1_000)
        XCTAssertEqual(corpus.warmTraversal.count, 1_000)
        XCTAssertEqual(corpus.warmWindows.count, 27)
        return corpus
    }
}

/// This fixture is the one complete configuration shared by the iOS,
/// Android, and FlatList harnesses. The corpus intentionally contains node
/// kinds beyond the default schema, so an empty configuration is invalid.
private struct PreparedProseBenchmarkConfiguration: Decodable {
    let configuration: JSONValue
    let imageLoadingPolicy: JSONValue

    static func load() throws -> Self {
        guard let url = Bundle(for: NativePerformanceTests.self).url(
            forResource: "prepared-prose-benchmark-config", withExtension: "json"
        ) else { throw NSError(domain: "PreparedProseBenchmarkConfiguration", code: 1, userInfo: [NSLocalizedDescriptionKey: "Bundled prepared prose benchmark configuration is missing."]) }
        return try JSONDecoder().decode(Self.self, from: Data(contentsOf: url))
    }

    func viewerConfiguration(imagesEnabled: Bool) throws -> ProseViewerConfiguration {
        ProseViewerConfiguration(
            configJSON: String(data: try JSONEncoder().encode(configuration), encoding: .utf8) ?? "{}",
            imagePolicyJSON: String(data: try JSONEncoder().encode(imageLoadingPolicy), encoding: .utf8),
            imagesEnabled: imagesEnabled,
            collapsesWhenEmpty: true
        )
    }
}

private struct PreparedProseHarnessStaticFixtures: Decodable {
    struct Preparation: Decodable { let entryId: String; let widthPoints: CGFloat }
    struct DifferingHeights: Decodable { let shortEntryId: String; let longEntryId: String; let widthPoints: CGFloat }
    struct DrawEvidence: Decodable { let phase: String }
    struct FrameClassification: Decodable {
        let nominalFramePeriodNanos: UInt64
        let singleTickToleranceNanos: UInt64
        let oneTickDeltasNanos: [UInt64]
        let delayedDeltaNanos: UInt64
    }
    let preparation: Preparation
    let differingHeights: DifferingHeights
    let drawEvidence: DrawEvidence
    let frameClassification: FrameClassification

    static func load() throws -> Self {
        guard let url = Bundle(for: NativePerformanceTests.self).url(
            forResource: "prepared-prose-harness-static-fixtures", withExtension: "json"
        ) else { throw NSError(domain: "PreparedProseHarnessStaticFixtures", code: 1, userInfo: [NSLocalizedDescriptionKey: "Bundled prepared prose harness fixtures are missing."]) }
        return try JSONDecoder().decode(Self.self, from: Data(contentsOf: url))
    }
}

private struct PreparedProseBenchmarkExportContract: Decodable {
    struct CacheSnapshot: Decodable {
        let unmountedCurrentBytes: Int
        let unmountedHighWaterBytes: Int
        let unmountedCurrentResidentCount: Int
        let unmountedHighWaterResidentCount: Int
        let compiledCurrentBytes: Int
        let compiledCurrentResidentCount: Int
    }
    struct Phase: Decodable {
        let imageRequestCount: Int
        let imageMetadataCount: Int
        let imageDecodeCount: Int
    }
    struct PhaseSamples: Decodable {
        let cold: Phase
        let warm: Phase
        let imagesDisabled: Phase
    }
    struct WindowEvidence: Decodable {
        let windowId: String
        let phase: String
    }
    let schemaVersion: Int
    let nominalFramePeriodNanos: UInt64
    let singleTickToleranceNanos: UInt64
    let phaseSamples: PhaseSamples
    let windowEvidence: [WindowEvidence]
    let preResetSnapshot: CacheSnapshot
    let postResetSnapshot: CacheSnapshot
}

private enum JSONValue: Codable {
    case string(String), number(Double), bool(Bool), object([String: JSONValue]), array([JSONValue]), null
    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() { self = .null } else if let value = try? container.decode(Bool.self) { self = .bool(value) } else if let value = try? container.decode(Double.self) { self = .number(value) } else if let value = try? container.decode(String.self) { self = .string(value) } else if let value = try? container.decode([String: JSONValue].self) { self = .object(value) } else { self = .array(try container.decode([JSONValue].self)) }
    }
    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case let .string(value): try container.encode(value)
        case let .number(value): try container.encode(value)
        case let .bool(value): try container.encode(value)
        case let .object(value): try container.encode(value)
        case let .array(value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
}

private final class PreparedProseCollectionHarness: NSObject, UICollectionViewDataSource, UICollectionViewDelegate {
    struct WindowPhaseResult {
        let residentKeyCount: Int
        let residentKeyDigest: String
        let compileCount: Int
        let layoutCount: Int
        let cacheMisses: Int
    }

    struct WindowTraversalResult {
        let windowId: String
        let prime: WindowPhaseResult
        let warm: WindowPhaseResult
        let renderedHeight: CGFloat
        let preparedArtifactHeight: CGFloat
    }

    private enum Direction { case forward, reverse }
    private struct CounterBaseline {
        let compileCount: Int
        let layoutCount: Int
        let cacheMisses: Int
    }
    private struct ActiveTraversal {
        let windows: [PreparedProseBenchmarkCorpus.WarmWindow]
        let phase: PreparedProseInstrumentation.TraversalPhase
        let imagesEnabled: Bool
        let completion: (Result<[WindowTraversalResult], Error>) -> Void
        var index = 0
        var direction: Direction = .forward
        var prime: WindowPhaseResult?
        var results: [WindowTraversalResult] = []
        var counterBaseline = CounterBaseline(compileCount: 0, layoutCount: 0, cacheMisses: 0)
    }

    private let corpus: PreparedProseBenchmarkCorpus
    private let configuration: PreparedProseBenchmarkConfiguration
    private let defaultImagesEnabled: Bool
    private let byID: [String: PreparedProseBenchmarkCorpus.Entry]
    private let sourceByID: [String: String]
    private let collectionView: UICollectionView
    private let window: UIWindow
    private var orderedEntries: [PreparedProseBenchmarkCorpus.Entry] = []
    private var activeImagesEnabled = true
    private var activeViewerConfiguration: ProseViewerConfiguration?
    private var displayLink: CADisplayLink?
    private var traversal: ActiveTraversal?

    init(corpus: PreparedProseBenchmarkCorpus, configuration: PreparedProseBenchmarkConfiguration, imagesEnabled: Bool) {
        self.corpus = corpus; self.configuration = configuration; self.defaultImagesEnabled = imagesEnabled
        byID = Dictionary(uniqueKeysWithValues: corpus.documents.map { ($0.id, $0) })
        sourceByID = Dictionary(uniqueKeysWithValues: corpus.documents.map { entry in
            guard let data = try? JSONEncoder().encode(entry.contentJSON),
                  let source = String(data: data, encoding: .utf8)
            else { preconditionFailure("invalid corpus entry \(entry.id)") }
            return (entry.id, source)
        })
        let layout = UICollectionViewFlowLayout()
        layout.estimatedItemSize = UICollectionViewFlowLayout.automaticSize
        layout.minimumLineSpacing = 8
        layout.sectionInset = .zero
        collectionView = UICollectionView(frame: CGRect(x: 0, y: 0, width: 390, height: 844), collectionViewLayout: layout)
        window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        super.init()
        collectionView.dataSource = self; collectionView.delegate = self
        collectionView.register(PreparedProseCollectionCell.self, forCellWithReuseIdentifier: "prepared")
        let host = UIViewController(); host.view = collectionView; window.rootViewController = host; window.isHidden = false
    }
    deinit { displayLink?.invalidate(); window.isHidden = true }
    func resetCache() { PreparedProseLayoutRegistry.shared.didReceiveMemoryWarning() }
    var hasMountedPreparedViewer: Bool {
        collectionView.visibleCells.contains { ($0 as? PreparedProseCollectionCell)?.hasPreparedArtifact == true }
    }

    func traverseWindows(
        _ windows: [PreparedProseBenchmarkCorpus.WarmWindow],
        phase: PreparedProseInstrumentation.TraversalPhase,
        imagesEnabled: Bool? = nil,
        completion: @escaping (Result<[WindowTraversalResult], Error>) -> Void
    ) {
        guard traversal == nil else {
            completion(.failure(NSError(domain: "PreparedProseCollectionHarness", code: 1, userInfo: [NSLocalizedDescriptionKey: "a traversal is already active"])))
            return
        }
        guard phase == .cold || phase == .imagesDisabled else {
            completion(.failure(NSError(domain: "PreparedProseCollectionHarness", code: 2, userInfo: [NSLocalizedDescriptionKey: "window traversal begins with cold or imagesDisabled"])))
            return
        }
        traversal = .init(
            windows: windows,
            phase: phase,
            imagesEnabled: imagesEnabled ?? defaultImagesEnabled,
            completion: completion
        )
        startCurrentWindow()
    }

    private func startCurrentWindow() {
        guard var traversal, traversal.index < traversal.windows.count else {
            finishTraversal()
            return
        }
        let window = traversal.windows[traversal.index]
        orderedEntries = window.primeIds.compactMap { byID[$0] }
        guard orderedEntries.count == window.primeIds.count else {
            finishTraversal(error: NSError(domain: "PreparedProseCollectionHarness", code: 3, userInfo: [NSLocalizedDescriptionKey: "window \(window.id) references an unknown entry"]))
            return
        }
        activeImagesEnabled = traversal.imagesEnabled
        do {
            activeViewerConfiguration = try configuration.viewerConfiguration(imagesEnabled: activeImagesEnabled)
        } catch {
            finishTraversal(error: error)
            return
        }
        traversal.direction = .forward
        traversal.prime = nil
        self.traversal = traversal
        collectionView.contentOffset = .zero
        collectionView.reloadData()
        beginWindowPass(phase: traversal.phase)
    }

    private func beginWindowPass(phase: PreparedProseInstrumentation.TraversalPhase) {
        PreparedProseLayoutRegistry.shared.beginBenchmarkResidentCensus()
        PreparedProseInstrumentation.beginPhase(phase)
        if var traversal {
            let counters = PreparedProseInstrumentation.phaseCounters()
            traversal.counterBaseline = .init(
                compileCount: counters.compileCount,
                layoutCount: counters.layoutCount,
                cacheMisses: counters.cacheMisses
            )
            self.traversal = traversal
        }
        if displayLink == nil {
            let link = CADisplayLink(target: self, selector: #selector(displayLinkTick(_:)))
            displayLink = link
            link.add(to: .main, forMode: .common)
        }
    }

    @objc private func displayLinkTick(_ link: CADisplayLink) {
        PreparedProseInstrumentation.displayLinkDidTick(link)
        driveCurrentWindow(with: link)
    }

    private func driveCurrentWindow(with link: CADisplayLink) {
        guard let traversal, !orderedEntries.isEmpty else { return }
        let maximumOffset = max(0, collectionView.contentSize.height - collectionView.bounds.height)
        if maximumOffset == 0 {
            guard collectionView.indexPathsForVisibleItems.contains(IndexPath(item: 0, section: 0)) else { return }
            finishCurrentWindowPass()
            return
        }
        let distance = CGFloat(max(1, 2_000 * link.duration))
        let currentOffset = collectionView.contentOffset.y
        let target: CGFloat
        switch traversal.direction {
        case .forward:
            target = min(maximumOffset, currentOffset + distance)
        case .reverse:
            target = max(0, currentOffset - distance)
        }
        collectionView.contentOffset = CGPoint(x: 0, y: target)

        let destination = traversal.direction == .forward ? orderedEntries.count - 1 : 0
        guard target == (traversal.direction == .forward ? maximumOffset : 0),
              collectionView.indexPathsForVisibleItems.contains(IndexPath(item: destination, section: 0))
        else { return }
        finishCurrentWindowPass()
    }

    private func finishCurrentWindowPass() {
        guard var traversal else { return }
        let window = traversal.windows[traversal.index]
        let census = PreparedProseLayoutRegistry.shared.endBenchmarkResidentCensus()
        let counters = PreparedProseInstrumentation.phaseCounters()
        let result = WindowPhaseResult(
            residentKeyCount: census.count,
            residentKeyDigest: census.digest,
            compileCount: counters.compileCount - traversal.counterBaseline.compileCount,
            layoutCount: counters.layoutCount - traversal.counterBaseline.layoutCount,
            cacheMisses: counters.cacheMisses - traversal.counterBaseline.cacheMisses
        )
        let phase: PreparedProseInstrumentation.TraversalPhase =
            traversal.direction == .reverse && traversal.phase == .cold ? .warm : traversal.phase
        PreparedProseInstrumentation.recordWindow(
            windowId: window.id,
            entryIds: traversal.direction == .forward ? window.primeIds : window.warmIds,
            phase: phase,
            residentKeyCount: result.residentKeyCount,
            residentKeyDigest: result.residentKeyDigest,
            cache: PreparedProseInstrumentation.snapshotCache(),
            counters: (result.compileCount, result.layoutCount, result.cacheMisses)
        )
        PreparedProseInstrumentation.endPhase()

        if traversal.direction == .forward {
            traversal.prime = result
            traversal.direction = .reverse
            self.traversal = traversal
            beginWindowPass(phase: traversal.phase == .cold ? .warm : traversal.phase)
            return
        }

        let warm = result
        let visible = collectionView.cellForItem(at: IndexPath(item: 0, section: 0)) as? PreparedProseCollectionCell
        guard let visible else {
            finishTraversal(error: NSError(domain: "PreparedProseCollectionHarness", code: 4, userInfo: [NSLocalizedDescriptionKey: "leading cell was not attached at window completion"]))
            return
        }
        traversal.results.append(
            .init(
                windowId: window.id,
                prime: traversal.prime ?? result,
                warm: warm,
                renderedHeight: visible.bounds.height,
                preparedArtifactHeight: visible.preparedArtifactHeight
            )
        )
        traversal.index += 1
        self.traversal = traversal
        startCurrentWindow()
    }

    private func finishTraversal(error: Error? = nil) {
        displayLink?.invalidate()
        displayLink = nil
        guard let traversal else { return }
        self.traversal = nil
        if let error {
            traversal.completion(.failure(error))
        } else {
            traversal.completion(.success(traversal.results))
        }
    }

    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int { orderedEntries.count }
    func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
        let cell = collectionView.dequeueReusableCell(withReuseIdentifier: "prepared", for: indexPath) as! PreparedProseCollectionCell
        let entry = orderedEntries[indexPath.item]
        guard let source = sourceByID[entry.id], let activeViewerConfiguration else {
            XCTFail("missing stable benchmark input for \(entry.id)")
            return cell
        }
        do {
            try cell.configure(
                source: source,
                configuration: activeViewerConfiguration
            )
        } catch {
            XCTFail("invalid benchmark configuration: \(error)")
        }
        return cell
    }
}

private final class PreparedProseCollectionCell: UICollectionViewCell {
    private let viewer = ProseViewerView()
    private(set) var preparedArtifactHeight: CGFloat = 0
    var hasPreparedArtifact: Bool { preparedArtifactHeight > 0 }
    override init(frame: CGRect) {
        super.init(frame: frame)
        viewer.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(viewer)
        NSLayoutConstraint.activate([
            viewer.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            viewer.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            viewer.topAnchor.constraint(equalTo: contentView.topAnchor),
            viewer.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError("PreparedProseCollectionCell is programmatic") }
    override func prepareForReuse() { super.prepareForReuse(); preparedArtifactHeight = 0; viewer.prepareForReuse() }
    func configure(source: String, configuration: ProseViewerConfiguration) throws {
        guard viewer.apply(source: .json(source), configuration: configuration) else {
            throw NSError(domain: "PreparedProseCollectionCell", code: 1, userInfo: [NSLocalizedDescriptionKey: "benchmark source was rejected"])
        }
        setNeedsLayout()
    }
    override func preferredLayoutAttributesFitting(_ attributes: UICollectionViewLayoutAttributes) -> UICollectionViewLayoutAttributes {
        let fitted = attributes.copy() as! UICollectionViewLayoutAttributes
        let width = max(1, attributes.size.width)
        preparedArtifactHeight = max(1, ceil(viewer.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height))
        fitted.size = CGSize(width: width, height: preparedArtifactHeight)
        return fitted
    }
}

private enum PreparedProsePerformanceGates {
    private struct DelayedInterval: Decodable { let rawDeltaNanos: UInt64 }
    private struct CacheSnapshot: Decodable { let unmountedCurrentBytes: Int; let unmountedHighWaterBytes: Int; let unmountedCurrentResidentCount: Int; let unmountedHighWaterResidentCount: Int; let compiledCurrentBytes: Int; let compiledCurrentResidentCount: Int }
    private struct WindowEvidence: Decodable { let windowId: String; let entryIds: [String]; let phase: String; let residentKeyCount: Int; let compileCount: Int; let layoutCount: Int; let cacheMisses: Int }
    private struct Phase: Decodable { let combinedCompileLayoutNanos: [UInt64]; let cacheLookupNanos: [UInt64]; let drawNanos: [UInt64]; let rawFrameDeltasNanos: [UInt64]; let nominalFrameCount: Int; let viewerCausedDelayedIntervals: [DelayedInterval]; let imageRequestCount: Int; let imageMetadataCount: Int; let imageDecodeCount: Int; let drawCount: Int }
    private struct Export: Decodable { let percentileDefinition: String; let phaseSamples: [String: Phase]; let windowEvidence: [WindowEvidence]; let preResetSnapshot: CacheSnapshot; let postResetSnapshot: CacheSnapshot; let duplicatePublications: Int }
    static func assertPasses(exportJSON: String, expectedDocuments: Int) throws {
        let export = try JSONDecoder().decode(Export.self, from: Data(exportJSON.utf8))
        guard let cold = export.phaseSamples["cold"], let warm = export.phaseSamples["warm"], let imagesDisabled = export.phaseSamples["imagesDisabled"] else { XCTFail("every traversal phase must export samples"); return }
        requireNonEmpty(cold.combinedCompileLayoutNanos, "cold compile+layout")
        requireNonEmpty(cold.cacheLookupNanos, "cold cache lookup")
        requireNonEmpty(cold.drawNanos, "cold draw")
        XCTAssertGreaterThanOrEqual(cold.combinedCompileLayoutNanos.count, expectedDocuments)
        XCTAssertLessThan(percentile(cold.combinedCompileLayoutNanos, 0.95), 4_000_000)
        XCTAssertLessThan(percentile(cold.cacheLookupNanos, 0.99), 100_000)
        XCTAssertLessThan(percentile(cold.drawNanos, 0.95), 1_000_000)
        XCTAssertEqual(export.percentileDefinition, "nearest-rank: sorted[ceil(p*n)-1]")
        for phase in [cold, warm, imagesDisabled] {
            XCTAssertGreaterThan(phase.drawCount, 0, "phase must include actual viewer draw evidence")
            requireNonEmpty(phase.rawFrameDeltasNanos, "phase raw frame")
            XCTAssertGreaterThanOrEqual(
                Double(phase.rawFrameDeltasNanos.filter {
                    PreparedProseInstrumentation.classifyFrame(
                        rawDeltaNanos: $0,
                        nominalFramePeriodNanos: PreparedProseInstrumentation.nominalFramePeriodNanos,
                        singleTickToleranceNanos: PreparedProseInstrumentation.singleTickToleranceNanos
                    ).nominalFrameCount == 1
                }.count) / Double(phase.rawFrameDeltasNanos.count),
                0.99
            )
        }
        XCTAssertLessThanOrEqual(warm.viewerCausedDelayedIntervals.map(\.rawDeltaNanos).max() ?? 0, 33_300_000)
        XCTAssertEqual(imagesDisabled.imageRequestCount, 0)
        XCTAssertEqual(imagesDisabled.imageMetadataCount, 0)
        XCTAssertEqual(imagesDisabled.imageDecodeCount, 0)
        XCTAssertLessThanOrEqual(export.preResetSnapshot.unmountedHighWaterBytes, 32 * 1024 * 1024)
        XCTAssertEqual(export.postResetSnapshot.unmountedCurrentBytes, 0)
        XCTAssertEqual(export.postResetSnapshot.unmountedCurrentResidentCount, 0)
        XCTAssertEqual(export.postResetSnapshot.compiledCurrentBytes, 0)
        XCTAssertEqual(export.postResetSnapshot.compiledCurrentResidentCount, 0)
        let coldWindows = export.windowEvidence.filter { $0.phase == "cold" }
        let warmWindows = export.windowEvidence.filter { $0.phase == "warm" }
        XCTAssertEqual(coldWindows.count, 27)
        XCTAssertEqual(warmWindows.count, 27)
        for evidence in coldWindows + warmWindows {
            XCTAssertFalse(evidence.windowId.isEmpty)
            XCTAssertEqual(evidence.residentKeyCount, evidence.entryIds.count)
        }
        for evidence in warmWindows {
            XCTAssertEqual(evidence.compileCount, 0)
            XCTAssertEqual(evidence.layoutCount, 0)
            XCTAssertEqual(evidence.cacheMisses, 0)
        }
        XCTAssertEqual(export.duplicatePublications, 0)
    }
    private static func requireNonEmpty(_ values: [UInt64], _ name: String) { XCTAssertFalse(values.isEmpty, "\(name) evidence must be nonempty") }
    private static func percentile(_ values: [UInt64], _ percentile: Double) -> UInt64 { guard !values.isEmpty else { return .max }; return values.sorted()[max(0, Int((Double(values.count) * percentile).rounded(.up)) - 1)] }
}

private enum NativePerformanceFixtureFactory {
    private static let blockCount = 96
    private static let paragraphCharacterCount = 180

    struct ParagraphSplitSession {
        let editorId: UInt64
        let textView: EditorTextView
        let splitOffset: Int
        let initialTextLength: Int
    }

    struct HostedParagraphSplitSession {
        let editorId: UInt64
        let window: UIWindow
        let view: RichTextEditorView
        let splitOffset: Int
        let initialTextLength: Int
    }

    static func largeRenderJSON() -> String {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        return EditorV2Shadow.setJson(id: editorId, json: largeDocumentJSONString())
    }

    static func loadLargeDocument(into editorId: UInt64) -> String {
        _ = EditorV2Shadow.setJson(id: editorId, json: largeDocumentJSONString())
        return EditorV2Shadow.getCurrentState(id: editorId)
    }

    static func remoteSelections(
        editorId: UInt64,
        peerCount: Int = 6,
        selectionWidth: Int = 0
    ) -> [RemoteSelectionDecoration] {
        let totalScalar = EditorV2Shadow.docToScalar(id: editorId, docPos: editorDocumentContentSize(id: editorId))
        let upperBound = max(1, Int(totalScalar > 0 ? totalScalar - 1 : 0))
        let samplePoints = evenlySpacedValues(from: 1, through: upperBound, count: peerCount)

        return samplePoints.enumerated().map { index, scalar in
            let headScalar = (selectionWidth > 0 && !index.isMultiple(of: 2))
                ? min(upperBound, scalar + selectionWidth)
                : scalar
            let anchorDoc = EditorV2Shadow.scalarToDoc(id: editorId, scalar: UInt32(scalar))
            let headDoc = EditorV2Shadow.scalarToDoc(id: editorId, scalar: UInt32(headScalar))
            return RemoteSelectionDecoration(
                clientId: String(index + 1),
                anchor: anchorDoc,
                head: headDoc,
                color: indexedColor(index),
                name: "Peer \(index + 1)",
                isFocused: true
            )
        }
    }

    static func typingCursorOffset(in textView: UITextView) -> Int {
        selectionScrubOffsets(in: textView, points: 1).first ?? 0
    }

    static func paragraphSplitSessions(count: Int, autoGrow: Bool = false) -> [ParagraphSplitSession] {
        (0..<count).map { _ in
            let editorId = makeV2Editor()
            _ = loadLargeDocument(into: editorId)

            let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
            textView.heightBehavior = autoGrow ? .autoGrow : .fixed
            textView.captureApplyUpdateTraceForTesting = true
            textView.bindEditor(id: editorId)
            textView.layoutIfNeeded()

            return ParagraphSplitSession(
                editorId: editorId,
                textView: textView,
                splitOffset: paragraphSplitCursorOffset(in: textView),
                initialTextLength: textView.attributedText.length
            )
        }
    }

    static func hostedParagraphSplitSessions(count: Int) -> [HostedParagraphSplitSession] {
        (0..<count).map { _ in
            let editorId = makeV2Editor()

            let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 390, height: 0))
            let window = hostEditorView(view, size: CGSize(width: 390, height: 844))
            view.heightBehavior = .autoGrow
            view.textView.captureApplyUpdateTraceForTesting = true
            view.editorId = editorId
            view.setContent(json: largeDocumentJSONString())
            flushMainQueue()

            let measuredHeight = ceil(view.intrinsicContentSize.height)
            view.frame.size.height = measuredHeight
            view.layoutIfNeeded()

            return HostedParagraphSplitSession(
                editorId: editorId,
                window: window,
                view: view,
                splitOffset: paragraphSplitCursorOffset(in: view.textView),
                initialTextLength: view.textView.attributedText.length
            )
        }
    }

    static func selectionScrubOffsets(in textView: UITextView, points: Int) -> [Int] {
        let candidates = visibleCharacterOffsets(in: textView.textStorage.string as NSString)
        guard !candidates.isEmpty else { return [0] }
        return evenlySpacedValues(from: 0, through: candidates.count - 1, count: points).map { candidates[$0] }
    }

    static func paragraphSplitCursorOffset(in textView: UITextView) -> Int {
        let text = textView.textStorage.string as NSString
        let firstBlockBreak = (0..<text.length).first { index in
            let character = text.character(at: index)
            return character == 0x000A || character == 0x000D
        }

        guard let firstBlockBreak else {
            return typingCursorOffset(in: textView)
        }

        let paragraphOffsets = visibleCharacterOffsets(in: text).filter { $0 > firstBlockBreak }
        guard !paragraphOffsets.isEmpty else {
            return typingCursorOffset(in: textView)
        }

        return paragraphOffsets[min(32, paragraphOffsets.count - 1)]
    }

    private static func largeDocumentJSONString() -> String {
        let jsonObject: [String: Any] = [
            "type": "doc",
            "content": largeDocumentContent(),
        ]
        let data = try! JSONSerialization.data(withJSONObject: jsonObject, options: [])
        return String(data: data, encoding: .utf8)!
    }

    private static func largeDocumentContent() -> [[String: Any]] {
        var content: [[String: Any]] = [
            [
                "type": "h1",
                "content": [textNode(textFragment(seed: 10_000, minCharacterCount: 40))],
            ],
        ]

        for index in 0..<blockCount {
            if index > 0 && index % 18 == 0 {
                content.append(["type": "horizontalRule"])
            }

            if index % 12 == 5 {
                content.append([
                    "type": "blockquote",
                    "content": [[
                        "type": "paragraph",
                        "content": richInlineContent(seed: index, totalCharacters: paragraphCharacterCount),
                    ]],
                ])
                continue
            }

            if index % 9 == 3 {
                content.append([
                    "type": "h2",
                    "content": [textNode(textFragment(seed: index + 2_000, minCharacterCount: 72))],
                ])
                continue
            }

            content.append([
                "type": "paragraph",
                "content": richInlineContent(seed: index, totalCharacters: paragraphCharacterCount),
            ])
        }

        return content
    }

    private static func richInlineContent(seed: Int, totalCharacters: Int) -> [[String: Any]] {
        let text = textFragment(seed: seed, minCharacterCount: totalCharacters)
        let characters = Array(text)
        let count = characters.count
        let cutA = count / 4
        let cutB = count / 2
        let cutC = (count * 3) / 4

        let segments: [(String, [[String: Any]]?)] = [
            (String(characters[0..<cutA]), nil),
            (String(characters[cutA..<cutB]), [["type": "bold"]]),
            (String(characters[cutB..<cutC]), [["type": "italic"]]),
            (
                String(characters[cutC..<count]),
                [[
                    "type": "link",
                    "attrs": [
                        "href": "https://example.com/item/\(seed)",
                        "target": "_blank",
                        "rel": "noopener noreferrer nofollow",
                        "class": NSNull(),
                        "title": NSNull(),
                    ],
                ]]
            ),
        ]

        return segments.compactMap { text, marks in
            guard !text.isEmpty else { return nil }
            return textNode(text, marks: marks)
        }
    }

    private static func textNode(_ text: String, marks: [[String: Any]]? = nil) -> [String: Any] {
        var node: [String: Any] = [
            "type": "text",
            "text": text,
        ]
        if let marks, !marks.isEmpty {
            node["marks"] = marks
        }
        return node
    }

    private static func textFragment(seed: Int, minCharacterCount: Int) -> String {
        let words = [
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
            "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo",
            "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
        ]

        var result = ""
        var cursor = 0
        while result.count < minCharacterCount {
            if !result.isEmpty {
                result.append(" ")
            }
            result.append(words[(seed + cursor) % words.count])
            cursor += 1
        }
        return String(result.prefix(minCharacterCount))
    }

    private static func indexedColor(_ index: Int) -> UIColor {
        let colors: [UIColor] = [
            .systemBlue,
            .systemGreen,
            .systemOrange,
            .systemPink,
            .systemPurple,
            .systemTeal,
        ]
        return colors[index % colors.count]
    }

    private static func visibleCharacterOffsets(in text: NSString) -> [Int] {
        (0..<text.length).compactMap { index in
            switch text.character(at: index) {
            case 0xFFFC, 0x200B, 0x000A, 0x000D:
                return nil
            default:
                return index
            }
        }
    }

    private static func evenlySpacedValues(from start: Int, through end: Int, count: Int) -> [Int] {
        guard count > 1, end > start else {
            return [min(start, end)]
        }

        return (0..<count).map { index in
            start + Int((Double(end - start) * Double(index) / Double(count - 1)).rounded(.toNearestOrAwayFromZero))
        }
    }

    private static func editorDocumentContentSize(id: UInt64) -> UInt32 {
        guard let data = EditorV2Shadow.getJson(id: id).data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return 0
        }
        let children = json["content"] as? [[String: Any]] ?? []
        return children.reduce(UInt32(0)) { partial, child in
            partial + nodeSize(child)
        }
    }

    private static func nodeSize(_ node: [String: Any]) -> UInt32 {
        let type = node["type"] as? String ?? ""
        if type == "text" {
            let text = node["text"] as? String ?? ""
            return UInt32(text.count)
        }

        if isVoidNode(type) {
            return 1
        }

        let children = node["content"] as? [[String: Any]] ?? []
        let childrenSize = children.reduce(UInt32(0)) { partial, child in
            partial + nodeSize(child)
        }

        return 1 + childrenSize + 1
    }

    private static func isVoidNode(_ type: String) -> Bool {
        switch type {
        case "horizontalRule", "hardBreak", "image", "mention":
            return true
        default:
            return false
        }
    }
}

private func setSelection(in textView: UITextView, utf16Range: NSRange) {
    guard
        let start = textView.position(from: textView.beginningOfDocument, offset: utf16Range.location),
        let end = textView.position(from: start, offset: utf16Range.length),
        let range = textView.textRange(from: start, to: end)
    else {
        XCTFail("expected selection range \(utf16Range)")
        return
    }

    textView.selectedTextRange = range
}

private func hostEditorView(_ view: RichTextEditorView, size: CGSize) -> UIWindow {
    let window = UIWindow(frame: CGRect(origin: .zero, size: size))
    let viewController = UIViewController()
    window.rootViewController = viewController
    window.makeKeyAndVisible()
    view.frame = CGRect(origin: .zero, size: size)
    viewController.view.addSubview(view)
    view.layoutIfNeeded()
    return window
}

private func flushMainQueue() {
    let expectation = XCTestExpectation(description: "flush main queue")
    DispatchQueue.main.async {
        expectation.fulfill()
    }
    XCTWaiter().wait(for: [expectation], timeout: 1.0)
}
