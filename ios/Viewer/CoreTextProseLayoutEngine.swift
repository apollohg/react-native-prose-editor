import CoreText
import UIKit

let preparedAtomAttribute = NSAttributedString.Key("PREPPreparedAtom")
/// Core Text has no strikethrough attribute. This marks a shaped run so its
/// immutable strike rectangle can be prepared from Core Text's own metrics.
let preparedStrikeAttribute = NSAttributedString.Key("PREPPreparedStrike")

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
        let highlightingRequest = theme.codeHighlighting.map { configuration in
            PreparedViewerHighlightingRequest(configuration: configuration, generation: key.semanticGenerationIdentity, blocks: document.blocks.enumerated().compactMap { index, block in
                guard EditorStyleSheet.element(block.nodeType) == "codeBlock" else { return nil }
                let text = block.inlines.compactMap { inline -> String? in
                    if case let .text(text, _) = inline { return text }
                    return nil
                }.joined()
                return NativeCodeHighlightBlock(start: index, text: text, language: block.language)
            })
        }
        let highlighting = highlightingRequest.flatMap { PreparedViewerHighlightingStore.result(for: $0.generation) }
        var cursorY = theme.contentInsets.top
        var blocks: [PreparedProseBlock] = []
        var containerBounds: [Int: CGRect] = [:]
        var containerStyles: [Int: (EditorStyleBox, Int)] = [:]
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
            let priorIds = Set(index > 0 ? document.blocks[index - 1].styleAncestors.map(\.identity) : [])
            let followingIds = Set(document.blocks.indices.contains(index + 1) ? document.blocks[index + 1].styleAncestors.map(\.identity) : [])
            let opening = block.styleAncestors.filter { !priorIds.contains($0.identity) }
            let closing = block.styleAncestors.filter { !followingIds.contains($0.identity) }
            if let sheet = theme.styleSheet, index > 0 {
                let previous = document.blocks[index - 1]
                let shared = zip(previous.styleAncestors, block.styleAncestors).prefix { $0 == $1 }.count
                let previousSibling = previous.styleAncestors.dropFirst(shared).first?.nodeType ?? previous.nodeType
                let nextSibling = block.styleAncestors.dropFirst(shared).first?.nodeType ?? block.nodeType
                let previousMargin = sheet.box(previousSibling).margin.bottom
                let nextMargin = sheet.box(nextSibling).margin.top
                cursorY -= previousMargin + nextMargin - EditorStyleSheet.collapsedMargin(previousMargin, nextMargin)
            }
            let omitBottomMargin = block.nodeType == "paragraph"
                && block.styleAncestors.last.map { $0.nodeType == "blockquote" && closing.contains($0) } == true
            let top = opening.reduce(CGFloat.zero) { $0 + (theme.styleSheet?.box($1.nodeType).outerInsets.top ?? 0) }
            let bottom = closing.reduce(CGFloat.zero) { $0 + (theme.styleSheet?.box($1.nodeType).outerInsets.bottom ?? 0) }
            cursorY += top
            let prepared = prepareBlock(
                block,
                highlighting: highlighting?.ranges[index] ?? [],
                attachmentOrdinal: imageAttachments.count,
                listMarker: listMarker,
                theme: theme,
                width: canonicalWidth,
                cursorY: cursorY,
                omitBottomMargin: omitBottomMargin,
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
            if let sheet = theme.styleSheet {
                var outerLeft: CGFloat = 0
                var outerRight: CGFloat = 0
                for (depth, ancestor) in block.styleAncestors.enumerated() {
                    let box = sheet.box(ancestor.nodeType)
                    let remaining = block.styleAncestors.dropFirst(depth + 1)
                    let innerTop = remaining.reduce(CGFloat.zero) { $0 + (opening.contains($1) ? sheet.box($1.nodeType).outerInsets.top : 0) }
                    let innerBottom = remaining.reduce(CGFloat.zero) { $0 + (closing.contains($1) ? sheet.box($1.nodeType).outerInsets.bottom : 0) }
                    let y = cursorY - innerTop - (opening.contains(ancestor) ? box.inset.top : 0)
                    let end = prepared.nextY + innerBottom + (closing.contains(ancestor) ? box.inset.bottom : 0)
                    let rect = CGRect(x: theme.contentInsets.left + outerLeft + box.margin.left, y: y,
                                      width: max(1, canonicalWidth - theme.contentInsets.left - theme.contentInsets.right - outerLeft - outerRight - box.margin.left - box.margin.right), height: max(0, end - y))
                    containerBounds[ancestor.identity] = containerBounds[ancestor.identity].map { $0.union(rect) } ?? rect
                    containerStyles[ancestor.identity] = (box, depth)
                    outerLeft += box.outerInsets.left
                    outerRight += box.outerInsets.right
                }
            }
            cursorY = prepared.nextY + bottom
            retainedBytes += prepared.retainedBytes
        }
        let renderedBottom = blocks.map(\.bounds.maxY).max() ?? cursorY
        cursorY = (theme.styleSheet == nil ? renderedBottom : max(cursorY, renderedBottom)) + theme.contentInsets.bottom
        var decorations = containerBounds.keys.sorted { (containerStyles[$0]?.1 ?? 0) < (containerStyles[$1]?.1 ?? 0) }.compactMap { identity -> PreparedProseFragment? in
            guard let bounds = containerBounds[identity], let style = containerStyles[identity]?.0 else { return nil }
            return PreparedProseFragment(kind: .background, bounds: bounds, styleBox: style)
        }
        if let sheet = theme.styleSheet {
            decorations.insert(PreparedProseFragment(kind: .background, bounds: CGRect(x: 0, y: 0, width: canonicalWidth, height: cursorY), styleBox: sheet.box("content")), at: 0)
        }
        retainedBytes += decorations.count * 512 + (highlightingRequest?.retainedBytes ?? 0) + (highlighting?.retainedBytes ?? 0)
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
            retainedBytes: retainedBytes,
            decorations: decorations,
            highlightingRequest: highlightingRequest,
            highlightingResolved: highlighting != nil
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
        highlighting: [NativeCodeHighlightRange],
        attachmentOrdinal: Int,
        listMarker: PreparedListMarker?,
        theme: PreparedProseTheme,
        width: CGFloat,
        cursorY: CGFloat,
        omitBottomMargin: Bool,
        disappearingListItemIdentities: Set<Int>,
        displayScale: CGFloat,
        warningSemanticGeneration: String
    ) -> (block: PreparedProseBlock, interactions: [PreparedProseInteraction], accessibilityNodes: [PreparedProseAccessibilityNode], attachment: ViewerImageAttachment?, nextY: CGFloat, retainedBytes: Int) {
        let sheet = theme.styleSheet
        let box = sheet?.box(block.nodeType) ?? EditorStyleBox()
        let ancestors = block.styleAncestors.reduce(UIEdgeInsets.zero) { $0.adding(sheet?.box($1.nodeType).outerInsets ?? .zero) }
        let cursorY = cursorY + box.margin.top
        let contentX = theme.contentInsets.left + ancestors.left + box.margin.left
        let contentWidth = max(1, width - theme.contentInsets.left - theme.contentInsets.right - ancestors.left - ancestors.right - box.margin.left - box.margin.right)
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
        let listName = block.listContext.map { $0.kind == "task" ? "taskList" : $0.ordered ? "orderedList" : "bulletList" } ?? "bulletList"
        let listValues = sheet?[listName] ?? [:]
        let listIndent = EditorTheme.cgFloat(listValues["indent"]) ?? theme.listIndent
        let baseMultiplier = EditorTheme.cgFloat(listValues["baseIndentMultiplier"]) ?? theme.listBaseIndentMultiplier
        let listBaseIndent = block.listContext == nil ? 0 : max(0, listIndent * baseMultiplier)
        let nestedListIndent = block.listContext == nil ? 0 : max(0, listIndent * CGFloat(listDepth))
        let checkbox = block.listContext.flatMap { $0.kind == "task" ? sheet?.checkbox(checked: $0.checked) : nil }
        let markerGap = checkbox?.number("gap", fallback: 8) ?? theme.listMarkerGap
        let markerGutter = measuredListMarker.map { max(markerGap, $0.width + markerGap) } ?? 0
        let listInset = listBaseIndent + nestedListIndent + markerGutter
        let quoteInset = block.inBlockquote ? theme.quoteBorderWidth + theme.quoteMarkerGap + theme.quoteIndent : 0
        let codeInset = block.nodeType == "codeBlock" ? theme.codePaddingHorizontal : 0
        let textX = contentX + listInset + quoteInset + codeInset + box.inset.left
        let itemSpacing: CGFloat
        if sheet != nil {
            itemSpacing = omitBottomMargin ? 0 : box.margin.bottom
        } else if block.listContext == nil {
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
        if block.isBlockAtom, let atoms = theme.viewerAtoms,
           atoms.nodeTypes.contains(block.nodeType),
           case let .atom(nodeType, docPos, attrsJSON, _)? = block.inlines.first {
            let slotWidth = max(1, contentWidth - listInset - quoteInset)
            let height = atoms.height(nodeType: nodeType, docPos: docPos, width: slotWidth)
            let bounds = CGRect(x: textX, y: cursorY, width: slotWidth, height: height)
            var fragments: [PreparedProseFragment] = []
            if block.inBlockquote {
                fragments.append(.init(kind: .border, bounds: CGRect(x: contentX, y: cursorY,
                    width: theme.quoteBorderWidth, height: height), color: theme.quoteBorderColor.cgColor,
                    strokeWidth: theme.quoteBorderWidth))
            }
            if let marker {
                let markerX = textX - markerGutter
                let markerTop = cursorY + max(0, (height - marker.ascent - marker.descent) / 2)
                let markerBounds = CGRect(x: markerX, y: markerTop, width: marker.width,
                                          height: marker.ascent + marker.descent)
                fragments.append(.init(kind: .marker, line: marker.line,
                    origin: CGPoint(x: markerX, y: markerTop + marker.ascent), bounds: markerBounds,
                    color: theme.listMarkerColor.cgColor, label: marker.label, checked: marker.checked, styleBox: checkbox))
            }
            let blockBounds = fragments.reduce(bounds) { $0.union($1.bounds) }
            let prepared = PreparedProseBlock(fragments: fragments, bounds: blockBounds,
                atomSlot: PreparedProseAtomSlot(nodeType: nodeType, docPos: docPos, attrsJSON: attrsJSON, bounds: bounds))
            return (prepared, [], [], nil, blockBounds.maxY + itemSpacing, prepared.estimatedRetainedBytes)
        }
        if block.nodeType == "image", let image = ViewerImageAttachment.sourceAndDeclaredSize(in: block) {
            let availableImageWidth = max(1, contentWidth - listInset - quoteInset - box.inset.left - box.inset.right)
            let imageWidth = sheet == nil ? availableImageWidth : min(availableImageWidth, image.declaredSize?.width ?? availableImageWidth)
            let provisionalHeight = max(44, min(240, imageWidth * 0.56))
            let declared = image.declaredSize
            let resolvedSize = declared ?? ViewerImageIntrinsicStore.shared.size(for: image.id)
            let height = resolvedSize.map { imageWidth * $0.height / max(1, $0.width) } ?? provisionalHeight
            let bounds = CGRect(x: textX - box.inset.left, y: cursorY, width: imageWidth + box.inset.left + box.inset.right, height: height + box.inset.top + box.inset.bottom)
            let attachment = ViewerImageAttachment(ordinal: attachmentOrdinal, id: image.id, source: image.source, bounds: bounds, declaredSize: declared)
            let fragments = [PreparedProseFragment(kind: .image, bounds: bounds, color: UIColor.systemGray5.cgColor, styleBox: sheet == nil ? nil : box)]
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
            let ruleX = contentX + listInset + quoteInset + box.inset.left
            let ruleWidth = max(1, contentWidth - listInset - quoteInset - box.inset.left - box.inset.right)
            let y = cursorY + theme.ruleMargin + box.inset.top
            let rule = CGRect(x: ruleX, y: y, width: ruleWidth, height: theme.ruleThickness)
            var fragments: [PreparedProseFragment] = [.init(kind: .rule, bounds: rule, color: theme.ruleColor.cgColor, strokeWidth: theme.ruleThickness)]
            let totalEnd = y + theme.ruleThickness + theme.ruleMargin + box.inset.bottom
            if sheet != nil {
                fragments = [.init(kind: .background, bounds: CGRect(x: contentX, y: cursorY, width: contentWidth, height: totalEnd - cursorY), styleBox: box)]
            }
            if block.inBlockquote {
                fragments.append(.init(kind: .border, bounds: CGRect(x: contentX, y: cursorY, width: theme.quoteBorderWidth, height: totalEnd - cursorY), color: theme.quoteBorderColor.cgColor, strokeWidth: theme.quoteBorderWidth))
            }
            if let marker {
                let markerX = textX - markerGutter
                let markerHeight = marker.ascent + marker.descent
                let markerTop = cursorY + (totalEnd - cursorY - markerHeight) / 2
                let markerBaseline = markerTop + marker.ascent
                let markerBounds = CGRect(x: markerX, y: markerTop, width: marker.width, height: markerHeight)
                fragments.append(.init(kind: .marker, line: marker.line, origin: CGPoint(x: markerX, y: markerBaseline), bounds: markerBounds, color: theme.listMarkerColor.cgColor, label: marker.label, checked: marker.checked, styleBox: checkbox))
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

        let availableWidth = max(1, contentWidth - listInset - quoteInset - codeInset * 2 - box.inset.left - box.inset.right)
        let attributed = makeAttributedString(block.inlines, paint: paint, theme: theme, warningSemanticGeneration: warningSemanticGeneration)
        let highlighted = NSMutableAttributedString(attributedString: attributed.string)
        NativeCodeHighlightPresentation.apply(highlighting, to: highlighted)
        let typesetter = CTTypesetterCreateWithAttributedString(highlighted)
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
        let codeTopInset = (block.nodeType == "codeBlock" ? theme.codePaddingVertical : 0) + box.inset.top
        let firstLineHeight = max(paint.font.lineHeight, paint.lineHeight ?? 0)
        let markerTopProtection = marker.map {
            max(0, ($0.ascent + $0.descent - firstLineHeight) / 2 - codeTopInset)
        } ?? 0
        var textTop = cursorY + codeTopInset + (cursorY == theme.contentInsets.top ? markerTopProtection : 0)
        var firstLineBounds: CGRect?
        while location < attributed.string.length {
            let suggested = CTTypesetterSuggestLineBreak(typesetter, location, availableWidth)
            let count = max(1, suggested)
            let shapedLine = CTTypesetterCreateLine(typesetter, CFRange(location: location, length: count))
            let shouldJustify = paint.textValues["textAlign"] as? String == "justify" && location + count < attributed.string.length && (attributed.string.string as NSString).character(at: location + count - 1) != 10
            let line = shouldJustify ? CTLineCreateJustifiedLine(shapedLine, 1, availableWidth) ?? shapedLine : shapedLine
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            var leading: CGFloat = 0
            let lineWidth = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
            let naturalHeight = ascent + descent + leading
            var requestedHeight = paint.lineHeight ?? 0
            attributed.string.enumerateAttribute(editorInlineLineHeightAttribute, in: NSRange(location: location, length: count)) { value, _, _ in
                requestedHeight = max(requestedHeight, EditorTheme.cgFloat(value) ?? 0)
            }
            let lineHeight = max(naturalHeight, requestedHeight)
            let baseline = textTop + (lineHeight - naturalHeight) / 2 + ascent
            let alignment = paint.textValues["textAlign"] as? String
            let lineTextX = textX + (alignment == "center" ? max(0, availableWidth - lineWidth) / 2 : alignment == "right" ? max(0, availableWidth - lineWidth) : 0)
            let lineBounds = CGRect(x: lineTextX, y: textTop, width: min(availableWidth, max(0, lineWidth)), height: lineHeight)
            fragments.append(contentsOf: inlineBackgroundFragments(for: line, bounds: lineBounds))
            let lineRange = NSRange(location: location, length: count)
            fragments.append(.init(kind: .text, line: line, origin: CGPoint(x: lineTextX, y: baseline), bounds: lineBounds))
            fragments.append(contentsOf: strikeFragments(
                for: line,
                lineOrigin: CGPoint(x: lineTextX, y: baseline),
                displayScale: displayScale
            ))
            appendShapedRects(
                ranges: semanticGeometryRanges,
                line: line,
                lineRange: lineRange,
                lineBounds: lineBounds,
                textX: lineTextX,
                displayScale: displayScale,
                rangeCursor: &semanticGeometryCursor,
                to: &interactionRects
            )
            appendShapedRects(
                ranges: accessibilityGeometryRanges,
                line: line,
                lineRange: lineRange,
                lineBounds: lineBounds,
                textX: lineTextX,
                displayScale: displayScale,
                rangeCursor: &accessibilityGeometryCursor,
                to: &accessibilityRects
            )
            if firstLineBounds == nil { firstLineBounds = lineBounds }
            for atom in attributed.atoms where NSIntersectionRange(atom.range, lineRange).length > 0 {
                let offset = CGFloat(CTLineGetOffsetForStringIndex(line, atom.range.location, nil))
                let atomBounds = CGRect(
                    x: lineTextX + offset,
                    y: baseline - atom.metrics.ascent,
                    width: atom.metrics.width,
                    height: atom.metrics.ascent + atom.metrics.descent
                )
                fragments.append(contentsOf: strikeFragments(for: atom.line, lineOrigin: CGPoint(x: atomBounds.minX + atom.appearance.padding.left, y: baseline), displayScale: displayScale))
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
                        label: atom.label,
                        styleBox: atom.appearance.styleBox
                    )
                )
            }
            location += count
            textTop += lineHeight
        }
        if fragments.isEmpty {
            let fallbackHeight = paint.lineHeight ?? paint.font.lineHeight
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: "\u{200B}", attributes: baseAttributes(paint)))
            let alignment = paint.textValues["textAlign"] as? String
            let lineTextX = textX + (alignment == "center" ? max(0, availableWidth) / 2 : alignment == "right" ? max(0, availableWidth) : 0)
            let lineBounds = CGRect(x: lineTextX, y: textTop, width: 0, height: fallbackHeight)
            fragments.append(.init(kind: .text, line: line, origin: CGPoint(x: textX, y: textTop + paint.font.ascender), bounds: lineBounds))
            firstLineBounds = lineBounds
            textTop += fallbackHeight
        }
        let textEnd = textTop
        let totalEnd = textEnd + (block.nodeType == "codeBlock" ? theme.codePaddingVertical : 0) + box.inset.bottom
        let blockRect = CGRect(x: contentX, y: cursorY, width: contentWidth, height: max(0, totalEnd - cursorY))
        if sheet != nil {
            fragments.insert(.init(kind: .background, bounds: blockRect, styleBox: box), at: 0)
        } else if block.nodeType == "codeBlock" {
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
            fragments.append(.init(kind: .marker, line: marker.line, origin: CGPoint(x: markerX, y: markerBaseline), bounds: markerBounds, color: theme.listMarkerColor.cgColor, label: marker.label, checked: marker.checked, styleBox: checkbox))
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

}
