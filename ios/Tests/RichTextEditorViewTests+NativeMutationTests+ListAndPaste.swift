import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testBackspaceAtListItemStartAfterNestedListUnwrapsIntoParagraph() {
        for tag in ["ul", "ol"] {
            let editorId = makeV2Editor()
            defer { destroyV2Editor(id: editorId) }
            let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
            let window = hostEditorView(view)
            defer {
                view.removeFromSuperview()
                window.isHidden = true
            }
            view.editorId = editorId
            view.setContent(html: "<\(tag)><li><p>Parent</p><\(tag)><li><p>Nested</p></li></\(tag)></li><li><p>Last</p></li></\(tag)>")
            let lastRange = (view.textView.textStorage.string as NSString).range(of: "Last")
            XCTAssertNotEqual(lastRange.location, NSNotFound)
            setCollapsedSelection(in: view.textView, utf16Offset: lastRange.location)
            flushMainQueue()
            view.textView.deleteBackward()

            XCTAssertEqual(
                EditorV2Shadow.getHtml(id: editorId),
                "<\(tag)><li><p>Parent</p><\(tag)><li><p>Nested</p></li></\(tag)></li></\(tag)><p>Last</p>"
            )
            view.textView.insertText("!")
            XCTAssertTrue(EditorV2Shadow.getHtml(id: editorId).contains("<p>!Last</p>"))
        }
    }

    func testRepeatedBackspaceAfterNestedListContinuesThroughParagraph() {
        for tag in ["ul", "ol"] {
            let editorId = makeV2Editor()
            defer { destroyV2Editor(id: editorId) }
            let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
            let window = hostEditorView(view)
            defer {
                view.removeFromSuperview()
                window.isHidden = true
            }
            view.editorId = editorId
            view.setContent(html: "<\(tag)><li><p>Parent</p><\(tag)><li><p>Nested</p></li></\(tag)></li><li><p>Last</p></li></\(tag)>")
            let lastRange = (view.textView.textStorage.string as NSString).range(of: "Last")
            XCTAssertNotEqual(lastRange.location, NSNotFound)
            setCollapsedSelection(in: view.textView, utf16Offset: lastRange.location)
            flushMainQueue()
            view.textView.deleteBackward()
            view.textView.deleteBackward()
            XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId),
                "<\(tag)><li><p>Parent</p><\(tag)><li><p>NestedLast</p></li></\(tag)></li></\(tag)>")
            view.textView.insertText("!")
            XCTAssertTrue(EditorV2Shadow.getHtml(id: editorId).contains("Nested!Last"))
            for _ in 0..<32 {
                let before = EditorV2Shadow.getHtml(id: editorId)
                if before == "<p>Last</p>" { break }
                view.textView.deleteBackward()
                XCTAssertNotEqual(EditorV2Shadow.getHtml(id: editorId), before)
            }
            XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Last</p>")
        }
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

}
