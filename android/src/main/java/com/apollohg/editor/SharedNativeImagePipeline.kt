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
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.Future
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
    val maxDecodeDimensionPx: Int
) {
    companion object {
        val DEFAULT = ImageLoadingPolicy(
            maxSourceBytes = 10 * 1024 * 1024,
            connectTimeoutMs = 10_000,
            readTimeoutMs = 20_000,
            requestTimeoutMs = 60_000,
            maxConcurrentRequests = 2,
            maxPendingRequests = 64,
            maxDecodeDimensionPx = 2_048
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
                boundedPositiveInt("maxDecodeDimensionPx", DEFAULT.maxDecodeDimensionPx, 8_192)
            )
        }

        internal fun canonicalBytes(policy: ImageLoadingPolicy): ByteArray = ByteBuffer
            .allocate(Int.SIZE_BYTES * 7)
            .putInt(policy.maxSourceBytes)
            .putInt(policy.connectTimeoutMs)
            .putInt(policy.readTimeoutMs)
            .putInt(policy.requestTimeoutMs)
            .putInt(policy.maxConcurrentRequests)
            .putInt(policy.maxPendingRequests)
            .putInt(policy.maxDecodeDimensionPx)
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

    fun decodeSource(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT,
        cancellation: Cancellation? = null,
        clock: MonotonicClock = systemMonotonicClock,
        deadlineMs: Long = deadlineAfter(clock.elapsedRealtime(), policy.requestTimeoutMs)
    ): Bitmap? {
        if (unavailable(cancellation, clock, deadlineMs)) return null
        if (source.regionMatches(0, "data:image/", 0, "data:image/".length, ignoreCase = true)) {
            val bytes = decodeDataUrlBytes(source, policy, cancellation) ?: return null
            if (unavailable(cancellation, clock, deadlineMs)) return null
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
        val decoded = decodeBitmap(remoteBytes, policy)
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
        maxHeight: Int = ImageLoadingPolicy.DEFAULT.maxDecodeDimensionPx
    ): Int {
        if (width <= 0 || height <= 0) return 1

        if (maxWidth <= 0 || maxHeight <= 0) return 1
        var sampleSize = 1L
        var sampledWidth = width.toLong()
        var sampledHeight = height.toLong()
        while (sampledWidth > maxWidth.toLong() || sampledHeight > maxHeight.toLong()) {
            if (sampleSize >= (1L shl 30)) return (1 shl 30)
            sampleSize = sampleSize shl 1
            sampledWidth = (width.toLong() + sampleSize - 1L) / sampleSize
            sampledHeight = (height.toLong() + sampleSize - 1L) / sampleSize
        }
        return sampleSize.toInt().coerceAtLeast(1)
    }

    private fun decodeBitmap(bytes: ByteArray, policy: ImageLoadingPolicy): Bitmap? {
        bitmapDecoderOverride?.let {
            return constrainDecodedBitmap(it(bytes, policy), policy.maxDecodeDimensionPx)
        }
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
        return constrainDecodedBitmap(
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options),
            policy.maxDecodeDimensionPx
        )
    }

    private fun constrainDecodedBitmap(bitmap: Bitmap?, maximumDimension: Int): Bitmap? {
        bitmap ?: return null
        if (bitmap.width <= maximumDimension && bitmap.height <= maximumDimension) return bitmap
        val scale = minOf(
            maximumDimension.toDouble() / bitmap.width.toDouble(),
            maximumDimension.toDouble() / bitmap.height.toDouble()
        )
        val targetWidth = kotlin.math.floor(bitmap.width.toDouble() * scale)
            .toInt()
            .coerceIn(1, maximumDimension)
        val targetHeight = kotlin.math.floor(bitmap.height.toDouble() * scale)
            .toInt()
            .coerceIn(1, maximumDimension)
        val constrained = runCatching {
            Bitmap.createScaledBitmap(bitmap, targetWidth, targetHeight, true)
        }.getOrNull() ?: return null
        if (constrained !== bitmap) bitmap.recycle()
        return constrained
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
        callback: (Bitmap?) -> Unit,
    ): RenderImageLoader.LoadHandle = RenderImageLoader.load(source, callback)
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
        val deliver: (Bitmap?) -> Unit
    )
    private class PendingRequest(
        val key: RequestKey,
        val source: String,
        val callbacks: MutableList<Callback>,
        val cancellation: RenderImageDecoder.Cancellation,
        val startedAtMs: Long
    ) {
        val deadlineMs: Long = deadlineAfter(startedAtMs, key.policy.requestTimeoutMs)
        var future: Future<*>? = null
        var timeoutFuture: ScheduledFuture<*>? = null
        var submitted = false
        var dispatching = false
        val started = AtomicBoolean(false)
        val terminal = AtomicBoolean(false)
        val workerSlotReleased = AtomicBoolean(false)
    }
    private data class PolicyState(
        var submittedCount: Int = 0,
        val pending: java.util.ArrayDeque<PendingRequest> = java.util.ArrayDeque()
    )

    private val cache = object : LruCache<CacheKey, Bitmap>(32 * 1024 * 1024) {
        override fun sizeOf(key: CacheKey, value: Bitmap): Int =
            saturatingAdd(decodedAllocationBytes(value), key.digest.size.toLong() + CACHE_ENTRY_OVERHEAD_BYTES)
                .coerceAtMost(Int.MAX_VALUE.toLong())
                .toInt()
    }

    /** The cache is the only decoded-pixel allocation owner. */
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
    private val digestConstructionCount = AtomicLong()
    private val mainHandler by lazy { Handler(Looper.getMainLooper()) }
    private var globalExecutor = createGlobalExecutor()
    private val timeoutScheduler = Executors.newSingleThreadScheduledExecutor { runnable ->
        Thread(runnable, "native-editor-image-deadline").apply { isDaemon = true }
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

    fun cached(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
    ): Bitmap? = prepare(source, policy)?.let {
        synchronized(cache) { cache.get(cacheKey(it.source, it.policy)) }
    }

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

    fun load(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT,
        onLoaded: (Bitmap?) -> Unit
    ): LoadHandle = load(source, policy, null, onLoaded)

    internal fun load(
        prepared: PreparedSource,
        onLoaded: (Bitmap?) -> Unit
    ): LoadHandle = load(prepared.source, prepared.policy, prepared, onLoaded)

    private fun load(
        source: String,
        policy: ImageLoadingPolicy,
        prepared: PreparedSource?,
        onLoaded: (Bitmap?) -> Unit
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
            onLoaded
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
        synchronized(cache) { cache.get(resolvedRequestKey.digest) }?.let { bitmap ->
            scheduleCachedDelivery(callback, bitmap, deadlineMs)
            return handle
        }
        var drain = false
        var reject = false
        var createdRequest: PendingRequest? = null
        synchronized(lock) {
            val existing = inFlight[resolvedRequestKey]
            if (existing != null) {
                existing.callbacks += callback
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
                        readyToSubmit.addLast(pending)
                        drain = true
                    }
                    state.pending.size < policy.maxPendingRequests -> {
                        createdRequest = pending
                        inFlight[resolvedRequestKey] = pending
                        state.pending.addLast(pending)
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
            val future = globalExecutor.submit {
                request.started.set(true)
                var bitmap: Bitmap? = null
                try {
                    bitmap = decode(request)
                } catch (_: Exception) {
                    bitmap = null
                } finally {
                    completeDecodedRequest(request, bitmap)
                    beforeWorkerReturnOverride?.invoke(request.source)
                }
            }
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

    private fun completeDecodedRequest(request: PendingRequest, bitmap: Bitmap?) {
        synchronized(lock) {
            releaseRequestSlotLocked(request)
        }
        mainHandler.post { drainSubmissions() }
        if (request.terminal.get()) return
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
        candidateBitmap: Bitmap?,
        deliverInline: Boolean
    ): Boolean {
        if (!request.terminal.compareAndSet(false, true)) return false
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
        if (resolvedBitmap == null) request.cancellation.cancel()
        if (resolvedBitmap != null) {
            synchronized(cache) { cache.put(request.key.digest, resolvedBitmap) }
        }
        callbacks.forEach(::releaseAdmission)
        if (deliverInline) {
            callbacks.forEach { callback -> deliverCallback(callback, resolvedBitmap) }
            drainSubmissions()
        } else {
            postCallbacks(callbacks, resolvedBitmap)
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

    private fun scheduleCachedDelivery(callback: Callback, bitmap: Bitmap, deadlineMs: Long) {
        val terminal = AtomicBoolean(false)
        val delayMs = (deadlineMs - monotonicNowMs()).coerceAtLeast(0L)
        val timeoutFuture = timeoutScheduler.schedule(
            {
                deadlineExecutionGateOverride?.invoke()
                if (terminal.compareAndSet(false, true)) {
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
                val result = bitmap.takeIf {
                    !callback.cancelled.get() && monotonicNowMs() < deadlineMs
                }
                deliverCallback(callback, result)
            }
        ) {
            timeoutFuture.cancel(false)
            terminal.set(true)
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

    private fun postCallbacks(callbacks: List<Callback>, bitmap: Bitmap?) {
        if (!mainHandler.post {
                callbacks.forEach { callback -> deliverCallback(callback, bitmap) }
                drainSubmissions()
            }
        ) {
            callbacks.forEach { it.handle.finish() }
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

    private fun deliverCallback(callback: Callback, bitmap: Bitmap?) {
        try {
            if (!callback.cancelled.get()) callback.deliver(bitmap)
        } catch (_: Exception) {
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
            readyToSubmit.addLast(next)
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
        ArrayBlockingQueue(GLOBAL_QUEUE_CAPACITY)
    ) {
        override fun afterExecute(runnable: Runnable?, throwable: Throwable?) {
            super.afterExecute(runnable, throwable)
            // A transient rejection is requeued. Signal again only after a worker task
            // has returned; posting to main lets the worker dequeue its next task first.
            mainHandler.post { drainSubmissions() }
        }
    }.apply { allowCoreThreadTimeOut(true) }

    private fun decode(request: PendingRequest): Bitmap? =
        decodeSourceOverride?.invoke(request.source, request.key.policy)
            ?: RenderImageDecoder.decodeSource(
                request.source,
                request.key.policy,
                request.cancellation,
                monotonicClockOverride ?: systemMonotonicClock,
                request.deadlineMs
            )

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
