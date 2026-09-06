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
import android.os.SystemClock
import android.util.Base64
import android.util.Log
import android.util.Xml
import com.caverock.androidsvg.SVG
import org.xmlpull.v1.XmlPullParser
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
import java.nio.ByteBuffer
import java.security.MessageDigest
import java.util.concurrent.Callable
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.Semaphore
import java.util.concurrent.Future
import java.util.concurrent.FutureTask
import java.util.concurrent.PriorityBlockingQueue
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

internal data class ImageLoadingPolicy(
    val maxSourceBytes: Int,
    val connectTimeoutMs: Int,
    val readTimeoutMs: Int,
    val requestTimeoutMs: Int,
    val maxConcurrentRequests: Int,
    val maxPendingRequests: Int,
    val maxDecodeDimensionPx: Int,
    val maxDecodedBytes: Int,
) {
    companion object {
        val DEFAULT = ImageLoadingPolicy(
            maxSourceBytes = 10 * 1024 * 1024,
            connectTimeoutMs = 10_000,
            readTimeoutMs = 20_000,
            requestTimeoutMs = 60_000,
            maxConcurrentRequests = 2,
            maxPendingRequests = 64,
            maxDecodeDimensionPx = 2_048,
            maxDecodedBytes = 32 * 1024 * 1024,
        )

        fun fromJson(json: String?): ImageLoadingPolicy {
            val values = runCatching { json?.let(::JSONObject) }.getOrNull() ?: return DEFAULT
            fun boundedPositiveInt(key: String, fallback: Int, hardMaximum: Int): Int {
                val value = values.opt(key) as? Number ?: return fallback
                val doubleValue = value.toDouble()
                if (!doubleValue.isFinite() || doubleValue % 1.0 != 0.0 || doubleValue <= 0.0 ||
                    doubleValue > hardMaximum.toDouble()
                ) return fallback
                return doubleValue.toInt()
            }
            return ImageLoadingPolicy(
                boundedPositiveInt("maxSourceBytes", DEFAULT.maxSourceBytes, 64 * 1024 * 1024),
                boundedPositiveInt("connectTimeoutMs", DEFAULT.connectTimeoutMs, 600_000),
                boundedPositiveInt("readTimeoutMs", DEFAULT.readTimeoutMs, 600_000),
                boundedPositiveInt("requestTimeoutMs", DEFAULT.requestTimeoutMs, 600_000),
                boundedPositiveInt("maxConcurrentRequests", DEFAULT.maxConcurrentRequests, 16),
                boundedPositiveInt("maxPendingRequests", DEFAULT.maxPendingRequests, 512),
                boundedPositiveInt("maxDecodeDimensionPx", DEFAULT.maxDecodeDimensionPx, 8_192),
                boundedPositiveInt("maxDecodedBytes", DEFAULT.maxDecodedBytes, 256 * 1024 * 1024),
            )
        }

        internal fun canonicalBytes(policy: ImageLoadingPolicy): ByteArray = ByteBuffer
            .allocate(Int.SIZE_BYTES * 8)
            .putInt(policy.maxSourceBytes)
            .putInt(policy.connectTimeoutMs)
            .putInt(policy.readTimeoutMs)
            .putInt(policy.requestTimeoutMs)
            .putInt(policy.maxConcurrentRequests)
            .putInt(policy.maxPendingRequests)
            .putInt(policy.maxDecodeDimensionPx)
            .putInt(policy.maxDecodedBytes)
            .array()
    }
}

internal fun interface MonotonicClock {
    fun elapsedRealtime(): Long
}

private val systemMonotonicClock = MonotonicClock { SystemClock.elapsedRealtime() }

private fun deadlineAfter(startedAtMs: Long, timeoutMs: Int): Long =
    if (startedAtMs > Long.MAX_VALUE - timeoutMs.toLong()) Long.MAX_VALUE
    else startedAtMs + timeoutMs.toLong()

internal object RenderImageDecoder {
    internal const val LOG_TAG = "NativeEditorImage"

    @Volatile
    internal var connectionFactoryOverride: ((URL) -> HttpURLConnection)? = null
    @Volatile
    internal var bitmapDecoderOverride: ((ByteArray, ImageLoadingPolicy) -> Bitmap?)? = null

    internal data class DataUrlAdmission(
        val commaIndex: Int,
        val maximumEncodedCharacters: Long,
        val sanitizedPayloadLength: Int
    )

    internal class Cancellation {
        private val cancelled = AtomicBoolean(false)
        @Volatile private var connection: HttpURLConnection? = null
        @Volatile private var stream: InputStream? = null

        fun isCancelled(): Boolean = cancelled.get()

        fun bind(connection: HttpURLConnection) {
            this.connection = connection
            if (cancelled.get()) runCatching { connection.disconnect() }
        }

        fun bind(stream: InputStream) {
            this.stream = stream
            if (cancelled.get()) runCatching { stream.close() }
        }

        fun cancel() {
            if (!cancelled.compareAndSet(false, true)) return
            runCatching { connection?.disconnect() }
            runCatching { stream?.close() }
        }
    }

    internal fun decodeSourceLease(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT,
        cancellation: Cancellation? = null,
        clock: MonotonicClock = systemMonotonicClock,
        deadlineMs: Long = deadlineAfter(clock.elapsedRealtime(), policy.requestTimeoutMs),
        priority: DecodedBitmapPriority = DecodedBitmapPriority.VISIBLE,
    ): DecodedBitmapLease? {
        if (unavailable(cancellation, clock, deadlineMs)) return null
        if (source.regionMatches(0, "data:image/", 0, "data:image/".length, ignoreCase = true)) {
            val bytes = decodeDataUrlBytes(source, policy, cancellation) ?: return null
            if (unavailable(cancellation, clock, deadlineMs)) return null
            val decoded = decodeBitmapLease(bytes, policy, priority)
            if (decoded == null) {
                Log.w(LOG_TAG, "decodeSource: failed to decode data URL bytes (${sourceSummary(source)})")
            } else {
                Log.d(
                    LOG_TAG,
                    "decodeSource: decoded data URL ${sourceSummary(source)} -> ${decoded.bitmap.width}x${decoded.bitmap.height}"
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
            if (unavailable(cancellation, clock, deadlineMs)) return null
            connection.connectTimeout = boundedTransportTimeout(
                policy.connectTimeoutMs,
                clock,
                deadlineMs
            )
            connection.readTimeout = boundedTransportTimeout(policy.readTimeoutMs, clock, deadlineMs)
            connection.instanceFollowRedirects = true
            val status = connection.responseCode
            if (unavailable(cancellation, clock, deadlineMs) || status !in 200..299 ||
                connection.contentLengthLong > policy.maxSourceBytes
            ) {
                null
            } else {
                connection.inputStream.use { input ->
                    cancellation?.bind(input)
                    readBounded(input, policy, clock, cancellation, deadlineMs)
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
        if (unavailable(cancellation, clock, deadlineMs)) return null
        val decoded = decodeBitmapLease(remoteBytes, policy, priority)
        if (decoded == null) {
            Log.w(LOG_TAG, "decodeSource: failed to decode remote bytes (${sourceSummary(source)})")
        }
        return decoded
    }

    fun decodeDataUrlBytes(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT,
        cancellation: Cancellation? = null
    ): ByteArray? {
        val admission = preflightDataUrl(source, policy, cancellation) ?: return null
        val commaIndex = admission.commaIndex
        val maximumEncodedCharacters = admission.maximumEncodedCharacters
        val payload = StringBuilder(admission.sanitizedPayloadLength)
        for (payloadIndex in commaIndex + 1 until source.length) {
            if (cancellation?.isCancelled() == true) return null
            val character = source[payloadIndex]
            if (!character.isWhitespace()) {
                payload.append(character)
                if (payload.length.toLong() > maximumEncodedCharacters) return null
            }
        }

        val decodeFlags = intArrayOf(
            Base64.DEFAULT,
            Base64.NO_WRAP,
            Base64.URL_SAFE or Base64.NO_WRAP,
            Base64.URL_SAFE
        )
        for (flags in decodeFlags) {
            val bytes = runCatching { Base64.decode(payload.toString(), flags) }.getOrNull()
            if (bytes != null && bytes.size <= policy.maxSourceBytes) {
                return bytes
            }
        }
        Log.w(LOG_TAG, "decodeDataUrlBytes: unsupported base64 payload (${sourceSummary(source)})")
        return null
    }

    internal fun preflightDataUrl(
        source: String,
        policy: ImageLoadingPolicy,
        cancellation: Cancellation? = null
    ): DataUrlAdmission? {
        if (cancellation?.isCancelled() == true ||
            !source.regionMatches(0, "data:image/", 0, "data:image/".length, ignoreCase = true)
        ) return null
        var commaIndex = -1
        var metadataUtf8Bytes = 0
        var index = 0
        while (index < source.length) {
            if (cancellation?.isCancelled() == true) return null
            val character = source[index]
            if (character == ',') {
                commaIndex = index
                break
            }
            metadataUtf8Bytes += utf8BytesAt(source, index)
            if (metadataUtf8Bytes > MAX_DATA_URL_METADATA_BYTES) return null
            index += if (Character.isHighSurrogate(character) &&
                index + 1 < source.length && Character.isLowSurrogate(source[index + 1])
            ) 2 else 1
        }
        if (commaIndex <= 0) return null
        if (!hasBase64MetadataToken(source, commaIndex)) return null
        val maximumEncodedCharacters = ((policy.maxSourceBytes.toLong() + 2L) / 3L) * 4L
        val maximumRawPayload = maximumEncodedCharacters + DATA_URL_WHITESPACE_ALLOWANCE_BYTES
        val rawPayloadLength = source.length.toLong() - commaIndex.toLong() - 1L
        if (rawPayloadLength > maximumRawPayload) return null
        var sanitizedPayloadLength = 0L
        for (payloadIndex in commaIndex + 1 until source.length) {
            if (cancellation?.isCancelled() == true) return null
            if (!source[payloadIndex].isWhitespace()) {
                sanitizedPayloadLength += 1L
                if (sanitizedPayloadLength > maximumEncodedCharacters) return null
            }
        }
        return DataUrlAdmission(
            commaIndex,
            maximumEncodedCharacters,
            sanitizedPayloadLength.toInt()
        )
    }

    private fun hasBase64MetadataToken(source: String, commaIndex: Int): Boolean {
        var index = "data:image/".length
        while (index < commaIndex) {
            if (source[index] == ';') {
                val tokenStart = index + 1
                val tokenEnd = tokenStart + "base64".length
                if (tokenEnd <= commaIndex &&
                    source.regionMatches(
                        tokenStart,
                        "base64",
                        0,
                        "base64".length,
                        ignoreCase = true
                    ) && (tokenEnd == commaIndex || source[tokenEnd] == ';')
                ) return true
            }
            index += 1
        }
        return false
    }

    fun readBounded(
        input: InputStream,
        maxBytes: Int,
        cancellation: Cancellation? = null
    ): ByteArray? = readBoundedInternal(
        input,
        maxBytes,
        cancellation,
        systemMonotonicClock,
        Long.MAX_VALUE
    )

    fun readBounded(
        input: InputStream,
        policy: ImageLoadingPolicy,
        clock: MonotonicClock,
        cancellation: Cancellation? = null,
        deadlineMs: Long = deadlineAfter(clock.elapsedRealtime(), policy.requestTimeoutMs)
    ): ByteArray? = readBoundedInternal(
        input,
        policy.maxSourceBytes,
        cancellation,
        clock,
        deadlineMs
    )

    private fun readBoundedInternal(
        input: InputStream,
        maxBytes: Int,
        cancellation: Cancellation?,
        clock: MonotonicClock,
        deadlineMs: Long
    ): ByteArray? {
        val output = ByteArrayOutputStream(minOf(maxBytes, 16 * 1024))
        val buffer = ByteArray(8 * 1024)
        var total = 0
        while (true) {
            if (unavailable(cancellation, clock, deadlineMs)) return null
            val read = input.read(buffer)
            if (unavailable(cancellation, clock, deadlineMs)) return null
            if (read < 0) break
            total += read
            if (total > maxBytes) return null
            output.write(buffer, 0, read)
        }
        return output.toByteArray()
    }

    private fun boundedTransportTimeout(
        configuredMs: Int,
        clock: MonotonicClock,
        deadlineMs: Long
    ): Int {
        val remaining = (deadlineMs - clock.elapsedRealtime()).coerceAtLeast(1L)
        return minOf(configuredMs.toLong(), remaining, Int.MAX_VALUE.toLong()).toInt()
    }

    private fun unavailable(
        cancellation: Cancellation?,
        clock: MonotonicClock,
        deadlineMs: Long
    ): Boolean = cancellation?.isCancelled() == true || clock.elapsedRealtime() >= deadlineMs

    private fun utf8BytesAt(value: String, index: Int): Int {
        val character = value[index]
        return when {
            character.code <= 0x7f -> 1
            character.code <= 0x7ff -> 2
            Character.isHighSurrogate(character) && index + 1 < value.length &&
                Character.isLowSurrogate(value[index + 1]) -> 4
            else -> 3
        }
    }

    fun calculateInSampleSize(
        width: Int,
        height: Int,
        maxWidth: Int = ImageLoadingPolicy.DEFAULT.maxDecodeDimensionPx,
        maxHeight: Int = ImageLoadingPolicy.DEFAULT.maxDecodeDimensionPx,
        maxDecodedBytes: Long = ImageLoadingPolicy.DEFAULT.maxDecodedBytes.toLong(),
    ): Int {
        if (width <= 0 || height <= 0) return 1

        if (maxWidth <= 0 || maxHeight <= 0) return 1
        var sampleSize = 1L
        var sampledWidth = width.toLong()
        var sampledHeight = height.toLong()
        while (
            sampledWidth > maxWidth.toLong() ||
            sampledHeight > maxHeight.toLong() ||
            exceedsArgbBytes(sampledWidth, sampledHeight, maxDecodedBytes)
        ) {
            if (sampleSize >= (1L shl 30)) return (1 shl 30)
            sampleSize = sampleSize shl 1
            sampledWidth = (width.toLong() + sampleSize - 1L) / sampleSize
            sampledHeight = (height.toLong() + sampleSize - 1L) / sampleSize
        }
        return sampleSize.toInt().coerceAtLeast(1)
    }

    private fun exceedsArgbBytes(width: Long, height: Long, maximumBytes: Long): Boolean =
        width <= 0L || height <= 0L || maximumBytes <= 0L ||
            width > Long.MAX_VALUE / height ||
            width * height > maximumBytes / 4L

    private fun decodeBitmapLease(
        bytes: ByteArray,
        policy: ImageLoadingPolicy,
        priority: DecodedBitmapPriority,
    ): DecodedBitmapLease? {
        bitmapDecoderOverride?.let {
            val bitmap = try {
                it(bytes, policy)
            } catch (_: OutOfMemoryError) {
                null
            } ?: return null
            val bytesRetained = decodedAllocationBytes(bitmap)
            val reservation = DecodedBitmapBudget.shared().reserve(
                bytesRetained,
                priority,
            ) ?: return constrainUnbudgetedBitmap(bitmap, bytesRetained, policy, priority)
            val lease = reservation.commit(bitmap, bytesRetained)
                ?: return constrainUnbudgetedBitmap(bitmap, bytesRetained, policy, priority)
            return constrainDecodedLease(lease, policy, priority)
        }
        val bounds = BitmapFactory.Options().apply {
            inJustDecodeBounds = true
        }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) {
            return decodeSvgLease(bytes, policy, priority)
        }

        val sampleSize = calculateInSampleSize(
                bounds.outWidth,
                bounds.outHeight,
                policy.maxDecodeDimensionPx,
                policy.maxDecodeDimensionPx,
                policy.maxDecodedBytes.toLong(),
            )
        val estimatedBytes = estimatedArgbBytes(bounds.outWidth, bounds.outHeight, sampleSize)
        if (estimatedBytes > policy.maxDecodedBytes.toLong()) return null
        val reservation = DecodedBitmapBudget.shared().reserve(
            estimatedBytes,
            priority,
        ) ?: return null
        val bitmap = try {
            BitmapFactory.decodeByteArray(
                bytes,
                0,
                bytes.size,
                BitmapFactory.Options().apply { inSampleSize = sampleSize },
            )
        } catch (_: OutOfMemoryError) {
            null
        } ?: run {
            reservation.close()
            return null
        }
        val decodedBytes = decodedAllocationBytes(bitmap)
        val lease = reservation.commit(bitmap, decodedBytes)
            ?: return constrainUnbudgetedBitmap(bitmap, decodedBytes, policy, priority)
        return constrainDecodedLease(lease, policy, priority)
    }

    private fun decodeSvgLease(
        bytes: ByteArray,
        policy: ImageLoadingPolicy,
        priority: DecodedBitmapPriority,
    ): DecodedBitmapLease? {
        val svg = try {
            if (!isSelfContainedSvg(bytes)) return null
            SVG.getFromInputStream(bytes.inputStream())
        } catch (_: Exception) {
            return null
        } catch (_: OutOfMemoryError) {
            return null
        }
        val viewBox = svg.documentViewBox
        val aspectRatio = svg.documentAspectRatio.toDouble()
        var width = svg.documentWidth.toDouble()
        var height = svg.documentHeight.toDouble()
        if (width <= 0 && height > 0 && aspectRatio > 0) width = height * aspectRatio
        if (height <= 0 && width > 0 && aspectRatio > 0) height = width / aspectRatio
        if (width <= 0) width = viewBox?.width()?.toDouble() ?: 300.0
        if (height <= 0) height = viewBox?.height()?.toDouble() ?: 150.0
        if (!width.isFinite() || !height.isFinite() || width <= 0 || height <= 0 ||
            policy.maxDecodedBytes < 4 || policy.maxDecodeDimensionPx <= 0
        ) return null
        val scale = minOf(
            1.0,
            policy.maxDecodeDimensionPx / width,
            policy.maxDecodeDimensionPx / height,
            kotlin.math.sqrt(policy.maxDecodedBytes.toDouble() / 4.0 / width / height),
        )
        val targetWidth = kotlin.math.floor(width * scale).toInt().coerceAtLeast(1)
        val targetHeight = kotlin.math.floor(height * scale).toInt().coerceAtLeast(1)
        val estimatedBytes = estimatedArgbBytes(targetWidth, targetHeight, 1)
        if (estimatedBytes > policy.maxDecodedBytes) return null
        val reservation = DecodedBitmapBudget.shared().reserve(estimatedBytes, priority) ?: return null
        var bitmap: Bitmap? = null
        try {
            bitmap = Bitmap.createBitmap(targetWidth, targetHeight, Bitmap.Config.ARGB_8888)
            if (viewBox == null) svg.setDocumentViewBox(0f, 0f, width.toFloat(), height.toFloat())
            svg.setDocumentWidth("100%")
            svg.setDocumentHeight("100%")
            svg.renderToCanvas(Canvas(bitmap))
            val decodedBytes = decodedAllocationBytes(bitmap)
            if (decodedBytes > policy.maxDecodedBytes) return null
            val lease = reservation.commit(bitmap, decodedBytes) ?: return null
            bitmap = null
            return lease
        } catch (_: Exception) {
            return null
        } catch (_: OutOfMemoryError) {
            return null
        } finally {
            bitmap?.recycle()
            reservation.close()
        }
    }

    private const val MAX_SVG_ELEMENTS = 8192
    private const val MAX_SVG_DEPTH = 128
    private val svgUrlReference = Regex("url\\s*\\(([^)]*)\\)", RegexOption.IGNORE_CASE)

    private val svgUrlStart = Regex("url\\s*\\(", RegexOption.IGNORE_CASE)
    private val svgLiteralAttributes = setOf("id", "href", "title", "class", "aria-label")

    private class SvgNode {
        val children = mutableListOf<Int>()
        val references = mutableListOf<String>()
    }

    private fun isSelfContainedSvg(bytes: ByteArray): Boolean {
        val parser = Xml.newPullParser()
        parser.setFeature(XmlPullParser.FEATURE_PROCESS_NAMESPACES, true)
        parser.setInput(bytes.inputStream(), null)
        val nodes = mutableListOf<SvgNode>()
        val ids = mutableMapOf<String, Int>()
        val stack = ArrayDeque<Int>()
        var styleText: StringBuilder? = null
        while (true) {
            when (parser.nextToken()) {
                XmlPullParser.END_DOCUMENT -> break
                // Reject declarations before AndroidSVG can expand entities or load CSS.
                XmlPullParser.DOCDECL, XmlPullParser.PROCESSING_INSTRUCTION -> return false
                XmlPullParser.START_TAG -> {
                    if (nodes.isEmpty() && (parser.name != "svg" ||
                            parser.namespace != "http://www.w3.org/2000/svg")
                    ) return false
                    if (nodes.size >= MAX_SVG_ELEMENTS || stack.size >= MAX_SVG_DEPTH ||
                        parser.name in setOf("script", "foreignObject", "image")
                    ) return false
                    val node = SvgNode()
                    val index = nodes.size
                    stack.lastOrNull()?.let { nodes[it].children.add(index) }
                    nodes.add(node)
                    stack.addLast(index)
                    if (parser.name == "style") styleText = StringBuilder()
                    for (attribute in 0 until parser.attributeCount) {
                        val name = parser.getAttributeName(attribute)
                        val value = parser.getAttributeValue(attribute)
                        val literal = name in svgLiteralAttributes || name.startsWith("data-")
                        if (name.startsWith("on", ignoreCase = true) ||
                            (name == "href" && !value.trim().startsWith("#")) ||
                            (!literal && !hasOnlyLocalSvgReferences(value))
                        ) return false
                        if (name == "id" && ids.put(value, index) != null) return false
                        if (name == "href") node.references.add(value.trim().removePrefix("#"))
                        if (!literal) {
                            svgUrlReference.findAll(value).forEach {
                                node.references.add(it.groupValues[1].trim().trim('\'', '"').removePrefix("#"))
                            }
                        }
                    }
                }
                XmlPullParser.TEXT, XmlPullParser.CDSECT, XmlPullParser.ENTITY_REF ->
                    styleText?.append(parser.text.orEmpty())
                XmlPullParser.END_TAG -> {
                    if (parser.name == "style") {
                        val css = styleText.toString()
                        // Stylesheet selectors obscure reference cycles; inline styles remain supported.
                        if (!hasOnlyLocalSvgReferences(css) || svgUrlReference.containsMatchIn(css)) return false
                        styleText = null
                    }
                    stack.removeLast()
                }
            }
        }
        if (nodes.isEmpty()) return false
        val costs = IntArray(nodes.size)
        val heights = IntArray(nodes.size)
        val visiting = BooleanArray(nodes.size)
        fun expandedCost(index: Int, depth: Int): Int {
            if (depth > MAX_SVG_DEPTH || visiting[index]) return MAX_SVG_ELEMENTS + 1
            if (costs[index] != 0) {
                return if (depth + heights[index] - 1 <= MAX_SVG_DEPTH) costs[index]
                else MAX_SVG_ELEMENTS + 1
            }
            visiting[index] = true
            var cost = 1
            var height = 1
            val node = nodes[index]
            for (child in node.children + node.references.mapNotNull { ids[it] }) {
                cost += expandedCost(child, depth + 1)
                height = maxOf(height, 1 + heights[child])
                if (cost > MAX_SVG_ELEMENTS) break
            }
            visiting[index] = false
            costs[index] = cost
            heights[index] = height
            return cost
        }
        return expandedCost(0, 1) <= MAX_SVG_ELEMENTS
    }

    private fun hasOnlyLocalSvgReferences(value: String): Boolean {
        if (value.contains('\\') || value.contains('@')) return false
        if (!svgUrlReference.findAll(value).all {
                it.groupValues[1].trim().trim('\'', '"').startsWith("#")
            }
        ) return false
        return !svgUrlStart.containsMatchIn(svgUrlReference.replace(value, ""))
    }

    private data class ConstrainedBitmapTarget(
        val width: Int,
        val height: Int,
        val estimatedBytes: Long,
    )

    private fun constrainedBitmapTarget(
        bitmap: Bitmap,
        decodedBytes: Long,
        policy: ImageLoadingPolicy,
    ): ConstrainedBitmapTarget? {
        val maximumDimension = policy.maxDecodeDimensionPx
        val dimensionScale = minOf(
            1.0,
            maximumDimension.toDouble() / bitmap.width.toDouble(),
            maximumDimension.toDouble() / bitmap.height.toDouble()
        )
        val byteScale = if (decodedBytes > policy.maxDecodedBytes.toLong()) {
            kotlin.math.sqrt(policy.maxDecodedBytes.toDouble() / decodedBytes.toDouble())
        } else {
            1.0
        }
        val scale = minOf(dimensionScale, byteScale)
        if (scale >= 1.0) return null
        val targetWidth = kotlin.math.floor(bitmap.width.toDouble() * scale)
            .toInt()
            .coerceIn(1, maximumDimension)
        val targetHeight = kotlin.math.floor(bitmap.height.toDouble() * scale)
            .toInt()
            .coerceIn(1, maximumDimension)
        return ConstrainedBitmapTarget(
            targetWidth,
            targetHeight,
            estimatedArgbBytes(targetWidth, targetHeight, 1),
        )
    }

    private fun constrainUnbudgetedBitmap(
        bitmap: Bitmap,
        decodedBytes: Long,
        policy: ImageLoadingPolicy,
        priority: DecodedBitmapPriority,
    ): DecodedBitmapLease? {
        val target = constrainedBitmapTarget(bitmap, decodedBytes, policy) ?: return null
        val reservation = DecodedBitmapBudget.shared().reserve(
            target.estimatedBytes,
            priority,
        ) ?: return null
        val constrained = try {
            Bitmap.createScaledBitmap(bitmap, target.width, target.height, true)
        } catch (_: RuntimeException) {
            null
        } catch (_: OutOfMemoryError) {
            null
        } ?: run {
            reservation.close()
            return null
        }
        if (constrained === bitmap) {
            reservation.close()
            return null
        }
        val lease = reservation.commit(constrained, decodedAllocationBytes(constrained)) ?: return null
        if (lease.byteCount > policy.maxDecodedBytes.toLong()) {
            lease.close()
            return null
        }
        return lease
    }

    private fun constrainDecodedLease(
        lease: DecodedBitmapLease,
        policy: ImageLoadingPolicy,
        priority: DecodedBitmapPriority = DecodedBitmapPriority.VISIBLE,
    ): DecodedBitmapLease? {
        val bitmap = lease.bitmap
        val decodedBytes = lease.byteCount
        val target = constrainedBitmapTarget(bitmap, decodedBytes, policy) ?: return lease
        val reservation = DecodedBitmapBudget.shared().reserve(
            target.estimatedBytes,
            priority,
        ) ?: run {
            lease.close()
            return constrainUnbudgetedBitmap(bitmap, decodedBytes, policy, priority)
        }
        val constrained = try {
            Bitmap.createScaledBitmap(bitmap, target.width, target.height, true)
        } catch (_: RuntimeException) {
            null
        } catch (_: OutOfMemoryError) {
            null
        } ?: run {
            reservation.close()
            lease.close()
            return null
        }
        if (constrained === bitmap) {
            reservation.close()
            return if (lease.byteCount <= policy.maxDecodedBytes.toLong()) {
                lease
            } else {
                lease.close()
                null
            }
        }
        val constrainedLease = reservation.commit(constrained, decodedAllocationBytes(constrained))
        lease.close()
        constrainedLease ?: return null
        if (constrainedLease.byteCount > policy.maxDecodedBytes.toLong()) {
            constrainedLease.close()
            return null
        }
        return constrainedLease
    }

    private fun estimatedArgbBytes(width: Int, height: Int, sampleSize: Int): Long {
        if (width <= 0 || height <= 0 || sampleSize <= 0) return Long.MAX_VALUE
        val sampledWidth = (width.toLong() + sampleSize - 1L) / sampleSize
        val sampledHeight = (height.toLong() + sampleSize - 1L) / sampleSize
        if (sampledWidth > Long.MAX_VALUE / sampledHeight) return Long.MAX_VALUE
        val pixels = sampledWidth * sampledHeight
        return if (pixels > Long.MAX_VALUE / 4L) Long.MAX_VALUE else pixels * 4L
    }

    private fun decodedAllocationBytes(bitmap: Bitmap): Long {
        val allocation = runCatching { bitmap.allocationByteCount.toLong() }.getOrNull()
        if (allocation != null && allocation > 0L) return allocation
        return estimatedArgbBytes(bitmap.width, bitmap.height, 1)
    }

    internal fun resetForTesting() {
        connectionFactoryOverride = null
        bitmapDecoderOverride = null
    }

    private fun sourceSummary(source: String): String {
        if (!source.regionMatches(0, "data:image/", 0, "data:image/".length, ignoreCase = true)) {
            return "urlLength=${source.length}"
        }
        var commaIndex = -1
        for (index in 0..minOf(source.lastIndex, MAX_DATA_URL_METADATA_BYTES)) {
            if (source[index] == ',') {
                commaIndex = index
                break
            }
        }
        if (commaIndex <= 0) {
            return "dataUrlLength=${source.length}"
        }
        val metadata = source.substring(0, commaIndex)
        val payloadLength = source.length - commaIndex - 1
        return "$metadata payloadLength=$payloadLength"
    }

    private const val MAX_DATA_URL_METADATA_BYTES = 256
    private const val DATA_URL_WHITESPACE_ALLOWANCE_BYTES = 4_096L
}

/**
 * Neutral editor/viewer facade over the shared bounded image loader.
 * Keep viewer code out of the editor render bridge's dependency graph.
 */
internal object NativeImagePipeline {
    fun prepare(source: String, policy: ImageLoadingPolicy): RenderImageLoader.PreparedSource? =
        RenderImageLoader.prepare(source, policy)

    fun load(
        source: RenderImageLoader.PreparedSource,
        ownerId: Long,
        priority: DecodedBitmapPriority,
        callback: (DecodedBitmapLease?) -> Unit,
    ): RenderImageLoader.LoadHandle = RenderImageLoader.loadLease(
        source,
        ownerId,
        priority,
        callback,
    )
}

internal object RenderImageLoader {
    // Public policy values are per-policy upper bounds. These process-wide
    // ceilings keep aggregate work bounded when many differently configured
    // editor/viewer instances coexist; contention may yield lower concurrency.
    private const val GLOBAL_WORKERS = 4
    private const val GLOBAL_QUEUE_CAPACITY = 256
    private const val GLOBAL_ADMISSION_LIMIT = GLOBAL_WORKERS + GLOBAL_QUEUE_CAPACITY
    private const val REJECTION_NOTIFICATION_LIMIT = 64
    private const val CACHE_ENTRY_OVERHEAD_BYTES = 64

    internal class CacheKey(val digest: ByteArray) {
        override fun equals(other: Any?): Boolean =
            other is CacheKey && digest.contentEquals(other.digest)

        override fun hashCode(): Int = digest.contentHashCode()
    }

    internal data class RequestKey(val digest: CacheKey, val policy: ImageLoadingPolicy)
    internal class PreparedSource internal constructor(
        internal val source: String,
        internal val policy: ImageLoadingPolicy
    )
    internal class LoadHandle(private val cancelAction: () -> Unit) {
        private val finished = AtomicBoolean(false)
        private val finishListeners = mutableListOf<() -> Unit>()

        fun cancel() {
            try {
                runCatching { cancelAction() }
            } finally {
                finish()
            }
        }

        fun onFinished(listener: () -> Unit) {
            val invokeNow = synchronized(finishListeners) {
                if (finished.get()) true else {
                    finishListeners += listener
                    false
                }
            }
            if (invokeNow) listener()
        }

        internal fun finish() {
            if (!finished.compareAndSet(false, true)) return
            val listeners = synchronized(finishListeners) {
                finishListeners.toList().also { finishListeners.clear() }
            }
            listeners.forEach { runCatching { it() } }
        }
    }
    private data class Callback(
        val id: Long,
        val cancelled: AtomicBoolean,
        val admissionReleased: AtomicBoolean,
        val handle: LoadHandle,
        val ownerId: Long?,
        val ownerLimitBytes: Long,
        val priority: DecodedBitmapPriority,
        val deliver: (DecodedBitmapLease?) -> Unit,
    )
    private class PendingRequest(
        val key: RequestKey,
        val source: String,
        val callbacks: MutableList<Callback>,
        val cancellation: RenderImageDecoder.Cancellation,
        val startedAtMs: Long
    ) {
        @Volatile var priority: DecodedBitmapPriority = callbacks.first().priority
        val deadlineMs: Long = deadlineAfter(startedAtMs, key.policy.requestTimeoutMs)
        var future: Future<*>? = null
        var timeoutFuture: ScheduledFuture<*>? = null
        var submitted = false
        var dispatching = false
        val started = AtomicBoolean(false)
        val terminal = AtomicBoolean(false)
        val workerSlotReleased = AtomicBoolean(false)
    }
    private class PrioritizedTask(
        private val request: PendingRequest,
        private val sequence: Long,
        action: () -> Unit,
    ) : FutureTask<Unit>(Callable { action(); Unit }), Comparable<PrioritizedTask> {
        override fun compareTo(other: PrioritizedTask): Int {
            val priority = request.priority.compareTo(other.request.priority)
            return if (priority != 0) priority else sequence.compareTo(other.sequence)
        }
    }
    internal class BoundedPriorityQueue<E>(capacity: Int) : PriorityBlockingQueue<E>() {
        private val permits = Semaphore(capacity)

        override fun offer(element: E): Boolean {
            if (!permits.tryAcquire()) return false
            return try {
                super.offer(element).also { added -> if (!added) permits.release() }
            } catch (throwable: Throwable) {
                permits.release()
                throw throwable
            }
        }

        override fun poll(): E? = super.poll()?.also { permits.release() }

        override fun poll(timeout: Long, unit: TimeUnit): E? =
            super.poll(timeout, unit)?.also { permits.release() }

        override fun take(): E = super.take().also { permits.release() }

        override fun remove(element: E?): Boolean =
            super.remove(element).also { removed -> if (removed) permits.release() }

        override fun clear() {
            while (poll() != null) Unit
        }

        override fun drainTo(target: MutableCollection<in E>): Int =
            super.drainTo(target).also(permits::release)

        override fun drainTo(target: MutableCollection<in E>, maxElements: Int): Int =
            super.drainTo(target, maxElements).also(permits::release)
    }
    private data class PolicyState(
        var submittedCount: Int = 0,
        val pending: java.util.ArrayDeque<PendingRequest> = java.util.ArrayDeque()
    )

    private val cache = object : LruCache<CacheKey, DecodedBitmapLease>(32 * 1024 * 1024) {
        override fun sizeOf(key: CacheKey, value: DecodedBitmapLease): Int =
            saturatingAdd(value.byteCount, key.digest.size.toLong() + CACHE_ENTRY_OVERHEAD_BYTES)
                .coerceAtMost(Int.MAX_VALUE.toLong())
                .toInt()

        override fun entryRemoved(
            evicted: Boolean,
            key: CacheKey,
            oldValue: DecodedBitmapLease,
            newValue: DecodedBitmapLease?,
        ) {
            if (oldValue !== newValue) oldValue.close()
        }
    }

    /** Counts the allocation once even when cache and mounted leases share it. */
    private fun decodedAllocationBytes(bitmap: Bitmap): Long {
        val allocation = runCatching { bitmap.allocationByteCount.toLong() }.getOrNull()
        if (allocation != null && allocation >= 0) return allocation
        val pixels = bitmap.width.coerceAtLeast(0).toLong()
        val rows = bitmap.height.coerceAtLeast(0).toLong()
        return if (pixels == 0L || rows == 0L) 0L
        else if (pixels > Long.MAX_VALUE / rows || pixels * rows > Long.MAX_VALUE / 4L) Long.MAX_VALUE
        else pixels * rows * 4L
    }

    private fun saturatingAdd(left: Long, right: Long): Long =
        if (right > 0 && left > Long.MAX_VALUE - right) Long.MAX_VALUE else left + right
    private val lock = Any()
    private val inFlight = mutableMapOf<RequestKey, PendingRequest>()
    private val policyStates = mutableMapOf<ImageLoadingPolicy, PolicyState>()
    private val readyToSubmit = java.util.ArrayDeque<PendingRequest>()
    private val rejectionLock = Any()
    private val rejectionNotifications = java.util.ArrayDeque<Callback>()
    private var rejectionDrainPosted = false
    private var admissionCount = 0
    private val nextCallbackId = AtomicLong()
    private val submissionRejectionCount = AtomicLong()
    private val submissionSequence = AtomicLong()
    private val digestConstructionCount = AtomicLong()
    private val mainHandler by lazy { Handler(Looper.getMainLooper()) }
    private var globalExecutor = createGlobalExecutor()
    private val timeoutScheduler = Executors.newSingleThreadScheduledExecutor { runnable ->
        Thread(runnable, "native-editor-image-deadline").apply { isDaemon = true }
    }

    init {
        DecodedBitmapBudget.shared().setPressureHandler {
            synchronized(cache) { cache.evictAll() }
        }
    }

    @Volatile
    internal var decodeSourceOverride: ((String, ImageLoadingPolicy) -> Bitmap?)? = null

    @Volatile
    internal var beforeWorkerReturnOverride: ((String) -> Unit)? = null

    @Volatile
    internal var deadlineExecutionGateOverride: (() -> Unit)? = null

    @Volatile
    internal var beforeCacheCommitOverride: (() -> Unit)? = null

    @Volatile
    internal var beforeTerminalClaimOverride: (() -> Unit)? = null

    @Volatile
    internal var decodedDeliveryPostedOverride: (() -> Unit)? = null

    @Volatile
    internal var monotonicClockOverride: MonotonicClock? = null

    @Volatile
    internal var beforeDigestOverride: (() -> Unit)? = null

    internal fun isCachedForTesting(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
    ): Boolean = prepare(source, policy)?.let {
        synchronized(cache) { cache.get(cacheKey(it.source, it.policy)) != null }
    } ?: false

    internal fun prepare(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
    ): PreparedSource? {
        if (source.regionMatches(0, "data:image/", 0, "data:image/".length, ignoreCase = true) &&
            RenderImageDecoder.preflightDataUrl(source, policy) == null
        ) return null
        return PreparedSource(source, policy)
    }

    internal fun cacheKeyByteCountForTesting(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
    ): Int = cacheKey(source, policy).digest.size

    internal fun cacheEntryCountForTesting(): Int = synchronized(cache) { cache.snapshot().size }

    internal fun cacheRetainedCostForTesting(): Int = synchronized(cache) { cache.size() }

    internal fun digestConstructionCountForTesting(): Long = digestConstructionCount.get()

    internal fun resetForTesting() {
        synchronized(cache) {
            cache.evictAll()
        }
        val pending: List<PendingRequest>
        val executor: ThreadPoolExecutor
        synchronized(lock) {
            pending = inFlight.values.toList()
            inFlight.clear()
            policyStates.clear()
            readyToSubmit.clear()
            admissionCount = 0
            executor = globalExecutor
            globalExecutor = createGlobalExecutor()
        }
        pending.forEach { request ->
            runCatching { request.cancellation.cancel() }
            runCatching { request.future?.cancel(true) }
            runCatching { request.timeoutFuture?.cancel(false) }
        }
        executor.shutdownNow()
        val rejected = synchronized(rejectionLock) {
            rejectionNotifications.toList().also {
                rejectionNotifications.clear()
                rejectionDrainPosted = false
            }
        }
        rejected.forEach { it.handle.finish() }
        submissionRejectionCount.set(0)
        digestConstructionCount.set(0)
        decodeSourceOverride = null
        beforeWorkerReturnOverride = null
        deadlineExecutionGateOverride = null
        beforeCacheCommitOverride = null
        beforeTerminalClaimOverride = null
        decodedDeliveryPostedOverride = null
        monotonicClockOverride = null
        beforeDigestOverride = null
    }

    internal fun executionResourceCountForTesting(): Int = 1

    internal fun globalQueuedTaskCountForTesting(): Int = globalExecutor.queue.size

    internal fun globalQueueLimitForTesting(): Int = GLOBAL_QUEUE_CAPACITY

    internal fun globalActiveWorkerCountForTesting(): Int = globalExecutor.activeCount

    internal fun globalWorkerLimitForTesting(): Int = GLOBAL_WORKERS

    internal fun globalAdmissionCountForTesting(): Int = synchronized(lock) { admissionCount }

    internal fun globalAdmissionLimitForTesting(): Int = GLOBAL_ADMISSION_LIMIT

    internal fun rejectionNotificationCountForTesting(): Int =
        synchronized(rejectionLock) { rejectionNotifications.size }

    internal fun rejectionNotificationLimitForTesting(): Int = REJECTION_NOTIFICATION_LIMIT

    internal fun submissionRejectionCountForTesting(): Long = submissionRejectionCount.get()

    internal fun loadLease(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT,
        ownerId: Long,
        priority: DecodedBitmapPriority,
        onLoaded: (DecodedBitmapLease?) -> Unit,
    ): LoadHandle = loadInternal(
        source,
        policy,
        null,
        ownerId,
        priority,
        onLoaded,
    )

    internal fun loadLease(
        prepared: PreparedSource,
        ownerId: Long,
        priority: DecodedBitmapPriority,
        onLoaded: (DecodedBitmapLease?) -> Unit,
    ): LoadHandle = loadInternal(
        prepared.source,
        prepared.policy,
        prepared,
        ownerId,
        priority,
        onLoaded,
    )

    private fun loadInternal(
        source: String,
        policy: ImageLoadingPolicy,
        prepared: PreparedSource?,
        ownerId: Long?,
        priority: DecodedBitmapPriority,
        onLoaded: (DecodedBitmapLease?) -> Unit,
    ): LoadHandle {
        val cancelled = AtomicBoolean(false)
        var requestKey: RequestKey? = null
        lateinit var callback: Callback
        val handle = LoadHandle {
            cancelled.set(true)
            requestKey?.let { cancelCallback(it, callback) }
        }
        callback = Callback(
            nextCallbackId.incrementAndGet(),
            cancelled,
            AtomicBoolean(false),
            handle,
            ownerId,
            policy.maxDecodedBytes.toLong(),
            priority,
            onLoaded,
        )
        val admitted = synchronized(lock) {
            if (admissionCount >= GLOBAL_ADMISSION_LIMIT) {
                false
            } else {
                admissionCount += 1
                true
            }
        }
        if (!admitted) {
            enqueueRejectionNotification(callback)
            return handle
        }
        handle.onFinished {
            releaseAdmission(callback)
        }
        val requestedAtMs = monotonicNowMs()
        val deadlineMs = deadlineAfter(requestedAtMs, policy.requestTimeoutMs)
        val resolvedSource = prepared ?: prepare(source, policy)
        if (resolvedSource == null) {
            releaseAdmission(callback)
            postCallbacks(listOf(callback), null)
            return handle
        }
        if (monotonicNowMs() >= deadlineMs) {
            releaseAdmission(callback)
            postCallbacks(listOf(callback), null)
            return handle
        }
        val resolvedRequestKey = RequestKey(
            cacheKey(resolvedSource.source, resolvedSource.policy),
            resolvedSource.policy
        )
        requestKey = resolvedRequestKey
        if (monotonicNowMs() >= deadlineMs) {
            releaseAdmission(callback)
            postCallbacks(listOf(callback), null)
            return handle
        }
        synchronized(cache) {
            cache.get(resolvedRequestKey.digest)?.let { cached -> leaseForCallback(cached, callback) }
        }?.let { lease ->
            scheduleCachedDelivery(callback, lease, deadlineMs)
            return handle
        }
        var drain = false
        var reject = false
        var createdRequest: PendingRequest? = null
        synchronized(lock) {
            val existing = inFlight[resolvedRequestKey]
            if (existing != null) {
                existing.callbacks += callback
                if (
                    callback.priority == DecodedBitmapPriority.VISIBLE &&
                    existing.priority != DecodedBitmapPriority.VISIBLE
                ) {
                    existing.priority = DecodedBitmapPriority.VISIBLE
                    promoteRequestLocked(existing)
                }
            } else {
                val pending = PendingRequest(
                    key = resolvedRequestKey,
                    source = source,
                    callbacks = mutableListOf(callback),
                    cancellation = RenderImageDecoder.Cancellation(),
                    startedAtMs = requestedAtMs
                )
                val state = policyStates.getOrPut(policy) { PolicyState() }
                when {
                    state.submittedCount < policy.maxConcurrentRequests -> {
                        createdRequest = pending
                        inFlight[resolvedRequestKey] = pending
                        state.submittedCount += 1
                        pending.submitted = true
                        enqueueReadyLocked(pending)
                        drain = true
                    }
                    state.pending.size < policy.maxPendingRequests -> {
                        createdRequest = pending
                        inFlight[resolvedRequestKey] = pending
                        if (pending.priority == DecodedBitmapPriority.VISIBLE) {
                            state.pending.addFirst(pending)
                        } else {
                            state.pending.addLast(pending)
                        }
                    }
                    else -> reject = true
                }
            }
        }
        createdRequest?.let(::scheduleDeadline)
        if (reject) {
            enqueueRejectionNotification(callback)
        } else if (drain) {
            drainSubmissions()
        }
        return handle
    }

    private fun promoteRequestLocked(request: PendingRequest) {
        val state = policyStates[request.key.policy]
        if (!request.submitted) {
            if (state?.pending?.remove(request) == true) state.pending.addFirst(request)
            return
        }
        if (readyToSubmit.remove(request)) {
            readyToSubmit.addFirst(request)
            return
        }
        val future = request.future ?: return
        if (!request.started.get() && globalExecutor.remove(future as Runnable)) {
            try {
                globalExecutor.execute(future)
            } catch (_: RejectedExecutionException) {
                submissionRejectionCount.incrementAndGet()
                request.future = null
                enqueueReadyLocked(request)
                mainHandler.post { drainSubmissions() }
            }
        }
    }

    private fun enqueueReadyLocked(request: PendingRequest) {
        if (request.priority == DecodedBitmapPriority.VISIBLE) readyToSubmit.addFirst(request)
        else readyToSubmit.addLast(request)
    }

    private fun drainSubmissions() {
        while (true) {
            val request = synchronized(lock) {
                readyToSubmit.pollFirst()?.also { it.dispatching = true }
            } ?: return
            if (!submitRequest(request)) {
                synchronized(lock) {
                    request.dispatching = false
                    if (inFlight[request.key] === request) {
                        readyToSubmit.addFirst(request)
                    } else {
                        releaseRequestSlotLocked(request)
                    }
                }
                return
            }
        }
    }

    private fun submitRequest(request: PendingRequest): Boolean {
        try {
            val future = PrioritizedTask(
                request,
                submissionSequence.incrementAndGet(),
            ) {
                request.started.set(true)
                var bitmap: DecodedBitmapLease? = null
                try {
                    bitmap = decode(request)
                } catch (_: Exception) {
                    bitmap = null
                } catch (_: OutOfMemoryError) {
                    bitmap = null
                } finally {
                    completeDecodedRequest(request, bitmap)
                    beforeWorkerReturnOverride?.invoke(request.source)
                }
            }
            globalExecutor.execute(future)
            var orphaned = false
            synchronized(lock) {
                request.dispatching = false
                request.future = future
                orphaned = inFlight[request.key] !== request
            }
            if (orphaned) cancelOrphanedSubmission(request, future)
            return true
        } catch (_: RejectedExecutionException) {
            submissionRejectionCount.incrementAndGet()
            return false
        }
    }

    private fun cancelOrphanedSubmission(request: PendingRequest, future: Future<*>) {
        runCatching { request.cancellation.cancel() }
        val removed = !request.started.get() &&
            runCatching { globalExecutor.remove(future as Runnable) }.getOrDefault(false)
        if (removed) {
            runCatching { future.cancel(false) }
            if (!request.terminal.get()) {
                synchronized(lock) { releaseRequestSlotLocked(request) }
                mainHandler.post { drainSubmissions() }
            }
        } else if (request.started.get()) {
            runCatching { future.cancel(true) }
        }
    }

    private fun completeDecodedRequest(request: PendingRequest, bitmap: DecodedBitmapLease?) {
        synchronized(lock) {
            releaseRequestSlotLocked(request)
        }
        mainHandler.post { drainSubmissions() }
        if (request.terminal.get()) {
            bitmap?.close()
            return
        }
        if (bitmap == null || request.cancellation.isCancelled() ||
            monotonicNowMs() >= request.deadlineMs
        ) {
            claimRequestOutcome(request, null, deliverInline = false)
            return
        }
        decodedDeliveryPostedOverride?.invoke()
        if (!mainHandler.post {
                if (request.terminal.get()) return@post
                beforeCacheCommitOverride?.invoke()
                val deliverable = bitmap.takeIf {
                    !request.cancellation.isCancelled() &&
                        monotonicNowMs() < request.deadlineMs
                }
                beforeTerminalClaimOverride?.invoke()
                claimRequestOutcome(request, deliverable, deliverInline = true)
            }
        ) {
            claimRequestOutcome(request, null, deliverInline = false)
        }
    }

    private fun claimRequestOutcome(
        request: PendingRequest,
        candidateBitmap: DecodedBitmapLease?,
        deliverInline: Boolean
    ): Boolean {
        if (!request.terminal.compareAndSet(false, true)) {
            candidateBitmap?.close()
            return false
        }
        request.timeoutFuture?.cancel(false)
        val callbacks: List<Callback>
        synchronized(lock) {
            if (inFlight[request.key] === request) inFlight.remove(request.key)
            releaseRequestSlotLocked(request)
            callbacks = request.callbacks.toList()
        }
        val resolvedBitmap = candidateBitmap?.takeIf {
            !request.cancellation.isCancelled() &&
                monotonicNowMs() < request.deadlineMs &&
                callbacks.any { callback -> !callback.cancelled.get() }
        }
        if (resolvedBitmap == null) {
            candidateBitmap?.close()
            request.cancellation.cancel()
        }
        val deliveries = callbacks.map { callback ->
            callback to resolvedBitmap?.let { leaseForCallback(it, callback) }
        }
        if (resolvedBitmap != null) {
            synchronized(cache) { cache.put(request.key.digest, resolvedBitmap) }
        }
        callbacks.forEach(::releaseAdmission)
        if (deliverInline) {
            deliveries.forEach { (callback, lease) -> deliverCallback(callback, lease) }
            drainSubmissions()
        } else {
            postDeliveries(deliveries)
        }
        return true
    }

    private fun scheduleDeadline(request: PendingRequest) {
        val delayMs = (request.deadlineMs - monotonicNowMs()).coerceAtLeast(0L)
        val future = timeoutScheduler.schedule(
            {
                deadlineExecutionGateOverride?.invoke()
                expireRequest(request)
            },
            delayMs,
            TimeUnit.MILLISECONDS
        )
        request.timeoutFuture = future
        if (request.terminal.get()) future.cancel(false)
    }

    private fun scheduleCachedDelivery(
        callback: Callback,
        lease: DecodedBitmapLease,
        deadlineMs: Long,
    ) {
        val terminal = AtomicBoolean(false)
        val delayMs = (deadlineMs - monotonicNowMs()).coerceAtLeast(0L)
        val timeoutFuture = timeoutScheduler.schedule(
            {
                deadlineExecutionGateOverride?.invoke()
                if (terminal.compareAndSet(false, true)) {
                    lease.close()
                    releaseAdmission(callback)
                    postCallbacks(listOf(callback), null)
                }
            },
            delayMs,
            TimeUnit.MILLISECONDS
        )
        if (!mainHandler.post {
                if (!terminal.compareAndSet(false, true)) return@post
                timeoutFuture.cancel(false)
                releaseAdmission(callback)
                val result = lease.takeIf {
                    !callback.cancelled.get() && monotonicNowMs() < deadlineMs
                }
                if (result == null) lease.close()
                deliverCallback(callback, result)
            }
        ) {
            timeoutFuture.cancel(false)
            terminal.set(true)
            lease.close()
            callback.handle.finish()
        }
    }

    private fun expireRequest(request: PendingRequest) {
        if (!claimRequestOutcome(request, null, deliverInline = false)) return
        val future = request.future
        if (future != null) {
            if (!request.started.get()) runCatching { globalExecutor.remove(future as Runnable) }
            runCatching { future.cancel(true) }
        }
    }

    private fun postCallbacks(callbacks: List<Callback>, bitmap: DecodedBitmapLease?) {
        val deliveries = callbacks.map { callback ->
            callback to bitmap?.let { leaseForCallback(it, callback) }
        }
        postDeliveries(deliveries)
    }

    private fun postDeliveries(deliveries: List<Pair<Callback, DecodedBitmapLease?>>) {
        if (!mainHandler.post {
                deliveries.forEach { (callback, lease) -> deliverCallback(callback, lease) }
                drainSubmissions()
            }
        ) {
            deliveries.forEach { (callback, lease) ->
                lease?.close()
                callback.handle.finish()
            }
            drainSubmissions()
        }
    }

    private fun enqueueRejectionNotification(callback: Callback) {
        var postDrain = false
        val dropNotification = synchronized(rejectionLock) {
            if (rejectionNotifications.size >= REJECTION_NOTIFICATION_LIMIT) {
                true
            } else {
                rejectionNotifications.addLast(callback)
                if (!rejectionDrainPosted) {
                    rejectionDrainPosted = true
                    postDrain = true
                }
                false
            }
        }
        if (dropNotification) {
            // Rejection delivery itself is bounded. Once the retained main-thread batch
            // is full, shed the notification but always finish the handle. Calling the
            // consumer inline would violate async/main-thread delivery and enable retry
            // recursion under sustained overload.
            callback.handle.finish()
        } else if (postDrain && !mainHandler.post { drainRejectionNotifications() }) {
            // A stopped looper cannot honor the delivery contract; finish without
            // invoking consumer code on the caller thread.
            takeRejectionNotifications().forEach { it.handle.finish() }
        }
    }

    private fun drainRejectionNotifications() {
        takeRejectionNotifications().forEach { deliverCallback(it, null) }
    }

    private fun takeRejectionNotifications(): List<Callback> =
        synchronized(rejectionLock) {
            rejectionNotifications.toList().also {
                rejectionNotifications.clear()
                rejectionDrainPosted = false
            }
        }

    private fun deliverCallback(callback: Callback, lease: DecodedBitmapLease?) {
        try {
            if (!callback.cancelled.get()) callback.deliver(lease) else lease?.close()
        } catch (_: Exception) {
            lease?.close()
            // Consumer failures must not crash the delivery runnable or retain admission.
        } finally {
            callback.handle.finish()
        }
    }

    private fun releaseAdmission(callback: Callback) {
        if (!callback.admissionReleased.compareAndSet(false, true)) return
        synchronized(lock) {
            admissionCount = (admissionCount - 1).coerceAtLeast(0)
        }
    }

    private fun cancelCallback(key: RequestKey, callback: Callback) {
        callback.cancelled.set(true)
        var requestToCancel: PendingRequest? = null
        synchronized(lock) {
            val request = inFlight[key] ?: return@synchronized
            request.callbacks.removeAll { it.id == callback.id }
            if (request.callbacks.isNotEmpty()) return@synchronized
            if (!request.terminal.compareAndSet(false, true)) return@synchronized
            inFlight.remove(key)
            requestToCancel = request
            releaseRequestSlotLocked(request)
        }
        val request = requestToCancel ?: return
        runCatching { request.timeoutFuture?.cancel(false) }
        runCatching { request.cancellation.cancel() }
        val future = request.future
        if (future != null) {
            if (!request.started.get()) runCatching { globalExecutor.remove(future as Runnable) }
            runCatching { future.cancel(true) }
        }
        drainSubmissions()
    }

    private fun releaseRequestSlotLocked(request: PendingRequest) {
        if (!request.submitted) {
            policyStates[request.key.policy]?.pending?.remove(request)
            removePolicyStateIfEmptyLocked(request.key.policy)
            return
        }
        if (!request.workerSlotReleased.compareAndSet(false, true)) return
        readyToSubmit.remove(request)
        releaseSubmittedSlotLocked(request.key.policy)
    }

    private fun releaseSubmittedSlotLocked(policy: ImageLoadingPolicy) {
        val state = policyStates[policy] ?: return
        state.submittedCount = (state.submittedCount - 1).coerceAtLeast(0)
        val next = state.pending.pollFirst()
        if (next != null) {
            state.submittedCount += 1
            next.submitted = true
            enqueueReadyLocked(next)
        }
        removePolicyStateIfEmptyLocked(policy)
    }

    private fun removePolicyStateIfEmptyLocked(policy: ImageLoadingPolicy) {
        val state = policyStates[policy] ?: return
        if (state.submittedCount == 0 && state.pending.isEmpty()) policyStates.remove(policy)
    }

    private fun createGlobalExecutor() = object : ThreadPoolExecutor(
        GLOBAL_WORKERS,
        GLOBAL_WORKERS,
        30L,
        TimeUnit.SECONDS,
        BoundedPriorityQueue<Runnable>(GLOBAL_QUEUE_CAPACITY)
    ) {
        override fun afterExecute(runnable: Runnable?, throwable: Throwable?) {
            super.afterExecute(runnable, throwable)
            // A transient rejection is requeued. Signal again only after a worker task
            // has returned; posting to main lets the worker dequeue its next task first.
            mainHandler.post { drainSubmissions() }
        }
    }.apply { allowCoreThreadTimeOut(true) }

    private fun decode(request: PendingRequest): DecodedBitmapLease? {
        decodeSourceOverride?.let { override ->
            val bitmap = try {
                override(request.source, request.key.policy)
            } catch (_: OutOfMemoryError) {
                null
            } ?: return null
            val bytes = decodedAllocationBytes(bitmap)
            val reservation = DecodedBitmapBudget.shared().reserve(
                bytes,
                request.priority,
            ) ?: return null
            return reservation.commit(bitmap, bytes)
        }
        return RenderImageDecoder.decodeSourceLease(
                request.source,
                request.key.policy,
                request.cancellation,
                monotonicClockOverride ?: systemMonotonicClock,
                request.deadlineMs,
                request.priority,
            )
    }

    private fun leaseForCallback(
        lease: DecodedBitmapLease,
        callback: Callback,
    ): DecodedBitmapLease? = callback.ownerId?.let { ownerId ->
        lease.fork(ownerId, callback.ownerLimitBytes, callback.priority)
    } ?: lease.forkUnowned()

    private fun monotonicNowMs(): Long =
        (monotonicClockOverride ?: systemMonotonicClock).elapsedRealtime()

    private fun cacheKey(source: String, policy: ImageLoadingPolicy): CacheKey {
        digestConstructionCount.incrementAndGet()
        beforeDigestOverride?.invoke()
        val digest = MessageDigest.getInstance("SHA-256")
        digest.update(source.toByteArray(Charsets.UTF_8))
        digest.update(ImageLoadingPolicy.canonicalBytes(policy))
        return CacheKey(digest.digest())
    }
}
