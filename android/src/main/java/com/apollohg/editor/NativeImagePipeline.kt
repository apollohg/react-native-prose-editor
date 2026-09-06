package com.apollohg.editor

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
