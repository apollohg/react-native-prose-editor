package com.apollohg.editor

import android.text.Spanned

internal fun Spanned.resolvedBlockSpacing(): Map<EditorBlockBoxSpan, EditorBoxStyle> {
    data class Box(val span: EditorBlockBoxSpan, val start: Int, val end: Int)
    val spans = getSpans(0, length, EditorBlockBoxSpan::class.java)
        .map { Box(it, getSpanStart(it), getSpanEnd(it)) }
        .sortedWith(compareBy({ it.start }, { it.span.depth }, { -it.end }))
    val styles = spans.associate { it.span to it.span.box }.toMutableMap()
    val stack = mutableListOf<Box>()
    val children = mutableMapOf<Box?, MutableList<Box>>()
    for (span in spans) {
        while (stack.isNotEmpty() && (stack.last().span.depth >= span.span.depth || stack.last().end < span.end)) {
            stack.removeAt(stack.lastIndex)
        }
        children.getOrPut(stack.lastOrNull()) { mutableListOf() }.add(span)
        stack.add(span)
    }
    for ((parent, siblings) in children) {
        val last = siblings.last()
        if (parent?.span?.nodeType == "blockquote" && last.span.nodeType == "paragraph" && last.end == parent.end) {
            styles[last.span] = last.span.box.copy(margin = last.span.box.margin.copy(bottom = 0f))
        }
        for ((previous, current) in siblings.zipWithNext()) {
            val start = previous.end
            val end = current.start
            if (start > end || (start until end).any { this[it] != '\n' && this[it] != '\u200B' }) continue
            val bottom = previous.span.marginForCollapsing.bottom
            val top = current.span.marginForCollapsing.top
            val collapsed = maxOf(0f, bottom, top) + minOf(0f, bottom, top)
            val style = styles.getValue(current.span)
            styles[current.span] = style.copy(margin = style.margin.copy(top = style.margin.top + collapsed - bottom - top))
        }
    }
    return styles
}
