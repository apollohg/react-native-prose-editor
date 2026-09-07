import UIKit
import ImageIO
import CryptoKit

extension RenderBridge {
    static func attributedStringApplyingLeadingTopLevelChildIndexIfNeeded(
        _ attributedString: NSAttributedString,
        topLevelChildIndex: Int?,
        resultIsEmpty: Bool
    ) -> NSAttributedString {
        guard resultIsEmpty,
              let topLevelChildIndex,
              attributedString.length > 0
        else {
            return attributedString
        }

        let tagged = NSMutableAttributedString(attributedString: attributedString)
        let firstComposedCharacterRange = (tagged.string as NSString)
            .rangeOfComposedCharacterSequence(at: 0)
        tagged.addAttribute(
            RenderBridgeAttributes.topLevelChildIndex,
            value: NSNumber(value: topLevelChildIndex),
            range: firstComposedCharacterRange
        )
        return tagged
    }

    static func removingLeadingTopLevelChildIndex(
        from attributedString: NSAttributedString,
        topLevelChildIndex: Int
    ) -> NSAttributedString {
        guard attributedString.length > 0 else { return attributedString }

        let firstValue = attributedString.attribute(
            RenderBridgeAttributes.topLevelChildIndex,
            at: 0,
            effectiveRange: nil
        ) as? NSNumber
        guard firstValue?.intValue == topLevelChildIndex else {
            return attributedString
        }

        let adjusted = NSMutableAttributedString(attributedString: attributedString)
        var effectiveRange = NSRange(location: 0, length: 0)
        adjusted.attribute(
            RenderBridgeAttributes.topLevelChildIndex,
            at: 0,
            longestEffectiveRange: &effectiveRange,
            in: NSRange(location: 0, length: adjusted.length)
        )
        adjusted.removeAttribute(
            RenderBridgeAttributes.topLevelChildIndex,
            range: effectiveRange
        )
        return adjusted
    }

    static func effectiveBlockContext(_ blockStack: [BlockContext]) -> BlockContext? {
        guard let currentBlock = blockStack.last else { return nil }
        if currentBlock.listContext != nil {
            return currentBlock
        }
        guard let inheritedListBlock = nearestListBlock(in: Array(blockStack.dropLast())) else {
            return currentBlock
        }
        return BlockContext(
            nodeType: currentBlock.nodeType,
            depth: currentBlock.depth,
            listContext: inheritedListBlock.listContext,
            listMarkerContext: currentBlock.listMarkerContext,
            markerPending: false
        )
    }

    private static func nearestListBlock(in contexts: [BlockContext]) -> BlockContext? {
        for context in contexts.reversed() where context.listContext != nil {
            return context
        }
        return nil
    }

    static func trailingRenderedContentHasBlockquote(
        in result: NSAttributedString
    ) -> Bool {
        guard result.length > 0 else { return false }
        let nsString = result.string as NSString

        for index in stride(from: result.length - 1, through: 0, by: -1) {
            let scalar = nsString.character(at: index)
            if scalar == 0x000A || scalar == 0x000D {
                continue
            }
            return result.attribute(
                RenderBridgeAttributes.blockquoteBorderColor,
                at: index,
                effectiveRange: nil
            ) != nil
        }

        return false
    }

    static func consumePendingListMarker(from blockStack: inout [BlockContext]) -> [String: Any]? {
        guard blockStack.count >= 2 else { return nil }
        for idx in stride(from: blockStack.count - 2, through: 0, by: -1) {
            guard blockStack[idx].markerPending else { continue }
            blockStack[idx].markerPending = false
            return blockStack[idx].listContext
        }
        return nil
    }

    static func isListItemNodeType(_ nodeType: String) -> Bool {
        EditorNodeTypes.isListItem(nodeType)
    }

    static func overrideTrailingParagraphSpacing(
        in result: NSMutableAttributedString,
        paragraphSpacing: CGFloat
    ) {
        guard result.length > 0 else { return }

        let nsString = result.string as NSString
        let paragraphRange = nsString.paragraphRange(for: NSRange(location: result.length - 1, length: 0))
        result.enumerateAttribute(
            .paragraphStyle,
            in: paragraphRange,
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, _ in
            let sourceStyle = (value as? NSParagraphStyle)?.mutableCopy() as? NSMutableParagraphStyle
                ?? NSMutableParagraphStyle()
            sourceStyle.paragraphSpacing = paragraphSpacing
            result.addAttribute(.paragraphStyle, value: sourceStyle, range: range)
        }
    }

    static func collapseTrailingSpacingBeforeHorizontalRuleIfNeeded(
        in result: NSMutableAttributedString,
        pendingParagraphSpacing: inout CGFloat?,
        nodeType: String,
        theme: EditorTheme?
    ) {
        guard EditorNodeTypes.isHorizontalRule(nodeType) else { return }
        let horizontalRuleMargin = resolvedHorizontalRuleVerticalMargin(theme: theme)

        if let pendingSpacing = pendingParagraphSpacing {
            pendingParagraphSpacing = collapsedSpacing(
                existingSpacing: pendingSpacing,
                adjacentHorizontalRuleMargin: horizontalRuleMargin
            )
            return
        }

        guard let trailingParagraphSpacing = trailingParagraphSpacing(in: result) else { return }
        let adjustedSpacing = collapsedSpacing(
            existingSpacing: trailingParagraphSpacing,
            adjacentHorizontalRuleMargin: horizontalRuleMargin
        )
        guard abs(adjustedSpacing - trailingParagraphSpacing) > 0.01 else { return }
        overrideTrailingParagraphSpacing(in: result, paragraphSpacing: adjustedSpacing)
    }

    static func collapsedParagraphSpacingAfterHorizontalRule(
        in result: NSAttributedString,
        separatorBlockStack: [BlockContext],
        theme: EditorTheme?,
        baseFont: UIFont
    ) -> CGFloat? {
        guard let horizontalRuleMargin = trailingHorizontalRuleMargin(in: result),
              let separatorSpacing = separatorParagraphSpacing(
                  for: separatorBlockStack,
                  theme: theme,
                  baseFont: baseFont
              )
        else {
            return nil
        }

        return collapsedSpacing(
            existingSpacing: separatorSpacing,
            adjacentHorizontalRuleMargin: horizontalRuleMargin
        )
    }

    @discardableResult
    static func applyPendingTrailingParagraphSpacing(
        in result: NSMutableAttributedString,
        pendingParagraphSpacing: inout CGFloat?
    ) -> Bool {
        guard let paragraphSpacing = pendingParagraphSpacing else { return false }
        overrideTrailingParagraphSpacing(in: result, paragraphSpacing: paragraphSpacing)
        pendingParagraphSpacing = nil
        return true
    }

    private static func trailingParagraphSpacing(in result: NSAttributedString) -> CGFloat? {
        guard result.length > 0 else { return nil }

        let nsString = result.string as NSString
        let paragraphRange = nsString.paragraphRange(for: NSRange(location: result.length - 1, length: 0))
        var spacing: CGFloat? = nil
        result.enumerateAttribute(
            .paragraphStyle,
            in: paragraphRange,
            options: [.reverse, .longestEffectiveRangeNotRequired]
        ) { value, _, stop in
            if let paragraphStyle = value as? NSParagraphStyle {
                spacing = paragraphStyle.paragraphSpacing
                stop.pointee = true
            }
        }
        return spacing
    }

    private static func separatorParagraphSpacing(
        for blockStack: [BlockContext],
        theme: EditorTheme?,
        baseFont: UIFont
    ) -> CGFloat? {
        guard let currentBlock = effectiveBlockContext(blockStack) else { return nil }
        return paragraphStyleForBlock(
            currentBlock,
            blockStack: blockStack,
            theme: theme,
            baseFont: baseFont
        ).paragraphSpacing
    }

    private static func trailingHorizontalRuleMargin(in result: NSAttributedString) -> CGFloat? {
        guard result.length > 0 else { return nil }
        let nsString = result.string as NSString

        for index in stride(from: result.length - 1, through: 0, by: -1) {
            let scalar = nsString.character(at: index)
            if scalar == 0x000A || scalar == 0x000D {
                continue
            }
            guard let nodeType = result.attribute(
                RenderBridgeAttributes.voidNodeType,
                at: index,
                effectiveRange: nil
            ) as? String, EditorNodeTypes.isHorizontalRule(nodeType) else {
                return nil
            }
            return (
                result.attribute(.attachment, at: index, effectiveRange: nil)
                    as? HorizontalRuleAttachment
            )?.verticalPadding
        }

        return nil
    }

    static func resolvedHorizontalRuleVerticalMargin(theme: EditorTheme?) -> CGFloat {
        theme?.horizontalRule?.verticalMargin ?? LayoutConstants.horizontalRuleVerticalPadding
    }

    private static func collapsedSpacing(
        existingSpacing: CGFloat,
        adjacentHorizontalRuleMargin: CGFloat
    ) -> CGFloat {
        max(existingSpacing, adjacentHorizontalRuleMargin) - adjacentHorizontalRuleMargin
    }

    static func appendTrailingLineBreakPlaceholderIfNeeded(
        in result: NSMutableAttributedString,
        endedBlock: BlockContext,
        remainingBlockStack: [BlockContext],
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme?
    ) {
        guard result.length > 0 else { return }
        guard !isListItemNodeType(endedBlock.nodeType) else { return }
        let nodeType = result.attribute(
            RenderBridgeAttributes.voidNodeType,
            at: result.length - 1,
            effectiveRange: nil
        ) as? String
        let isHardBreak = nodeType.map(EditorNodeTypes.isHardBreak) ?? false
        let isCodeNewline = EditorStyleSheet.element(endedBlock.nodeType) == "codeBlock"
            && (result.string as NSString).character(at: result.length - 1) == 0x000A
        guard isHardBreak || isCodeNewline else { return }

        let placeholderBlockStack = remainingBlockStack + [endedBlock]
        let blockFont = resolvedFont(
            for: placeholderBlockStack,
            baseFont: baseFont,
            theme: theme
        )
        let blockColor = resolvedTextColor(
            for: placeholderBlockStack,
            textColor: textColor,
            theme: theme
        )
        var attrs = defaultAttributes(baseFont: blockFont, textColor: blockColor)
        attrs[RenderBridgeAttributes.syntheticPlaceholder] = true
        var styledAttrs = applyBlockStyle(
            to: attrs,
            blockStack: placeholderBlockStack,
            theme: theme,
            blockBaseFont: blockFont
        )
        if let paragraphStyle = (styledAttrs[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy()
            as? NSMutableParagraphStyle
        {
            paragraphStyle.paragraphSpacing = 0
            styledAttrs[.paragraphStyle] = paragraphStyle
        }
        result.append(NSAttributedString(string: "\u{200B}", attributes: styledAttrs))
    }

}
