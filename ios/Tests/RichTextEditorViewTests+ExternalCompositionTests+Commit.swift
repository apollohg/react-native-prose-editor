import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
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

}
