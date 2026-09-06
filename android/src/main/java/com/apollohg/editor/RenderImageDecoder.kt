package com.apollohg.editor

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.util.Base64
import android.util.Log
import android.util.Xml
import com.caverock.androidsvg.SVG
import org.xmlpull.v1.XmlPullParser
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.atomic.AtomicBoolean

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
