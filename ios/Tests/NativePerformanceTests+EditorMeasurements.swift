import XCTest
import UIKit

extension NativePerformanceTests {
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

}
