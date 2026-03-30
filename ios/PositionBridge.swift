import UIKit

// MARK: - PositionBridge

/// Converts between UITextView cursor positions (UTF-16 code unit offsets, snapped
/// to grapheme cluster boundaries) and Rust editor-core scalar offsets (Unicode
/// scalar values = Unicode code points).
///
/// UIKit's text system uses UTF-16 internally (NSString). Emoji like U+1F468
/// (man) occupy 2 UTF-16 code units (a surrogate pair) but 1 Unicode scalar.
/// Composed emoji sequences like 👨‍👩‍👧‍👦 are multiple scalars joined by
/// ZWJ but render as a single grapheme cluster.
///
/// Rust's editor-core counts positions in Unicode scalars (what Rust calls `char`).
/// The PositionMap in Rust converts between doc positions and scalar offsets.
/// This bridge converts between those scalar offsets and UITextView UTF-16 offsets.
final class PositionBridge {

    struct VirtualListMarker {
        let paragraphStartUtf16: Int
        let scalarLength: UInt32
    }

    // MARK: - UTF-16 <-> Scalar Conversion

    /// Convert a UITextView cursor position (UTF-16 offset) to a Rust scalar offset.
    ///
    /// Walks the string from the beginning, counting Unicode scalars consumed as
    /// we advance through UTF-16 code units. Surrogate pairs (code units > U+FFFF)
    /// contribute 2 UTF-16 code units but only 1 scalar.
    ///
    /// - Parameters:
    ///   - position: A `UITextPosition` obtained from the text view.
    ///   - textView: The text view containing the text.
    /// - Returns: The equivalent Unicode scalar offset.
    static func textViewToScalar(_ position: UITextPosition, in textView: UITextView) -> UInt32 {
        let utf16Offset = textView.offset(from: textView.beginningOfDocument, to: position)
        return utf16OffsetToScalar(utf16Offset, in: textView)
    }

    /// Convert a Rust scalar offset to a UITextView position.
    ///
    /// Walks the string counting scalars until we reach the target, then returns
    /// the corresponding UTF-16 offset as a UITextPosition.
    ///
    /// - Parameters:
    ///   - scalar: The Unicode scalar offset from Rust.
    ///   - textView: The text view containing the text.
    /// - Returns: The equivalent `UITextPosition`, or the end of document if the
    ///   scalar offset exceeds the text length.
    static func scalarToTextView(_ scalar: UInt32, in textView: UITextView) -> UITextPosition {
        let utf16Offset = scalarToUtf16Offset(scalar, in: textView)
        return textView.position(
            from: textView.beginningOfDocument,
            offset: utf16Offset
        ) ?? textView.endOfDocument
    }

    static func utf16OffsetToScalar(_ utf16Offset: Int, in textView: UITextView) -> UInt32 {
        let text = textView.text ?? ""
        let nsString = text as NSString
        let clampedOffset = min(max(utf16Offset, 0), nsString.length)
        var scalarOffset = utf16OffsetToScalar(clampedOffset, in: text)

        for marker in virtualListMarkers(in: textView) where clampedOffset >= marker.paragraphStartUtf16 {
            scalarOffset += marker.scalarLength
        }

        return scalarOffset
    }

    static func scalarToUtf16Offset(_ scalar: UInt32, in textView: UITextView) -> Int {
        let text = textView.text ?? ""
        let maxUtf16 = (text as NSString).length

        if scalar == 0 || maxUtf16 == 0 {
            return 0
        }

        var low = 0
        var high = maxUtf16
        while low < high {
            let mid = (low + high) / 2
            if utf16OffsetToScalar(mid, in: textView) < scalar {
                low = mid + 1
            } else {
                high = mid
            }
        }

        return low
    }

    /// Convert a UTF-16 offset to a Unicode scalar offset within a string.
    ///
    /// This is the core conversion used by `textViewToScalar`. Exposed as a
    /// static method for direct use and testing.
    ///
    /// - Parameters:
    ///   - utf16Offset: The UTF-16 code unit offset.
    ///   - text: The string to walk.
    /// - Returns: The number of Unicode scalars from the start to the given UTF-16 offset.
    static func utf16OffsetToScalar(_ utf16Offset: Int, in text: String) -> UInt32 {
        guard utf16Offset > 0 else { return 0 }

        let utf16View = text.utf16
        let endIndex = min(utf16Offset, utf16View.count)
        var scalarCount: UInt32 = 0
        var utf16Pos = 0

        for scalar in text.unicodeScalars {
            if utf16Pos >= endIndex { break }
            let scalarUtf16Len = scalar.utf16.count
            utf16Pos += scalarUtf16Len
            scalarCount += 1
        }

        return scalarCount
    }

    /// Convert a Unicode scalar offset to a UTF-16 offset within a string.
    ///
    /// This is the core conversion used by `scalarToTextView`. Exposed as a
    /// static method for direct use and testing.
    ///
    /// - Parameters:
    ///   - scalar: The Unicode scalar offset.
    ///   - text: The string to walk.
    /// - Returns: The number of UTF-16 code units from the start to the given scalar offset.
    static func scalarToUtf16Offset(_ scalar: UInt32, in text: String) -> Int {
        guard scalar > 0 else { return 0 }

        var utf16Len = 0
        var scalarsSeen: UInt32 = 0

        for s in text.unicodeScalars {
            if scalarsSeen >= scalar { break }
            utf16Len += s.utf16.count
            scalarsSeen += 1
        }

        return utf16Len
    }

    // MARK: - Grapheme Boundary Snapping

    /// Snap a UTF-16 offset to the nearest grapheme cluster boundary.
    ///
    /// UITextView may report offsets in the middle of a grapheme cluster (e.g.
    /// between the scalars of a flag emoji or a composed character sequence).
    /// This method snaps the offset forward to the end of the current grapheme
    /// cluster, since that is the position the user would perceive.
    ///
    /// - Parameters:
    ///   - utf16Offset: A UTF-16 code unit offset that may be mid-grapheme.
    ///   - text: The string to inspect.
    /// - Returns: The nearest grapheme-aligned UTF-16 offset. If the input is
    ///   already on a boundary, it is returned unchanged.
    static func snapToGraphemeBoundary(_ utf16Offset: Int, in text: String) -> Int {
        guard !text.isEmpty else { return 0 }

        let nsString = text as NSString
        let clampedOffset = min(max(utf16Offset, 0), nsString.length)

        // If we're at the very start or end, already on a boundary.
        if clampedOffset == 0 || clampedOffset == nsString.length {
            return clampedOffset
        }

        // composedCharacterSequence(at:) returns the full grapheme cluster range
        // containing the given UTF-16 index. We snap to the end of that range
        // (forward bias) since that's what a user moving the cursor expects.
        let range = nsString.rangeOfComposedCharacterSequence(at: clampedOffset)

        // If the offset is already at the start of a grapheme cluster, it's on a boundary.
        if range.location == clampedOffset {
            return clampedOffset
        }

        // Otherwise, snap to the end of this cluster.
        return NSMaxRange(range)
    }

    // MARK: - UITextRange <-> Scalar Range

    /// Convert a UITextRange to a (from, to) pair of Rust scalar offsets.
    ///
    /// - Parameters:
    ///   - range: A `UITextRange` from the text view.
    ///   - textView: The text view containing the text.
    /// - Returns: A tuple of (from, to) scalar offsets where from <= to.
    static func textRangeToScalarRange(
        _ range: UITextRange,
        in textView: UITextView
    ) -> (from: UInt32, to: UInt32) {
        let from = textViewToScalar(range.start, in: textView)
        let to = textViewToScalar(range.end, in: textView)
        return (from: min(from, to), to: max(from, to))
    }

    /// Convert a pair of Rust scalar offsets to a UITextRange.
    ///
    /// - Parameters:
    ///   - from: The start scalar offset.
    ///   - to: The end scalar offset.
    ///   - textView: The text view.
    /// - Returns: The corresponding `UITextRange`, or nil if the positions are invalid.
    static func scalarRangeToTextRange(
        from: UInt32,
        to: UInt32,
        in textView: UITextView
    ) -> UITextRange? {
        let startPos = scalarToTextView(from, in: textView)
        let endPos = scalarToTextView(to, in: textView)
        return textView.textRange(from: startPos, to: endPos)
    }

    // MARK: - Cursor Scalar Offset (Convenience)

    /// Get the current cursor position as a Rust scalar offset.
    ///
    /// If there is a range selection, returns the head (moving end) position.
    ///
    /// - Parameter textView: The text view.
    /// - Returns: The scalar offset of the cursor, or 0 if no selection exists.
    static func cursorScalarOffset(in textView: UITextView) -> UInt32 {
        guard let selectedRange = textView.selectedTextRange else { return 0 }
        return textViewToScalar(selectedRange.end, in: textView)
    }

    static func virtualListMarker(
        atUtf16Offset utf16Offset: Int,
        in textView: UITextView
    ) -> VirtualListMarker? {
        virtualListMarkers(in: textView).first { $0.paragraphStartUtf16 == utf16Offset }
    }

    private static func virtualListMarkers(in textView: UITextView) -> [VirtualListMarker] {
        let textStorage = textView.textStorage
        guard textStorage.length > 0 else { return [] }

        let nsString = textStorage.string as NSString
        var markers: [VirtualListMarker] = []
        var seenStarts = Set<Int>()
        let fullRange = NSRange(location: 0, length: textStorage.length)

        textStorage.enumerateAttribute(
            RenderBridgeAttributes.listMarkerContext,
            in: fullRange,
            options: []
        ) { value, range, _ in
            guard range.length > 0, let listContext = value as? [String: Any] else { return }

            let paragraphStart = nsString.paragraphRange(
                for: NSRange(location: range.location, length: 0)
            ).location
            guard !EditorLayoutManager.isParagraphStartCreatedByHardBreak(
                paragraphStart,
                in: textStorage
            ) else {
                return
            }
            guard seenStarts.insert(paragraphStart).inserted else { return }

            let markerLength = UInt32(
                RenderBridge.listMarkerString(listContext: listContext).unicodeScalars.count
            )
            markers.append(
                VirtualListMarker(
                    paragraphStartUtf16: paragraphStart,
                    scalarLength: markerLength
                )
            )
        }

        return markers.sorted { $0.paragraphStartUtf16 < $1.paragraphStartUtf16 }
    }
}
