import XCTest
import CoreText

extension RenderBridgeTests {
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
        XCTAssertEqual(firstStyle?.firstLineHeadIndent, 48.0 + LayoutConstants.listMarkerWidth)
        XCTAssertEqual(firstStyle?.headIndent, 48.0 + LayoutConstants.listMarkerWidth)
    }

    func testOrderedListMarkerFormatterCyclesSchemesAndFormatsBoundaries() {
        let theme = EditorOrderedListMarkerTheme(dictionary: [
            "schemes": ["decimal", "lowerAlpha", "lowerRoman"],
            "suffix": ")",
        ])

        XCTAssertEqual(OrderedListMarkerFormatter.label(index: 1, nestingDepth: 0, theme: theme), "1)")
        XCTAssertEqual(OrderedListMarkerFormatter.label(index: 26, nestingDepth: 1, theme: theme), "z)")
        XCTAssertEqual(OrderedListMarkerFormatter.label(index: 27, nestingDepth: 1, theme: theme), "aa)")
        XCTAssertEqual(OrderedListMarkerFormatter.label(index: 9, nestingDepth: 2, theme: theme), "ix)")
        XCTAssertEqual(OrderedListMarkerFormatter.label(index: 2, nestingDepth: 3, theme: theme), "2)")
    }

    func testOrderedListMarkerFormatterFallsBackToDecimal() {
        let invalid = EditorOrderedListMarkerTheme(dictionary: [
            "schemes": ["unknown"],
            "suffix": "!",
        ])

        XCTAssertEqual(OrderedListMarkerFormatter.label(index: 4_000, nestingDepth: 2, theme: invalid), "4000.")
    }

    func testOrderedListMarkerThemeNormalizesMissingEmptyAndMixedSchemes() {
        let missing = EditorOrderedListMarkerTheme(dictionary: [:])
        let empty = EditorOrderedListMarkerTheme(dictionary: ["schemes": []])
        let mixed = EditorOrderedListMarkerTheme(dictionary: [
            "schemes": ["lowerAlpha", 7, NSNull(), "unknown", "upperRoman"],
        ])
        let malformed = EditorOrderedListMarkerTheme(dictionary: [
            "schemes": [7, NSNull(), "unknown"],
        ])

        let defaultSchemes: [EditorOrderedListNumberingScheme] = [
            .decimal,
            .lowerAlpha,
            .lowerRoman,
        ]
        XCTAssertEqual(missing.schemes, defaultSchemes)
        XCTAssertEqual(empty.schemes, defaultSchemes)
        XCTAssertEqual(mixed.schemes, [.lowerAlpha, .upperRoman])
        XCTAssertEqual(malformed.schemes, defaultSchemes)
    }

    func testOrderedListMarkerFormatterFormatsUppercaseSchemesAndRomanBoundary() {
        let theme = EditorOrderedListMarkerTheme(dictionary: [
            "schemes": ["upperAlpha", "upperRoman"],
            "suffix": ")",
        ])

        XCTAssertEqual(OrderedListMarkerFormatter.label(index: 27, nestingDepth: 0, theme: theme), "AA)")
        XCTAssertEqual(OrderedListMarkerFormatter.label(index: 9, nestingDepth: 1, theme: theme), "IX)")
        XCTAssertEqual(OrderedListMarkerFormatter.label(index: 3_999, nestingDepth: 1, theme: theme), "MMMCMXCIX)")
    }

    func testRender_absentOrderedMarkerCyclesDefaultSchemesBySemanticDepth() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Depth zero", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Depth one", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 2,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 3},
            {"type": "textRun", "text": "Depth two", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 3,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 4},
            {"type": "textRun", "text": "Depth three", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: EditorTheme(dictionary: ["list": [:]])
        )

        let text = result.string as NSString
        let labels = ["Depth zero", "Depth one", "Depth two", "Depth three"].map { value in
            result.attribute(
                RenderBridgeAttributes.orderedListMarkerLabel,
                at: text.range(of: value).location,
                effectiveRange: nil
            ) as? String
        }

        XCTAssertEqual(labels, ["1.", "a.", "i.", "1."])
    }

    func testRender_orderedUnderTaskUsesTaskAncestryDepth() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "taskItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "kind": "task", "checked": false, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Task", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Nested ordered", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "list": ["orderedMarker": ["schemes": ["decimal", "lowerAlpha"]]],
        ])
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )
        let nestedLocation = (result.string as NSString).range(of: "Nested ordered").location

        XCTAssertEqual(
            result.attribute(
                RenderBridgeAttributes.orderedListMarkerLabel,
                at: nestedLocation,
                effectiveRange: nil
            ) as? String,
            "a."
        )
    }

    func testRender_taskKindTakesPrecedenceOverAdversarialOrderedFlag() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "taskItem", "depth": 0,
             "listContext": {"ordered": true, "index": 4, "kind": "task", "checked": false, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Task", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: EditorTheme(dictionary: [
                "list": ["orderedMarker": ["schemes": ["upperRoman"], "suffix": ")"]],
            ])
        )

        XCTAssertEqual(
            RenderBridge.listMarkerString(listContext: [
                "ordered": true,
                "index": 4,
                "kind": "task",
                "checked": false,
            ]),
            "\u{2610} "
        )
        XCTAssertNil(
            result.attribute(
                RenderBridgeAttributes.orderedListMarkerLabel,
                at: 0,
                effectiveRange: nil
            )
        )
    }

    func testRender_nestedOrderedListUsesThemedLabelsWithoutChangingCanonicalMarkers() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Outer item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Nested item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "list": [
                "orderedMarker": [
                    "schemes": ["decimal", "lowerAlpha"],
                    "suffix": ".",
                ],
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )
        let text = result.string as NSString
        let outerLocation = text.range(of: "Outer item").location
        let nestedLocation = text.range(of: "Nested item").location

        XCTAssertEqual(
            result.attribute(
                RenderBridgeAttributes.orderedListMarkerLabel,
                at: outerLocation,
                effectiveRange: nil
            ) as? String,
            "1."
        )
        XCTAssertEqual(
            result.attribute(
                RenderBridgeAttributes.orderedListMarkerLabel,
                at: nestedLocation,
                effectiveRange: nil
            ) as? String,
            "a."
        )
        XCTAssertEqual(
            RenderBridge.listMarkerString(listContext: ["ordered": true, "index": 1]),
            "1. "
        )
        XCTAssertEqual(
            RenderBridge.listMarkerString(listContext: ["ordered": true, "index": 2]),
            "2. "
        )
    }

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

}
