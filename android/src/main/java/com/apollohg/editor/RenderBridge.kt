package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Typeface
import android.text.Annotation
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.style.AbsoluteSizeSpan
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.LeadingMarginSpan
import android.text.style.LineHeightSpan
import android.text.style.ReplacementSpan
import android.text.style.StrikethroughSpan
import android.text.style.StyleSpan
import android.text.style.TypefaceSpan
import android.text.style.URLSpan
import android.text.style.UnderlineSpan
import org.json.JSONArray
import org.json.JSONObject

// ── Layout Constants ────────────────────────────────────────────────────

/**
 * Layout constants for paragraph styles and rendering, matching the iOS
 * [LayoutConstants] enum.
 */
object LayoutConstants {
    /** Base indentation per depth level (pixels at base scale). */
    const val INDENT_PER_DEPTH: Float = 24f

    /** Width reserved for the list bullet/number (pixels at base scale). */
    const val LIST_MARKER_WIDTH: Float = 20f

    /** Gap between the list marker and the text that follows (pixels at base scale). */
    const val LIST_MARKER_TEXT_GAP: Float = 8f

    /** Height of the horizontal rule separator line (pixels). */
    const val HORIZONTAL_RULE_HEIGHT: Float = 1f

    /** Vertical padding above and below the horizontal rule (pixels). */
    const val HORIZONTAL_RULE_VERTICAL_PADDING: Float = 8f

    /** Bullet character for unordered list items. */
    const val UNORDERED_LIST_BULLET: String = "\u2022 "

    /** Scale factor applied only to unordered list marker glyphs. */
    const val UNORDERED_LIST_MARKER_FONT_SCALE: Float = 2.0f

    /** Object replacement character used for void block elements. */
    const val OBJECT_REPLACEMENT_CHARACTER: String = "\uFFFC"

    /** Background color for inline code spans (light gray). */
    const val CODE_BACKGROUND_COLOR: Int = 0x1A000000  // 10% black
}

// ── BlockContext ─────────────────────────────────────────────────────────

/**
 * Transient context while rendering block elements. Pushed onto a stack
 * when a `blockStart` element is encountered and popped on `blockEnd`.
 */
data class BlockContext(
    val nodeType: String,
    val depth: Int,
    val listContext: JSONObject?,
    var markerPending: Boolean = false,
    var renderStart: Int = 0
)

private data class PendingLeadingMargin(
    val indentPx: Int,
    val restIndentPx: Int?
)

// ── HorizontalRuleSpan ──────────────────────────────────────────────────

/**
 * A [LeadingMarginSpan] replacement span that draws a horizontal separator line.
 *
 * Used for `horizontalRule` void block elements. Renders as a thin line
 * across the available width with vertical padding.
 */
class HorizontalRuleSpan(
    private val lineColor: Int,
    private val lineHeight: Float = LayoutConstants.HORIZONTAL_RULE_HEIGHT,
    private val verticalPadding: Float = LayoutConstants.HORIZONTAL_RULE_VERTICAL_PADDING
) : LeadingMarginSpan {

    override fun getLeadingMargin(first: Boolean): Int = 0

    override fun drawLeadingMargin(
        canvas: Canvas,
        paint: Paint,
        x: Int,
        dir: Int,
        top: Int,
        baseline: Int,
        bottom: Int,
        text: CharSequence,
        start: Int,
        end: Int,
        first: Boolean,
        layout: android.text.Layout?
    ) {
        val savedColor = paint.color
        val savedStyle = paint.style

        paint.color = lineColor
        paint.style = Paint.Style.FILL

        val lineY = (top + bottom) / 2f
        val lineWidth = layout?.width?.toFloat() ?: canvas.width.toFloat()
        canvas.drawRect(
            x.toFloat(),
            lineY - lineHeight / 2f,
            lineWidth,
            lineY + lineHeight / 2f,
            paint
        )

        paint.color = savedColor
        paint.style = savedStyle
    }
}

class FixedLineHeightSpan(
    private val lineHeightPx: Int
) : LineHeightSpan {
    override fun chooseHeight(
        text: CharSequence,
        start: Int,
        end: Int,
        spanstartv: Int,
        v: Int,
        fm: android.graphics.Paint.FontMetricsInt
    ) {
        val currentHeight = fm.descent - fm.ascent
        if (lineHeightPx <= 0 || currentHeight <= 0) return
        if (lineHeightPx == currentHeight) return

        val extra = lineHeightPx - currentHeight
        fm.descent += extra
        fm.bottom = fm.descent
    }
}

/**
 * Adds vertical spacing after a paragraph by increasing the descent of the
 * inter-block newline character.
 *
 * Uses [ReplacementSpan] (not [LineHeightSpan]/[android.text.style.ParagraphStyle])
 * because Android's StaticLayout normalizes ParagraphStyle metrics across all
 * lines in a paragraph, making per-line spacing impossible.
 *
 * ReplacementSpan only affects the single character it covers, so the extra
 * descent applies only to the newline's line — creating a gap below the
 * preceding paragraph without inflating other lines.
 */
class ParagraphSpacerSpan(
    private val spacingPx: Int,
    private val baseFontSize: Int,
    private val textColor: Int
) : ReplacementSpan() {
    override fun getSize(
        paint: Paint,
        text: CharSequence,
        start: Int,
        end: Int,
        fm: Paint.FontMetricsInt?
    ): Int {
        if (fm != null && spacingPx > 0) {
            // Keep the natural ascent/top (from baseFontSize) so the newline
            // line doesn't shrink above the baseline. Add spacing as descent.
            val savedSize = paint.textSize
            paint.textSize = baseFontSize.toFloat()
            paint.getFontMetricsInt(fm)
            paint.textSize = savedSize
            fm.descent += spacingPx
            fm.bottom = fm.descent
        }
        return 0
    }

    override fun draw(
        canvas: Canvas,
        text: CharSequence,
        start: Int,
        end: Int,
        x: Float,
        top: Int,
        y: Int,
        bottom: Int,
        paint: Paint
    ) {
        // Draw nothing — pure spacing.
    }
}

class CenteredBulletSpan(
    private val textColor: Int,
    private val markerWidthPx: Float,
    private val bulletRadiusPx: Float,
    private val bodyFontSizePx: Float,
    private val markerGapToTextPx: Float
) : ReplacementSpan() {
    override fun getSize(
        paint: Paint,
        text: CharSequence,
        start: Int,
        end: Int,
        fm: Paint.FontMetricsInt?
    ): Int {
        return kotlin.math.ceil(markerWidthPx).toInt()
    }

    override fun draw(
        canvas: Canvas,
        text: CharSequence,
        start: Int,
        end: Int,
        x: Float,
        top: Int,
        y: Int,
        bottom: Int,
        paint: Paint
    ) {
        val previousColor = paint.color
        val previousStyle = paint.style
        val previousSize = paint.textSize

        paint.color = textColor
        paint.style = Paint.Style.FILL

        // Use body text metrics (not the marker's inflated font) for centering.
        paint.textSize = bodyFontSizePx
        val fm = paint.fontMetrics
        val centerX = resolvedCenterX(x)
        val centerY = y + (fm.ascent + fm.descent) / 2f
        canvas.drawCircle(centerX, centerY, bulletRadiusPx, paint)

        paint.color = previousColor
        paint.style = previousStyle
        paint.textSize = previousSize
    }

    fun textSideGapPx(x: Float): Float {
        return (x + markerWidthPx) - (resolvedCenterX(x) + bulletRadiusPx)
    }

    private fun resolvedCenterX(x: Float): Float {
        return x + markerWidthPx - markerGapToTextPx - bulletRadiusPx
    }
}

// ── RenderBridge ────────────────────────────────────────────────────────

/**
 * Converts RenderElement JSON (emitted by Rust editor-core via UniFFI) into
 * [SpannableStringBuilder] for display in an Android EditText.
 *
 * The JSON format matches the output of `serialize_render_elements` in lib.rs:
 * ```json
 * [
 *   {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
 *   {"type": "textRun", "text": "Hello ", "marks": []},
 *   {"type": "textRun", "text": "world", "marks": ["bold"]},
 *   {"type": "blockEnd"},
 *   {"type": "voidInline", "nodeType": "hardBreak", "docPos": 12},
 *   {"type": "voidBlock", "nodeType": "horizontalRule", "docPos": 15}
 * ]
 * ```
 */
object RenderBridge {

    // ── Public API ──────────────────────────────────────────────────────

    /**
     * Convert a JSON array of RenderElements into a [SpannableStringBuilder].
     *
     * @param json A JSON string representing an array of render elements.
     * @param baseFontSize The default font size in pixels for unstyled text.
     * @param textColor The default text color as an ARGB int.
     * @return The rendered spannable string. Returns an empty builder if the JSON is invalid.
     */
    fun buildSpannable(
        json: String,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme? = null,
        density: Float = 1f
    ): SpannableStringBuilder {
        val elements = try {
            JSONArray(json)
        } catch (_: Exception) {
            return SpannableStringBuilder()
        }

        return buildSpannableFromArray(elements, baseFontSize, textColor, theme, density)
    }

    /**
     * Convert a parsed [JSONArray] of RenderElements into a [SpannableStringBuilder].
     *
     * This is the main rendering entry point. It processes elements in order,
     * maintaining a block context stack for proper paragraph styling.
     *
     * @param elements Parsed JSON array where each element is a [JSONObject].
     * @param baseFontSize The default font size in pixels for unstyled text.
     * @param textColor The default text color as an ARGB int.
     * @return The rendered spannable string.
     */
    fun buildSpannableFromArray(
        elements: JSONArray,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme? = null,
        density: Float = 1f
    ): SpannableStringBuilder {
        val result = SpannableStringBuilder()
        val blockStack = mutableListOf<BlockContext>()
        val pendingLeadingMargins = linkedMapOf<Int, PendingLeadingMargin>()
        var isFirstBlock = true
        var nextBlockSpacingBefore: Float? = null

        for (i in 0 until elements.length()) {
            val element = elements.optJSONObject(i) ?: continue
            val type = element.optString("type", "")

            when (type) {
                "textRun" -> {
                    val text = element.optString("text", "")
                    val marksArray = element.optJSONArray("marks")
                    val marks = parseMarks(marksArray)
                    appendStyledText(
                        result,
                        text,
                        marks,
                        baseFontSize,
                        textColor,
                        blockStack,
                        pendingLeadingMargins,
                        theme,
                        density
                    )
                }

                "voidInline" -> {
                    val nodeType = element.optString("nodeType", "")
                    appendVoidInline(
                        result,
                        nodeType,
                        baseFontSize,
                        textColor,
                        blockStack,
                        pendingLeadingMargins,
                        theme,
                        density
                    )
                }

                "voidBlock" -> {
                    val nodeType = element.optString("nodeType", "")
                        if (!isFirstBlock) {
                            val spacingPx = ((nextBlockSpacingBefore ?: 0f) * density).toInt()
                            appendInterBlockNewline(result, baseFontSize, textColor, spacingPx)
                        }
                        isFirstBlock = false
                    val spacingBefore = theme?.effectiveTextStyle(nodeType)?.spacingAfter
                        ?: theme?.list?.itemSpacing
                    nextBlockSpacingBefore = spacingBefore
                    appendVoidBlock(
                        result,
                        nodeType,
                        baseFontSize,
                        textColor,
                        theme,
                        density,
                        spacingBefore
                    )
                }

                "opaqueInlineAtom" -> {
                    val nodeType = element.optString("nodeType", "")
                    val label = element.optString("label", "?")
                    appendOpaqueInlineAtom(
                        result,
                        nodeType,
                        label,
                        baseFontSize,
                        textColor,
                        blockStack,
                        pendingLeadingMargins,
                        theme,
                        density
                    )
                }

                "opaqueBlockAtom" -> {
                    val nodeType = element.optString("nodeType", "")
                    val label = element.optString("label", "?")
                    val blockSpacing = theme?.effectiveTextStyle(nodeType)?.spacingAfter
                    if (!isFirstBlock) {
                        val spacingPx = ((nextBlockSpacingBefore ?: 0f) * density).toInt()
                        appendInterBlockNewline(result, baseFontSize, textColor, spacingPx)
                    }
                    isFirstBlock = false
                    nextBlockSpacingBefore = blockSpacing
                    appendOpaqueBlockAtom(result, nodeType, label, baseFontSize, textColor, theme, blockSpacing)
                }

                "blockStart" -> {
                    val nodeType = element.optString("nodeType", "")
                    val depth = element.optInt("depth", 0)
                    val listContext = element.optJSONObject("listContext")
                    val isListItemContainer = nodeType == "listItem" && listContext != null
                    val nestedListItemContainer =
                        isListItemContainer && blockStack.any { it.nodeType == "listItem" && it.listContext != null }
                    val blockSpacing = if (isListItemContainer) {
                        null
                    } else {
                        theme?.effectiveTextStyle(nodeType)?.spacingAfter
                            ?: (if (listContext != null) theme?.list?.itemSpacing else null)
                    }

                    if (!isListItemContainer) {
                        if (!isFirstBlock) {
                            val spacingPx = ((nextBlockSpacingBefore ?: 0f) * density).toInt()
                            appendInterBlockNewline(result, baseFontSize, textColor, spacingPx)
                        }
                        isFirstBlock = false
                        nextBlockSpacingBefore = blockSpacing
                    } else if (nestedListItemContainer && theme?.list?.itemSpacing != null) {
                        nextBlockSpacingBefore = theme.list.itemSpacing
                    }

                    val ctx = BlockContext(
                        nodeType = nodeType,
                        depth = depth,
                        listContext = listContext,
                        markerPending = isListItemContainer,
                        renderStart = result.length
                    )
                    blockStack.add(ctx)

                    val markerListContext = when {
                        isListItemContainer -> null
                        listContext != null -> listContext
                        else -> consumePendingListMarker(blockStack, result.length)
                    }

                    if (markerListContext != null) {
                        val ordered = markerListContext.optBoolean("ordered", false)
                        val marker = listMarkerString(markerListContext)
                        val markerBaseSize =
                            resolveTextStyle(nodeType, theme).fontSize?.times(density) ?: baseFontSize
                        val markerTextStyle = resolveTextStyle(nodeType, theme)
                        appendStyledText(
                            result,
                            marker,
                            emptyList(),
                            markerBaseSize,
                            theme?.list?.markerColor ?: textColor,
                            blockStack,
                            pendingLeadingMargins,
                            null,
                            density,
                            applyBlockSpans = false
                        )
                        if (!ordered) {
                            val markerStart = result.length - marker.length
                            val markerEnd = result.length
                            val markerScale =
                                theme?.list?.markerScale ?: LayoutConstants.UNORDERED_LIST_MARKER_FONT_SCALE
                            val markerWidth = calculateMarkerWidth(density)
                            val bulletRadius = ((markerBaseSize * markerScale) * 0.16f).coerceAtLeast(2f * density)
                            result.setSpan(
                                CenteredBulletSpan(
                                    textColor = theme?.list?.markerColor ?: textColor,
                                    markerWidthPx = markerWidth,
                                    bulletRadiusPx = bulletRadius,
                                    bodyFontSizePx = markerBaseSize,
                                    markerGapToTextPx = LayoutConstants.LIST_MARKER_TEXT_GAP * density
                                ),
                                markerStart,
                                markerEnd,
                                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
                            )
                        }
                        applyLineHeightSpan(
                            builder = result,
                            start = result.length - marker.length,
                            end = result.length,
                            lineHeight = markerTextStyle.lineHeight,
                            density = density
                        )
                    }
                }

                "blockEnd" -> {
                    if (blockStack.isNotEmpty()) {
                        val endedBlock = blockStack.removeAt(blockStack.lastIndex)
                        if (endedBlock.nodeType == "listItem" && endedBlock.listContext != null) {
                            nextBlockSpacingBefore = theme?.list?.itemSpacing
                        }
                    }
                }
            }
        }

        applyPendingLeadingMargins(result, pendingLeadingMargins)
        return result
    }

    // ── Mark Handling ───────────────────────────────────────────────────

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
    private fun appendStyledText(
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

        val currentBlock = effectiveBlockContext(blockStack)
        val textStyle = currentBlock?.let { resolveTextStyle(it.nodeType, theme) } ?: theme?.effectiveTextStyle("paragraph")
        val resolvedTextSize = textStyle?.fontSize?.times(density) ?: baseFontSize
        val resolvedTextColor = textStyle?.color ?: textColor

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

        // Determine which marks are active.
        var hasBold = textStyle?.typefaceStyle()?.let { it == Typeface.BOLD || it == Typeface.BOLD_ITALIC } == true
        var hasItalic = textStyle?.typefaceStyle()?.let { it == Typeface.ITALIC || it == Typeface.BOLD_ITALIC } == true
        var hasUnderline = false
        var hasStrike = false
        var hasCode = false
        var linkHref: String? = null

        for (mark in marks) {
            when {
                mark is String -> when (mark) {
                    "bold", "strong" -> hasBold = true
                    "italic", "em" -> hasItalic = true
                    "underline" -> hasUnderline = true
                    "strike", "strikethrough" -> hasStrike = true
                    "code" -> hasCode = true
                }
                mark is JSONObject -> {
                    val markType = mark.optString("type", "")
                    if (markType == "link") {
                        linkHref = mark.takeUnless { it.isNull("href") }?.optString("href")
                    }
                }
            }
        }

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

        val fontFamily = textStyle?.fontFamily
        if (!hasCode && !fontFamily.isNullOrBlank()) {
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

        if (hasCode) {
            builder.setSpan(
                TypefaceSpan("monospace"), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
            builder.setSpan(
                BackgroundColorSpan(LayoutConstants.CODE_BACKGROUND_COLOR),
                start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
        }

        if (linkHref != null) {
            builder.setSpan(URLSpan(linkHref), start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }

        // Apply block-level indentation spans if in a block context.
        if (applyBlockSpans) {
            applyBlockStyle(builder, start, end, blockStack, pendingLeadingMargins, theme, density)
        }
    }

    // ── Void Inline Elements ────────────────────────────────────────────

    /**
     * Append a void inline element (e.g. hardBreak) to the builder.
     *
     * A hardBreak is rendered as a newline character. Unknown void inlines
     * are rendered as the object replacement character.
     */
    private fun appendVoidInline(
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
            "hardBreak" -> {
                val start = builder.length
                builder.append("\n")
                val end = builder.length
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

    // ── Void Block Elements ─────────────────────────────────────────────

    /**
     * Append a void block element (e.g. horizontalRule) to the builder.
     *
     * Horizontal rules are rendered as the object replacement character
     * with a [HorizontalRuleSpan] that draws a separator line.
     */
    private fun appendVoidBlock(
        builder: SpannableStringBuilder,
        nodeType: String,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme?,
        density: Float,
        spacingBefore: Float?
    ) {
        when (nodeType) {
            "horizontalRule" -> {
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
                        verticalPadding = (theme?.horizontalRule?.verticalMargin ?: LayoutConstants.HORIZONTAL_RULE_VERTICAL_PADDING) * density
                    ),
                    start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
                )
            }
            else -> {
                builder.append(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER)
            }
        }
    }

    // ── Opaque Atoms ────────────────────────────────────────────────────

    /**
     * Append an opaque inline atom (unknown inline void) as a bracketed label.
     */
    private fun appendOpaqueInlineAtom(
        builder: SpannableStringBuilder,
        nodeType: String,
        label: String,
        baseFontSize: Float,
        textColor: Int,
        blockStack: MutableList<BlockContext>,
        pendingLeadingMargins: MutableMap<Int, PendingLeadingMargin>,
        theme: EditorTheme?,
        density: Float
    ) {
        val isMention = nodeType == "mention"
        val text = if (isMention) label else "[$label]"
        val start = builder.length
        builder.append(text)
        val end = builder.length
        val inlineTextColor = if (isMention) {
            theme?.mentions?.textColor ?: resolveInlineTextColor(blockStack, textColor, theme)
        } else {
            resolveInlineTextColor(blockStack, textColor, theme)
        }
        builder.setSpan(
            ForegroundColorSpan(inlineTextColor),
            start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        builder.setSpan(
            BackgroundColorSpan(
                if (isMention) {
                    theme?.mentions?.backgroundColor ?: 0x1f1d4ed8
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
        if (isMention && (theme?.mentions?.fontWeight == "bold" ||
                theme?.mentions?.fontWeight?.toIntOrNull()?.let { it >= 600 } == true)
        ) {
            builder.setSpan(
                StyleSpan(Typeface.BOLD),
                start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
        }
        applyBlockStyle(builder, start, end, blockStack, pendingLeadingMargins, theme, density)
    }

    /**
     * Append an opaque block atom (unknown block void) as a bracketed label.
     */
    private fun appendOpaqueBlockAtom(
        builder: SpannableStringBuilder,
        nodeType: String,
        label: String,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme?,
        spacingBefore: Float?
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
    }

    // ── Block Styling ───────────────────────────────────────────────────

    /**
     * Apply the current block context's indentation as a [LeadingMarginSpan]
     * to a span range.
     */
    private fun applyBlockStyle(
        builder: SpannableStringBuilder,
        start: Int,
        end: Int,
        blockStack: List<BlockContext>,
        pendingLeadingMargins: MutableMap<Int, PendingLeadingMargin>,
        theme: EditorTheme?,
        density: Float
    ) {
        val currentBlock = effectiveBlockContext(blockStack) ?: return
        val indent = calculateIndent(currentBlock, theme, density)
        val markerWidth = calculateMarkerWidth(density)
        val paragraphStart = effectiveParagraphStart(blockStack).coerceIn(0, builder.length)
        if (paragraphStart < end) {
            if (currentBlock.listContext != null) {
                pendingLeadingMargins[paragraphStart] = PendingLeadingMargin(
                    indentPx = indent.toInt(),
                    restIndentPx = (indent + markerWidth).toInt()
                )
            } else if (indent > 0) {
                pendingLeadingMargins[paragraphStart] = PendingLeadingMargin(
                    indentPx = indent.toInt(),
                    restIndentPx = null
                )
            }
        }

        val lineHeight = resolveTextStyle(currentBlock.nodeType, theme).lineHeight
        applyLineHeightSpan(builder, start, end, lineHeight, density)
    }

    private fun applyLineHeightSpan(
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

    private fun applyPendingLeadingMargins(
        builder: SpannableStringBuilder,
        pendingLeadingMargins: Map<Int, PendingLeadingMargin>
    ) {
        if (pendingLeadingMargins.isEmpty()) return

        val text = builder.toString()
        pendingLeadingMargins.toSortedMap().forEach { (paragraphStart, spec) ->
            if (paragraphStart >= builder.length) {
                return@forEach
            }
            val newlineIndex = text.indexOf('\n', paragraphStart)
            val paragraphEnd = if (newlineIndex >= 0) newlineIndex + 1 else builder.length
            val span = spec.restIndentPx?.let {
                LeadingMarginSpan.Standard(spec.indentPx, it)
            } ?: LeadingMarginSpan.Standard(spec.indentPx)

            builder
                .getSpans(0, builder.length, LeadingMarginSpan.Standard::class.java)
                .filter { builder.getSpanStart(it) == paragraphStart }
                .forEach(builder::removeSpan)

            builder.setSpan(span, paragraphStart, paragraphEnd, Spanned.SPAN_PARAGRAPH)
        }
    }


    /**
     * Calculate the leading margin indent for a block context.
     *
     * List items get the base depth indent. The list marker width is handled
     * by the marker text itself, matching the iOS hanging indent approach.
     */
    private fun calculateIndent(context: BlockContext, theme: EditorTheme?, density: Float): Float {
        val indentPerDepth = (theme?.list?.indent ?: LayoutConstants.INDENT_PER_DEPTH) * density
        return context.depth * indentPerDepth
    }

    private fun effectiveBlockContext(blockStack: List<BlockContext>): BlockContext? {
        val currentBlock = blockStack.lastOrNull() ?: return null
        if (currentBlock.listContext != null) {
            return currentBlock
        }
        val inheritedListContext = blockStack
            .dropLast(1)
            .asReversed()
            .firstOrNull { it.listContext != null }
            ?.listContext ?: return currentBlock
        return currentBlock.copy(listContext = inheritedListContext, markerPending = false)
    }

    private fun effectiveParagraphStart(blockStack: List<BlockContext>): Int {
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

    private fun consumePendingListMarker(
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

    private fun calculateMarkerWidth(density: Float): Float {
        return LayoutConstants.LIST_MARKER_WIDTH * density
    }

    private fun resolveTextStyle(nodeType: String, theme: EditorTheme?): EditorTextStyle {
        return theme?.effectiveTextStyle(nodeType) ?: EditorTextStyle()
    }

    private fun resolveInlineTextColor(
        blockStack: List<BlockContext>,
        fallbackColor: Int,
        theme: EditorTheme?
    ): Int {
        val nodeType = effectiveBlockContext(blockStack)?.nodeType ?: "paragraph"
        return resolveTextStyle(nodeType, theme).color ?: fallbackColor
    }

    // ── List Markers ────────────────────────────────────────────────────

    /**
     * Generate the list marker string (bullet or number) from a list context.
     *
     * @param listContext A [JSONObject] with at least `ordered` (bool) and `index` (int).
     * @return The marker string, e.g. "1. " for ordered or bullet + space for unordered.
     */
    fun listMarkerString(listContext: JSONObject): String {
        val ordered = listContext.optBoolean("ordered", false)
        return if (ordered) {
            val index = listContext.optInt("index", 1)
            "$index. "
        } else {
            LayoutConstants.UNORDERED_LIST_BULLET
        }
    }

    // ── Private Helpers ─────────────────────────────────────────────────

    /**
     * Parse a [JSONArray] of marks into a list of mark identifiers.
     *
     * Each mark can be either a plain string (e.g. "bold") or a JSON object
     * (e.g. `{"type": "link", "href": "https://..."}`). Returns a mixed list
     * of [String] and [JSONObject].
     */
    private fun parseMarks(marksArray: JSONArray?): List<Any> {
        if (marksArray == null || marksArray.length() == 0) return emptyList()
        val marks = mutableListOf<Any>()
        for (i in 0 until marksArray.length()) {
            // Try as string first, then as object.
            val markStr = marksArray.optString(i, null)
            if (markStr != null && markStr != "null") {
                marks.add(markStr)
            } else {
                val markObj = marksArray.optJSONObject(i)
                if (markObj != null) {
                    marks.add(markObj)
                }
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
    private fun appendInterBlockNewline(
        builder: SpannableStringBuilder,
        baseFontSize: Float,
        textColor: Int,
        spacingPx: Int = 0
    ) {
        val start = builder.length
        builder.append("\n")
        val end = builder.length
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
    }
}
