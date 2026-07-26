package com.apollohg.editor.viewer

import android.graphics.Bitmap
import android.graphics.Rect
import com.apollohg.editor.ImageLoadingPolicy
import com.apollohg.editor.RenderImageLoader
import com.apollohg.editor.viewer.ViewerInline.Atom
import org.json.JSONObject

/** Immutable attachment geometry emitted by preparation; bitmap state is not part of an artifact. */
internal data class ViewerImageAttachment(
    val id: String,
    val source: String,
    val bounds: Rect,
    val declaredSize: Pair<Int, Int>?,
) {
    val hasDeclaredSize: Boolean get() = (declaredSize?.first ?: 0) > 0 && (declaredSize?.second ?: 0) > 0

    companion object {
        fun sourceAndDeclaredSize(block: ViewerBlock): Triple<String, String, Pair<Int, Int>?>? {
            val atom = block.inlines.filterIsInstance<Atom>().firstOrNull { it.nodeType == "image" } ?: return null
            val attrs = runCatching { JSONObject(atom.attrsJson) }.getOrNull() ?: return null
            val source = attrs.optString("src").takeIf(String::isNotEmpty) ?: return null
            val width = attrs.optDouble("width", Double.NaN).takeIf { it.isFinite() && it > 0 }?.toInt()
            val height = attrs.optDouble("height", Double.NaN).takeIf { it.isFinite() && it > 0 }?.toInt()
            return Triple("${atom.docPos}:$source", source, if (width != null && height != null) width to height else null)
        }
    }
}

internal object ViewerImageIntrinsicStore {
    private val values = mutableMapOf<String, Pair<Int, Int>>()
    fun size(id: String): Pair<Int, Int>? = synchronized(values) { values[id] }
    fun store(id: String, size: Pair<Int, Int>) = synchronized(values) { values[id] = size }
}

/** Shared native image admission/loading boundary used by editor spans and viewer fragments. */
internal object NativeImagePipeline {
    fun prepare(source: String, policy: ImageLoadingPolicy): RenderImageLoader.PreparedSource? =
        RenderImageLoader.prepare(source, policy)

    fun load(source: RenderImageLoader.PreparedSource, callback: (Bitmap?) -> Unit): RenderImageLoader.LoadHandle =
        RenderImageLoader.load(source, callback)
}

/** Atomically persists only the first valid intrinsic size for an unknown attachment. */
internal class ViewerAttachmentRevisionState {
    private val lock = Any()
    private val intrinsicSizes = mutableMapOf<String, Pair<Int, Int>>()
    var revision: Long = 0
        private set

    fun intrinsicSize(id: String): Pair<Int, Int>? = synchronized(lock) { intrinsicSizes[id] }

    fun recordIntrinsicSize(id: String, width: Int, height: Int, declaredSize: Pair<Int, Int>?): Boolean = synchronized(lock) {
        if (declaredSize != null || width <= 0 || height <= 0 || intrinsicSizes.containsKey(id)) return@synchronized false
        intrinsicSizes[id] = width to height
        ViewerImageIntrinsicStore.store(id, width to height)
        revision += 1
        true
    }
}

/**
 * Viewport/generation facade over the editor's established bounded pipeline.
 * `RenderImageLoader` remains the one owner of source admission, decode/fetch
 * limits, byte-bounded cache, de-duplication, cancellation receipts and errors.
 */
internal class ViewerImagePipeline(
    private val load: (RenderImageLoader.PreparedSource, (Bitmap?) -> Unit) -> RenderImageLoader.LoadHandle = { source, callback -> RenderImageLoader.load(source, callback) },
) {
    companion object { const val PREFETCH_MARGIN_PX = 480 }

    private val lock = Any()
    private var generation = ""
    private var enabled = false
    private var policy = ImageLoadingPolicy.DEFAULT
    private val requested = mutableSetOf<String>()
    private val receipts = mutableMapOf<String, RenderImageLoader.LoadHandle>()
    var requestCountForTesting: Int = 0
        private set
    var onPixels: ((ViewerImageAttachment, Bitmap) -> Unit)? = null
    var onIntrinsicMetadata: ((ViewerImageAttachment, Int, Int) -> Unit)? = null

    fun begin(generation: String, imagesEnabled: Boolean, policy: ImageLoadingPolicy = this.policy) = synchronized(lock) {
        if (this.generation == generation && enabled == imagesEnabled && this.policy == policy) return@synchronized
        receipts.values.forEach(RenderImageLoader.LoadHandle::cancel)
        receipts.clear()
        requested.clear()
        requestCountForTesting = 0
        this.generation = generation
        this.enabled = imagesEnabled
        this.policy = policy
    }

    fun cancel() = synchronized(lock) {
        generation = ""
        enabled = false
        receipts.values.forEach(RenderImageLoader.LoadHandle::cancel)
        receipts.clear()
        requested.clear()
    }

    fun acceptsCompletion(generation: String): Boolean = synchronized(lock) {
        enabled && this.generation.isNotEmpty() && this.generation == generation
    }

    fun updateVisibleRect(visible: Rect, attachments: List<ViewerImageAttachment>) {
        val prefetched = Rect(visible).apply { inset(-PREFETCH_MARGIN_PX, -PREFETCH_MARGIN_PX) }
        val start = synchronized(lock) {
            if (!enabled || generation.isEmpty()) return
            attachments.filter { it.source.isNotEmpty() && Rect.intersects(it.bounds, prefetched) && requested.add(it.id) }
                .also { requestCountForTesting += it.size }
                .map { it to generation }
        }
        start.forEach { (attachment, requestGeneration) ->
            val source = NativeImagePipeline.prepare(attachment.source, policy) ?: return@forEach
            val receipt = load(source) { bitmap ->
                if (bitmap == null || !acceptsCompletion(requestGeneration)) return@load
                onIntrinsicMetadata?.invoke(attachment, bitmap.width, bitmap.height)
                if (acceptsCompletion(requestGeneration)) onPixels?.invoke(attachment, bitmap)
            }
            synchronized(lock) { if (acceptsCompletion(requestGeneration)) receipts[attachment.id] = receipt else receipt.cancel() }
        }
    }
}
