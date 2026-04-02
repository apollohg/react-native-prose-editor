import XCTest
import CoreText

// MARK: - RenderBridge Tests

final class RenderBridgeTests: XCTestCase {

    // MARK: - Test Fixtures

    private let baseFont = UIFont.systemFont(ofSize: 16)
    private let textColor = UIColor.black

    // MARK: - Plain Text Rendering

    /// A single paragraph with unstyled text should produce the text with base font.
    func testRender_plainParagraph() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Hello, world!", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "Hello, world!",
            "Plain paragraph should render as the text content"
        )

        // Verify the font is the base font.
        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertNotNil(font, "Should have a font attribute")
        XCTAssertEqual(
            font?.pointSize, baseFont.pointSize,
            "Font size should match base font"
        )
    }

    // MARK: - Bold Text Rendering

    /// Bold mark should produce a font with the bold trait.
    func testRender_boldText() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "bold text", "marks": ["bold"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "bold text")

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertNotNil(font, "Should have a font attribute")
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "Font should have bold trait. Got font: \(String(describing: font))"
        )
    }

    // MARK: - Italic Text Rendering

    func testRender_italicText() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "italic text", "marks": ["italic"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "italic text")

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertNotNil(font, "Should have a font attribute")
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitItalic) ?? false,
            "Font should have italic trait. Got font: \(String(describing: font))"
        )
    }

    // MARK: - Bold + Italic Combined

    func testRender_boldItalic() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "bold italic", "marks": ["bold", "italic"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertNotNil(font, "Should have a font attribute")

        let traits = font?.fontDescriptor.symbolicTraits ?? []
        XCTAssertTrue(
            traits.contains(.traitBold),
            "Font should have bold trait. Traits: \(traits)"
        )
        XCTAssertTrue(
            traits.contains(.traitItalic),
            "Font should have italic trait. Traits: \(traits)"
        )
    }

    // MARK: - Underline

    func testRender_underline() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "underlined", "marks": ["underline"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let underline = attrs[.underlineStyle] as? Int
        XCTAssertNotNil(underline, "Should have underline style attribute")
        XCTAssertEqual(
            underline, NSUnderlineStyle.single.rawValue,
            "Underline should be single. Got: \(String(describing: underline))"
        )
    }

    // MARK: - Strikethrough

    func testRender_strikethrough() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "struck", "marks": ["strike"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let strikethrough = attrs[.strikethroughStyle] as? Int
        XCTAssertNotNil(strikethrough, "Should have strikethrough style attribute")
        XCTAssertEqual(
            strikethrough, NSUnderlineStyle.single.rawValue,
            "Strikethrough should be single. Got: \(String(describing: strikethrough))"
        )
    }

    func testRender_linkMarkObjectAppliesVisualLinkStylingWithoutInteractiveAttribute() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "OpenAI", "marks": [{"type": "link", "href": "https://openai.com"}]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        XCTAssertEqual(
            attrs[.underlineStyle] as? Int,
            NSUnderlineStyle.single.rawValue
        )
        XCTAssertEqual(attrs[.foregroundColor] as? UIColor, UIColor.systemBlue)
        XCTAssertNil(attrs[.link])
    }

    // MARK: - Code Mark (Monospace)

    func testRender_codeInline() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "code", "marks": ["code"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertNotNil(font, "Should have a font attribute")
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitMonoSpace) ?? false,
            "Code mark should produce monospace font. Got font: \(String(describing: font))"
        )
    }

    // MARK: - Hard Break (Void Inline)

    /// A hardBreak void inline should render as a newline character.
    func testRender_hardBreak() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Line 1", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 7},
            {"type": "textRun", "text": "Line 2", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "Line 1\nLine 2",
            "Hard break should render as newline. Got: '\(result.string)'"
        )

        // Verify the newline character has the void attribute.
        let newlineIndex = 6  // "Line 1" = 6 chars, newline at index 6
        let attrs = result.attributes(at: newlineIndex, effectiveRange: nil)
        let voidType = attrs[RenderBridgeAttributes.voidNodeType] as? String
        XCTAssertEqual(
            voidType, "hardBreak",
            "Newline should have voidNodeType='hardBreak' attribute. Got: \(String(describing: voidType))"
        )
        let docPos = attrs[RenderBridgeAttributes.docPos] as? UInt32
        XCTAssertEqual(
            docPos, 7,
            "Newline should have docPos=7. Got: \(String(describing: docPos))"
        )
    }

    func testRender_hardBreakDoesNotKeepParagraphSpacingBetweenVisualLines() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Line 1", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 7},
            {"type": "textRun", "text": "Line 2", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "spacingAfter": 14,
            ],
        ])
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let leadingStyle = result.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle
        let newlineStyle = result.attribute(.paragraphStyle, at: 6, effectiveRange: nil) as? NSParagraphStyle

        XCTAssertEqual(leadingStyle?.paragraphSpacing ?? -1, 0, accuracy: 0.1)
        XCTAssertEqual(newlineStyle?.paragraphSpacing ?? -1, 0, accuracy: 0.1)
    }

    func testRender_trailingHardBreakAppendsSyntheticPlaceholder() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 2},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "A\n\u{200B}")
        let placeholderIndex = (result.string as NSString).length - 1
        XCTAssertEqual(
            result.attribute(RenderBridgeAttributes.syntheticPlaceholder, at: placeholderIndex, effectiveRange: nil) as? Bool,
            true
        )
    }

    func testRender_trailingHardBreakPlaceholderKeepsBlockquoteBorderAttributes() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 2},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "A\n\u{200B}")

        let placeholderIndex = (result.string as NSString).length - 1
        XCTAssertEqual(
            result.attribute(RenderBridgeAttributes.syntheticPlaceholder, at: placeholderIndex, effectiveRange: nil) as? Bool,
            true
        )
        XCTAssertNotNil(
            result.attribute(RenderBridgeAttributes.blockquoteBorderColor, at: placeholderIndex, effectiveRange: nil),
            "trailing hard-break placeholder inside a blockquote should keep blockquote styling"
        )
    }

    // MARK: - Horizontal Rule (Void Block)

    /// A horizontalRule should render as U+FFFC with an NSTextAttachment.
    func testRender_horizontalRule() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Above", "marks": []},
            {"type": "blockEnd"},
            {"type": "voidBlock", "nodeType": "horizontalRule", "docPos": 7},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Below", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        // The expected structure is: "Above" + "\n" + U+FFFC + "\n" + "Below"
        // The newlines are inter-block separators.
        let string = result.string
        XCTAssertTrue(
            string.contains("\u{FFFC}"),
            "Horizontal rule should contain object replacement character. Got: '\(string)'"
        )

        // Find the FFFC character and check its attributes.
        if let fffcRange = string.range(of: "\u{FFFC}") {
            let nsRange = NSRange(fffcRange, in: string)
            let attrs = result.attributes(at: nsRange.location, effectiveRange: nil)

            let voidType = attrs[RenderBridgeAttributes.voidNodeType] as? String
            XCTAssertEqual(
                voidType, "horizontalRule",
                "FFFC should have voidNodeType='horizontalRule'. Got: \(String(describing: voidType))"
            )

            let attachment = attrs[.attachment] as? NSTextAttachment
            XCTAssertNotNil(
                attachment,
                "FFFC should have an NSTextAttachment"
            )
            XCTAssertTrue(
                attachment is HorizontalRuleAttachment,
                "Attachment should be HorizontalRuleAttachment. Got: \(String(describing: type(of: attachment)))"
            )
        } else {
            XCTFail("Could not find FFFC character in rendered string")
        }
    }

    // MARK: - Multiple Paragraphs

    /// Two consecutive paragraphs should be separated by a newline.
    func testRender_multipleParagraphs() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "First", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Second", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "First\nSecond",
            "Two paragraphs should be separated by a newline"
        )
    }

    // MARK: - Mixed Marks in Same Paragraph

    /// A paragraph with mixed styled runs should produce the correct combined string
    /// with different attributes at different ranges.
    func testRender_mixedMarksInParagraph() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "normal ", "marks": []},
            {"type": "textRun", "text": "bold", "marks": ["bold"]},
            {"type": "textRun", "text": " end", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "normal bold end")

        // Check "normal " (offset 0) has base font, not bold.
        let normalAttrs = result.attributes(at: 0, effectiveRange: nil)
        let normalFont = normalAttrs[.font] as? UIFont
        XCTAssertFalse(
            normalFont?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? true,
            "'normal' should not be bold"
        )

        // Check "bold" (offset 7) has bold font.
        let boldAttrs = result.attributes(at: 7, effectiveRange: nil)
        let boldFont = boldAttrs[.font] as? UIFont
        XCTAssertTrue(
            boldFont?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "'bold' should have bold trait"
        )

        // Check " end" (offset 11) has base font, not bold.
        let endAttrs = result.attributes(at: 11, effectiveRange: nil)
        let endFont = endAttrs[.font] as? UIFont
        XCTAssertFalse(
            endFont?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? true,
            "'end' should not be bold"
        )
    }

    // MARK: - Ordered List

    /// Ordered list items should reserve gutter space without injecting marker text.
    func testRender_orderedListItem() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "First item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 2, "total": 2, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Second item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "First item\nSecond item")

        let firstAttrs = result.attributes(at: 0, effectiveRange: nil)
        let firstStyle = firstAttrs[.paragraphStyle] as? NSParagraphStyle
        XCTAssertNotNil(firstAttrs[RenderBridgeAttributes.listContext])
        XCTAssertEqual(firstStyle?.firstLineHeadIndent, 68.0)
        XCTAssertEqual(firstStyle?.headIndent, 68.0)
    }

    // MARK: - Unordered List

    func testRender_unorderedListItem() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Bullet item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "Bullet item")
        XCTAssertNotNil(result.attribute(RenderBridgeAttributes.listContext, at: 0, effectiveRange: nil))
    }

    func testRender_unorderedListMarkerUsesLargerFontThanItemText() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Bullet item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let textFont = result.attribute(.font, at: 0, effectiveRange: nil) as? UIFont
        XCTAssertEqual(textFont?.pointSize, baseFont.pointSize)
        XCTAssertNotNil(result.attribute(RenderBridgeAttributes.listContext, at: 0, effectiveRange: nil))
    }

    func testRender_emptyUnorderedListItemDoesNotInsertParagraphNewlineAfterMarker() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "\\u200B", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "\u{200B}",
            "An empty list item should render only its placeholder text. Got: '\(result.string)'"
        )
        XCTAssertNotNil(result.attribute(RenderBridgeAttributes.listContext, at: 0, effectiveRange: nil))
    }

    func testRender_emptyParagraphAfterListUsesItsOwnParagraphStyle() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "\\u200B", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "A\n\u{200B}")

        let placeholderIndex = (result.string as NSString).length - 1
        let placeholderStyle = result.attribute(
            .paragraphStyle,
            at: placeholderIndex,
            effectiveRange: nil
        ) as? NSParagraphStyle

        XCTAssertNotNil(placeholderStyle, "Empty paragraph placeholder should carry paragraph style")
        XCTAssertEqual(placeholderStyle?.firstLineHeadIndent, 0)
        XCTAssertEqual(placeholderStyle?.headIndent, 0)
    }

    func testRender_secondParagraphInListItemDoesNotGetListMarkerContext() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "\\u200B", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertNotNil(
            result.attribute(RenderBridgeAttributes.listMarkerContext, at: 0, effectiveRange: nil),
            "The first paragraph in a list item should keep its marker context"
        )
        XCTAssertNil(
            result.attribute(RenderBridgeAttributes.listMarkerContext, at: 2, effectiveRange: nil),
            "The second paragraph in a list item should not render a separate list marker"
        )
    }

    // MARK: - Invalid JSON

    func testRender_invalidJSON() {
        let result = RenderBridge.renderElements(
            fromJSON: "not valid json",
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "",
            "Invalid JSON should produce empty attributed string"
        )
    }

    func testRender_emptyArray() {
        let result = RenderBridge.renderElements(
            fromJSON: "[]",
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "",
            "Empty array should produce empty attributed string"
        )
    }

    // MARK: - Mark Attributes Isolated Tests

    /// Test attributesForMarks directly to verify all mark combinations.
    func testAttributesForMarks_noMarks() {
        let attrs = RenderBridge.attributesForMarks([], baseFont: baseFont, textColor: textColor)
        let font = attrs[.font] as? UIFont
        XCTAssertEqual(font, baseFont, "No marks should use base font")
        XCTAssertNil(attrs[.underlineStyle], "No marks should have no underline")
        XCTAssertNil(attrs[.strikethroughStyle], "No marks should have no strikethrough")
    }

    func testAttributesForMarks_strongAlias() {
        // "strong" is an alias for "bold"
        let attrs = RenderBridge.attributesForMarks(
            ["strong"],
            baseFont: baseFont,
            textColor: textColor
        )
        let font = attrs[.font] as? UIFont
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "'strong' should produce bold font"
        )
    }

    func testAttributesForMarks_emAlias() {
        // "em" is an alias for "italic"
        let attrs = RenderBridge.attributesForMarks(
            ["em"],
            baseFont: baseFont,
            textColor: textColor
        )
        let font = attrs[.font] as? UIFont
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitItalic) ?? false,
            "'em' should produce italic font"
        )
    }

    func testAttributesForMarks_strikethroughAlias() {
        // "strikethrough" is an alias for "strike"
        let attrs = RenderBridge.attributesForMarks(
            ["strikethrough"],
            baseFont: baseFont,
            textColor: textColor
        )
        let strikethrough = attrs[.strikethroughStyle] as? Int
        XCTAssertEqual(
            strikethrough, NSUnderlineStyle.single.rawValue,
            "'strikethrough' should produce strikethrough style"
        )
    }

    func testAttributesForMarks_allCombined() {
        let attrs = RenderBridge.attributesForMarks(
            ["bold", "italic", "underline", "strike"],
            baseFont: baseFont,
            textColor: textColor
        )
        let font = attrs[.font] as? UIFont
        let traits = font?.fontDescriptor.symbolicTraits ?? []
        XCTAssertTrue(traits.contains(.traitBold), "Should have bold")
        XCTAssertTrue(traits.contains(.traitItalic), "Should have italic")
        XCTAssertEqual(
            attrs[.underlineStyle] as? Int,
            NSUnderlineStyle.single.rawValue,
            "Should have underline"
        )
        XCTAssertEqual(
            attrs[.strikethroughStyle] as? Int,
            NSUnderlineStyle.single.rawValue,
            "Should have strikethrough"
        )
    }

    func testAttributesForMarks_unknownMarkIgnored() {
        let attrs = RenderBridge.attributesForMarks(
            ["unknownMark"],
            baseFont: baseFont,
            textColor: textColor
        )
        let font = attrs[.font] as? UIFont
        XCTAssertEqual(
            font, baseFont,
            "Unknown marks should be ignored, producing base font"
        )
    }

    // MARK: - Paragraph Style Tests

    func testParagraphStyle_depth0() {
        let ctx = BlockContext(nodeType: "paragraph", depth: 0, listContext: nil)
        let style = RenderBridge.paragraphStyleForBlock(ctx, blockStack: [ctx])
        XCTAssertEqual(
            style.firstLineHeadIndent, 0,
            "Depth 0 paragraph should have 0 indentation"
        )
        XCTAssertEqual(
            style.headIndent, 0,
            "Depth 0 paragraph should have 0 head indent"
        )
    }

    func testParagraphStyle_depth2() {
        let ctx = BlockContext(nodeType: "paragraph", depth: 2, listContext: nil)
        let style = RenderBridge.paragraphStyleForBlock(ctx, blockStack: [ctx])
        let expectedIndent: CGFloat = 2 * 24.0  // 2 * indentPerDepth
        XCTAssertEqual(
            style.firstLineHeadIndent, expectedIndent,
            "Depth 2 paragraph should have \(expectedIndent) first line indent"
        )
    }

    func testParagraphStyle_listItem() {
        let listCtx: [String: Any] = [
            "ordered": true,
            "index": 1,
            "total": 3,
            "start": 1,
            "isFirst": true,
            "isLast": false,
        ]
        let ctx = BlockContext(nodeType: "listItem", depth: 1, listContext: listCtx)
        let style = RenderBridge.paragraphStyleForBlock(ctx, blockStack: [ctx])

        let baseIndent: CGFloat = 1 * 24.0  // depth * indentPerDepth
        XCTAssertEqual(
            style.firstLineHeadIndent, baseIndent + 20.0,
            "List item first line indent should reserve marker width"
        )
        XCTAssertEqual(
            style.headIndent, baseIndent + 20.0,  // + listMarkerWidth
            "List item head indent should include marker width"
        )
    }

    func testParagraphStyle_unorderedMarkerScaleDoesNotWidenTextGutter() {
        let baseContext = BlockContext(
            nodeType: "listItem",
            depth: 1,
            listContext: [
                "ordered": false,
                "index": 1,
                "total": 1,
                "start": 1,
                "isFirst": true,
                "isLast": true,
            ]
        )
        let baseTheme = EditorTheme(dictionary: [
            "list": [
                "indent": 24,
                "markerScale": 1,
            ],
        ])
        let scaledTheme = EditorTheme(dictionary: [
            "list": [
                "indent": 24,
                "markerScale": 2,
            ],
        ])

        let largeBaseFont = UIFont.systemFont(ofSize: 40)
        let baseStyle = RenderBridge.paragraphStyleForBlock(
            baseContext,
            blockStack: [baseContext],
            theme: baseTheme,
            baseFont: largeBaseFont
        )
        let scaledStyle = RenderBridge.paragraphStyleForBlock(
            baseContext,
            blockStack: [baseContext],
            theme: scaledTheme,
            baseFont: largeBaseFont
        )

        XCTAssertEqual(baseStyle.headIndent, scaledStyle.headIndent, accuracy: 0.1)
        XCTAssertEqual(baseStyle.firstLineHeadIndent, scaledStyle.firstLineHeadIndent, accuracy: 0.1)
    }

    func testParagraphStyle_blockquoteUsesQuoteIndent() {
        let quote = BlockContext(nodeType: "blockquote", depth: 0, listContext: nil)
        let paragraph = BlockContext(nodeType: "paragraph", depth: 1, listContext: nil)
        let theme = EditorTheme(dictionary: [
            "blockquote": [
                "indent": 20,
                "borderColor": "#aa5500",
                "borderWidth": 4,
                "markerGap": 10,
            ],
        ])

        let style = RenderBridge.paragraphStyleForBlock(
            paragraph,
            blockStack: [quote, paragraph],
            theme: theme,
            baseFont: baseFont
        )

        XCTAssertEqual(style.firstLineHeadIndent, 20, accuracy: 0.1)
        XCTAssertEqual(style.headIndent, 20, accuracy: 0.1)
    }

    func testParagraphStyle_nestedListItemInsideBlockquoteAddsListIndent() {
        let quote = BlockContext(nodeType: "blockquote", depth: 0, listContext: nil)
        let parentListItem = BlockContext(
            nodeType: "listItem",
            depth: 1,
            listContext: ["ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false]
        )
        let parentParagraph = BlockContext(nodeType: "paragraph", depth: 2, listContext: nil)
        let nestedListItem = BlockContext(
            nodeType: "listItem",
            depth: 2,
            listContext: ["ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true]
        )
        let nestedParagraph = BlockContext(nodeType: "paragraph", depth: 3, listContext: nil)

        let parentStyle = RenderBridge.paragraphStyleForBlock(
            parentParagraph,
            blockStack: [quote, parentListItem, parentParagraph],
            theme: nil,
            baseFont: baseFont
        )
        let nestedStyle = RenderBridge.paragraphStyleForBlock(
            nestedParagraph,
            blockStack: [quote, parentListItem, nestedListItem, nestedParagraph],
            theme: nil,
            baseFont: baseFont
        )

        XCTAssertGreaterThan(
            nestedStyle.headIndent,
            parentStyle.headIndent,
            "nested list item inside a blockquote should indent more than its parent item"
        )
        XCTAssertGreaterThan(
            nestedStyle.firstLineHeadIndent,
            parentStyle.firstLineHeadIndent,
            "nested list marker should also move inward inside a blockquote"
        )
    }

    func testParagraphStyle_firstLevelListInsideBlockquoteAddsListIndentInsideQuote() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Quoted item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )
        let style = result.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle
        let quote = BlockContext(nodeType: "blockquote", depth: 0, listContext: nil)
        let quotedParagraph = BlockContext(nodeType: "paragraph", depth: 1, listContext: nil)
        let quotedListParagraph = BlockContext(
            nodeType: "paragraph",
            depth: 2,
            listContext: ["ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true]
        )
        let plainQuotedStyle = RenderBridge.paragraphStyleForBlock(
            quotedParagraph,
            blockStack: [quote, quotedParagraph],
            theme: nil,
            baseFont: baseFont
        )
        let expectedStyle = RenderBridge.paragraphStyleForBlock(
            quotedListParagraph,
            blockStack: [quote, quotedListParagraph],
            theme: nil,
            baseFont: baseFont
        )

        XCTAssertEqual(
            style?.headIndent ?? -1,
            expectedStyle.headIndent,
            accuracy: 0.1,
            "first-level list paragraphs inside a blockquote should keep their extra list indent"
        )
        XCTAssertEqual(
            style?.firstLineHeadIndent ?? -1,
            expectedStyle.firstLineHeadIndent,
            accuracy: 0.1,
            "first-level quoted list markers should keep their extra list indent"
        )
        XCTAssertGreaterThan(
            style?.headIndent ?? -1,
            plainQuotedStyle.headIndent,
            "quoted list text should indent further than plain quoted text"
        )
        XCTAssertGreaterThan(
            style?.firstLineHeadIndent ?? -1,
            plainQuotedStyle.firstLineHeadIndent,
            "quoted list marker gutter should indent further than plain quoted text"
        )
    }

    func testRender_blockquoteAppliesBorderAttributesAndTextTheme() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Quoted", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: EditorTheme(dictionary: [
                "blockquote": [
                    "indent": 20,
                    "borderColor": "#aa5500",
                    "borderWidth": 4,
                    "markerGap": 10,
                    "text": [
                        "color": "#334455",
                    ],
                ],
            ])
        )
        let expectedTextColor = UIColor(
            red: 51.0 / 255.0,
            green: 68.0 / 255.0,
            blue: 85.0 / 255.0,
            alpha: 1
        )
        let expectedBorderColor = UIColor(
            red: 170.0 / 255.0,
            green: 85.0 / 255.0,
            blue: 0.0,
            alpha: 1
        )
        var foundStyledRun = false
        result.enumerateAttributes(
            in: NSRange(location: 0, length: result.length),
            options: []
        ) { attrs, _, stop in
            guard attrs[RenderBridgeAttributes.blockquoteBorderColor] != nil else { return }
            XCTAssertEqual(attrs[.foregroundColor] as? UIColor, expectedTextColor)
            XCTAssertEqual(attrs[RenderBridgeAttributes.blockquoteBorderColor] as? UIColor, expectedBorderColor)
            XCTAssertEqual(
                (attrs[RenderBridgeAttributes.blockquoteBorderWidth] as? NSNumber)?.doubleValue ?? 0,
                4,
                accuracy: 0.1
            )
            XCTAssertEqual(
                (attrs[RenderBridgeAttributes.blockquoteMarkerGap] as? NSNumber)?.doubleValue ?? 0,
                10,
                accuracy: 0.1
            )
            foundStyledRun = true
            stop.pointee = true
        }

        XCTAssertTrue(foundStyledRun, "Expected a rendered run carrying blockquote border attributes")
    }

    func testRender_blockquoteDoesNotInsertExtraLeadingParagraphBreak() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Hello", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "World", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "Hello\nWorld")
    }

    // MARK: - List Marker Generation

    func testListMarker_ordered() {
        let ctx: [String: Any] = ["ordered": true, "index": 3]
        let marker = RenderBridge.listMarkerString(listContext: ctx)
        XCTAssertEqual(marker, "3. ", "Ordered list item 3 should produce '3. '")
    }

    func testListMarker_unordered() {
        let ctx: [String: Any] = ["ordered": false, "index": 1]
        let marker = RenderBridge.listMarkerString(listContext: ctx)
        XCTAssertEqual(marker, "\u{2022} ", "Unordered list should produce bullet + space")
    }

    // MARK: - Opaque Atoms

    func testRender_opaqueInlineAtom() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "before ", "marks": []},
            {"type": "opaqueInlineAtom", "label": "widget", "docPos": 8},
            {"type": "textRun", "text": " after", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertTrue(
            result.string.contains("[widget]"),
            "Opaque inline atom should render as '[widget]'. Got: '\(result.string)'"
        )
    }

    func testRender_mentionInlineAtomUsesVisibleLabelAndTheme() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Hello ", "marks": []},
            {"type": "opaqueInlineAtom", "nodeType": "mention", "label": "@Alice", "docPos": 7},
            {"type": "textRun", "text": "!", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "mentions": [
                "textColor": "#112233",
                "backgroundColor": "#ddeeff",
                "fontWeight": "bold",
            ],
        ])
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        XCTAssertTrue(
            result.string.contains("@Alice"),
            "Mention inline atom should render its visible label. Got: '\(result.string)'"
        )
        XCTAssertFalse(
            result.string.contains("[@Alice]"),
            "Mention inline atom should not render using generic opaque brackets. Got: '\(result.string)'"
        )

        let mentionRange = (result.string as NSString).range(of: "@Alice")
        XCTAssertNotEqual(mentionRange.location, NSNotFound)

        let attrs = result.attributes(at: mentionRange.location, effectiveRange: nil)
        XCTAssertEqual(
            attrs[.foregroundColor] as? UIColor,
            UIColor(
                red: 0x11 as CGFloat / 255.0,
                green: 0x22 as CGFloat / 255.0,
                blue: 0x33 as CGFloat / 255.0,
                alpha: 1.0
            )
        )
        XCTAssertEqual(
            attrs[.backgroundColor] as? UIColor,
            UIColor(
                red: 0xdd as CGFloat / 255.0,
                green: 0xee as CGFloat / 255.0,
                blue: 0xff as CGFloat / 255.0,
                alpha: 1.0
            )
        )
        let font = attrs[.font] as? UIFont
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "Mention theme should be able to request a bold font"
        )
    }

    func testRender_opaqueBlockAtom() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Above", "marks": []},
            {"type": "blockEnd"},
            {"type": "opaqueBlockAtom", "label": "codeBlock", "docPos": 7}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertTrue(
            result.string.contains("[codeBlock]"),
            "Opaque block atom should render as '[codeBlock]'. Got: '\(result.string)'"
        )
    }

    // MARK: - Theme Rendering

    func testRender_themeOverridesParagraphTypography() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Styled", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "text": [
                "fontFamily": "Courier",
                "fontSize": 18,
                "color": "#112233",
            ],
            "paragraph": [
                "lineHeight": 28,
                "spacingAfter": 14,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        let color = attrs[.foregroundColor] as? UIColor
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(font?.pointSize ?? 0, 18, accuracy: 0.1)
        XCTAssertEqual(color, EditorTheme.color(from: "#112233"))
        XCTAssertEqual(paragraphStyle?.minimumLineHeight ?? 0, 28, accuracy: 0.1)
        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? 0, 14, accuracy: 0.1)
    }

    func testRender_listItemUsesListItemSpacingWhenParagraphSpacingUnset() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "First item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 2, "total": 2, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Second item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "list": [
                "itemSpacing": 14,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? 0, 14, accuracy: 0.1)
    }

    func testRender_listItemSpacingOverridesParagraphSpacingForSiblingListItems() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "First item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 2, "total": 2, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Second item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "spacingAfter": 14,
            ],
            "list": [
                "itemSpacing": 6,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let nsString = result.string as NSString
        let firstRange = nsString.range(of: "First item")
        XCTAssertNotEqual(firstRange.location, NSNotFound)

        let attrs = result.attributes(at: firstRange.location, effectiveRange: nil)
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? -1, 6, accuracy: 0.1)
    }

    func testRender_nestedFirstListItemDoesNotKeepParentParagraphSpacingWhenItemSpacingIsZero() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Parent item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Nested item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "spacingAfter": 14,
            ],
            "list": [
                "itemSpacing": 0,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let nsString = result.string as NSString
        let parentRange = nsString.range(of: "Parent item")
        XCTAssertNotEqual(parentRange.location, NSNotFound)

        let attrs = result.attributes(at: parentRange.location, effectiveRange: nil)
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? -1, 0, accuracy: 0.1)
    }

    func testRender_nestedSiblingListItemsUseListItemSpacingInsteadOfParagraphSpacing() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Parent item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Child one", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 2, "total": 2, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Child two", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "spacingAfter": 14,
            ],
            "list": [
                "itemSpacing": 6,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let nsString = result.string as NSString
        let childRange = nsString.range(of: "Child one")
        XCTAssertNotEqual(childRange.location, NSNotFound)

        let attrs = result.attributes(at: childRange.location, effectiveRange: nil)
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? -1, 6, accuracy: 0.1)
    }

    func testRender_themeOverridesHorizontalRuleMetrics() {
        let json = """
        [
            {"type": "voidBlock", "nodeType": "horizontalRule", "docPos": 0}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "horizontalRule": [
                "color": "#445566",
                "thickness": 3,
                "verticalMargin": 12,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attachment = result.attribute(.attachment, at: 0, effectiveRange: nil)
            as? HorizontalRuleAttachment
        XCTAssertEqual(attachment?.lineColor, EditorTheme.color(from: "#445566"))
        XCTAssertEqual(attachment?.lineHeight ?? 0, 3, accuracy: 0.1)
        XCTAssertEqual(attachment?.verticalPadding ?? 0, 12, accuracy: 0.1)
    }

    func testListMarkerDrawingRectUsesParagraphLineBox() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 10, width: 160, height: 28)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let baselineY: CGFloat = 28.140625
        let rect = EditorLayoutManager.markerDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            origin: CGPoint(x: 0, y: 0)
        )
        let typographicHeight = markerFont.ascender - markerFont.descender
        let leading = max(markerFont.lineHeight - typographicHeight, 0)
        let expectedY = baselineY - markerFont.ascender - (leading / 2.0)

        XCTAssertEqual(rect.origin.x, 4, accuracy: 0.1)
        XCTAssertEqual(rect.origin.y, expectedY, accuracy: 0.1)
        XCTAssertEqual(rect.height, markerFont.lineHeight, accuracy: 0.1)
    }

    func testListMarkerDrawingRectUsesFullLineFragmentWhenGlyphsUseShorterRect() {
        let markerFont = baseFont.withSize(18)
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 17)
        let baselineY: CGFloat = 30.140625
        let rect = EditorLayoutManager.markerDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            origin: CGPoint(x: 0, y: 0)
        )
        let typographicHeight = markerFont.ascender - markerFont.descender
        let leading = max(markerFont.lineHeight - typographicHeight, 0)
        let expectedY = baselineY - markerFont.ascender - (leading / 2.0)

        XCTAssertEqual(rect.origin.x, 4, accuracy: 0.1)
        XCTAssertEqual(rect.origin.y, expectedY, accuracy: 0.1)
        XCTAssertEqual(rect.height, markerFont.lineHeight, accuracy: 0.1)
    }

    func testListMarkerDrawingRectFallsBackToLineFragmentWhenUsedRectIsEmpty() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 10, width: 160, height: 28)
        let rect = EditorLayoutManager.markerDrawingRect(
            usedRect: CGRect(x: 24, y: 10, width: 160, height: 0),
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: 28.140625,
            markerFont: markerFont,
            origin: CGPoint(x: 0, y: 0)
        )
        let typographicHeight = markerFont.ascender - markerFont.descender
        let leading = max(markerFont.lineHeight - typographicHeight, 0)
        let expectedY = 28.140625 - markerFont.ascender - (leading / 2.0)

        XCTAssertEqual(rect.origin.x, 4, accuracy: 0.1)
        XCTAssertEqual(rect.origin.y, expectedY, accuracy: 0.1)
    }

    func testOrderedMarkerDrawingOriginAlignsToBaselineWithoutParagraphLineHeight() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let baselineY: CGFloat = 30.140625
        let markerText = "12. "

        let point = EditorLayoutManager.orderedMarkerDrawingOrigin(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            markerText: markerText,
            origin: .zero
        )
        let markerWidth = ceil(("12." as NSString).size(withAttributes: [
            .font: markerFont,
        ]).width)

        XCTAssertEqual(point.x, usedRect.minX - 4.0 - markerWidth, accuracy: 0.1)
        XCTAssertEqual(point.y, baselineY - markerFont.ascender, accuracy: 0.1)
    }

    func testOrderedMarkerDrawingOriginIgnoresTrailingSpaceForHorizontalAlignment() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let baselineY: CGFloat = 30.140625
        let markerText = "12. "

        let point = EditorLayoutManager.orderedMarkerDrawingOrigin(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            markerText: markerText,
            origin: .zero
        )
        let visibleWidth = ceil(("12." as NSString).size(withAttributes: [
            .font: markerFont,
        ]).width)
        let fullWidth = ceil((markerText as NSString).size(withAttributes: [
            .font: markerFont,
        ]).width)

        XCTAssertEqual(point.x, usedRect.minX - 4.0 - visibleWidth, accuracy: 0.1)
        XCTAssertNotEqual(point.x, usedRect.minX - 4.0 - fullWidth, accuracy: 0.1)
    }

    func testListMarkerParagraphStylePreservesThemedLineHeight() {
        let sourceStyle = NSMutableParagraphStyle()
        sourceStyle.minimumLineHeight = 28
        sourceStyle.maximumLineHeight = 28

        let markerStyle = EditorLayoutManager.markerParagraphStyle(from: [
            .paragraphStyle: sourceStyle,
        ])

        XCTAssertEqual(markerStyle.minimumLineHeight, 28, accuracy: 0.1)
        XCTAssertEqual(markerStyle.maximumLineHeight, 28, accuracy: 0.1)
        XCTAssertEqual(markerStyle.alignment, .right)
        XCTAssertEqual(markerStyle.lineBreakMode, .byClipping)
        XCTAssertEqual(markerStyle.firstLineHeadIndent, 0, accuracy: 0.1)
        XCTAssertEqual(markerStyle.headIndent, 0, accuracy: 0.1)
        XCTAssertEqual(markerStyle.tailIndent, 0, accuracy: 0.1)
    }

    func testUnorderedBulletDrawingRectCentersBulletOnTextMidline() {
        let rect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: CGRect(x: 24, y: 14, width: 160, height: 19),
            lineFragmentRect: CGRect(x: 24, y: 8, width: 160, height: 32),
            markerWidth: 20,
            baselineY: 28.140625,
            baseFont: baseFont,
            markerScale: 2,
            origin: .zero
        )
        let targetMidline = 28.140625 - ((baseFont.xHeight > 0 ? baseFont.xHeight : baseFont.capHeight) / 2.0)

        XCTAssertEqual(rect.midY, targetMidline, accuracy: 0.1)
        XCTAssertGreaterThan(rect.width, 0)
        XCTAssertGreaterThan(rect.height, 0)
    }

    func testUnorderedBulletDrawingRectPreservesTextSideGapAcrossMarkerScales() {
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let baselineY: CGFloat = 28.140625

        let normalRect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            baseFont: baseFont,
            markerScale: 1,
            origin: .zero
        )
        let scaledRect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            baseFont: baseFont,
            markerScale: 2,
            origin: .zero
        )

        XCTAssertEqual(usedRect.minX - normalRect.maxX, usedRect.minX - scaledRect.maxX, accuracy: 0.1)
        XCTAssertEqual(usedRect.minX - scaledRect.maxX, LayoutConstants.listMarkerTextGap, accuracy: 0.1)
    }

    func testUnorderedBulletDrawingRectReproducesTallLineHeightListItem() {
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
            "list": [
                "markerScale": 2,
            ],
        ])
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Bullet item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let textFont = attrs[.font] as? UIFont ?? baseFont
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle
        let markerScale = (attrs[RenderBridgeAttributes.listMarkerScale] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? 1
        let bulletRect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: CGRect(x: 24, y: 14, width: 160, height: 19),
            lineFragmentRect: CGRect(x: 24, y: 8, width: 160, height: 32),
            markerWidth: 20,
            baselineY: 28.140625,
            baseFont: textFont,
            markerScale: markerScale,
            origin: .zero
        )
        let expectedCenterY = 28.140625 - ((textFont.xHeight > 0 ? textFont.xHeight : textFont.capHeight) / 2.0)

        XCTAssertNotNil(attrs[RenderBridgeAttributes.listMarkerContext])
        XCTAssertEqual(paragraphStyle?.minimumLineHeight ?? 0, 32, accuracy: 0.1)
        XCTAssertEqual(paragraphStyle?.maximumLineHeight ?? 0, 32, accuracy: 0.1)
        XCTAssertEqual(bulletRect.midY, expectedCenterY, accuracy: 0.1)
        XCTAssertGreaterThan(bulletRect.width, 0)
        XCTAssertGreaterThan(bulletRect.height, 0)
        XCTAssertEqual(bulletRect.width, bulletRect.height, accuracy: 0.1)
    }

    func testOrderedListMarkerBaselineOffsetIsNeutral() {
        let orderedContext: [String: Any] = ["ordered": true]

        let offset = EditorLayoutManager.markerBaselineOffset(
            for: orderedContext,
            baseFont: baseFont,
            markerFont: baseFont
        )

        XCTAssertEqual(offset, 0, accuracy: 0.1)
    }

    // MARK: - HorizontalRuleAttachment

    func testHorizontalRuleAttachment_bounds() {
        let attachment = HorizontalRuleAttachment()
        let proposedRect = CGRect(x: 0, y: 0, width: 320, height: 20)
        let bounds = attachment.attachmentBounds(
            for: nil,
            proposedLineFragment: proposedRect,
            glyphPosition: .zero,
            characterIndex: 0
        )

        XCTAssertEqual(
            bounds.width, 320,
            "Attachment width should match proposed line fragment width"
        )
        let expectedHeight = 1.0 + (8.0 * 2)  // line + padding
        XCTAssertEqual(
            bounds.height, expectedHeight,
            "Attachment height should be line height + 2 * vertical padding"
        )
    }

    func testHorizontalRuleAttachment_rendersImage() {
        let attachment = HorizontalRuleAttachment()
        attachment.lineColor = .red
        let bounds = CGRect(x: 0, y: 0, width: 200, height: 17)
        let image = attachment.image(
            forBounds: bounds,
            textContainer: nil,
            characterIndex: 0
        )
        XCTAssertNotNil(image, "HorizontalRuleAttachment should produce a non-nil image")
    }
}
