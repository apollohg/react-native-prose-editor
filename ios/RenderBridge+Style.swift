import UIKit
import ImageIO
import CryptoKit

extension RenderBridge {
    /// Create a paragraph style for a block context.
    ///
    /// Applies indentation based on depth and list context. List items get
    /// a hanging indent so the bullet/number sits in the margin.
    static func paragraphStyleForBlock(
        _ context: BlockContext,
        blockStack: [BlockContext],
        theme: EditorTheme? = nil,
        baseFont: UIFont = .systemFont(ofSize: 16)
    ) -> NSMutableParagraphStyle {
        let style = NSMutableParagraphStyle()
        if let sheet = theme?.styleSheet {
            let ancestors = blockStack.dropLast().map(\.nodeType)
            let text = sheet.textStyle(context.nodeType, ancestors: ancestors)
            let horizontal = blockStack.reduce(UIEdgeInsets.zero) { $0.adding(sheet.box($1.nodeType).outerInsets) }
            let listName = context.listContext.map { ($0["kind"] as? String) == "task" ? "taskList" : (($0["ordered"] as? NSNumber)?.boolValue == true ? "orderedList" : "bulletList") }
            let list = listName.map { sheet[$0] } ?? [:]
            let indent = EditorTheme.cgFloat(list["indent"]) ?? LayoutConstants.indentPerDepth
            let multiplier = EditorTheme.cgFloat(list["baseIndentMultiplier"]) ?? 1
            let listDepth = max(0, blockStack.filter { $0.listContext != nil }.count - 1)
            let listInset = listName == nil ? 0 : indent * (CGFloat(listDepth) + multiplier) + listMarkerWidth(for: context, theme: theme, baseFont: baseFont)
            style.headIndent = horizontal.left + listInset
            style.firstLineHeadIndent = style.headIndent
            style.tailIndent = -horizontal.right
            if let height = text.lineHeight { style.minimumLineHeight = height; style.maximumLineHeight = height }
            let values = sheet.textValues(context.nodeType, ancestors: ancestors)
            switch values["textAlign"] as? String {
            case "center": style.alignment = .center
            case "right": style.alignment = .right
            case "justify": style.alignment = .justified
            default: break
            }
            return style
        }

        let blockStyle = theme?.effectiveTextStyle(
            for: context.nodeType,
            inBlockquote: blockquoteDepth(in: blockStack) > 0
        )
        let spacing = blockStyle?.spacingAfter
            ?? (context.listContext != nil ? theme?.list?.itemSpacing : nil)
            ?? LayoutConstants.paragraphSpacing
        style.paragraphSpacing = spacing

        let indentPerDepth = theme?.list?.indent ?? LayoutConstants.indentPerDepth
        let markerWidth = listMarkerWidth(for: context, theme: theme, baseFont: baseFont)
        let quoteDepth = CGFloat(blockquoteDepth(in: blockStack))
        let quoteIndent = max(
            theme?.blockquote?.indent ?? LayoutConstants.blockquoteIndent,
            (theme?.blockquote?.markerGap ?? LayoutConstants.blockquoteMarkerGap)
                + (theme?.blockquote?.borderWidth ?? LayoutConstants.blockquoteBorderWidth)
        )
        let listBaseIndentMultiplier = max(theme?.list?.baseIndentMultiplier ?? 1, 0)
        let listBaseIndentAdjustment = context.listContext != nil
            ? ((listBaseIndentMultiplier - 1) * indentPerDepth)
            : 0
        let columnsDepth = CGFloat(columnContainerDepth(in: blockStack))
        let baseIndent = (CGFloat(context.depth) * indentPerDepth)
            - (quoteDepth * indentPerDepth)
            - (columnsDepth * indentPerDepth)
            + listBaseIndentAdjustment
            + (quoteDepth * quoteIndent)

        if context.listContext != nil {
            // List item: reserve a fixed gutter and align all wrapped lines to
            // the text start since the marker is drawn separately.
            style.firstLineHeadIndent = baseIndent + markerWidth
            style.headIndent = baseIndent + markerWidth
        } else {
            style.firstLineHeadIndent = baseIndent
            style.headIndent = baseIndent
        }

        if context.nodeType == "codeBlock" {
            let horizontalPadding = theme?.codeBlock?.paddingHorizontal ?? 12
            style.firstLineHeadIndent += horizontalPadding
            style.headIndent += horizontalPadding
            style.tailIndent = -horizontalPadding
        }

        if let lineHeight = blockStyle?.lineHeight {
            style.minimumLineHeight = lineHeight
            style.maximumLineHeight = lineHeight
        }

        return style
    }

    /// Generate the list marker string (bullet or number) from a list context.
    static func listMarkerString(listContext: [String: Any]) -> String {
        if (listContext["kind"] as? String) == "task" {
            let checked = (listContext["checked"] as? NSNumber)?.boolValue ?? false
            return checked ? "\u{2611} " : "\u{2610} "
        }
        let ordered = (listContext["ordered"] as? NSNumber)?.boolValue ?? false

        if ordered {
            guard let rawIndex = listContext["index"] else { return "1. " }
            guard let index = v2ExactUInt32(rawIndex as? NSNumber) else { return "" }
            return "\(index). "
        } else {
            return LayoutConstants.unorderedListBullet
        }
    }

    /// Extract a `UInt32` from a JSON value produced by `JSONSerialization`.
    static func jsonUInt32(_ value: Any?) -> UInt32? {
        v2ExactUInt32(value as? NSNumber)
    }

    /// Extract a `UInt8` from a JSON value produced by `JSONSerialization`.
    static func jsonUInt8(_ value: Any?) -> UInt8 {
        if let number = value as? NSNumber {
            return number.uint8Value
        }
        return 0
    }

    static func jsonInt(_ value: Any?) -> Int? {
        if let number = value as? NSNumber {
            return number.intValue
        }
        if let string = value as? String,
           let resolved = Int(string.trimmingCharacters(in: .whitespacesAndNewlines))
        {
            return resolved
        }
        return nil
    }

    /// Extract a positive `CGFloat` from a JSON value produced by `JSONSerialization`.
    static func jsonCGFloat(_ value: Any?) -> CGFloat? {
        if let number = value as? NSNumber {
            let resolved = CGFloat(truncating: number)
            return resolved > 0 ? resolved : nil
        }
        if let string = value as? String,
           let resolved = Double(string.trimmingCharacters(in: .whitespacesAndNewlines)),
           resolved > 0
        {
            return CGFloat(resolved)
        }
        return nil
    }

    static func defaultAttributes(
        baseFont: UIFont,
        textColor: UIColor
    ) -> [NSAttributedString.Key: Any] {
        [
            .font: baseFont,
            .foregroundColor: textColor,
        ]
    }

    @discardableResult
    static func applyBlockStyle(
        to attrs: [NSAttributedString.Key: Any],
        blockStack: [BlockContext],
        theme: EditorTheme?,
        blockBaseFont: UIFont? = nil
    ) -> [NSAttributedString.Key: Any] {
        guard let currentBlock = effectiveBlockContext(blockStack) else { return attrs }
        var mutableAttrs = attrs
        if theme?.styleSheet != nil { mutableAttrs[editorStyledContentAttribute] = true }
        let renderedFont = mutableAttrs[.font] as? UIFont ?? .systemFont(ofSize: 16)
        let paragraphBaseFont = blockBaseFont ?? renderedFont
        mutableAttrs[.paragraphStyle] = paragraphStyleForBlock(
            currentBlock,
            blockStack: blockStack,
            theme: theme,
            baseFont: paragraphBaseFont
        )
        mutableAttrs[RenderBridgeAttributes.blockNodeType] = currentBlock.nodeType
        mutableAttrs[RenderBridgeAttributes.blockDepth] = currentBlock.depth
        if let listContext = currentBlock.listContext {
            mutableAttrs[RenderBridgeAttributes.listContext] = listContext
        }
        if let markerContext = currentBlock.listMarkerContext {
            mutableAttrs[RenderBridgeAttributes.listMarkerContext] = markerContext
            let visualListDepth = max(0, blockStack.filter { $0.listContext != nil }.count - 1)
            if (markerContext["kind"] as? String) != "task",
               (markerContext["ordered"] as? NSNumber)?.boolValue == true,
               let rawIndex = markerContext["index"] as? NSNumber,
               let index = v2ExactUInt32(rawIndex)
            {
                mutableAttrs[RenderBridgeAttributes.orderedListMarkerLabel] =
                    OrderedListMarkerFormatter.label(
                        index: index,
                        nestingDepth: visualListDepth,
                        theme: theme?.list?.orderedMarker
                    )
            }
            mutableAttrs[RenderBridgeAttributes.listMarkerColor] = theme?.list?.markerColor
            mutableAttrs[RenderBridgeAttributes.listMarkerScale] = theme?.list?.markerScale
            if let sheet = theme?.styleSheet {
                mutableAttrs[RenderBridgeAttributes.listMarkerScale] = EditorTheme.cgFloat(sheet["listMarker"]["scale"]) ?? ((markerContext["ordered"] as? Bool) == true ? 1 : LayoutConstants.unorderedListMarkerFontScale)
            }
            mutableAttrs[RenderBridgeAttributes.listMarkerGap] = theme?.list?.markerGap
            mutableAttrs[RenderBridgeAttributes.listMarkerBaseFont] = paragraphBaseFont
            if let sheet = theme?.styleSheet, (markerContext["kind"] as? String) == "task" {
                let checkbox = sheet.checkbox(checked: markerContext["checked"] as? Bool == true)
                mutableAttrs[editorTaskCheckboxAttribute] = EditorMentionRenderedBox(box: checkbox)
                mutableAttrs[RenderBridgeAttributes.listMarkerGap] = checkbox.number("gap", fallback: 8)
            }
            mutableAttrs[RenderBridgeAttributes.listMarkerWidth] = listMarkerWidth(
                for: currentBlock,
                theme: theme,
                baseFont: paragraphBaseFont
            )
        }
        if currentBlock.nodeType == "codeBlock", theme?.styleSheet == nil {
            mutableAttrs[RenderBridgeAttributes.codeBlockBackgroundColor] =
                theme?.codeBlock?.backgroundColor ?? UIColor.secondarySystemBackground
            mutableAttrs[RenderBridgeAttributes.codeBlockBorderRadius] =
                theme?.codeBlock?.borderRadius ?? 8
            mutableAttrs[RenderBridgeAttributes.codeBlockPaddingHorizontal] =
                theme?.codeBlock?.paddingHorizontal ?? 12
            mutableAttrs[RenderBridgeAttributes.codeBlockPaddingVertical] =
                theme?.codeBlock?.paddingVertical ?? 8
        }
        if blockquoteDepth(in: blockStack) > 0, theme?.styleSheet == nil {
            let foreground = mutableAttrs[.foregroundColor] as? UIColor ?? .separator
            mutableAttrs[RenderBridgeAttributes.blockquoteBorderColor] =
                theme?.blockquote?.borderColor
                ?? foreground.withAlphaComponent(0.3)
            mutableAttrs[RenderBridgeAttributes.blockquoteBorderWidth] =
                theme?.blockquote?.borderWidth ?? LayoutConstants.blockquoteBorderWidth
            mutableAttrs[RenderBridgeAttributes.blockquoteMarkerGap] =
                theme?.blockquote?.markerGap ?? LayoutConstants.blockquoteMarkerGap
        }
        return mutableAttrs
    }

    /// Create a newline attributed string used between blocks.
    ///
    /// This newline separates consecutive blocks in the flat rendered text.
    static func interBlockNewline(
        baseFont: UIFont,
        textColor: UIColor,
        blockStack: [BlockContext],
        theme: EditorTheme?,
        paragraphSpacingOverride: CGFloat? = nil,
        topLevelChildIndex: Int? = nil
    ) -> NSAttributedString {
        var attrs = applyBlockStyle(
            to: defaultAttributes(baseFont: baseFont, textColor: textColor),
            blockStack: blockStack,
            theme: theme,
            blockBaseFont: baseFont
        )
        if let topLevelChildIndex {
            attrs[RenderBridgeAttributes.topLevelChildIndex] = NSNumber(value: topLevelChildIndex)
        }
        attrs[RenderBridgeAttributes.blockBoundary] = true
        if let paragraphSpacingOverride,
           let paragraphStyle = (attrs[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy()
               as? NSMutableParagraphStyle
        {
            paragraphStyle.paragraphSpacing = paragraphSpacingOverride
            attrs[.paragraphStyle] = paragraphStyle
        }
        return NSAttributedString(string: "\n", attributes: attrs)
    }

    static func listMarkerWidth(
        for context: BlockContext,
        theme: EditorTheme?,
        baseFont: UIFont
    ) -> CGFloat {
        guard let listContext = context.listContext else { return 0 }
        if let sheet = theme?.styleSheet {
            if (listContext["kind"] as? String) == "task" {
                let box = sheet.checkbox(checked: listContext["checked"] as? Bool == true)
                return box.number("size", fallback: 24) + box.number("gap", fallback: 8)
            }
            let ordered = (listContext["ordered"] as? Bool) == true
            let scale = EditorTheme.cgFloat(sheet["listMarker"]["scale"]) ?? (ordered ? 1 : LayoutConstants.unorderedListMarkerFontScale)
            let gap = EditorTheme.cgFloat(sheet["listMarker"]["gap"]) ?? 8
            if !ordered {
                return EditorLayoutManager.unorderedBulletDrawingRect(usedRect: .zero, lineFragmentRect: .zero, markerWidth: 0, baselineY: 0, baseFont: baseFont, markerScale: scale, origin: .zero).width + gap
            }
            let label = ordered
                ? OrderedListMarkerFormatter.label(index: jsonUInt32(listContext["index"]) ?? 1, nestingDepth: Int(context.depth), theme: theme?.list?.orderedMarker)
                : "•"
            return ceil((label as NSString).size(withAttributes: [.font: baseFont.withSize(baseFont.pointSize * scale)]).width) + gap
        }
        return LayoutConstants.listMarkerWidth
    }

    private static func resolvedTextStyle(
        for blockStack: [BlockContext],
        theme: EditorTheme?
    ) -> EditorTextStyle? {
        if let sheet = theme?.styleSheet {
            return sheet.textStyle(blockStack.last?.nodeType ?? "paragraph", ancestors: blockStack.dropLast().map(\.nodeType))
        }
        let inBlockquote = blockquoteDepth(in: blockStack) > 0
        guard let currentBlock = effectiveBlockContext(blockStack) else {
            return theme?.effectiveTextStyle(for: "paragraph", inBlockquote: inBlockquote)
        }
        return theme?.effectiveTextStyle(for: currentBlock.nodeType, inBlockquote: inBlockquote)
    }

    static func blockquoteDepth(in blockStack: [BlockContext]) -> Int {
        blockStack.reduce(into: 0) { count, context in
            if context.nodeType == "blockquote" {
                count += 1
            }
        }
    }

    private static func columnContainerDepth(in blockStack: [BlockContext]) -> Int {
        blockStack.reduce(into: 0) { count, context in
            if context.nodeType == "columns" || context.nodeType == "column" {
                count += 1
            }
        }
    }

    static func isTransparentContainer(_ nodeType: String) -> Bool {
        nodeType == "blockquote" || nodeType == "columns" || nodeType == "column" || ["bulletList", "orderedList", "taskList"].contains(EditorStyleSheet.element(nodeType))
    }

    static func resolvedFont(
        for blockStack: [BlockContext],
        baseFont: UIFont,
        theme: EditorTheme?
    ) -> UIFont {
        resolvedTextStyle(for: blockStack, theme: theme)?.resolvedFont(fallback: baseFont)
            ?? baseFont
    }

    static func resolvedTextColor(
        for blockStack: [BlockContext],
        textColor: UIColor,
        theme: EditorTheme?
    ) -> UIColor {
        resolvedTextStyle(for: blockStack, theme: theme)?.color ?? textColor
    }

}
