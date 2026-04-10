import UIKit
import ObjectiveC

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

    private struct StringConversionTable {
        let utf16ToScalar: [UInt32]
        let scalarToUtf16: [Int]
    }

    private final class TextViewConversionTable: NSObject {
        let adjustedUtf16ToScalar: [UInt32]

        init(adjustedUtf16ToScalar: [UInt32]) {
            self.adjustedUtf16ToScalar = adjustedUtf16ToScalar
        }
    }

    struct VirtualListMarker {
        let paragraphStartUtf16: Int
        let scalarLength: UInt32
    }

    private struct PositionAdjustments {
        let placeholders: [Int]
        let listMarkers: [VirtualListMarker]
    }

    private static var textViewConversionTableKey: UInt8 = 0
    private static let stringTableLock = NSLock()
    private static var lastStringTableText = ""
    private static var lastStringTable: StringConversionTable?

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
        let clampedOffset = min(max(utf16Offset, 0), (text as NSString).length)
        let conversionTable = textViewConversionTable(for: textView)
        return conversionTable.adjustedUtf16ToScalar[clampedOffset]
    }

    static func scalarToUtf16Offset(_ scalar: UInt32, in textView: UITextView) -> Int {
        let conversionTable = textViewConversionTable(for: textView)
        let utf16ToScalar = conversionTable.adjustedUtf16ToScalar
        guard scalar > 0, !utf16ToScalar.isEmpty else {
            return 0
        }

        if let last = utf16ToScalar.last, scalar > last {
            return utf16ToScalar.count - 1
        }

        var low = 0
        var high = utf16ToScalar.count - 1
        while low < high {
            let mid = (low + high) / 2
            if utf16ToScalar[mid] < scalar {
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
        let conversionTable = stringConversionTable(for: text)
        let clampedOffset = min(max(utf16Offset, 0), conversionTable.utf16ToScalar.count - 1)
        return conversionTable.utf16ToScalar[clampedOffset]
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
        let conversionTable = stringConversionTable(for: text)
        guard scalar > 0 else { return 0 }
        let scalarIndex = min(Int(scalar), conversionTable.scalarToUtf16.count - 1)
        return conversionTable.scalarToUtf16[scalarIndex]
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
        virtualListMarkers(in: textView.textStorage).first { $0.paragraphStartUtf16 == utf16Offset }
    }

    static func invalidateCache(for textView: UITextView) {
        objc_setAssociatedObject(
            textView,
            &textViewConversionTableKey,
            nil,
            .OBJC_ASSOCIATION_RETAIN_NONATOMIC
        )
    }

    @discardableResult
    static func applyAttributedPatchIfPossible(
        for textView: UITextView,
        replaceRange: NSRange,
        replacement: NSAttributedString
    ) -> Bool {
        guard let cached = objc_getAssociatedObject(textView, &textViewConversionTableKey) as? TextViewConversionTable else {
            return false
        }

        let oldAdjusted = cached.adjustedUtf16ToScalar
        let oldUtf16Count = max(0, oldAdjusted.count - 1)
        guard replaceRange.location >= 0,
              replaceRange.length >= 0,
              replaceRange.location + replaceRange.length <= oldUtf16Count
        else {
            return false
        }

        let startOffset = replaceRange.location
        let endOffset = replaceRange.location + replaceRange.length
        let replacementAdjusted = adjustedConversionTable(for: replacement)
        let patched = patchedAdjustedConversionTable(
            oldAdjusted: oldAdjusted,
            startOffset: startOffset,
            endOffset: endOffset,
            replacementAdjusted: replacementAdjusted
        )

        objc_setAssociatedObject(
            textView,
            &textViewConversionTableKey,
            TextViewConversionTable(adjustedUtf16ToScalar: patched),
            .OBJC_ASSOCIATION_RETAIN_NONATOMIC
        )
        return true
    }

    @discardableResult
    static func applyPlainTextPatchIfPossible(
        for textView: UITextView,
        replaceRange: NSRange,
        replacementText: String
    ) -> Bool {
        guard let cached = objc_getAssociatedObject(textView, &textViewConversionTableKey) as? TextViewConversionTable else {
            return false
        }

        let oldAdjusted = cached.adjustedUtf16ToScalar
        let oldUtf16Count = max(0, oldAdjusted.count - 1)
        guard replaceRange.location >= 0,
              replaceRange.length >= 0,
              replaceRange.location + replaceRange.length <= oldUtf16Count
        else {
            return false
        }

        let startOffset = replaceRange.location
        let endOffset = replaceRange.location + replaceRange.length
        let replacementBase = stringConversionTable(for: replacementText).utf16ToScalar
        let patched = patchedAdjustedConversionTable(
            oldAdjusted: oldAdjusted,
            startOffset: startOffset,
            endOffset: endOffset,
            replacementAdjusted: replacementBase
        )

        objc_setAssociatedObject(
            textView,
            &textViewConversionTableKey,
            TextViewConversionTable(adjustedUtf16ToScalar: patched),
            .OBJC_ASSOCIATION_RETAIN_NONATOMIC
        )
        return true
    }

    private static func patchedAdjustedConversionTable(
        oldAdjusted: [UInt32],
        startOffset: Int,
        endOffset: Int,
        replacementAdjusted: [UInt32]
    ) -> [UInt32] {
        let startScalar = Int32(oldAdjusted[startOffset])
        let deletedScalarCount = Int32(oldAdjusted[endOffset]) - startScalar
        let replacementScalarCount = Int32(replacementAdjusted.last ?? 0)
        let scalarDelta = replacementScalarCount - deletedScalarCount
        let replacement = replacementAdjusted.map { value in
            UInt32(max(0, Int32(value) + startScalar))
        }
        let prefix = Array(oldAdjusted[..<startOffset])
        let suffix = oldAdjusted[(endOffset + 1)...].map { value in
            UInt32(max(0, Int32(value) + scalarDelta))
        }
        return prefix + replacement + suffix
    }

    private static func stringConversionTable(for text: String) -> StringConversionTable {
        stringTableLock.lock()
        if lastStringTableText == text, let lastStringTable {
            stringTableLock.unlock()
            return lastStringTable
        }
        stringTableLock.unlock()

        let utf16Count = text.utf16.count
        let scalarCount = text.unicodeScalars.count
        var utf16ToScalar = Array(repeating: UInt32(0), count: utf16Count + 1)
        var scalarToUtf16 = Array(repeating: 0, count: scalarCount + 1)
        var utf16Pos = 0
        var scalarPos = 0

        for scalar in text.unicodeScalars {
            let nextUtf16Pos = utf16Pos + scalar.utf16.count
            scalarPos += 1
            if nextUtf16Pos > utf16Pos {
                for offset in (utf16Pos + 1)...nextUtf16Pos {
                    utf16ToScalar[offset] = UInt32(scalarPos)
                }
            }
            scalarToUtf16[scalarPos] = nextUtf16Pos
            utf16Pos = nextUtf16Pos
        }

        let conversionTable = StringConversionTable(
            utf16ToScalar: utf16ToScalar,
            scalarToUtf16: scalarToUtf16
        )

        stringTableLock.lock()
        lastStringTableText = text
        lastStringTable = conversionTable
        stringTableLock.unlock()

        return conversionTable
    }

    private static func adjustedConversionTable(for attributedString: NSAttributedString) -> [UInt32] {
        let baseTable = stringConversionTable(for: attributedString.string)
        let adjustments = positionAdjustments(in: attributedString)
        return adjustedUtf16ToScalar(
            baseUtf16ToScalar: baseTable.utf16ToScalar,
            placeholders: adjustments.placeholders,
            listMarkers: adjustments.listMarkers
        )
    }

    private static func textViewConversionTable(for textView: UITextView) -> TextViewConversionTable {
        if let cached = objc_getAssociatedObject(textView, &textViewConversionTableKey) as? TextViewConversionTable {
            return cached
        }

        let text = textView.text ?? ""
        let baseTable = stringConversionTable(for: text)
        let adjustments = positionAdjustments(in: textView.textStorage)
        let adjustedUtf16ToScalar = adjustedUtf16ToScalar(
            baseUtf16ToScalar: baseTable.utf16ToScalar,
            placeholders: adjustments.placeholders,
            listMarkers: adjustments.listMarkers
        )
        let conversionTable = TextViewConversionTable(adjustedUtf16ToScalar: adjustedUtf16ToScalar)
        objc_setAssociatedObject(
            textView,
            &textViewConversionTableKey,
            conversionTable,
            .OBJC_ASSOCIATION_RETAIN_NONATOMIC
        )
        return conversionTable
    }

    private static func adjustedUtf16ToScalar(
        baseUtf16ToScalar: [UInt32],
        placeholders: [Int],
        listMarkers: [VirtualListMarker] = [],
    ) -> [UInt32] {
        let utf16Count = max(0, baseUtf16ToScalar.count - 1)
        var deltas = Array(repeating: Int32(0), count: utf16Count + 2)

        for placeholderOffset in placeholders {
            let startOffset = min(max(placeholderOffset + 1, 0), utf16Count + 1)
            if startOffset <= utf16Count {
                deltas[startOffset] -= 1
            }
        }

        for marker in listMarkers {
            let startOffset = min(max(marker.paragraphStartUtf16, 0), utf16Count)
            deltas[startOffset] += Int32(marker.scalarLength)
        }

        var adjustedUtf16ToScalar = Array(repeating: UInt32(0), count: utf16Count + 1)
        var runningDelta: Int32 = 0
        for offset in 0...utf16Count {
            runningDelta += deltas[offset]
            let adjustedValue = Int32(baseUtf16ToScalar[offset]) + runningDelta
            adjustedUtf16ToScalar[offset] = UInt32(max(0, adjustedValue))
        }
        return adjustedUtf16ToScalar
    }

    private static func adjustedUtf16ToScalar(
        baseUtf16ToScalar: [UInt32],
        listMarkers: [VirtualListMarker]
    ) -> [UInt32] {
        adjustedUtf16ToScalar(baseUtf16ToScalar: baseUtf16ToScalar, placeholders: [], listMarkers: listMarkers)
    }

    private static func virtualListMarkers(in attributedString: NSAttributedString) -> [VirtualListMarker] {
        positionAdjustments(in: attributedString).listMarkers
    }

    private static func positionAdjustments(in attributedString: NSAttributedString) -> PositionAdjustments {
        guard attributedString.length > 0 else {
            return PositionAdjustments(placeholders: [], listMarkers: [])
        }

        let nsString = attributedString.string as NSString
        var placeholders: [Int] = []
        var markers: [VirtualListMarker] = []
        var seenStarts = Set<Int>()
        let fullRange = NSRange(location: 0, length: attributedString.length)

        attributedString.enumerateAttributes(
            in: fullRange,
            options: [.longestEffectiveRangeNotRequired]
        ) { attrs, range, _ in
            guard range.length > 0 else { return }

            if attrs[RenderBridgeAttributes.syntheticPlaceholder] as? Bool == true {
                placeholders.append(range.location)
            }

            guard let listContext = attrs[RenderBridgeAttributes.listMarkerContext] as? [String: Any] else {
                return
            }

            let paragraphStart = nsString.paragraphRange(
                for: NSRange(location: range.location, length: 0)
            ).location
            guard !isParagraphStartCreatedByHardBreak(
                paragraphStart,
                in: attributedString
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

        return PositionAdjustments(
            placeholders: placeholders,
            listMarkers: markers.sorted { $0.paragraphStartUtf16 < $1.paragraphStartUtf16 }
        )
    }

    private static func virtualListMarkers(in textStorage: NSTextStorage) -> [VirtualListMarker] {
        virtualListMarkers(in: textStorage as NSAttributedString)
    }

    private static func syntheticPlaceholderOffsets(in attributedString: NSAttributedString) -> [Int] {
        positionAdjustments(in: attributedString).placeholders
    }

    private static func syntheticPlaceholderOffsets(in textStorage: NSTextStorage) -> [Int] {
        syntheticPlaceholderOffsets(in: textStorage as NSAttributedString)
    }

    private static func isParagraphStartCreatedByHardBreak(
        _ paragraphStart: Int,
        in attributedString: NSAttributedString
    ) -> Bool {
        guard paragraphStart > 0, paragraphStart <= attributedString.length else { return false }
        let previousVoidType = attributedString.attribute(
            RenderBridgeAttributes.voidNodeType,
            at: paragraphStart - 1,
            effectiveRange: nil
        ) as? String
        return previousVoidType == "hardBreak"
    }
}
