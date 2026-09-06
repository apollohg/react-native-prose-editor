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

extension RenderBridgeTests {
    func testVersionedStylesInheritParagraphLineHeightAndReserveBoxSpace() throws {
        let theme = try XCTUnwrap(EditorTheme.from(json: """
        {"version":1,"styles":{"text":{"lineHeight":30,"color":"#123456ff"},"paragraph":{"paddingLeft":12,"paddingRight":7,"borderLeftWidth":4,"borderRightWidth":2,"marginTop":5,"marginBottom":9}}}
        """))
        let rendered = RenderBridge.renderElements(fromArray: [
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
            ["type": "textRun", "text": "Styled", "marks": []], ["type": "blockEnd"]
        ], baseFont: baseFont, textColor: textColor, theme: theme)
        let style = try XCTUnwrap(rendered.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle)
        XCTAssertEqual(style.minimumLineHeight, 30)
        XCTAssertEqual(style.headIndent, 16)
        XCTAssertEqual(style.tailIndent, -9)
        XCTAssertEqual(style.paragraphSpacingBefore, 5)
        XCTAssertEqual(style.paragraphSpacing, 9)
        XCTAssertEqual(theme.effectiveTextStyle(for: "paragraph").color, EditorTheme.color(from: "#123456ff"))
    }

    func testVersionedInlineStylesApplyFixedOrderAndExplicitDecorationReset() throws {
        let theme = try XCTUnwrap(EditorTheme.from(json: """
        {"version":1,"styles":{"bold":{"color":"#ff0000ff"},"link":{"fontWeight":"400","textDecorationLine":"none"},"strike":{"textDecorationLine":"underline line-through","letterSpacing":2}}}
        """))
        let attrs = RenderBridge.attributesForMarks(["strike", "link", "bold"], baseFont: baseFont, textColor: textColor, theme: theme)
        XCTAssertFalse(try XCTUnwrap(attrs[.font] as? UIFont).fontDescriptor.symbolicTraits.contains(.traitBold))
        XCTAssertEqual(attrs[.kern] as? CGFloat, 2)
        XCTAssertEqual(attrs[.underlineStyle] as? Int, NSUnderlineStyle.single.rawValue)
        XCTAssertEqual(attrs[.strikethroughStyle] as? Int, NSUnderlineStyle.single.rawValue)
    }

    func testVersionedThemeRejectsUnknownVersionAndInvalidStylesShape() {
        XCTAssertNil(EditorTheme.from(json: "{\"version\":true,\"styles\":{}}"))
        XCTAssertNil(EditorTheme.from(json: "{\"version\":1.5,\"styles\":{}}"))
        XCTAssertNil(EditorTheme.from(json: "{\"version\":2,\"styles\":{}}"))
        XCTAssertNil(EditorTheme.from(json: "{\"version\":1,\"styles\":[]}"))
    }
}

extension RenderBridgeTests {
    func testStyleBoxResolvesAsymmetricBordersCornersAndImageFit() {
        let box = EditorStyleBox(["borderTopWidth": 2, "borderLeftWidth": 4, "borderRightWidth": 0,
                                  "borderBottomWidth": 3, "paddingLeft": 6, "borderTopLeftRadius": 20,
                                  "borderTopRightRadius": 0, "resizeMode": "contain"])
        XCTAssertEqual(box.inset.left, 10)
        XCTAssertEqual(box.inset.right, 0)
        XCTAssertEqual(box.radii, [20, 0, 0, 0])
        let bounds = CGRect(x: 0, y: 0, width: 100, height: 100)
        XCTAssertFalse(box.path(in: bounds).contains(CGPoint(x: 1, y: 1)))
        XCTAssertTrue(box.path(in: bounds).contains(CGPoint(x: 99, y: 1)))
        XCTAssertEqual(box.imageRect(CGSize(width: 180, height: 90), in: bounds), CGRect(x: 10, y: 27, width: 90, height: 45))
    }

    func testStyledEmptyBlockRetainsCaretAndDecorationIdentity() throws {
        let theme = try XCTUnwrap(EditorTheme.from(json: """
        {"version":1,"styles":{"paragraph":{"backgroundColor":"#ff0000ff","paddingTop":10,"paddingBottom":12}}}
        """))
        let rendered = RenderBridge.renderElements(fromArray: [
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0], ["type": "blockEnd"]
        ], baseFont: baseFont, textColor: textColor, theme: theme)
        XCTAssertGreaterThan(rendered.length, 0)
        XCTAssertNotNil(rendered.attribute(editorStyleBoxesAttribute, at: 0, effectiveRange: nil))
        XCTAssertEqual(rendered.attribute(RenderBridgeAttributes.syntheticPlaceholder, at: 0, effectiveRange: nil) as? Bool, true)
    }
}

extension RenderBridgeTests {
    func testStyledWrappedParagraphKeepsTextKitCaretInsideAllocatedBox() throws {
        let theme = try XCTUnwrap(EditorTheme.from(json: """
        {"version":1,"styles":{"paragraph":{"paddingTop":11,"paddingBottom":13,"paddingLeft":7,"borderTopWidth":2,"borderLeftWidth":4,"marginTop":17,"marginBottom":19}}}
        """))
        let rendered = RenderBridge.renderElements(fromArray: [
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
            ["type": "textRun", "text": String(repeating: "wrapped text ", count: 8), "marks": []],
            ["type": "blockEnd"]
        ], baseFont: baseFont, textColor: textColor, theme: theme)
        let storage = NSTextStorage(attributedString: rendered)
        let manager = EditorLayoutManager()
        let container = NSTextContainer(size: CGSize(width: 160, height: 10_000))
        container.lineFragmentPadding = 0
        manager.addTextContainer(container)
        storage.addLayoutManager(manager)
        manager.ensureLayout(for: container)
        let first = manager.lineFragmentUsedRect(forGlyphAt: 0, effectiveRange: nil)
        XCTAssertEqual(first.minX, 11)
        XCTAssertEqual(first.minY, 30)
        var count = 0
        manager.enumerateLineFragments(forGlyphRange: NSRange(location: 0, length: manager.numberOfGlyphs)) { _, used, _, _, _ in
            XCTAssertEqual(used.minX, 11)
            count += 1
        }
        XCTAssertGreaterThan(count, 1)
    }
}

extension RenderBridgeTests {
    func testInlineLineHeightExpandsOnlyItsTextKitLine() throws {
        let theme = try XCTUnwrap(EditorTheme.from(json: """
        {"version":1,"styles":{"bold":{"lineHeight":60},"paragraph":{"marginBottom":0}}}
        """))
        let rendered = RenderBridge.renderElements(fromArray: [
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
            ["type": "textRun", "text": "Tall", "marks": ["bold"]], ["type": "blockEnd"],
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
            ["type": "textRun", "text": "Short", "marks": []], ["type": "blockEnd"]
        ], baseFont: baseFont, textColor: textColor, theme: theme)
        let storage = NSTextStorage(attributedString: rendered)
        let manager = EditorLayoutManager()
        let container = NSTextContainer(size: CGSize(width: 200, height: 500))
        manager.addTextContainer(container); storage.addLayoutManager(manager)
        manager.ensureLayout(for: container)
        XCTAssertEqual(manager.lineFragmentUsedRect(forGlyphAt: 0, effectiveRange: nil).height, 60)
        XCTAssertLessThan(manager.lineFragmentUsedRect(forGlyphAt: manager.numberOfGlyphs - 1, effectiveRange: nil).height, 60)
    }
}

extension RenderBridgeTests {
    func testAtomicAdmissionAcceptsCoreLanguageAndRejectsMalformedLanguage() throws {
        let config = #"{"schema":{"nodes":[{"name":"doc","content":"block+","role":"doc"},{"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},{"name":"codeBlock","content":"text*","group":"block","role":"textBlock","attrs":{"language":{"default":null}}},{"name":"text","group":"inline","role":"text"}],"marks":[]},"initialization":{"type":"localEmpty"}}"#
        let editorId = makeV2Editor(configJson: config)
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setJson(id: editorId, json: #"{"type":"doc","content":[{"type":"codeBlock","attrs":{"language":"rust"},"content":[{"type":"text","text":"let value = 1;"}]}]}"#)
        let raw = try XCTUnwrap(editorV2RenderUpdate(editorId: String(editorId), mirrorScalarAnchor: nil, mirrorScalarHead: nil).value)
        XCTAssertNotNil(EditorV2Adapter.parseAtomicRenderSnapshot(raw))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any])
        var blocks = try XCTUnwrap(object["renderBlocks"] as? [[[String: Any]]])
        for language: Any in [42, true, ["bad"]] {
            blocks[0][0]["language"] = language
            object["renderBlocks"] = blocks
            XCTAssertNil(EditorV2Adapter.parseAtomicRenderSnapshot(String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)))
        }
    }

    func testAtomicAdmissionValidatesRichMentionStyleWithoutOpeningUnknownKeys() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let raw = try XCTUnwrap(editorV2RenderUpdate(editorId: String(editorId), mirrorScalarAnchor: nil, mirrorScalarHead: nil).value)
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any])
        func snapshot(style: Any) throws -> String {
            object["renderBlocks"] = [[
                ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
                ["type": "opaqueInlineAtom", "nodeType": "mention", "label": "Jay", "docPos": 1,
                 "mentionTheme": ["node": ["style": style]]],
                ["type": "blockEnd"]
            ]]
            return String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
        }
        XCTAssertNotNil(EditorV2Adapter.parseAtomicRenderSnapshot(try snapshot(style: ["fontSize": 20, "color": "#123456ff", "borderLeftWidth": 3, "borderTopRightRadius": 8])))
        for style: Any in [[], ["unknown": 1], ["fontSize": -1], ["borderLeftWidth": true], ["color": "bad-color"], ["fontStyle": "oblique"]] {
            XCTAssertNil(EditorV2Adapter.parseAtomicRenderSnapshot(try snapshot(style: style)))
        }
    }
}

extension RenderBridgeTests {
    func testRichMentionReservesChipGeometryWithoutChangingTextOffsets() throws {
        let theme = try XCTUnwrap(EditorTheme.from(json: #"{"version":1,"styles":{"mention":{"fontSize":20,"borderLeftWidth":7,"borderRightWidth":9,"borderTopWidth":3,"borderBottomWidth":5}}}"#))
        let rendered = RenderBridge.renderElements(fromArray: [
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
            ["type": "opaqueInlineAtom", "nodeType": "mention", "label": "Jay Den", "docPos": 1],
            ["type": "textRun", "text": "!", "marks": []], ["type": "blockEnd"]
        ], baseFont: baseFont, textColor: textColor, theme: theme)
        XCTAssertEqual(rendered.string, "Jay Den!")
        let storage = NSTextStorage(attributedString: rendered)
        let manager = EditorLayoutManager()
        let container = NSTextContainer(size: CGSize(width: 300, height: 500))
        container.lineFragmentPadding = 0
        manager.addTextContainer(container)
        storage.addLayoutManager(manager)
        manager.ensureLayout(for: container)
        let font = try XCTUnwrap(rendered.attribute(.font, at: 0, effectiveRange: nil) as? UIFont)
        let labelSize = ("Jay Den" as NSString).size(withAttributes: [.font: font])
        let following = manager.location(forGlyphAt: manager.glyphIndexForCharacter(at: 7))
        XCTAssertEqual(following.x, ceil(labelSize.width) + 12 + 7 + 9, accuracy: 1)
        XCTAssertGreaterThanOrEqual(manager.lineFragmentUsedRect(forGlyphAt: 0, effectiveRange: nil).height, ceil(labelSize.height) + 8 + 3 + 5)
        XCTAssertEqual(storage.string, "Jay Den!")
    }
}

extension RenderBridgeTests {
    func testCodeBlockVerticalInsetsApplyOnlyAtOuterBoundaries() throws {
        let theme = try XCTUnwrap(EditorTheme.from(json: #"{"version":1,"styles":{"codeBlock":{"paddingTop":12,"paddingBottom":12,"marginTop":12,"marginBottom":12}}}"#))
        let rendered = RenderBridge.renderElements(fromArray: [
            ["type": "blockStart", "nodeType": "codeBlock", "depth": 0],
            ["type": "textRun", "text": "first\nsecond\nthird", "marks": []], ["type": "blockEnd"]
        ], baseFont: baseFont, textColor: textColor, theme: theme)
        let middle = try XCTUnwrap(rendered.attribute(.paragraphStyle, at: 6, effectiveRange: nil) as? NSParagraphStyle)
        XCTAssertEqual(middle.paragraphSpacingBefore, 0)
        XCTAssertEqual(middle.paragraphSpacing, 0)
        let storage = NSTextStorage(attributedString: rendered)
        let manager = EditorLayoutManager()
        let container = NSTextContainer(size: CGSize(width: 300, height: 500))
        manager.addTextContainer(container)
        storage.addLayoutManager(manager)
        manager.ensureLayout(for: container)
        var lines: [CGRect] = []
        manager.enumerateLineFragments(forGlyphRange: NSRange(location: 0, length: manager.numberOfGlyphs)) { _, used, _, _, _ in lines.append(used) }
        XCTAssertEqual(lines.count, 3)
        XCTAssertEqual(lines[1].minY - lines[0].minY, lines[0].height, accuracy: 1)
        XCTAssertEqual(lines[2].minY - lines[1].minY, lines[1].height, accuracy: 1)
    }

    func testQuoteDecorationExcludesPreviousParagraphSeparator() throws {
        let theme = try XCTUnwrap(EditorTheme.from(json: ##"{"version":1,"styles":{"blockquote":{"borderLeftWidth":3,"borderLeftColor":"#123456ff","paddingTop":8}}}"##))
        let rendered = RenderBridge.renderElements(fromArray: [
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
            ["type": "textRun", "text": "Before", "marks": []], ["type": "blockEnd"],
            ["type": "blockStart", "nodeType": "blockquote", "depth": 0],
            ["type": "blockStart", "nodeType": "paragraph", "depth": 1],
            ["type": "textRun", "text": "Quote", "marks": []], ["type": "blockEnd"], ["type": "blockEnd"]
        ], baseFont: baseFont, textColor: textColor, theme: theme)
        XCTAssertEqual(rendered.string, "Before\nQuote")
        let previousBoxes = rendered.attribute(editorStyleBoxesAttribute, at: 6, effectiveRange: nil) as? [EditorRenderedBox] ?? []
        XCTAssertFalse(previousBoxes.contains { $0.box.color("borderLeftColor") == EditorTheme.color(from: "#123456ff") })
        let quoteBoxes = rendered.attribute(editorStyleBoxesAttribute, at: 7, effectiveRange: nil) as? [EditorRenderedBox] ?? []
        XCTAssertTrue(quoteBoxes.contains { $0.box.color("borderLeftColor") == EditorTheme.color(from: "#123456ff") })
    }
}
