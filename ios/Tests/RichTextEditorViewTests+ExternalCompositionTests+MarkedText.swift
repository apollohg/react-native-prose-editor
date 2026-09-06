import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
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

}
