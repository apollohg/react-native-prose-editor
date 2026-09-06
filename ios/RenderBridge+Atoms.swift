import UIKit
import ImageIO
import CryptoKit

extension RenderBridge {
    /// Build NSAttributedString attributes for a set of render marks.
    ///
    /// Supported marks:
    /// - `bold` -> adds `.traitBold` to the font descriptor
    /// - `italic` -> adds `.traitItalic` to the font descriptor
    /// - `underline` -> sets `.underlineStyle = .single`
    /// - `strike` / `strikethrough` -> sets `.strikethroughStyle = .single`
    /// - `code` -> uses a monospaced font variant
    ///
    /// Multiple marks are combined: "bold italic" produces a bold-italic font.
    static func attributesForMarks(
        _ marks: [Any],
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme? = nil
    ) -> [NSAttributedString.Key: Any] {
        var attrs = defaultAttributes(baseFont: baseFont, textColor: textColor)

        if marks.isEmpty {
            return attrs
        }

        var traits: UIFontDescriptor.SymbolicTraits = []
        var useMonospace = false
        var linkTheme: EditorLinkTheme?
        var shouldUnderline = false
        for mark in marks {
            let markObject = mark as? [String: Any]
            let markType: String
            if let markName = mark as? String {
                markType = markName
            } else if let resolvedType = markObject?["type"] as? String {
                markType = resolvedType
            } else {
                continue
            }

            switch markType {
            case "bold", "strong":
                traits.insert(.traitBold)
            case "italic", "em":
                traits.insert(.traitItalic)
            case "underline":
                shouldUnderline = true
            case "strike", "strikethrough":
                attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
            case "code":
                useMonospace = true
            case "link":
                linkTheme = theme?.links
                if theme?.links?.underline ?? true {
                    shouldUnderline = true
                }
                attrs[.foregroundColor] = theme?.links?.color ?? UIColor.systemBlue
                if let backgroundColor = theme?.links?.backgroundColor {
                    attrs[.backgroundColor] = backgroundColor
                }
                if let href = markObject?["href"] as? String, !href.isEmpty {
                    attrs[RenderBridgeAttributes.linkHref] = href
                }
            default:
                break
            }
        }

        var resolvedFont = linkTheme?.resolvedFont(fallback: baseFont) ?? baseFont

        resolvedFont = ViewerFontEnvironment.shared.resolveFont(
            family: useMonospace ? "monospace" : nil,
            size: resolvedFont.pointSize,
            fallback: resolvedFont,
            additionalTraits: traits,
            semanticGeneration: "legacy-editor-theme"
        )

        if shouldUnderline {
            attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue
        }
        attrs[.font] = resolvedFont
        return attrs
    }

    /// Create an attributed string for a void inline element (e.g. hardBreak).
    ///
    /// A hardBreak is rendered as a newline character with custom attributes
    /// so the position bridge knows it represents a single doc position.
    static func attributedStringForVoidInline(
        nodeType: String,
        docPos: UInt32,
        attrs _: [String: Any],
        baseFont: UIFont,
        textColor: UIColor,
        blockStack: [BlockContext],
        topLevelChildIndex _: Int?,
        theme: EditorTheme?
    ) -> NSAttributedString {
        let blockFont = resolvedFont(for: blockStack, baseFont: baseFont, theme: theme)
        let blockColor = resolvedTextColor(for: blockStack, textColor: textColor, theme: theme)
        var attrs = defaultAttributes(baseFont: blockFont, textColor: blockColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos
        let styledAttrs = applyBlockStyle(
            to: attrs,
            blockStack: blockStack,
            theme: theme,
            blockBaseFont: blockFont
        )

        switch nodeType {
        case "hardBreak", "hard_break":
            var hardBreakAttrs = styledAttrs
            if let paragraphStyle = (hardBreakAttrs[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy()
                as? NSMutableParagraphStyle
            {
                paragraphStyle.paragraphSpacing = 0
                hardBreakAttrs[.paragraphStyle] = paragraphStyle
            }
            return NSAttributedString(string: "\n", attributes: hardBreakAttrs)
        default:
            // Unknown void inline: render as object replacement character.
            return NSAttributedString(
                string: LayoutConstants.objectReplacementCharacter,
                attributes: styledAttrs
            )
        }
    }

    /// Create an attributed string for a void block element (e.g. horizontalRule).
    ///
    /// Horizontal rules are rendered as U+FFFC (object replacement character)
    /// with an NSTextAttachment that draws a separator line.
    static func attributedStringForVoidBlock(
        nodeType: String,
        docPos: UInt32,
        elementAttrs: [String: Any],
        baseFont: UIFont,
        textColor: UIColor,
        topLevelChildIndex: Int?,
        theme: EditorTheme?,
        atomKey: String,
        atomConfiguration: AtomRenderConfiguration?
    ) -> NSAttributedString {
        var attrs = defaultAttributes(baseFont: baseFont, textColor: textColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos
        if let topLevelChildIndex {
            attrs[RenderBridgeAttributes.topLevelChildIndex] = NSNumber(value: topLevelChildIndex)
        }

        if atomConfiguration?.registeredNodeTypes.contains(nodeType) == true {
            let attachment = AtomBlockAttachment(
                atomKey: atomKey,
                nodeType: nodeType,
                docPos: docPos,
                reservedHeight: atomConfiguration?.reservedHeight(
                    atomKey: atomKey,
                    nodeType: nodeType
                ) ?? 0
            )
            let attrStr = NSMutableAttributedString(attachment: attachment)
            attrStr.addAttributes(attrs, range: NSRange(location: 0, length: attrStr.length))
            return attrStr
        }

        switch nodeType {
        case "horizontalRule", "horizontal_rule":
            let attachment = HorizontalRuleAttachment()
            attachment.lineColor = theme?.horizontalRule?.color ?? textColor.withAlphaComponent(0.3)
            attachment.lineHeight = theme?.horizontalRule?.thickness ?? LayoutConstants.horizontalRuleHeight
            attachment.verticalPadding = resolvedHorizontalRuleVerticalMargin(theme: theme)
            let attrStr = NSMutableAttributedString(
                attachment: attachment
            )
            // Apply our custom attributes to the attachment character.
            let range = NSRange(location: 0, length: attrStr.length)
            attrStr.addAttributes(attrs, range: range)
            return attrStr
        case "image":
            guard let source = (elementAttrs["src"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !source.isEmpty
            else {
                return NSAttributedString(
                    string: LayoutConstants.objectReplacementCharacter,
                    attributes: attrs
                )
            }
            let attachment = BlockImageAttachment(
                source: source,
                placeholderTint: textColor,
                preferredWidth: jsonCGFloat(elementAttrs["width"]),
                preferredHeight: jsonCGFloat(elementAttrs["height"])
            )
            let attrStr = NSMutableAttributedString(attachment: attachment)
            let range = NSRange(location: 0, length: attrStr.length)
            attrStr.addAttributes(attrs, range: range)
            return attrStr
        default:
            // Unknown void block: render as object replacement character.
            return NSAttributedString(
                string: LayoutConstants.objectReplacementCharacter,
                attributes: attrs
            )
        }
    }

    /// Create an attributed string for an opaque inline atom (unknown inline void).
    static func attributedStringForOpaqueInlineAtom(
        nodeType: String,
        label: String,
        docPos: UInt32,
        baseFont: UIFont,
        textColor: UIColor,
        blockStack: [BlockContext],
        topLevelChildIndex _: Int?,
        theme: EditorTheme?,
        mentionTheme: EditorMentionTheme?
    ) -> NSAttributedString {
        let blockFont = resolvedFont(for: blockStack, baseFont: baseFont, theme: theme)
        let blockColor = resolvedTextColor(for: blockStack, textColor: textColor, theme: theme)
        var attrs = defaultAttributes(baseFont: blockFont, textColor: blockColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos
        if nodeType == "mention" {
            let resolvedMentionTheme = theme?.mentions?.merged(with: mentionTheme) ?? mentionTheme
            let node = resolvedMentionTheme?.node
            attrs[.foregroundColor] = node?.textColor ?? blockColor
            attrs[.backgroundColor] =
                node?.backgroundColor ?? UIColor.systemBlue.withAlphaComponent(0.12)
            if let mentionFont = mentionFont(from: blockFont, theme: node) {
                attrs[.font] = mentionFont
            }
        } else {
            attrs[.backgroundColor] = UIColor.systemGray5
        }
        let styledAttrs = applyBlockStyle(
            to: attrs,
            blockStack: blockStack,
            theme: theme,
            blockBaseFont: blockFont
        )

        let visibleText = nodeType == "mention" ? label : "[\(label)]"
        return NSAttributedString(string: visibleText, attributes: styledAttrs)
    }

    /// Create an attributed string for an opaque block atom (unknown block void).
    static func attributedStringForOpaqueBlockAtom(
        nodeType: String,
        label: String,
        docPos: UInt32,
        baseFont: UIFont,
        textColor: UIColor,
        topLevelChildIndex: Int?,
        theme: EditorTheme?
    ) -> NSAttributedString {
        var attrs = defaultAttributes(baseFont: baseFont, textColor: textColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos
        attrs[.backgroundColor] = UIColor.systemGray5
        if let topLevelChildIndex {
            attrs[RenderBridgeAttributes.topLevelChildIndex] = NSNumber(value: topLevelChildIndex)
        }

        return NSAttributedString(string: "[\(label)]", attributes: attrs)
    }

    private static func mentionFont(from baseFont: UIFont, theme: EditorMentionNodeTheme?)
        -> UIFont?
    {
        guard let fontWeight = theme?.fontWeight else { return nil }
        return EditorTextStyle(fontWeight: fontWeight).resolvedFont(fallback: baseFont)
    }

}
