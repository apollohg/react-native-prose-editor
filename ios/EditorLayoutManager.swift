import UIKit
import CoreText

/// Draws list markers visually in the gutter without inserting them into the
/// editable text storage. This keeps UIKit paragraph-start behaviors, such as
/// sentence auto-capitalization, working naturally inside list items.
final class EditorLayoutManager: NSLayoutManager, NSLayoutManagerDelegate {
    override init() {
        super.init()
        delegate = self
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        delegate = self
    }

    func layoutManager(_ layoutManager: NSLayoutManager, shouldGenerateGlyphs glyphs: UnsafePointer<CGGlyph>, properties props: UnsafePointer<NSLayoutManager.GlyphProperty>, characterIndexes: UnsafePointer<Int>, font: UIFont, forGlyphRange glyphRange: NSRange) -> Int {
        guard let storage = textStorage else { return 0 }
        // Reserve one chip glyph while retaining every canonical UTF-16 offset.
        var properties = Array(UnsafeBufferPointer(start: props, count: glyphRange.length))
        var changed = false
        for index in 0..<glyphRange.length {
            var range = NSRange()
            guard storage.attribute(editorMentionBoxAttribute, at: characterIndexes[index], effectiveRange: &range) is EditorMentionRenderedBox else { continue }
            properties[index] = characterIndexes[index] == range.location && (index == 0 || characterIndexes[index - 1] != range.location) ? .controlCharacter : .null
            changed = true
        }
        guard changed else { return 0 }
        setGlyphs(glyphs, properties: properties, characterIndexes: characterIndexes, font: font, forGlyphRange: glyphRange)
        return glyphRange.length
    }

    func layoutManager(_ layoutManager: NSLayoutManager, shouldUse action: NSLayoutManager.ControlCharacterAction, forControlCharacterAt charIndex: Int) -> NSLayoutManager.ControlCharacterAction {
        textStorage?.attribute(editorMentionBoxAttribute, at: charIndex, effectiveRange: nil) is EditorMentionRenderedBox ? .whitespace : action
    }

    func layoutManager(_ layoutManager: NSLayoutManager, boundingBoxForControlGlyphAt glyphIndex: Int, for textContainer: NSTextContainer, proposedLineFragment proposedRect: CGRect, glyphPosition: CGPoint, characterIndex charIndex: Int) -> CGRect {
        guard let chip = textStorage?.attribute(editorMentionBoxAttribute, at: charIndex, effectiveRange: nil) as? EditorMentionRenderedBox else { return .zero }
        return CGRect(origin: .zero, size: chip.size)
    }

    func layoutManager(_ layoutManager: NSLayoutManager, shouldBreakLineByWordBeforeCharacterAt charIndex: Int) -> Bool {
        guard let storage = textStorage, charIndex < storage.length else { return true }
        var range = NSRange()
        guard storage.attribute(editorMentionBoxAttribute, at: charIndex, effectiveRange: &range) is EditorMentionRenderedBox else { return true }
        return charIndex == range.location
    }

    func layoutManager(_ layoutManager: NSLayoutManager, shouldSetLineFragmentRect lineFragmentRect: UnsafeMutablePointer<CGRect>, lineFragmentUsedRect: UnsafeMutablePointer<CGRect>, baselineOffset: UnsafeMutablePointer<CGFloat>, in textContainer: NSTextContainer, forGlyphRange glyphRange: NSRange) -> Bool {
        guard let storage = textStorage, storage.length > 0 else { return false }
        let characters = characterRange(forGlyphRange: glyphRange, actualGlyphRange: nil)
        var height = lineFragmentUsedRect.pointee.height
        storage.enumerateAttribute(editorInlineLineHeightAttribute, in: characters) { value, _, _ in
            height = max(height, EditorTheme.cgFloat(value) ?? 0)
        }
        let extra = height - lineFragmentUsedRect.pointee.height
        var leading: CGFloat = 0
        if glyphRange.location == 0,
           storage.attribute(editorStyledContentAttribute, at: 0, effectiveRange: nil) != nil,
           let style = storage.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle {
            leading = style.paragraphSpacingBefore
        }
        guard extra != 0 || leading != 0 else { return false }
        lineFragmentRect.pointee.size.height += extra + leading
        lineFragmentUsedRect.pointee.size.height = height
        lineFragmentUsedRect.pointee.origin.y += leading
        baselineOffset.pointee += extra / 2 + leading
        return true
    }

    override func usedRect(for textContainer: NSTextContainer) -> CGRect {
        var rect = super.usedRect(for: textContainer)
        if let storage = textStorage, storage.length > 0,
           storage.attribute(editorStyledContentAttribute, at: storage.length - 1, effectiveRange: nil) != nil,
           let paragraph = storage.attribute(.paragraphStyle, at: storage.length - 1, effectiveRange: nil) as? NSParagraphStyle {
            rect.size.height += paragraph.paragraphSpacing
        }
        return rect
    }
    private(set) var blockquoteStripeDrawPassesForTesting: [[CGRect]] = []
    private(set) var codeBlockDrawPassesForTesting: [[CGRect]] = []

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

    func resetCodeBlockDrawPassesForTesting() {
        codeBlockDrawPassesForTesting.removeAll()
    }

    override func drawGlyphs(forGlyphRange glyphsToShow: NSRange, at origin: CGPoint) {
        guard let textStorage, glyphsToShow.length > 0 else { return }

        drawStyleBoxes(in: textStorage, glyphsToShow: glyphsToShow, origin: origin)
        drawMentionBoxes(in: textStorage, glyphsToShow: glyphsToShow, origin: origin)
        drawCodeBlockBackgrounds(
            in: textStorage,
            glyphsToShow: glyphsToShow,
            origin: origin
        )
        super.drawGlyphs(forGlyphRange: glyphsToShow, at: origin)

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

    private func drawMentionBoxes(in storage: NSTextStorage, glyphsToShow: NSRange, origin: CGPoint) {
        guard let context = UIGraphicsGetCurrentContext() else { return }
        let visible = characterRange(forGlyphRange: glyphsToShow, actualGlyphRange: nil)
        storage.enumerateAttribute(editorMentionBoxAttribute, in: visible) { value, range, _ in
            guard let box = value as? EditorMentionRenderedBox else { return }
            let glyph = self.glyphIndexForCharacter(at: range.location)
            let line = self.lineFragmentUsedRect(forGlyphAt: glyph, effectiveRange: nil)
            let location = self.location(forGlyphAt: glyph)
            let fragment = self.lineFragmentRect(forGlyphAt: glyph, effectiveRange: nil)
            let rect = CGRect(x: fragment.minX + location.x + origin.x, y: line.midY - box.size.height / 2 + origin.y, width: box.size.width, height: box.size.height)
            box.box.draw(in: rect, context: context)
            let labelHeight = box.label?.size().height ?? 0
            box.label?.draw(at: CGPoint(x: rect.minX + box.padding.left,
                                       y: rect.minY + box.padding.top + (rect.height - box.padding.top - box.padding.bottom - labelHeight) / 2))
        }
    }

    private func drawStyleBoxes(in storage: NSTextStorage, glyphsToShow: NSRange, origin: CGPoint) {
        guard let context = UIGraphicsGetCurrentContext(), let container = textContainers.first else { return }
        let visible = characterRange(forGlyphRange: glyphsToShow, actualGlyphRange: nil)
        var groups: [ObjectIdentifier: (EditorRenderedBox, NSRange)] = [:]
        storage.enumerateAttribute(editorStyleBoxesAttribute, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            for box in value as? [EditorRenderedBox] ?? [] {
                let id = ObjectIdentifier(box)
                groups[id] = (box, groups[id].map { NSUnionRange($0.1, range) } ?? range)
            }
        }
        for (descriptor, range) in groups.values.sorted(by: { $0.0.depth < $1.0.depth }) where NSIntersectionRange(range, visible).length > 0 {
            let glyphs = glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            var union = CGRect.null
            enumerateLineFragments(forGlyphRange: glyphs) { line, used, _, _, _ in
                union = union.union(used.height > 0 ? used : line)
            }
            guard !union.isNull else { continue }
            let rect = CGRect(x: origin.x + descriptor.leading + container.lineFragmentPadding,
                              y: origin.y + union.minY - descriptor.topInset,
                              width: max(0, container.size.width - descriptor.leading - descriptor.trailing - container.lineFragmentPadding * 2),
                              height: union.height + descriptor.topInset + descriptor.bottomInset)
            descriptor.box.draw(in: rect, context: context)
        }
    }

    func taskListMarkerParagraphStart(
        at point: CGPoint,
        in textStorage: NSTextStorage,
        textContainerOrigin: CGPoint
    ) -> Int? {
        guard numberOfGlyphs > 0, let container = textContainers.first else { return nil }

        // Resolve the touched line first — O(1) instead of walking every
        // task item in the document on every touch.
        let containerPoint = CGPoint(
            x: point.x - textContainerOrigin.x,
            y: point.y - textContainerOrigin.y
        )
        let glyphIndex = self.glyphIndex(for: containerPoint, in: container)
        if let hit = taskListMarkerParagraphStart(
            forParagraphContainingGlyphAt: glyphIndex,
            tapPoint: point,
            in: textStorage,
            textContainerOrigin: textContainerOrigin
        ) {
            return hit
        }

        // The tap-slop inset (dx: -10, dy: -8) below, plus the checkbox
        // itself already being taller than one line's pitch, means the
        // marker rect for the PREVIOUS or NEXT line can still contain
        // `point` even though `point` glyph-resolves to a different line.
        // Probe both neighboring lines by nudging just past the resolved
        // line's own fragment bounds — this reaches the adjacent paragraph
        // regardless of how deep `point` sits within its own line, and it
        // stays O(1) (two extra glyph lookups).
        let lineFragmentRect = self.lineFragmentRect(forGlyphAt: glyphIndex, effectiveRange: nil)

        if lineFragmentRect.minY > 0 {
            let aboveGlyphIndex = self.glyphIndex(
                for: CGPoint(x: containerPoint.x, y: lineFragmentRect.minY - 1),
                in: container
            )
            if let hit = taskListMarkerParagraphStart(
                forParagraphContainingGlyphAt: aboveGlyphIndex,
                tapPoint: point,
                in: textStorage,
                textContainerOrigin: textContainerOrigin
            ) {
                return hit
            }
        }

        let belowGlyphIndex = self.glyphIndex(
            for: CGPoint(x: containerPoint.x, y: lineFragmentRect.maxY + 1),
            in: container
        )
        if let hit = taskListMarkerParagraphStart(
            forParagraphContainingGlyphAt: belowGlyphIndex,
            tapPoint: point,
            in: textStorage,
            textContainerOrigin: textContainerOrigin
        ) {
            return hit
        }

        return nil
    }

    /// Resolves the paragraph containing `glyphIndex` and, if it is a task
    /// item's paragraph start, checks whether that item's marker rect
    /// contains `tapPoint`.
    private func taskListMarkerParagraphStart(
        forParagraphContainingGlyphAt glyphIndex: Int,
        tapPoint: CGPoint,
        in textStorage: NSTextStorage,
        textContainerOrigin: CGPoint
    ) -> Int? {
        let charIndex = characterIndexForGlyph(at: glyphIndex)
        guard charIndex < textStorage.length else { return nil }

        let nsString = textStorage.string as NSString
        let paragraphRange = nsString.paragraphRange(for: NSRange(location: charIndex, length: 0))
        let paragraphStart = paragraphRange.location
        guard paragraphStart < textStorage.length else { return nil }
        guard !Self.isParagraphStartCreatedByHardBreak(paragraphStart, in: textStorage) else { return nil }

        // Only a task item's paragraph qualifies.
        guard let listContext = textStorage.attribute(
                  RenderBridgeAttributes.listMarkerContext,
                  at: paragraphStart,
                  effectiveRange: nil
              ) as? [String: Any],
              (listContext["kind"] as? String) == "task"
        else { return nil }

        // Existing marker-rect math, applied to just this paragraph.
        let startGlyphIndex = glyphIndexForCharacter(at: paragraphStart)
        guard startGlyphIndex < numberOfGlyphs else { return nil }

        let attrs = textStorage.attributes(at: paragraphStart, effectiveRange: nil)
        let baseFont = Self.markerBaseFont(from: attrs)
        let markerWidth = (attrs[RenderBridgeAttributes.listMarkerWidth] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? LayoutConstants.listMarkerWidth
        let markerGap = (attrs[RenderBridgeAttributes.listMarkerGap] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? LayoutConstants.listMarkerTextGap

        var lineGlyphRange = NSRange()
        let usedRect = lineFragmentUsedRect(forGlyphAt: startGlyphIndex, effectiveRange: &lineGlyphRange)
        let lineFragmentRect = self.lineFragmentRect(forGlyphAt: startGlyphIndex, effectiveRange: nil)
        let glyphLocation = location(forGlyphAt: startGlyphIndex)
        let baselineY = lineFragmentRect.minY + glyphLocation.y
        let markerRect = Self.taskMarkerDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: markerWidth,
            baselineY: baselineY,
            baseFont: baseFont,
            origin: textContainerOrigin,
            markerGap: markerGap,
            explicitSize: (attrs[editorTaskCheckboxAttribute] as? EditorMentionRenderedBox)?.box.number("size", fallback: 24)
        ).insetBy(dx: -10, dy: -8)

        return markerRect.contains(tapPoint) ? paragraphStart : nil
    }

    private func drawCodeBlockBackgrounds(
        in textStorage: NSTextStorage,
        glyphsToShow: NSRange,
        origin: CGPoint
    ) {
        let characterRange = characterRange(forGlyphRange: glyphsToShow, actualGlyphRange: nil)
        let nsString = textStorage.string as NSString
        var drawnBlockStarts = Set<Int>()
        var drawnCodeBlockRects: [CGRect] = []

        textStorage.enumerateAttribute(
            RenderBridgeAttributes.codeBlockBackgroundColor,
            in: characterRange,
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, _ in
            guard range.length > 0, let color = value as? UIColor else { return }

            let paragraphRange = nsString.paragraphRange(for: NSRange(location: range.location, length: 0))
            let paragraphStart = paragraphRange.location
            let codeBlockRange = Self.codeBlockCharacterRange(
                containing: paragraphStart,
                in: textStorage,
                nsString: nsString
            )
            guard drawnBlockStarts.insert(codeBlockRange.location).inserted else { return }

            guard let rect = self.codeBlockRect(
                characterRange: codeBlockRange,
                textStorage: textStorage,
                origin: origin
            ) else {
                return
            }

            let attrs = textStorage.attributes(at: paragraphStart, effectiveRange: nil)
            let radius = (attrs[RenderBridgeAttributes.codeBlockBorderRadius] as? NSNumber)
                .map { CGFloat(truncating: $0) }
                ?? 8

            color.setFill()
            UIBezierPath(roundedRect: rect, cornerRadius: radius).fill()
            drawnCodeBlockRects.append(rect)
        }

        if !drawnCodeBlockRects.isEmpty {
            codeBlockDrawPassesForTesting.append(drawnCodeBlockRects)
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
        let markerGap = (attrs[RenderBridgeAttributes.listMarkerGap] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? LayoutConstants.listMarkerTextGap
        let ordered = (listContext["ordered"] as? NSNumber)?.boolValue ?? false
        let isTask = (listContext["kind"] as? String) == "task"

        let glyphLocation = location(forGlyphAt: glyphIndex)
        let baselineY = lineFragmentRect.minY + glyphLocation.y

        if isTask {
            let checkboxRect = Self.taskMarkerDrawingRect(
                usedRect: usedRect,
                lineFragmentRect: lineFragmentRect,
                markerWidth: markerWidth,
                baselineY: baselineY,
                baseFont: baseFont,
                origin: origin,
                markerGap: markerGap,
                explicitSize: (attrs[editorTaskCheckboxAttribute] as? EditorMentionRenderedBox)?.box.number("size", fallback: 24)
            )
            if let box = attrs[editorTaskCheckboxAttribute] as? EditorMentionRenderedBox,
               let context = UIGraphicsGetCurrentContext() {
                EditorStyleSheet.drawCheckbox(box.box, in: checkboxRect, checked: listContext["checked"] as? Bool == true, context: context)
                return
            }
            drawTaskCheckbox(
                in: checkboxRect,
                checked: (listContext["checked"] as? NSNumber)?.boolValue ?? false,
                color: textColor
            )
            return
        }

        if ordered {
            let markerFont = attrs[editorStyledContentAttribute] != nil
                ? baseFont.withSize(baseFont.pointSize * markerScale)
                : markerFont(for: listContext, baseFont: baseFont, markerScale: markerScale)
            let markerText = attrs[RenderBridgeAttributes.orderedListMarkerLabel] as? String
                ?? RenderBridge.listMarkerString(listContext: listContext)
                    .trimmingCharacters(in: .whitespaces)
            let markerOrigin = Self.orderedMarkerDrawingOrigin(
                usedRect: usedRect,
                lineFragmentRect: lineFragmentRect,
                markerWidth: markerWidth,
                baselineY: baselineY,
                markerFont: markerFont,
                markerText: markerText,
                origin: origin,
                markerGap: markerGap
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
            origin: origin,
            markerGap: markerGap
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

    /// Walk contiguous paragraphs around `paragraphStart` while
    /// `paragraphPredicate` holds. When `requireAttributedJoin` is non-nil,
    /// a neighboring paragraph only joins the group if the newline character
    /// separating the two paragraphs ALSO carries that attribute — this is
    /// what distinguishes intra-block newlines (attributed) from the
    /// separator between two distinct blocks (bare).
    private static func attributeGroupCharacterRange(
        containing paragraphStart: Int,
        in textStorage: NSTextStorage,
        nsString: NSString,
        paragraphPredicate: (NSRange, NSTextStorage) -> Bool,
        requireAttributedJoin: NSAttributedString.Key?
    ) -> NSRange {
        let initialParagraphRange = nsString.paragraphRange(for: NSRange(location: paragraphStart, length: 0))
        var groupStart = initialParagraphRange.location
        var groupEnd = NSMaxRange(initialParagraphRange)

        func newlineJoins(at separatorIndex: Int) -> Bool {
            guard let key = requireAttributedJoin else { return true }
            guard separatorIndex >= 0, separatorIndex < textStorage.length else { return false }
            return textStorage.attribute(key, at: separatorIndex, effectiveRange: nil) != nil
        }

        var probeStart = groupStart
        while probeStart > 0 {
            let previousParagraphRange = nsString.paragraphRange(for: NSRange(location: probeStart - 1, length: 0))
            guard paragraphPredicate(previousParagraphRange, textStorage),
                  newlineJoins(at: probeStart - 1)
            else { break }
            groupStart = previousParagraphRange.location
            probeStart = previousParagraphRange.location
        }

        var nextParagraphLocation = groupEnd
        while nextParagraphLocation < textStorage.length {
            let nextParagraphRange = nsString.paragraphRange(for: NSRange(location: nextParagraphLocation, length: 0))
            guard paragraphPredicate(nextParagraphRange, textStorage),
                  newlineJoins(at: nextParagraphLocation - 1)
            else { break }
            groupEnd = NSMaxRange(nextParagraphRange)
            nextParagraphLocation = groupEnd
        }

        return NSRange(location: groupStart, length: groupEnd - groupStart)
    }

    private static func blockquoteGroupCharacterRange(
        containing paragraphStart: Int,
        in textStorage: NSTextStorage,
        nsString: NSString
    ) -> NSRange {
        attributeGroupCharacterRange(
            containing: paragraphStart,
            in: textStorage,
            nsString: nsString,
            paragraphPredicate: paragraphHasBlockquoteBorder,
            requireAttributedJoin: nil  // blockquote merging keeps its existing semantics
        )
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

    static func codeBlockCharacterRange(
        containing paragraphStart: Int,
        in textStorage: NSTextStorage,
        nsString: NSString
    ) -> NSRange {
        attributeGroupCharacterRange(
            containing: paragraphStart,
            in: textStorage,
            nsString: nsString,
            paragraphPredicate: paragraphHasCodeBlockBackground,
            requireAttributedJoin: RenderBridgeAttributes.codeBlockBackgroundColor
        )
    }

    private static func paragraphHasCodeBlockBackground(
        _ paragraphRange: NSRange,
        in textStorage: NSTextStorage
    ) -> Bool {
        guard paragraphRange.length > 0 else { return false }
        return textStorage.attribute(
            RenderBridgeAttributes.codeBlockBackgroundColor,
            at: paragraphRange.location,
            effectiveRange: nil
        ) != nil
    }

    private func codeBlockRect(
        characterRange: NSRange,
        textStorage: NSTextStorage,
        origin: CGPoint
    ) -> CGRect? {
        guard characterRange.location < textStorage.length, !textContainers.isEmpty else {
            return nil
        }

        ensureLayout(forCharacterRange: characterRange)
        let glyphRange = self.glyphRange(forCharacterRange: characterRange, actualCharacterRange: nil)
        guard glyphRange.length > 0 else { return nil }

        let attrs = textStorage.attributes(at: characterRange.location, effectiveRange: nil)
        let horizontalPadding = (attrs[RenderBridgeAttributes.codeBlockPaddingHorizontal] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? 12
        let verticalPadding = (attrs[RenderBridgeAttributes.codeBlockPaddingVertical] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? 8

        var minX: CGFloat?
        var maxX: CGFloat?
        var minY: CGFloat?
        var maxY: CGFloat?

        enumerateLineFragments(forGlyphRange: glyphRange) { lineFragmentRect, usedRect, _, _, _ in
            let referenceRect = usedRect.height > 0 ? usedRect : lineFragmentRect
            let lineMinX = referenceRect.minX - horizontalPadding
            let lineMaxX = lineFragmentRect.maxX + horizontalPadding
            let lineMinY = lineFragmentRect.minY
            let lineMaxY = referenceRect.maxY

            minX = min(minX ?? lineMinX, lineMinX)
            maxX = max(maxX ?? lineMaxX, lineMaxX)
            minY = min(minY ?? lineMinY, lineMinY)
            maxY = max(maxY ?? lineMaxY, lineMaxY)
        }

        guard let minX, let maxX, let minY, let maxY, maxY > minY else { return nil }

        return CGRect(
            x: origin.x + minX,
            y: origin.y + minY - verticalPadding,
            width: maxX - minX,
            height: (maxY - minY) + (verticalPadding * 2)
        )
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

    static func taskMarkerDrawingRect(
        usedRect: CGRect,
        lineFragmentRect: CGRect,
        markerWidth: CGFloat,
        baselineY: CGFloat,
        baseFont: UIFont,
        origin: CGPoint,
        markerGap: CGFloat = LayoutConstants.listMarkerTextGap,
        explicitSize: CGFloat? = nil
    ) -> CGRect {
        let referenceRect = usedRect.height > 0 ? usedRect : lineFragmentRect
        let checkboxSize = explicitSize ?? min(
            max(baseFont.lineHeight * 1.05, 24),
            max(markerWidth - 4, 24)
        )
        let centerY = baselineY - ((baseFont.ascender + baseFont.descender) / 2.0)
        let x = origin.x + referenceRect.minX - markerGap - checkboxSize
        let y = origin.y + centerY - (checkboxSize / 2.0)
        return CGRect(x: x, y: y, width: checkboxSize, height: checkboxSize)
    }

    static func orderedMarkerDrawingOrigin(
        usedRect: CGRect,
        lineFragmentRect: CGRect,
        markerWidth: CGFloat,
        baselineY: CGFloat,
        markerFont: UIFont,
        markerText: String,
        origin: CGPoint,
        markerGap: CGFloat = LayoutConstants.listMarkerTextGap
    ) -> CGPoint {
        let referenceRect = usedRect.height > 0 ? usedRect : lineFragmentRect
        let visibleMarkerText = markerText.trimmingCharacters(in: .whitespaces)
        let markerSize = (visibleMarkerText as NSString).size(withAttributes: [
            .font: markerFont,
        ])
        let x = origin.x + referenceRect.minX - markerGap - ceil(markerSize.width)
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
        origin: CGPoint,
        markerGap: CGFloat = LayoutConstants.listMarkerTextGap
    ) -> CGRect {
        let markerFont = baseFont.withSize(baseFont.pointSize * markerScale)
        let bulletBounds = unorderedBulletGlyphBounds(for: markerFont)
        let bulletDiameter = max(max(bulletBounds.width, bulletBounds.height), 1)
        let targetCenterAboveBaseline = (baseFont.xHeight > 0 ? baseFont.xHeight : baseFont.capHeight) / 2.0
        let centerY = baselineY - targetCenterAboveBaseline
        let referenceRect = usedRect.height > 0 ? usedRect : lineFragmentRect
        let x = origin.x + referenceRect.minX - markerGap - bulletDiameter
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
        return EditorNodeTypes.isHardBreak(previousVoidType)
    }

    private func markerFont(
        for listContext: [String: Any],
        baseFont: UIFont,
        markerScale: CGFloat
    ) -> UIFont {
        let ordered = (listContext["ordered"] as? NSNumber)?.boolValue ?? false
        let isTask = (listContext["kind"] as? String) == "task"
        if ordered {
            return baseFont
        }
        if isTask {
            return baseFont.withSize(baseFont.pointSize * 1.35)
        }
        return baseFont.withSize(baseFont.pointSize * markerScale)
    }

    private func drawTaskCheckbox(
        in rect: CGRect,
        checked: Bool,
        color: UIColor
    ) {
        let path = UIBezierPath(roundedRect: rect, cornerRadius: min(rect.width, rect.height) * 0.22)
        color.setStroke()
        path.lineWidth = max(1.8, rect.width * 0.09)
        path.stroke()

        guard checked else { return }

        let checkPath = UIBezierPath()
        checkPath.move(to: CGPoint(x: rect.minX + rect.width * 0.22, y: rect.midY + rect.height * 0.04))
        checkPath.addLine(to: CGPoint(x: rect.minX + rect.width * 0.42, y: rect.maxY - rect.height * 0.24))
        checkPath.addLine(to: CGPoint(x: rect.maxX - rect.width * 0.18, y: rect.minY + rect.height * 0.24))
        checkPath.lineCapStyle = .round
        checkPath.lineJoinStyle = .round
        checkPath.lineWidth = max(2.1, rect.width * 0.12)
        color.setStroke()
        checkPath.stroke()
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
