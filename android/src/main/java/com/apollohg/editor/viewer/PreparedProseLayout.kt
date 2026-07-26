package com.apollohg.editor.viewer

import android.graphics.Rect
import android.text.StaticLayout
import com.apollohg.editor.ProseViewerError

internal data class ProseLayoutKey(
    val semanticKey: String,
    val widthPx: Int,
    val themeDigest: String,
    val fontRevision: Long,
    val densityBits: Long,
    val attachmentRevision: Long,
    val generationIdentity: String,
)

internal data class FabricSurfaceToken(val surfaceId: Int, val componentTag: Int)

internal data class FabricGenerationToken(
    val surface: FabricSurfaceToken,
    val generationIdentity: String,
)

internal data class ProseMountKey(
    val generationIdentity: String,
    val widthPx: Int,
    val densityBits: Long,
)

internal data class PreparedProseBlock(
    val layout: StaticLayout,
    val topPx: Int,
    val bottomPx: Int,
) {
    fun intersects(clip: Rect): Boolean = bottomPx > clip.top && topPx < clip.bottom
}

internal data class PreparedProseInteraction(val unused: Unit = Unit)
internal data class PreparedProseAccessibilityNode(val unused: Unit = Unit)

/** A fully prepared, immutable Android artifact. */
internal data class PreparedProseLayout(
    val key: ProseLayoutKey,
    val widthPx: Int,
    val heightPx: Int,
    val blocks: List<PreparedProseBlock>,
    val interactions: List<PreparedProseInteraction> = emptyList(),
    val accessibilityNodes: List<PreparedProseAccessibilityNode> = emptyList(),
    val retainedBytes: Long,
    val error: ProseViewerError? = null,
) {
    companion object {
        fun error(key: ProseLayoutKey, widthPx: Int, error: ProseViewerError) =
            PreparedProseLayout(
                key = key,
                widthPx = widthPx,
                heightPx = 0,
                blocks = emptyList(),
                retainedBytes = 0,
                error = error,
            )
    }
}
