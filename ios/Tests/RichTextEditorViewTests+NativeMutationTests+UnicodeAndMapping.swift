import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
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

}
