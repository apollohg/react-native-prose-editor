import XCTest
import CoreText

extension RenderBridgeTests {
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

    func testRender_leadingTopLevelChildMetadataCoversWholeEmoji() {
        let blocks: [[[String: Any]]] = [[
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
            ["type": "textRun", "text": "😀", "marks": []],
            ["type": "blockEnd"],
        ]]

        let result = RenderBridge.renderBlocks(
            fromArray: blocks,
            baseFont: baseFont,
            textColor: textColor
        )
        let firstComposedRange = (result.string as NSString)
            .rangeOfComposedCharacterSequence(at: 0)
        var effectiveRange = NSRange(location: NSNotFound, length: 0)
        let value = result.attribute(
            RenderBridgeAttributes.topLevelChildIndex,
            at: 0,
            longestEffectiveRange: &effectiveRange,
            in: NSRange(location: 0, length: result.length)
        ) as? NSNumber

        XCTAssertEqual(result.string, "😀")
        XCTAssertGreaterThan(firstComposedRange.length, 1, "test must cover a surrogate-pair emoji")
        XCTAssertEqual(value?.intValue, 0)
        XCTAssertEqual(
            effectiveRange,
            firstComposedRange,
            "top-level metadata must not split a leading emoji surrogate pair into separate attribute runs"
        )
    }

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
        XCTAssertEqual(
            attrs[RenderBridgeAttributes.linkHref] as? String,
            "https://openai.com"
        )
    }

    func testRender_linkMarkUsesThemeOverrides() {
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
            textColor: textColor,
            theme: EditorTheme(dictionary: [
                "links": [
                    "color": "#445566",
                    "backgroundColor": "#eef6ff",
                    "fontSize": 18,
                    "fontWeight": "700",
                    "fontStyle": "italic",
                    "underline": false,
                ],
            ])
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertEqual(attrs[.foregroundColor] as? UIColor, EditorTheme.color(from: "#445566"))
        XCTAssertEqual(attrs[.backgroundColor] as? UIColor, EditorTheme.color(from: "#eef6ff"))
        XCTAssertNil(attrs[.underlineStyle])
        XCTAssertEqual(font?.pointSize, 18)
        XCTAssertTrue(font?.fontDescriptor.symbolicTraits.contains(.traitBold) == true)
        XCTAssertTrue(font?.fontDescriptor.symbolicTraits.contains(.traitItalic) == true)
        XCTAssertEqual(
            attrs[RenderBridgeAttributes.linkHref] as? String,
            "https://openai.com"
        )
    }

    func testRenderBlocks_withLeadingSeparatorDoesNotDuplicateTopLevelChildIndexOnContent() {
        let blocks: [[[String: Any]]] = [[
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
            ["type": "textRun", "text": "Hello", "marks": []],
            ["type": "blockEnd"],
        ]]

        let result = RenderBridge.renderBlocks(
            fromArray: blocks,
            startIndex: 3,
            includeLeadingInterBlockSeparator: true,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "\nHello")
        XCTAssertEqual(
            (result.attribute(RenderBridgeAttributes.topLevelChildIndex, at: 0, effectiveRange: nil)
                as? NSNumber)?.intValue,
            3
        )
        XCTAssertNil(
            result.attribute(RenderBridgeAttributes.topLevelChildIndex, at: 1, effectiveRange: nil),
            "Leading content should not duplicate the separator's top-level child index"
        )
    }

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

    /// Inline code must retain every requested emphasis trait through the same
    /// monospace resolver used by prepared viewer runs.
    func testRender_inlineCodeMatchesViewerForBoldAndItalicCombinations() {
        let cases: [(name: String, marks: [Any], traits: UIFontDescriptor.SymbolicTraits)] = [
            ("bold", ["code", "bold"], [.traitBold]),
            ("italic", ["code", "italic"], [.traitItalic]),
            ("bold italic", ["code", "bold", "italic"], [.traitBold, .traitItalic]),
        ]
        let viewer = ViewerFontEnvironment(notificationCenter: .default)

        for fixture in cases {
            let editor = RenderBridge.attributesForMarks(
                fixture.marks,
                baseFont: baseFont,
                textColor: textColor
            )[.font] as? UIFont
            let preparedViewer = viewer.resolveFont(
                family: "monospace",
                size: baseFont.pointSize,
                fallback: baseFont,
                additionalTraits: fixture.traits,
                semanticGeneration: "inline-code-parity-\(fixture.name)"
            )

            XCTAssertNotNil(editor, "\(fixture.name) inline code should resolve a font")
            XCTAssertTrue(editor!.fontDescriptor.symbolicTraits.contains(.traitMonoSpace))
            XCTAssertTrue(
                ViewerFontEnvironment.satisfiesRequestedEmphasis(editor!, requestedTraits: fixture.traits),
                "editor inline code must retain \(fixture.name) emphasis"
            )
            XCTAssertTrue(
                ViewerFontEnvironment.satisfiesRequestedEmphasis(preparedViewer, requestedTraits: fixture.traits),
                "prepared viewer inline code must retain \(fixture.name) emphasis"
            )
            XCTAssertEqual(
                editor!.fontDescriptor.symbolicTraits.intersection([.traitBold, .traitItalic]),
                preparedViewer.fontDescriptor.symbolicTraits.intersection([.traitBold, .traitItalic]),
                "editor and prepared viewer must agree for \(fixture.name) inline code"
            )
        }
    }

    func testRender_inlineCodePreservesTraitsWithThemedCustomLinkFamily() {
        let theme = EditorTheme(dictionary: [
            "links": ["fontFamily": "AppleColorEmoji"],
        ])
        let requested: UIFontDescriptor.SymbolicTraits = [.traitBold, .traitItalic]
        let font = RenderBridge.attributesForMarks(
            ["link", "code", "bold", "italic"],
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )[.font] as? UIFont

        XCTAssertNotNil(font)
        XCTAssertTrue(font!.fontDescriptor.symbolicTraits.contains(.traitMonoSpace))
        XCTAssertTrue(ViewerFontEnvironment.satisfiesRequestedEmphasis(font!, requestedTraits: requested))
        XCTAssertNotEqual(font!.familyName, UIFont(name: "AppleColorEmoji", size: 17)!.familyName)
    }

    /// A code block with no marks and no theme override must still render as
    /// regular-weight monospace (baseline behavior, must not regress).
    func testRender_codeBlock_plainTextIsRegularMonospace() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "let x", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(fromJSON: json, baseFont: baseFont, textColor: textColor)

        let font = result.attributes(at: 0, effectiveRange: nil)[.font] as? UIFont
        XCTAssertNotNil(font)
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitMonoSpace)
                || font!.fontName.lowercased().contains("mono"),
            "Plain code block text should be monospaced. Got font: \(font!.fontName)"
        )
        XCTAssertFalse(
            font!.fontDescriptor.symbolicTraits.contains(.traitBold),
            "Plain code block text must not be bold. Got font: \(font!.fontName)"
        )
    }

    /// Bold marks inside a code block must survive the monospace substitution
    /// (parity with Android, which layers StyleSpan(BOLD) over the monospace
    /// typeface).
    func testRender_codeBlock_preservesBoldTrait() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "let x", "marks": [{"type": "bold"}]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(fromJSON: json, baseFont: baseFont, textColor: textColor)

        let font = result.attributes(at: 0, effectiveRange: nil)[.font] as? UIFont
        XCTAssertNotNil(font)
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitBold),
            "Bold trait must survive in code blocks; got \(font!.fontName)"
        )
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitMonoSpace)
                || font!.fontName.lowercased().contains("mono"),
            "Code block text should still be monospaced"
        )
    }

    /// Combined bold+italic marks inside a code block must not silently lose
    /// BOTH traits when the mono family lacks a bold-italic face. Bold must
    /// always survive; italic survives whenever the resolved face supports
    /// layering it on top of bold. This uses the system-default mono
    /// substitution path (no theme font family override) so both traits are
    /// expected to survive deterministically.
    func testRender_codeBlock_preservesBoldAndItalicTraits() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "let x", "marks": [{"type": "bold"}, {"type": "italic"}]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(fromJSON: json, baseFont: baseFont, textColor: textColor)

        let font = result.attributes(at: 0, effectiveRange: nil)[.font] as? UIFont
        XCTAssertNotNil(font)
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitBold),
            "Bold trait must always survive in code blocks, even combined with italic; got \(font!.fontName)"
        )
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitItalic),
            "Italic trait should survive alongside bold on the system-default monospace path; got \(font!.fontName)"
        )
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitMonoSpace)
                || font!.fontName.lowercased().contains("mono"),
            "Code block text should still be monospaced"
        )
    }

}
