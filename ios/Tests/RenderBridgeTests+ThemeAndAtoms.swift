import XCTest
import CoreText

extension RenderBridgeTests {
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
                "node": [
                    "textColor": "#112233",
                    "backgroundColor": "#ddeeff",
                    "fontWeight": "bold",
                ],
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

    func testRender_mentionInlineAtomMergesElementMentionThemeOverride() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {
                "type": "opaqueInlineAtom",
                "nodeType": "mention",
                "label": "@Alice",
                "docPos": 1,
                "mentionTheme": {"node": {"textColor": "#445566"}}
            },
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "mentions": [
                "node": [
                    "textColor": "#112233",
                    "backgroundColor": "#ddeeff",
                    "fontWeight": "bold",
                ],
            ],
        ])
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        XCTAssertEqual(result.string, "@Alice")

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        XCTAssertEqual(
            attrs[.foregroundColor] as? UIColor,
            UIColor(
                red: 0x44 as CGFloat / 255.0,
                green: 0x55 as CGFloat / 255.0,
                blue: 0x66 as CGFloat / 255.0,
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
            "Mention override should preserve global bold styling. Got: \(String(describing: font))"
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

    func testRender_themeOverridesSpecificHeadingLevelTypography() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "h2", "depth": 0},
            {"type": "textRun", "text": "Section title", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "text": [
                "fontSize": 16,
                "color": "#112233",
            ],
            "headings": [
                "h2": [
                    "fontSize": 28,
                    "fontWeight": "700",
                    "color": "#445566",
                    "lineHeight": 34,
                    "spacingAfter": 12,
                ],
                "h4": [
                    "fontSize": 18,
                    "color": "#AA5500",
                ],
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

        XCTAssertEqual(font?.pointSize ?? 0, 28, accuracy: 0.1)
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "Configured h2 heading should resolve to a bold font"
        )
        XCTAssertEqual(color, EditorTheme.color(from: "#445566"))
        XCTAssertEqual(paragraphStyle?.minimumLineHeight ?? 0, 34, accuracy: 0.1)
        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? 0, 12, accuracy: 0.1)
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

    func testRender_nestedListSpacingAfter() {
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
            {"type": "textRun", "text": "Parent item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Nested item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "After nested", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "After list", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "list": [
                "itemSpacing": 6,
                "spacingAfter": 20,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )
        let text = result.string as NSString
        let firstStyle = result.attribute(
            .paragraphStyle,
            at: text.range(of: "First item").location,
            effectiveRange: nil
        ) as? NSParagraphStyle
        let nestedStyle = result.attribute(
            .paragraphStyle,
            at: text.range(of: "Nested item").location,
            effectiveRange: nil
        ) as? NSParagraphStyle
        let outerFinalStyle = result.attribute(
            .paragraphStyle,
            at: text.range(of: "After nested").location,
            effectiveRange: nil
        ) as? NSParagraphStyle

        XCTAssertEqual(firstStyle?.paragraphSpacing ?? -1, 6, accuracy: 0.1)
        XCTAssertEqual(nestedStyle?.paragraphSpacing ?? -1, 20, accuracy: 0.1)
        XCTAssertEqual(outerFinalStyle?.paragraphSpacing ?? -1, 20, accuracy: 0.1)
    }

    func testRender_stackedNestedListSpacingAfter() {
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
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "After list", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "list": [
                "itemSpacing": 6,
                "spacingAfter": 20,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )
        let text = result.string as NSString
        let nestedStyle = result.attribute(
            .paragraphStyle,
            at: text.range(of: "Nested item").location,
            effectiveRange: nil
        ) as? NSParagraphStyle

        XCTAssertEqual(nestedStyle?.paragraphSpacing ?? -1, 40, accuracy: 0.1)
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

}
