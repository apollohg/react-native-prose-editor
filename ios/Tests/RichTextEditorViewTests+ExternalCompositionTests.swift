import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testExternalTextCompositionUpdatesViewWithoutMutatingEngine() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))

        let begin = parseJSONObject(
            textView.beginExternalTextComposition(sessionId: "speech-1")
        )
        let update = parseJSONObject(
            textView.updateExternalTextComposition(sessionId: "speech-1", text: "on arrival")
        )
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "O/A")

        XCTAssertEqual(Set(begin.keys), Set(["version", "type", "sessionId"]))
        XCTAssertEqual(begin["version"] as? Int, 1)
        XCTAssertEqual(begin["type"] as? String, "active")
        XCTAssertEqual(begin["sessionId"] as? String, "speech-1")
        XCTAssertEqual(Set(update.keys), Set(["version", "type", "sessionId"]))
        XCTAssertEqual(textView.textStorage.string, "O/A")
        XCTAssertTrue(EditorV2Shadow.getHtml(id: editorId).contains("arrival"))
        XCTAssertFalse(EditorV2Shadow.getHtml(id: editorId).contains("O/A"))
    }

    func testExternalTextCompositionUpdatesPlaceholderFromProvisionalText() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.placeholder = "Type here"
        textView.bindEditor(id: editorId, initialHTML: "<p></p>")

        XCTAssertTrue(textView.isPlaceholderVisibleForTesting())

        _ = textView.beginExternalTextComposition(sessionId: "placeholder")
        _ = textView.updateExternalTextComposition(sessionId: "placeholder", text: "draft")
        XCTAssertFalse(textView.isPlaceholderVisibleForTesting())

        _ = textView.updateExternalTextComposition(sessionId: "placeholder", text: "")
        XCTAssertTrue(textView.isPlaceholderVisibleForTesting())

        _ = textView.updateExternalTextComposition(sessionId: "placeholder", text: "draft")
        XCTAssertFalse(textView.isPlaceholderVisibleForTesting())

        _ = textView.cancelExternalTextComposition(sessionId: "placeholder", cause: "consumer")
        XCTAssertTrue(textView.isPlaceholderVisibleForTesting())
    }

    func testExternalTextCompositionExpoEventCarriesBoundEditorIdentity() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        setSelection(
            in: view.richTextView.textView,
            utf16Range: NSRange(location: 0, length: 7)
        )
        var events: [[String: Any]] = []
        view.onExternalTextCompositionEndForTesting = { events.append($0) }

        _ = view.beginExternalTextComposition(sessionId: "speech-1")
        _ = view.updateExternalTextComposition(sessionId: "speech-1", text: "on arrival")
        _ = view.commitExternalTextComposition(sessionId: "speech-1", finalText: "O/A")

        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events[0]["editorId"] as? String, String(view.richTextView.editorId))
        XCTAssertNotNil(events[0]["resultJson"] as? String)
    }

    func testExternalTextCompositionRebindCancelsInsteadOfCommitting() throws {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>arrival</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(firstEditorId)
        setSelection(
            in: view.richTextView.textView,
            utf16Range: NSRange(location: 0, length: 7)
        )
        var events: [[String: Any]] = []
        view.onExternalTextCompositionEndForTesting = { events.append($0) }
        _ = view.beginExternalTextComposition(sessionId: "speech-1")
        _ = view.updateExternalTextComposition(sessionId: "speech-1", text: "O/A")

        view.setEditorId(secondEditorId)

        XCTAssertFalse(EditorV2Shadow.getHtml(id: firstEditorId).contains("O/A"))
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events[0]["editorId"] as? String, String(firstEditorId))
    }

    func testExternalTextCompositionDestroyCancelsOnce() throws {
        let editorId = makeV2Editor()
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        setSelection(
            in: view.richTextView.textView,
            utf16Range: NSRange(location: 0, length: 7)
        )
        var events: [[String: Any]] = []
        view.onExternalTextCompositionEndForTesting = { events.append($0) }
        _ = view.beginExternalTextComposition(sessionId: "speech-1")
        _ = view.updateExternalTextComposition(sessionId: "speech-1", text: "O/A")

        NativeEditorViewRegistry.shared.destroy(editorId: editorId) {
            destroyV2Editor(id: editorId)
        }

        XCTAssertEqual(view.richTextView.editorId, 0)
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events[0]["editorId"] as? String, String(editorId))
    }

    func testExternalTextCompositionFinalUnbindCancelsOnce() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        var events: [[String: Any]] = []
        weak var releasedView: NativeEditorExpoView?

        autoreleasepool {
            let view = NativeEditorExpoView()
            releasedView = view
            let window = hostNativeEditorExpoView(view)
            view.setEditorId(editorId)
            setSelection(
                in: view.richTextView.textView,
                utf16Range: NSRange(location: 0, length: 7)
            )
            view.onExternalTextCompositionEndForTesting = { events.append($0) }
            _ = view.beginExternalTextComposition(sessionId: "speech-1")
            _ = view.updateExternalTextComposition(sessionId: "speech-1", text: "O/A")

            view.removeFromSuperview()
            XCTAssertTrue(events.isEmpty)
            view.setEditorId(0)
            XCTAssertEqual(events.count, 1)
            window.isHidden = true
        }

        XCTAssertNil(releasedView)
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events.first?["editorId"] as? String, String(editorId))
        XCTAssertFalse(EditorV2Shadow.getHtml(id: editorId).contains("O/A"))
    }

    func testExternalTextCompositionReadOnlyCancelsWithoutCommitting() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        setSelection(
            in: view.richTextView.textView,
            utf16Range: NSRange(location: 0, length: 7)
        )
        let revisionBefore = adapter.baseDocumentRevision
        var events: [[String: Any]] = []
        view.onExternalTextCompositionEndForTesting = { events.append($0) }
        _ = view.beginExternalTextComposition(sessionId: "speech-read-only")
        _ = view.updateExternalTextComposition(
            sessionId: "speech-read-only",
            text: "O/A"
        )

        view.setEditable(false)

        let event = try XCTUnwrap(events.first)
        let result = parseJSONObject(try XCTUnwrap(event["resultJson"] as? String))
        XCTAssertFalse(view.richTextView.textView.isEditable)
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(event["editorId"] as? String, String(editorId))
        XCTAssertEqual(result["outcome"] as? String, "cancelled")
        XCTAssertEqual(result["cause"] as? String, "lifecycle")
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>arrival</p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "arrival")
    }

    func testExternalTextCompositionDeinitCancelsOnceWithBoundEditorIdentity() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        var events: [[String: Any]] = []
        weak var releasedView: NativeEditorExpoView?

        autoreleasepool {
            let view = NativeEditorExpoView()
            releasedView = view
            view.setEditorId(editorId)
            setSelection(
                in: view.richTextView.textView,
                utf16Range: NSRange(location: 0, length: 7)
            )
            view.onExternalTextCompositionEndForTesting = { events.append($0) }
            _ = view.beginExternalTextComposition(sessionId: "speech-deinit")
            _ = view.updateExternalTextComposition(sessionId: "speech-deinit", text: "O/A")
        }

        let event = try XCTUnwrap(events.first)
        let result = parseJSONObject(try XCTUnwrap(event["resultJson"] as? String))
        XCTAssertNil(releasedView)
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(event["editorId"] as? String, String(editorId))
        XCTAssertEqual(result["outcome"] as? String, "cancelled")
        XCTAssertEqual(result["cause"] as? String, "lifecycle")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>arrival</p>")
    }

    func testExternalTextCompositionCommitsFinalTextOnce() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "on arrival")
        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-1",
            finalText: "O/A"
        )
        let result = parseJSONObject(resultJSON)
        let duplicate = textView.commitExternalTextComposition(
            sessionId: "speech-1",
            finalText: "ignored"
        )

        XCTAssertTrue(EditorV2Shadow.getHtml(id: editorId).contains("O/A"))
        XCTAssertEqual(
            Set(result.keys),
            Set(["version", "type", "sessionId", "outcome", "cause", "text"])
        )
        XCTAssertEqual(result["version"] as? Int, 1)
        XCTAssertEqual(result["type"] as? String, "ended")
        XCTAssertEqual(result["sessionId"] as? String, "speech-1")
        XCTAssertEqual(result["outcome"] as? String, "committed")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        XCTAssertEqual(result["text"] as? String, "O/A")
        XCTAssertEqual(duplicate, resultJSON)
        XCTAssertEqual(spy.externalCompositionEnds.count, 1)
        XCTAssertEqual(spy.receivedUpdates.count, 1)
    }

    func testExternalTextCompositionCancelRestoresAuthorizedTextAndSelection() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "O/A")
        let resultJSON = textView.cancelExternalTextComposition(
            sessionId: "speech-1",
            cause: "consumer"
        )
        let result = parseJSONObject(resultJSON)
        _ = textView.cancelExternalTextComposition(sessionId: "speech-1", cause: "consumer")

        XCTAssertEqual(textView.textStorage.string, "arrival")
        assertSelectedUtf16Range(in: textView, NSRange(location: 0, length: 7))
        XCTAssertEqual(result["outcome"] as? String, "cancelled")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        XCTAssertEqual(result["text"] as? String, "O/A")
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
        XCTAssertTrue(spy.receivedUpdates.isEmpty)
    }

    func testExternalTextCompositionCancelRestoresCurrentStateAfterDirectMutation() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>abc</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 3))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy

        _ = textView.beginExternalTextComposition(sessionId: "speech-current")
        _ = textView.updateExternalTextComposition(sessionId: "speech-current", text: "draft")
        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991103","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"Z"}}"#
        )
        XCTAssertNil(external.error)
        let stateBeforeCancel = try XCTUnwrap(editorV2GetState(editorId: adapter.editorId).value)

        let resultJSON = textView.cancelExternalTextComposition(
            sessionId: "speech-current",
            cause: "consumer"
        )
        let duplicate = textView.cancelExternalTextComposition(
            sessionId: "speech-current",
            cause: "consumer"
        )
        let result = parseJSONObject(resultJSON)

        XCTAssertEqual(result["outcome"] as? String, "cancelled")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")
        XCTAssertEqual(textView.textStorage.string, "Zabc")
        XCTAssertEqual(editorV2GetState(editorId: adapter.editorId).value, stateBeforeCancel)
        XCTAssertLessThanOrEqual(NSMaxRange(textView.selectedRange), textView.textStorage.length)
        XCTAssertEqual(duplicate, resultJSON)
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
        XCTAssertTrue(spy.receivedUpdates.isEmpty)
    }

    func testExternalTextCompositionNoOpCommitPreservesRevisionAndUndoHistory() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))
        let revisionBefore = adapter.baseDocumentRevision
        let historyBefore = try XCTUnwrap(adapter.historyFlags())

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "arrival")
        let result = parseJSONObject(
            textView.commitExternalTextComposition(sessionId: "speech-1", finalText: "arrival")
        )

        XCTAssertEqual(result["outcome"] as? String, "committed")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>arrival</p>")
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore)
        XCTAssertEqual(adapter.historyFlags()?.canUndo, historyBefore.canUndo)
        XCTAssertEqual(adapter.historyFlags()?.canRedo, historyBefore.canRedo)
    }

    func testExternalTextCompositionEmptyFinalTextDeletesTheSelectedRange() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))
        let revisionBefore = adapter.baseDocumentRevision

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "draft")
        let result = parseJSONObject(
            textView.commitExternalTextComposition(sessionId: "speech-1", finalText: "")
        )

        XCTAssertEqual(result["outcome"] as? String, "committed")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p></p>")
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore + 1)
    }

    func testExternalTextCompositionCommitUsesExplicitFinalText() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "latest partial")
        let result = parseJSONObject(
            textView.commitExternalTextComposition(sessionId: "speech-1", finalText: "O/A")
        )

        XCTAssertEqual(result["text"] as? String, "O/A")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>O/A</p>")
        XCTAssertEqual(textView.textStorage.string, "O/A")
    }

    func testExternalTextCompositionReplacesUnicodeSelectionsByScalarRange() {
        let cases: [(source: String, range: NSRange, replacement: String, expected: String)] = [
            ("A\u{1F600}B", NSRange(location: 1, length: 2), "X", "AXB"),
            ("Cafe\u{301}", NSRange(location: 3, length: 2), "Z", "CafZ"),
            ("abc אבג def", NSRange(location: 4, length: 3), "RTL", "abc RTL def"),
        ]

        for (index, testCase) in cases.enumerated() {
            let editorId = makeV2Editor()
            defer { destroyV2Editor(id: editorId) }
            let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
            textView.bindEditor(id: editorId, initialHTML: "<p>\(testCase.source)</p>")
            setSelection(in: textView, utf16Range: testCase.range)
            let sessionId = "speech-\(index)"

            _ = textView.beginExternalTextComposition(sessionId: sessionId)
            _ = textView.updateExternalTextComposition(
                sessionId: sessionId,
                text: "transient \u{1F642}"
            )
            _ = textView.commitExternalTextComposition(
                sessionId: sessionId,
                finalText: testCase.replacement
            )

            XCTAssertEqual(textView.textStorage.string, testCase.expected)
        }
    }

    func testExternalTextCompositionRejectsNonTextSelection() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        let selectionResult = editorV2SetSelection(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991101","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","selection":{"type":"all"}}"#
        )
        XCTAssertNil(selectionResult.error)

        let result = parseJSONObject(textView.beginExternalTextComposition(sessionId: "speech-1"))

        XCTAssertEqual(result["type"] as? String, "error")
        assertExternalCompositionError(
            result,
            code: "EXTERNAL_COMPOSITION_SELECTION_INCOMPATIBLE"
        )
    }

    func testExternalTextCompositionRejectsReadOnlyEditor() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        textView.isEditable = false

        let result = parseJSONObject(textView.beginExternalTextComposition(sessionId: "speech-1"))

        XCTAssertEqual(result["type"] as? String, "error")
        assertExternalCompositionError(result, code: "EXTERNAL_COMPOSITION_UNAVAILABLE")
    }

    func testExternalTextCompositionSecondSessionCommitsFirstForConsumer() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "draft")
        let second = parseJSONObject(textView.beginExternalTextComposition(sessionId: "speech-2"))

        XCTAssertEqual(second["type"] as? String, "active")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>draft</p>")
        XCTAssertEqual(spy.externalCompositionEnds.count, 1)
        let firstEnd = parseJSONObject(spy.externalCompositionEnds[0])
        XCTAssertEqual(firstEnd["sessionId"] as? String, "speech-1")
        XCTAssertEqual(firstEnd["outcome"] as? String, "committed")
        XCTAssertEqual(firstEnd["cause"] as? String, "consumer")
        _ = textView.cancelExternalTextComposition(sessionId: "speech-2", cause: "consumer")
    }

    func testExternalTextCompositionTypingCommitsBeforeInteraction() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "draft")
        textView.insertText("!")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>draft!</p>")
        XCTAssertEqual(spy.externalCompositionEnds.count, 1)
        XCTAssertEqual(parseJSONObject(spy.externalCompositionEnds[0])["cause"] as? String, "interaction")
    }

    func testExternalTextCompositionUnmarkCommitsBeforeInteractionOnce() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy

        _ = textView.beginExternalTextComposition(sessionId: "speech-unmark")
        _ = textView.updateExternalTextComposition(sessionId: "speech-unmark", text: "draft")
        textView.unmarkText()
        let resultJSON = try XCTUnwrap(spy.externalCompositionEnds.first)
        let duplicate = textView.commitExternalTextComposition(
            sessionId: "speech-unmark",
            finalText: "draft"
        )
        let result = parseJSONObject(resultJSON)

        XCTAssertEqual(result["outcome"] as? String, "committed")
        XCTAssertEqual(result["cause"] as? String, "interaction")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>draft</p>")
        XCTAssertEqual(textView.textStorage.string, "draft")
        XCTAssertEqual(duplicate, resultJSON)
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
    }

    func testExternalTextCompositionSelectionMovementCommitsBeforeInteraction() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "draft")
        setCollapsedSelection(in: textView, utf16Offset: 0)
        textView.delegate?.textViewDidChangeSelection?(textView)

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>draft</p>")
        XCTAssertEqual(spy.externalCompositionEnds.count, 1)
        XCTAssertEqual(parseJSONObject(spy.externalCompositionEnds[0])["cause"] as? String, "interaction")
        assertSelectedUtf16Range(in: textView, NSRange(location: 0, length: 0))
    }

    func testExternalTextCompositionExternalMutationPreflightCommitsForDocumentChange() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "draft")

        XCTAssertTrue(textView.prepareForExternalEditorUpdate())
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>draft</p>")
        XCTAssertEqual(spy.externalCompositionEnds.count, 1)
        XCTAssertEqual(parseJSONObject(spy.externalCompositionEnds[0])["cause"] as? String, "documentChange")
    }

    func testExternalTextCompositionRebindCancelsForLifecycle() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: firstEditorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "draft")

        textView.bindEditor(id: secondEditorId, initialHTML: "<p>other</p>")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: firstEditorId), "<p>arrival</p>")
        XCTAssertEqual(textView.textStorage.string, "other")
        XCTAssertEqual(spy.externalCompositionEnds.count, 1)
        let end = parseJSONObject(spy.externalCompositionEnds[0])
        XCTAssertEqual(end["outcome"] as? String, "cancelled")
        XCTAssertEqual(end["cause"] as? String, "lifecycle")
    }

    func testExternalTextCompositionMaximumLengthFailureCancelsAndRestores() throws {
        try assertExternalCompositionCommitFailureRestores(
            configJSON: #"{"initialization":{"type":"localEmpty"},"policy":{"maxLength":3}}"#,
            initialText: "ab",
            finalText: "long"
        )
    }

    func testExternalTextCompositionInputFilterFailureCancelsAndRestores() throws {
        let creation = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"},"policy":{"inputFilter":"[unclosed"}}"#,
            snapshotState: nil
        )
        let handle = try XCTUnwrap(creation.value.flatMap(createdV2TestEditorHandle))
        var collaborationWakes: [CollaborationWakeReason] = []
        let adapter = try XCTUnwrap(
            EditorV2Adapter.attach(
                editorId: handle.handle,
                roomBound: true,
                collaborationWake: { _, reason in
                    collaborationWakes.append(reason)
                }
            )
        )
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        defer { EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: handle.nativeViewId, initialHTML: "<p>12</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 2))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        let stateBefore = try XCTUnwrap(editorV2GetState(editorId: handle.handle).value)
        let revisionBefore = adapter.baseDocumentRevision
        let historyBefore = try XCTUnwrap(adapter.historyFlags())
        collaborationWakes.removeAll()

        _ = textView.beginExternalTextComposition(sessionId: "speech-filter")
        _ = textView.updateExternalTextComposition(
            sessionId: "speech-filter",
            text: "letters"
        )
        XCTAssertTrue(collaborationWakes.isEmpty)
        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-filter",
            finalText: "letters"
        )
        let result = parseJSONObject(resultJSON)
        let duplicate = textView.commitExternalTextComposition(
            sessionId: "speech-filter",
            finalText: "ignored"
        )

        XCTAssertEqual(result["outcome"] as? String, "cancelled")
        assertExternalCompositionError(result, code: "EXTERNAL_COMPOSITION_COMMIT_FAILED")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: handle.nativeViewId), "<p>12</p>")
        XCTAssertEqual(textView.textStorage.string, "12")
        XCTAssertEqual(editorV2GetState(editorId: handle.handle).value, stateBefore)
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore)
        XCTAssertEqual(adapter.historyFlags()?.canUndo, historyBefore.canUndo)
        XCTAssertEqual(adapter.historyFlags()?.canRedo, historyBefore.canRedo)
        XCTAssertTrue(collaborationWakes.isEmpty)
        XCTAssertEqual(duplicate, resultJSON)
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
        XCTAssertTrue(spy.receivedUpdates.isEmpty)
    }

    func testExternalTextCompositionMergesRemoteFirstMutationThroughDeferredRegistryRefresh() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>abc</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        let textView = view.richTextView.textView
        setSelection(in: textView, utf16Range: NSRange(location: 1, length: 1))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "X")

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991102","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"Z"}}"#
        )
        XCTAssertNil(external.error)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")

        NativeEditorViewRegistry.shared.applyRemoteCommitRefresh(editorId: editorId)

        XCTAssertEqual(textView.textStorage.string, "aXc")
        XCTAssertTrue(spy.receivedUpdates.isEmpty)

        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-1",
            finalText: "Y"
        )
        let result = parseJSONObject(resultJSON)

        XCTAssertEqual(result["outcome"] as? String, "committed")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        XCTAssertNil(result["error"])
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>ZaYc</p>")
        XCTAssertEqual(textView.textStorage.string, "ZaYc")
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
        XCTAssertEqual(spy.receivedUpdates.count, 1)
    }

    func testExternalTextCompositionRemoteFirstNoOpAdoptsRenderWithoutLocalUpdate() throws {
        let creation = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"},"policy":{"inputFilter":"[0-9]"}}"#,
            snapshotState: nil
        )
        let handle = try XCTUnwrap(creation.value.flatMap(createdV2TestEditorHandle))
        let adapter = try XCTUnwrap(
            EditorV2Adapter.attach(editorId: handle.handle, roomBound: false)
        )
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        defer { EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId) }
        _ = EditorV2Shadow.setHtml(id: handle.nativeViewId, html: "<p>123</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(handle.nativeViewId)
        let textView = view.richTextView.textView
        setSelection(in: textView, utf16Range: NSRange(location: 1, length: 1))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        _ = textView.beginExternalTextComposition(sessionId: "speech-noop")
        _ = textView.updateExternalTextComposition(sessionId: "speech-noop", text: "X")

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991104","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"4"}}"#
        )
        XCTAssertNil(external.error)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: handle.nativeViewId), "<p>4123</p>")
        let remoteOutcome = parseJSONObject(try XCTUnwrap(external.value))
        let remoteRevision = try XCTUnwrap(remoteOutcome["documentRevision"] as? String)
        NativeEditorViewRegistry.shared.applyRemoteCommitRefresh(editorId: handle.nativeViewId)
        XCTAssertTrue(spy.receivedUpdates.isEmpty)

        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-noop",
            finalText: "letters"
        )
        let result = parseJSONObject(resultJSON)

        XCTAssertEqual(result["outcome"] as? String, "committed")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        XCTAssertNil(result["error"])
        XCTAssertEqual(EditorV2Shadow.getHtml(id: handle.nativeViewId), "<p>4123</p>")
        XCTAssertEqual(
            parseJSONObject(EditorV2Shadow.getCurrentState(id: handle.nativeViewId))["documentVersion"] as? String,
            remoteRevision
        )
        XCTAssertEqual(textView.textStorage.string, "4123")
        XCTAssertEqual(
            spy.externalCompositionEnds,
            [resultJSON],
            "the successful no-op commit must emit one terminal event"
        )
        XCTAssertTrue(
            spy.receivedUpdates.isEmpty,
            "the remote render must not be reported as a local content update; got \(spy.receivedUpdates.count)"
        )
    }

    func testExternalTextCompositionReleasedPositionEpochCancelsAndRestoresAuthoritativeRender() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>abc</p>")
        let view = NativeEditorExpoView()
        var hostErrors: [[String: Any]] = []
        view.onEditorErrorForTesting = { hostErrors.append($0) }
        view.setEditorId(editorId)
        let textView = view.richTextView.textView
        setSelection(in: textView, utf16Range: NSRange(location: 1, length: 1))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        _ = textView.beginExternalTextComposition(sessionId: "speech-invalid-epoch")
        _ = textView.updateExternalTextComposition(
            sessionId: "speech-invalid-epoch",
            text: "X"
        )

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991105","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"Z"}}"#
        )
        XCTAssertNil(external.error)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")
        let authoritativeState = try XCTUnwrap(editorV2GetState(editorId: adapter.editorId).value)
        let requestBeforeCommit = adapter.lastRequestIdForTesting ?? 0
        XCTAssertTrue(adapter.releaseCurrentNativeOwnerInRustForTesting())

        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-invalid-epoch",
            finalText: "Y"
        )
        let result = parseJSONObject(resultJSON)

        XCTAssertEqual(result["outcome"] as? String, "cancelled")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        assertExternalCompositionError(result, code: "EXTERNAL_COMPOSITION_COMMIT_FAILED")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")
        XCTAssertEqual(textView.textStorage.string, "Zabc")
        XCTAssertEqual(adapter.lastRequestIdForTesting, requestBeforeCommit + 1)
        XCTAssertEqual(editorV2GetState(editorId: adapter.editorId).value, authoritativeState)
        XCTAssertTrue(spy.receivedUpdates.isEmpty)
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
        flushMainQueue()
        XCTAssertEqual(hostErrors.count, 1)
        XCTAssertEqual(hostErrors.first?["editorId"] as? String, adapter.editorId)
        let hostError = try XCTUnwrap(hostErrors.first?["error"] as? [String: Any])
        XCTAssertEqual(hostError["code"] as? String, "POSITION_EPOCH_INVALID")
    }

    func testExternalTextCompositionRemoteFirstCollapsedEmptyRemapsCaretWithoutLocalUpdate() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>abc</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        let textView = view.richTextView.textView
        setSelection(in: textView, utf16Range: NSRange(location: 2, length: 0))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        _ = textView.beginExternalTextComposition(sessionId: "speech-remote-empty")
        _ = textView.updateExternalTextComposition(
            sessionId: "speech-remote-empty",
            text: "X"
        )

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991103","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"Z"}}"#
        )
        XCTAssertNil(external.error)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")
        let remoteOutcome = parseJSONObject(try XCTUnwrap(external.value))
        let revisionBeforeCommit = try XCTUnwrap(remoteOutcome["documentRevision"] as? String)
        let requestBeforeCommit = adapter.lastRequestIdForTesting ?? 0

        NativeEditorViewRegistry.shared.applyRemoteCommitRefresh(editorId: editorId)

        XCTAssertEqual(textView.textStorage.string, "abXc")
        spy.receivedUpdates.removeAll()
        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-remote-empty",
            finalText: ""
        )
        let result = parseJSONObject(resultJSON)

        XCTAssertEqual(result["outcome"] as? String, "committed")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        XCTAssertNil(result["error"])
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")
        XCTAssertEqual(textView.textStorage.string, "Zabc")
        XCTAssertEqual(textView.selectedRange, NSRange(location: 3, length: 0))
        XCTAssertEqual(adapter.lastRequestIdForTesting, requestBeforeCommit + 1)
        XCTAssertEqual(
            parseJSONObject(EditorV2Shadow.getCurrentState(id: editorId))["documentVersion"] as? String,
            revisionBeforeCommit
        )
        XCTAssertTrue(spy.receivedUpdates.isEmpty)
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
    }

    func testMarkedTextDoesNotReconcileWhileCompositionIsTransient() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))

        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
        XCTAssertEqual(textView.reconciliationCount, 0)
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>Hello world</p>",
            "marked text should stay visible-only until the IME commits it"
        )
    }

    func testUnmarkTextCommitsAtOriginalAuthorizedOffset() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))
        textView.unmarkText()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
    }

    func testUnmarkTextReplacesOriginalAuthorizedSelection() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 6, length: 5))

        textView.setMarkedText("there", selectedRange: NSRange(location: 5, length: 0))
        textView.unmarkText()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello there</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello there")
    }

    func testSetMarkedTextNilCommitsVisibleComposition() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))
        textView.setMarkedText(nil, selectedRange: NSRange(location: 0, length: 0))

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
        XCTAssertEqual(textView.authorizedTextForTesting(), "Hello brave world")
    }

    func testSetMarkedTextNilCommitsEmptyReplacementOverOriginalSelection() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 6, length: 5))

        textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        textView.setMarkedText(nil, selectedRange: NSRange(location: 0, length: 0))

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello </p>")
        XCTAssertEqual(textView.textStorage.string, "Hello ")
        XCTAssertEqual(textView.authorizedTextForTesting(), "Hello ")
    }

    func testExternalUpdatePreflightCommitsActiveCompositionOnce() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))

        XCTAssertTrue(textView.applyTheme(EditorTheme(dictionary: ["textColor": "#123456"])))
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")

        textView.unmarkText()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
    }

    func testToolbarCommandsCommitActiveMarkedCompositionBeforeMutatingEditor() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))
        textView.performToolbarToggleMark("bold")

        XCTAssertTrue(
            EditorV2Shadow.getHtml(id: editorId).contains("Hello brave world"),
            "toolbar mark command should commit the active composition before mutating the editor"
        )
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
        XCTAssertEqual(textView.reconciliationCount, 0)

        setCollapsedSelection(in: textView, utf16Offset: textView.textStorage.length)
        textView.setMarkedText("!", selectedRange: NSRange(location: 1, length: 0))
        textView.performToolbarInsertNode("horizontal_rule")

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(html.contains("Hello brave world"), "toolbar node insert should preserve the earlier composed text, got: \(html)")
        XCTAssertTrue(html.contains("!"), "toolbar node insert should preserve the newly composed text, got: \(html)")
        XCTAssertTrue(html.contains("<hr>"), "toolbar node insert should still apply after the composition drain, got: \(html)")
        XCTAssertEqual(textView.reconciliationCount, 0)
    }

    func testExternalUpdatePreflightCommitsEmptySelectedCompositionAsDeletion() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 6, length: 5))

        textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        XCTAssertTrue(textView.applyTheme(EditorTheme(dictionary: ["textColor": "#123456"])))
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello </p>")
        XCTAssertEqual(textView.textStorage.string, "Hello ")

        textView.unmarkText()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello </p>")
        XCTAssertEqual(textView.textStorage.string, "Hello ")
    }

    func testInsertTextDuringMarkedCompositionUsesOriginalReplacementRange() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brav", selectedRange: NSRange(location: 4, length: 0))
        textView.insertText("brave ")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
    }

    func testReturnDuringMarkedCorrectionCommitsCorrectionThenSplitsListItem() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 180))
        textView.bindEditor(
            id: editorId,
            initialHTML: "<ul><li><p>wrd</p></li><li><p>Next</p></li></ul>"
        )
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 3))

        textView.setMarkedText("word", selectedRange: NSRange(location: 4, length: 0))
        textView.insertText("\n")

        XCTAssertEqual(textView.textStorage.string, "word\n\u{200B}\nNext")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>word</p></li><li><p></p></li><li><p>Next</p></li></ul>"
        )

        textView.insertText("x")

        XCTAssertEqual(textView.textStorage.string, "word\nx\nNext")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>word</p></li><li><p>x</p></li><li><p>Next</p></li></ul>"
        )

        textView.deleteBackward()

        XCTAssertEqual(textView.textStorage.string, "word\n\u{200B}\nNext")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>word</p></li><li><p></p></li><li><p>Next</p></li></ul>"
        )
    }

    func testUpdatedMarkedTextStillUsesOriginalAuthorizedOffset() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("abc ", selectedRange: NSRange(location: 3, length: 0))
        textView.setMarkedText("ab ", selectedRange: NSRange(location: 3, length: 0))

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello world</p>")

        textView.unmarkText()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello ab world</p>")
    }

    func testDeleteBackwardDuringMarkedCompositionDoesNotMutateRust() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("abc ", selectedRange: NSRange(location: 3, length: 0))
        textView.deleteBackward()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello world</p>")

        textView.unmarkText()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello world</p>")
    }


    private func assertExternalCompositionCommitFailureRestores(
        configJSON: String,
        initialText: String,
        finalText: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let editorId = makeV2Editor(configJson: configJSON, file: file, line: line)
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(
            EditorV2Registry.adapter(forLegacyId: editorId),
            file: file,
            line: line
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>\(initialText)</p>")
        setSelection(
            in: textView,
            utf16Range: NSRange(location: 0, length: (initialText as NSString).length)
        )
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        let revisionBefore = adapter.baseDocumentRevision
        let historyBefore = try XCTUnwrap(adapter.historyFlags(), file: file, line: line)

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: finalText)
        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-1",
            finalText: finalText
        )
        let result = parseJSONObject(resultJSON)
        let duplicate = textView.commitExternalTextComposition(
            sessionId: "speech-1",
            finalText: "ignored"
        )

        XCTAssertEqual(result["outcome"] as? String, "cancelled", file: file, line: line)
        assertExternalCompositionError(
            result,
            code: "EXTERNAL_COMPOSITION_COMMIT_FAILED",
            file: file,
            line: line
        )
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>\(initialText)</p>",
            file: file,
            line: line
        )
        XCTAssertEqual(textView.textStorage.string, initialText, file: file, line: line)
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore, file: file, line: line)
        XCTAssertEqual(adapter.historyFlags()?.canUndo, historyBefore.canUndo, file: file, line: line)
        XCTAssertEqual(adapter.historyFlags()?.canRedo, historyBefore.canRedo, file: file, line: line)
        XCTAssertEqual(duplicate, resultJSON, file: file, line: line)
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON], file: file, line: line)
        XCTAssertTrue(spy.receivedUpdates.isEmpty, file: file, line: line)
    }

    private func assertExternalCompositionError(
        _ result: [String: Any],
        code: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        guard let error = result["error"] as? [String: Any] else {
            XCTFail("expected external composition error", file: file, line: line)
            return
        }
        XCTAssertEqual(
            Set(error.keys),
            Set([
                "domain",
                "code",
                "message",
                "requestId",
                "operationIndex",
                "limit",
                "actual",
                "details",
            ]),
            file: file,
            line: line
        )
        XCTAssertEqual(error["domain"] as? String, "lifecycle", file: file, line: line)
        XCTAssertEqual(error["code"] as? String, code, file: file, line: line)
        XCTAssertNotNil(error["message"] as? String, file: file, line: line)
        for key in ["requestId", "operationIndex", "limit", "actual", "details"] {
            XCTAssertTrue(error[key] is NSNull, "expected null \(key)", file: file, line: line)
        }
    }

}
