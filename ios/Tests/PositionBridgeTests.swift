import XCTest

// MARK: - PositionBridge Tests

final class PositionBridgeTests: XCTestCase {

    private func makeTextView(with attributedText: NSAttributedString) -> UITextView {
        let layoutManager = EditorLayoutManager()
        let textContainer = NSTextContainer(size: .zero)
        let textStorage = NSTextStorage(attributedString: attributedText)
        layoutManager.addTextContainer(textContainer)
        textStorage.addLayoutManager(layoutManager)
        return UITextView(frame: .zero, textContainer: textContainer)
    }

    // MARK: - UTF-16 -> Scalar: ASCII

    /// ASCII characters are 1 UTF-16 code unit = 1 scalar each.
    func testUtf16ToScalar_asciiOnly() {
        let text = "Hello"
        // "Hello" = 5 UTF-16 code units = 5 scalars
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(0, in: text), 0,
            "Offset 0 should map to scalar 0"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(1, in: text), 1,
            "Offset 1 in ASCII should map to scalar 1"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(5, in: text), 5,
            "Offset 5 (end of 'Hello') should map to scalar 5"
        )
    }

    /// Empty string edge case.
    func testUtf16ToScalar_emptyString() {
        let text = ""
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(0, in: text), 0,
            "Empty string at offset 0 should return scalar 0"
        )
    }

    func testUtf16ToScalar_listParagraphAddsVirtualMarkerScalars() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = makeTextView(with: attributed)

        XCTAssertEqual(PositionBridge.utf16OffsetToScalar(0, in: textView), 2)
        XCTAssertEqual(PositionBridge.utf16OffsetToScalar(4, in: textView), 6)
    }

    func testScalarToUtf16_listMarkerPrefixMapsToParagraphStart() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 12, "total": 1, "start": 12, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = makeTextView(with: attributed)

        XCTAssertEqual(PositionBridge.scalarToUtf16Offset(1, in: textView), 0)
        XCTAssertEqual(PositionBridge.scalarToUtf16Offset(4, in: textView), 0)
        XCTAssertEqual(PositionBridge.scalarToUtf16Offset(5, in: textView), 1)
    }

    func testUtf16ToScalar_secondParagraphInListItemDoesNotAddAnotherVirtualMarker() {
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
        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = makeTextView(with: attributed)

        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(2, in: textView), 4,
            "Only the first paragraph in a list item should contribute virtual marker scalars"
        )
    }

    func testUtf16ToScalar_hardBreakInsideListParagraphDoesNotAddAnotherVirtualMarker() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 4},
            {"type": "textRun", "text": "B", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = makeTextView(with: attributed)

        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(2, in: textView), 4,
            "A hardBreak inside a list paragraph should not create a second virtual marker"
        )
        XCTAssertTrue(
            EditorLayoutManager.isParagraphStartCreatedByHardBreak(2, in: textView.textStorage),
            "The visual line after a hardBreak should be recognized as a synthetic paragraph start"
        )
    }

    func testUtf16ToScalar_trailingHardBreakPlaceholderDoesNotCountAsScalar() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 2},
            {"type": "blockEnd"}
        ]
        """
        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = makeTextView(with: attributed)

        XCTAssertEqual(textView.text, "A\n\u{200B}")
        XCTAssertEqual(PositionBridge.utf16OffsetToScalar(2, in: textView), 2)
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(3, in: textView),
            2,
            "The synthetic trailing hardBreak placeholder should not count as content"
        )
    }

    // MARK: - UTF-16 -> Scalar: BMP Emoji (1 scalar, 1 UTF-16)

    /// Simple emoji in the BMP (e.g. U+263A, white smiley face) are 1 UTF-16 code unit.
    func testUtf16ToScalar_bmpCharacters() {
        // U+00E9 (e-acute) is 1 UTF-16 code unit, 1 scalar
        let text = "caf\u{00E9}"  // "cafe" with accent
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(4, in: text), 4,
            "BMP character: UTF-16 offset 4 should be scalar 4"
        )
    }

    // MARK: - UTF-16 -> Scalar: Surrogate Pairs (1 scalar, 2 UTF-16)

    /// Characters above U+FFFF (supplementary plane) are encoded as
    /// surrogate pairs in UTF-16: 2 code units per scalar.
    func testUtf16ToScalar_surrogatePair() {
        // U+1F600 (grinning face) = 2 UTF-16 code units, 1 scalar
        let text = "A\u{1F600}B"
        // UTF-16: A(1) + U+1F600(2) + B(1) = 4 code units
        // Scalars: A(1) + U+1F600(1) + B(1) = 3 scalars

        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(0, in: text), 0,
            "Before 'A'"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(1, in: text), 1,
            "After 'A', before emoji"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(3, in: text), 2,
            "After emoji (UTF-16 offset 3 = scalar 2)"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(4, in: text), 3,
            "After 'B' (end of string)"
        )
    }

    /// The family emoji 👨‍👩‍👧‍👦 is composed of multiple scalars joined by ZWJ.
    /// U+1F468 U+200D U+1F469 U+200D U+1F467 U+200D U+1F466
    /// = 7 scalars, 11 UTF-16 code units (4 surrogates + 3 ZWJ)
    /// But it renders as 1 grapheme cluster.
    func testUtf16ToScalar_familyEmoji() {
        let text = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"
        let expectedUtf16Count = 11
        let expectedScalarCount: UInt32 = 7

        let nsString = text as NSString
        XCTAssertEqual(
            nsString.length, expectedUtf16Count,
            "Family emoji should be \(expectedUtf16Count) UTF-16 code units"
        )
        XCTAssertEqual(
            UInt32(text.unicodeScalars.count), expectedScalarCount,
            "Family emoji should be \(expectedScalarCount) Unicode scalars"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(expectedUtf16Count, in: text),
            expectedScalarCount,
            "Full family emoji: UTF-16 end should map to scalar end"
        )
    }

    // MARK: - UTF-16 -> Scalar: CJK

    /// CJK characters (Chinese/Japanese/Korean) are in the BMP: 1 UTF-16 = 1 scalar.
    func testUtf16ToScalar_cjk() {
        let text = "\u{4F60}\u{597D}"  // 你好
        // UTF-16: 2 code units, 2 scalars
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(1, in: text), 1,
            "CJK character at offset 1 should be scalar 1"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(2, in: text), 2,
            "End of CJK string should be scalar 2"
        )
    }

    /// CJK extension B characters (U+20000+) are in the supplementary plane.
    func testUtf16ToScalar_cjkExtensionB() {
        // U+20000 (CJK Unified Ideographs Extension B) = 2 UTF-16 code units, 1 scalar
        let text = "A\u{20000}B"
        // UTF-16: A(1) + U+20000(2) + B(1) = 4
        // Scalars: A(1) + U+20000(1) + B(1) = 3
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(3, in: text), 2,
            "After CJK Extension B character"
        )
    }

    // MARK: - UTF-16 -> Scalar: Mixed Content

    /// Mixed ASCII, emoji, and CJK in one string.
    func testUtf16ToScalar_mixedContent() {
        let text = "Hi\u{1F600}\u{4F60}\u{597D}!"
        // UTF-16: H(1) + i(1) + U+1F600(2) + U+4F60(1) + U+597D(1) + !(1) = 7
        // Scalars: H(1) + i(1) + U+1F600(1) + U+4F60(1) + U+597D(1) + !(1) = 6

        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(2, in: text), 2,
            "After 'Hi'"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(4, in: text), 3,
            "After emoji (UTF-16 offset 4 = scalar 3)"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(7, in: text), 6,
            "End of mixed string"
        )
    }

    // MARK: - Scalar -> UTF-16: ASCII

    func testScalarToUtf16_asciiOnly() {
        let text = "Hello"
        XCTAssertEqual(
            PositionBridge.scalarToUtf16Offset(0, in: text), 0,
            "Scalar 0 should map to UTF-16 offset 0"
        )
        XCTAssertEqual(
            PositionBridge.scalarToUtf16Offset(3, in: text), 3,
            "Scalar 3 in ASCII should map to UTF-16 offset 3"
        )
        XCTAssertEqual(
            PositionBridge.scalarToUtf16Offset(5, in: text), 5,
            "Scalar 5 (end) should map to UTF-16 offset 5"
        )
    }

    // MARK: - Scalar -> UTF-16: Surrogate Pairs

    func testScalarToUtf16_surrogatePair() {
        let text = "A\u{1F600}B"
        // Scalar 0 = A -> UTF-16 offset 0
        // Scalar 1 = U+1F600 -> UTF-16 offset 1
        // Scalar 2 = B -> UTF-16 offset 3 (after 2 UTF-16 code units for emoji)
        XCTAssertEqual(
            PositionBridge.scalarToUtf16Offset(1, in: text), 1,
            "Scalar 1 (emoji start) should be UTF-16 offset 1"
        )
        XCTAssertEqual(
            PositionBridge.scalarToUtf16Offset(2, in: text), 3,
            "Scalar 2 (after emoji) should be UTF-16 offset 3"
        )
        XCTAssertEqual(
            PositionBridge.scalarToUtf16Offset(3, in: text), 4,
            "Scalar 3 (after B) should be UTF-16 offset 4"
        )
    }

    // MARK: - Scalar -> UTF-16: Family Emoji

    func testScalarToUtf16_familyEmoji() {
        let text = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"
        // 7 scalars, 11 UTF-16 code units
        // Scalar 7 (end) should map to UTF-16 offset 11
        XCTAssertEqual(
            PositionBridge.scalarToUtf16Offset(7, in: text), 11,
            "End of family emoji: scalar 7 -> UTF-16 offset 11"
        )
    }

    // MARK: - Roundtrip Consistency

    /// Verify that utf16->scalar->utf16 roundtrips correctly for various inputs.
    func testRoundtrip_utf16ToScalarAndBack() {
        let testCases: [(String, String)] = [
            ("Hello, world!", "ASCII"),
            ("A\u{1F600}B", "surrogate pair"),
            ("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}", "family emoji"),
            ("\u{4F60}\u{597D}\u{4E16}\u{754C}", "CJK"),
            ("abc\u{1F600}\u{4F60}xyz", "mixed"),
            ("\u{0041}\u{0301}", "combining character (A + combining acute)"),
        ]

        for (text, label) in testCases {
            let nsString = text as NSString
            // Test at every valid UTF-16 offset.
            for utf16Offset in 0...nsString.length {
                let scalar = PositionBridge.utf16OffsetToScalar(utf16Offset, in: text)
                let backToUtf16 = PositionBridge.scalarToUtf16Offset(scalar, in: text)
                // The roundtrip may not be exact if utf16Offset lands in the middle
                // of a surrogate pair. But at surrogate boundaries it should match.
                // We verify that the back-converted offset is >= the original
                // (snapping forward past any mid-surrogate position).
                XCTAssertGreaterThanOrEqual(
                    backToUtf16, utf16Offset - 1,
                    "Roundtrip for '\(label)' at UTF-16 offset \(utf16Offset): " +
                    "scalar=\(scalar), back=\(backToUtf16)"
                )
            }
        }
    }

    /// Verify that scalar->utf16->scalar roundtrips exactly.
    func testRoundtrip_scalarToUtf16AndBack() {
        let testCases: [(String, String)] = [
            ("Hello, world!", "ASCII"),
            ("A\u{1F600}B", "surrogate pair"),
            ("\u{4F60}\u{597D}", "CJK"),
            ("abc\u{1F600}\u{4F60}xyz", "mixed"),
        ]

        for (text, label) in testCases {
            let scalarCount = UInt32(text.unicodeScalars.count)
            for scalarOffset in 0...scalarCount {
                let utf16 = PositionBridge.scalarToUtf16Offset(scalarOffset, in: text)
                let backToScalar = PositionBridge.utf16OffsetToScalar(utf16, in: text)
                XCTAssertEqual(
                    backToScalar, scalarOffset,
                    "Roundtrip for '\(label)' at scalar \(scalarOffset): " +
                    "utf16=\(utf16), back=\(backToScalar) - should equal original scalar"
                )
            }
        }
    }

    // MARK: - Grapheme Boundary Snapping

    /// Snapping at an already-valid boundary returns the same offset.
    func testSnapToGraphemeBoundary_alreadyOnBoundary() {
        let text = "Hello"
        for i in 0...5 {
            XCTAssertEqual(
                PositionBridge.snapToGraphemeBoundary(i, in: text), i,
                "ASCII offset \(i) is already on a grapheme boundary"
            )
        }
    }

    /// Snapping in the middle of a surrogate pair snaps to the end of the grapheme cluster.
    func testSnapToGraphemeBoundary_midSurrogatePair() {
        let text = "A\u{1F600}B"
        // UTF-16: A(offset 0), emoji high surrogate(offset 1), low surrogate(offset 2), B(offset 3)
        // Offset 2 is mid-surrogate. The grapheme cluster for the emoji is offsets 1-2.
        // Snapping offset 2 should go to 3 (end of emoji grapheme).
        let snapped = PositionBridge.snapToGraphemeBoundary(2, in: text)
        XCTAssertEqual(
            snapped, 3,
            "Mid-surrogate offset 2 should snap to 3 (end of emoji grapheme)"
        )
    }

    /// Snapping in the middle of a family emoji snaps to the end.
    func testSnapToGraphemeBoundary_midFamilyEmoji() {
        let text = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"
        // The entire sequence is 1 grapheme cluster = 11 UTF-16 code units.
        // Any offset 1..10 should snap to 11.
        for midOffset in 1...10 {
            let snapped = PositionBridge.snapToGraphemeBoundary(midOffset, in: text)
            XCTAssertEqual(
                snapped, 11,
                "Mid-family-emoji offset \(midOffset) should snap to 11 (end of grapheme)"
            )
        }
    }

    /// Snapping in the middle of a combining character sequence.
    func testSnapToGraphemeBoundary_combiningCharacter() {
        // "e" + combining acute accent = "e\u{0301}" = "e" visually with accent
        // This is 2 UTF-16 code units but 1 grapheme cluster.
        let text = "e\u{0301}"
        let snapped = PositionBridge.snapToGraphemeBoundary(1, in: text)
        XCTAssertEqual(
            snapped, 2,
            "Between base and combining character should snap to end of grapheme"
        )
    }

    /// Snapping on empty string always returns 0.
    func testSnapToGraphemeBoundary_emptyString() {
        XCTAssertEqual(
            PositionBridge.snapToGraphemeBoundary(0, in: ""), 0,
            "Empty string should always return 0"
        )
        XCTAssertEqual(
            PositionBridge.snapToGraphemeBoundary(5, in: ""), 0,
            "Empty string with out-of-range offset should return 0"
        )
    }

    /// Snapping at the end of string returns the end offset.
    func testSnapToGraphemeBoundary_atEnd() {
        let text = "Hello"
        XCTAssertEqual(
            PositionBridge.snapToGraphemeBoundary(5, in: text), 5,
            "At end of string should return end offset"
        )
    }

    /// Snapping with negative offset clamps to 0.
    func testSnapToGraphemeBoundary_negativeOffset() {
        let text = "Hello"
        XCTAssertEqual(
            PositionBridge.snapToGraphemeBoundary(-1, in: text), 0,
            "Negative offset should clamp to 0"
        )
    }

    // MARK: - Flag Emoji (Regional Indicators)

    /// Flag emoji are composed of two regional indicator symbols.
    /// e.g. 🇺🇸 = U+1F1FA U+1F1F8 = 4 UTF-16 code units, 2 scalars, 1 grapheme.
    func testUtf16ToScalar_flagEmoji() {
        let text = "A\u{1F1FA}\u{1F1F8}B"
        // UTF-16: A(1) + U+1F1FA(2) + U+1F1F8(2) + B(1) = 6
        // Scalars: A(1) + U+1F1FA(1) + U+1F1F8(1) + B(1) = 4

        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(1, in: text), 1,
            "After 'A'"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(5, in: text), 3,
            "After flag emoji (UTF-16 offset 5 = scalar 3)"
        )
        XCTAssertEqual(
            PositionBridge.utf16OffsetToScalar(6, in: text), 4,
            "After 'B'"
        )
    }

    func testSnapToGraphemeBoundary_flagEmoji() {
        let text = "\u{1F1FA}\u{1F1F8}"
        // 4 UTF-16 code units, 1 grapheme cluster.
        // Offsets 1, 2, 3 should snap to 4.
        for midOffset in 1...3 {
            let snapped = PositionBridge.snapToGraphemeBoundary(midOffset, in: text)
            XCTAssertEqual(
                snapped, 4,
                "Mid-flag-emoji offset \(midOffset) should snap to 4"
            )
        }
    }
}
