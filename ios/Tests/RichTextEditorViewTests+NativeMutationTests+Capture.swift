import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testUnauthorizedTextMutationReconcilesOnNextRunLoop() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        let authorizedText = textView.textStorage.string

        textView.textStorage.replaceCharacters(in: NSRange(location: 0, length: 1), with: "X")

        XCTAssertEqual(textView.reconciliationCount, 1)
        XCTAssertEqual(
            textView.textStorage.string,
            "Xello",
            "reconciliation should not run synchronously inside the text storage edit callback"
        )

        flushMainQueue()

        XCTAssertEqual(textView.textStorage.string, authorizedText)
    }

    func testFocusedNativeTextMutationCommitsToRustInsteadOfReconciling() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 6, length: 5),
            with: "there"
        )

        XCTAssertEqual(view.textView.textStorage.string, "Hello there")
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello there</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello there")
    }

    func testFocusedNativeAutocompleteInsertionCommitsToRustOnNextRunLoop() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 6)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 6, length: 0),
            with: "there"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello there</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello there")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeAutocompleteInsertionMapsStaleCaretBeforeNextTypedCharacter() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 6)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 6, length: 0),
            with: "there"
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 6, length: 0))

        view.textView.insertText("!")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello there!</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello there!")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 12, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 12)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeAutocompleteInsertionMapsStaleCaretOnScheduledCommit() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 6)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 6, length: 0),
            with: "there"
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 6, length: 0))

        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello there</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello there")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 11, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 11)

        view.textView.insertText("!")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello there!</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello there!")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 12, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 12)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeReplacementKeepsCollapsedStaleCaretCollapsedInsideReplacementRange() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>abcd </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 2)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 4),
            with: "correct"
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 2, length: 0))

        view.textView.insertText("!")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>correct! </p>")
        XCTAssertEqual(view.textView.textStorage.string, "correct! ")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 8, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 8)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testInlinePredictionMutationIsNotCommittedToRust() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>autocom</p>")

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setCollapsedSelection(in: view.textView, utf16Offset: 7)
        flushMainQueue()

        // Simulate iOS inline prediction: iOS mutates textStorage directly
        // and sets markedTextRange without calling setMarkedText.
        view.textView.setMarkedText("plete", selectedRange: NSRange(location: 5, length: 0))

        flushMainQueue()

        // The prediction text must NOT be committed to Rust — Rust state
        // should still reflect "autocom", not "autocomplete".
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>autocom</p>",
            "inline prediction text must not be committed to Rust"
        )
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        // Now the user types 'p' while prediction is active.
        // This should commit only 'p' and discard the prediction.
        view.textView.insertText("p")

        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>autocomp</p>",
            "only the typed character should be committed, not the prediction"
        )
        XCTAssertEqual(view.textView.textStorage.string, "autocomp")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testInlinePredictionDoesNotCauseReconciliation() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>hello wor</p>")

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setCollapsedSelection(in: view.textView, utf16Offset: 9)
        flushMainQueue()

        // Simulate prediction appearing: textStorage gets "ld" appended as marked text.
        view.textView.setMarkedText("ld", selectedRange: NSRange(location: 2, length: 0))

        // Prediction must be treated as transient — no reconciliation.
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        flushMainQueue()

        // After a run loop cycle, still no reconciliation and Rust unchanged.
        XCTAssertEqual(view.textView.reconciliationCount, 0)
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>hello wor</p>",
            "prediction text must not leak into Rust state"
        )
    }

    func testFocusedNativeDeletionCorrectionCommitsToRustOnNextRunLoop() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello  world</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 7)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 5, length: 1),
            with: ""
        )
        setCollapsedSelection(in: view.textView, utf16Offset: 5)

        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello world</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello world")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationFlushesBeforeNextTypedCharacter() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>teh </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        view.textView.insertText("n")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

}
