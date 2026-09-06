package com.apollohg.editor

import android.os.SystemClock
import org.json.JSONObject
import java.nio.ByteBuffer

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

internal val systemMonotonicClock = MonotonicClock { SystemClock.elapsedRealtime() }

internal fun deadlineAfter(startedAtMs: Long, timeoutMs: Int): Long =
    if (startedAtMs > Long.MAX_VALUE - timeoutMs.toLong()) Long.MAX_VALUE
    else startedAtMs + timeoutMs.toLong()
