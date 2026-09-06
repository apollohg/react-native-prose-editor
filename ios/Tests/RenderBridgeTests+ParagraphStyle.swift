import XCTest
import CoreText

extension RenderBridgeTests {
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
            style.firstLineHeadIndent, baseIndent + LayoutConstants.listMarkerWidth,
            "List item first line indent should reserve marker width"
        )
        XCTAssertEqual(
            style.headIndent, baseIndent + LayoutConstants.listMarkerWidth,
            "List item head indent should include marker width"
        )
    }

    func testParagraphStyle_listBaseIndentMultiplierCanCollapseTopLevelIndent() {
        let listCtx: [String: Any] = [
            "ordered": false,
            "index": 1,
            "total": 1,
            "start": 1,
            "isFirst": true,
            "isLast": true,
        ]
        let topLevelCtx = BlockContext(nodeType: "paragraph", depth: 1, listContext: listCtx)
        let nestedCtx = BlockContext(nodeType: "paragraph", depth: 2, listContext: listCtx)
        let theme = EditorTheme(dictionary: [
            "list": [
                "indent": 24,
                "baseIndentMultiplier": 0,
            ],
        ])

        let topLevelStyle = RenderBridge.paragraphStyleForBlock(
            topLevelCtx,
            blockStack: [topLevelCtx],
            theme: theme,
            baseFont: baseFont
        )
        let nestedStyle = RenderBridge.paragraphStyleForBlock(
            nestedCtx,
            blockStack: [nestedCtx],
            theme: theme,
            baseFont: baseFont
        )

        XCTAssertEqual(
            topLevelStyle.firstLineHeadIndent,
            LayoutConstants.listMarkerWidth,
            accuracy: 0.1,
            "Top-level list items should be flush-left apart from the marker gutter"
        )
        XCTAssertEqual(
            topLevelStyle.headIndent,
            LayoutConstants.listMarkerWidth,
            accuracy: 0.1,
            "Wrapped lines should align with the marker gutter when the base indent multiplier is zero"
        )
        XCTAssertEqual(
            nestedStyle.headIndent - topLevelStyle.headIndent,
            24,
            accuracy: 0.1,
            "Nested list levels should still add one indent unit each"
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

    func testListMarker_ordered() {
        let ctx: [String: Any] = ["ordered": true, "index": 3]
        let marker = RenderBridge.listMarkerString(listContext: ctx)
        XCTAssertEqual(marker, "3. ", "Ordered list item 3 should produce '3. '")
    }

    func testListMarker_orderedPreservesExactU32Boundary() {
        let max: [String: Any] = ["ordered": true, "index": NSNumber(value: UInt32.max)]
        XCTAssertEqual(RenderBridge.listMarkerString(listContext: max), "4294967295. ")

        for malformedIndex: Any in [
            NSNumber(value: -1),
            NSNumber(value: 1.5),
            NSNull(),
            "1",
            NSNumber(value: UInt64(UInt32.max) + 1),
        ] {
            let context: [String: Any] = ["ordered": true, "index": malformedIndex]
            XCTAssertEqual(
                RenderBridge.listMarkerString(listContext: context),
                "",
                "present malformed ordered-list index must be rejected: \(malformedIndex)"
            )
        }
    }

    func testListMarker_unordered() {
        let ctx: [String: Any] = ["ordered": false, "index": 1]
        let marker = RenderBridge.listMarkerString(listContext: ctx)
        XCTAssertEqual(marker, "\u{2022} ", "Unordered list should produce bullet + space")
    }

}
