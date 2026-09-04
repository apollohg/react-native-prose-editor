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

    func testPendingNativeTextMutationInListUsesAdjustedScalarOffsetsBeforeNextTypedCharacter() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<ul><li><p>teh </p></li></ul>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<ul><li><p>the n</p></li></ul>")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationInListMapsStaleCaretBeforeNextTypedCharacter() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<ul><li><p>teh </p></li></ul>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 4, length: 0))

        view.textView.insertText("n")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<ul><li><p>the n</p></li></ul>")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 5, length: 0))
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationInSecondListItemUsesAdjustedScalarOffsets() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 140))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<ul><li><p>one</p></li><li><p>teh </p></li></ul>")
        let correctionRange = (view.textView.textStorage.string as NSString).range(of: "teh")
        XCTAssertNotEqual(correctionRange.location, NSNotFound)
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(in: correctionRange, with: "the")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>one</p></li><li><p>the n</p></li></ul>"
        )
        XCTAssertEqual(view.textView.textStorage.string, "one\nthe n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationInNestedListUsesAdjustedScalarOffsets() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<ul><li><p>parent</p><ul><li><p>teh </p></li></ul></li></ul>")
        let correctionRange = (view.textView.textStorage.string as NSString).range(of: "teh")
        XCTAssertNotEqual(correctionRange.location, NSNotFound)
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(in: correctionRange, with: "the")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>parent</p><ul><li><p>the n</p></li></ul></li></ul>"
        )
        XCTAssertEqual(view.textView.textStorage.string, "parent\nthe n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationInTwoDigitOrderedListUsesAdjustedScalarOffsets() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<ol start=\"10\"><li><p>one</p></li><li><p>teh </p></li></ol>")
        let correctionRange = (view.textView.textStorage.string as NSString).range(of: "teh")
        XCTAssertNotEqual(correctionRange.location, NSNotFound)
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(in: correctionRange, with: "the")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ol start=\"10\"><li><p>one</p></li><li><p>the n</p></li></ol>"
        )
        XCTAssertEqual(view.textView.textStorage.string, "one\nthe n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPasteFlushesPendingNativeAutocorrectBeforePlainTextPaste() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            UIPasteboard.general.items = []
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
        setCollapsedSelection(in: view.textView, utf16Offset: 4)

        UIPasteboard.general.string = "now"
        view.textView.paste(nil)

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the now</p>")
        XCTAssertEqual(view.textView.textStorage.string, "the now")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    /// A typed word whose autocorrection has not been accepted yet — no space
    /// pressed — leaves the keyboard holding a correction that neither the
    /// engine nor the text storage has seen. Tapping the list button wraps the
    /// line first, and UIKit only then applies the correction, addressing the
    /// word through the range it was already holding.
    ///
    /// Wrapping does not change the view's text, so that range still covers
    /// the word. It does insert the list and listItem openings ahead of it, so
    /// every scalar offset inside the line has moved by two. The correction
    /// must land on the word and leave the caret after it.
    func testListToggleAppliesAPendingAutocorrectWithoutMovingTheCaret() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p></p>")
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        for character in "Ahysyc" {
            view.textView.insertText(String(character))
        }
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Ahysyc</p>")
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 6)

        view.textView.performToolbarToggleList("bullet_list", isActive: false)
        flushMainQueue()

        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>Ahysyc</p></li></ul>",
            "precondition: the line is wrapped before the correction arrives"
        )
        // Wrapping inserts the list and listItem openings ahead of the text, so
        // the end of the six-scalar line moves from scalar 6 to scalar 8.
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 8)

        // The keyboard now applies the correction it was holding. It rewrites
        // its own text storage without routing through `replace(_:withText:)`,
        // which is why the device log shows the corrected word appearing with
        // no interception of its own.
        //
        // The replacement is an NSAttributedString carrying no attributes,
        // which is what the keyboard supplies. That matters: the
        // `replaceCharacters(in:with: String)` overload would inherit the
        // replaced run's attributes and keep `listMarkerContext` on the text,
        // leaving the utf16→scalar conversion still aware of the list. The
        // keyboard's replacement strips it, which is what makes the rebuilt
        // conversion table lose the list and listItem openings.
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 6),
            with: NSAttributedString(string: "Abyss")
        )
        setCollapsedSelection(in: view.textView, utf16Offset: 5)

        // Deliberately no flush here. On device nothing observes the rewrite
        // until the next keystroke drains it, which is why the log shows the
        // insert's interception nested at depth 2 inside the drain's own.
        // Draining it separately first is what hid this.
        XCTAssertEqual(view.textView.textStorage.string, "Abyss")

        view.textView.insertText(" ")
        flushMainQueue()

        XCTAssertEqual(
            view.textView.textStorage.string,
            "Abyss ",
            "the space must land after the word, not inside it"
        )
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<ul><li><p>Abyss </p></li></ul>")
        // The caret is the reported symptom: it must end after the space, not
        // back inside the corrected word.
        assertSelectedUtf16Range(
            in: view.textView,
            NSRange(location: 6, length: 0)
        )
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 8)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testProseMirrorListToggleUsesSnakeCaseListItem() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        view.editorId = editorId
        view.setContent(html: "<p>item</p>")

        view.textView.performToolbarToggleList("bullet_list", isActive: false)

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<ul><li><p>item</p></li></ul>")
    }

    func testNativeMutationUsesUIKitSelectionAlreadyMovedBeforeCapture() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>abcdef</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.beginEditing()
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "ABC"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: 3)
        view.textView.textStorage.endEditing()

        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>ABCdef</p>")
        XCTAssertEqual(view.textView.textStorage.string, "ABCdef")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 3, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 3)

        view.textView.insertText("!")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>ABC!def</p>")
        XCTAssertEqual(view.textView.textStorage.string, "ABC!def")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 4, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 4)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPasteFlushesPendingNativeAutocorrectBeforeReplacingSelectedText() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            UIPasteboard.general.items = []
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>teh old</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 3)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        let oldRange = (view.textView.textStorage.string as NSString).range(of: "old")
        XCTAssertNotEqual(oldRange.location, NSNotFound)
        setSelection(in: view.textView, utf16Range: oldRange)

        UIPasteboard.general.string = "now"
        view.textView.paste(nil)

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the now</p>")
        XCTAssertEqual(view.textView.textStorage.string, "the now")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testHTMLPasteFlushesPendingNativeAutocorrectBeforePaste() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            UIPasteboard.general.items = []
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
        setCollapsedSelection(in: view.textView, utf16Offset: 4)

        UIPasteboard.general.setData(
            Data("<strong>now</strong>".utf8),
            forPasteboardType: "public.html"
        )
        view.textView.paste(nil)

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p><p><strong>now</strong></p>")
        XCTAssertEqual(view.textView.textStorage.string, "the \nnow")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testRTFPasteFlushesPendingNativeAutocorrectBeforePaste() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            UIPasteboard.general.items = []
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
        setCollapsedSelection(in: view.textView, utf16Offset: 4)

        let attributedPaste = NSAttributedString(
            string: "now",
            attributes: [.font: UIFont.boldSystemFont(ofSize: 14)]
        )
        let rtfData = try attributedPaste.data(
            from: NSRange(location: 0, length: attributedPaste.length),
            documentAttributes: [.documentType: NSAttributedString.DocumentType.rtf]
        )
        UIPasteboard.general.setData(rtfData, forPasteboardType: "public.rtf")
        XCTAssertNotNil(UIPasteboard.general.data(forPasteboardType: "public.rtf"))

        view.textView.paste(nil)

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(html.contains("the"), "RTF paste should preserve native correction, got: \(html)")
        XCTAssertTrue(html.contains("now"), "RTF paste should insert converted rich text, got: \(html)")
        XCTAssertTrue(view.textView.textStorage.string.contains("the"))
        XCTAssertTrue(view.textView.textStorage.string.contains("now"))
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testInterceptWindowAutocorrectCommitsBeforeImmediateNextCharacter() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>teh</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 3)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.insertText(" ")
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: 4)

        view.textView.insertText("n")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeReplaceAutocorrectWithEmojiPrefixCommitsBeforeNextCharacter() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>😀 teh </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        let start = try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 3))
        let end = try XCTUnwrap(view.textView.position(from: start, offset: 3))
        let correctionRange = try XCTUnwrap(view.textView.textRange(from: start, to: end))
        view.textView.replace(correctionRange, withText: "the")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>😀 the n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "😀 the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeEmojiReplacementAutocorrectDoesNotSplitSurrogatePairs() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>😀 test</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 2),
            with: "😁"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("!")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>😁 test!</p>")
        XCTAssertEqual(view.textView.textStorage.string, "😁 test!")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeAutocorrectAfterComplexEmojiGraphemesPreservesScalarMapping() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>👨‍👩‍👧‍👦 🇦🇺 1️⃣ teh </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        let correctionRange = (view.textView.textStorage.string as NSString).range(of: "teh")
        XCTAssertNotEqual(correctionRange.location, NSNotFound)
        view.textView.textStorage.replaceCharacters(in: correctionRange, with: "the")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>👨‍👩‍👧‍👦 🇦🇺 1️⃣ the n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "👨‍👩‍👧‍👦 🇦🇺 1️⃣ the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testLengthChangingAutocorrectAfterComplexEmojiGraphemesMapsStaleCaret() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        let prefix = "👨‍👩‍👧‍👦 🇦🇺 1️⃣ "
        view.editorId = editorId
        view.setContent(html: "<p>\(prefix)dont </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        let correctionRange = (view.textView.textStorage.string as NSString).range(of: "dont")
        XCTAssertNotEqual(correctionRange.location, NSNotFound)
        view.textView.textStorage.replaceCharacters(in: correctionRange, with: "don't")
        assertSelectedUtf16Range(
            in: view.textView,
            NSRange(location: prefix.utf16.count + 5, length: 0)
        )

        view.textView.insertText("n")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>\(prefix)don't n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "\(prefix)don't n")
        assertSelectedUtf16Range(
            in: view.textView,
            NSRange(location: view.textView.textStorage.length, length: 0)
        )
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testLengthChangingAutocorrectInvalidatesCachedPositionMappingBeforeSelectionCapture() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>dont </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        _ = PositionBridge.cursorScalarOffset(in: view.textView)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 4),
            with: "don't"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>don't n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "don't n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testLengthChangingAutocorrectMapsStaleCaretBeforeNextTypedCharacter() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>dont </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 3, length: 1),
            with: "'t"
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 5, length: 0))

        view.textView.insertText("n")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>don't n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "don't n")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 7, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 7)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testLengthShrinkingAutocorrectMapsStaleCaretBeforeNextTypedCharacter() {
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
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 7, length: 0))

        view.textView.insertText("!")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello !world</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello !world")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 7, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 7)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testSetMarkedTextFlushesPendingStaleNativeAutocorrectBeforeComposition() {
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
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 4, length: 0))

        view.textView.setMarkedText("n", selectedRange: NSRange(location: 1, length: 0))

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        view.textView.unmarkText()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlurTimeAutocorrectAfterResignStillCommitsToRust() {
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
        XCTAssertTrue(view.textView.resignFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlurTimeAutocorrectAfterNextMainQueueTurnStillCommitsToRust() {
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
        XCTAssertTrue(view.textView.resignFirstResponder())
        flushMainQueue()

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlurTimeAutocorrectAfterGracePeriodReconcilesInsteadOfCommitting() {
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
        XCTAssertTrue(view.textView.resignFirstResponder())
        view.textView.expireNativeTextMutationAfterBlurDeadlineForTesting()

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>teh </p>")
        XCTAssertEqual(view.textView.textStorage.string, "teh ")
        XCTAssertEqual(view.textView.reconciliationCount, 1)
    }

    func testRejectedBlurredMutationCannotBecomeAuthorizedAfterRefocus() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 5)
        flushMainQueue()
        XCTAssertTrue(view.textView.becomeFirstResponder())
        XCTAssertTrue(view.textView.resignFirstResponder())
        view.textView.expireNativeTextMutationAfterBlurDeadlineForTesting()

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 5, length: 0),
            with: "x"
        )
        XCTAssertEqual(view.textView.textStorage.string, "Hellox")
        XCTAssertEqual(view.textView.reconciliationCount, 1)

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.insertText("!")
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello!</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello!")
    }

    func testBlurTimeAutocorrectAfterContentReplacementReconcilesInsteadOfCommitting() {
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
        XCTAssertTrue(view.textView.resignFirstResponder())

        view.setContent(html: "<p>Remote</p>")
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: view.textView.textStorage.length),
            with: "the "
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Remote</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Remote")
        XCTAssertEqual(view.textView.reconciliationCount, 1)
    }

    func testBlurTimeAutocorrectGraceWindowIsConsumedAfterCommit() {
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
        XCTAssertTrue(view.textView.resignFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "xxx"
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 1)
    }

    func testThemeRefreshDrainsPendingNativeAutocorrectBeforeApplyingRustState() {
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

        view.textView.applyTheme(EditorTheme(dictionary: [
            "textColor": "#123456",
        ]))

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testSetEditableFalseDrainsPendingNativeAutocorrectBeforeReadOnly() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>teh </p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.setEditable(false)

        XCTAssertFalse(view.richTextView.textView.isEditable)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "the ")
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testExternalAtomicRenderAdoptionLetsTheFirstKeystrokeCommitAtTheNextRevision() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected the v2 adapter paired to the native editor")
            return
        }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>base</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)

        let revisionN = adapter.baseDocumentRevision
        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991001","baseDocumentRevision":"\#(revisionN)","command":{"type":"insertText","text":"EXT"}}"#
        )
        XCTAssertNil(external.error, "external mutation failed: \(String(describing: external.error))")
        let snapshot = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        )
        guard let externalRender = snapshot.value, snapshot.error == nil else {
            XCTFail("external render failed: \(String(describing: snapshot.error))")
            return
        }

        XCTAssertTrue(view.applyEditorUpdate(externalRender))
        XCTAssertEqual(adapter.baseDocumentRevision, revisionN + 1)

        view.richTextView.textView.insertText("!")

        XCTAssertEqual(adapter.baseDocumentRevision, revisionN + 2)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>EXT!base</p>")
        XCTAssertFalse(
            adapter.debugNotes.contains(where: { $0.contains("mismatch-refresh") }),
            "the first keystroke after an adopted external render must not race its own cache"
        )
    }

    func testExternalRenderCapturedBeforeMarkedTextPreflightRefreshesBeforeNextKeystroke() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected the v2 adapter paired to the native editor")
            return
        }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>base</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())

        let revisionN = adapter.baseDocumentRevision
        let snapshot = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        )
        guard let externalRenderAtN = snapshot.value, snapshot.error == nil else {
            XCTFail("external render failed: \(String(describing: snapshot.error))")
            return
        }

        view.richTextView.textView.setMarkedText("IME", selectedRange: NSRange(location: 3, length: 0))

        let renderCallsBeforePreflight = adapter.renderUpdateCallCountForTesting
        XCTAssertTrue(view.applyEditorUpdate(externalRenderAtN))
        XCTAssertEqual(adapter.baseDocumentRevision, revisionN + 1)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>baseIME</p>")
        XCTAssertEqual(
            adapter.renderUpdateCallCountForTesting,
            renderCallsBeforePreflight + 1,
            "the composition preflight commit must supply its already-adopted atomic render without a second refresh"
        )

        view.richTextView.textView.insertText("!")

        XCTAssertEqual(adapter.baseDocumentRevision, revisionN + 2)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>baseIME!</p>")
        XCTAssertFalse(
            adapter.debugNotes.contains(where: { $0.contains("mismatch-refresh") }),
            "the stale external render must not overwrite the post-preflight adapter revision"
        )
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }


    private func assertCollapsedEditorSelection(
        in editorId: UInt64,
        scalarOffset: UInt32,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let selection = currentSelection(in: editorId)
        let expectedDocPos = EditorV2Shadow.scalarToDoc(id: editorId, scalar: scalarOffset)
        XCTAssertEqual(selection["type"] as? String, "text", file: file, line: line)
        XCTAssertEqual((selection["anchor"] as? NSNumber)?.uint32Value, expectedDocPos, file: file, line: line)
        XCTAssertEqual((selection["head"] as? NSNumber)?.uint32Value, expectedDocPos, file: file, line: line)
    }

}
