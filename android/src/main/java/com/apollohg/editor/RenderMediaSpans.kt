package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.util.Log
import android.text.style.ReplacementSpan
import android.view.View
import java.lang.ref.WeakReference
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

internal class AtomBlockSpan(
    val atomKey: String,
    val nodeType: String,
    val docPos: Int,
    var reservedHeightPx: Int,
    val hasStableAtomId: Boolean,
    val isDirectRootChild: Boolean,
) : ReplacementSpan() {
    override fun getSize(
        paint: Paint,
        text: CharSequence,
        start: Int,
        end: Int,
        fm: Paint.FontMetricsInt?
    ): Int {
        fm?.apply {
            ascent = -reservedHeightPx
            top = ascent
            descent = 0
            bottom = 0
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
    ) = Unit
}

internal class BlockImageSpan(
    private val source: String,
    hostView: View?,
    private val density: Float,
    private val preferredWidthDp: Float?,
    private val preferredHeightDp: Float?,
    internal var imageStyle: EditorElementStyle? = null
) : ReplacementSpan() {
    internal var ancestorWidthInset: Float = 0f
    private val hostRef = WeakReference(hostView)
    private val policy = (hostView as? EditorEditText)?.imageLoadingPolicy
        ?: ImageLoadingPolicy.DEFAULT
    private val generation = (hostView as? EditorEditText)?.currentImageLoadGeneration()
    private val boundEditorId = (hostView as? EditorEditText)?.editorId
    private val boundDriver = WeakReference((hostView as? EditorEditText)?.v2Driver)
    private val hadBoundDriver = boundDriver.get() != null
    private val ownerId = (hostView as? EditorEditText)?.decodedBitmapOwnerId
        ?: DecodedBitmapBudget.nextOwnerId()
    private val preparedSource = NativeImagePipeline.prepare(source, policy)

    private val retired = AtomicBoolean(false)
    private val bitmapLease = AtomicReference<DecodedBitmapLease?>()
    private val loadHandle = AtomicReference<RenderImageLoader.LoadHandle?>()
    @Volatile
    private var lastDrawRect: RectF? = null

    init {
        if (preparedSource != null) {
            val handle = NativeImagePipeline.load(
                preparedSource,
                ownerId,
                DecodedBitmapPriority.VISIBLE,
            ) { loaded ->
                val currentHost = hostRef.get()
                if (
                    retired.get() ||
                    currentHost is EditorEditText &&
                    !canReuseFor(currentHost)
                ) {
                    loaded?.close()
                    return@load
                }
                if (loaded == null) {
                    Log.w(
                        RenderImageDecoder.LOG_TAG,
                        "BlockImageSpan: loader returned null for image source"
                    )
                    return@load
                }
                val previous = bitmapLease.getAndSet(loaded)
                if (retired.get()) {
                    if (bitmapLease.compareAndSet(loaded, null)) loaded.close()
                    previous?.close()
                    return@load
                }
                previous?.close()
                currentHost?.post {
                    if (
                        currentHost is EditorEditText &&
                        !canReuseFor(currentHost)
                    ) {
                        close()
                        return@post
                    }
                    if (currentHost is EditorEditText) {
                        currentHost.onImageSpanSizeMayChange(this@BlockImageSpan)
                    } else {
                        currentHost.requestLayout()
                        currentHost.invalidate()
                    }
                }
            }
            if (!loadHandle.compareAndSet(null, handle) || retired.get()) {
                loadHandle.getAndSet(null)?.cancel()
            }
            handle.onFinished { loadHandle.compareAndSet(handle, null) }
            (hostView as? EditorEditText)?.registerImageLoad(handle)
        }
    }

    internal fun matches(source: String, width: Float?, height: Float?): Boolean = !retired.get() && this.source == source && preferredWidthDp == width && preferredHeightDp == height

    internal fun canReuseFor(host: EditorEditText): Boolean {
        val sameDriver = if (hadBoundDriver) boundDriver.get()?.let { it === host.v2Driver } == true else host.v2Driver == null
        return !retired.get() && hostRef.get() === host && sameDriver && boundEditorId == host.editorId &&
            generation == host.currentImageLoadGeneration() && policy == host.imageLoadingPolicy
    }

    internal fun reloadedFor(hostView: View): BlockImageSpan = BlockImageSpan(
        source = source,
        hostView = hostView,
        density = density,
        preferredWidthDp = preferredWidthDp,
        preferredHeightDp = preferredHeightDp,
        imageStyle = imageStyle
    ).also { it.ancestorWidthInset = ancestorWidthInset }

    internal fun close() {
        if (!retired.compareAndSet(false, true)) return
        loadHandle.getAndSet(null)?.cancel()
        bitmapLease.getAndSet(null)?.close()
    }

    override fun getSize(
        paint: Paint,
        text: CharSequence,
        start: Int,
        end: Int,
        fm: Paint.FontMetricsInt?
    ): Int {
        val (widthPx, heightPx) = currentSizePx()
        val inset = imageStyle?.box?.outerInset?.scaled(density) ?: EditorEdges()
        if (fm != null) {
            fm.ascent = -heightPx - (inset.top + inset.bottom).toInt()
            fm.descent = 0
            fm.top = fm.ascent
            fm.bottom = 0
        }
        return widthPx + (inset.left + inset.right).toInt()
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
        val rect = boxRect(x, y.toFloat())
        val host = hostRef.get()
        val documentRect = ((host as? EditorTextSurface)?.layout as? EditorDocumentLayout)?.imageBounds(this)
        lastDrawRect = RectF(documentRect ?: rect).apply {
            if (host != null) {
                offset((host as? android.widget.TextView)?.compoundPaddingLeft?.toFloat() ?: host.paddingLeft.toFloat(), (host as? android.widget.TextView)?.extendedPaddingTop?.toFloat() ?: host.paddingTop.toFloat())
            }
        }
        val loadedBitmap = bitmapLease.get()?.bitmap
        imageStyle?.let {
            val box = it.box.scaled(density)
            EditorBoxDrawing.draw(canvas, rect, box)
            if (loadedBitmap != null) EditorBoxDrawing.drawImage(canvas, loadedBitmap, rect, box, it.resizeMode)
            return
        }
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

    internal fun boxRect(x: Float, baseline: Float): RectF {
        val (widthPx, heightPx) = currentSizePx()
        val box = imageStyle?.box?.scaled(density) ?: EditorBoxStyle()
        return RectF(
            x + box.margin.left,
            baseline - heightPx - box.inset.top - box.inset.bottom - box.margin.bottom,
            x + box.margin.left + widthPx + box.inset.left + box.inset.right,
            baseline - box.margin.bottom,
        )
    }

    internal fun currentSizePx(): Pair<Int, Int> {
        val maxWidth = resolvedMaxWidth()
        val loadedBitmap = bitmapLease.get()?.bitmap
        val fallbackAspectRatio = if (loadedBitmap != null && loadedBitmap.width > 0 && loadedBitmap.height > 0) {
            loadedBitmap.height.toFloat() / loadedBitmap.width.toFloat()
        } else {
            0.56f
        }

        var widthPx = checkedPixels(preferredWidthDp)
        var heightPx = checkedPixels(preferredHeightDp)

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

        if (!widthPx.isFinite() || widthPx <= 0f || !heightPx.isFinite() || heightPx <= 0f) {
            widthPx = maxWidth
            heightPx = minOf(maxWidth * 0.56f, policy.maxDecodeDimensionPx.toFloat())
        }
        val maximumDimension = policy.maxDecodeDimensionPx.toFloat()
        val scale = minOf(
            1f,
            maxWidth / widthPx.coerceAtLeast(1f),
            maximumDimension / heightPx.coerceAtLeast(1f)
        )
        return Pair(
            checkedPositiveInt(widthPx * scale),
            checkedPositiveInt(heightPx * scale)
        )
    }

    internal fun currentDrawRect(): RectF? = lastDrawRect?.let(::RectF)

    private fun resolvedMaxWidth(): Float {
        val host = hostRef.get()
        val hostWidth = host?.let {
            maxOf(it.width, it.measuredWidth) - ((it as? android.widget.TextView)?.totalPaddingLeft ?: it.paddingLeft) - ((it as? android.widget.TextView)?.totalPaddingRight ?: it.paddingRight)
        } ?: 0
        val inset = imageStyle?.box?.outerInset?.scaled(density) ?: EditorEdges()
        val candidate = (if (hostWidth > 0) hostWidth.toDouble() else 240.0 * density.toDouble()) - inset.left - inset.right - ancestorWidthInset
        return candidate
            .takeIf { it.isFinite() && it > 0.0 }
            ?.coerceAtMost(policy.maxDecodeDimensionPx.toDouble())
            ?.toFloat()
            ?: policy.maxDecodeDimensionPx.toFloat()
    }

    private fun checkedPixels(preferredDp: Float?): Float? {
        val value = preferredDp ?: return null
        if (!value.isFinite() || value <= 0f || !density.isFinite() || density <= 0f) return null
        val pixels = value.toDouble() * density.toDouble()
        return pixels.takeIf { it.isFinite() && it > 0.0 && it <= Int.MAX_VALUE.toDouble() }?.toFloat()
    }

    private fun checkedPositiveInt(value: Float): Int = value
        .takeIf { it.isFinite() && it > 0f }
        ?.toDouble()
        ?.coerceAtMost(Int.MAX_VALUE.toDouble())
        ?.toInt()
        ?.coerceAtLeast(1)
        ?: 1
}
