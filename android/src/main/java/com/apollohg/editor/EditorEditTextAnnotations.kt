package com.apollohg.editor

import com.apollohg.editor.EditorEditText.AccessibleAnnotationTarget
import com.apollohg.editor.EditorEditText.InteractiveAnnotationHit
import com.apollohg.editor.EditorEditText.AccessibleAnnotation
import com.apollohg.editor.EditorEditText.LinkHit
import com.apollohg.editor.EditorEditText.MentionHit
import android.graphics.Rect
import android.text.Annotation
import android.text.Layout
import android.text.Spanned

internal fun EditorEditText.textOffsetHitAt(x: Float, y: Float): Pair<Spanned, Int>? {
    val spannable = text as? Spanned ?: return null
    val layout = layout ?: return null
    if (spannable.isEmpty()) return null

    val localX = x - totalPaddingLeft + scrollX
    val localY = y - totalPaddingTop + scrollY
    if (localY < 0f || localY > layout.height.toFloat()) {
        return null
    }

    val line = layout.getLineForVertical(localY.toInt())
    val lineLeft = layout.getLineLeft(line)
    val lineRight = layout.getLineRight(line)
    if (localX < lineLeft || localX > lineRight) {
        return null
    }

    val offset = layout.getOffsetForHorizontal(line, localX)
        .coerceIn(0, maxOf(spannable.length - 1, 0))
    return spannable to offset
}

internal fun EditorEditText.mentionHitAtImpl(x: Float, y: Float): MentionHit? {
    val (spannable, offset) = textOffsetHitAt(x, y) ?: return null
    val annotations = spannable.getSpans(
        offset,
        (offset + 1).coerceAtMost(spannable.length),
        Annotation::class.java
    )
    val mentionAnnotation = annotations.firstOrNull {
        it.key == "nativeVoidNodeType" && it.value == "mention"
    } ?: return null
    val docPos = annotations.firstOrNull { it.key == "nativeDocPos" }
        ?.value
        ?.viewerMentionDocPos() ?: return null
    val start = spannable.getSpanStart(mentionAnnotation)
    val end = spannable.getSpanEnd(mentionAnnotation)
    if (start < 0 || end <= start) {
        return null
    }

    return MentionHit(
        docPos = docPos,
        label = spannable.subSequence(start, end).toString()
    )
}

internal fun EditorEditText.linkHitAtImpl(x: Float, y: Float): LinkHit? {
    val (spannable, offset) = textOffsetHitAt(x, y) ?: return null
    val annotations = spannable.getSpans(
        offset,
        (offset + 1).coerceAtMost(spannable.length),
        Annotation::class.java
    )
    val linkAnnotation = annotations.firstOrNull {
        it.key == RenderBridge.NATIVE_LINK_HREF_ANNOTATION && it.value.isNotBlank()
    } ?: return null
    val start = spannable.getSpanStart(linkAnnotation)
    val end = spannable.getSpanEnd(linkAnnotation)
    if (start < 0 || end <= start) {
        return null
    }

    return LinkHit(
        href = linkAnnotation.value,
        text = spannable.subSequence(start, end).toString()
    )
}

internal fun EditorEditText.accessibleAnnotationsImpl(): List<AccessibleAnnotation> {
    val spannable = text as? Spanned ?: return emptyList()
    val textLayout = layout ?: return emptyList()
    val annotations = spannable.getSpans(0, spannable.length, Annotation::class.java)
    val results = mutableListOf<Pair<Int, AccessibleAnnotation>>()
    annotations.forEach { annotation ->
        val start = spannable.getSpanStart(annotation)
        val end = spannable.getSpanEnd(annotation)
        if (start < 0 || end <= start) return@forEach
        val label = spannable.subSequence(start, end).toString()
        val target = when {
            annotation.key == RenderBridge.NATIVE_LINK_HREF_ANNOTATION &&
                annotation.value.isNotBlank() -> AccessibleAnnotationTarget.Link(
                href = annotation.value,
                text = label
            )
            annotation.key == "nativeVoidNodeType" && annotation.value == "mention" -> {
                val docPos = annotations.firstOrNull { candidate ->
                    candidate.key == "nativeDocPos" &&
                        spannable.getSpanStart(candidate) == start &&
                        spannable.getSpanEnd(candidate) == end
                }?.value?.viewerMentionDocPos() ?: return@forEach
                AccessibleAnnotationTarget.Mention(docPos, label)
            }
            else -> return@forEach
        }
        val role = when (target) {
            is AccessibleAnnotationTarget.Link -> "link"
            is AccessibleAnnotationTarget.Mention -> "mention"
        }
        results += start to AccessibleAnnotation(
            target = target,
            label = label,
            role = role,
            bounds = annotationBounds(textLayout, start, end),
            annotation = annotation,
            start = start,
            end = end
        )
    }
    return results.sortedBy { it.first }.map { it.second }
}

internal fun EditorEditText.interactiveAnnotationHitAtImpl(x: Float, y: Float): InteractiveAnnotationHit? {
    val (spannable, offset) = textOffsetHitAt(x, y) ?: return null
    val annotations = spannable.getSpans(
        offset,
        (offset + 1).coerceAtMost(spannable.length),
        Annotation::class.java
    )
    val mentionAnnotation = annotations.firstOrNull {
        it.key == "nativeVoidNodeType" && it.value == "mention"
    }
    val annotation = mentionAnnotation ?: annotations.firstOrNull {
        it.key == RenderBridge.NATIVE_LINK_HREF_ANNOTATION && it.value.isNotBlank()
    } ?: return null
    val start = spannable.getSpanStart(annotation)
    val end = spannable.getSpanEnd(annotation)
    if (start < 0 || end <= start) return null
    val label = spannable.subSequence(start, end).toString()
    val target = if (annotation === mentionAnnotation) {
        val docPos = annotations.firstOrNull {
            it.key == "nativeDocPos" &&
                spannable.getSpanStart(it) == start &&
                spannable.getSpanEnd(it) == end
        }?.value?.viewerMentionDocPos() ?: return null
        AccessibleAnnotationTarget.Mention(docPos, label)
    } else {
        AccessibleAnnotationTarget.Link(annotation.value, label)
    }
    return InteractiveAnnotationHit(target, annotation, start, end)
}

    /** Annotation values are decimal Rust u32 positions, never signed editor offsets. */
private fun String.viewerMentionDocPos(): Long? =
    toLongOrNull()?.takeIf { it in 0L..UInt.MAX_VALUE.toLong() }

internal fun EditorEditText.annotationBounds(textLayout: Layout, start: Int, end: Int): Rect {
    val firstLine = textLayout.getLineForOffset(start)
    val lastLine = textLayout.getLineForOffset((end - 1).coerceAtLeast(start))
    val bounds = Rect()
    for (line in firstLine..lastLine) {
        val segmentStart = maxOf(start, textLayout.getLineStart(line))
        val segmentEnd = minOf(end, textLayout.getLineEnd(line))
        val left = totalPaddingLeft + minOf(
            textLayout.getPrimaryHorizontal(segmentStart),
            textLayout.getPrimaryHorizontal(segmentEnd)
        ) - scrollX
        val right = totalPaddingLeft + maxOf(
            textLayout.getPrimaryHorizontal(segmentStart),
            textLayout.getPrimaryHorizontal(segmentEnd)
        ) - scrollX
        val lineBounds = Rect(
            kotlin.math.floor(left.toDouble()).toInt(),
            totalPaddingTop + textLayout.getLineTop(line) - scrollY,
            kotlin.math.ceil(right.toDouble()).toInt().coerceAtLeast(
                kotlin.math.floor(left.toDouble()).toInt() + 1
            ),
            totalPaddingTop + textLayout.getLineBottom(line) - scrollY
        )
        if (bounds.isEmpty) bounds.set(lineBounds) else bounds.union(lineBounds)
    }
    return bounds
}
