import XCTest

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

    func testPerformance_renderBridgeLargeDocument() {
        let renderJSON = NativePerformanceFixtureFactory.largeRenderJSON()
        let options = measureOptions()

        measure(metrics: [XCTClockMetric()], options: options) {
            autoreleasepool {
                let attributed = RenderBridge.renderElements(
                    fromJSON: renderJSON,
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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
                editorDestroy(id: session.editorId)
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
                XCTAssertTrue(session.textView.lastRenderAppliedPatch())
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
                editorDestroy(id: session.editorId)
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
                XCTAssertTrue(session.textView.lastRenderAppliedPatch())
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
                editorDestroy(id: session.editorId)
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
                XCTAssertTrue(session.view.textView.lastRenderAppliedPatch())
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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

    private func measureOptions() -> XCTMeasureOptions {
        let options = XCTMeasureOptions()
        options.iterationCount = 5
        return options
    }
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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        return editorSetJson(id: editorId, json: largeDocumentJSONString())
    }

    static func loadLargeDocument(into editorId: UInt64) -> String {
        _ = editorSetJson(id: editorId, json: largeDocumentJSONString())
        return editorGetCurrentState(id: editorId)
    }

    static func remoteSelections(
        editorId: UInt64,
        peerCount: Int = 6,
        selectionWidth: Int = 0
    ) -> [RemoteSelectionDecoration] {
        let totalScalar = editorDocToScalar(id: editorId, docPos: editorDocumentContentSize(id: editorId))
        let upperBound = max(1, Int(totalScalar > 0 ? totalScalar - 1 : 0))
        let samplePoints = evenlySpacedValues(from: 1, through: upperBound, count: peerCount)

        return samplePoints.enumerated().map { index, scalar in
            let headScalar = (selectionWidth > 0 && !index.isMultiple(of: 2))
                ? min(upperBound, scalar + selectionWidth)
                : scalar
            let anchorDoc = editorScalarToDoc(id: editorId, scalar: UInt32(scalar))
            let headDoc = editorScalarToDoc(id: editorId, scalar: UInt32(headScalar))
            return RemoteSelectionDecoration(
                clientId: index + 1,
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
            let editorId = editorCreate(configJson: "{}")
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
            let editorId = editorCreate(configJson: "{}")

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
        guard let data = editorGetJson(id: id).data(using: .utf8),
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
