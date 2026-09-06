package com.apollohg.editor.viewer

import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.Path
import android.graphics.PathMeasure
import android.graphics.Typeface
import android.text.Layout
import android.text.Spanned
import android.text.SpannableString
import android.text.StaticLayout
import android.text.TextPaint
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.LineHeightSpan
import android.text.style.MetricAffectingSpan
import android.text.style.StrikethroughSpan
import android.text.style.UnderlineSpan
import android.text.style.ReplacementSpan
import com.apollohg.editor.EditorLinkTheme
import com.apollohg.editor.EditorMentionTheme
import com.apollohg.editor.EditorOrderedListMarkerTheme
import com.apollohg.editor.EditorTextStyle
import com.apollohg.editor.EditorTheme
import com.apollohg.editor.OrderedListMarkerFormatter
import com.apollohg.editor.ProseViewerError
import java.text.Bidi
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min

/** Immutable, density-resolved inputs. No TextPaint is shared with drawing. */
internal data class PreparedTextPaint(
    val typeface: Typeface,
    val sizePx: Float,
    val color: Int,
    val lineHeightPx: Int?,
    val spacingAfterPx: Int,
) {
    fun newTextPaint(): TextPaint = TextPaint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
        typeface = this@PreparedTextPaint.typeface
        textSize = sizePx
        color = this@PreparedTextPaint.color
    }
}

internal fun PreparedTextPaint.withStyle(
    style: EditorTextStyle,
    density: Float,
    semanticGeneration: String,
): PreparedTextPaint {
    val currentBold = typeface.style == Typeface.BOLD || typeface.style == Typeface.BOLD_ITALIC
    val currentItalic = typeface.style == Typeface.ITALIC || typeface.style == Typeface.BOLD_ITALIC
    val bold = style.fontWeight?.let { it == "bold" || it.toIntOrNull()?.let { value -> value >= 600 } == true } ?: currentBold
    val italic = style.fontStyle?.let { it == "italic" } ?: currentItalic
    val resolvedStyle = when {
        bold && italic -> Typeface.BOLD_ITALIC
        bold -> Typeface.BOLD
        italic -> Typeface.ITALIC
        else -> Typeface.NORMAL
    }
    val resolvedTypeface = ViewerFontEnvironment.resolveFont(style.fontFamily, resolvedStyle, typeface, semanticGeneration)
    return copy(
        typeface = resolvedTypeface,
        sizePx = style.fontSize?.times(density)?.takeIf { it.isFinite() && it > 0f } ?: sizePx,
        color = style.color ?: color,
    )
}

internal data class PreparedProseTheme(
    val density: Float,
    val fontDensity: Float,
    val text: PreparedTextPaint,
    val paragraph: PreparedTextPaint,
    val headings: Map<String, PreparedTextPaint>,
    val blockquote: PreparedTextPaint,
    val code: PreparedTextPaint,
    val insetTopPx: Int,
    val insetRightPx: Int,
    val insetBottomPx: Int,
    val insetLeftPx: Int,
    val listIndentPx: Int,
    val listBaseIndentMultiplier: Float,
    val listItemSpacingPx: Int,
    val listSpacingAfterPx: Int,
    val listMarkerColor: Int,
    val listMarkerScale: Float,
    val listMarkerGapPx: Int,
    val orderedListMarker: EditorOrderedListMarkerTheme?,
    val quoteIndentPx: Int,
    val quoteBorderColor: Int,
    val quoteBorderWidthPx: Int,
    val quoteMarkerGapPx: Int,
    val codeBackground: Int,
    val codeRadiusPx: Float,
    val codePaddingHorizontalPx: Int,
    val codePaddingVerticalPx: Int,
    val ruleColor: Int,
    val ruleThicknessPx: Int,
    val ruleMarginPx: Int,
    val link: EditorLinkTheme?,
    val mention: EditorMentionTheme?,
    val atomPaddingHorizontalPx: Int,
    val atomPaddingVerticalPx: Int,
    val viewerAtoms: ViewerAtomConfiguration? = null,
) {
    companion object {
        fun resolve(
            themeJson: String?,
            density: Float,
            fontScale: Float = 1f,
            semanticGeneration: String = "standalone-theme",
        ): PreparedProseTheme {
            val theme = EditorTheme.fromJson(themeJson) ?: EditorTheme()
            val resolvedFontScale = fontScale.takeIf { it.isFinite() && it > 0f } ?: 1f
            val scaledDensity = density * resolvedFontScale
            fun px(value: Float, fallback: Float): Int = max(0, (value.takeIf { it.isFinite() } ?: fallback).times(density).toInt())
            fun fontPx(value: Float, fallback: Float): Float = ((value.takeIf { it.isFinite() } ?: fallback) * scaledDensity)
            fun typeface(style: EditorTextStyle?, fallback: Typeface): Typeface {
                return ViewerFontEnvironment.resolveFont(
                    style?.fontFamily,
                    style?.typefaceStyle() ?: fallback.style,
                    fallback,
                    semanticGeneration,
                )
            }
            fun paint(style: EditorTextStyle?, fallback: PreparedTextPaint? = null): PreparedTextPaint {
                val base = fallback ?: PreparedTextPaint(Typeface.DEFAULT, 17f * scaledDensity, 0xFF212121.toInt(), null, 0)
                return PreparedTextPaint(
                    typeface = typeface(style, base.typeface),
                    sizePx = style?.fontSize?.let { fontPx(it, 17f) } ?: base.sizePx,
                    color = style?.color ?: base.color,
                    lineHeightPx = style?.lineHeight?.let { fontPx(it, 0f).toInt().takeIf { value -> value > 0 } } ?: base.lineHeightPx,
                    spacingAfterPx = style?.spacingAfter?.let { fontPx(it, 0f).toInt() } ?: base.spacingAfterPx,
                )
            }
            val text = paint(theme.text)
            theme.links?.let { link ->
                link.fontFamily?.let { linkFamily ->
                    ViewerFontEnvironment.resolveFont(linkFamily, link.asTextStyle().typefaceStyle(), text.typeface, semanticGeneration)
                }
            }
            val paragraph = paint(theme.effectiveTextStyle("paragraph"), text)
            val quote = paint(theme.effectiveTextStyle("paragraph", true), paragraph)
            val codeFallback = PreparedTextPaint(Typeface.MONOSPACE, text.sizePx, text.color, text.lineHeightPx, text.spacingAfterPx)
            val headings = listOf("h1" to 32f, "h2" to 28f, "h3" to 24f, "h4" to 21f, "h5" to 19f, "h6" to 17f).associate { (name, size) ->
                name to paint(EditorTextStyle(fontSize = size, fontWeight = "700", spacingAfter = 10f).mergedWith(theme.headings[name]), paragraph)
            }
            val listItemSpacingPx = px(theme.list?.itemSpacing ?: 4f, 4f)
            return PreparedProseTheme(
                density, scaledDensity, text, paragraph, headings, quote, paint(theme.effectiveTextStyle("codeBlock"), codeFallback),
                px(theme.contentInsets?.top ?: 0f, 0f), px(theme.contentInsets?.right ?: 0f, 0f), px(theme.contentInsets?.bottom ?: 0f, 0f), px(theme.contentInsets?.left ?: 0f, 0f),
                px(theme.list?.indent ?: 28f, 28f), theme.list?.baseIndentMultiplier ?: 1f, listItemSpacingPx, px(theme.list?.spacingAfter ?: theme.list?.itemSpacing ?: 4f, 4f), theme.list?.markerColor ?: text.color, theme.list?.markerScale ?: 1f,
                px(theme.list?.markerGap ?: PREPARED_LIST_MARKER_GAP_DP, PREPARED_LIST_MARKER_GAP_DP),
                theme.list?.orderedMarker,
                px(theme.blockquote?.indent ?: 16f, 16f), theme.blockquote?.borderColor ?: 0xFFC7C7CC.toInt(), px(theme.blockquote?.borderWidth ?: 3f, 3f), px(theme.blockquote?.markerGap ?: 10f, 10f),
                theme.codeBlock?.backgroundColor ?: 0xFFF2F2F7.toInt(), (theme.codeBlock?.borderRadius ?: 8f) * density, px(theme.codeBlock?.paddingHorizontal ?: 12f, 12f), px(theme.codeBlock?.paddingVertical ?: 8f, 8f),
                theme.horizontalRule?.color ?: 0xFFC7C7CC.toInt(), max(1, px(theme.horizontalRule?.thickness ?: 1f, 1f)), px(theme.horizontalRule?.verticalMargin ?: 12f, 12f),
                theme.links, theme.mentions, px(6f, 6f), px(4f, 4f),
                ViewerAtomConfiguration.parse(themeJson),
            )
        }
    }

    fun paintFor(block: ViewerBlock): PreparedTextPaint = when {
        block.nodeType == "codeBlock" -> code
        headings.containsKey(block.nodeType) -> headings.getValue(block.nodeType)
        block.inBlockquote -> blockquote
        else -> paragraph
    }

    val retainedBytes: Long get() = 3_072L + headings.size * 384L + (viewerAtoms?.retainedBytes ?: 0)
}
