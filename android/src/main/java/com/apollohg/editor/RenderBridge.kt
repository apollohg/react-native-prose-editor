package com.apollohg.editor

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.os.Handler
import android.os.Looper
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
import android.text.style.LineBackgroundSpan
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
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.Future
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

object LayoutConstants {
    /** Base indentation per depth level (pixels at base scale). */
    const val INDENT_PER_DEPTH: Float = 24f

    /** Width reserved for the list bullet/number (pixels at base scale). */
    const val LIST_MARKER_WIDTH: Float = 36f

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

    /** Rendered marker text for task list items. Must stay in sync with the
     *  Rust core's task_list_marker_string (render/mod.rs) — the marker's
     *  scalar length is part of the position-mapping contract. */
    const val TASK_LIST_MARKER_CHECKED: String = "☑ "
    const val TASK_LIST_MARKER_UNCHECKED: String = "☐ "

    /** Scale factor applied to task checkbox marker glyphs. */
    const val TASK_LIST_MARKER_FONT_SCALE: Float = 1.55f

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

private data class PendingCodeBlockSpan(
    val start: Int,
    val end: Int
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

class CodeBlockSpan(
    private val backgroundColor: Int,
    private val cornerRadiusPx: Float,
    private val paddingHorizontalPx: Int,
    private val paddingVerticalPx: Int
) : LeadingMarginSpan, LineBackgroundSpan {
    override fun getLeadingMargin(first: Boolean): Int = paddingHorizontalPx

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
        layout: Layout
    ) = Unit

    override fun drawBackground(
        canvas: Canvas,
        paint: Paint,
        left: Int,
        right: Int,
        top: Int,
        baseline: Int,
        bottom: Int,
        text: CharSequence,
        start: Int,
        end: Int,
        lineNumber: Int
    ) {
        val spanned = text as? Spanned ?: return
        val spanStart = spanned.getSpanStart(this)
        val spanEnd = spanned.getSpanEnd(this)
        if (spanStart < 0 || start >= spanEnd || end <= spanStart) return

        val isFirstLine = start <= spanStart
        val isLastLine = end >= spanEnd
        val rect = RectF(
            left.toFloat(),
            if (isFirstLine) top.toFloat() - paddingVerticalPx else top.toFloat(),
            (right - paddingHorizontalPx).toFloat(),
            if (isLastLine) bottom.toFloat() + paddingVerticalPx else bottom.toFloat()
        )

        val savedColor = paint.color
        val savedStyle = paint.style
        paint.color = backgroundColor
        paint.style = Paint.Style.FILL

        when {
            isFirstLine && isLastLine -> canvas.drawRoundRect(rect, cornerRadiusPx, cornerRadiusPx, paint)
            isFirstLine -> {
                canvas.drawRoundRect(rect, cornerRadiusPx, cornerRadiusPx, paint)
                canvas.drawRect(rect.left, rect.centerY(), rect.right, rect.bottom, paint)
            }
            isLastLine -> {
                canvas.drawRoundRect(rect, cornerRadiusPx, cornerRadiusPx, paint)
                canvas.drawRect(rect.left, rect.top, rect.right, rect.centerY(), paint)
            }
            else -> canvas.drawRect(rect, paint)
        }

        paint.color = savedColor
        paint.style = savedStyle
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

internal data class ImageLoadingPolicy(
    val maxSourceBytes: Int,
    val connectTimeoutMs: Int,
    val readTimeoutMs: Int,
    val maxConcurrentRequests: Int,
    val maxPendingRequests: Int,
    val maxDecodeDimensionPx: Int
) {
    companion object {
        val DEFAULT = ImageLoadingPolicy(
            maxSourceBytes = 10 * 1024 * 1024,
            connectTimeoutMs = 10_000,
            readTimeoutMs = 20_000,
            maxConcurrentRequests = 2,
            maxPendingRequests = 64,
            maxDecodeDimensionPx = 2_048
        )

        fun fromJson(json: String?): ImageLoadingPolicy {
            val values = runCatching { json?.let(::JSONObject) }.getOrNull() ?: return DEFAULT
            fun positiveInt(key: String, fallback: Int): Int {
                val value = values.opt(key) as? Number ?: return fallback
                val doubleValue = value.toDouble()
                if (!doubleValue.isFinite() || doubleValue % 1.0 != 0.0 || doubleValue <= 0.0 ||
                    doubleValue > Int.MAX_VALUE.toDouble()
                ) return fallback
                return doubleValue.toInt()
            }
            return ImageLoadingPolicy(
                positiveInt("maxSourceBytes", DEFAULT.maxSourceBytes),
                positiveInt("connectTimeoutMs", DEFAULT.connectTimeoutMs),
                positiveInt("readTimeoutMs", DEFAULT.readTimeoutMs),
                positiveInt("maxConcurrentRequests", DEFAULT.maxConcurrentRequests),
                positiveInt("maxPendingRequests", DEFAULT.maxPendingRequests),
                positiveInt("maxDecodeDimensionPx", DEFAULT.maxDecodeDimensionPx)
            )
        }
    }
}

internal object RenderImageDecoder {
    internal const val LOG_TAG = "NativeEditorImage"

    @Volatile
    internal var connectionFactoryOverride: ((URL) -> HttpURLConnection)? = null
    @Volatile
    internal var bitmapDecoderOverride: ((ByteArray, ImageLoadingPolicy) -> Bitmap?)? = null

    internal class Cancellation {
        private val cancelled = AtomicBoolean(false)
        @Volatile private var connection: HttpURLConnection? = null
        @Volatile private var stream: InputStream? = null

        fun isCancelled(): Boolean = cancelled.get()

        fun bind(connection: HttpURLConnection) {
            this.connection = connection
            if (cancelled.get()) connection.disconnect()
        }

        fun bind(stream: InputStream) {
            this.stream = stream
            if (cancelled.get()) runCatching { stream.close() }
        }

        fun cancel() {
            if (!cancelled.compareAndSet(false, true)) return
            connection?.disconnect()
            runCatching { stream?.close() }
        }
    }

    fun decodeSource(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT,
        cancellation: Cancellation? = null
    ): Bitmap? {
        if (cancellation?.isCancelled() == true) return null
        decodeDataUrlBytes(source, policy)?.let { bytes ->
            if (cancellation?.isCancelled() == true) return null
            val decoded = decodeBitmap(bytes, policy)
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
        val connection = runCatching {
            connectionFactoryOverride?.invoke(URL(source))
                ?: (URL(source).openConnection() as HttpURLConnection)
        }.getOrNull() ?: return null
        cancellation?.bind(connection)
        val remoteBytes = try {
            connection.connectTimeout = policy.connectTimeoutMs
            connection.readTimeout = policy.readTimeoutMs
            connection.instanceFollowRedirects = true
            val status = connection.responseCode
            if (status !in 200..299 || connection.contentLengthLong > policy.maxSourceBytes) {
                null
            } else {
                connection.inputStream.use { input ->
                    cancellation?.bind(input)
                    readBounded(input, policy.maxSourceBytes, cancellation)
                }
            }
        } catch (_: Exception) {
            null
        } finally {
            connection.disconnect()
        } ?: run {
            Log.w(LOG_TAG, "decodeSource: failed to load remote image (${sourceSummary(source)})")
            return null
        }
        if (cancellation?.isCancelled() == true) return null
        val decoded = decodeBitmap(remoteBytes, policy)
        if (decoded == null) {
            Log.w(LOG_TAG, "decodeSource: failed to decode remote bytes (${sourceSummary(source)})")
        }
        return decoded
    }

    fun decodeDataUrlBytes(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
    ): ByteArray? {
        val trimmed = source.trim()
        if (!trimmed.startsWith("data:image/", ignoreCase = true)) return null
        val commaIndex = trimmed.indexOf(',')
        if (commaIndex <= 0) return null
        val metadata = trimmed.substring(0, commaIndex).lowercase()
        if (!metadata.contains(";base64")) return null
        val rawPayload = trimmed.substring(commaIndex + 1)
        val payloadCharacters = rawPayload.count { !it.isWhitespace() }
        val maximumEncodedCharacters = ((policy.maxSourceBytes.toLong() + 2L) / 3L) * 4L
        if (payloadCharacters.toLong() > maximumEncodedCharacters) return null
        val payload = rawPayload.filterNot(Char::isWhitespace)

        val decodeFlags = intArrayOf(
            Base64.DEFAULT,
            Base64.NO_WRAP,
            Base64.URL_SAFE or Base64.NO_WRAP,
            Base64.URL_SAFE
        )
        for (flags in decodeFlags) {
            val bytes = runCatching { Base64.decode(payload, flags) }.getOrNull()
            if (bytes != null && bytes.size <= policy.maxSourceBytes) {
                return bytes
            }
        }
        Log.w(LOG_TAG, "decodeDataUrlBytes: unsupported base64 payload (${sourceSummary(source)})")
        return null
    }

    fun readBounded(
        input: InputStream,
        maxBytes: Int,
        cancellation: Cancellation? = null
    ): ByteArray? {
        val output = ByteArrayOutputStream(minOf(maxBytes, 16 * 1024))
        val buffer = ByteArray(8 * 1024)
        var total = 0
        while (true) {
            if (cancellation?.isCancelled() == true) return null
            val read = input.read(buffer)
            if (read < 0) break
            total += read
            if (total > maxBytes) return null
            output.write(buffer, 0, read)
        }
        return output.toByteArray()
    }

    fun calculateInSampleSize(
        width: Int,
        height: Int,
        maxWidth: Int = ImageLoadingPolicy.DEFAULT.maxDecodeDimensionPx,
        maxHeight: Int = ImageLoadingPolicy.DEFAULT.maxDecodeDimensionPx
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

    private fun decodeBitmap(bytes: ByteArray, policy: ImageLoadingPolicy): Bitmap? {
        bitmapDecoderOverride?.let { return it(bytes, policy) }
        val bounds = BitmapFactory.Options().apply {
            inJustDecodeBounds = true
        }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
            Log.w(LOG_TAG, "decodeBitmap: invalid image bounds for ${bytes.size} bytes")
            return null
        }

        val options = BitmapFactory.Options().apply {
            inSampleSize = calculateInSampleSize(
                bounds.outWidth,
                bounds.outHeight,
                policy.maxDecodeDimensionPx,
                policy.maxDecodeDimensionPx
            )
        }
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
    }

    internal fun resetForTesting() {
        connectionFactoryOverride = null
        bitmapDecoderOverride = null
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
    private data class RequestKey(val source: String, val policy: ImageLoadingPolicy)
    internal class LoadHandle(private val cancelAction: () -> Unit) {
        fun cancel() = cancelAction()
    }
    private data class Callback(
        val id: Long,
        val cancelled: AtomicBoolean,
        val deliver: (Bitmap?) -> Unit
    )
    private class PendingRequest(
        val callbacks: MutableList<Callback>,
        val cancellation: RenderImageDecoder.Cancellation,
        val executor: ThreadPoolExecutor
    ) {
        var future: Future<*>? = null
    }

    private val cache = object : LruCache<String, Bitmap>(32 * 1024 * 1024) {
        override fun sizeOf(key: String, value: Bitmap): Int = value.byteCount
    }
    private val lock = Any()
    private val inFlight = mutableMapOf<RequestKey, PendingRequest>()
    private val executors = mutableMapOf<ImageLoadingPolicy, ThreadPoolExecutor>()
    private val nextCallbackId = AtomicLong()
    private val mainHandler by lazy { Handler(Looper.getMainLooper()) }

    @Volatile
    internal var decodeSourceOverride: ((String, ImageLoadingPolicy) -> Bitmap?)? = null

    fun cached(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
    ): Bitmap? = synchronized(cache) { cache.get(cacheKey(source, policy)) }

    internal fun resetForTesting() {
        synchronized(cache) {
            cache.evictAll()
        }
        synchronized(lock) {
            inFlight.values.forEach { pending ->
                pending.cancellation.cancel()
                pending.future?.cancel(true)
            }
            inFlight.clear()
            executors.values.forEach { it.shutdownNow() }
            executors.clear()
        }
        decodeSourceOverride = null
    }

    internal fun activeExecutorCountForTesting(): Int = synchronized(lock) { executors.size }

    fun load(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT,
        onLoaded: (Bitmap?) -> Unit
    ): LoadHandle {
        val cancelled = AtomicBoolean(false)
        val callback = Callback(nextCallbackId.incrementAndGet(), cancelled, onLoaded)
        val requestKey = RequestKey(source, policy)
        cached(source, policy)?.let { bitmap ->
            mainHandler.post { if (!cancelled.get()) onLoaded(bitmap) }
            return LoadHandle { cancelled.set(true) }
        }
        var shouldStart = false
        lateinit var pending: PendingRequest
        synchronized(lock) {
            val existing = inFlight[requestKey]
            if (existing != null) {
                existing.callbacks += callback
                pending = existing
            } else {
                pending = PendingRequest(
                    callbacks = mutableListOf(callback),
                    cancellation = RenderImageDecoder.Cancellation(),
                    executor = executor(policy)
                )
                inFlight[requestKey] = pending
                shouldStart = true
            }
        }
        if (shouldStart) try {
            val future = pending.executor.submit {
                val bitmap = decode(source, policy, pending.cancellation)
                if (bitmap != null && !pending.cancellation.isCancelled()) {
                    synchronized(cache) {
                        cache.put(cacheKey(source, policy), bitmap)
                    }
                }
                val callbacks = synchronized(lock) {
                    val current = inFlight[requestKey]
                    if (current === pending) inFlight.remove(requestKey)
                    retireExecutorIfUnused(policy)
                    pending.callbacks.toList()
                }
                mainHandler.post {
                    callbacks.forEach { if (!it.cancelled.get()) it.deliver(bitmap) }
                }
            }
            synchronized(lock) {
                pending.future = future
                if (inFlight[requestKey] !== pending) {
                    future.cancel(true)
                    pending.executor.remove(future as Runnable)
                }
            }
        } catch (_: RejectedExecutionException) {
            val callbacks = synchronized(lock) {
                if (inFlight[requestKey] === pending) inFlight.remove(requestKey)
                retireExecutorIfUnused(policy)
                pending.callbacks.toList()
            }
            mainHandler.post { callbacks.forEach { if (!it.cancelled.get()) it.deliver(null) } }
        }
        return LoadHandle {
            cancelled.set(true)
            synchronized(lock) {
                val current = inFlight[requestKey]
                if (current === pending) {
                    current.callbacks.removeAll { it.id == callback.id }
                    if (current.callbacks.isEmpty()) {
                        inFlight.remove(requestKey)
                        current.cancellation.cancel()
                        current.future?.let { future ->
                            future.cancel(true)
                            current.executor.remove(future as Runnable)
                        }
                        retireExecutorIfUnused(policy)
                    }
                }
            }
        }
    }

    private fun executor(policy: ImageLoadingPolicy): ThreadPoolExecutor = synchronized(lock) {
        executors.getOrPut(policy) {
            ThreadPoolExecutor(
                policy.maxConcurrentRequests,
                policy.maxConcurrentRequests,
                30L,
                TimeUnit.SECONDS,
                ArrayBlockingQueue(policy.maxPendingRequests)
            ).apply { allowCoreThreadTimeOut(true) }
        }
    }

    private fun retireExecutorIfUnused(policy: ImageLoadingPolicy) {
        if (inFlight.keys.any { it.policy == policy }) return
        executors.remove(policy)?.shutdown()
    }

    private fun decode(
        source: String,
        policy: ImageLoadingPolicy,
        cancellation: RenderImageDecoder.Cancellation
    ): Bitmap? = decodeSourceOverride?.invoke(source, policy)
        ?: RenderImageDecoder.decodeSource(source, policy, cancellation)

    private fun cacheKey(source: String, policy: ImageLoadingPolicy) = "$policy\n$source"
}

internal class BlockImageSpan(
    private val source: String,
    hostView: TextView?,
    private val density: Float,
    private val preferredWidthDp: Float?,
    private val preferredHeightDp: Float?
) : ReplacementSpan() {
    private val hostRef = WeakReference(hostView)
    private val policy = (hostView as? EditorEditText)?.imageLoadingPolicy
        ?: ImageLoadingPolicy.DEFAULT
    private val generation = (hostView as? EditorEditText)?.currentImageLoadGeneration()

    @Volatile
    private var bitmap: Bitmap? = RenderImageLoader.cached(source, policy)
    @Volatile
    private var lastDrawRect: RectF? = null

    init {
        if (bitmap == null) {
            val handle = RenderImageLoader.load(source, policy) { loaded ->
                val currentHost = hostRef.get()
                if (
                    currentHost is EditorEditText &&
                    generation != currentHost.currentImageLoadGeneration()
                ) {
                    return@load
                }
                if (loaded == null) {
                    Log.w(
                        RenderImageDecoder.LOG_TAG,
                        "BlockImageSpan: loader returned null for image source"
                    )
                    return@load
                }
                bitmap = loaded
                currentHost?.post {
                    if (
                        currentHost is EditorEditText &&
                        generation != currentHost.currentImageLoadGeneration()
                    ) return@post
                    currentHost.requestLayout()
                    currentHost.invalidate()
                    (currentHost as? EditorEditText)?.onSelectionOrContentMayChange?.invoke()
                }
            }
            (hostView as? EditorEditText)?.registerImageLoad(handle)
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
    internal const val NATIVE_LINK_HREF_ANNOTATION = "nativeLinkHref"
    internal const val NATIVE_TASK_LIST_MARKER_ANNOTATION = "nativeTaskListMarker"
    private const val NATIVE_SYNTHETIC_PLACEHOLDER_ANNOTATION = "nativeSyntheticPlaceholder"

    private data class RenderBuildState(
        val result: SpannableStringBuilder = SpannableStringBuilder(),
        val blockStack: MutableList<BlockContext> = mutableListOf(),
        val pendingLeadingMargins: MutableMap<Int, PendingLeadingMargin> = linkedMapOf(),
        val pendingCodeBlockSpans: MutableList<PendingCodeBlockSpan> = mutableListOf(),
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
        applyPendingCodeBlockSpans(state.result, state.pendingCodeBlockSpans, theme, density)
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
        applyPendingCodeBlockSpans(state.result, state.pendingCodeBlockSpans, theme, density)
        return state.result
    }

    fun measureHeight(
        json: String,
        themeJson: String?,
        width: Float,
        density: Float
    ): Float {
        if (width <= 0) return 0f

        val theme = EditorTheme.fromJson(themeJson)
        val baseFontSize = theme?.text?.fontSize
            ?: theme?.paragraph?.fontSize
            ?: 16f

        val spannable = buildSpannable(
            json = json,
            baseFontSize = baseFontSize,
            textColor = android.graphics.Color.BLACK,
            theme = theme,
            density = density,
            hostView = null
        )

        if (spannable.isEmpty()) return 0f

        val contentInsets = theme?.contentInsets
        val topInset = ((contentInsets?.top ?: 0f) * density).toInt()
        val bottomInset = ((contentInsets?.bottom ?: 0f) * density).toInt()
        val leftInset = ((contentInsets?.left ?: 0f) * density).toInt()
        val rightInset = ((contentInsets?.right ?: 0f) * density).toInt()

        val paint = android.text.TextPaint().apply {
            textSize = baseFontSize * density
            isAntiAlias = true
        }

        val availableWidth = (width - leftInset - rightInset).coerceAtLeast(0f).toInt()

        val staticLayout = android.text.StaticLayout.Builder
            .obtain(spannable, 0, spannable.length, paint, availableWidth)
            .setAlignment(android.text.Layout.Alignment.ALIGN_NORMAL)
            .setIncludePad(true)
            .build()

        val height = staticLayout.height + topInset + bottomInset
        return height.toFloat()
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
                    val mentionTheme = EditorMentionTheme.fromJson(
                        element.optJSONObject("mentionTheme")
                    )
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
                        mentionTheme,
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
                    val isListItemContainer = isListItemNodeType(nodeType) && listContext != null
                    val isTransparentContainer = isTransparentContainer(nodeType)
                    val nestedListItemContainer =
                        isListItemContainer &&
                            state.blockStack.any {
                                isListItemNodeType(it.nodeType) && it.listContext != null
                            }
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
                        val isTask = markerListContext.optString("kind", "") == "task"
                        val marker = listMarkerString(markerListContext)
                        val markerBaseSize =
                            resolveTextStyle(
                                nodeType,
                                theme,
                                blockquoteDepth(state.blockStack) > 0
                            ).fontSize?.times(density) ?: baseFontSize
                        val resolvedMarkerBaseSize = if (isTask) {
                            markerBaseSize * LayoutConstants.TASK_LIST_MARKER_FONT_SCALE
                        } else {
                            markerBaseSize
                        }
                        val markerTextStyle = resolveTextStyle(
                            nodeType,
                            theme,
                            blockquoteDepth(state.blockStack) > 0
                        )
                        appendStyledText(
                            state.result,
                            marker,
                            emptyList(),
                            resolvedMarkerBaseSize,
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
                        if (isTask) {
                            state.result.setSpan(
                                Annotation(NATIVE_TASK_LIST_MARKER_ANNOTATION, "1"),
                                markerStart,
                                markerEnd,
                                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
                            )
                        }
                        if (!ordered && !isTask) {
                            val markerScale =
                                theme?.list?.markerScale ?: LayoutConstants.UNORDERED_LIST_MARKER_FONT_SCALE
                            val markerWidth = calculateMarkerWidth(density)
                            val bulletRadius = ((markerBaseSize * markerScale) * 0.16f).coerceAtLeast(2f * density)
                            state.result.setSpan(
                                CenteredBulletSpan(
                                    textColor = theme?.list?.markerColor ?: textColor,
                                    markerWidthPx = markerWidth,
                                    bulletRadiusPx = bulletRadius,
                                    bodyFontSizePx = resolvedMarkerBaseSize,
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
                        if (isListItemNodeType(endedBlock.nodeType) && endedBlock.listContext != null) {
                            state.nextBlockSpacingBefore = theme?.list?.itemSpacing
                        }
                        if (endedBlock.nodeType == "codeBlock" && endedBlock.renderStart < state.result.length) {
                            state.pendingCodeBlockSpans.add(
                                PendingCodeBlockSpan(
                                    start = endedBlock.renderStart,
                                    end = state.result.length
                                )
                            )
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
        mentionTheme: EditorMentionTheme?,
        density: Float
    ) {
        val isMention = nodeType == "mention"
        val text = if (isMention) label else "[$label]"
        val start = builder.length
        builder.append(text)
        val end = builder.length
        val resolvedMentionTheme = if (isMention) {
            theme?.mentions?.mergedWith(mentionTheme) ?: mentionTheme
        } else {
            null
        }
        val inlineTextColor = if (isMention) {
            resolvedMentionTheme?.textColor ?: resolveInlineTextColor(blockStack, textColor, theme)
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
        if (isMention && (resolvedMentionTheme?.fontWeight == "bold" ||
                resolvedMentionTheme?.fontWeight?.toIntOrNull()?.let { it >= 600 } == true)
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
        val indentPerDepth = (theme?.list?.indent ?: LayoutConstants.INDENT_PER_DEPTH) * density
        val listBaseIndentAdjustment =
            calculateListBaseIndentAdjustment(currentBlock, theme, density)
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

    private fun applyPendingCodeBlockSpans(
        builder: SpannableStringBuilder,
        pendingCodeBlockSpans: List<PendingCodeBlockSpan>,
        theme: EditorTheme?,
        density: Float
    ) {
        if (pendingCodeBlockSpans.isEmpty()) return

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

    private fun calculateListBaseIndentAdjustment(
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

    private fun columnContainerDepth(blockStack: List<BlockContext>): Float {
        return blockStack.count { it.nodeType == "columns" || it.nodeType == "column" }.toFloat()
    }

    private fun isTransparentContainer(nodeType: String): Boolean {
        return nodeType == "blockquote" || nodeType == "columns" || nodeType == "column"
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
        if (listContext.optString("kind", "") == "task") {
            return if (listContext.optBoolean("checked", false)) {
                LayoutConstants.TASK_LIST_MARKER_CHECKED
            } else {
                LayoutConstants.TASK_LIST_MARKER_UNCHECKED
            }
        }
        val ordered = listContext.optBoolean("ordered", false)
        return if (ordered) {
            val index = listContext.optInt("index", 1)
            "$index. "
        } else {
            LayoutConstants.UNORDERED_LIST_BULLET
        }
    }

    private fun isListItemNodeType(nodeType: String): Boolean {
        return nodeType == "listItem" || nodeType == "taskItem"
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
