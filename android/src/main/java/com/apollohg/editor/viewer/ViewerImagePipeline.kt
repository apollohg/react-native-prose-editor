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
    /** Ordinal within the immutable prepared artifact; id is source-qualified cache identity. */
    val ordinal: Int = -1,
) {
    val hasDeclaredSize: Boolean get() = (declaredSize?.first ?: 0) > 0 && (declaredSize?.second ?: 0) > 0

    companion object {
        /** Compiler/admission ceiling; supports far more than the old 256 IDs. */
        const val MAXIMUM_ADMITTED_ATTACHMENTS = 8_192
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

internal class ViewerImageIntrinsicStore(private val entryLimit: Int = 256) {
    companion object { val shared = ViewerImageIntrinsicStore() }

    private data class Entry(val size: Pair<Int, Int>, var access: Long)
    private val lock = Any()
    private val values = mutableMapOf<String, Entry>()
    private var access = 0L

    fun size(id: String): Pair<Int, Int>? {
        val cached = synchronized(lock) {
        values[id]?.also { entry ->
            access += 1
            entry.access = access
        }?.size
        }
        return cached ?: ViewerAttachmentRevisionState.authoritativeSize(id)
    }

    fun store(id: String, size: Pair<Int, Int>) = synchronized(lock) {
        if (size.first <= 0 || size.second <= 0) return@synchronized
        access += 1
        values[id] = Entry(size, access)
        while (values.size > entryLimit) {
            val oldest = values.minWithOrNull(compareBy<Map.Entry<String, Entry>> { it.value.access }.thenBy { it.key }) ?: break
            values.remove(oldest.key)
        }
    }
}

/** Shared native image admission/loading boundary used by editor spans and viewer fragments. */
internal object NativeImagePipeline {
    fun prepare(source: String, policy: ImageLoadingPolicy): RenderImageLoader.PreparedSource? =
        RenderImageLoader.prepare(source, policy)

    fun load(source: RenderImageLoader.PreparedSource, callback: (Bitmap?) -> Unit): RenderImageLoader.LoadHandle =
        RenderImageLoader.load(source, callback)
}

/**
 * Per-surface reflow-publication state. Compact ordinal metadata plus a
 * bitset makes the global LRU an optimization; reset only for semantic
 * replacement or recycle/teardown, never request cancellation.
 */
internal class ViewerAttachmentRevisionState {
    companion object {
        private val activeStateLock = Any()
        private val activeStates = mutableListOf<java.lang.ref.WeakReference<ViewerAttachmentRevisionState>>()

        fun authoritativeSize(id: String): Pair<Int, Int>? = synchronized(activeStateLock) {
            activeStates.removeAll { it.get() == null }
            activeStates.firstNotNullOfOrNull { it.get()?.intrinsicSizeForSourceQualifiedId(id) }
        }

        private fun register(state: ViewerAttachmentRevisionState) = synchronized(activeStateLock) {
            activeStates.removeAll { it.get() == null }
            if (activeStates.none { it.get() === state }) activeStates += java.lang.ref.WeakReference(state)
        }

        private fun unregister(state: ViewerAttachmentRevisionState) = synchronized(activeStateLock) {
            activeStates.removeAll { it.get() == null || it.get() === state }
        }
    }
    private val lock = Any()
    private var publishedBits = ByteArray(0)
    private var intrinsicWidths = IntArray(0)
    private var intrinsicHeights = IntArray(0)
    private var sourceQualifiedIds = arrayOfNulls<String>(0)
    private var admittedAttachmentCount = 0
    var revision: Long = 0
        private set

    /** Exact per-host state: one bit for every admitted immutable attachment. */
    val retainedPublicationBytesForTesting: Int get() = synchronized(lock) {
        publishedBits.size + intrinsicWidths.size * Int.SIZE_BYTES + intrinsicHeights.size * Int.SIZE_BYTES + sourceQualifiedIds.size * Long.SIZE_BYTES
    }

    fun admit(attachmentCount: Int) {
        val count = attachmentCount.coerceAtLeast(0)
        synchronized(lock) {
            if (admittedAttachmentCount == count) return@synchronized
            admittedAttachmentCount = count
            publishedBits = ByteArray((count + 7) / 8)
            intrinsicWidths = IntArray(count)
            intrinsicHeights = IntArray(count)
            sourceQualifiedIds = arrayOfNulls(count)
        }
        if (count > 0) register(this)
    }

    fun reset() = synchronized(lock) {
        publishedBits = ByteArray(0)
        intrinsicWidths = IntArray(0)
        intrinsicHeights = IntArray(0)
        sourceQualifiedIds = arrayOfNulls(0)
        admittedAttachmentCount = 0
        revision = 0
    }.also { unregister(this) }

    fun recordIntrinsicSize(id: String, ordinal: Int, width: Int, height: Int, declaredSize: Pair<Int, Int>?): Boolean = synchronized(lock) {
        if (declaredSize != null || width <= 0 || height <= 0 || ordinal !in 0 until admittedAttachmentCount) return@synchronized false
        val byteIndex = ordinal / 8
        val mask = 1 shl (ordinal % 8)
        if ((publishedBits[byteIndex].toInt() and mask) != 0) return@synchronized false
        publishedBits[byteIndex] = (publishedBits[byteIndex].toInt() or mask).toByte()
        intrinsicWidths[ordinal] = width
        intrinsicHeights[ordinal] = height
        sourceQualifiedIds[ordinal] = id
        ViewerImageIntrinsicStore.shared.store(id, width to height)
        revision += 1
        true
    }

    fun intrinsicSize(ordinal: Int): Pair<Int, Int>? = synchronized(lock) {
        if (ordinal !in 0 until admittedAttachmentCount) return@synchronized null
        val mask = 1 shl (ordinal % 8)
        if ((publishedBits[ordinal / 8].toInt() and mask) == 0) null
        else intrinsicWidths[ordinal] to intrinsicHeights[ordinal]
    }

    private fun intrinsicSizeForSourceQualifiedId(id: String): Pair<Int, Int>? = synchronized(lock) {
        val ordinal = sourceQualifiedIds.indexOfFirst { it == id }
        if (ordinal < 0) return@synchronized null
        val mask = 1 shl (ordinal % 8)
        if ((publishedBits[ordinal / 8].toInt() and mask) == 0) null
        else intrinsicWidths[ordinal] to intrinsicHeights[ordinal]
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
    private val failed = mutableSetOf<String>()
    private val receipts = mutableMapOf<String, RenderImageLoader.LoadHandle>()
    var requestCountForTesting: Int = 0
        private set
    var onPixels: ((ViewerImageAttachment, Bitmap) -> Unit)? = null
    var onIntrinsicMetadata: ((ViewerImageAttachment, Int, Int) -> Unit)? = null
    /** Contains only the internal attachment token, never its source URL. */
    var onResourceFailure: ((ViewerImageAttachment) -> Unit)? = null

    fun begin(generation: String, imagesEnabled: Boolean, policy: ImageLoadingPolicy = this.policy) = synchronized(lock) {
        if (this.generation == generation && enabled == imagesEnabled && this.policy == policy) return@synchronized
        receipts.values.forEach(RenderImageLoader.LoadHandle::cancel)
        receipts.clear()
        requested.clear()
        failed.clear()
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
        failed.clear()
    }

    fun acceptsCompletion(generation: String): Boolean = synchronized(lock) {
        enabled && this.generation.isNotEmpty() && this.generation == generation
    }

    fun updateVisibleRect(visible: Rect, attachments: List<ViewerImageAttachment>) {
        if (visible.isEmpty) return
        val prefetched = Rect(visible).apply { inset(-PREFETCH_MARGIN_PX, -PREFETCH_MARGIN_PX) }
        val start = synchronized(lock) {
            if (!enabled || generation.isEmpty()) return
            attachments.filter { it.ordinal >= 0 && it.source.isNotEmpty() && Rect.intersects(it.bounds, prefetched) && requested.add(it.id) }
                .also { requestCountForTesting += it.size }
                .map { it to generation }
        }
        start.forEach { (attachment, requestGeneration) ->
            val source = NativeImagePipeline.prepare(attachment.source, policy) ?: run {
                reportFailure(attachment, requestGeneration)
                return@forEach
            }
            val receipt = load(source) { bitmap ->
                if (!acceptsCompletion(requestGeneration)) return@load
                if (bitmap == null || bitmap.width <= 0 || bitmap.height <= 0) {
                    reportFailure(attachment, requestGeneration)
                    return@load
                }
                onIntrinsicMetadata?.invoke(attachment, bitmap.width, bitmap.height)
                if (acceptsCompletion(requestGeneration)) onPixels?.invoke(attachment, bitmap)
            }
            synchronized(lock) { if (acceptsCompletion(requestGeneration)) receipts[attachment.id] = receipt else receipt.cancel() }
        }
    }

    private fun reportFailure(attachment: ViewerImageAttachment, requestGeneration: String) {
        val callback = synchronized(lock) {
            if (!enabled || generation != requestGeneration || !failed.add(attachment.id)) null else onResourceFailure
        }
        callback?.invoke(attachment)
    }

    internal fun reportFailureForTesting(attachment: ViewerImageAttachment) {
        val current = synchronized(lock) { generation }
        reportFailure(attachment, current)
    }
}
