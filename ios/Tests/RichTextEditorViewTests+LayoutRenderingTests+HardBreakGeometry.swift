import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testReturnInsideBlockquoteAfterPlainParagraphKeepsOneStripeGroup() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 260))
        textView.bindEditor(id: editorId, initialHTML: "<p>Intro</p><blockquote><p>Hello</p></blockquote>")
        textView.layoutIfNeeded()

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 11, scalarHead: 11)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.insertText("\n")
        textView.layoutIfNeeded()

        let stripeRects = textView.blockquoteStripeRectsForTesting()
        XCTAssertEqual(
            stripeRects.count,
            1,
            "pressing Return inside a blockquote should not split the quote stripe when the quote follows plain content"
        )
        XCTAssertGreaterThan(
            stripeRects[0].minY,
            0.5,
            "quote stripe should start within the blockquote, not at the preceding paragraph"
        )
        XCTAssertLessThan(
            stripeRects[0].height,
            60.0,
            "quote stripe should stop at the quoted content, not the paragraph spacing below it"
        )
    }

    func testBlockquoteHardBreakAndFollowingParagraphShareOneStripeGroup() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 260))
        textView.bindEditor(
            id: editorId,
            initialHTML: "<blockquote><p>Hello<br>World</p><p>Tail</p></blockquote>"
        )
        textView.layoutIfNeeded()

        let stripeRects = textView.blockquoteStripeRectsForTesting()
        XCTAssertEqual(
            stripeRects.count,
            1,
            "hard breaks inside a blockquote should not split the quote stripe from later quoted content"
        )
        XCTAssertGreaterThan(
            stripeRects[0].height,
            60.0,
            "quote stripe should extend through the hard-break line and following quoted paragraph"
        )
    }

    func testTrailingHardBreakInBlockquoteKeepsStripeConnectedToFollowingParagraph() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 260))
        textView.bindEditor(
            id: editorId,
            initialHTML: "<blockquote><p>Hello<br></p><p>Tail</p></blockquote>"
        )
        textView.layoutIfNeeded()

        let stripeRects = textView.blockquoteStripeRectsForTesting()
        XCTAssertEqual(
            stripeRects.count,
            1,
            "a trailing hard break inside a blockquote should not split the quote stripe from the following quoted paragraph"
        )
        XCTAssertGreaterThan(
            stripeRects[0].height,
            40.0,
            "quote stripe should extend through the trailing hard-break line and following quoted paragraph"
        )
    }

    func testCaretRectAtParagraphStartDoesNotDropByOneLineHeight() {
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
        ])
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "First paragraph.", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Second paragraph starts here.", "marks": []},
            {"type": "blockEnd"}
        ]
        """

        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label,
            theme: theme
        )

        let secondParagraphOffset = (attributed.string as NSString).range(of: "Second").location
        XCTAssertNotEqual(secondParagraphOffset, NSNotFound)

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 240))
        let plainTextView = UITextView(frame: CGRect(x: 0, y: 0, width: 220, height: 240))
        textView.attributedText = attributed
        plainTextView.attributedText = attributed
        textView.layoutIfNeeded()
        plainTextView.layoutIfNeeded()

        guard
            let position = textView.position(from: textView.beginningOfDocument, offset: secondParagraphOffset),
            let plainPosition = plainTextView.position(from: plainTextView.beginningOfDocument, offset: secondParagraphOffset)
        else {
            XCTFail("expected caret positions at paragraph start")
            return
        }

        let caretRect = textView.caretRect(for: position)
        let plainCaretRect = plainTextView.caretRect(for: plainPosition)
        let expected = expectedCaretRect(
            in: plainTextView,
            offset: secondParagraphOffset,
            referenceRect: plainCaretRect,
            font: UIFont.systemFont(ofSize: 16)
        )

        XCTAssertEqual(caretRect.origin.y, expected.origin.y, accuracy: 1.0)
        XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0)
    }

    func testDirectScalarHardBreakTwiceInListItemPreservesExistingText() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<ul><li><p>A</p></li></ul>")

        let firstUpdate = EditorV2Shadow.insertNodeAtSelectionScalar(
            id: editorId,
            scalarAnchor: 3,
            scalarHead: 3,
            nodeType: "hard_break"
        )
        XCTAssertFalse(firstUpdate.isEmpty)
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>A<br></p></li></ul>",
            "first hardBreak should preserve the existing list item text"
        )

        let secondUpdate = EditorV2Shadow.insertNodeAtSelectionScalar(
            id: editorId,
            scalarAnchor: 4,
            scalarHead: 4,
            nodeType: "hard_break"
        )
        XCTAssertFalse(secondUpdate.isEmpty)
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>A<br><br></p></li></ul>",
            "second hardBreak at the next scalar position should preserve the original text"
        )
    }

    func testToolbarHardBreakTwiceInListItemPreservesExistingText() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: .zero)
        textView.bindEditor(id: editorId, initialHTML: "<ul><li><p>A</p></li></ul>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 3, scalarHead: 3)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        textView.performToolbarInsertNode("hard_break")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>A<br></p></li></ul>",
            "first hardBreak should preserve the existing list item text"
        )

        textView.performToolbarInsertNode("hard_break")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>A<br><br></p></li></ul>",
            "second hardBreak should append after the first one rather than replacing the text"
        )
    }

    func testToolbarHardBreakMovesCaretToNextVisualLine() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
        ])

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.applyTheme(theme)
        textView.bindEditor(id: editorId, initialHTML: "<p>A</p>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 1, scalarHead: 1)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        guard let beforePosition = textView.selectedTextRange?.start else {
            XCTFail("expected initial caret position")
            return
        }
        let beforeCaretRect = textView.caretRect(for: beforePosition)

        textView.performToolbarInsertNode("hard_break")
        textView.layoutIfNeeded()

        let selectionOffset = textView.offset(
            from: textView.beginningOfDocument,
            to: textView.selectedTextRange?.start ?? textView.endOfDocument
        )
        XCTAssertEqual(selectionOffset, 2, "caret should land immediately after the inserted hard break")

        guard let afterPosition = textView.selectedTextRange?.start else {
            XCTFail("expected caret position after hard break")
            return
        }
        let caretRect = textView.caretRect(for: afterPosition)
        XCTAssertGreaterThan(caretRect.minY, beforeCaretRect.minY, "caret should move to the next visual line")
        XCTAssertEqual(caretRect.minY - beforeCaretRect.minY, 32, accuracy: 1.0)
    }

    func testToolbarHardBreakReservesTrailingVisualLineBeforeTyping() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
        ])

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.applyTheme(theme)
        textView.bindEditor(id: editorId, initialHTML: "<p>A</p>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 1, scalarHead: 1)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.performToolbarInsertNode("hard_break")
        textView.layoutIfNeeded()
        let heightAfterBreak = ceil(
            textView.sizeThatFits(CGSize(width: 220, height: CGFloat.greatestFiniteMagnitude)).height
        )

        textView.insertText("B")
        textView.layoutIfNeeded()
        let heightAfterTyping = ceil(
            textView.sizeThatFits(CGSize(width: 220, height: CGFloat.greatestFiniteMagnitude)).height
        )

        XCTAssertEqual(heightAfterBreak, heightAfterTyping, accuracy: 1.0)
    }

    func testCaretBeforeHorizontalRuleUsesPreviousParagraphLine() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p><hr><p>World</p>")
        textView.layoutIfNeeded()

        guard let hrRange = firstHorizontalRuleRange(in: textView) else {
            XCTFail("expected a horizontal rule attachment in the rendered text")
            return
        }
        guard let previousCharacterIndex = previousVisibleCharacterIndex(before: hrRange.location, in: textView) else {
            XCTFail("expected visible content before the horizontal rule")
            return
        }

        setCollapsedSelection(in: textView, utf16Offset: hrRange.location)
        guard let position = textView.selectedTextRange?.start else {
            XCTFail("expected caret position before the horizontal rule")
            return
        }

        let caretRect = textView.caretRect(for: position)
        let expected = expectedCaretRectForCharacterEdge(
            in: textView,
            characterIndex: previousCharacterIndex,
            edge: .trailing,
            font: UIFont.systemFont(ofSize: 16)
        )

        XCTAssertEqual(caretRect.minX, expected.minX, accuracy: 1.0)
        XCTAssertEqual(caretRect.minY, expected.minY, accuracy: 1.0)
        XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0)
    }

    func testCaretAfterHorizontalRuleUsesFollowingParagraphLine() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p><hr><p>World</p>")
        textView.layoutIfNeeded()

        guard let hrRange = firstHorizontalRuleRange(in: textView) else {
            XCTFail("expected a horizontal rule attachment in the rendered text")
            return
        }
        guard let nextCharacterIndex = nextVisibleCharacterIndex(after: hrRange.location, in: textView) else {
            XCTFail("expected visible content after the horizontal rule")
            return
        }

        setCollapsedSelection(in: textView, utf16Offset: hrRange.location + hrRange.length)
        guard let position = textView.selectedTextRange?.start else {
            XCTFail("expected caret position after the horizontal rule")
            return
        }

        let caretRect = textView.caretRect(for: position)
        let expected = expectedCaretRectForCharacterEdge(
            in: textView,
            characterIndex: nextCharacterIndex,
            edge: .leading,
            font: UIFont.systemFont(ofSize: 16)
        )

        XCTAssertEqual(caretRect.minX, expected.minX, accuracy: 1.0)
        XCTAssertEqual(caretRect.minY, expected.minY, accuracy: 1.0)
        XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0)
    }

    func testToolbarHorizontalRulePlacesCaretInTrailingParagraphLine() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 3, scalarHead: 3)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.performToolbarInsertNode("horizontal_rule")
        textView.layoutIfNeeded()

        guard let hrRange = firstHorizontalRuleRange(in: textView) else {
            XCTFail("expected a horizontal rule attachment after toolbar insertion")
            return
        }
        guard let position = textView.selectedTextRange?.start else {
            XCTFail("expected a caret after inserting a horizontal rule")
            return
        }

        let selectionOffset = textView.offset(from: textView.beginningOfDocument, to: position)
        let caretRect = textView.caretRect(for: position)
        let hrRect = renderedRect(in: textView, utf16Range: hrRange)

        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>Hello</p><hr><p></p>",
            "toolbar hr insert should create a trailing empty paragraph"
        )
        XCTAssertEqual(
            selectionOffset,
            textView.text.count,
            "toolbar hr insert should place the caret at the end of the rendered trailing paragraph"
        )
        XCTAssertGreaterThan(
            caretRect.midY,
            hrRect.midY,
            "caret after inserting a horizontal rule should render below the rule line"
        )
    }

}
