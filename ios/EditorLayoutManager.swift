import UIKit
import CoreText

/// Draws list markers visually in the gutter without inserting them into the
/// editable text storage. This keeps UIKit paragraph-start behaviors, such as
/// sentence auto-capitalization, working naturally inside list items.
final class EditorLayoutManager: NSLayoutManager {
    private(set) var blockquoteStripeDrawPassesForTesting: [[CGRect]] = []

    func blockquoteStripeRectsForTesting(
        in textStorage: NSTextStorage,
        visibleGlyphRange: NSRange? = nil,
        origin: CGPoint = .zero
    ) -> [CGRect] {
        let glyphsToShow = visibleGlyphRange ?? NSRange(location: 0, length: numberOfGlyphs)
        guard glyphsToShow.length > 0 else { return [] }

        let characterRange = characterRange(forGlyphRange: glyphsToShow, actualGlyphRange: nil)
        let nsString = textStorage.string as NSString
        var drawnBlockquoteStarts = Set<Int>()
        var rects: [CGRect] = []

        textStorage.enumerateAttribute(
            RenderBridgeAttributes.blockquoteBorderColor,
            in: characterRange,
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, _ in
            guard range.length > 0, let color = value as? UIColor else { return }

            let paragraphRange = nsString.paragraphRange(for: NSRange(location: range.location, length: 0))
            let paragraphStart = paragraphRange.location
            let groupRange = Self.blockquoteGroupCharacterRange(
                containing: paragraphStart,
                in: textStorage,
                nsString: nsString
            )
            let groupStart = groupRange.location
            guard drawnBlockquoteStarts.insert(groupStart).inserted else { return }
            guard let rect = blockquoteStripeRect(
                characterRange: groupRange,
                color: color,
                textStorage: textStorage,
                origin: origin
            ) else {
                return
            }
            rects.append(rect)
        }

        return rects
    }

    func resetBlockquoteStripeDrawPassesForTesting() {
        blockquoteStripeDrawPassesForTesting.removeAll()
    }

    override func drawGlyphs(forGlyphRange glyphsToShow: NSRange, at origin: CGPoint) {
        super.drawGlyphs(forGlyphRange: glyphsToShow, at: origin)

        guard let textStorage, glyphsToShow.length > 0 else { return }

        let characterRange = characterRange(forGlyphRange: glyphsToShow, actualGlyphRange: nil)
        let nsString = textStorage.string as NSString
        var drawnParagraphStarts = Set<Int>()
        var drawnBlockquoteStarts = Set<Int>()
        var drawnStripeRects: [CGRect] = []

        textStorage.enumerateAttribute(
            RenderBridgeAttributes.listMarkerContext,
            in: characterRange,
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, _ in
            guard range.length > 0, let listContext = value as? [String: Any] else { return }

            let paragraphRange = nsString.paragraphRange(for: NSRange(location: range.location, length: 0))
            let paragraphStart = paragraphRange.location
            guard !Self.isParagraphStartCreatedByHardBreak(paragraphStart, in: textStorage) else {
                return
            }
            guard drawnParagraphStarts.insert(paragraphStart).inserted else { return }

            self.drawListMarker(
                listContext: listContext,
                paragraphStart: paragraphStart,
                origin: origin,
                textStorage: textStorage
            )
        }

        textStorage.enumerateAttribute(
            RenderBridgeAttributes.blockquoteBorderColor,
            in: characterRange,
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, _ in
            guard range.length > 0, let color = value as? UIColor else { return }

            let paragraphRange = nsString.paragraphRange(for: NSRange(location: range.location, length: 0))
            let paragraphStart = paragraphRange.location
            let groupRange = Self.blockquoteGroupCharacterRange(
                containing: paragraphStart,
                in: textStorage,
                nsString: nsString
            )
            let groupStart = groupRange.location
            guard drawnBlockquoteStarts.insert(groupStart).inserted else { return }

            guard let stripeRect = self.blockquoteStripeRect(
                characterRange: groupRange,
                color: color,
                textStorage: textStorage,
                origin: origin
            ) else {
                return
            }
            self.drawBlockquoteBorder(
                stripeRect: stripeRect,
                color: color
            )
            drawnStripeRects.append(stripeRect)
        }

        if !drawnStripeRects.isEmpty {
            blockquoteStripeDrawPassesForTesting.append(drawnStripeRects)
        }
    }

    private func drawListMarker(
        listContext: [String: Any],
        paragraphStart: Int,
        origin: CGPoint,
        textStorage: NSTextStorage
    ) {
        guard paragraphStart < textStorage.length else { return }

        let glyphIndex = glyphIndexForCharacter(at: paragraphStart)
        guard glyphIndex < numberOfGlyphs else { return }

        var lineGlyphRange = NSRange()
        let usedRect = lineFragmentUsedRect(forGlyphAt: glyphIndex, effectiveRange: &lineGlyphRange)
        let lineFragmentRect = self.lineFragmentRect(forGlyphAt: glyphIndex, effectiveRange: nil)
        let attrs = textStorage.attributes(at: paragraphStart, effectiveRange: nil)

        let baseFont = Self.markerBaseFont(from: attrs)
        let textColor = attrs[RenderBridgeAttributes.listMarkerColor] as? UIColor
            ?? attrs[.foregroundColor] as? UIColor
            ?? .label
        let markerScale = (attrs[RenderBridgeAttributes.listMarkerScale] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? LayoutConstants.unorderedListMarkerFontScale
        let markerWidth = (attrs[RenderBridgeAttributes.listMarkerWidth] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? LayoutConstants.listMarkerWidth
        let ordered = (listContext["ordered"] as? NSNumber)?.boolValue ?? false

        let glyphLocation = location(forGlyphAt: glyphIndex)
        let baselineY = lineFragmentRect.minY + glyphLocation.y

        if ordered {
            let markerFont = markerFont(
                for: listContext,
                baseFont: baseFont,
                markerScale: markerScale
            )
            let markerText = RenderBridge.listMarkerString(listContext: listContext)
            let markerOrigin = Self.orderedMarkerDrawingOrigin(
                usedRect: usedRect,
                lineFragmentRect: lineFragmentRect,
                markerWidth: markerWidth,
                baselineY: baselineY,
                markerFont: markerFont,
                markerText: markerText,
                origin: origin
            )
            let markerAttrs: [NSAttributedString.Key: Any] = [
                .font: markerFont,
                .foregroundColor: textColor,
            ]
            NSAttributedString(string: markerText, attributes: markerAttrs).draw(at: markerOrigin)
            return
        }

        let bulletRect = Self.unorderedBulletDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: markerWidth,
            baselineY: baselineY,
            baseFont: baseFont,
            markerScale: markerScale,
            origin: origin
        )
        let path = UIBezierPath(ovalIn: bulletRect)
        textColor.setFill()
        path.fill()
    }

    private func blockquoteStripeRect(
        characterRange: NSRange,
        color: UIColor,
        textStorage: NSTextStorage,
        origin: CGPoint
    ) -> CGRect? {
        guard characterRange.location < textStorage.length, !textContainers.isEmpty else {
            return nil
        }

        ensureLayout(forCharacterRange: characterRange)
        let glyphRange = self.glyphRange(forCharacterRange: characterRange, actualCharacterRange: nil)
        guard glyphRange.length > 0 else { return nil }

        var topEdge: CGFloat?
        var bottomEdge: CGFloat?
        var textLeadingEdge: CGFloat?
        enumerateLineFragments(forGlyphRange: glyphRange) { lineFragmentRect, usedRect, _, _, _ in
            let verticalReferenceRect = usedRect.height > 0 ? usedRect : lineFragmentRect
            if let currentTop = topEdge {
                topEdge = min(currentTop, lineFragmentRect.minY)
            } else {
                topEdge = lineFragmentRect.minY
            }
            if let currentBottom = bottomEdge {
                bottomEdge = max(currentBottom, verticalReferenceRect.maxY)
            } else {
                bottomEdge = verticalReferenceRect.maxY
            }
            let referenceMinX = usedRect.width > 0 ? usedRect.minX : lineFragmentRect.minX
            if let current = textLeadingEdge {
                textLeadingEdge = min(current, referenceMinX)
            } else {
                textLeadingEdge = referenceMinX
            }
        }
        guard let topEdge, let bottomEdge, bottomEdge > topEdge, let textLeadingEdge else { return nil }

        let attrs = textStorage.attributes(at: characterRange.location, effectiveRange: nil)
        let borderWidth = (attrs[RenderBridgeAttributes.blockquoteBorderWidth] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? LayoutConstants.blockquoteBorderWidth
        let gap = (attrs[RenderBridgeAttributes.blockquoteMarkerGap] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? LayoutConstants.blockquoteMarkerGap

        let stripeX = origin.x + textLeadingEdge - gap - borderWidth
        let stripeRect = CGRect(
            x: stripeX,
            y: origin.y + topEdge,
            width: borderWidth,
            height: bottomEdge - topEdge
        )
        return stripeRect
    }

    private func drawBlockquoteBorder(
        stripeRect: CGRect,
        color: UIColor
    ) {
        color.setFill()
        UIBezierPath(rect: stripeRect).fill()
    }

    private static func blockquoteGroupCharacterRange(
        containing paragraphStart: Int,
        in textStorage: NSTextStorage,
        nsString: NSString
    ) -> NSRange {
        let initialParagraphRange = nsString.paragraphRange(
            for: NSRange(location: paragraphStart, length: 0)
        )
        var groupStart = initialParagraphRange.location
        var groupEnd = NSMaxRange(initialParagraphRange)

        var probeStart = groupStart
        while probeStart > 0 {
            let previousParagraphRange = nsString.paragraphRange(
                for: NSRange(location: probeStart - 1, length: 0)
            )
            let previousStart = previousParagraphRange.location
            guard paragraphHasBlockquoteBorder(
                      previousParagraphRange,
                      in: textStorage
                  )
            else {
                break
            }

            groupStart = previousStart
            probeStart = previousStart
        }

        var nextParagraphLocation = groupEnd
        while nextParagraphLocation < textStorage.length {
            let nextParagraphRange = nsString.paragraphRange(
                for: NSRange(location: nextParagraphLocation, length: 0)
            )
            guard paragraphHasBlockquoteBorder(
                      nextParagraphRange,
                      in: textStorage
                  )
            else {
                break
            }

            groupEnd = NSMaxRange(nextParagraphRange)
            nextParagraphLocation = groupEnd
        }

        return NSRange(location: groupStart, length: groupEnd - groupStart)
    }

    private static func paragraphHasBlockquoteBorder(
        _ paragraphRange: NSRange,
        in textStorage: NSTextStorage
    ) -> Bool {
        guard paragraphRange.length > 0 else { return false }
        let nsString = textStorage.string as NSString
        var sawQuotedContent = false
        var sawAnyQuotedCharacter = false

        for offset in 0..<paragraphRange.length {
            let index = paragraphRange.location + offset
            guard index < textStorage.length else { break }

            let hasBorder = textStorage.attribute(
                RenderBridgeAttributes.blockquoteBorderColor,
                at: index,
                effectiveRange: nil
            ) != nil
            guard hasBorder else { continue }
            sawAnyQuotedCharacter = true

            let scalar = nsString.character(at: index)
            if scalar != 0x000A, scalar != 0x000D {
                sawQuotedContent = true
                break
            }
        }

        if sawQuotedContent {
            return true
        }

        let trimmed = nsString.substring(with: paragraphRange)
            .trimmingCharacters(in: .newlines)
        return trimmed.isEmpty && sawAnyQuotedCharacter
    }

    static func markerParagraphStyle(from attrs: [NSAttributedString.Key: Any]) -> NSMutableParagraphStyle {
        let markerStyle = NSMutableParagraphStyle()
        let sourceStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        markerStyle.minimumLineHeight = sourceStyle?.minimumLineHeight ?? 0
        markerStyle.maximumLineHeight = sourceStyle?.maximumLineHeight ?? 0
        markerStyle.lineHeightMultiple = sourceStyle?.lineHeightMultiple ?? 0
        markerStyle.baseWritingDirection = sourceStyle?.baseWritingDirection ?? .natural
        markerStyle.alignment = .right
        markerStyle.lineBreakMode = .byClipping
        markerStyle.firstLineHeadIndent = 0
        markerStyle.headIndent = 0
        markerStyle.tailIndent = 0

        return markerStyle
    }

    static func markerDrawingRect(
        usedRect: CGRect,
        lineFragmentRect: CGRect,
        markerWidth: CGFloat,
        baselineY: CGFloat,
        markerFont: UIFont,
        origin: CGPoint
    ) -> CGRect {
        let typographicHeight = markerFont.ascender - markerFont.descender
        let leading = max(markerFont.lineHeight - typographicHeight, 0)
        let topY = baselineY - markerFont.ascender - (leading / 2.0)
        let referenceRect = usedRect.height > 0 ? usedRect : lineFragmentRect
        return CGRect(
            x: origin.x + referenceRect.minX - markerWidth,
            y: origin.y + topY,
            width: markerWidth - 4.0,
            height: markerFont.lineHeight
        )
    }

    static func orderedMarkerDrawingOrigin(
        usedRect: CGRect,
        lineFragmentRect: CGRect,
        markerWidth: CGFloat,
        baselineY: CGFloat,
        markerFont: UIFont,
        markerText: String,
        origin: CGPoint
    ) -> CGPoint {
        let referenceRect = usedRect.height > 0 ? usedRect : lineFragmentRect
        let visibleMarkerText = markerText.trimmingCharacters(in: .whitespaces)
        let markerSize = (visibleMarkerText as NSString).size(withAttributes: [
            .font: markerFont,
        ])
        let rightInset: CGFloat = 4.0
        let x = origin.x + referenceRect.minX - rightInset - ceil(markerSize.width)
        let y = origin.y + baselineY - markerFont.ascender
        return CGPoint(x: x, y: y)
    }

    static func markerBaselineOffset(
        for listContext: [String: Any],
        baseFont: UIFont,
        markerFont: UIFont
    ) -> CGFloat {
        let ordered = (listContext["ordered"] as? NSNumber)?.boolValue ?? false
        guard !ordered else { return 0 }

        let targetMidline = (baseFont.xHeight > 0 ? baseFont.xHeight : baseFont.capHeight) / 2.0
        let glyphMidline = unorderedBulletGlyphMidline(for: markerFont)
        return targetMidline - glyphMidline
    }

    static func unorderedBulletDrawingRect(
        usedRect: CGRect,
        lineFragmentRect: CGRect,
        markerWidth: CGFloat,
        baselineY: CGFloat,
        baseFont: UIFont,
        markerScale: CGFloat,
        origin: CGPoint
    ) -> CGRect {
        let markerFont = baseFont.withSize(baseFont.pointSize * markerScale)
        let bulletBounds = unorderedBulletGlyphBounds(for: markerFont)
        let bulletDiameter = max(max(bulletBounds.width, bulletBounds.height), 1)
        let targetCenterAboveBaseline = (baseFont.xHeight > 0 ? baseFont.xHeight : baseFont.capHeight) / 2.0
        let centerY = baselineY - targetCenterAboveBaseline
        let referenceRect = usedRect.height > 0 ? usedRect : lineFragmentRect
        let rightInset = LayoutConstants.listMarkerTextGap
        let x = origin.x + referenceRect.minX - rightInset - bulletDiameter
        let y = origin.y + centerY - (bulletDiameter / 2.0)

        return CGRect(
            x: x,
            y: y,
            width: bulletDiameter,
            height: bulletDiameter
        )
    }

    static func isParagraphStartCreatedByHardBreak(
        _ paragraphStart: Int,
        in textStorage: NSTextStorage
    ) -> Bool {
        guard paragraphStart > 0, paragraphStart <= textStorage.length else { return false }
        let previousVoidType = textStorage.attribute(
            RenderBridgeAttributes.voidNodeType,
            at: paragraphStart - 1,
            effectiveRange: nil
        ) as? String
        return previousVoidType == "hardBreak"
    }

    private func markerFont(
        for listContext: [String: Any],
        baseFont: UIFont,
        markerScale: CGFloat
    ) -> UIFont {
        let ordered = (listContext["ordered"] as? NSNumber)?.boolValue ?? false
        if ordered {
            return baseFont
        }
        return baseFont.withSize(baseFont.pointSize * markerScale)
    }

    static func markerBaseFont(
        from attrs: [NSAttributedString.Key: Any],
        fallback fallbackFont: UIFont = .systemFont(ofSize: 16)
    ) -> UIFont {
        (attrs[RenderBridgeAttributes.listMarkerBaseFont] as? UIFont)
            ?? (attrs[.font] as? UIFont)
            ?? fallbackFont
    }

    private static func unorderedBulletGlyphBounds(for font: UIFont) -> CGRect {
        let ctFont = font as CTFont
        let bullet = UniChar(0x2022)
        var glyph = CGGlyph()
        guard CTFontGetGlyphsForCharacters(ctFont, [bullet], &glyph, 1) else {
            let fallbackDiameter = max(font.pointSize * 0.28, 1)
            return CGRect(x: 0, y: 0, width: fallbackDiameter, height: fallbackDiameter)
        }

        var boundingRect = CGRect.zero
        CTFontGetBoundingRectsForGlyphs(ctFont, .default, [glyph], &boundingRect, 1)
        if boundingRect.isNull || boundingRect.isEmpty {
            let fallbackDiameter = max(font.pointSize * 0.28, 1)
            return CGRect(x: 0, y: 0, width: fallbackDiameter, height: fallbackDiameter)
        }

        return boundingRect
    }

    private static func unorderedBulletGlyphMidline(for font: UIFont) -> CGFloat {
        unorderedBulletGlyphBounds(for: font).midY
    }
}
