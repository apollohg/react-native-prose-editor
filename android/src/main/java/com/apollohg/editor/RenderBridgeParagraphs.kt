package com.apollohg.editor

import android.text.Annotation
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.style.AbsoluteSizeSpan
import android.text.style.ForegroundColorSpan
import org.json.JSONArray
import org.json.JSONObject

internal fun RenderBridge.resolveTextStyle(
    nodeType: String,
    theme: EditorTheme?,
    inBlockquote: Boolean = false
): EditorTextStyle {
    return theme?.effectiveTextStyle(nodeType, inBlockquote) ?: EditorTextStyle()
}

internal fun RenderBridge.resolveInlineTextStyle(
    blockStack: List<BlockContext>,
    theme: EditorTheme?
): EditorTextStyle {
    val nodeType = effectiveBlockContext(blockStack)?.nodeType ?: "paragraph"
    return resolveTextStyle(nodeType, theme, blockquoteDepth(blockStack) > 0)
}

internal fun RenderBridge.resolveInlineTextColor(
    blockStack: List<BlockContext>,
    fallbackColor: Int,
    theme: EditorTheme?
): Int = resolveInlineTextStyle(blockStack, theme).color ?: fallbackColor

internal fun RenderBridge.isListItemNodeType(nodeType: String): Boolean {
    return EditorNodeTypes.isListItem(nodeType)
}

    /**
     * Parse a [JSONArray] of marks into a list of mark identifiers.
     *
     * Each mark can be either a plain string (e.g. "bold") or a JSON object
     * (e.g. `{"type": "link", "href": "https://..."}`). Returns a mixed list
     * of [String] and [JSONObject].
     */
internal fun RenderBridge.parseMarks(marksArray: JSONArray?): List<Any> {
    if (marksArray == null || marksArray.length() == 0) return emptyList()
    val marks = mutableListOf<Any>()
    for (i in 0 until marksArray.length()) {
        when (val mark = marksArray.opt(i)) {
            is String -> marks.add(mark)
            is JSONObject -> marks.add(mark)
        }
    }
    return marks
}

    /**
     * Append a newline used between blocks (inter-block separator).
     *
     * When [spacingPx] > 0, applies a [ParagraphSpacerSpan] to the newline
     * character to create vertical spacing after the preceding block.
     */
internal fun RenderBridge.appendInterBlockNewline(
    builder: SpannableStringBuilder,
    baseFontSize: Float,
    textColor: Int,
    spacingPx: Int = 0,
    inBlockquote: Boolean = false,
    topLevelChildIndex: Int? = null
) {
    val start = builder.length
    builder.append("\n")
    val end = builder.length
    builder.setSpan(
        Annotation(NATIVE_INTER_BLOCK_SEPARATOR_ANNOTATION, "1"),
        start,
        end,
        Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    if (spacingPx > 0) {
        builder.setSpan(
            ParagraphSpacerSpan(spacingPx, baseFontSize.toInt(), textColor),
            start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    } else {
        builder.setSpan(
            ForegroundColorSpan(textColor),
            start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        builder.setSpan(
            AbsoluteSizeSpan(baseFontSize.toInt(), false),
            start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }
    annotateTopLevelChild(builder, start, end, topLevelChildIndex)
    if (inBlockquote) {
        builder.setSpan(
            Annotation(NATIVE_BLOCKQUOTE_ANNOTATION, "1"),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }
}

internal fun RenderBridge.appendTrailingHardBreakPlaceholderIfNeeded(
    builder: SpannableStringBuilder,
    endedBlock: BlockContext,
    remainingBlockStack: List<BlockContext>,
    baseFontSize: Float,
    textColor: Int,
    theme: EditorTheme?,
    density: Float,
    pendingLeadingMargins: MutableMap<Int, PendingLeadingMargin>
) {
    if (builder.isEmpty()) return
    if (isListItemNodeType(endedBlock.nodeType)) return
    if (!lastCharacterIsHardBreak(builder)) return

    val start = builder.length
    builder.append(LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER)
    val end = builder.length
    builder.setSpan(
        Annotation(NATIVE_SYNTHETIC_PLACEHOLDER_ANNOTATION, "1"),
        start,
        end,
        Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    builder.setSpan(
        ForegroundColorSpan(resolveInlineTextColor(remainingBlockStack + endedBlock, textColor, theme)),
        start,
        end,
        Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    applyBlockStyle(
        builder,
        start,
        end,
        remainingBlockStack + endedBlock,
        pendingLeadingMargins,
        theme,
        density
    )
}

internal fun RenderBridge.lastCharacterIsHardBreak(builder: SpannableStringBuilder): Boolean {
    if (builder.isEmpty()) return false
    val lastIndex = builder.length - 1
    return builder.getSpans(lastIndex, builder.length, Annotation::class.java).any {
        it.key == "nativeVoidNodeType" && EditorNodeTypes.isHardBreak(it.value)
    }
}

internal fun RenderBridge.annotateTopLevelChild(
    builder: SpannableStringBuilder,
    start: Int,
    end: Int,
    topLevelChildIndex: Int?
) {
    if (topLevelChildIndex == null || start >= end) return
    builder.setSpan(
        Annotation(NATIVE_TOP_LEVEL_CHILD_INDEX_ANNOTATION, topLevelChildIndex.toString()),
        start,
        end,
        Spanned.SPAN_INCLUSIVE_EXCLUSIVE
    )
}

internal fun RenderBridge.trailingRenderedContentHasBlockquote(builder: Spanned): Boolean {
    for (index in builder.length - 1 downTo 0) {
        val ch = builder[index]
        if (ch == '\n' || ch == '\r') continue
        return hasBlockquoteAnnotationAt(builder, index)
    }
    return false
}

internal fun RenderBridge.defaultParagraphEnd(text: String, length: Int, paragraphStart: Int): Int {
    val newlineIndex = text.indexOf('\n', paragraphStart)
    return if (newlineIndex >= 0) newlineIndex + 1 else length
}

internal fun RenderBridge.blockquoteSpanEnd(
    builder: Spanned,
    text: String,
    paragraphStart: Int
): Int {
    var cursor = paragraphStart
    while (cursor < builder.length) {
        val newlineIndex = text.indexOf('\n', cursor)
        if (newlineIndex < 0) {
            return builder.length
        }
        val newlineQuoted = hasBlockquoteAnnotationAt(builder, newlineIndex)
        val nextIndex = newlineIndex + 1
        val nextQuoted = nextIndex < builder.length && hasBlockquoteAnnotationAt(builder, nextIndex)

        if (!newlineQuoted && !nextQuoted) {
            return nextIndex
        }
        cursor = nextIndex
    }
    return builder.length
}

internal fun RenderBridge.hasBlockquoteAnnotationAt(text: Spanned, index: Int): Boolean {
    if (index < 0 || index >= text.length) return false
    return text.getSpans(index, index + 1, Annotation::class.java).any {
        it.key == NATIVE_BLOCKQUOTE_ANNOTATION
    }
}
