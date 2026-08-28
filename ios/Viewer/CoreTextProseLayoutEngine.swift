import CoreText
import UIKit

private let preparedAtomAttribute = NSAttributedString.Key("PREPPreparedAtom")
/// Core Text has no strikethrough attribute. This marks a shaped run so its
/// immutable strike rectangle can be prepared from Core Text's own metrics.
private let preparedStrikeAttribute = NSAttributedString.Key("PREPPreparedStrike")

private extension Int {
    func rendererSaturatingMultiply(_ other: Int) -> Int {
        let result = multipliedReportingOverflow(by: other)
        return result.overflow ? Int.max : result.partialValue
    }
}

enum PreparedProseInteractionGeometry {
    static func visualOrder(_ left: CGRect, _ right: CGRect) -> Bool {
        if left.minY != right.minY { return left.minY < right.minY }
        if left.minX != right.minX { return left.minX < right.minX }
        if left.maxY != right.maxY { return left.maxY < right.maxY }
        return left.maxX < right.maxX
    }

    static func appendSameLinePiece(
        _ rect: CGRect,
        to rects: inout [CGRect],
        mayMergeWithPrior: Bool
    ) {
        guard mayMergeWithPrior,
              let prior = rects.last,
              prior.minY == rect.minY,
              // A semantic hit region may include only real glyph geometry:
              // overlapping pieces and exact edge contact are contiguous; any
              // positive gap must remain separately hittable/accessibility-visible.
              prior.maxX >= rect.minX
        else {
            rects.append(rect)
            return
        }
        rects[rects.count - 1] = prior.union(rect)
    }
}

private final class PreparedAtomMetrics {
    let width: CGFloat
    let ascent: CGFloat
    let descent: CGFloat

    init(width: CGFloat, ascent: CGFloat, descent: CGFloat) {
        self.width = width
        self.ascent = ascent
        self.descent = descent
    }
}

private func preparedAtomDelegate(_ metrics: PreparedAtomMetrics) -> CTRunDelegate {
    var callbacks = CTRunDelegateCallbacks(
        version: kCTRunDelegateVersion1,
        dealloc: { refCon in
            Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).release()
        },
        getAscent: { refCon in
            return Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).takeUnretainedValue().ascent
        },
        getDescent: { refCon in
            return Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).takeUnretainedValue().descent
        },
        getWidth: { refCon in
            return Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).takeUnretainedValue().width
        }
    )
    return CTRunDelegateCreate(&callbacks, Unmanaged.passRetained(metrics).toOpaque())!
}

struct PreparedTextPaint {
    let font: UIFont
    let color: UIColor
    let lineHeight: CGFloat?
    let spacingAfter: CGFloat
}

/// Theme parsing is deliberately outside the drawing path. A registry stores
/// this value once per generation and every width-specific artifact reuses it.
struct PreparedProseTheme {
    let fontScale: CGFloat
    let text: PreparedTextPaint
    let paragraph: PreparedTextPaint
    let headings: [String: PreparedTextPaint]
    let blockquote: PreparedTextPaint
    let code: PreparedTextPaint
    let contentInsets: UIEdgeInsets
    let listIndent: CGFloat
    let listBaseIndentMultiplier: CGFloat
    let listItemSpacing: CGFloat
    let listSpacingAfter: CGFloat
    let listMarkerColor: UIColor
    let listMarkerScale: CGFloat
    let listMarkerGap: CGFloat
    let orderedListMarker: EditorOrderedListMarkerTheme?
    static let defaultListMarkerGap: CGFloat = 6
    let quoteIndent: CGFloat
    let quoteBorderColor: UIColor
    let quoteBorderWidth: CGFloat
    let quoteMarkerGap: CGFloat
    let codeBackground: UIColor
    let codeRadius: CGFloat
    let codePaddingHorizontal: CGFloat
    let codePaddingVertical: CGFloat
    let ruleColor: UIColor
    let ruleThickness: CGFloat
    let ruleMargin: CGFloat
    let link: EditorLinkTheme?
    let mention: EditorMentionTheme?

    static func resolve(
        themeJSON: String?,
        fontScale: CGFloat = 1,
        semanticGeneration: String = "standalone-theme"
    ) -> PreparedProseTheme {
        let theme = EditorTheme.from(json: themeJSON) ?? EditorTheme(dictionary: [:])
        let resolvedScale = fontScale.isFinite && fontScale > 0 ? fontScale : 1
        let baseFont = UIFont.systemFont(ofSize: 17 * resolvedScale)
        func paint(_ style: EditorTextStyle?, fallback: PreparedTextPaint? = nil) -> PreparedTextPaint {
            let fallback = fallback ?? PreparedTextPaint(font: baseFont, color: .label, lineHeight: nil, spacingAfter: 0)
            guard let style else { return fallback }
            let resolvedFont = ViewerFontEnvironment.shared.resolveFont(
                style: style,
                fallback: fallback.font,
                fontScale: resolvedScale,
                semanticGeneration: semanticGeneration
            )
            return PreparedTextPaint(
                font: resolvedFont,
                color: style.color ?? fallback.color,
                lineHeight: style.lineHeight.map { $0 * resolvedScale } ?? fallback.lineHeight,
                spacingAfter: style.spacingAfter.map { $0 * resolvedScale } ?? fallback.spacingAfter
            )
        }
        let text = paint(theme.text)
        if let link = theme.links {
            _ = ViewerFontEnvironment.shared.resolveFont(
                style: EditorTextStyle(
                    fontFamily: link.fontFamily,
                    fontSize: link.fontSize,
                    fontWeight: link.fontWeight,
                    fontStyle: link.fontStyle
                ),
                fallback: text.font,
                fontScale: resolvedScale,
                semanticGeneration: semanticGeneration
            )
        }
        let paragraph = paint(theme.effectiveTextStyle(for: "paragraph"), fallback: text)
        let quote = paint(theme.effectiveTextStyle(for: "paragraph", inBlockquote: true), fallback: paragraph)
        let codeStyle = theme.effectiveTextStyle(for: "codeBlock")
        let codeFallback = PreparedTextPaint(
            font: UIFont.monospacedSystemFont(ofSize: text.font.pointSize, weight: .regular),
            color: text.color,
            lineHeight: text.lineHeight,
            spacingAfter: text.spacingAfter
        )
        var headings: [String: PreparedTextPaint] = [:]
        let defaults: [(String, CGFloat)] = [("h1", 32), ("h2", 28), ("h3", 24), ("h4", 21), ("h5", 19), ("h6", 17)]
        for (name, size) in defaults {
            let defaultHeading = EditorTextStyle(fontSize: size, fontWeight: "700", spacingAfter: 10)
            headings[name] = paint(
                theme.effectiveTextStyle(for: name, defaultStyle: defaultHeading),
                fallback: paragraph
            )
        }
        let listItemSpacing = theme.list?.itemSpacing ?? 4
        return PreparedProseTheme(
            fontScale: resolvedScale,
            text: text,
            paragraph: paragraph,
            headings: headings,
            blockquote: quote,
            code: paint(codeStyle, fallback: codeFallback),
            contentInsets: UIEdgeInsets(
                top: theme.contentInsets?.top ?? 0,
                left: theme.contentInsets?.left ?? 0,
                bottom: theme.contentInsets?.bottom ?? 0,
                right: theme.contentInsets?.right ?? 0
            ),
            listIndent: theme.list?.indent ?? 28,
            listBaseIndentMultiplier: theme.list?.baseIndentMultiplier ?? 1,
            listItemSpacing: listItemSpacing,
            listSpacingAfter: theme.list?.spacingAfter ?? listItemSpacing,
            listMarkerColor: theme.list?.markerColor ?? text.color,
            listMarkerScale: theme.list?.markerScale ?? 1,
            listMarkerGap: theme.list?.markerGap ?? PreparedProseTheme.defaultListMarkerGap,
            orderedListMarker: theme.list?.orderedMarker,
            quoteIndent: theme.blockquote?.indent ?? 16,
            quoteBorderColor: theme.blockquote?.borderColor ?? UIColor.systemGray3,
            quoteBorderWidth: theme.blockquote?.borderWidth ?? 3,
            quoteMarkerGap: theme.blockquote?.markerGap ?? 10,
            codeBackground: theme.codeBlock?.backgroundColor ?? UIColor.secondarySystemBackground,
            codeRadius: theme.codeBlock?.borderRadius ?? 8,
            codePaddingHorizontal: theme.codeBlock?.paddingHorizontal ?? 12,
            codePaddingVertical: theme.codeBlock?.paddingVertical ?? 8,
            ruleColor: theme.horizontalRule?.color ?? UIColor.separator,
            ruleThickness: theme.horizontalRule?.thickness ?? 1,
            ruleMargin: theme.horizontalRule?.verticalMargin ?? 12,
            link: theme.links,
            mention: theme.mentions
        )
    }

    func paint(for block: ViewerBlock) -> PreparedTextPaint {
        if block.nodeType == "codeBlock" { return code }
        if let heading = headings[block.nodeType] { return heading }
        if block.inBlockquote { return blockquote }
        return paragraph
    }

    /// UIFont/UIColor bridge objects and the resolved heading dictionary are
    /// retained by each cached generation theme. Keep the LRU's accounting
    /// deliberately conservative; paint values themselves are immutable.
    var estimatedRetainedBytes: Int { 3_072 + headings.count * 384 }
}

private struct PreparedAtomAppearance {
    let attributes: [NSAttributedString.Key: Any]
    let background: UIColor
    let borderColor: UIColor?
    let borderWidth: CGFloat
    let radius: CGFloat
    let padding: UIEdgeInsets
}

private struct PreparedAtomSpec {
    let range: NSRange
    let nodeType: String
    let docPos: UInt32
    let label: String
    let metrics: PreparedAtomMetrics
    let line: CTLine
    let appearance: PreparedAtomAppearance
}

private struct PreparedAttributedBlock {
    let string: NSAttributedString
    let atoms: [PreparedAtomSpec]
    let semanticRanges: [PreparedSemanticRange]
    let accessibilityRanges: [PreparedAccessibilityRange]
    let retainedBytes: Int
}

private enum PreparedSemanticRange {
    case link(range: NSRange, href: String, text: String)
    case mention(range: NSRange, docPos: UInt32, label: String, attrsJSON: String)

    var range: NSRange {
        switch self {
        case let .link(range, _, _), let .mention(range, _, _, _): range
        }
    }
}

private struct PreparedAccessibilityRange {
    enum Role: Equatable {
        case text
        case link(semanticIndex: Int)
        case mention(semanticIndex: Int)
    }

    let range: NSRange
    let label: String
    let role: Role
}

private struct PreparedListMarker {
    let line: CTLine?
    let label: String
    let width: CGFloat
    let ascent: CGFloat
    let descent: CGFloat
    let checked: Bool
}

/// Performs the width-dependent, immutable Core Text preparation step.
final class CoreTextProseLayoutEngine {
    /// UIFont and CTFont are toll-free bridged. Recreating a system font from
    /// UIFont.fontName is not equivalent on current iOS releases: private
    /// .SFUI names are not valid public Core Text PostScript names.
    static func coreTextFont(from font: UIFont) -> CTFont {
        font as CTFont
    }

    func prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        widthPoints: CGFloat,
        displayScale: CGFloat,
        semanticGenerationIdentity: String? = nil
    ) throws -> PreparedProseLayout {
        // This context is deliberately passed separately from the layout key's
        // revision-sensitive generation identity. A replacement layout for an
        // attachment/font/width revision must not reopen a missing-font warning.
        let warningSemanticGeneration = semanticGenerationIdentity ?? key.semanticGenerationIdentity
        guard let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: widthPoints, scale: displayScale) else {
            return .error(key: key, width: 0, error: .hostContract(message: "A finite positive width is required for prose measurement."))
        }
        let canonicalWidth = ProseLayoutMetrics.canonicalWidth(widthPixels: widthPixels, scale: displayScale)
        if document.isEmpty {
            return PreparedProseLayout(key: key, size: CGSize(width: canonicalWidth, height: 0), blocks: [], retainedBytes: document.retainedBytes)
        }

        let theme = document.preparedTheme ?? PreparedProseTheme.resolve(themeJSON: nil)
        var cursorY = theme.contentInsets.top
        var blocks: [PreparedProseBlock] = []
        var interactions: [PreparedProseInteraction] = []
        var accessibilityNodes: [PreparedProseAccessibilityNode] = []
        var imageAttachments: [ViewerImageAttachment] = []
        var retainedBytes = document.retainedBytes
        var listMarkersByIdentity: [Int: PreparedListMarker] = [:]
        for block in document.blocks {
            guard let boundary = block.listItemBoundary,
                  let context = block.listContext,
                  listMarkersByIdentity[boundary.identity] == nil
            else { continue }
            let markerNestingDepth = block.listItemAncestors.firstIndex { ancestor in
                ancestor.identity == boundary.identity
            } ?? 0
            listMarkersByIdentity[boundary.identity] = makeListMarker(
                context,
                nestingDepth: markerNestingDepth,
                paint: theme.paint(for: block),
                theme: theme
            )
        }
        for (index, block) in document.blocks.enumerated() {
            let listMarker = block.listItemBoundary.flatMap { listMarkersByIdentity[$0.identity] }
            let nextAncestorIdentities = Set(
                document.blocks.indices.contains(index + 1)
                    ? listItemAncestors(document.blocks[index + 1]).map(\.identity)
                    : []
            )
            let disappearingListItemIdentities = Set(
                listItemAncestors(block)
                    .filter { !nextAncestorIdentities.contains($0.identity) }
                    .map(\.identity)
            )
            let prepared = prepareBlock(
                block,
                attachmentOrdinal: imageAttachments.count,
                listMarker: listMarker,
                theme: theme,
                width: canonicalWidth,
                cursorY: cursorY,
                disappearingListItemIdentities: disappearingListItemIdentities,
                displayScale: displayScale,
                warningSemanticGeneration: warningSemanticGeneration
            )
            blocks.append(prepared.block)
            let interactionIndexOffset = interactions.count
            accessibilityNodes.append(contentsOf: prepared.accessibilityNodes.map { node in
                PreparedProseAccessibilityNode(
                    interactionIndex: node.interactionIndex.map { interactionIndexOffset + $0 },
                    role: node.role,
                    label: node.label,
                    rects: node.rects
                )
            })
            interactions.append(contentsOf: prepared.interactions)
            if let attachment = prepared.attachment { imageAttachments.append(attachment) }
            cursorY = prepared.nextY
            retainedBytes += prepared.retainedBytes
        }
        cursorY = (blocks.map(\.bounds.maxY).max() ?? cursorY) + theme.contentInsets.bottom
        let pixelHeight = ceil(cursorY * displayScale)
        retainedBytes += interactions.reduce(0) { $0 + $1.estimatedRetainedBytes }
            + accessibilityNodes.reduce(0) { $0 + $1.estimatedRetainedBytes }
        // Mounted image-publication sidecars are runtime surface ownership,
        // not immutable artifact/cache ownership; account them at the host.
        return PreparedProseLayout(
            key: key,
            size: CGSize(width: canonicalWidth, height: pixelHeight / displayScale),
            blocks: blocks,
            interactions: interactions,
            accessibilityNodes: accessibilityNodes,
            imageAttachments: imageAttachments,
            retainedBytes: retainedBytes
        )
    }

    private func listItemAncestors(_ block: ViewerBlock) -> [ViewerListItemAncestor] {
        if !block.listItemAncestors.isEmpty {
            return block.listItemAncestors
        }
        guard let boundary = block.listItemBoundary,
              let context = block.listContext
        else { return [] }
        return [ViewerListItemAncestor(identity: boundary.identity, context: context)]
    }

    private func prepareBlock(
        _ block: ViewerBlock,
        attachmentOrdinal: Int,
        listMarker: PreparedListMarker?,
        theme: PreparedProseTheme,
        width: CGFloat,
        cursorY: CGFloat,
        disappearingListItemIdentities: Set<Int>,
        displayScale: CGFloat,
        warningSemanticGeneration: String
    ) -> (block: PreparedProseBlock, interactions: [PreparedProseInteraction], accessibilityNodes: [PreparedProseAccessibilityNode], attachment: ViewerImageAttachment?, nextY: CGFloat, retainedBytes: Int) {
        let contentX = theme.contentInsets.left
        let contentWidth = max(1, width - theme.contentInsets.left - theme.contentInsets.right)
        let paint = theme.paint(for: block)
        let listDepth = block.listContext == nil
            ? 0
            : (block.listItemBoundary.map { Int($0.nestingDepth) } ?? max(0, Int(block.depth) - 1))
        let fallbackMarkerNestingDepth = max(0, block.listItemAncestors.count - 1)
        let measuredListMarker = listMarker ?? block.listContext.map {
            makeListMarker($0, nestingDepth: fallbackMarkerNestingDepth, paint: paint, theme: theme)
        }
        let marker = block.listItemBoundary.map { $0.isFirstRenderableLeaf ? measuredListMarker : nil } ?? measuredListMarker
        // The marker gutter is an independently measured column. In particular,
        // baseIndentMultiplier == 0 must not permit text to overlap a scaled
        // ordered marker or task box. `measuredListMarker` stays item-scoped,
        // which keeps paragraph/code/atom descendants aligned.
        let listBaseIndent = block.listContext == nil ? 0 : max(0, theme.listIndent * theme.listBaseIndentMultiplier)
        let nestedListIndent = block.listContext == nil ? 0 : max(0, theme.listIndent * CGFloat(listDepth))
        let markerGutter = measuredListMarker.map { max(theme.listMarkerGap, $0.width + theme.listMarkerGap) } ?? 0
        let listInset = listBaseIndent + nestedListIndent + markerGutter
        let quoteInset = block.inBlockquote ? theme.quoteBorderWidth + theme.quoteMarkerGap + theme.quoteIndent : 0
        let codeInset = block.nodeType == "codeBlock" ? theme.codePaddingHorizontal : 0
        let textX = contentX + listInset + quoteInset + codeInset
        let itemSpacing: CGFloat
        if block.listContext == nil {
            itemSpacing = paint.spacingAfter
        } else {
            let boundarySpacing = listItemAncestors(block).reduce(CGFloat.zero) { spacing, ancestor in
                if disappearingListItemIdentities.contains(ancestor.identity) {
                    return spacing + (ancestor.context.isLast ? theme.listSpacingAfter : theme.listItemSpacing)
                }
                if ancestor.identity == block.listItemBoundary?.identity,
                   block.listItemBoundary?.isFinalRenderableLeaf == true
                {
                    return spacing + theme.listItemSpacing
                }
                return spacing
            }
            itemSpacing = boundarySpacing
        }
        if block.nodeType == "image", let image = ViewerImageAttachment.sourceAndDeclaredSize(in: block) {
            let imageWidth = max(1, contentWidth - listInset - quoteInset)
            let provisionalHeight = max(44, min(240, imageWidth * 0.56))
            let declared = image.declaredSize
            let resolvedSize = declared ?? ViewerImageIntrinsicStore.shared.size(for: image.id)
            let height = resolvedSize.map { imageWidth * $0.height / max(1, $0.width) } ?? provisionalHeight
            let bounds = CGRect(x: textX, y: cursorY, width: imageWidth, height: height)
            let attachment = ViewerImageAttachment(ordinal: attachmentOrdinal, id: image.id, source: image.source, bounds: bounds, declaredSize: declared)
            let fragments = [PreparedProseFragment(kind: .image, bounds: bounds, color: UIColor.systemGray5.cgColor)]
            let prepared = PreparedProseBlock(fragments: fragments, bounds: bounds)
            let imageLabel = block.inlines.compactMap { inline -> String? in
                guard case let .atom("image", _, attrsJSON, _) = inline else { return nil }
                let alt = jsonDictionary(attrsJSON)["alt"] as? String
                return alt?.trimmingCharacters(in: .whitespacesAndNewlines)
            }.first
            let accessibleImageLabel = imageLabel.flatMap { $0.isEmpty ? nil : $0 } ?? "Image"
            let node = PreparedProseAccessibilityNode(
                interactionIndex: nil,
                role: .image,
                label: accessibleImageLabel,
                bounds: bounds
            )
            return (prepared, [], [node], attachment, bounds.maxY + itemSpacing, prepared.estimatedRetainedBytes + 192)
        }
        if block.nodeType == "horizontalRule" || block.nodeType == "horizontal_rule" {
            let ruleX = contentX + listInset + quoteInset
            let ruleWidth = max(1, contentWidth - listInset - quoteInset)
            let y = cursorY + theme.ruleMargin
            let rule = CGRect(x: ruleX, y: y, width: ruleWidth, height: theme.ruleThickness)
            var fragments: [PreparedProseFragment] = [.init(kind: .rule, bounds: rule, color: theme.ruleColor.cgColor, strokeWidth: theme.ruleThickness)]
            let totalEnd = y + theme.ruleThickness + theme.ruleMargin
            if block.inBlockquote {
                fragments.append(.init(kind: .border, bounds: CGRect(x: contentX, y: cursorY, width: theme.quoteBorderWidth, height: totalEnd - cursorY), color: theme.quoteBorderColor.cgColor, strokeWidth: theme.quoteBorderWidth))
            }
            if let marker {
                let markerX = textX - markerGutter
                let markerHeight = marker.ascent + marker.descent
                let markerTop = cursorY + (totalEnd - cursorY - markerHeight) / 2
                let markerBaseline = markerTop + marker.ascent
                let markerBounds = CGRect(x: markerX, y: markerTop, width: marker.width, height: markerHeight)
                fragments.append(.init(kind: .marker, line: marker.line, origin: CGPoint(x: markerX, y: markerBaseline), bounds: markerBounds, color: theme.listMarkerColor.cgColor, label: marker.label, checked: marker.checked))
            }
            let seedBounds = CGRect(x: contentX, y: cursorY, width: contentWidth, height: totalEnd - cursorY)
            let bounds = fragments.reduce(seedBounds) { $0.union($1.bounds) }
            let prepared = PreparedProseBlock(
                fragments: fragments,
                bounds: bounds
            )
            return (
                prepared,
                [],
                [PreparedProseAccessibilityNode(
                    interactionIndex: nil,
                    role: .separator,
                    label: "Separator",
                    bounds: bounds
                )],
                nil,
                totalEnd + itemSpacing,
                prepared.estimatedRetainedBytes
            )
        }

        let availableWidth = max(1, contentWidth - listInset - quoteInset - codeInset * 2)
        let attributed = makeAttributedString(block.inlines, paint: paint, theme: theme, warningSemanticGeneration: warningSemanticGeneration)
        let typesetter = CTTypesetterCreateWithAttributedString(attributed.string)
        var location = 0
        var fragments: [PreparedProseFragment] = []
        var interactionRects: [[CGRect]] = Array(repeating: [], count: attributed.semanticRanges.count)
        var accessibilityRects: [[CGRect]] = Array(repeating: [], count: attributed.accessibilityRanges.count)
        let semanticGeometryRanges = attributed.semanticRanges.enumerated().map {
            (index: $0.offset, range: $0.element.range)
        }
        let accessibilityGeometryRanges = attributed.accessibilityRanges.enumerated().compactMap {
            index, range -> (index: Int, range: NSRange)? in
            guard range.role == .text else { return nil }
            return (index, range.range)
        }
        var semanticGeometryCursor = 0
        var accessibilityGeometryCursor = 0
        let codeTopInset = block.nodeType == "codeBlock" ? theme.codePaddingVertical : 0
        let firstLineHeight = max(paint.font.lineHeight, paint.lineHeight ?? 0)
        let markerTopProtection = marker.map {
            max(0, ($0.ascent + $0.descent - firstLineHeight) / 2 - codeTopInset)
        } ?? 0
        var textTop = cursorY + codeTopInset + (cursorY == theme.contentInsets.top ? markerTopProtection : 0)
        var firstLineBounds: CGRect?
        while location < attributed.string.length {
            let suggested = CTTypesetterSuggestLineBreak(typesetter, location, availableWidth)
            let count = max(1, suggested)
            let line = CTTypesetterCreateLine(typesetter, CFRange(location: location, length: count))
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            var leading: CGFloat = 0
            let lineWidth = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
            let naturalHeight = ascent + descent + leading
            let lineHeight = max(naturalHeight, paint.lineHeight ?? 0)
            let baseline = textTop + (lineHeight - naturalHeight) / 2 + ascent
            let lineBounds = CGRect(x: textX, y: textTop, width: min(availableWidth, max(0, lineWidth)), height: lineHeight)
            let lineRange = NSRange(location: location, length: count)
            fragments.append(.init(kind: .text, line: line, origin: CGPoint(x: textX, y: baseline), bounds: lineBounds))
            fragments.append(contentsOf: strikeFragments(
                for: line,
                lineOrigin: CGPoint(x: textX, y: baseline),
                displayScale: displayScale
            ))
            appendShapedRects(
                ranges: semanticGeometryRanges,
                line: line,
                lineRange: lineRange,
                lineBounds: lineBounds,
                textX: textX,
                displayScale: displayScale,
                rangeCursor: &semanticGeometryCursor,
                to: &interactionRects
            )
            appendShapedRects(
                ranges: accessibilityGeometryRanges,
                line: line,
                lineRange: lineRange,
                lineBounds: lineBounds,
                textX: textX,
                displayScale: displayScale,
                rangeCursor: &accessibilityGeometryCursor,
                to: &accessibilityRects
            )
            if firstLineBounds == nil { firstLineBounds = lineBounds }
            for atom in attributed.atoms where NSIntersectionRange(atom.range, lineRange).length > 0 {
                let offset = CGFloat(CTLineGetOffsetForStringIndex(line, atom.range.location, nil))
                let atomBounds = CGRect(
                    x: textX + offset,
                    y: baseline - atom.metrics.ascent,
                    width: atom.metrics.width,
                    height: atom.metrics.ascent + atom.metrics.descent
                )
                fragments.append(
                    .init(
                        kind: .atom,
                        line: atom.line,
                        origin: CGPoint(x: atomBounds.minX + atom.appearance.padding.left, y: baseline),
                        bounds: atomBounds,
                        color: atom.appearance.background.cgColor,
                        borderColor: atom.appearance.borderColor?.cgColor,
                        cornerRadius: atom.appearance.radius,
                        strokeWidth: atom.appearance.borderWidth,
                        padding: atom.appearance.padding,
                        label: atom.label
                    )
                )
            }
            location += count
            textTop += lineHeight
        }
        if fragments.isEmpty {
            let fallbackHeight = paint.lineHeight ?? paint.font.lineHeight
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: "\u{200B}", attributes: baseAttributes(paint)))
            let lineBounds = CGRect(x: textX, y: textTop, width: 0, height: fallbackHeight)
            fragments.append(.init(kind: .text, line: line, origin: CGPoint(x: textX, y: textTop + paint.font.ascender), bounds: lineBounds))
            firstLineBounds = lineBounds
            textTop += fallbackHeight
        }
        let textEnd = textTop
        let totalEnd = textEnd + (block.nodeType == "codeBlock" ? theme.codePaddingVertical : 0)
        let blockRect = CGRect(x: contentX, y: cursorY, width: contentWidth, height: max(0, totalEnd - cursorY))
        if block.nodeType == "codeBlock" {
            fragments.insert(.init(kind: .background, bounds: blockRect, color: theme.codeBackground.cgColor, cornerRadius: theme.codeRadius), at: 0)
        }
        if block.inBlockquote {
            let border = CGRect(x: contentX, y: cursorY, width: theme.quoteBorderWidth, height: max(0, totalEnd - cursorY))
            fragments.append(.init(kind: .border, bounds: border, color: theme.quoteBorderColor.cgColor, strokeWidth: theme.quoteBorderWidth))
        }
        if let marker, let firstLineBounds {
            let markerX = textX - markerGutter
            let markerHeight = marker.ascent + marker.descent
            let markerTop = firstLineBounds.midY - markerHeight / 2
            let markerBaseline = markerTop + marker.ascent
            let markerBounds = CGRect(
                x: markerX,
                y: markerTop,
                width: marker.width,
                height: markerHeight
            )
            fragments.append(.init(kind: .marker, line: marker.line, origin: CGPoint(x: markerX, y: markerBaseline), bounds: markerBounds, color: theme.listMarkerColor.cgColor, label: marker.label, checked: marker.checked))
        }
        let seedBounds = CGRect(x: contentX, y: cursorY, width: contentWidth, height: max(0, totalEnd - cursorY))
        let bounds = fragments.reduce(seedBounds) { $0.union($1.bounds) }
        let prepared = PreparedProseBlock(fragments: fragments, bounds: bounds)
        var interactions: [PreparedProseInteraction] = []
        var interactionIndexBySemanticIndex: [Int: Int] = [:]
        for (semanticIndex, semantic) in attributed.semanticRanges.enumerated() {
            let rects = interactionRects[semanticIndex]
            guard !rects.isEmpty else { continue }
            let interaction: PreparedProseInteraction
            switch semantic {
            case let .link(_, href, text):
                interaction = PreparedProseInteraction(kind: .link, rects: rects, href: href, visibleText: text, docPos: nil, label: text, attrsJSON: nil)
            case let .mention(_, docPos, label, attrsJSON):
                interaction = PreparedProseInteraction(kind: .mention, rects: rects, href: nil, visibleText: label, docPos: docPos, label: label, attrsJSON: attrsJSON)
            }
            interactionIndexBySemanticIndex[semanticIndex] = interactions.count
            interactions.append(interaction)
        }
        var markerPending = block.listItemBoundary?.isFirstRenderableLeaf == true
            ? block.listContext.map { context in
                context.kind == "task" ? (context.checked ? "Checked" : "Unchecked") : "Item"
            }
            : nil
        let accessibilityNodes = attributed.accessibilityRanges.enumerated().compactMap { index, range -> PreparedProseAccessibilityNode? in
            let label = range.label.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !label.isEmpty else { return nil }
            let accessibleLabel: String
            if let marker = markerPending {
                markerPending = nil
                accessibleLabel = "\(marker), \(label)"
            } else {
                accessibleLabel = label
            }
            let interactionIndex: Int?
            let role: PreparedProseAccessibilityNode.Role
            let rects: [CGRect]
            switch range.role {
            case .text:
                interactionIndex = nil
                role = block.nodeType == "heading" || theme.headings[block.nodeType] != nil ? .heading : .text
                rects = accessibilityRects[index]
            case let .link(semanticIndex):
                interactionIndex = interactionIndexBySemanticIndex[semanticIndex]
                role = interactionIndex == nil ? .text : .link
                rects = interactionRects[semanticIndex]
            case let .mention(semanticIndex):
                interactionIndex = interactionIndexBySemanticIndex[semanticIndex]
                role = interactionIndex == nil ? .text : .mention
                rects = interactionRects[semanticIndex]
            }
            guard !rects.isEmpty else { return nil }
            return PreparedProseAccessibilityNode(
                interactionIndex: interactionIndex,
                role: role,
                label: accessibleLabel,
                rects: rects
            )
        }
        return (
            prepared,
            interactions,
            accessibilityNodes,
            nil,
            totalEnd + itemSpacing,
            256 + attributed.retainedBytes + prepared.estimatedRetainedBytes
        )
    }

    private func appendShapedRects(
        ranges: [(index: Int, range: NSRange)],
        line: CTLine,
        lineRange: NSRange,
        lineBounds: CGRect,
        textX: CGFloat,
        displayScale: CGFloat,
        rangeCursor: inout Int,
        to rects: inout [[CGRect]]
    ) {
        guard ranges.indices.contains(rangeCursor) else { return }
        while ranges.indices.contains(rangeCursor), ranges[rangeCursor].range.upperBound <= lineRange.location {
            rangeCursor += 1
        }
        guard ranges.indices.contains(rangeCursor) else { return }
        guard ranges[rangeCursor].range.location < lineRange.upperBound else { return }
        let glyphRuns = CTLineGetGlyphRuns(line) as? [CTRun] ?? []
        var index = rangeCursor
        while ranges.indices.contains(index), ranges[index].range.location < lineRange.upperBound {
            let geometryRange = ranges[index]
            let range = geometryRange.range
            var visualPieces: [(rect: CGRect, rightToLeft: Bool)] = []
            for run in glyphRuns {
                let stringRange = CTRunGetStringRange(run)
                let runRange = NSRange(location: stringRange.location, length: stringRange.length)
                let overlap = NSIntersectionRange(NSIntersectionRange(range, lineRange), runRange)
                guard overlap.length > 0 else { continue }
                let start = CGFloat(CTLineGetOffsetForStringIndex(line, overlap.location, nil))
                let end = CGFloat(CTLineGetOffsetForStringIndex(line, overlap.location + overlap.length, nil))
                visualPieces.append((
                    CGRect(
                        x: textX + min(start, end),
                        y: lineBounds.minY,
                        width: max(1 / displayScale, abs(end - start)),
                        height: lineBounds.height
                    ),
                    CTRunGetStatus(run).contains(.rightToLeft)
                ))
            }
            var priorDirection: Bool?
            for piece in visualPieces.sorted(by: { PreparedProseInteractionGeometry.visualOrder($0.rect, $1.rect) }) {
                PreparedProseInteractionGeometry.appendSameLinePiece(
                    piece.rect,
                    to: &rects[geometryRange.index],
                    mayMergeWithPrior: priorDirection == piece.rightToLeft
                )
                priorDirection = piece.rightToLeft
            }
            index += 1
        }
    }

    private func makeAttributedString(
        _ inlines: [ViewerInline],
        paint: PreparedTextPaint,
        theme: PreparedProseTheme,
        warningSemanticGeneration: String
    ) -> PreparedAttributedBlock {
        let result = NSMutableAttributedString()
        var atoms: [PreparedAtomSpec] = []
        var semanticRanges: [PreparedSemanticRange] = []
        var accessibilityRanges: [PreparedAccessibilityRange] = []

        func appendAccessibilityRange(_ range: NSRange, label: String, role: PreparedAccessibilityRange.Role) {
            guard range.length > 0 else { return }
            if let previous = accessibilityRanges.last,
               previous.role == role,
               previous.range.upperBound == range.location
            {
                accessibilityRanges[accessibilityRanges.count - 1] = PreparedAccessibilityRange(
                    range: NSRange(location: previous.range.location, length: previous.range.length + range.length),
                    label: previous.label + label,
                    role: role
                )
            } else {
                accessibilityRanges.append(PreparedAccessibilityRange(range: range, label: label, role: role))
            }
        }

        for inline in inlines {
            switch inline {
            case let .text(text: text, marks: marks):
                let start = result.length
                result.append(NSAttributedString(string: text, attributes: attributes(for: marks, paint: paint, theme: theme, warningSemanticGeneration: warningSemanticGeneration)))
                let range = NSRange(location: start, length: (text as NSString).length)
                if let href = href(in: marks), !text.isEmpty {
                    let semanticIndex: Int
                    if case let .link(previous, previousHref, previousText)? = semanticRanges.last,
                       previousHref == href, previous.upperBound == range.location {
                        semanticRanges[semanticRanges.count - 1] = .link(range: NSRange(location: previous.location, length: previous.length + range.length), href: href, text: previousText + text)
                        semanticIndex = semanticRanges.count - 1
                    } else {
                        semanticRanges.append(.link(range: range, href: href, text: text))
                        semanticIndex = semanticRanges.count - 1
                    }
                    appendAccessibilityRange(range, label: text, role: .link(semanticIndex: semanticIndex))
                } else {
                    appendAccessibilityRange(range, label: text, role: .text)
                }
            case let .atom(nodeType: nodeType, docPos: docPos, attrsJSON: attrsJSON, label: label):
                if nodeType == "hardBreak" || nodeType == "hard_break" {
                    let range = NSRange(location: result.length, length: 1)
                    result.append(NSAttributedString(string: "\n", attributes: baseAttributes(paint)))
                    appendAccessibilityRange(range, label: "\n", role: .text)
                    continue
                }
                let appearance = atomAppearance(
                    nodeType: nodeType,
                    attrsJSON: attrsJSON,
                    paint: paint,
                    theme: theme,
                    warningSemanticGeneration: warningSemanticGeneration
                )
                let displayLabel = label.isEmpty ? " " : label
                let labelLine = CTLineCreateWithAttributedString(
                    NSAttributedString(string: displayLabel, attributes: appearance.attributes)
                )
                var labelAscent: CGFloat = 0
                var labelDescent: CGFloat = 0
                var labelLeading: CGFloat = 0
                let labelWidth = CGFloat(CTLineGetTypographicBounds(labelLine, &labelAscent, &labelDescent, &labelLeading))
                let metrics = PreparedAtomMetrics(
                    width: max(paint.font.lineHeight, labelWidth + appearance.padding.left + appearance.padding.right),
                    ascent: labelAscent + appearance.padding.top,
                    descent: max(labelDescent, 2) + appearance.padding.bottom
                )
                let range = NSRange(location: result.length, length: 1)
                result.append(NSAttributedString(string: "\u{FFFC}", attributes: [
                    kCTRunDelegateAttributeName as NSAttributedString.Key: preparedAtomDelegate(metrics),
                    preparedAtomAttribute: nodeType,
                ]))
                atoms.append(
                    PreparedAtomSpec(
                        range: range,
                        nodeType: nodeType,
                        docPos: docPos,
                        label: displayLabel,
                        metrics: metrics,
                        line: labelLine,
                        appearance: appearance
                    )
                )
                if nodeType == "mention" {
                    semanticRanges.append(.mention(range: range, docPos: docPos, label: displayLabel, attrsJSON: attrsJSON))
                    appendAccessibilityRange(
                        range,
                        label: displayLabel,
                        role: .mention(semanticIndex: semanticRanges.count - 1)
                    )
                } else {
                    appendAccessibilityRange(range, label: displayLabel, role: .text)
                }
            }
        }
        // NSMutableAttributedString retains UTF-16 storage, attribute runs,
        // atom delegates, and copied labels. This scales with every character,
        // even when a narrow width turns it into many CTLines.
        let stringBytes = result.length.rendererSaturatingMultiply(4)
        let attributeBytes = max(1, result.length).rendererSaturatingMultiply(48)
        let atomBytes = atoms.reduce(0) { partial, atom in
            partial + 256 + atom.label.utf8.count.rendererSaturatingMultiply(2)
        }
        return PreparedAttributedBlock(
            string: result,
            atoms: atoms,
            semanticRanges: semanticRanges,
            accessibilityRanges: accessibilityRanges,
            retainedBytes: 256 + stringBytes + attributeBytes + atomBytes
        )
    }

    private func href(in marks: [FfiViewerMark]) -> String? {
        for mark in marks where mark.markType == "link" {
            if let href = jsonDictionary(mark.attrsJson)["href"] as? String, !href.isEmpty { return href }
        }
        return nil
    }

    private func attributes(for marks: [FfiViewerMark], paint: PreparedTextPaint, theme: PreparedProseTheme, warningSemanticGeneration: String) -> [NSAttributedString.Key: Any] {
        var linkTheme: EditorLinkTheme?
        var explicitForeground: UIColor?
        var background: UIColor?
        var fontFamily: String?
        var fontSize: CGFloat?
        var underline = false
        var useMonospace = false
        var strike = false
        var wantsBold = false
        var wantsItalic = false
        var hasLink = false
        for mark in marks {
            let values = jsonDictionary(mark.attrsJson)
            switch mark.markType {
            case "bold", "strong": wantsBold = true
            case "italic", "em": wantsItalic = true
            case "underline": underline = true
            case "strike", "strikethrough": strike = true
            case "code": useMonospace = true
            case "link":
                hasLink = true
                linkTheme = theme.link
                if let linkBackground = theme.link?.backgroundColor { background = linkBackground }
                underline = underline || (theme.link?.underline ?? true)
            case "textColor", "color", "foregroundColor":
                explicitForeground = EditorTheme.color(from: values["color"] ?? values["textColor"]) ?? explicitForeground
            case "highlight", "backgroundColor":
                background = EditorTheme.color(from: values["color"] ?? values["backgroundColor"]) ?? background
            case "textStyle", "font":
                if let family = values["fontFamily"] as? String, !family.isEmpty { fontFamily = family }
                if let markedSize = (values["fontSize"] as? NSNumber).map({ CGFloat(truncating: $0) }), markedSize.isFinite, markedSize > 0 {
                    fontSize = markedSize
                }
            default: break
            }
        }
        let linkStyle = linkTheme.map {
            EditorTextStyle(fontFamily: $0.fontFamily, fontSize: $0.fontSize, fontWeight: $0.fontWeight, fontStyle: $0.fontStyle)
        }
        var font = ViewerFontEnvironment.shared.resolveFont(
            style: linkStyle,
            fallback: paint.font,
            fontScale: theme.fontScale,
            semanticGeneration: warningSemanticGeneration
        )
        let scaledMarkSize = fontSize.map { $0 * theme.fontScale }
        if let fontFamily {
            font = ViewerFontEnvironment.shared.resolveFont(
                family: fontFamily,
                size: scaledMarkSize ?? font.pointSize,
                fallback: font,
                semanticGeneration: warningSemanticGeneration
            )
        }
        if let scaledMarkSize { font = font.withSize(scaledMarkSize) }
        var markTraits: UIFontDescriptor.SymbolicTraits = []
        if wantsBold { markTraits.insert(.traitBold) }
        if wantsItalic { markTraits.insert(.traitItalic) }
        if useMonospace {
            font = ViewerFontEnvironment.shared.resolveFont(
                family: "monospace",
                size: font.pointSize,
                fallback: font,
                additionalTraits: markTraits,
                semanticGeneration: warningSemanticGeneration
            )
        } else {
            font = ViewerFontEnvironment.shared.resolveFont(
                family: nil,
                size: font.pointSize,
                fallback: font,
                additionalTraits: markTraits,
                semanticGeneration: warningSemanticGeneration
            )
        }
        var attributes = baseAttributes(paint)
        // An explicit text-color mark is document content and therefore wins
        // over link-theme paint regardless of compiler mark ordering.
        let foreground = explicitForeground ?? (hasLink ? linkTheme?.color ?? UIColor.systemBlue : paint.color)
        attributes[kCTFontAttributeName as NSAttributedString.Key] = Self.coreTextFont(from: font)
        attributes[kCTForegroundColorAttributeName as NSAttributedString.Key] = foreground.cgColor
        if let background { attributes[kCTBackgroundColorAttributeName as NSAttributedString.Key] = background.cgColor }
        if underline { attributes[kCTUnderlineStyleAttributeName as NSAttributedString.Key] = NSNumber(value: CTUnderlineStyle.single.rawValue) }
        if strike { attributes[preparedStrikeAttribute] = NSNumber(value: true) }
        return attributes
    }

    private func strikeFragments(
        for line: CTLine,
        lineOrigin: CGPoint,
        displayScale: CGFloat
    ) -> [PreparedProseFragment] {
        let unit = displayScale.isFinite && displayScale > 0 ? 1 / displayScale : 1
        return (CTLineGetGlyphRuns(line) as? [CTRun] ?? []).compactMap { run in
            let attributes = CTRunGetAttributes(run) as? [NSAttributedString.Key: Any] ?? [:]
            guard (attributes[preparedStrikeAttribute] as? NSNumber)?.boolValue == true,
                  let colorValue = attributes[kCTForegroundColorAttributeName as NSAttributedString.Key]
            else { return nil }
            let color = colorValue as! CGColor

            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            var leading: CGFloat = 0
            let typographicWidth = CGFloat(CTRunGetTypographicBounds(run, CFRange(location: 0, length: 0), &ascent, &descent, &leading))
            let stringRange = CTRunGetStringRange(run)
            let start = CGFloat(CTLineGetOffsetForStringIndex(line, stringRange.location, nil))
            let end = CGFloat(CTLineGetOffsetForStringIndex(line, stringRange.location + stringRange.length, nil))
            let width = max(typographicWidth, abs(end - start))
            guard width.isFinite, width > 0, ascent.isFinite, ascent > 0 else { return nil }

            let thickness = max(unit, min(2, ascent * 0.08))
            let centerY = lineOrigin.y - ascent * 0.35
            return PreparedProseFragment(
                kind: .strike,
                bounds: CGRect(
                    x: lineOrigin.x + min(start, end),
                    y: centerY - thickness / 2,
                    width: width,
                    height: thickness
                ),
                color: color,
                strokeWidth: thickness
            )
        }
    }

    private func baseAttributes(_ paint: PreparedTextPaint) -> [NSAttributedString.Key: Any] {
        [
            kCTFontAttributeName as NSAttributedString.Key: Self.coreTextFont(from: paint.font),
            kCTForegroundColorAttributeName as NSAttributedString.Key: paint.color.cgColor,
        ]
    }

    private func makeListMarker(
        _ context: ViewerListContext,
        nestingDepth: Int,
        paint: PreparedTextPaint,
        theme: PreparedProseTheme
    ) -> PreparedListMarker {
        let scale = !context.ordered && context.kind != "task"
            ? max(0.01, theme.listMarkerScale)
            : 1
        let font = paint.font.withSize(max(1, paint.font.pointSize * scale))
        let label: String
        if context.kind == "task" {
            label = ""
        } else if context.ordered {
            label = OrderedListMarkerFormatter.label(
                index: UInt32(exactly: context.index) ?? 0,
                nestingDepth: nestingDepth,
                theme: theme.orderedListMarker
            )
        } else {
            label = "•"
        }
        guard !label.isEmpty else {
            let side = max(font.lineHeight, font.pointSize)
            return PreparedListMarker(line: nil, label: label, width: side, ascent: side * 0.75, descent: side * 0.25, checked: context.checked)
        }
        let line = CTLineCreateWithAttributedString(
            NSAttributedString(
                string: label,
                attributes: [
                    kCTFontAttributeName as NSAttributedString.Key: Self.coreTextFont(from: font),
                    kCTForegroundColorAttributeName as NSAttributedString.Key: theme.listMarkerColor.cgColor,
                ]
            )
        )
        var ascent: CGFloat = 0
        var descent: CGFloat = 0
        var leading: CGFloat = 0
        let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
        let imageBounds = CTLineGetImageBounds(line, nil)
        let visualAscent = imageBounds.isNull || imageBounds.isEmpty ? ascent : imageBounds.maxY
        let visualDescent = imageBounds.isNull || imageBounds.isEmpty ? descent : -imageBounds.minY
        return PreparedListMarker(line: line, label: label, width: max(1, width), ascent: visualAscent, descent: visualDescent, checked: context.checked)
    }

    private func atomAppearance(
        nodeType: String,
        attrsJSON: String,
        paint: PreparedTextPaint,
        theme: PreparedProseTheme,
        warningSemanticGeneration: String
    ) -> PreparedAtomAppearance {
        if nodeType == "mention" {
            let values = jsonDictionary(attrsJSON)
            let localMention = (values["mentionTheme"] as? [String: Any]).map(EditorMentionTheme.init(dictionary:))
            let mention = (theme.mention?.merged(with: localMention) ?? localMention)?.node
            var attributes = baseAttributes(paint)
            if let weight = mention?.fontWeight {
                let font = ViewerFontEnvironment.shared.resolveFont(
                    style: EditorTextStyle(fontWeight: weight),
                    fallback: paint.font,
                    fontScale: 1,
                    semanticGeneration: warningSemanticGeneration
                )
                attributes[kCTFontAttributeName as NSAttributedString.Key] = Self.coreTextFont(from: font)
            }
            attributes[kCTForegroundColorAttributeName as NSAttributedString.Key] = (mention?.textColor ?? paint.color).cgColor
            return PreparedAtomAppearance(
                attributes: attributes,
                background: mention?.backgroundColor ?? UIColor.systemBlue.withAlphaComponent(0.12),
                borderColor: mention?.borderColor,
                borderWidth: max(0, mention?.borderWidth ?? 0),
                radius: max(0, mention?.borderRadius ?? 6),
                padding: UIEdgeInsets(top: 4, left: 6, bottom: 4, right: 6)
            )
        }
        return PreparedAtomAppearance(
            attributes: baseAttributes(paint),
            background: UIColor.systemGray5,
            borderColor: nil,
            borderWidth: 0,
            radius: 5,
            padding: UIEdgeInsets(top: 4, left: 6, bottom: 4, right: 6)
        )
    }

    private func jsonDictionary(_ json: String) -> [String: Any] {
        guard let data = json.data(using: .utf8), let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return value
    }
}
