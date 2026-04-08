import UIKit

extension Notification.Name {
    static let editorImageAttachmentDidLoad = Notification.Name(
        "com.apollohg.editor.imageAttachmentDidLoad"
    )
}

private enum RenderImageCache {
    static let cache = NSCache<NSString, UIImage>()
    static let queue = DispatchQueue(label: "com.apollohg.editor.image-loader", qos: .userInitiated)
}

// MARK: - Constants

/// Custom NSAttributedString attribute keys for editor metadata.
enum RenderBridgeAttributes {
    /// Marks a character as a void element placeholder (hardBreak, horizontalRule).
    /// The value is the node type string (e.g. "hardBreak", "horizontalRule").
    static let voidNodeType = NSAttributedString.Key("com.apollohg.editor.voidNodeType")

    /// Stores the Rust document position (UInt32) for void elements.
    static let docPos = NSAttributedString.Key("com.apollohg.editor.docPos")

    /// Marks a character as a block boundary (for block start/end tracking).
    static let blockBoundary = NSAttributedString.Key("com.apollohg.editor.blockBoundary")

    /// Stores the block node type (e.g. "paragraph", "listItem").
    static let blockNodeType = NSAttributedString.Key("com.apollohg.editor.blockNodeType")

    /// Stores the block depth (UInt8).
    static let blockDepth = NSAttributedString.Key("com.apollohg.editor.blockDepth")

    /// Stores list context info as a dictionary for list items.
    static let listContext = NSAttributedString.Key("com.apollohg.editor.listContext")

    /// Marks blocks that should render a visible list marker.
    static let listMarkerContext = NSAttributedString.Key("com.apollohg.editor.listMarkerContext")

    /// Stores the rendered list marker color for the paragraph marker.
    static let listMarkerColor = NSAttributedString.Key("com.apollohg.editor.listMarkerColor")

    /// Stores the rendered list marker scale for unordered bullets.
    static let listMarkerScale = NSAttributedString.Key("com.apollohg.editor.listMarkerScale")

    /// Stores the reserved list marker gutter width.
    static let listMarkerWidth = NSAttributedString.Key("com.apollohg.editor.listMarkerWidth")

    /// Stores the rendered blockquote border color.
    static let blockquoteBorderColor = NSAttributedString.Key("com.apollohg.editor.blockquoteBorderColor")

    /// Stores the rendered blockquote border width.
    static let blockquoteBorderWidth = NSAttributedString.Key("com.apollohg.editor.blockquoteBorderWidth")

    /// Stores the rendered blockquote gap between border and text.
    static let blockquoteMarkerGap = NSAttributedString.Key("com.apollohg.editor.blockquoteMarkerGap")

    /// Marks synthetic zero-width placeholders used only for UIKit layout.
    static let syntheticPlaceholder = NSAttributedString.Key("com.apollohg.editor.syntheticPlaceholder")
}

/// Layout constants for paragraph styles.
enum LayoutConstants {
    /// Spacing between paragraphs (points).
    static let paragraphSpacing: CGFloat = 8.0

    /// Base indentation per depth level (points).
    static let indentPerDepth: CGFloat = 24.0

    /// Width reserved for the list bullet/number (points).
    static let listMarkerWidth: CGFloat = 20.0

    /// Gap between the list marker and the text that follows (points).
    static let listMarkerTextGap: CGFloat = 8.0

    /// Height of the horizontal rule separator line (points).
    static let horizontalRuleHeight: CGFloat = 1.0

    /// Vertical padding above and below the horizontal rule (points).
    static let horizontalRuleVerticalPadding: CGFloat = 8.0

    /// Total leading inset reserved for each blockquote depth.
    static let blockquoteIndent: CGFloat = 18.0

    /// Width of the rendered blockquote border bar.
    static let blockquoteBorderWidth: CGFloat = 3.0

    /// Gap between the blockquote border bar and the text that follows.
    static let blockquoteMarkerGap: CGFloat = 8.0

    /// Bullet character for unordered list items.
    static let unorderedListBullet = "\u{2022} "

    /// Scale factor applied only to unordered list marker glyphs.
    static let unorderedListMarkerFontScale: CGFloat = 2.0

    /// Object replacement character used for void block elements.
    static let objectReplacementCharacter = "\u{FFFC}"
}

// MARK: - RenderBridge

/// Converts RenderElement JSON (emitted by Rust editor-core via UniFFI) into
/// NSAttributedString for display in a UITextView.
///
/// The JSON format matches the output of `serialize_render_elements` in lib.rs:
/// ```json
/// [
///   {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
///   {"type": "textRun", "text": "Hello ", "marks": []},
///   {"type": "textRun", "text": "world", "marks": ["bold"]},
///   {"type": "blockEnd"},
///   {"type": "voidInline", "nodeType": "hardBreak", "docPos": 12},
///   {"type": "voidBlock", "nodeType": "horizontalRule", "docPos": 15}
/// ]
/// ```
final class RenderBridge {

    // MARK: - Public API

    /// Convert a JSON array of RenderElements into an NSAttributedString.
    ///
    /// - Parameters:
    ///   - json: A JSON string representing an array of render elements.
    ///   - baseFont: The default font for unstyled text.
    ///   - textColor: The default text color.
    /// - Returns: The rendered attributed string. Returns an empty attributed
    ///   string if the JSON is invalid.
    static func renderElements(
        fromJSON json: String,
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme? = nil
    ) -> NSAttributedString {
        guard let data = json.data(using: .utf8),
              let parsed = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else {
            return NSAttributedString()
        }

        return renderElements(
            fromArray: parsed,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )
    }

    /// Convert a parsed array of RenderElement dictionaries into an NSAttributedString.
    ///
    /// This is the main rendering entry point. It processes elements in order,
    /// maintaining a block context stack for proper paragraph styling.
    ///
    /// - Parameters:
    ///   - elements: Parsed JSON array where each element is a dictionary.
    ///   - baseFont: The default font for unstyled text.
    ///   - textColor: The default text color.
    /// - Returns: The rendered attributed string.
    static func renderElements(
        fromArray elements: [[String: Any]],
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme? = nil
    ) -> NSAttributedString {
        let result = NSMutableAttributedString()
        var blockStack: [BlockContext] = []
        var isFirstBlock = true
        var pendingTrailingParagraphSpacing: CGFloat? = nil

        for element in elements {
            guard let type = element["type"] as? String else { continue }

            switch type {
            case "textRun":
                let text = element["text"] as? String ?? ""
                let marks = element["marks"] as? [Any] ?? []
                let blockFont = resolvedFont(
                    for: blockStack,
                    baseFont: baseFont,
                    theme: theme
                )
                let blockColor = resolvedTextColor(
                    for: blockStack,
                    textColor: textColor,
                    theme: theme
                )
                let baseAttrs = attributesForMarks(
                    marks,
                    baseFont: blockFont,
                    textColor: blockColor
                )
                let attrs = applyBlockStyle(
                    to: baseAttrs,
                    blockStack: blockStack,
                    theme: theme
                )
                result.append(NSAttributedString(string: text, attributes: attrs))

            case "voidInline":
                let nodeType = element["nodeType"] as? String ?? ""
                let docPos = jsonUInt32(element["docPos"])
                let attrs = element["attrs"] as? [String: Any] ?? [:]
                if nodeType == "hardBreak" {
                    overrideTrailingParagraphSpacing(in: result, paragraphSpacing: 0)
                }
                let attrStr = attributedStringForVoidInline(
                    nodeType: nodeType,
                    docPos: docPos,
                    attrs: attrs,
                    baseFont: baseFont,
                    textColor: textColor,
                    blockStack: blockStack,
                    theme: theme
                )
                result.append(attrStr)

            case "voidBlock":
                let nodeType = element["nodeType"] as? String ?? ""
                let docPos = jsonUInt32(element["docPos"])
                let attrs = element["attrs"] as? [String: Any] ?? [:]

                // Add inter-block newline if not the first block.
                if !isFirstBlock {
                    collapseTrailingSpacingBeforeHorizontalRuleIfNeeded(
                        in: result,
                        pendingParagraphSpacing: &pendingTrailingParagraphSpacing,
                        nodeType: nodeType,
                        theme: theme
                    )
                    applyPendingTrailingParagraphSpacing(
                        in: result,
                        pendingParagraphSpacing: &pendingTrailingParagraphSpacing
                    )
                    result.append(
                        interBlockNewline(
                            baseFont: baseFont,
                            textColor: textColor,
                            blockStack: [],
                            theme: theme
                        )
                    )
                }
                isFirstBlock = false

                let attrStr = attributedStringForVoidBlock(
                    nodeType: nodeType,
                    docPos: docPos,
                    elementAttrs: attrs,
                    baseFont: baseFont,
                    textColor: textColor,
                    theme: theme
                )
                result.append(attrStr)

            case "opaqueInlineAtom":
                let nodeType = element["nodeType"] as? String ?? ""
                let label = element["label"] as? String ?? "?"
                let docPos = jsonUInt32(element["docPos"])
                let attrStr = attributedStringForOpaqueInlineAtom(
                    nodeType: nodeType,
                    label: label,
                    docPos: docPos,
                    baseFont: baseFont,
                    textColor: textColor,
                    blockStack: blockStack,
                    theme: theme
                )
                result.append(attrStr)

            case "opaqueBlockAtom":
                let nodeType = element["nodeType"] as? String ?? ""
                let label = element["label"] as? String ?? "?"
                let docPos = jsonUInt32(element["docPos"])

                if !isFirstBlock {
                    applyPendingTrailingParagraphSpacing(
                        in: result,
                        pendingParagraphSpacing: &pendingTrailingParagraphSpacing
                    )
                    result.append(
                        interBlockNewline(
                            baseFont: baseFont,
                            textColor: textColor,
                            blockStack: [],
                            theme: theme
                        )
                    )
                }
                isFirstBlock = false

                let attrStr = attributedStringForOpaqueBlockAtom(
                    nodeType: nodeType,
                    label: label,
                    docPos: docPos,
                    baseFont: baseFont,
                    textColor: textColor,
                    theme: theme
                )
                result.append(attrStr)

            case "blockStart":
                let nodeType = element["nodeType"] as? String ?? ""
                let depth = jsonUInt8(element["depth"])
                let listContext = element["listContext"] as? [String: Any]
                let isListItemContainer = nodeType == "listItem" && listContext != nil
                let isTransparentContainer = nodeType == "blockquote"
                let ctx = BlockContext(
                    nodeType: nodeType,
                    depth: depth,
                    listContext: listContext,
                    markerPending: isListItemContainer
                )
                let nestedListItemContainer =
                    isListItemContainer && (theme?.list?.itemSpacing != nil)
                    && blockStack.contains(where: { $0.nodeType == "listItem" && $0.listContext != nil })

                if !isListItemContainer && !isTransparentContainer {
                    // Add inter-block newline before non-first rendered blocks.
                    if !isFirstBlock {
                        applyPendingTrailingParagraphSpacing(
                            in: result,
                            pendingParagraphSpacing: &pendingTrailingParagraphSpacing
                        )
                        let newlineBlockStack: [BlockContext]
                        if blockquoteDepth(in: blockStack + [ctx]) > 0,
                           !trailingRenderedContentHasBlockquote(in: result)
                        {
                            newlineBlockStack = []
                        } else {
                            newlineBlockStack = blockStack + [ctx]
                        }
                        let collapsedSeparatorSpacing = collapsedParagraphSpacingAfterHorizontalRule(
                            in: result,
                            separatorBlockStack: newlineBlockStack,
                            theme: theme,
                            baseFont: baseFont
                        )
                        result.append(
                            interBlockNewline(
                                baseFont: baseFont,
                                textColor: textColor,
                                blockStack: newlineBlockStack,
                                theme: theme,
                                paragraphSpacingOverride: collapsedSeparatorSpacing
                            )
                        )
                    }
                    isFirstBlock = false
                } else if applyPendingTrailingParagraphSpacing(
                    in: result,
                    pendingParagraphSpacing: &pendingTrailingParagraphSpacing
                ) {
                    // Applied list item spacing queued when the previous item ended.
                } else if nestedListItemContainer {
                    overrideTrailingParagraphSpacing(
                        in: result,
                        paragraphSpacing: CGFloat(theme?.list?.itemSpacing ?? 0)
                    )
                }

                // Push block context for inline children to reference.
                blockStack.append(ctx)

                var markerListContext: [String: Any]? = nil
                if !isListItemContainer {
                    if let directListContext = listContext {
                        markerListContext = directListContext
                    } else {
                        markerListContext = consumePendingListMarker(from: &blockStack)
                    }
                }

                if markerListContext != nil {
                    if var currentBlock = blockStack.popLast() {
                        currentBlock.listMarkerContext = markerListContext
                        if currentBlock.listContext != nil {
                            currentBlock.listContext = markerListContext
                        }
                        blockStack.append(currentBlock)
                    }
                    // On iOS we draw list markers outside the editable text stream so
                    // UIKit still sees paragraph-start for native capitalization.
                }

            case "blockEnd":
                if let endedBlock = blockStack.popLast() {
                    appendTrailingHardBreakPlaceholderIfNeeded(
                        in: result,
                        endedBlock: endedBlock,
                        remainingBlockStack: blockStack,
                        baseFont: baseFont,
                        textColor: textColor,
                        theme: theme
                    )
                    if endedBlock.listContext != nil {
                        pendingTrailingParagraphSpacing = theme?.list?.itemSpacing
                    }
                }

            default:
                break
            }
        }

        return result
    }

    // MARK: - Mark Handling

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
        textColor: UIColor
    ) -> [NSAttributedString.Key: Any] {
        var attrs = defaultAttributes(baseFont: baseFont, textColor: textColor)

        if marks.isEmpty {
            return attrs
        }

        var traits: UIFontDescriptor.SymbolicTraits = []
        var useMonospace = false
        for mark in marks {
            let markType: String
            if let markName = mark as? String {
                markType = markName
            } else if let markObject = mark as? [String: Any],
                      let resolvedType = markObject["type"] as? String {
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
                attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue
            case "strike", "strikethrough":
                attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
            case "code":
                useMonospace = true
            case "link":
                attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue
                attrs[.foregroundColor] = UIColor.systemBlue
            default:
                break
            }
        }

        var resolvedFont = baseFont

        if useMonospace {
            resolvedFont = UIFont.monospacedSystemFont(
                ofSize: baseFont.pointSize,
                weight: traits.contains(.traitBold) ? .bold : .regular
            )
            // Monospaced doesn't support italic via descriptor traits, but we
            // still apply bold via the weight parameter above. If italic is also
            // requested alongside code, we skip it (monospaced italic is rare).
            if traits.contains(.traitItalic) && !traits.contains(.traitBold) {
                // For code+italic only, try applying italic trait.
                if let descriptor = resolvedFont.fontDescriptor.withSymbolicTraits(.traitItalic) {
                    resolvedFont = UIFont(descriptor: descriptor, size: baseFont.pointSize)
                }
            }
        } else if !traits.isEmpty {
            if let descriptor = baseFont.fontDescriptor.withSymbolicTraits(traits) {
                resolvedFont = UIFont(descriptor: descriptor, size: baseFont.pointSize)
            }
        }

        attrs[.font] = resolvedFont
        return attrs
    }

    // MARK: - Void Inline Elements

    /// Create an attributed string for a void inline element (e.g. hardBreak).
    ///
    /// A hardBreak is rendered as a newline character with custom attributes
    /// so the position bridge knows it represents a single doc position.
    private static func attributedStringForVoidInline(
        nodeType: String,
        docPos: UInt32,
        attrs _: [String: Any],
        baseFont: UIFont,
        textColor: UIColor,
        blockStack: [BlockContext],
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
            theme: theme
        )

        switch nodeType {
        case "hardBreak":
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

    // MARK: - Void Block Elements

    /// Create an attributed string for a void block element (e.g. horizontalRule).
    ///
    /// Horizontal rules are rendered as U+FFFC (object replacement character)
    /// with an NSTextAttachment that draws a separator line.
    private static func attributedStringForVoidBlock(
        nodeType: String,
        docPos: UInt32,
        elementAttrs: [String: Any],
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme?
    ) -> NSAttributedString {
        var attrs = defaultAttributes(baseFont: baseFont, textColor: textColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos

        switch nodeType {
        case "horizontalRule":
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

    // MARK: - Opaque Atoms

    /// Create an attributed string for an opaque inline atom (unknown inline void).
    private static func attributedStringForOpaqueInlineAtom(
        nodeType: String,
        label: String,
        docPos: UInt32,
        baseFont: UIFont,
        textColor: UIColor,
        blockStack: [BlockContext],
        theme: EditorTheme?
    ) -> NSAttributedString {
        let blockFont = resolvedFont(for: blockStack, baseFont: baseFont, theme: theme)
        let blockColor = resolvedTextColor(for: blockStack, textColor: textColor, theme: theme)
        var attrs = defaultAttributes(baseFont: blockFont, textColor: blockColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos
        if nodeType == "mention" {
            attrs[.foregroundColor] = theme?.mentions?.textColor ?? blockColor
            attrs[.backgroundColor] = theme?.mentions?.backgroundColor ?? UIColor.systemBlue.withAlphaComponent(0.12)
            if let mentionFont = mentionFont(from: blockFont, theme: theme?.mentions) {
                attrs[.font] = mentionFont
            }
        } else {
            attrs[.backgroundColor] = UIColor.systemGray5
        }
        let styledAttrs = applyBlockStyle(
            to: attrs,
            blockStack: blockStack,
            theme: theme
        )

        let visibleText = nodeType == "mention" ? label : "[\(label)]"
        return NSAttributedString(string: visibleText, attributes: styledAttrs)
    }

    /// Create an attributed string for an opaque block atom (unknown block void).
    private static func attributedStringForOpaqueBlockAtom(
        nodeType: String,
        label: String,
        docPos: UInt32,
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme?
    ) -> NSAttributedString {
        var attrs = defaultAttributes(baseFont: baseFont, textColor: textColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos
        attrs[.backgroundColor] = UIColor.systemGray5

        return NSAttributedString(string: "[\(label)]", attributes: attrs)
    }

    private static func mentionFont(from baseFont: UIFont, theme: EditorMentionTheme?) -> UIFont? {
        guard let fontWeight = theme?.fontWeight else { return nil }
        let descriptorTraits = EditorTheme.shouldApplyBoldTrait(fontWeight)
            ? UIFontDescriptor.SymbolicTraits.traitBold
            : []
        if descriptorTraits.isEmpty {
            return UIFont.systemFont(
                ofSize: baseFont.pointSize,
                weight: EditorTheme.fontWeight(from: fontWeight)
            )
        }
        guard let descriptor = baseFont.fontDescriptor.withSymbolicTraits(descriptorTraits) else {
            return UIFont.systemFont(
                ofSize: baseFont.pointSize,
                weight: EditorTheme.fontWeight(from: fontWeight)
            )
        }
        return UIFont(descriptor: descriptor, size: baseFont.pointSize)
    }

    // MARK: - Block Styling

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
        let baseIndent = (CGFloat(context.depth) * indentPerDepth)
            - (quoteDepth * indentPerDepth)
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

        if let lineHeight = blockStyle?.lineHeight {
            style.minimumLineHeight = lineHeight
            style.maximumLineHeight = lineHeight
        }

        return style
    }

    // MARK: - List Markers

    /// Generate the list marker string (bullet or number) from a list context.
    static func listMarkerString(listContext: [String: Any]) -> String {
        let ordered = (listContext["ordered"] as? NSNumber)?.boolValue ?? false

        if ordered {
            let index = (listContext["index"] as? NSNumber)?.intValue ?? 1
            return "\(index). "
        } else {
            return LayoutConstants.unorderedListBullet
        }
    }

    // MARK: - Private Helpers

    /// Extract a `UInt32` from a JSON value produced by `JSONSerialization`.
    static func jsonUInt32(_ value: Any?) -> UInt32 {
        if let number = value as? NSNumber {
            return number.uint32Value
        }
        return 0
    }

    /// Extract a `UInt8` from a JSON value produced by `JSONSerialization`.
    static func jsonUInt8(_ value: Any?) -> UInt8 {
        if let number = value as? NSNumber {
            return number.uint8Value
        }
        return 0
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

    private static func defaultAttributes(
        baseFont: UIFont,
        textColor: UIColor
    ) -> [NSAttributedString.Key: Any] {
        [
            .font: baseFont,
            .foregroundColor: textColor,
        ]
    }

    @discardableResult
    private static func applyBlockStyle(
        to attrs: [NSAttributedString.Key: Any],
        blockStack: [BlockContext],
        theme: EditorTheme?
    ) -> [NSAttributedString.Key: Any] {
        guard let currentBlock = effectiveBlockContext(blockStack) else { return attrs }
        var mutableAttrs = attrs
        let blockFont = mutableAttrs[.font] as? UIFont ?? .systemFont(ofSize: 16)
        mutableAttrs[.paragraphStyle] = paragraphStyleForBlock(
            currentBlock,
            blockStack: blockStack,
            theme: theme,
            baseFont: blockFont
        )
        mutableAttrs[RenderBridgeAttributes.blockNodeType] = currentBlock.nodeType
        mutableAttrs[RenderBridgeAttributes.blockDepth] = currentBlock.depth
        if let listContext = currentBlock.listContext {
            mutableAttrs[RenderBridgeAttributes.listContext] = listContext
        }
        if let markerContext = currentBlock.listMarkerContext {
            mutableAttrs[RenderBridgeAttributes.listMarkerContext] = markerContext
            mutableAttrs[RenderBridgeAttributes.listMarkerColor] = theme?.list?.markerColor
            mutableAttrs[RenderBridgeAttributes.listMarkerScale] = theme?.list?.markerScale
            mutableAttrs[RenderBridgeAttributes.listMarkerWidth] = listMarkerWidth(
                for: currentBlock,
                theme: theme,
                baseFont: blockFont
            )
        }
        if blockquoteDepth(in: blockStack) > 0 {
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
    /// It carries minimal styling (base font, no special attributes).
    private static func interBlockNewline(
        baseFont: UIFont,
        textColor: UIColor,
        blockStack: [BlockContext],
        theme: EditorTheme?,
        paragraphSpacingOverride: CGFloat? = nil
    ) -> NSAttributedString {
        var attrs = applyBlockStyle(
            to: defaultAttributes(baseFont: baseFont, textColor: textColor),
            blockStack: blockStack,
            theme: theme
        )
        if let paragraphSpacingOverride,
           let paragraphStyle = (attrs[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy()
               as? NSMutableParagraphStyle
        {
            paragraphStyle.paragraphSpacing = paragraphSpacingOverride
            attrs[.paragraphStyle] = paragraphStyle
        }
        return NSAttributedString(string: "\n", attributes: attrs)
    }

    private static func effectiveBlockContext(_ blockStack: [BlockContext]) -> BlockContext? {
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

    private static func trailingRenderedContentHasBlockquote(
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

    private static func consumePendingListMarker(from blockStack: inout [BlockContext]) -> [String: Any]? {
        guard blockStack.count >= 2 else { return nil }
        for idx in stride(from: blockStack.count - 2, through: 0, by: -1) {
            guard blockStack[idx].markerPending else { continue }
            blockStack[idx].markerPending = false
            return blockStack[idx].listContext
        }
        return nil
    }

    private static func overrideTrailingParagraphSpacing(
        in result: NSMutableAttributedString,
        paragraphSpacing: CGFloat
    ) {
        guard result.length > 0 else { return }

        let nsString = result.string as NSString
        let paragraphRange = nsString.paragraphRange(for: NSRange(location: result.length - 1, length: 0))
        result.enumerateAttribute(.paragraphStyle, in: paragraphRange, options: []) { value, range, _ in
            let sourceStyle = (value as? NSParagraphStyle)?.mutableCopy() as? NSMutableParagraphStyle
                ?? NSMutableParagraphStyle()
            sourceStyle.paragraphSpacing = paragraphSpacing
            result.addAttribute(.paragraphStyle, value: sourceStyle, range: range)
        }
    }

    private static func collapseTrailingSpacingBeforeHorizontalRuleIfNeeded(
        in result: NSMutableAttributedString,
        pendingParagraphSpacing: inout CGFloat?,
        nodeType: String,
        theme: EditorTheme?
    ) {
        guard nodeType == "horizontalRule" else { return }
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

    private static func collapsedParagraphSpacingAfterHorizontalRule(
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
    private static func applyPendingTrailingParagraphSpacing(
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
        result.enumerateAttribute(.paragraphStyle, in: paragraphRange, options: [.reverse]) { value, _, stop in
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
            guard result.attribute(
                RenderBridgeAttributes.voidNodeType,
                at: index,
                effectiveRange: nil
            ) as? String == "horizontalRule" else {
                return nil
            }
            return (
                result.attribute(.attachment, at: index, effectiveRange: nil)
                    as? HorizontalRuleAttachment
            )?.verticalPadding
        }

        return nil
    }

    private static func resolvedHorizontalRuleVerticalMargin(theme: EditorTheme?) -> CGFloat {
        theme?.horizontalRule?.verticalMargin ?? LayoutConstants.horizontalRuleVerticalPadding
    }

    private static func collapsedSpacing(
        existingSpacing: CGFloat,
        adjacentHorizontalRuleMargin: CGFloat
    ) -> CGFloat {
        max(existingSpacing, adjacentHorizontalRuleMargin) - adjacentHorizontalRuleMargin
    }

    private static func appendTrailingHardBreakPlaceholderIfNeeded(
        in result: NSMutableAttributedString,
        endedBlock: BlockContext,
        remainingBlockStack: [BlockContext],
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme?
    ) {
        guard result.length > 0 else { return }
        guard endedBlock.nodeType != "listItem" else { return }
        guard result.attribute(
            RenderBridgeAttributes.voidNodeType,
            at: result.length - 1,
            effectiveRange: nil
        ) as? String == "hardBreak" else {
            return
        }

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
            theme: theme
        )
        if let paragraphStyle = (styledAttrs[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy()
            as? NSMutableParagraphStyle
        {
            paragraphStyle.paragraphSpacing = 0
            styledAttrs[.paragraphStyle] = paragraphStyle
        }
        result.append(NSAttributedString(string: "\u{200B}", attributes: styledAttrs))
    }

    private static func listMarkerWidth(
        for context: BlockContext,
        theme: EditorTheme?,
        baseFont: UIFont
    ) -> CGFloat {
        guard let listContext = context.listContext else { return 0 }
        _ = listContext
        _ = theme
        _ = baseFont
        return LayoutConstants.listMarkerWidth
    }

    private static func resolvedTextStyle(
        for blockStack: [BlockContext],
        theme: EditorTheme?
    ) -> EditorTextStyle? {
        let inBlockquote = blockquoteDepth(in: blockStack) > 0
        guard let currentBlock = effectiveBlockContext(blockStack) else {
            return theme?.effectiveTextStyle(for: "paragraph", inBlockquote: inBlockquote)
        }
        return theme?.effectiveTextStyle(for: currentBlock.nodeType, inBlockquote: inBlockquote)
    }

    private static func blockquoteDepth(in blockStack: [BlockContext]) -> Int {
        blockStack.reduce(into: 0) { count, context in
            if context.nodeType == "blockquote" {
                count += 1
            }
        }
    }

    private static func resolvedFont(
        for blockStack: [BlockContext],
        baseFont: UIFont,
        theme: EditorTheme?
    ) -> UIFont {
        resolvedTextStyle(for: blockStack, theme: theme)?.resolvedFont(fallback: baseFont)
            ?? baseFont
    }

    private static func resolvedTextColor(
        for blockStack: [BlockContext],
        textColor: UIColor,
        theme: EditorTheme?
    ) -> UIColor {
        resolvedTextStyle(for: blockStack, theme: theme)?.color ?? textColor
    }
}

// MARK: - BlockContext

/// Transient context while rendering block elements. Pushed onto a stack
/// when a `blockStart` element is encountered and popped on `blockEnd`.
struct BlockContext {
    let nodeType: String
    let depth: UInt8
    var listContext: [String: Any]?
    var listMarkerContext: [String: Any]? = nil
    var markerPending: Bool = false
}

// MARK: - HorizontalRuleAttachment

/// NSTextAttachment subclass that draws a horizontal separator line.
///
/// The attachment renders as a thin line across the available width with
/// vertical padding. Used for `horizontalRule` void block elements.
final class HorizontalRuleAttachment: NSTextAttachment {

    var lineColor: UIColor = .separator
    var lineHeight: CGFloat = LayoutConstants.horizontalRuleHeight
    var verticalPadding: CGFloat = LayoutConstants.horizontalRuleVerticalPadding

    override func attachmentBounds(
        for textContainer: NSTextContainer?,
        proposedLineFragment lineFrag: CGRect,
        glyphPosition position: CGPoint,
        characterIndex charIndex: Int
    ) -> CGRect {
        let totalHeight = lineHeight + (verticalPadding * 2)
        return CGRect(
            x: 0,
            y: 0,
            width: lineFrag.width,
            height: totalHeight
        )
    }

    override func image(
        forBounds imageBounds: CGRect,
        textContainer: NSTextContainer?,
        characterIndex charIndex: Int
    ) -> UIImage? {
        let renderer = UIGraphicsImageRenderer(bounds: imageBounds)
        return renderer.image { context in
            lineColor.setFill()
            let lineY = imageBounds.midY - (lineHeight / 2)
            let lineRect = CGRect(
                x: imageBounds.origin.x,
                y: lineY,
                width: imageBounds.width,
                height: lineHeight
            )
            context.fill(lineRect)
        }
    }
}

final class BlockImageAttachment: NSTextAttachment {
    private let source: String
    private let placeholderTint: UIColor
    private var preferredWidth: CGFloat?
    private var preferredHeight: CGFloat?
    private var loadedImage: UIImage?

    init(
        source: String,
        placeholderTint: UIColor,
        preferredWidth: CGFloat?,
        preferredHeight: CGFloat?
    ) {
        self.source = source
        self.placeholderTint = placeholderTint
        self.preferredWidth = preferredWidth
        self.preferredHeight = preferredHeight
        super.init(data: nil, ofType: nil)
        loadImageIfNeeded()
    }

    required init?(coder: NSCoder) {
        return nil
    }

    func setPreferredSize(width: CGFloat, height: CGFloat) {
        preferredWidth = width
        preferredHeight = height
    }

    func previewImage() -> UIImage? {
        loadedImage ?? image
    }

    override func attachmentBounds(
        for textContainer: NSTextContainer?,
        proposedLineFragment lineFrag: CGRect,
        glyphPosition position: CGPoint,
        characterIndex charIndex: Int
    ) -> CGRect {
        let lineFragmentWidth = lineFrag.width.isFinite ? lineFrag.width : 0
        let containerWidth = textContainer.map {
            max(0, $0.size.width - ($0.lineFragmentPadding * 2))
        } ?? 0
        let widthCandidates = [lineFragmentWidth, containerWidth].filter { $0.isFinite && $0 > 0 }
        let maxWidth = max(160, widthCandidates.min() ?? 160)
        let fallbackAspectRatio = loadedImage.flatMap { image -> CGFloat? in
            let imageSize = image.size
            guard imageSize.width > 0, imageSize.height > 0 else { return nil }
            return imageSize.height / imageSize.width
        } ?? 0.56

        var resolvedWidth = preferredWidth
        var resolvedHeight = preferredHeight

        if resolvedWidth == nil, resolvedHeight == nil, let loadedImage {
            let imageSize = loadedImage.size
            if imageSize.width > 0, imageSize.height > 0 {
                resolvedWidth = imageSize.width
                resolvedHeight = imageSize.height
            }
        } else if resolvedWidth == nil, let resolvedHeight {
            resolvedWidth = resolvedHeight / fallbackAspectRatio
        } else if resolvedHeight == nil, let resolvedWidth {
            resolvedHeight = resolvedWidth * fallbackAspectRatio
        }

        let width = max(1, resolvedWidth ?? maxWidth)
        let height = max(1, resolvedHeight ?? min(180, maxWidth * fallbackAspectRatio))
        let scale = min(1, maxWidth / width)
        return CGRect(x: 0, y: 0, width: width * scale, height: height * scale)
    }

    override func image(
        forBounds imageBounds: CGRect,
        textContainer: NSTextContainer?,
        characterIndex charIndex: Int
    ) -> UIImage? {
        if let loadedImage {
            return loadedImage
        }

        let renderer = UIGraphicsImageRenderer(bounds: imageBounds)
        return renderer.image { _ in
            let path = UIBezierPath(roundedRect: imageBounds, cornerRadius: 12)
            UIColor.secondarySystemFill.setFill()
            path.fill()

            let iconSize = min(imageBounds.width, imageBounds.height) * 0.28
            let iconOrigin = CGPoint(
                x: imageBounds.midX - (iconSize / 2),
                y: imageBounds.midY - (iconSize / 2)
            )
            let iconRect = CGRect(origin: iconOrigin, size: CGSize(width: iconSize, height: iconSize))

            if #available(iOS 13.0, *) {
                let config = UIImage.SymbolConfiguration(pointSize: iconSize, weight: .medium)
                let icon = UIImage(systemName: "photo", withConfiguration: config)?
                    .withTintColor(placeholderTint.withAlphaComponent(0.7), renderingMode: .alwaysOriginal)
                icon?.draw(in: iconRect)
            }
        }
    }

    private func loadImageIfNeeded() {
        if let cached = RenderImageCache.cache.object(forKey: source as NSString) {
            loadedImage = cached
            image = cached
            return
        }

        if let inlineData = Self.decodeDataURL(source),
           let image = UIImage(data: inlineData)
        {
            RenderImageCache.cache.setObject(image, forKey: source as NSString)
            loadedImage = image
            self.image = image
            return
        }

        guard let url = URL(string: source) else { return }
        RenderImageCache.queue.async { [weak self] in
            guard let self else { return }
            let data: Data?
            if url.isFileURL {
                data = try? Data(contentsOf: url)
            } else {
                data = try? Data(contentsOf: url)
            }
            guard let data,
                  let image = UIImage(data: data)
            else {
                return
            }
            RenderImageCache.cache.setObject(image, forKey: self.source as NSString)
            DispatchQueue.main.async {
                self.loadedImage = image
                self.image = image
                NotificationCenter.default.post(name: .editorImageAttachmentDidLoad, object: self)
            }
        }
    }

    private static func decodeDataURL(_ source: String) -> Data? {
        let trimmed = source.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.lowercased().hasPrefix("data:image/"),
              let commaIndex = trimmed.firstIndex(of: ",")
        else {
            return nil
        }
        let metadata = String(trimmed[..<commaIndex]).lowercased()
        let payload = String(trimmed[trimmed.index(after: commaIndex)...])
        guard metadata.contains(";base64") else { return nil }
        return Data(base64Encoded: payload, options: [.ignoreUnknownCharacters])
    }
}
