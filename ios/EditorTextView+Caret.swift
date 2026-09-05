import UIKit
import os

extension EditorTextView {
    func resolvedCaretReferenceRect(for position: UITextPosition) -> CGRect {
        let directRect = super.caretRect(for: position)
        if let horizontalRuleRect = resolvedHorizontalRuleAdjacentCaretRect(
            for: position,
            directRect: directRect
        ) {
            return horizontalRuleRect
        }
        guard directRect.height <= 0 || directRect.isEmpty else {
            return directRect
        }

        let caretWidth = max(directRect.width, 2)

        if let nextPosition = self.position(from: position, offset: 1),
           let nextRange = textRange(from: position, to: nextPosition),
           let nextRect = selectionRects(for: nextRange)
               .map(\.rect)
               .first(where: { !$0.isEmpty && $0.width > 0 && $0.height > 0 })
        {
            return CGRect(
                x: nextRect.minX,
                y: nextRect.minY,
                width: caretWidth,
                height: max(directRect.height, nextRect.height)
            )
        }

        if let previousPosition = self.position(from: position, offset: -1),
           let previousRange = textRange(from: previousPosition, to: position),
           let previousRect = selectionRects(for: previousRange)
               .map(\.rect)
               .last(where: { !$0.isEmpty && $0.width > 0 && $0.height > 0 })
        {
            return CGRect(
                x: previousRect.maxX,
                y: previousRect.minY,
                width: caretWidth,
                height: max(directRect.height, previousRect.height)
            )
        }

        return directRect
    }

    private func resolvedHorizontalRuleAdjacentCaretRect(
        for position: UITextPosition,
        directRect: CGRect
    ) -> CGRect? {
        guard textStorage.length > 0 else { return nil }

        let utf16Offset = offset(from: beginningOfDocument, to: position)
        let caretWidth = max(directRect.width, 2)

        if isHorizontalRuleAttachment(at: utf16Offset),
           let previousCharacterIndex = nearestVisibleCharacterIndex(
               from: utf16Offset - 1,
               direction: -1
           ),
           let previousRect = visibleSelectionRect(forCharacterAt: previousCharacterIndex)
        {
            return CGRect(
                x: previousRect.maxX,
                y: previousRect.minY,
                width: caretWidth,
                height: max(directRect.height, previousRect.height)
            )
        }

        if isHorizontalRuleAttachment(at: utf16Offset - 1),
           let nextCharacterIndex = nearestVisibleCharacterIndex(
               from: utf16Offset,
               direction: 1
           ),
           let nextRect = visibleSelectionRect(forCharacterAt: nextCharacterIndex)
        {
            return CGRect(
                x: nextRect.minX,
                y: nextRect.minY,
                width: caretWidth,
                height: max(directRect.height, nextRect.height)
            )
        }

        return nil
    }

    static func adjustedCaretRect(
        from rect: CGRect,
        targetHeight: CGFloat,
        screenScale: CGFloat
    ) -> CGRect {
        guard rect.height > 0, targetHeight > 0, targetHeight < rect.height else {
            return rect
        }

        let scale = max(screenScale, 1)
        let alignedHeight = ceil(targetHeight * scale) / scale
        let centeredY = rect.minY + ((rect.height - alignedHeight) / 2.0)
        let alignedY = (centeredY * scale).rounded() / scale

        var adjusted = rect
        adjusted.origin.y = alignedY
        adjusted.size.height = alignedHeight
        return adjusted
    }

    static func adjustedCaretRect(
        from rect: CGRect,
        font: UIFont,
        screenScale: CGFloat
    ) -> CGRect {
        let scale = max(screenScale, 1)
        let lineHeight = max(font.lineHeight, 0)
        let alignedHeight = ceil(lineHeight * scale) / scale
        let alignedY = ((rect.maxY - alignedHeight) * scale).rounded() / scale

        var adjusted = rect
        adjusted.origin.y = alignedY
        adjusted.size.height = alignedHeight
        return adjusted
    }

    static func adjustedCaretRect(
        from rect: CGRect,
        baselineY: CGFloat,
        font: UIFont,
        screenScale: CGFloat
    ) -> CGRect {
        let scale = max(screenScale, 1)
        let lineHeight = max(font.lineHeight, 0)
        let alignedHeight = ceil(lineHeight * scale) / scale
        let typographicHeight = font.ascender - font.descender
        let leading = max(lineHeight - typographicHeight, 0)
        let topY = baselineY - font.ascender - (leading / 2.0)
        let alignedY = (topY * scale).rounded() / scale

        var adjusted = rect
        adjusted.origin.y = alignedY
        adjusted.size.height = alignedHeight
        return adjusted
    }

    func caretBaselineY(for position: UITextPosition, referenceRect: CGRect) -> CGFloat? {
        guard textStorage.length > 0 else { return nil }

        let rawOffset = offset(from: beginningOfDocument, to: position)
        let clampedOffset = min(max(rawOffset, 0), textStorage.length)

        if let horizontalRuleBaselineY = horizontalRuleAdjacentBaselineY(at: clampedOffset) {
            return horizontalRuleBaselineY
        }

        if let hardBreakBaselineY = hardBreakBaselineY(after: clampedOffset) {
            return hardBreakBaselineY
        }

        var candidateCharacters = Set<Int>()

        if clampedOffset < textStorage.length {
            candidateCharacters.insert(clampedOffset)
        }
        if clampedOffset > 0 {
            candidateCharacters.insert(clampedOffset - 1)
        }
        if clampedOffset + 1 < textStorage.length {
            candidateCharacters.insert(clampedOffset + 1)
        }

        guard !candidateCharacters.isEmpty else { return nil }

        let referenceMidY = referenceRect.midY
        let referenceMinY = referenceRect.minY
        var bestMatch: (score: CGFloat, baselineY: CGFloat)?

        for characterIndex in candidateCharacters.sorted() {
            let glyphIndex = layoutManager.glyphIndexForCharacter(at: characterIndex)
            guard glyphIndex < layoutManager.numberOfGlyphs else { continue }

            let lineFragmentRect = layoutManager.lineFragmentRect(
                forGlyphAt: glyphIndex,
                effectiveRange: nil
            )
            let lineRectInView = lineFragmentRect.offsetBy(dx: 0, dy: textContainerInset.top)
            let score = abs(lineRectInView.midY - referenceMidY) * 10
                + abs(lineRectInView.minY - referenceMinY)
            let glyphLocation = layoutManager.location(forGlyphAt: glyphIndex)
            let baselineY = textContainerInset.top + lineFragmentRect.minY + glyphLocation.y

            if let currentBest = bestMatch, currentBest.score <= score {
                continue
            }
            bestMatch = (score, baselineY)
        }

        return bestMatch?.baselineY
    }

    private func horizontalRuleAdjacentBaselineY(at utf16Offset: Int) -> CGFloat? {
        guard textStorage.length > 0 else { return nil }

        if isHorizontalRuleAttachment(at: utf16Offset),
           let previousCharacterIndex = nearestVisibleCharacterIndex(
               from: utf16Offset - 1,
               direction: -1
           )
        {
            return baselineY(forCharacterAt: previousCharacterIndex)
        }

        if isHorizontalRuleAttachment(at: utf16Offset - 1),
           let nextCharacterIndex = nearestVisibleCharacterIndex(
               from: utf16Offset,
               direction: 1
           )
        {
            return baselineY(forCharacterAt: nextCharacterIndex)
        }

        return nil
    }

    func baselineY(forCharacterAt characterIndex: Int) -> CGFloat? {
        guard characterIndex >= 0, characterIndex < textStorage.length else { return nil }

        let glyphIndex = layoutManager.glyphIndexForCharacter(at: characterIndex)
        guard glyphIndex < layoutManager.numberOfGlyphs else { return nil }

        let lineFragmentRect = layoutManager.lineFragmentRect(
            forGlyphAt: glyphIndex,
            effectiveRange: nil
        )
        let glyphLocation = layoutManager.location(forGlyphAt: glyphIndex)
        return textContainerInset.top + lineFragmentRect.minY + glyphLocation.y
    }

    private func visibleSelectionRect(forCharacterAt characterIndex: Int) -> CGRect? {
        guard characterIndex >= 0, characterIndex < textStorage.length else { return nil }
        guard let start = position(from: beginningOfDocument, offset: characterIndex),
              let end = position(from: start, offset: 1),
              let range = textRange(from: start, to: end)
        else {
            return nil
        }

        return selectionRects(for: range)
            .map(\.rect)
            .first(where: { !$0.isEmpty && $0.width > 0 && $0.height > 0 })
    }

    private func nearestVisibleCharacterIndex(from startIndex: Int, direction: Int) -> Int? {
        guard direction == -1 || direction == 1 else { return nil }
        guard textStorage.length > 0 else { return nil }

        let text = textStorage.string as NSString
        var index = startIndex

        while index >= 0, index < text.length {
            let attrs = textStorage.attributes(at: index, effectiveRange: nil)
            let character = text.substring(with: NSRange(location: index, length: 1))

            if attrs[.attachment] == nil,
               character != "\n",
               character != "\r",
               visibleSelectionRect(forCharacterAt: index) != nil
            {
                return index
            }

            index += direction
        }

        return nil
    }

    private func isHorizontalRuleAttachment(at utf16Offset: Int) -> Bool {
        guard utf16Offset >= 0, utf16Offset < textStorage.length else { return false }

        let attrs = textStorage.attributes(at: utf16Offset, effectiveRange: nil)
        return attrs[.attachment] is NSTextAttachment
            && EditorNodeTypes.isHorizontalRule(attrs[RenderBridgeAttributes.voidNodeType] as? String)
    }

    private func hardBreakBaselineY(after utf16Offset: Int) -> CGFloat? {
        guard utf16Offset > 0, utf16Offset <= textStorage.length else { return nil }
        let previousVoidType = textStorage.attribute(
            RenderBridgeAttributes.voidNodeType,
            at: utf16Offset - 1,
            effectiveRange: nil
        ) as? String
        guard EditorNodeTypes.isHardBreak(previousVoidType) else { return nil }

        let previousGlyphIndex = layoutManager.glyphIndexForCharacter(at: utf16Offset - 1)
        guard previousGlyphIndex < layoutManager.numberOfGlyphs else { return nil }

        let lineFragmentRect = layoutManager.lineFragmentRect(
            forGlyphAt: previousGlyphIndex,
            effectiveRange: nil
        )
        let glyphLocation = layoutManager.location(forGlyphAt: previousGlyphIndex)
        let previousBaselineY = textContainerInset.top + lineFragmentRect.minY + glyphLocation.y

        let paragraphStyle = textStorage.attribute(
            .paragraphStyle,
            at: utf16Offset - 1,
            effectiveRange: nil
        ) as? NSParagraphStyle
        let configuredLineHeight = max(
            paragraphStyle?.minimumLineHeight ?? 0,
            paragraphStyle?.maximumLineHeight ?? 0
        )
        let lineAdvance = configuredLineHeight > 0
            ? configuredLineHeight
            : lineFragmentRect.height

        return previousBaselineY + lineAdvance
    }

    func resolvedCaretFont(for position: UITextPosition) -> UIFont {
        guard textStorage.length > 0 else { return resolvedDefaultFont() }

        let offset = offset(from: beginningOfDocument, to: position)
        let attributeIndex: Int
        if offset <= 0 {
            attributeIndex = 0
        } else if offset < textStorage.length {
            attributeIndex = offset
        } else {
            attributeIndex = textStorage.length - 1
        }

        return (textStorage.attribute(.font, at: attributeIndex, effectiveRange: nil) as? UIFont)
            ?? resolvedDefaultFont()
    }

}
