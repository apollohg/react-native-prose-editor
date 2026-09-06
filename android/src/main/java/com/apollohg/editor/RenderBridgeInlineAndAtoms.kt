package com.apollohg.editor

import android.graphics.Color
import android.graphics.Typeface
import android.text.Annotation
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.style.AbsoluteSizeSpan
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.StrikethroughSpan
import android.text.style.StyleSpan
import android.text.style.TypefaceSpan
import android.text.style.UnderlineSpan
import android.view.View
import org.json.JSONObject

/**
     * Apply spans to a text run based on its mark names and append to the builder.
     *
     * Supported marks:
     * - `bold` / `strong` -> [StyleSpan] with [Typeface.BOLD]
     * - `italic` / `em` -> [StyleSpan] with [Typeface.ITALIC]
     * - `underline` -> [UnderlineSpan]
     * - `strike` / `strikethrough` -> [StrikethroughSpan]
     * - `code` -> [TypefaceSpan] with "monospace" + [BackgroundColorSpan]
     * - `link` -> [URLSpan] (when mark is an object with `href`)
     *
     * Multiple marks are combined on the same range.
     */
internal fun RenderBridge.appendStyledText(
    builder: SpannableStringBuilder,
    text: String,
    marks: List<Any>, // String or JSONObject for link marks
    baseFontSize: Float,
    textColor: Int,
    blockStack: MutableList<BlockContext>,
    pendingLeadingMargins: MutableMap<Int, PendingLeadingMargin>,
    theme: EditorTheme?,
    density: Float,
    applyBlockSpans: Boolean = true
) {
    val start = builder.length
    builder.append(text)
    val end = builder.length

    if (start == end) return

    theme?.styleSheet?.let { sheet ->
        val node = blockStack.lastOrNull()?.nodeType ?: "paragraph"
        val style = sheet.resolveText(node, blockStack.dropLast(1).map { it.nodeType }, marks.mapNotNull {
            when (it) { is String -> it; is JSONObject -> it.optString("type"); else -> null }
        })
        builder.setSpan(EditorResolvedTextSpan(EditorTextStyle(fontSize = baseFontSize / density, color = textColor).mergedWith(style), density), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        marks.filterIsInstance<JSONObject>().firstOrNull { it.optString("type") == "link" }?.optNullableString("href")?.let {
            builder.setSpan(Annotation(NATIVE_LINK_HREF_ANNOTATION, it), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }
        if (applyBlockSpans) applyBlockStyle(builder, start, end, blockStack, pendingLeadingMargins, theme, density)
        return
    }

    val currentBlock = effectiveBlockContext(blockStack)
    val isCodeBlock = currentBlock?.nodeType == "codeBlock"
    val textStyle = currentBlock?.let {
        resolveTextStyle(
            it.nodeType,
            theme,
            blockquoteDepth(blockStack) > 0
        )
    } ?: theme?.effectiveTextStyle("paragraph", inBlockquote = blockquoteDepth(blockStack) > 0)

    // Determine which marks are active.
    var markBold = false
    var markItalic = false
    var markUnderline = false
    var hasStrike = false
    var hasCode = false
    var isLink = false
    var linkHref: String? = null
    for (mark in marks) {
        when {
            mark is String -> when (mark) {
                "bold", "strong" -> markBold = true
                "italic", "em" -> markItalic = true
                "underline" -> markUnderline = true
                "strike", "strikethrough" -> hasStrike = true
                "code" -> hasCode = true
            }
            mark is JSONObject -> {
                val markType = mark.optString("type", "")
                if (markType == "link") {
                    isLink = true
                    linkHref = mark.optString("href", "").takeIf { it.isNotBlank() }
                }
            }
        }
    }
    val linkTheme = if (isLink) theme?.links else null
    val effectiveTextStyle = textStyle?.mergedWith(linkTheme?.asTextStyle())
        ?: linkTheme?.asTextStyle()
    val resolvedTextSize = effectiveTextStyle?.fontSize?.times(density) ?: baseFontSize
    val resolvedTextColor = if (isLink) {
        effectiveTextStyle?.color ?: LayoutConstants.DEFAULT_LINK_COLOR
    } else {
        effectiveTextStyle?.color ?: textColor
    }

    // Apply base styling.
    builder.setSpan(
        ForegroundColorSpan(resolvedTextColor),
        start, end,
        Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    builder.setSpan(
        AbsoluteSizeSpan(resolvedTextSize.toInt(), false),
        start, end,
        Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    linkTheme?.backgroundColor?.let { backgroundColor ->
        builder.setSpan(
            BackgroundColorSpan(backgroundColor),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }
    linkHref?.let { href ->
        builder.setSpan(
            Annotation(NATIVE_LINK_HREF_ANNOTATION, href),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }

    val typefaceStyle = effectiveTextStyle?.typefaceStyle()
    val hasBold = markBold ||
        typefaceStyle?.let { it == Typeface.BOLD || it == Typeface.BOLD_ITALIC } == true
    val hasItalic = markItalic ||
        typefaceStyle?.let { it == Typeface.ITALIC || it == Typeface.BOLD_ITALIC } == true
    val hasUnderline = markUnderline || (isLink && (linkTheme?.underline ?: true))

    // Apply bold/italic as a combined StyleSpan.
    if (hasBold && hasItalic) {
        builder.setSpan(
            StyleSpan(Typeface.BOLD_ITALIC), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    } else if (hasBold) {
        builder.setSpan(
            StyleSpan(Typeface.BOLD), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    } else if (hasItalic) {
        builder.setSpan(
            StyleSpan(Typeface.ITALIC), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }

    val fontFamily = effectiveTextStyle?.fontFamily
    if (!hasCode && !isCodeBlock && !fontFamily.isNullOrBlank()) {
        builder.setSpan(
            TypefaceSpan(fontFamily),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }

    if (hasUnderline) {
        builder.setSpan(UnderlineSpan(), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
    }

    if (hasStrike) {
        builder.setSpan(StrikethroughSpan(), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
    }

    if (hasCode || isCodeBlock) {
        builder.setSpan(
            TypefaceSpan("monospace"), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        if (hasCode && !isCodeBlock) {
            builder.setSpan(
                BackgroundColorSpan(LayoutConstants.CODE_BACKGROUND_COLOR),
                start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
        }
    }

    // Apply block-level indentation spans if in a block context.
    if (applyBlockSpans) {
        applyBlockStyle(builder, start, end, blockStack, pendingLeadingMargins, theme, density)
    }
}

    /**
     * Append a void inline element (e.g. hardBreak) to the builder.
     *
     * A hardBreak is rendered as a newline character. Unknown void inlines
     * are rendered as the object replacement character.
     */
internal fun RenderBridge.appendVoidInline(
    builder: SpannableStringBuilder,
    nodeType: String,
    baseFontSize: Float,
    textColor: Int,
    blockStack: MutableList<BlockContext>,
    pendingLeadingMargins: MutableMap<Int, PendingLeadingMargin>,
    theme: EditorTheme?,
    density: Float
) {
    when (nodeType) {
        "hardBreak", "hard_break" -> {
            val start = builder.length
            builder.append("\n")
            val end = builder.length
            builder.setSpan(
                Annotation("nativeVoidNodeType", nodeType),
                start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
            builder.setSpan(
                ForegroundColorSpan(resolveInlineTextColor(blockStack, textColor, theme)),
                start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
            applyBlockStyle(builder, start, end, blockStack, pendingLeadingMargins, theme, density)
        }
        else -> {
            val start = builder.length
            builder.append(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER)
            val end = builder.length
            builder.setSpan(
                ForegroundColorSpan(resolveInlineTextColor(blockStack, textColor, theme)),
                start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
            applyBlockStyle(builder, start, end, blockStack, pendingLeadingMargins, theme, density)
        }
    }
}

    /**
     * Append a void block element (e.g. horizontalRule) to the builder.
     *
     * Horizontal rules are rendered as the object replacement character
     * with a [HorizontalRuleSpan] that draws a separator line.
     */
internal fun RenderBridge.appendVoidBlock(
    builder: SpannableStringBuilder,
    nodeType: String,
    attrs: JSONObject?,
    baseFontSize: Float,
    textColor: Int,
    theme: EditorTheme?,
    density: Float,
    spacingBefore: Float?,
    hostView: View?,
    topLevelChildIndex: Int?,
    atomConfiguration: AtomRenderConfiguration?,
    atomKey: String,
    docPos: Int?,
    hasStableAtomId: Boolean,
    isDirectRootChild: Boolean,
    reusableImages: MutableList<BlockImageSpan> = mutableListOf(),
    ancestorBoxInset: EditorEdges = EditorEdges(),
    containerDepth: Int = 0,
) {
    if (docPos != null && atomConfiguration?.registeredNodeTypes?.contains(nodeType) == true) {
        val start = builder.length
        builder.append(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER)
        val end = builder.length
        builder.setSpan(
            AtomBlockSpan(
                atomKey,
                nodeType,
                docPos,
                atomConfiguration.reservedHeightPx(atomKey, nodeType, density),
                hasStableAtomId,
                isDirectRootChild,
            ),
            start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        builder.setSpan(
            Annotation("nativeVoidNodeType", nodeType),
            start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        builder.setSpan(
            Annotation("nativeDocPos", docPos.toUInt().toString()),
            start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        annotateTopLevelChild(builder, start, end, topLevelChildIndex)
        return
    }
    when (nodeType) {
        "horizontalRule", "horizontal_rule" -> {
            val start = builder.length
            builder.append(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER)
            val end = builder.length
            // Apply a dim version of the text color for the rule line.
            val ruleColor = theme?.horizontalRule?.color ?: Color.argb(
                (Color.alpha(textColor) * 0.3f).toInt(),
                Color.red(textColor),
                Color.green(textColor),
                Color.blue(textColor)
            )
            builder.setSpan(
                HorizontalRuleSpan(
                    lineColor = ruleColor,
                    lineHeight = (theme?.horizontalRule?.thickness ?: LayoutConstants.HORIZONTAL_RULE_HEIGHT) * density,
                    verticalPadding = (theme?.horizontalRule?.verticalMargin ?: LayoutConstants.HORIZONTAL_RULE_VERTICAL_PADDING) * density,
                    boxInset = theme?.styleSheet?.box("horizontalRule")?.outerInset?.scaled(density) ?: EditorEdges()
                ),
                start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
            theme?.styleSheet?.let {
                builder.setSpan(EditorBlockBoxSpan(it.box("horizontalRule").scaled(density), ancestorBoxInset, containerDepth, "horizontalRule"), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            }
            annotateTopLevelChild(builder, start, end, topLevelChildIndex)
        }
        "image" -> {
            val source = if (attrs != null && attrs.has("src") && !attrs.isNull("src")) {
                attrs.optString("src", "")
            } else {
                ""
            }
            val preferredWidthDp = attrs?.optPositiveFiniteFloat("width")
            val preferredHeightDp = attrs?.optPositiveFiniteFloat("height")
            if (source.isEmpty()) {
                builder.append(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER)
                return
            }
            val start = builder.length
            builder.append(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER)
            val end = builder.length
            val imageStyle = theme?.styleSheet?.let { it["image"] ?: EditorElementStyle(EditorTextStyle(), it.box("image")) }
            val reused = reusableImages.firstOrNull { it.matches(source, preferredWidthDp, preferredHeightDp) }
            val span = reused?.also { reusableImages.remove(it); it.imageStyle = imageStyle } ?: BlockImageSpan(source, hostView, density, preferredWidthDp, preferredHeightDp, imageStyle)
            span.ancestorWidthInset = ancestorBoxInset.left + ancestorBoxInset.right
            builder.setSpan(span, start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            imageStyle?.let {
                // Image margins already occupy space in the replacement span.
                builder.setSpan(EditorBlockBoxSpan(EditorBoxStyle(), ancestorBoxInset, containerDepth, "image", it.box.margin.scaled(density)), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            }
            annotateTopLevelChild(builder, start, end, topLevelChildIndex)
        }
        else -> {
            val start = builder.length
            builder.append(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER)
            val end = builder.length
            annotateTopLevelChild(builder, start, end, topLevelChildIndex)
        }
    }
}

internal fun RenderBridge.appendOpaqueInlineAtom(
    builder: SpannableStringBuilder,
    nodeType: String,
    label: String,
    docPos: Long,
    baseFontSize: Float,
    textColor: Int,
    blockStack: MutableList<BlockContext>,
    pendingLeadingMargins: MutableMap<Int, PendingLeadingMargin>,
    theme: EditorTheme?,
    mentionTheme: EditorMentionTheme?,
    density: Float
) {
    val isMention = nodeType == "mention"
    val text = if (isMention) label else "[$label]"
    val start = builder.length
    builder.append(text)
    val end = builder.length
    if (isMention && theme?.styleSheet != null) {
        val base = EditorTextStyle(fontSize = baseFontSize / density, color = textColor)
            .mergedWith(theme.styleSheet.resolveText(blockStack.lastOrNull()?.nodeType ?: "paragraph", blockStack.dropLast(1).map { it.nodeType }))
        builder.setSpan(EditorMentionSpan(resolvedMentionStyle(base, theme, mentionTheme), density), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        builder.setSpan(Annotation("nativeVoidNodeType", nodeType), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        builder.setSpan(Annotation("nativeDocPos", docPos.toString()), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        applyBlockStyle(builder, start, end, blockStack, pendingLeadingMargins, theme, density)
        return
    }
    val resolvedMentionTheme = if (isMention) {
        (theme?.mentions?.mergedWith(mentionTheme) ?: mentionTheme)?.node
    } else {
        null
    }
    // Atoms carry no marks, so the block text style is their only typography
    // source; a mention theme weight overrides that block weight.
    val inlineTextStyle = resolveInlineTextStyle(blockStack, theme).mergedWith(
        resolvedMentionTheme?.fontWeight?.let { EditorTextStyle(fontWeight = it) }
    )
    val inlineTextColor = if (isMention) {
        resolvedMentionTheme?.textColor ?: inlineTextStyle.color ?: textColor
    } else {
        inlineTextStyle.color ?: textColor
    }
    builder.setSpan(
        ForegroundColorSpan(inlineTextColor),
        start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    builder.setSpan(
        AbsoluteSizeSpan(
            (inlineTextStyle.fontSize?.times(density) ?: baseFontSize).toInt(),
            false
        ),
        start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    inlineTextStyle.fontFamily?.takeIf { it.isNotBlank() }?.let { fontFamily ->
        builder.setSpan(
            TypefaceSpan(fontFamily),
            start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }
    builder.setSpan(
        BackgroundColorSpan(
            if (isMention) {
                resolvedMentionTheme?.backgroundColor ?: 0x1f1d4ed8
            } else {
                0x20000000
            }
        ),
        start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    builder.setSpan(
        Annotation("nativeVoidNodeType", nodeType),
        start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    builder.setSpan(
        Annotation("nativeDocPos", docPos.toString()),
        start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    val typefaceStyle = inlineTextStyle.typefaceStyle()
    if (typefaceStyle != Typeface.NORMAL) {
        builder.setSpan(
            StyleSpan(typefaceStyle),
            start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }
    applyBlockStyle(builder, start, end, blockStack, pendingLeadingMargins, theme, density)
}

internal fun RenderBridge.appendOpaqueBlockAtom(
    builder: SpannableStringBuilder,
    nodeType: String,
    label: String,
    docPos: Long,
    baseFontSize: Float,
    textColor: Int,
    theme: EditorTheme?,
    spacingBefore: Float?,
    topLevelChildIndex: Int?
) {
    val text = if (nodeType == "mention") label else "[$label]"
    val start = builder.length
    builder.append(text)
    val end = builder.length
    builder.setSpan(
        ForegroundColorSpan(theme?.effectiveTextStyle("paragraph")?.color ?: textColor),
        start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    builder.setSpan(
        BackgroundColorSpan(0x20000000), // light gray
        start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    builder.setSpan(
        Annotation("nativeVoidNodeType", nodeType),
        start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    builder.setSpan(
        Annotation("nativeDocPos", docPos.toString()),
        start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    annotateTopLevelChild(builder, start, end, topLevelChildIndex)
}
