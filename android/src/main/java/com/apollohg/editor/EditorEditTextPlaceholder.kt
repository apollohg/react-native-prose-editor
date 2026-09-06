package com.apollohg.editor

import com.apollohg.editor.EditorEditText.Companion.EMPTY_BLOCK_PLACEHOLDER
import android.graphics.Typeface
import android.text.Layout
import android.text.StaticLayout
import android.text.TextPaint

/**
     * Whether the document holds nothing the user authored.
     *
     * Taken verbatim from the core. Deriving it cannot work: an empty list item
     * contributes no characters, so scanning the rendered content reports empty
     * and leaves the placeholder over a visible bullet. The scan below is only
     * the fallback for renders with no editor update.
     */
internal fun EditorEditText.isRenderedContentEmpty(content: CharSequence? = text): Boolean {
    coreReportedDocumentIsEmpty?.let { return it }

    val renderedContent = content ?: return true
    if (renderedContent.isEmpty()) return true

    for (index in 0 until renderedContent.length) {
        when (renderedContent[index]) {
            EMPTY_BLOCK_PLACEHOLDER, '\n', '\r' -> continue
            else -> return false
        }
    }

    return true
}

    /** Adopt the core's authoritative empty state from an editor update. */
internal fun EditorEditText.setCoreReportedDocumentIsEmptyImpl(isEmpty: Boolean?) {
    if (coreReportedDocumentIsEmpty == isEmpty) return
    coreReportedDocumentIsEmpty = isEmpty
    invalidate()
}

internal fun EditorEditText.shouldDisplayPlaceholder(): Boolean {
    return placeholderText.isNotEmpty() &&
        externalTextComposition?.latestText.isNullOrEmpty() &&
        isRenderedContentEmpty()
}

internal fun EditorEditText.shouldDisplayPlaceholderForTestingImpl(): Boolean = shouldDisplayPlaceholder()

internal fun EditorEditText.buildPlaceholderLayout(availableWidth: Int): StaticLayout? {
    if (!shouldDisplayPlaceholder()) return null
    if (availableWidth <= 0) return null
    val insets = placeholderContentInsets(availableWidth)
    val contentWidth = (availableWidth - insets.left - insets.right).toInt().coerceAtLeast(1)

    val placeholderPaint = resolvedPlaceholderPaint()
    val styledPlaceholder = theme?.styleSheet?.let { sheet ->
        android.text.SpannableStringBuilder(placeholderText).apply {
            setSpan(EditorResolvedTextSpan(sheet.resolveText("placeholder"), resources.displayMetrics.density), 0, length, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            applyStyleSheetLineMetrics()
        }
    } ?: placeholderText
    return StaticLayout.Builder
        .obtain(
            styledPlaceholder,
            0,
            placeholderText.length,
            placeholderPaint,
            contentWidth
        )
        .setAlignment(Layout.Alignment.ALIGN_NORMAL)
        .setIncludePad(includeFontPadding)
        .build()
}

internal fun EditorEditText.placeholderContentInsets(availableWidth: Int): EditorEdges {
    val current = layout
    val document = if (current is EditorDocumentLayout && current.width == availableWidth) current else {
        EditorDocumentLayout(editableText, paint, availableWidth.coerceAtLeast(1), includeFontPadding, lineSpacingMultiplier, lineSpacingExtra)
    }
    return EditorEdges(
        top = document.textLineTop(0).toFloat(),
        right = availableWidth - document.contentRight(0),
        bottom = (document.height - document.textLineBottom(document.lineCount - 1)).toFloat(),
        left = document.contentLeft(0),
    )
}

internal fun EditorEditText.resolvedPlaceholderPaint(): TextPaint {
    val textStyle = theme?.styleSheet?.resolveText("placeholder") ?: theme?.effectiveTextStyle("paragraph")
    val resolvedTextSize = textStyle?.fontSize?.times(resources.displayMetrics.density) ?: baseFontSize
    val resolvedTypeface = resolvePlaceholderTypeface(textStyle)

    return TextPaint(paint).apply {
        color = theme?.placeholderColor ?: currentHintTextColor
        textSize = resolvedTextSize
        typeface = resolvedTypeface
        textStyle?.letterSpacing?.let { letterSpacing = it * resources.displayMetrics.density / textSize }
        if (theme?.styleSheet != null && textStyle != null) EditorResolvedTextSpan(textStyle, resources.displayMetrics.density).updateDrawState(this)
    }
}

internal fun EditorEditText.resolvePlaceholderTypeface(textStyle: EditorTextStyle?): Typeface {
    val baseTypeface = typeface ?: Typeface.DEFAULT
    val requestedStyle = textStyle?.typefaceStyle() ?: Typeface.NORMAL
    val family = textStyle?.fontFamily?.takeIf { it.isNotBlank() }

    return when {
        family != null -> Typeface.create(family, requestedStyle)
        requestedStyle != Typeface.NORMAL -> Typeface.create(baseTypeface, requestedStyle)
        else -> baseTypeface
    }
}

internal fun EditorEditText.resolvePlaceholderHeightForMeasuredWidth(widthPx: Int): Int? {
    val availableWidth = (widthPx - compoundPaddingLeft - compoundPaddingRight).coerceAtLeast(0)
    return resolvePlaceholderHeightForAvailableWidth(availableWidth)
}

internal fun EditorEditText.resolvePlaceholderHeightForAvailableWidth(availableWidth: Int): Int? {
    val placeholderLayout = buildPlaceholderLayout(availableWidth) ?: return null
    val placeholderHeight = placeholderLayout.height.takeIf { it > 0 } ?: lineHeight
    val insets = placeholderContentInsets(availableWidth)
    return placeholderHeight + insets.top.toInt() + insets.bottom.toInt() + compoundPaddingTop + compoundPaddingBottom
}
