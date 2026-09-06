import XCTest
import CoreText

extension RenderBridgeTests {
    /// Two adjacent code blocks must produce two separate background groups —
    /// the separator newline between blocks carries no codeBlockBackgroundColor.
    func testCodeBlockGrouping_adjacentBlocksAreSeparate() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "first", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "second", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let rendered = RenderBridge.renderElements(fromJSON: json, baseFont: baseFont, textColor: textColor)
        let storage = NSTextStorage(attributedString: rendered)
        let nsString = storage.string as NSString
        // "first\nsecond" — paragraphStart of "second" is 6.
        let group = EditorLayoutManager.codeBlockCharacterRange(
            containing: 6, in: storage, nsString: nsString
        )
        XCTAssertEqual(group.location, 6, "Group must not absorb the preceding block")
        // And the first block's group must stop before the separator:
        let firstGroup = EditorLayoutManager.codeBlockCharacterRange(
            containing: 0, in: storage, nsString: nsString
        )
        XCTAssertEqual(NSMaxRange(firstGroup), 6, "Group may include its own separator paragraph end at most")
        XCTAssertNotEqual(firstGroup, group)
    }

    /// theme.codeBlock.text.fontFamily must not be silently replaced by the
    /// system monospace font.
    func testRender_codeBlock_honorsThemeFontFamily() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "let x", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "codeBlock": [
                "text": [
                    "fontFamily": "Courier New",
                ],
            ],
        ])
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let font = result.attributes(at: 0, effectiveRange: nil)[.font] as? UIFont
        XCTAssertNotNil(font)
        XCTAssertEqual(
            font!.familyName,
            "Courier New",
            "Themed codeBlock.text.fontFamily should be preserved, not overwritten by the system monospace font. Got: \(font!.familyName)"
        )
    }

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

    func testRender_proseMirrorVoidNodeNames() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Above", "marks": []},
            {"type": "voidInline", "nodeType": "hard_break", "docPos": 6},
            {"type": "textRun", "text": "Below", "marks": []},
            {"type": "blockEnd"},
            {"type": "voidBlock", "nodeType": "horizontal_rule", "docPos": 13}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertTrue(result.string.contains("Above\nBelow"))
        let ruleRange = (result.string as NSString).range(of: "\u{FFFC}")
        XCTAssertNotEqual(ruleRange.location, NSNotFound)
        XCTAssertTrue(result.attribute(.attachment, at: ruleRange.location, effectiveRange: nil) is HorizontalRuleAttachment)
    }

    func testRender_registeredVoidBlockGetsAtomSpacerAttachment() throws {
        let result = RenderBridge.renderElements(
            fromJSON: #"[{"type":"voidBlock","nodeType":"counterCard","docPos":1,"attrs":{"title":"t"}}]"#,
            baseFont: baseFont,
            textColor: textColor,
            atomConfiguration: AtomRenderConfiguration(
                registeredNodeTypes: ["counterCard"],
                estimatedHeights: ["counterCard": 120],
                measuredHeights: [:]
            )
        )

        let attachment = try XCTUnwrap(
            result.attribute(.attachment, at: 0, effectiveRange: nil) as? AtomBlockAttachment
        )
        XCTAssertEqual(result.string, "\u{FFFC}")
        XCTAssertEqual(attachment.atomKey, "counterCard:0")
        XCTAssertEqual(attachment.nodeType, "counterCard")
        XCTAssertEqual(attachment.docPos, 1)
        XCTAssertEqual(attachment.reservedHeight, 120)
        let bounds = attachment.attachmentBounds(
            for: nil,
            proposedLineFragment: CGRect(x: 0, y: 0, width: 280, height: 20),
            glyphPosition: .zero,
            characterIndex: 0
        )
        XCTAssertEqual(bounds.width, 280)
        XCTAssertEqual(bounds.height, 120)
        XCTAssertNil(
            attachment.image(forBounds: bounds, textContainer: nil, characterIndex: 0)
        )
        XCTAssertEqual(
            result.attribute(RenderBridgeAttributes.voidNodeType, at: 0, effectiveRange: nil) as? String,
            "counterCard"
        )
        XCTAssertEqual(
            result.attribute(RenderBridgeAttributes.docPos, at: 0, effectiveRange: nil) as? UInt32,
            1
        )
    }

    func testRender_registeredAtomsOverrideBuiltInVoidRendering() throws {
        for nodeType in ["image", "horizontalRule", "horizontal_rule"] {
            let result = RenderBridge.renderElements(
                fromJSON: """
                [{"type":"voidBlock","nodeType":"\(nodeType)","docPos":1,"atomId":"custom-1"}]
                """,
                baseFont: baseFont,
                textColor: textColor,
                atomConfiguration: AtomRenderConfiguration(
                    registeredNodeTypes: [nodeType],
                    estimatedHeights: [nodeType: 120],
                    measuredHeights: [:]
                )
            )
            let attachment = try XCTUnwrap(
                result.attribute(.attachment, at: 0, effectiveRange: nil) as? AtomBlockAttachment,
                nodeType
            )
            XCTAssertEqual(attachment.atomKey, "custom-1")
            XCTAssertEqual(attachment.nodeType, nodeType)
            XCTAssertEqual(attachment.reservedHeight, 120)
        }
    }

    func testRender_atomKeysFollowIdentityContract() {
        let result = RenderBridge.renderElements(
            fromArray: [
                ["type": "voidBlock", "nodeType": "counterCard", "docPos": 1],
                ["type": "voidBlock", "nodeType": "counterCard", "docPos": 3],
                [
                    "type": "voidBlock",
                    "nodeType": "counterCard",
                    "docPos": 5,
                    "atomId": "client-1:9",
                ],
            ],
            baseFont: baseFont,
            textColor: textColor,
            atomConfiguration: AtomRenderConfiguration(
                registeredNodeTypes: ["counterCard"],
                estimatedHeights: [:],
                measuredHeights: [:]
            )
        )
        var keys: [String] = []
        result.enumerateAttribute(
            .attachment,
            in: NSRange(location: 0, length: result.length)
        ) { value, _, _ in
            if let attachment = value as? AtomBlockAttachment {
                keys.append(attachment.atomKey)
            }
        }

        XCTAssertEqual(keys, ["counterCard:0", "counterCard:1", "client-1:9"])
    }

    func testRender_unregisteredVoidBlockKeepsBareReplacementCharacter() {
        let result = RenderBridge.renderElements(
            fromJSON: #"[{"type":"voidBlock","nodeType":"counterCard","docPos":1}]"#,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "\u{FFFC}")
        XCTAssertNil(result.attribute(.attachment, at: 0, effectiveRange: nil))
    }

    func testRender_measuredAtomHeightOverridesEstimate() throws {
        let result = RenderBridge.renderElements(
            fromJSON: #"[{"type":"voidBlock","nodeType":"counterCard","docPos":1}]"#,
            baseFont: baseFont,
            textColor: textColor,
            atomConfiguration: AtomRenderConfiguration(
                registeredNodeTypes: ["counterCard"],
                estimatedHeights: ["counterCard": 120],
                measuredHeights: ["counterCard:0": 260]
            )
        )

        let attachment = try XCTUnwrap(
            result.attribute(.attachment, at: 0, effectiveRange: nil) as? AtomBlockAttachment
        )
        XCTAssertEqual(attachment.reservedHeight, 260)
    }

    func testRender_proseMirrorListItemName() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "list_item", "depth": 1,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "Item")
        XCTAssertNotNil(result.attribute(RenderBridgeAttributes.listContext, at: 0, effectiveRange: nil))
    }

    func testRender_horizontalRuleCollapsesAdjacentParagraphSpacing() {
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
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "spacingAfter": 14,
            ],
            "horizontalRule": [
                "verticalMargin": 10,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let nsString = result.string as NSString
        let aboveRange = nsString.range(of: "Above")
        let hrRange = nsString.range(of: "\u{FFFC}")
        guard aboveRange.location != NSNotFound, hrRange.location != NSNotFound else {
            XCTFail("expected both paragraph text and horizontal rule in rendered output")
            return
        }

        let aboveParagraphStyle = result.attribute(.paragraphStyle, at: aboveRange.location, effectiveRange: nil)
            as? NSParagraphStyle
        let separatorParagraphStyle = result.attribute(
            .paragraphStyle,
            at: hrRange.location + hrRange.length,
            effectiveRange: nil
        ) as? NSParagraphStyle
        let attachment = result.attribute(.attachment, at: hrRange.location, effectiveRange: nil)
            as? HorizontalRuleAttachment

        XCTAssertEqual(attachment?.verticalPadding ?? 0, 10, accuracy: 0.1)
        XCTAssertEqual(aboveParagraphStyle?.paragraphSpacing ?? -1, 4, accuracy: 0.1)
        XCTAssertEqual(separatorParagraphStyle?.paragraphSpacing ?? -1, 4, accuracy: 0.1)
    }

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

}
