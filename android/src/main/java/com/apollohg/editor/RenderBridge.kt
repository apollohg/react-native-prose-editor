package com.apollohg.editor

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.util.Base64
import android.util.Log
import android.util.LruCache
import android.text.Annotation
import android.text.Layout
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
import android.text.style.UnderlineSpan
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.lang.ref.WeakReference
import java.net.URL
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit

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

    /** Total leading inset reserved for each blockquote depth. */
    const val BLOCKQUOTE_INDENT: Float = 18f

    /** Width of the rendered blockquote border bar (pixels at base scale). */
    const val BLOCKQUOTE_BORDER_WIDTH: Float = 3f

    /** Gap between the blockquote border bar and the text that follows. */
    const val BLOCKQUOTE_MARKER_GAP: Float = 8f

    /** Bullet character for unordered list items. */
    const val UNORDERED_LIST_BULLET: String = "\u2022 "

    /** Scale factor applied only to unordered list marker glyphs. */
    const val UNORDERED_LIST_MARKER_FONT_SCALE: Float = 2.0f

    /** Default visual treatment for link text when no explicit theme color exists. */
    const val DEFAULT_LINK_COLOR: Int = 0xFF1B73E8.toInt()

    /** Object replacement character used for void block elements. */
    const val OBJECT_REPLACEMENT_CHARACTER: String = "\uFFFC"

    /** Zero-width placeholder used to preserve trailing hard-break lines. */
    const val SYNTHETIC_PLACEHOLDER_CHARACTER: String = "\u200B"

    /** Background color for inline code spans (light gray). */
    const val CODE_BACKGROUND_COLOR: Int = 0x1A000000  // 10% black
}

data class BlockContext(
    val nodeType: String,
    val depth: Int,
    val listContext: JSONObject?,
    val topLevelChildIndex: Int? = null,
    var markerPending: Boolean = false,
    var renderStart: Int = 0
)

private data class PendingLeadingMargin(
    val indentPx: Int,
    val restIndentPx: Int?,
    val blockquoteIndentPx: Int = 0,
    val blockquoteStripeColor: Int? = null,
    val blockquoteStripeWidthPx: Int = 0,
    val blockquoteGapWidthPx: Int = 0,
    val blockquoteBaseIndentPx: Int = 0
)

class BlockquoteSpan(
    private val baseIndentPx: Int,
    private val totalIndentPx: Int,
    private val stripeColor: Int,
    private val stripeWidthPx: Int,
    private val gapWidthPx: Int
    ) : LeadingMarginSpan {

    override fun getLeadingMargin(first: Boolean): Int = totalIndentPx

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
        if (!lineContainsQuotedContent(text, start, end)) {
            return
        }

        val savedColor = paint.color
        val savedStyle = paint.style

        paint.color = stripeColor
        paint.style = Paint.Style.FILL

        val stripeStart = x + (dir * baseIndentPx)
        val stripeLeft = if (dir > 0) stripeStart.toFloat() else (stripeStart - stripeWidthPx).toFloat()
        val stripeRight = if (dir > 0) stripeLeft + stripeWidthPx else stripeLeft + stripeWidthPx
        val stripeBottom = resolvedStripeBottom(
            text = text,
            start = start,
            end = end,
            baseline = baseline,
            bottom = bottom,
            layout = layout,
            paint = paint
        )
        canvas.drawRect(
            stripeLeft,
            top.toFloat(),
            stripeRight,
            stripeBottom,
            paint
        )

        paint.color = savedColor
        paint.style = savedStyle
    }

    private fun lineContainsQuotedContent(text: CharSequence, start: Int, end: Int): Boolean {
        if (start >= end || text !is Spanned) return true
        for (index in start until end.coerceAtMost(text.length)) {
            val ch = text[index]
            if (ch == '\n' || ch == '\r') continue
            val quoted = text.getSpans(index, index + 1, Annotation::class.java).any {
                it.key == RenderBridge.NATIVE_BLOCKQUOTE_ANNOTATION
            }
            if (quoted) {
                return true
            }
        }
        return false
    }

    internal fun resolvedStripeBottom(
        text: CharSequence,
        start: Int,
        end: Int,
        baseline: Int,
        bottom: Int,
        layout: android.text.Layout?,
        paint: Paint? = null
    ): Float {
        if (layout == null || text.isEmpty()) {
            return bottom.toFloat()
        }
        val lineIndex = safeLineForOffset(layout, start, text.length)
        val nextLine = lineIndex + 1
        if (nextLine >= layout.lineCount) {
            return trimmedTextBottom(baseline, layout, lineIndex, paint)
        }

        val nextLineStart = layout.getLineStart(nextLine)
        val nextLineEnd = layout.getLineEnd(nextLine)
        return if (lineContainsQuotedContent(text, nextLineStart, nextLineEnd)) {
            bottom.toFloat()
        } else {
            trimmedTextBottom(baseline, layout, lineIndex, paint)
        }
    }

    private fun trimmedTextBottom(
        baseline: Int,
        layout: Layout,
        lineIndex: Int,
        paint: Paint?
    ): Float {
        val fontDescent = paint?.fontMetrics?.descent
        return if (fontDescent != null) {
            baseline + fontDescent
        } else {
            (baseline + layout.getLineDescent(lineIndex)).toFloat()
        }
    }

    private fun safeLineForOffset(layout: Layout, offset: Int, textLength: Int): Int {
        if (textLength <= 0) return 0
        val safeStart = offset.coerceIn(0, textLength - 1)
        return layout.getLineForOffset(safeStart)
    }
}

class HorizontalRuleSpan(
    private val lineColor: Int,
    private val lineHeight: Float = LayoutConstants.HORIZONTAL_RULE_HEIGHT,
    private val verticalPadding: Float = LayoutConstants.HORIZONTAL_RULE_VERTICAL_PADDING
) : ReplacementSpan(), LeadingMarginSpan {

    override fun getLeadingMargin(first: Boolean): Int = 0

    override fun getSize(
        paint: Paint,
        text: CharSequence,
        start: Int,
        end: Int,
        fm: Paint.FontMetricsInt?
    ): Int {
        if (fm != null) {
            val totalHeight = kotlin.math.ceil(lineHeight + (verticalPadding * 2)).toInt()
            val halfHeight = totalHeight / 2
            fm.ascent = -halfHeight
            fm.top = fm.ascent
            fm.descent = totalHeight - halfHeight
            fm.bottom = fm.descent
        }
        // Keep the placeholder atom in the text model without reserving
        // visible glyph width, so Android does not paint a tofu/OBJ box.
        return 0
    }

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
        // Intentionally empty: drawLeadingMargin renders the separator line,
        // and ReplacementSpan suppresses drawing the underlying FFFC glyph.
    }
}

internal object RenderImageDecoder {
    private const val MAX_DECODE_DIMENSION_PX = 2048
    internal const val LOG_TAG = "NativeEditorImage"

    fun decodeSource(source: String): Bitmap? {
        decodeDataUrlBytes(source)?.let { bytes ->
            val decoded = decodeBitmap(bytes)
            if (decoded == null) {
                Log.w(LOG_TAG, "decodeSource: failed to decode data URL bytes (${sourceSummary(source)})")
            } else {
                Log.d(
                    LOG_TAG,
                    "decodeSource: decoded data URL ${sourceSummary(source)} -> ${decoded.width}x${decoded.height}"
                )
            }
            return decoded
        }
        val remoteBytes = runCatching {
            URL(source).openStream().use { input ->
                input.readBytes()
            }
        }.getOrNull() ?: run {
            Log.w(LOG_TAG, "decodeSource: failed to load remote image (${sourceSummary(source)})")
            return null
        }
        val decoded = decodeBitmap(remoteBytes)
        if (decoded == null) {
            Log.w(LOG_TAG, "decodeSource: failed to decode remote bytes (${sourceSummary(source)})")
        }
        return decoded
    }

    fun decodeDataUrlBytes(source: String): ByteArray? {
        val trimmed = source.trim()
        if (!trimmed.startsWith("data:image/", ignoreCase = true)) return null
        val commaIndex = trimmed.indexOf(',')
        if (commaIndex <= 0) return null
        val metadata = trimmed.substring(0, commaIndex).lowercase()
        if (!metadata.contains(";base64")) return null
        val payload = trimmed.substring(commaIndex + 1)
            .filterNot(Char::isWhitespace)

        val decodeFlags = intArrayOf(
            Base64.DEFAULT,
            Base64.NO_WRAP,
            Base64.URL_SAFE or Base64.NO_WRAP,
            Base64.URL_SAFE
        )
        for (flags in decodeFlags) {
            val bytes = runCatching { Base64.decode(payload, flags) }.getOrNull()
            if (bytes != null) {
                return bytes
            }
        }
        Log.w(LOG_TAG, "decodeDataUrlBytes: unsupported base64 payload (${sourceSummary(source)})")
        return null
    }

    fun calculateInSampleSize(
        width: Int,
        height: Int,
        maxWidth: Int = MAX_DECODE_DIMENSION_PX,
        maxHeight: Int = MAX_DECODE_DIMENSION_PX
    ): Int {
        if (width <= 0 || height <= 0) return 1

        var sampleSize = 1
        var sampledWidth = width
        var sampledHeight = height
        while (sampledWidth > maxWidth || sampledHeight > maxHeight) {
            sampleSize *= 2
            sampledWidth = width / sampleSize
            sampledHeight = height / sampleSize
        }
        return sampleSize.coerceAtLeast(1)
    }

    private fun decodeBitmap(bytes: ByteArray): Bitmap? {
        val bounds = BitmapFactory.Options().apply {
            inJustDecodeBounds = true
        }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
            Log.w(LOG_TAG, "decodeBitmap: invalid image bounds for ${bytes.size} bytes")
            return null
        }

        val options = BitmapFactory.Options().apply {
            inSampleSize = calculateInSampleSize(bounds.outWidth, bounds.outHeight)
        }
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
    }

    private fun sourceSummary(source: String): String {
        val trimmed = source.trim()
        if (!trimmed.startsWith("data:image/", ignoreCase = true)) {
            return "urlLength=${trimmed.length}"
        }
        val commaIndex = trimmed.indexOf(',')
        if (commaIndex <= 0) {
            return "dataUrlLength=${trimmed.length}"
        }
        val metadata = trimmed.substring(0, commaIndex)
        val payloadLength = trimmed.length - commaIndex - 1
        return "$metadata payloadLength=$payloadLength"
    }
}

internal object RenderImageLoader {
    private val cache = object : LruCache<String, Bitmap>(32 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap): Int = value.byteCount
    }
    private val executor =
        ThreadPoolExecutor(
            2,
            2,
            30L,
            TimeUnit.SECONDS,
            LinkedBlockingQueue()
        )
    private val lock = Any()
    private val inFlight = mutableMapOf<String, MutableList<(Bitmap?) -> Unit>>()

    @Volatile
    internal var decodeSourceOverride: ((String) -> Bitmap?)? = null

    fun cached(source: String): Bitmap? = synchronized(cache) { cache.get(source) }

    internal fun resetForTesting() {
        synchronized(cache) {
            cache.evictAll()
        }
        synchronized(lock) {
            inFlight.clear()
        }
        decodeSourceOverride = null
    }

    fun load(source: String, onLoaded: (Bitmap?) -> Unit) {
        cached(source)?.let {
            onLoaded(it)
            return
        }

        if (source.trim().startsWith("data:image/", ignoreCase = true)) {
            val bitmap = decode(source)
            if (bitmap != null) {
                synchronized(cache) {
                    cache.put(source, bitmap)
                }
            }
            onLoaded(bitmap)
            return
        }

        synchronized(lock) {
            val existing = inFlight[source]
            if (existing != null) {
                existing += onLoaded
                return
            }
            inFlight[source] = mutableListOf(onLoaded)
        }

        executor.execute {
            val bitmap = decode(source)
            if (bitmap != null) {
                synchronized(cache) {
                    cache.put(source, bitmap)
                }
            }
            val callbacks = synchronized(lock) {
                inFlight.remove(source) ?: mutableListOf()
            }
            callbacks.forEach { it(bitmap) }
        }
    }

    private fun decode(source: String): Bitmap? {
        return decodeSourceOverride?.invoke(source) ?: RenderImageDecoder.decodeSource(source)
    }
}

internal class BlockImageSpan(
    private val source: String,
    hostView: TextView?,
    private val density: Float,
    private val preferredWidthDp: Float?,
    private val preferredHeightDp: Float?
) : ReplacementSpan() {
    private val hostRef = WeakReference(hostView)

    @Volatile
    private var bitmap: Bitmap? = RenderImageLoader.cached(source)
    @Volatile
    private var lastDrawRect: RectF? = null

    init {
        if (bitmap == null) {
            RenderImageLoader.load(source) { loaded ->
                if (loaded == null) {
                    Log.w(
                        RenderImageDecoder.LOG_TAG,
                        "BlockImageSpan: loader returned null for image source"
                    )
                    return@load
                }
                bitmap = loaded
                hostRef.get()?.post {
                    hostRef.get()?.requestLayout()
                    hostRef.get()?.invalidate()
                    (hostRef.get() as? EditorEditText)?.onSelectionOrContentMayChange?.invoke()
                }
            }
        }
    }

    override fun getSize(
        paint: Paint,
        text: CharSequence,
        start: Int,
        end: Int,
        fm: Paint.FontMetricsInt?
    ): Int {
        val (widthPx, heightPx) = currentSizePx()
        if (fm != null) {
            fm.ascent = -heightPx
            fm.descent = 0
            fm.top = fm.ascent
            fm.bottom = 0
        }
        return widthPx
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
        val (widthPx, heightPx) = currentSizePx()
        val rect = RectF(
            x,
            (bottom - heightPx).toFloat(),
            x + widthPx,
            bottom.toFloat()
        )
        val host = hostRef.get()
        lastDrawRect = RectF(rect).apply {
            if (host != null) {
                offset(host.compoundPaddingLeft.toFloat(), host.extendedPaddingTop.toFloat())
            }
        }
        val loadedBitmap = bitmap
        if (loadedBitmap != null) {
            canvas.drawBitmap(loadedBitmap, null, rect, null)
            return
        }

        val previousColor = paint.color
        val previousStyle = paint.style
        paint.color = Color.argb(24, 0, 0, 0)
        paint.style = Paint.Style.FILL
        canvas.drawRoundRect(rect, 16f * density, 16f * density, paint)
        paint.color = Color.argb(120, 0, 0, 0)
        val iconRadius = minOf(rect.width(), rect.height()) * 0.12f
        canvas.drawCircle(rect.centerX(), rect.centerY(), iconRadius, paint)
        paint.color = previousColor
        paint.style = previousStyle
    }

    internal fun currentSizePx(): Pair<Int, Int> {
        val maxWidth = resolvedMaxWidth()
        val loadedBitmap = bitmap
        val fallbackAspectRatio = if (loadedBitmap != null && loadedBitmap.width > 0 && loadedBitmap.height > 0) {
            loadedBitmap.height.toFloat() / loadedBitmap.width.toFloat()
        } else {
            0.56f
        }

        var widthPx = preferredWidthDp?.takeIf { it > 0f }?.times(density)
        var heightPx = preferredHeightDp?.takeIf { it > 0f }?.times(density)

        if (widthPx == null && heightPx == null && loadedBitmap != null && loadedBitmap.width > 0 && loadedBitmap.height > 0) {
            widthPx = loadedBitmap.width.toFloat()
            heightPx = loadedBitmap.height.toFloat()
        } else if (widthPx == null && heightPx != null) {
            widthPx = heightPx / fallbackAspectRatio
        } else if (heightPx == null && widthPx != null) {
            heightPx = widthPx * fallbackAspectRatio
        }

        if (widthPx == null || heightPx == null) {
            val placeholderWidth = maxWidth.coerceAtLeast(160f * density)
            val placeholderHeight = minOf(
                180f * density,
                placeholderWidth * fallbackAspectRatio
            ).coerceAtLeast(96f * density)
            widthPx = placeholderWidth
            heightPx = placeholderHeight
        }

        val scale = minOf(1f, maxWidth / widthPx.coerceAtLeast(1f))
        return Pair(
            (widthPx * scale).toInt().coerceAtLeast(1),
            (heightPx * scale).toInt().coerceAtLeast(1)
        )
    }

    internal fun currentDrawRect(): RectF? = lastDrawRect?.let(::RectF)

    private fun resolvedMaxWidth(): Float {
        val host = hostRef.get()
        val hostWidth = host?.let {
            maxOf(it.width, it.measuredWidth) - it.totalPaddingLeft - it.totalPaddingRight
        } ?: 0
        return if (hostWidth > 0) hostWidth.toFloat() else 240f * density
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

object RenderBridge {
    internal const val NATIVE_BLOCKQUOTE_ANNOTATION = "nativeBlockquote"
    internal const val NATIVE_TOP_LEVEL_CHILD_INDEX_ANNOTATION = "nativeTopLevelChildIndex"
    private const val NATIVE_SYNTHETIC_PLACEHOLDER_ANNOTATION = "nativeSyntheticPlaceholder"

    private data class RenderBuildState(
        val result: SpannableStringBuilder = SpannableStringBuilder(),
        val blockStack: MutableList<BlockContext> = mutableListOf(),
        val pendingLeadingMargins: MutableMap<Int, PendingLeadingMargin> = linkedMapOf(),
        var isFirstBlock: Boolean = true,
        var nextBlockSpacingBefore: Float? = null
    )

    fun buildSpannable(
        json: String,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme? = null,
        density: Float = 1f,
        hostView: TextView? = null
    ): SpannableStringBuilder {
        val elements = try {
            JSONArray(json)
        } catch (_: Exception) {
            return SpannableStringBuilder()
        }

        return buildSpannableFromArray(elements, baseFontSize, textColor, theme, density, hostView)
    }

    fun buildSpannableFromArray(
        elements: JSONArray,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme? = null,
        density: Float = 1f,
        hostView: TextView? = null
    ): SpannableStringBuilder {
        val state = RenderBuildState()
        appendElements(
            state = state,
            elements = elements,
            baseFontSize = baseFontSize,
            textColor = textColor,
            theme = theme,
            density = density,
            hostView = hostView
        )
        applyPendingLeadingMargins(state.result, state.pendingLeadingMargins)
        return state.result
    }

    fun buildSpannableFromBlocks(
        blocks: JSONArray,
        startIndex: Int = 0,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme? = null,
        density: Float = 1f,
        hostView: TextView? = null
    ): SpannableStringBuilder {
        val state = RenderBuildState()
        for (blockOffset in 0 until blocks.length()) {
            val blockElements = blocks.optJSONArray(blockOffset) ?: continue
            appendElements(
                state = state,
                elements = blockElements,
                baseFontSize = baseFontSize,
                textColor = textColor,
                theme = theme,
                density = density,
                hostView = hostView,
                topLevelChildIndex = startIndex + blockOffset
            )
        }
        applyPendingLeadingMargins(state.result, state.pendingLeadingMargins)
        return state.result
    }

    private fun appendElements(
        state: RenderBuildState,
        elements: JSONArray,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme?,
        density: Float,
        hostView: TextView?,
        topLevelChildIndex: Int? = null
    ) {
        for (i in 0 until elements.length()) {
            val element = elements.optJSONObject(i) ?: continue
            val type = element.optString("type", "")

            when (type) {
                "textRun" -> {
                    val text = element.optString("text", "")
                    val marksArray = element.optJSONArray("marks")
                    val marks = parseMarks(marksArray)
                    appendStyledText(
                        state.result,
                        text,
                        marks,
                        baseFontSize,
                        textColor,
                        state.blockStack,
                        state.pendingLeadingMargins,
                        theme,
                        density
                    )
                }

                "voidInline" -> {
                    val nodeType = element.optString("nodeType", "")
                    appendVoidInline(
                        state.result,
                        nodeType,
                        baseFontSize,
                        textColor,
                        state.blockStack,
                        state.pendingLeadingMargins,
                        theme,
                        density
                    )
                }

                "voidBlock" -> {
                    val nodeType = element.optString("nodeType", "")
                    val attrs = element.optJSONObject("attrs")
                    if (!state.isFirstBlock) {
                        val spacingPx = ((state.nextBlockSpacingBefore ?: 0f) * density).toInt()
                        appendInterBlockNewline(
                            state.result,
                            baseFontSize,
                            textColor,
                            spacingPx,
                            topLevelChildIndex = topLevelChildIndex
                        )
                    }
                    state.isFirstBlock = false
                    val spacingBefore = theme?.effectiveTextStyle(nodeType)?.spacingAfter
                        ?: theme?.list?.itemSpacing
                    state.nextBlockSpacingBefore = spacingBefore
                    appendVoidBlock(
                        state.result,
                        nodeType,
                        attrs,
                        baseFontSize,
                        textColor,
                        theme,
                        density,
                        spacingBefore,
                        hostView,
                        topLevelChildIndex
                    )
                }

                "opaqueInlineAtom" -> {
                    val nodeType = element.optString("nodeType", "")
                    val label = element.optString("label", "?")
                    val docPos = element.optInt("docPos", 0)
                    appendOpaqueInlineAtom(
                        state.result,
                        nodeType,
                        label,
                        docPos,
                        baseFontSize,
                        textColor,
                        state.blockStack,
                        state.pendingLeadingMargins,
                        theme,
                        density
                    )
                }

                "opaqueBlockAtom" -> {
                    val nodeType = element.optString("nodeType", "")
                    val label = element.optString("label", "?")
                    val docPos = element.optInt("docPos", 0)
                    val blockSpacing = theme?.effectiveTextStyle(nodeType)?.spacingAfter
                    if (!state.isFirstBlock) {
                        val spacingPx = ((state.nextBlockSpacingBefore ?: 0f) * density).toInt()
                        appendInterBlockNewline(
                            state.result,
                            baseFontSize,
                            textColor,
                            spacingPx,
                            topLevelChildIndex = topLevelChildIndex
                        )
                    }
                    state.isFirstBlock = false
                    state.nextBlockSpacingBefore = blockSpacing
                    appendOpaqueBlockAtom(
                        state.result,
                        nodeType,
                        label,
                        docPos,
                        baseFontSize,
                        textColor,
                        theme,
                        blockSpacing,
                        topLevelChildIndex
                    )
                }

                "blockStart" -> {
                    val nodeType = element.optString("nodeType", "")
                    val depth = element.optInt("depth", 0)
                    val listContext = element.optJSONObject("listContext")
                    val isListItemContainer = nodeType == "listItem" && listContext != null
                    val isTransparentContainer = nodeType == "blockquote"
                    val nestedListItemContainer =
                        isListItemContainer && state.blockStack.any { it.nodeType == "listItem" && it.listContext != null }
                    val blockSpacing = if (isListItemContainer) {
                        null
                    } else {
                        theme?.effectiveTextStyle(nodeType)?.spacingAfter
                            ?: (if (listContext != null) theme?.list?.itemSpacing else null)
                    }

                    if (!isListItemContainer && !isTransparentContainer) {
                        if (!state.isFirstBlock) {
                            val spacingPx = ((state.nextBlockSpacingBefore ?: 0f) * density).toInt()
                            val nextBlockStack = state.blockStack + BlockContext(
                                nodeType = nodeType,
                                depth = depth,
                                listContext = listContext,
                                topLevelChildIndex = topLevelChildIndex,
                                markerPending = isListItemContainer,
                                renderStart = state.result.length
                            )
                            val inBlockquoteSeparator =
                                blockquoteDepth(nextBlockStack) > 0f && trailingRenderedContentHasBlockquote(state.result)
                            appendInterBlockNewline(
                                state.result,
                                baseFontSize,
                                textColor,
                                spacingPx,
                                inBlockquote = inBlockquoteSeparator,
                                topLevelChildIndex = topLevelChildIndex
                            )
                        }
                        state.isFirstBlock = false
                        state.nextBlockSpacingBefore = blockSpacing
                    } else if (nestedListItemContainer && theme?.list?.itemSpacing != null) {
                        state.nextBlockSpacingBefore = theme.list.itemSpacing
                    }

                    val ctx = BlockContext(
                        nodeType = nodeType,
                        depth = depth,
                        listContext = listContext,
                        topLevelChildIndex = topLevelChildIndex,
                        markerPending = isListItemContainer,
                        renderStart = state.result.length
                    )
                    state.blockStack.add(ctx)

                    val markerListContext = when {
                        isListItemContainer -> null
                        listContext != null -> listContext
                        else -> consumePendingListMarker(state.blockStack, state.result.length)
                    }

                    if (markerListContext != null) {
                        val ordered = markerListContext.optBoolean("ordered", false)
                        val marker = listMarkerString(markerListContext)
                        val markerBaseSize =
                            resolveTextStyle(
                                nodeType,
                                theme,
                                blockquoteDepth(state.blockStack) > 0
                            ).fontSize?.times(density) ?: baseFontSize
                        val markerTextStyle = resolveTextStyle(
                            nodeType,
                            theme,
                            blockquoteDepth(state.blockStack) > 0
                        )
                        appendStyledText(
                            state.result,
                            marker,
                            emptyList(),
                            markerBaseSize,
                            theme?.list?.markerColor ?: textColor,
                            state.blockStack,
                            state.pendingLeadingMargins,
                            null,
                            density,
                            applyBlockSpans = false
                        )
                        val markerStart = state.result.length - marker.length
                        val markerEnd = state.result.length
                        annotateTopLevelChild(state.result, markerStart, markerEnd, topLevelChildIndex)
                        if (!ordered) {
                            val markerScale =
                                theme?.list?.markerScale ?: LayoutConstants.UNORDERED_LIST_MARKER_FONT_SCALE
                            val markerWidth = calculateMarkerWidth(density)
                            val bulletRadius = ((markerBaseSize * markerScale) * 0.16f).coerceAtLeast(2f * density)
                            state.result.setSpan(
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
                            builder = state.result,
                            start = markerStart,
                            end = markerEnd,
                            lineHeight = markerTextStyle.lineHeight,
                            density = density
                        )
                    }
                }

                "blockEnd" -> {
                    if (state.blockStack.isNotEmpty()) {
                        val endedBlock = state.blockStack.removeAt(state.blockStack.lastIndex)
                        appendTrailingHardBreakPlaceholderIfNeeded(
                            builder = state.result,
                            endedBlock = endedBlock,
                            remainingBlockStack = state.blockStack,
                            baseFontSize = baseFontSize,
                            textColor = textColor,
                            theme = theme,
                            density = density,
                            pendingLeadingMargins = state.pendingLeadingMargins
                        )
                        if (endedBlock.nodeType == "listItem" && endedBlock.listContext != null) {
                            state.nextBlockSpacingBefore = theme?.list?.itemSpacing
                        }
                    }
                }
            }
        }
    }

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
        val textStyle = currentBlock?.let {
            resolveTextStyle(
                it.nodeType,
                theme,
                blockquoteDepth(blockStack) > 0
            )
        } ?: theme?.effectiveTextStyle("paragraph", inBlockquote = blockquoteDepth(blockStack) > 0)
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
                        hasUnderline = true
                        builder.setSpan(
                            ForegroundColorSpan(LayoutConstants.DEFAULT_LINK_COLOR),
                            start,
                            end,
                            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
                        )
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
    private fun appendVoidBlock(
        builder: SpannableStringBuilder,
        nodeType: String,
        attrs: JSONObject?,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme?,
        density: Float,
        spacingBefore: Float?,
        hostView: TextView?,
        topLevelChildIndex: Int?
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
                annotateTopLevelChild(builder, start, end, topLevelChildIndex)
            }
            "image" -> {
                val source = if (attrs != null && attrs.has("src") && !attrs.isNull("src")) {
                    attrs.optString("src", "").trim()
                } else {
                    ""
                }
                val preferredWidthDp = attrs?.optPositiveFloat("width")
                val preferredHeightDp = attrs?.optPositiveFloat("height")
                if (source.isEmpty()) {
                    builder.append(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER)
                    return
                }
                val start = builder.length
                builder.append(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER)
                val end = builder.length
                builder.setSpan(
                    BlockImageSpan(
                        source = source,
                        hostView = hostView,
                        density = density,
                        preferredWidthDp = preferredWidthDp,
                        preferredHeightDp = preferredHeightDp
                    ),
                    start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
                )
                annotateTopLevelChild(builder, start, end, topLevelChildIndex)
            }
            else -> {
                val start = builder.length
                builder.append(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER)
                annotateTopLevelChild(builder, start, builder.length, topLevelChildIndex)
            }
        }
    }

    private fun appendOpaqueInlineAtom(
        builder: SpannableStringBuilder,
        nodeType: String,
        label: String,
        docPos: Int,
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
        builder.setSpan(
            Annotation("nativeDocPos", docPos.toString()),
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

    private fun appendOpaqueBlockAtom(
        builder: SpannableStringBuilder,
        nodeType: String,
        label: String,
        docPos: Int,
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
        val indent = calculateIndent(currentBlock, blockStack, theme, density)
        val markerWidth = calculateMarkerWidth(density)
        val quoteDepth = blockquoteDepth(blockStack)
        val quoteStripeColor = if (quoteDepth > 0) {
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
            ((currentBlock.depth * ((theme?.list?.indent ?: LayoutConstants.INDENT_PER_DEPTH) * density))
                - (quoteDepth * ((theme?.list?.indent ?: LayoutConstants.INDENT_PER_DEPTH) * density))
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
                    restIndentPx = (indent + markerWidth).toInt(),
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

        val lineHeight = resolveTextStyle(
            currentBlock.nodeType,
            theme,
            quoteDepth > 0
        ).lineHeight
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
                    .filter { builder.getSpanStart(it) == paragraphStart }
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
                    .filter { builder.getSpanStart(it) == paragraphStart }
                    .forEach(builder::removeSpan)

                builder.setSpan(span, paragraphStart, paragraphEnd, Spanned.SPAN_PARAGRAPH)
            }
        }
    }

    private fun applyAdditionalLeadingMargin(
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


    private fun calculateIndent(
        context: BlockContext,
        blockStack: List<BlockContext>,
        theme: EditorTheme?,
        density: Float
    ): Float {
        val indentPerDepth = (theme?.list?.indent ?: LayoutConstants.INDENT_PER_DEPTH) * density
        val quoteDepth = blockquoteDepth(blockStack)
        val quoteIndent = maxOf(
            theme?.blockquote?.indent ?: LayoutConstants.BLOCKQUOTE_INDENT,
            (theme?.blockquote?.markerGap ?: LayoutConstants.BLOCKQUOTE_MARKER_GAP) +
                (theme?.blockquote?.borderWidth ?: LayoutConstants.BLOCKQUOTE_BORDER_WIDTH)
        ) * density
        return (context.depth * indentPerDepth) - (quoteDepth * indentPerDepth) + (quoteDepth * quoteIndent)
    }

    private fun effectiveBlockContext(blockStack: List<BlockContext>): BlockContext? {
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

    private fun renderedParagraphStart(
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

    private fun blockquoteDepth(blockStack: List<BlockContext>): Float {
        return blockStack.count { it.nodeType == "blockquote" }.toFloat()
    }

    private fun resolveTextStyle(
        nodeType: String,
        theme: EditorTheme?,
        inBlockquote: Boolean = false
    ): EditorTextStyle {
        return theme?.effectiveTextStyle(nodeType, inBlockquote) ?: EditorTextStyle()
    }

    private fun resolveInlineTextColor(
        blockStack: List<BlockContext>,
        fallbackColor: Int,
        theme: EditorTheme?
    ): Int {
        val nodeType = effectiveBlockContext(blockStack)?.nodeType ?: "paragraph"
        return resolveTextStyle(nodeType, theme, blockquoteDepth(blockStack) > 0).color ?: fallbackColor
    }

    fun listMarkerString(listContext: JSONObject): String {
        val ordered = listContext.optBoolean("ordered", false)
        return if (ordered) {
            val index = listContext.optInt("index", 1)
            "$index. "
        } else {
            LayoutConstants.UNORDERED_LIST_BULLET
        }
    }

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
    private fun appendInterBlockNewline(
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

    private fun appendTrailingHardBreakPlaceholderIfNeeded(
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
        if (endedBlock.nodeType == "listItem") return
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

    private fun lastCharacterIsHardBreak(builder: SpannableStringBuilder): Boolean {
        if (builder.isEmpty()) return false
        val lastIndex = builder.length - 1
        return builder.getSpans(lastIndex, builder.length, Annotation::class.java).any {
            it.key == "nativeVoidNodeType" && it.value == "hardBreak"
        }
    }

    private fun annotateTopLevelChild(
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
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }

    private fun trailingRenderedContentHasBlockquote(builder: Spanned): Boolean {
        for (index in builder.length - 1 downTo 0) {
            val ch = builder[index]
            if (ch == '\n' || ch == '\r') continue
            return hasBlockquoteAnnotationAt(builder, index)
        }
        return false
    }

    private fun defaultParagraphEnd(text: String, length: Int, paragraphStart: Int): Int {
        val newlineIndex = text.indexOf('\n', paragraphStart)
        return if (newlineIndex >= 0) newlineIndex + 1 else length
    }

    private fun blockquoteSpanEnd(
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

    private fun hasBlockquoteAnnotationAt(text: Spanned, index: Int): Boolean {
        if (index < 0 || index >= text.length) return false
        return text.getSpans(index, index + 1, Annotation::class.java).any {
            it.key == NATIVE_BLOCKQUOTE_ANNOTATION
        }
    }
}

private fun JSONObject.optPositiveFloat(key: String): Float? {
    if (!has(key) || isNull(key)) return null
    val value = optDouble(key, Double.NaN)
    if (value.isNaN() || value <= 0.0) return null
    return value.toFloat()
}
