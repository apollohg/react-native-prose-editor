package com.apollohg.editor

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Unit tests for [PositionBridge] — UTF-16 <-> Unicode scalar offset conversion
 * and grapheme cluster boundary snapping.
 *
 * These tests are pure JVM (no Android framework dependencies) and mirror
 * the iOS PositionBridgeTests.swift test suite.
 *
 * Test coverage:
 * - ASCII text (1 UTF-16 = 1 scalar)
 * - BMP characters (accented chars, CJK)
 * - Surrogate pairs (emoji above U+FFFF)
 * - ZWJ sequences (family emoji)
 * - Regional indicator pairs (flag emoji)
 * - Mixed content
 * - Empty string edge cases
 * - Grapheme boundary snapping
 * - Roundtrip consistency
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PositionBridgeTest {

    // ── UTF-16 -> Scalar: ASCII ─────────────────────────────────────────

    /** ASCII characters are 1 UTF-16 code unit = 1 scalar each. */
    @Test
    fun `utf16ToScalar - ASCII only`() {
        val text = "Hello"
        // "Hello" = 5 UTF-16 code units = 5 scalars
        assertEquals(
            "Offset 0 should map to scalar 0",
            0, PositionBridge.utf16ToScalar(0, text)
        )
        assertEquals(
            "Offset 1 in ASCII should map to scalar 1",
            1, PositionBridge.utf16ToScalar(1, text)
        )
        assertEquals(
            "Offset 5 (end of 'Hello') should map to scalar 5",
            5, PositionBridge.utf16ToScalar(5, text)
        )
    }

    /** Empty string edge case. */
    @Test
    fun `utf16ToScalar - empty string`() {
        assertEquals(
            "Empty string at offset 0 should return scalar 0",
            0, PositionBridge.utf16ToScalar(0, "")
        )
    }

    // ── UTF-16 -> Scalar: BMP Characters ────────────────────────────────

    /** BMP characters (e.g. U+00E9, e-acute) are 1 UTF-16 code unit, 1 scalar. */
    @Test
    fun `utf16ToScalar - BMP characters`() {
        val text = "caf\u00E9"  // "cafe" with accent
        assertEquals(
            "BMP character: UTF-16 offset 4 should be scalar 4",
            4, PositionBridge.utf16ToScalar(4, text)
        )
    }

    // ── UTF-16 -> Scalar: Surrogate Pairs ───────────────────────────────

    /**
     * Characters above U+FFFF (supplementary plane) are encoded as
     * surrogate pairs in UTF-16: 2 code units per scalar.
     */
    @Test
    fun `utf16ToScalar - surrogate pair`() {
        // U+1F600 (grinning face) = 2 UTF-16 code units, 1 scalar
        val text = "A\uD83D\uDE00B"  // "A" + U+1F600 + "B"
        // UTF-16: A(1) + U+1F600(2) + B(1) = 4 code units
        // Scalars: A(1) + U+1F600(1) + B(1) = 3 scalars

        assertEquals("Before 'A'", 0, PositionBridge.utf16ToScalar(0, text))
        assertEquals("After 'A', before emoji", 1, PositionBridge.utf16ToScalar(1, text))
        assertEquals(
            "After emoji (UTF-16 offset 3 = scalar 2)",
            2, PositionBridge.utf16ToScalar(3, text)
        )
        assertEquals(
            "After 'B' (end of string)",
            3, PositionBridge.utf16ToScalar(4, text)
        )
    }

    /**
     * The family emoji is composed of multiple scalars joined by ZWJ (U+200D).
     * U+1F468 U+200D U+1F469 U+200D U+1F467 U+200D U+1F466
     * = 7 scalars, 11 UTF-16 code units (4 surrogates + 3 ZWJ)
     * But it renders as 1 grapheme cluster.
     */
    @Test
    fun `utf16ToScalar - family emoji (ZWJ sequence)`() {
        val text = "\uD83D\uDC68\u200D\uD83D\uDC69\u200D\uD83D\uDC67\u200D\uD83D\uDC66"
        val expectedUtf16Count = 11
        val expectedScalarCount = 7

        assertEquals(
            "Family emoji should be $expectedUtf16Count UTF-16 code units",
            expectedUtf16Count, text.length
        )
        assertEquals(
            "Family emoji should be $expectedScalarCount Unicode scalars",
            expectedScalarCount, text.codePointCount(0, text.length)
        )
        assertEquals(
            "Full family emoji: UTF-16 end should map to scalar end",
            expectedScalarCount,
            PositionBridge.utf16ToScalar(expectedUtf16Count, text)
        )
    }

    // ── UTF-16 -> Scalar: CJK ──────────────────────────────────────────

    /** CJK characters in the BMP: 1 UTF-16 = 1 scalar. */
    @Test
    fun `utf16ToScalar - CJK`() {
        val text = "\u4F60\u597D"  // nihao
        assertEquals(
            "CJK character at offset 1 should be scalar 1",
            1, PositionBridge.utf16ToScalar(1, text)
        )
        assertEquals(
            "End of CJK string should be scalar 2",
            2, PositionBridge.utf16ToScalar(2, text)
        )
    }

    /** CJK extension B characters (U+20000+) are in the supplementary plane. */
    @Test
    fun `utf16ToScalar - CJK Extension B`() {
        // U+20000 (CJK Unified Ideographs Extension B) = 2 UTF-16 code units, 1 scalar
        val text = "A\uD840\uDC00B"  // A + U+20000 + B
        // UTF-16: A(1) + U+20000(2) + B(1) = 4
        // Scalars: A(1) + U+20000(1) + B(1) = 3
        assertEquals(
            "After CJK Extension B character",
            2, PositionBridge.utf16ToScalar(3, text)
        )
    }

    // ── UTF-16 -> Scalar: Mixed Content ─────────────────────────────────

    /** Mixed ASCII, emoji, and CJK in one string. */
    @Test
    fun `utf16ToScalar - mixed content`() {
        val text = "Hi\uD83D\uDE00\u4F60\u597D!"
        // UTF-16: H(1) + i(1) + U+1F600(2) + U+4F60(1) + U+597D(1) + !(1) = 7
        // Scalars: H(1) + i(1) + U+1F600(1) + U+4F60(1) + U+597D(1) + !(1) = 6

        assertEquals("After 'Hi'", 2, PositionBridge.utf16ToScalar(2, text))
        assertEquals(
            "After emoji (UTF-16 offset 4 = scalar 3)",
            3, PositionBridge.utf16ToScalar(4, text)
        )
        assertEquals("End of mixed string", 6, PositionBridge.utf16ToScalar(7, text))
    }

    // ── UTF-16 -> Scalar: Flag Emoji ────────────────────────────────────

    /**
     * Flag emoji are composed of two regional indicator symbols.
     * e.g. US flag = U+1F1FA U+1F1F8 = 4 UTF-16 code units, 2 scalars, 1 grapheme.
     */
    @Test
    fun `utf16ToScalar - flag emoji`() {
        val text = "A\uD83C\uDDFA\uD83C\uDDF8B"  // A + U+1F1FA + U+1F1F8 + B
        // UTF-16: A(1) + U+1F1FA(2) + U+1F1F8(2) + B(1) = 6
        // Scalars: A(1) + U+1F1FA(1) + U+1F1F8(1) + B(1) = 4

        assertEquals("After 'A'", 1, PositionBridge.utf16ToScalar(1, text))
        assertEquals(
            "After flag emoji (UTF-16 offset 5 = scalar 3)",
            3, PositionBridge.utf16ToScalar(5, text)
        )
        assertEquals("After 'B'", 4, PositionBridge.utf16ToScalar(6, text))
    }

    // ── Scalar -> UTF-16: ASCII ─────────────────────────────────────────

    @Test
    fun `scalarToUtf16 - ASCII only`() {
        val text = "Hello"
        assertEquals(
            "Scalar 0 should map to UTF-16 offset 0",
            0, PositionBridge.scalarToUtf16(0, text)
        )
        assertEquals(
            "Scalar 3 in ASCII should map to UTF-16 offset 3",
            3, PositionBridge.scalarToUtf16(3, text)
        )
        assertEquals(
            "Scalar 5 (end) should map to UTF-16 offset 5",
            5, PositionBridge.scalarToUtf16(5, text)
        )
    }

    // ── Scalar -> UTF-16: Surrogate Pairs ───────────────────────────────

    @Test
    fun `scalarToUtf16 - surrogate pair`() {
        val text = "A\uD83D\uDE00B"  // A + U+1F600 + B
        // Scalar 0 = A -> UTF-16 offset 0
        // Scalar 1 = U+1F600 -> UTF-16 offset 1
        // Scalar 2 = B -> UTF-16 offset 3 (after 2 UTF-16 code units for emoji)
        assertEquals(
            "Scalar 1 (emoji start) should be UTF-16 offset 1",
            1, PositionBridge.scalarToUtf16(1, text)
        )
        assertEquals(
            "Scalar 2 (after emoji) should be UTF-16 offset 3",
            3, PositionBridge.scalarToUtf16(2, text)
        )
        assertEquals(
            "Scalar 3 (after B) should be UTF-16 offset 4",
            4, PositionBridge.scalarToUtf16(3, text)
        )
    }

    // ── Scalar -> UTF-16: Family Emoji ──────────────────────────────────

    @Test
    fun `scalarToUtf16 - family emoji`() {
        val text = "\uD83D\uDC68\u200D\uD83D\uDC69\u200D\uD83D\uDC67\u200D\uD83D\uDC66"
        // 7 scalars, 11 UTF-16 code units
        assertEquals(
            "End of family emoji: scalar 7 -> UTF-16 offset 11",
            11, PositionBridge.scalarToUtf16(7, text)
        )
    }

    // ── Roundtrip Consistency ───────────────────────────────────────────

    /** Verify that scalar->utf16->scalar roundtrips exactly. */
    @Test
    fun `roundtrip - scalar to utf16 and back`() {
        val testCases = listOf(
            "Hello, world!" to "ASCII",
            "A\uD83D\uDE00B" to "surrogate pair",
            "\u4F60\u597D" to "CJK",
            "abc\uD83D\uDE00\u4F60xyz" to "mixed"
        )

        for ((text, label) in testCases) {
            val scalarCount = text.codePointCount(0, text.length)
            for (scalarOffset in 0..scalarCount) {
                val utf16 = PositionBridge.scalarToUtf16(scalarOffset, text)
                val backToScalar = PositionBridge.utf16ToScalar(utf16, text)
                assertEquals(
                    "Roundtrip for '$label' at scalar $scalarOffset: " +
                            "utf16=$utf16, back=$backToScalar - should equal original scalar",
                    scalarOffset, backToScalar
                )
            }
        }
    }

    /**
     * Verify that utf16->scalar->utf16 roundtrips correctly for various inputs.
     * Note: mid-surrogate offsets may snap forward, so we check >= originalOffset - 1.
     */
    @Test
    fun `roundtrip - utf16 to scalar and back`() {
        val testCases = listOf(
            "Hello, world!" to "ASCII",
            "A\uD83D\uDE00B" to "surrogate pair",
            "\uD83D\uDC68\u200D\uD83D\uDC69\u200D\uD83D\uDC67\u200D\uD83D\uDC66" to "family emoji",
            "\u4F60\u597D\u4E16\u754C" to "CJK",
            "abc\uD83D\uDE00\u4F60xyz" to "mixed",
            "A\u0301" to "combining character (A + combining acute)"
        )

        for ((text, label) in testCases) {
            for (utf16Offset in 0..text.length) {
                val scalar = PositionBridge.utf16ToScalar(utf16Offset, text)
                val backToUtf16 = PositionBridge.scalarToUtf16(scalar, text)
                assertTrue(
                    "Roundtrip for '$label' at UTF-16 offset $utf16Offset: " +
                            "scalar=$scalar, back=$backToUtf16",
                    backToUtf16 >= utf16Offset - 1
                )
            }
        }
    }

    // ── Grapheme Boundary Snapping ──────────────────────────────────────

    /** Snapping at an already-valid boundary returns the same offset. */
    @Test
    fun `snapToGraphemeBoundary - already on boundary (ASCII)`() {
        val text = "Hello"
        for (i in 0..5) {
            assertEquals(
                "ASCII offset $i is already on a grapheme boundary",
                i, PositionBridge.snapToGraphemeBoundary(i, text)
            )
        }
    }

    /** Snapping in the middle of a surrogate pair snaps to the end of the grapheme cluster. */
    @Test
    fun `snapToGraphemeBoundary - mid surrogate pair`() {
        val text = "A\uD83D\uDE00B"
        // UTF-16: A(0), high surrogate(1), low surrogate(2), B(3)
        // Offset 2 is mid-surrogate. Should snap to 3 (end of emoji grapheme).
        val snapped = PositionBridge.snapToGraphemeBoundary(2, text)
        assertEquals(
            "Mid-surrogate offset 2 should snap to 3 (end of emoji grapheme)",
            3, snapped
        )
    }

    /** Snapping in the middle of a family emoji snaps to the end. */
    @Test
    fun `snapToGraphemeBoundary - mid family emoji`() {
        val text = "\uD83D\uDC68\u200D\uD83D\uDC69\u200D\uD83D\uDC67\u200D\uD83D\uDC66"
        // The entire sequence is 1 grapheme cluster = 11 UTF-16 code units.
        // Any offset 1..10 should snap to 11.
        for (midOffset in 1..10) {
            val snapped = PositionBridge.snapToGraphemeBoundary(midOffset, text)
            assertEquals(
                "Mid-family-emoji offset $midOffset should snap to 11 (end of grapheme)",
                11, snapped
            )
        }
    }

    /** Snapping in the middle of a combining character sequence. */
    @Test
    fun `snapToGraphemeBoundary - combining character`() {
        // "e" + combining acute accent = 2 UTF-16 code units but 1 grapheme cluster
        val text = "e\u0301"
        val snapped = PositionBridge.snapToGraphemeBoundary(1, text)
        assertEquals(
            "Between base and combining character should snap to end of grapheme",
            2, snapped
        )
    }

    /** Snapping on empty string always returns 0. */
    @Test
    fun `snapToGraphemeBoundary - empty string`() {
        assertEquals(
            "Empty string should always return 0",
            0, PositionBridge.snapToGraphemeBoundary(0, "")
        )
        assertEquals(
            "Empty string with out-of-range offset should return 0",
            0, PositionBridge.snapToGraphemeBoundary(5, "")
        )
    }

    /** Snapping at the end of string returns the end offset. */
    @Test
    fun `snapToGraphemeBoundary - at end`() {
        val text = "Hello"
        assertEquals(
            "At end of string should return end offset",
            5, PositionBridge.snapToGraphemeBoundary(5, text)
        )
    }

    /** Snapping with negative offset clamps to 0. */
    @Test
    fun `snapToGraphemeBoundary - negative offset`() {
        val text = "Hello"
        assertEquals(
            "Negative offset should clamp to 0",
            0, PositionBridge.snapToGraphemeBoundary(-1, text)
        )
    }

    /** Flag emoji mid-offset snapping. */
    @Test
    fun `snapToGraphemeBoundary - flag emoji`() {
        val text = "\uD83C\uDDFA\uD83C\uDDF8"  // US flag
        // 4 UTF-16 code units, 1 grapheme cluster.
        // Offsets 1, 2, 3 should snap to 4.
        for (midOffset in 1..3) {
            val snapped = PositionBridge.snapToGraphemeBoundary(midOffset, text)
            assertEquals(
                "Mid-flag-emoji offset $midOffset should snap to 4",
                4, snapped
            )
        }
    }

    // ── Edge Cases ──────────────────────────────────────────────────────

    /** Offset beyond string length should be clamped. */
    @Test
    fun `utf16ToScalar - offset beyond string length`() {
        val text = "Hi"
        assertEquals(
            "Offset beyond length should count all scalars",
            2, PositionBridge.utf16ToScalar(10, text)
        )
    }

    /** Scalar beyond code point count should return full UTF-16 length. */
    @Test
    fun `scalarToUtf16 - scalar beyond code point count`() {
        val text = "Hi"
        assertEquals(
            "Scalar beyond count should return full UTF-16 length",
            2, PositionBridge.scalarToUtf16(10, text)
        )
    }

    /** Mid-surrogate UTF-16 offset should still produce a valid scalar count. */
    @Test
    fun `utf16ToScalar - mid surrogate pair offset`() {
        val text = "A\uD83D\uDE00B"
        // Offset 2 is in the middle of the surrogate pair for U+1F600.
        // The method walks code points, so at offset 2 we've consumed
        // A (1 code unit) + U+1F600 (2 code units, but offset 2 is mid-pair).
        // Since we count code points via Character.codePointAt which handles
        // surrogates, offset 2 still "fits" within the second code point's range,
        // so only 1 scalar (A) is fully before offset 2.
        val scalar = PositionBridge.utf16ToScalar(2, text)
        // Should be 1 (only 'A' fully precedes offset 2) or 2 (if the emoji
        // is counted because we started reading it). Implementation walks
        // with codePointAt, charCount — at utf16Pos=1 we read codePoint
        // U+1F600 (charCount=2), advance to utf16Pos=3 which > endIndex=2,
        // but we already incremented scalarCount. So result is 2.
        assertEquals(
            "Mid-surrogate offset 2: implementation counts the emoji scalar",
            2, scalar
        )
    }
}
