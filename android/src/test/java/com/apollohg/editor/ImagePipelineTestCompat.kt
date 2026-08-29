package com.apollohg.editor

import android.graphics.Bitmap

internal fun RenderImageDecoder.decodeSource(
    source: String,
    policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT,
    cancellation: RenderImageDecoder.Cancellation? = null,
    clock: MonotonicClock = MonotonicClock { android.os.SystemClock.elapsedRealtime() },
    deadlineMs: Long = Long.MAX_VALUE,
): Bitmap? {
    val lease = decodeSourceLease(
        source,
        policy,
        cancellation,
        clock,
        deadlineMs,
        DecodedBitmapPriority.VISIBLE,
    ) ?: return null
    return lease.bitmap.also { lease.close() }
}

internal fun RenderImageLoader.load(
    source: String,
    policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT,
    onLoaded: (Bitmap?) -> Unit,
): RenderImageLoader.LoadHandle = loadLease(
    source,
    policy,
    DecodedBitmapBudget.nextOwnerId(),
    DecodedBitmapPriority.VISIBLE,
) { lease ->
    try {
        onLoaded(lease?.bitmap)
    } finally {
        lease?.close()
    }
}

internal fun RenderImageLoader.load(
    source: RenderImageLoader.PreparedSource,
    onLoaded: (Bitmap?) -> Unit,
): RenderImageLoader.LoadHandle = load(
    source.source,
    source.policy,
    onLoaded,
)
