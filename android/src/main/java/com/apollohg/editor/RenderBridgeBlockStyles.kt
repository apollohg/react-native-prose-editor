package com.apollohg.editor

import android.graphics.Color
import android.text.Annotation
import android.text.Layout
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.TextPaint
import android.text.style.LeadingMarginSpan
import org.json.JSONObject

internal fun RenderBridge.applyBlockStyle(
    builder: SpannableStringBuilder,
    start: Int,
    end: Int,
    blockStack: List<BlockContext>,
    pendingLeadingMargins: MutableMap<Int, PendingLeadingMargin>,
    theme: EditorTheme?,
    density: Float
) {
    val currentBlock = effectiveBlockContext(blockStack) ?: return
    val indent = calculateIndent(currentBlock, blockStack, theme, density)
    val quoteDepth = blockquoteDepth(blockStack)
    val indentPerDepth = (theme?.list?.indent ?: LayoutConstants.INDENT_PER_DEPTH) * density
    val listBaseIndentAdjustment =
        calculateListBaseIndentAdjustment(currentBlock, theme, density)
    val quoteStripeColor = if (quoteDepth > 0 && theme?.styleSheet == null) {
        theme?.blockquote?.borderColor ?: Color.argb(
            (Color.alpha(resolveInlineTextColor(blockStack, Color.BLACK, theme)) * 0.3f).toInt(),
            Color.red(resolveInlineTextColor(blockStack, Color.BLACK, theme)),
            Color.green(resolveInlineTextColor(blockStack, Color.BLACK, theme)),
            Color.blue(resolveInlineTextColor(blockStack, Color.BLACK, theme))
        )
    } else {
        null
    }
    val quoteStripeWidth = ((theme?.blockquote?.borderWidth
        ?: LayoutConstants.BLOCKQUOTE_BORDER_WIDTH) * density).toInt()
    val quoteGapWidth = ((theme?.blockquote?.markerGap
        ?: LayoutConstants.BLOCKQUOTE_MARKER_GAP) * density).toInt()
    val quoteIndent = maxOf(
        theme?.blockquote?.indent ?: LayoutConstants.BLOCKQUOTE_INDENT,
        (theme?.blockquote?.markerGap ?: LayoutConstants.BLOCKQUOTE_MARKER_GAP) +
            (theme?.blockquote?.borderWidth ?: LayoutConstants.BLOCKQUOTE_BORDER_WIDTH)
    ) * density
    val blockquoteIndentPx = (quoteDepth * quoteIndent).toInt()
    val quoteBaseIndent = if (quoteDepth > 0) {
        ((currentBlock.depth * indentPerDepth)
            - (quoteDepth * indentPerDepth)
            + listBaseIndentAdjustment
            + ((quoteDepth - 1f) * quoteIndent)).toInt()
    } else {
        0
    }
    val paragraphStart = renderedParagraphStart(
        builder = builder,
        candidateStart = effectiveParagraphStart(blockStack)
    )
    if (paragraphStart < end) {
        if (currentBlock.listContext != null) {
            pendingLeadingMargins[paragraphStart] = PendingLeadingMargin(
                indentPx = indent.toInt(),
                restIndentPx = indent.toInt() + renderedListMarkerWidth(builder, paragraphStart),
                blockquoteIndentPx = blockquoteIndentPx,
                blockquoteStripeColor = quoteStripeColor,
                blockquoteStripeWidthPx = quoteStripeWidth,
                blockquoteGapWidthPx = quoteGapWidth,
                blockquoteBaseIndentPx = quoteBaseIndent
            )
        } else if (indent > 0) {
            pendingLeadingMargins[paragraphStart] = PendingLeadingMargin(
                indentPx = indent.toInt(),
                restIndentPx = null,
                blockquoteIndentPx = blockquoteIndentPx,
                blockquoteStripeColor = quoteStripeColor,
                blockquoteStripeWidthPx = quoteStripeWidth,
                blockquoteGapWidthPx = quoteGapWidth,
                blockquoteBaseIndentPx = quoteBaseIndent
            )
        }
    }

    if (quoteDepth > 0f) {
        builder.setSpan(
            Annotation(NATIVE_BLOCKQUOTE_ANNOTATION, "1"),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }
    annotateTopLevelChild(builder, start, end, currentBlock.topLevelChildIndex)

    val lineHeight = theme?.styleSheet?.resolveText(currentBlock.nodeType, blockStack.dropLast(1).map { it.nodeType })?.lineHeight ?: resolveTextStyle(
        currentBlock.nodeType,
        theme,
        quoteDepth > 0
    ).lineHeight
    applyLineHeightSpan(builder, start, end, lineHeight, density)
}

private fun renderedListMarkerWidth(builder: Spanned, paragraphStart: Int): Int {
    val marker = builder.getSpans(paragraphStart, paragraphStart + 1, Annotation::class.java)
        .firstOrNull { it.key == RenderBridge.NATIVE_LIST_MARKER_ANNOTATION }
        ?: return 0
    val markerText = SpannableStringBuilder(builder, builder.getSpanStart(marker), builder.getSpanEnd(marker))
    // Desired width includes paragraph margins unless they are removed.
    markerText.getSpans(0, markerText.length, LeadingMarginSpan::class.java).forEach(markerText::removeSpan)
    val width = Layout.getDesiredWidth(markerText, TextPaint())
    return kotlin.math.ceil(width.toDouble()).toInt()
}

internal fun RenderBridge.applyLineHeightSpan(
    builder: SpannableStringBuilder,
    start: Int,
    end: Int,
    lineHeight: Float?,
    density: Float
) {
    if (lineHeight == null || lineHeight <= 0 || start >= end) {
        return
    }
    builder.setSpan(
        FixedLineHeightSpan((lineHeight * density).toInt()),
        start,
        end,
        Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
}

internal fun RenderBridge.applyPendingLeadingMargins(
    builder: SpannableStringBuilder,
    pendingLeadingMargins: Map<Int, PendingLeadingMargin>
) {
    if (pendingLeadingMargins.isEmpty()) return

    val text = builder.toString()
    val entries = pendingLeadingMargins.toSortedMap().entries.toList()
    var index = 0
    while (index < entries.size) {
        val paragraphStart = entries[index].key
        val spec = entries[index].value
        if (paragraphStart >= builder.length) {
            index += 1
            continue
        }
        if (spec.blockquoteStripeColor != null) {
            val paragraphEnd = blockquoteSpanEnd(builder, text, paragraphStart)
            val quoteEntries = mutableListOf(entries[index])
            var nextIndex = index + 1
            while (nextIndex < entries.size && entries[nextIndex].key < paragraphEnd) {
                quoteEntries.add(entries[nextIndex])
                nextIndex += 1
            }
            index = nextIndex

            builder
                .getSpans(0, builder.length, LeadingMarginSpan::class.java)
                .filter { it !is EditorBlockBoxSpan && builder.getSpanStart(it) == paragraphStart }
                .forEach(builder::removeSpan)

            builder.setSpan(
                BlockquoteSpan(
                    baseIndentPx = spec.blockquoteBaseIndentPx,
                    totalIndentPx = spec.blockquoteIndentPx,
                    stripeColor = spec.blockquoteStripeColor,
                    stripeWidthPx = spec.blockquoteStripeWidthPx,
                    gapWidthPx = spec.blockquoteGapWidthPx
                ),
                paragraphStart,
                paragraphEnd,
                Spanned.SPAN_PARAGRAPH
            )

            quoteEntries.forEach { (entryStart, entrySpec) ->
                applyAdditionalLeadingMargin(
                    builder = builder,
                    text = text,
                    paragraphStart = entryStart,
                    spec = entrySpec
                )
            }
        } else {
            index += 1
            val paragraphEnd = defaultParagraphEnd(text, builder.length, paragraphStart)
            val span = spec.restIndentPx?.let {
                LeadingMarginSpan.Standard(spec.indentPx, it)
            } ?: LeadingMarginSpan.Standard(spec.indentPx)

            builder
                .getSpans(0, builder.length, LeadingMarginSpan::class.java)
                .filter { it !is EditorBlockBoxSpan && builder.getSpanStart(it) == paragraphStart }
                .forEach(builder::removeSpan)

            builder.setSpan(span, paragraphStart, paragraphEnd, Spanned.SPAN_PARAGRAPH)
        }
    }
}

internal fun RenderBridge.applyPendingCodeBlockSpans(
    builder: SpannableStringBuilder,
    pendingCodeBlockSpans: List<PendingCodeBlockSpan>,
    theme: EditorTheme?,
    density: Float
) {
    if (pendingCodeBlockSpans.isEmpty() || theme?.styleSheet != null) return

    val backgroundColor = theme?.codeBlock?.backgroundColor ?: LayoutConstants.CODE_BACKGROUND_COLOR
    val cornerRadiusPx = (theme?.codeBlock?.borderRadius ?: 8f) * density
    val paddingHorizontalPx = ((theme?.codeBlock?.paddingHorizontal ?: 12f) * density).toInt()
    val paddingVerticalPx = ((theme?.codeBlock?.paddingVertical ?: 8f) * density).toInt()

    for (pending in pendingCodeBlockSpans) {
        if (pending.start >= pending.end || pending.start >= builder.length) {
            continue
        }
        val spanEnd = pending.end.coerceAtMost(builder.length)
        val span = CodeBlockSpan(
            backgroundColor = backgroundColor,
            cornerRadiusPx = cornerRadiusPx,
            paddingHorizontalPx = paddingHorizontalPx,
            paddingVerticalPx = paddingVerticalPx
        )
        builder.setSpan(
            span,
            pending.start,
            spanEnd,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }
}

internal fun RenderBridge.applyAdditionalLeadingMargin(
    builder: SpannableStringBuilder,
    text: String,
    paragraphStart: Int,
    spec: PendingLeadingMargin
) {
    val extraFirstIndent = (spec.indentPx - spec.blockquoteIndentPx).coerceAtLeast(0)
    val extraRestIndent = spec.restIndentPx?.let {
        (it - spec.blockquoteIndentPx).coerceAtLeast(0)
    }
    if (extraRestIndent != null) {
        builder.setSpan(
            LeadingMarginSpan.Standard(extraFirstIndent, extraRestIndent),
            paragraphStart,
            defaultParagraphEnd(text, builder.length, paragraphStart),
            Spanned.SPAN_PARAGRAPH
        )
    } else if (extraFirstIndent > 0) {
        builder.setSpan(
            LeadingMarginSpan.Standard(extraFirstIndent),
            paragraphStart,
            defaultParagraphEnd(text, builder.length, paragraphStart),
            Spanned.SPAN_PARAGRAPH
        )
    }
}

internal fun RenderBridge.calculateIndent(
    context: BlockContext,
    blockStack: List<BlockContext>,
    theme: EditorTheme?,
    density: Float
): Float {
    theme?.styleSheet?.let { sheet ->
        val lists = blockStack.filter { it.listContext != null }
        return lists.mapIndexed { index, block ->
            val context = block.listContext!!
            val name = if (context.optString("kind") == "task") "taskList" else if (context.optBoolean("ordered")) "orderedList" else "bulletList"
            val style = sheet[name]
            (style?.indent ?: LayoutConstants.INDENT_PER_DEPTH) * (if (index == 0) style?.baseIndentMultiplier ?: 1f else 1f) * density
        }.sum()
    }
    val indentPerDepth = (theme?.list?.indent ?: LayoutConstants.INDENT_PER_DEPTH) * density
    val quoteDepth = blockquoteDepth(blockStack)
    val columnsDepth = columnContainerDepth(blockStack)
    val quoteIndent = maxOf(
        theme?.blockquote?.indent ?: LayoutConstants.BLOCKQUOTE_INDENT,
        (theme?.blockquote?.markerGap ?: LayoutConstants.BLOCKQUOTE_MARKER_GAP) +
            (theme?.blockquote?.borderWidth ?: LayoutConstants.BLOCKQUOTE_BORDER_WIDTH)
    ) * density
    val listBaseIndentAdjustment = calculateListBaseIndentAdjustment(context, theme, density)
    return (context.depth * indentPerDepth) -
        (quoteDepth * indentPerDepth) +
        -(columnsDepth * indentPerDepth) +
        listBaseIndentAdjustment +
        (quoteDepth * quoteIndent)
}

internal fun RenderBridge.calculateListBaseIndentAdjustment(
    context: BlockContext,
    theme: EditorTheme?,
    density: Float
): Float {
    if (context.listContext == null) {
        return 0f
    }

    val indentPerDepth = (theme?.list?.indent ?: LayoutConstants.INDENT_PER_DEPTH) * density
    val listBaseIndentMultiplier = maxOf(theme?.list?.baseIndentMultiplier ?: 1f, 0f)
    return (listBaseIndentMultiplier - 1f) * indentPerDepth
}

internal fun RenderBridge.effectiveBlockContext(blockStack: List<BlockContext>): BlockContext? {
    val currentBlock = blockStack.lastOrNull() ?: return null
    if (currentBlock.listContext != null) {
        return currentBlock
    }
    val inheritedListBlock = blockStack
        .dropLast(1)
        .asReversed()
        .firstOrNull { it.listContext != null }
        ?: return currentBlock
    return currentBlock.copy(
        depth = currentBlock.depth,
        listContext = inheritedListBlock.listContext,
        markerPending = false
    )
}

internal fun RenderBridge.effectiveParagraphStart(blockStack: List<BlockContext>): Int {
    val currentBlock = blockStack.lastOrNull() ?: return 0
    if (currentBlock.listContext != null) {
        return currentBlock.renderStart
    }
    return blockStack
        .dropLast(1)
        .asReversed()
        .firstOrNull { it.listContext != null }
        ?.renderStart
        ?: currentBlock.renderStart
}

internal fun RenderBridge.renderedParagraphStart(
    builder: CharSequence,
    candidateStart: Int
): Int {
    val boundedStart = candidateStart.coerceIn(0, builder.length)
    if (boundedStart == 0) return 0

    for (index in boundedStart - 1 downTo 0) {
        if (builder[index] == '\n') {
            return index + 1
        }
    }
    return 0
}

internal fun RenderBridge.consumePendingListMarker(
    blockStack: MutableList<BlockContext>,
    markerRenderStart: Int
): JSONObject? {
    if (blockStack.size < 2) return null
    for (idx in blockStack.lastIndex - 1 downTo 0) {
        val context = blockStack[idx]
        if (!context.markerPending) continue
        context.markerPending = false
        context.renderStart = markerRenderStart
        return context.listContext
    }
    return null
}

internal fun RenderBridge.calculateMarkerWidth(density: Float): Float {
    return LayoutConstants.LIST_MARKER_WIDTH * density
}

internal fun RenderBridge.blockquoteDepth(blockStack: List<BlockContext>): Float {
    return blockStack.count { it.nodeType == "blockquote" }.toFloat()
}

internal fun RenderBridge.columnContainerDepth(blockStack: List<BlockContext>): Float {
    return blockStack.count { it.nodeType == "columns" || it.nodeType == "column" }.toFloat()
}

internal fun RenderBridge.isTransparentContainer(nodeType: String): Boolean {
    return nodeType in setOf("blockquote", "columns", "column", "bulletList", "orderedList", "taskList")
}
