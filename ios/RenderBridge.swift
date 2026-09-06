import UIKit
import ImageIO
import CryptoKit
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

    static let orderedListMarkerLabel = NSAttributedString.Key("com.apollohg.editor.orderedListMarkerLabel")

    /// Stores the rendered list marker color for the paragraph marker.
    static let listMarkerColor = NSAttributedString.Key("com.apollohg.editor.listMarkerColor")

    /// Stores the rendered list marker scale for unordered bullets.
    static let listMarkerScale = NSAttributedString.Key("com.apollohg.editor.listMarkerScale")
    static let listMarkerGap = NSAttributedString.Key("com.apollohg.editor.listMarkerGap")

    /// Stores the paragraph base font used to render the list marker.
    static let listMarkerBaseFont = NSAttributedString.Key("com.apollohg.editor.listMarkerBaseFont")

    /// Stores the reserved list marker gutter width.
    static let listMarkerWidth = NSAttributedString.Key("com.apollohg.editor.listMarkerWidth")

    /// Stores the rendered blockquote border color.
    static let blockquoteBorderColor = NSAttributedString.Key("com.apollohg.editor.blockquoteBorderColor")

    /// Stores the rendered blockquote border width.
    static let blockquoteBorderWidth = NSAttributedString.Key("com.apollohg.editor.blockquoteBorderWidth")

    /// Stores the rendered blockquote gap between border and text.
    static let blockquoteMarkerGap = NSAttributedString.Key("com.apollohg.editor.blockquoteMarkerGap")

    /// Marks code-block paragraphs for custom background drawing.
    static let codeBlockBackgroundColor = NSAttributedString.Key("com.apollohg.editor.codeBlockBackgroundColor")
    static let codeBlockBorderRadius = NSAttributedString.Key("com.apollohg.editor.codeBlockBorderRadius")
    static let codeBlockPaddingHorizontal = NSAttributedString.Key("com.apollohg.editor.codeBlockPaddingHorizontal")
    static let codeBlockPaddingVertical = NSAttributedString.Key("com.apollohg.editor.codeBlockPaddingVertical")

    /// Marks synthetic zero-width placeholders used only for UIKit layout.
    static let syntheticPlaceholder = NSAttributedString.Key("com.apollohg.editor.syntheticPlaceholder")

    /// Stores the link href for visually styled link text without enabling UITextView's default link interaction.
    static let linkHref = NSAttributedString.Key("com.apollohg.editor.linkHref")

    /// Stores the owning top-level document child index for partial native patching.
    static let topLevelChildIndex = NSAttributedString.Key("com.apollohg.editor.topLevelChildIndex")
}

/// Layout constants for paragraph styles.
enum LayoutConstants {
    /// Spacing between paragraphs (points).
    static let paragraphSpacing: CGFloat = 8.0

    /// Base indentation per depth level (points).
    static let indentPerDepth: CGFloat = 24.0

    /// Width reserved for the list bullet/number (points).
    static let listMarkerWidth: CGFloat = 36.0

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

struct AtomRenderConfiguration: Equatable {
    let registeredNodeTypes: Set<String>
    let estimatedHeights: [String: CGFloat]
    let measuredHeights: [String: CGFloat]

    func reservedHeight(atomKey: String, nodeType: String) -> CGFloat? {
        measuredHeights[atomKey] ?? estimatedHeights[nodeType]
    }

    static func from(json: String?) -> AtomRenderConfiguration? {
        guard let json,
              let data = json.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }

        let nodeTypes = Set((raw["nodeTypes"] as? [Any] ?? []).compactMap { $0 as? String })
        let rawHeights = raw["estimatedHeights"] as? [String: Any] ?? [:]
        var estimatedHeights: [String: CGFloat] = [:]
        for (nodeType, value) in rawHeights {
            guard let number = value as? NSNumber else { continue }
            let height = CGFloat(truncating: number)
            guard height.isFinite, height >= 0 else { continue }
            estimatedHeights[nodeType] = height
        }
        return AtomRenderConfiguration(
            registeredNodeTypes: nodeTypes,
            estimatedHeights: estimatedHeights,
            measuredHeights: [:]
        )
    }
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
        theme: EditorTheme? = nil,
        atomConfiguration: AtomRenderConfiguration? = nil
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
            theme: theme,
            atomConfiguration: atomConfiguration
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
        theme: EditorTheme? = nil,
        atomConfiguration: AtomRenderConfiguration? = nil
    ) -> NSAttributedString {
        let result = NSMutableAttributedString()
        var blockStack: [BlockContext] = []
        var isFirstBlock = true
        var pendingTrailingParagraphSpacing: CGFloat? = nil
        var atomOccurrences: [String: Int] = [:]

        for element in elements {
            guard let type = element["type"] as? String else { continue }
            let topLevelChildIndex = jsonInt(element["topLevelChildIndex"])

            switch type {
            case "textRun":
                let text = element["text"] as? String ?? ""
                let marks = element["marks"] as? [Any] ?? []
                let isCodeBlock = blockStack.last?.nodeType == "codeBlock"
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
                var baseAttrs = attributesForMarks(
                    marks,
                    baseFont: blockFont,
                    textColor: blockColor,
                    theme: theme
                )
                if let sheet = theme?.styleSheet {
                    var base = defaultAttributes(baseFont: blockFont, textColor: blockColor)
                    EditorStyleSheet.applyText(sheet.textValues(blockStack.last?.nodeType ?? "paragraph", ancestors: blockStack.dropLast().map(\.nodeType)), to: &base)
                    baseAttrs = sheet.inlineAttributes(marks, base: base)
                }
                if isCodeBlock {
                    // blockFont already carries theme.codeBlock.text. Keep an
                    // explicit code-block family for ordinary marked text;
                    // otherwise use the shared monospace resolver. In either
                    // case, the resolver accepts a face only if it satisfies
                    // the complete bold/italic request.
                    let resolvedFont = baseAttrs[.font] as? UIFont ?? blockFont
                    let markTraits = resolvedFont.fontDescriptor.symbolicTraits
                        .intersection([.traitBold, .traitItalic])
                    let themedFamily = theme?.codeBlock?.text?.fontFamily != nil
                    baseAttrs[.font] = ViewerFontEnvironment.shared.resolveFont(
                        family: themedFamily ? nil : "monospace",
                        size: resolvedFont.pointSize,
                        fallback: resolvedFont,
                        additionalTraits: markTraits,
                        semanticGeneration: "legacy-editor-theme"
                    )
                }
                let attrs = applyBlockStyle(
                    to: baseAttrs,
                    blockStack: blockStack,
                    theme: theme,
                    blockBaseFont: blockFont
                )
                let attributedText = NSAttributedString(string: text, attributes: attrs)
                result.append(
                    attributedStringApplyingLeadingTopLevelChildIndexIfNeeded(
                        attributedText,
                        topLevelChildIndex: topLevelChildIndex,
                        resultIsEmpty: result.length == 0
                    )
                )

            case "voidInline":
                let nodeType = element["nodeType"] as? String ?? ""
                guard let docPos = jsonUInt32(element["docPos"]) else { continue }
                let attrs = element["attrs"] as? [String: Any] ?? [:]
                if EditorNodeTypes.isHardBreak(nodeType) {
                    overrideTrailingParagraphSpacing(in: result, paragraphSpacing: 0)
                }
                let attrStr = attributedStringForVoidInline(
                    nodeType: nodeType,
                    docPos: docPos,
                    attrs: attrs,
                    baseFont: baseFont,
                    textColor: textColor,
                    blockStack: blockStack,
                    topLevelChildIndex: topLevelChildIndex,
                    theme: theme
                )
                result.append(
                    attributedStringApplyingLeadingTopLevelChildIndexIfNeeded(
                        attrStr,
                        topLevelChildIndex: topLevelChildIndex,
                        resultIsEmpty: result.length == 0
                    )
                )

            case "voidBlock":
                let nodeType = element["nodeType"] as? String ?? ""
                guard let docPos = jsonUInt32(element["docPos"]) else { continue }
                let attrs = element["attrs"] as? [String: Any] ?? [:]
                let occurrence = atomOccurrences[nodeType, default: 0]
                atomOccurrences[nodeType] = occurrence + 1
                let atomKey = (element["atomId"] as? String) ?? "\(nodeType):\(occurrence)"

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
                            theme: theme,
                            topLevelChildIndex: topLevelChildIndex
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
                    topLevelChildIndex: topLevelChildIndex,
                    theme: theme,
                    atomKey: atomKey,
                    atomConfiguration: atomConfiguration
                )
                if let sheet = theme?.styleSheet {
                    let styled = NSMutableAttributedString(attributedString: attrStr)
                    styled.addAttribute(editorStyledContentAttribute, value: true, range: NSRange(location: 0, length: styled.length))
                    let context = BlockContext(nodeType: nodeType, depth: blockStack.last?.depth ?? 0, listContext: nil)
                    let style = paragraphStyleForBlock(context, blockStack: blockStack + [context], theme: theme, baseFont: baseFont)
                    let inset = sheet.box(nodeType).inset
                    style.headIndent -= inset.left
                    style.firstLineHeadIndent -= inset.left
                    style.tailIndent += inset.right
                    style.paragraphSpacingBefore = sheet.box(nodeType).margin.top
                    style.paragraphSpacing = sheet.box(nodeType).margin.bottom
                    styled.addAttribute(.paragraphStyle, value: style, range: NSRange(location: 0, length: styled.length))
                    result.append(styled)
                } else { result.append(attrStr) }
                pendingTrailingParagraphSpacing = theme?.effectiveTextStyle(
                    for: nodeType
                ).spacingAfter

            case "opaqueInlineAtom":
                let nodeType = element["nodeType"] as? String ?? ""
                let label = element["label"] as? String ?? "?"
                guard let docPos = jsonUInt32(element["docPos"]) else { continue }
                let mentionTheme = (element["mentionTheme"] as? [String: Any]).map(
                    EditorMentionTheme.init(dictionary:)
                )
                let attrStr = attributedStringForOpaqueInlineAtom(
                    nodeType: nodeType,
                    label: label,
                    docPos: docPos,
                    baseFont: baseFont,
                    textColor: textColor,
                    blockStack: blockStack,
                    topLevelChildIndex: topLevelChildIndex,
                    theme: theme,
                    mentionTheme: mentionTheme
                )
                result.append(
                    attributedStringApplyingLeadingTopLevelChildIndexIfNeeded(
                        attrStr,
                        topLevelChildIndex: topLevelChildIndex,
                        resultIsEmpty: result.length == 0
                    )
                )

            case "opaqueBlockAtom":
                let nodeType = element["nodeType"] as? String ?? ""
                let label = element["label"] as? String ?? "?"
                guard let docPos = jsonUInt32(element["docPos"]) else { continue }

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
                            theme: theme,
                            topLevelChildIndex: topLevelChildIndex
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
                    topLevelChildIndex: topLevelChildIndex,
                    theme: theme
                )
                result.append(attrStr)

            case "blockStart":
                let nodeType = element["nodeType"] as? String ?? ""
                let depth = jsonUInt8(element["depth"])
                let listContext = element["listContext"] as? [String: Any]
                let isListItemContainer = isListItemNodeType(nodeType) && listContext != nil
                let isTransparentLayoutContainer = isTransparentContainer(nodeType)
                var ctx = BlockContext(
                    nodeType: nodeType,
                    depth: depth,
                    listContext: listContext,
                    topLevelChildIndex: topLevelChildIndex,
                    markerPending: isListItemContainer,
                    language: element["language"] as? String
                )
                let nestedListItemContainer =
                    isListItemContainer && (theme?.list?.itemSpacing != nil)
                    && blockStack.contains(where: {
                        isListItemNodeType($0.nodeType) && $0.listContext != nil
                    })

                if !isListItemContainer && !isTransparentLayoutContainer {
                    // Add inter-block newline before non-first rendered blocks.
                    if !isFirstBlock {
                        applyPendingTrailingParagraphSpacing(
                            in: result,
                            pendingParagraphSpacing: &pendingTrailingParagraphSpacing
                        )
                        let newlineBlockStack: [BlockContext]
                        if ctx.nodeType == "codeBlock" {
                            newlineBlockStack = []
                        } else if blockquoteDepth(in: blockStack + [ctx]) > 0,
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
                                paragraphSpacingOverride: collapsedSeparatorSpacing,
                                topLevelChildIndex: topLevelChildIndex
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

                if theme?.styleSheet != nil, isListItemContainer,
                   (listContext?["isFirst"] as? Bool) == true || (listContext?["index"] as? NSNumber)?.intValue == 1 {
                    let listName = (listContext?["kind"] as? String) == "task" ? "taskList" : ((listContext?["ordered"] as? Bool) == true ? "orderedList" : "bulletList")
                    blockStack.append(BlockContext(nodeType: listName, depth: depth, listContext: nil, styleStart: result.length))
                }
                ctx.styleStart = result.length

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
                    closeStyledBlock(endedBlock, ancestors: blockStack, in: result, theme: theme, baseFont: baseFont, textColor: textColor)
                    if EditorStyleSheet.element(endedBlock.nodeType) == "codeBlock", endedBlock.styleStart < result.length {
                        result.addAttribute(editorCodeBlockAttribute, value: EditorCodeBlockPresentation(language: endedBlock.language), range: NSRange(location: endedBlock.styleStart, length: result.length - endedBlock.styleStart))
                    }
                    if theme?.styleSheet != nil, endedBlock.listContext?["isLast"] as? Bool == true,
                       let container = blockStack.last,
                       ["bulletList", "orderedList", "taskList"].contains(container.nodeType) {
                        blockStack.removeLast()
                        closeStyledBlock(container, ancestors: blockStack, in: result, theme: theme, baseFont: baseFont, textColor: textColor)
                    }
                    if endedBlock.listContext != nil, theme?.styleSheet == nil {
                        let spacing = (endedBlock.listContext?["isLast"] as? Bool) == true
                            ? (theme?.list?.spacingAfter ?? theme?.list?.itemSpacing)
                            : theme?.list?.itemSpacing
                        if let spacing {
                            pendingTrailingParagraphSpacing = (pendingTrailingParagraphSpacing ?? 0) + spacing
                        }
                    }
                }

            default:
                break
            }
        }

        if theme?.styleSheet != nil, result.length > 1 {
            result.enumerateAttribute(RenderBridgeAttributes.blockBoundary, in: NSRange(location: 1, length: result.length - 1)) { value, range, _ in
                guard value != nil, let style = result.attribute(.paragraphStyle, at: range.location - 1, effectiveRange: nil) else { return }
                result.addAttribute(.paragraphStyle, value: style, range: range)
            }
        }
        return result
    }

    static func renderBlocks(
        fromArray blocks: [[[String: Any]]],
        startIndex: Int = 0,
        includeLeadingInterBlockSeparator: Bool = false,
        includeTrailingInterBlockSeparator: Bool = false,
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme? = nil,
        atomConfiguration: AtomRenderConfiguration? = nil
    ) -> NSAttributedString {
        var flattened: [[String: Any]] = []
        flattened.reserveCapacity(blocks.reduce(0) { $0 + $1.count })

        for (offset, block) in blocks.enumerated() {
            let topLevelChildIndex = startIndex + offset
            for element in block {
                var tagged = element
                tagged["topLevelChildIndex"] = topLevelChildIndex
                flattened.append(tagged)
            }
        }

        let renderedBlocks = renderElements(
            fromArray: flattened,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme,
            atomConfiguration: atomConfiguration
        )
        let needsLeadingInterBlockSeparator = includeLeadingInterBlockSeparator && startIndex > 0
        guard !blocks.isEmpty,
              needsLeadingInterBlockSeparator || includeTrailingInterBlockSeparator
        else {
            return renderedBlocks
        }

        let result = NSMutableAttributedString()
        if needsLeadingInterBlockSeparator {
            result.append(
                interBlockNewline(
                    baseFont: baseFont,
                    textColor: textColor,
                    blockStack: [],
                    theme: theme,
                    topLevelChildIndex: startIndex
                )
            )
            result.append(
                removingLeadingTopLevelChildIndex(
                    from: renderedBlocks,
                    topLevelChildIndex: startIndex
                )
            )
        } else {
            result.append(renderedBlocks)
        }
        if includeTrailingInterBlockSeparator {
            result.append(
                interBlockNewline(
                    baseFont: baseFont,
                    textColor: textColor,
                    blockStack: [],
                    theme: theme,
                    topLevelChildIndex: startIndex + blocks.count
                )
            )
        }
        return result
    }

    // MARK: - Height Pre-Measurement

    static func measureHeight(
        forRenderJSON renderJSON: String,
        themeJSON: String?,
        width: CGFloat
    ) -> CGFloat {
        if !Thread.isMainThread {
            return DispatchQueue.main.sync {
                measureHeight(
                    forRenderJSON: renderJSON,
                    themeJSON: themeJSON,
                    width: width
                )
            }
        }
        guard width > 0 else { return 0 }

        let theme = EditorTheme.from(json: themeJSON)
        let baseFontSize = theme?.text?.fontSize ?? theme?.paragraph?.fontSize ?? 16
        let baseFont = UIFont.systemFont(ofSize: baseFontSize)
        let textColor = theme?.text?.color ?? UIColor.label

        let attributedString = NativeImagePipeline.withoutLoading {
            renderElements(
                fromJSON: renderJSON,
                baseFont: baseFont,
                textColor: textColor,
                theme: theme
            )
        }

        guard attributedString.length > 0 else { return 0 }

        let contentInsets = theme?.contentInsets
        let topInset = contentInsets?.top ?? 0
        let bottomInset = contentInsets?.bottom ?? 0
        let leftInset = contentInsets?.left ?? 0
        let rightInset = contentInsets?.right ?? 0

        // When contentInsets are set, lineFragmentPadding is 0 (matches
        // RichTextEditorView.theme didSet). Otherwise use the UITextView
        // default of 5.
        let lineFragmentPadding: CGFloat = contentInsets != nil ? 0 : 5

        let textStorage = NSTextStorage(attributedString: attributedString)
        let layoutManager = EditorLayoutManager()
        let containerWidth = width - leftInset - rightInset - lineFragmentPadding * 2
        let textContainer = NSTextContainer(
            size: CGSize(width: max(containerWidth, 0), height: .greatestFiniteMagnitude)
        )
        textContainer.lineFragmentPadding = 0

        layoutManager.addTextContainer(textContainer)
        textStorage.addLayoutManager(layoutManager)

        layoutManager.ensureLayout(for: textContainer)

        var usedRect = layoutManager.usedRect(for: textContainer)
        let extraLineFragmentRect = layoutManager.extraLineFragmentRect
        if !extraLineFragmentRect.isEmpty {
            usedRect = usedRect.union(extraLineFragmentRect)
        }

        let height = ceil(usedRect.height + topInset + bottomInset)
        return height
    }

    // MARK: - Mark Handling

    // MARK: - Void Inline Elements

    // MARK: - Void Block Elements

    // MARK: - Opaque Atoms

    // MARK: - Block Styling

    // MARK: - List Markers

    // MARK: - Private Helpers

}

// MARK: - BlockContext

/// Transient context while rendering block elements. Pushed onto a stack
/// when a `blockStart` element is encountered and popped on `blockEnd`.
