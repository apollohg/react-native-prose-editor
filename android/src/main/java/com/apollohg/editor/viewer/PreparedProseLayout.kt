package com.apollohg.editor.viewer

import android.graphics.Rect
import android.text.StaticLayout
import com.apollohg.editor.ProseViewerError

internal data class ProseLayoutKey(
    val semanticKey: String,
    val widthPx: Int,
    val themeDigest: String,
    val nativeFontRevision: Long,
    val fontEnvironmentRevision: Long,
    val densityBits: Long,
    val attachmentRevision: Long,
    val generationIdentity: String,
    /** Immutable semantic diagnostic context; excludes replacement revisions. */
    val semanticGenerationIdentity: String = semanticKey,
)

internal data class FabricSurfaceToken(val surfaceId: Int, val componentTag: Int)
/**
 * The state-owned Fabric incarnation.  A recycled component may reuse the
 * same surface/tag and even the same semantic generation, so every handoff
 * operation must carry the native lease handle as well.
 */
internal data class FabricGenerationToken(
    val surface: FabricSurfaceToken,
    val generationIdentity: String,
    val leaseHandle: Long,
) {
    init {
        require(leaseHandle > 0) { "Fabric lease handles must be signed, non-zero Int64 values." }
    }
}

/** State-lifetime identity shared by all semantic/width revisions of a family. */
internal data class FabricLeaseOwner(
    val surface: FabricSurfaceToken,
    val leaseHandle: Long,
)
internal data class FabricLeaseKey(val generation: FabricGenerationToken, val layout: ProseLayoutKey)
internal data class ProseMountKey(val generationIdentity: String, val widthPx: Int, val densityBits: Long)

/** Paint-only operations which Android spans cannot represent accurately. */
internal enum class PreparedProseFragmentKind { TEXT, MARKER, BACKGROUND, BORDER, RULE, ATOM, STRIKE, IMAGE }

internal data class PreparedProseFragment(
    val kind: PreparedProseFragmentKind,
    val bounds: Rect,
    val layout: StaticLayout? = null,
    val layoutX: Int = bounds.left,
    val layoutY: Int = bounds.top,
    val labelLayout: StaticLayout? = null,
    val labelX: Int = bounds.left,
    val labelY: Int = bounds.top,
    val color: Int? = null,
    val borderColor: Int? = null,
    val cornerRadius: Float = 0f,
    val strokeWidth: Float = 0f,
    val label: String? = null,
    /** Immutable compiler identity for an inline atom; absent for non-atoms. */
    val atomNodeType: String? = null,
    val atomDocPos: Long? = null,
    val atomAttrsJson: String? = null,
    val checked: Boolean = false,
    val box: com.apollohg.editor.EditorBoxStyle? = null,
    val decorationBounds: Rect? = null,
    val resizeMode: String = "contain",
) {
    val retainedBytes: Long
        get() = 160L + (layout?.text?.length ?: 0).toLong() * 4 + (labelLayout?.text?.length ?: 0).toLong() * 4 + (label?.length ?: 0).toLong() * 2 + (atomNodeType?.length ?: 0).toLong() * 2 + (atomAttrsJson?.length ?: 0).toLong() * 2
}

/** A vertically sorted immutable culling unit. */
internal data class PreparedProseBlock(
    val fragments: List<PreparedProseFragment>,
    val bounds: Rect,
) {
    val topPx: Int get() = bounds.top
    val bottomPx: Int get() = bounds.bottom
    fun intersects(clip: Rect): Boolean = bottomPx > clip.top && topPx < clip.bottom
    val retainedBytes: Long get() = 160L + fragments.sumOf { it.retainedBytes }
}

internal data class PreparedProseInteraction(
    val kind: Kind,
    val rects: List<Rect>,
    val href: String? = null,
    val visibleText: String,
    /** Kept as Long so the complete unsigned Rust u32 domain is lossless. */
    val docPos: Long? = null,
    val label: String,
    val attrsJson: String? = null,
) {
    enum class Kind { LINK, MENTION }
    val retainedBytes: Long get() = 144L + rects.size * 32L + (href?.length ?: 0) * 2L + visibleText.length * 2L + label.length * 2L + (attrsJson?.length ?: 0) * 2L
}

internal data class PreparedProseAccessibilityNode(
    val interactionIndex: Int,
    val role: Role,
    val label: String,
    val bounds: Rect,
) {
    enum class Role { LINK, MENTION }
    val retainedBytes: Long get() = 96L + label.length * 2L
}

/** A fully prepared artifact. StaticLayout construction is complete before publication. */
internal data class PreparedProseLayout(
    val key: ProseLayoutKey,
    val widthPx: Int,
    val heightPx: Int,
    val blocks: List<PreparedProseBlock>,
    val interactions: List<PreparedProseInteraction> = emptyList(),
    val accessibilityNodes: List<PreparedProseAccessibilityNode> = emptyList(),
    val imageAttachments: List<ViewerImageAttachment> = emptyList(),
    val retainedBytes: Long,
    val error: ProseViewerError? = null,
    val viewerAtoms: List<PreparedViewerAtom> = emptyList(),
    val contentBox: com.apollohg.editor.EditorBoxStyle? = null,
    val codeHighlighting: com.apollohg.editor.NativeCodeHighlightingConfig? = null,
    val codeHighlightBlocks: List<com.apollohg.editor.CodeHighlightBlock> = emptyList(),
    val highlightedCodeKeys: Set<String> = emptySet(),
) {
    val fragmentKinds: Set<PreparedProseFragmentKind> get() = blocks.flatMapTo(linkedSetOf()) { block -> block.fragments.map { it.kind } }

    inline fun forEachBlockIntersecting(clip: Rect, action: (PreparedProseBlock) -> Unit) {
        var lower = 0
        var upper = blocks.size
        while (lower < upper) {
            val middle = (lower + upper) ushr 1
            if (blocks[middle].bottomPx > clip.top) upper = middle else lower = middle + 1
        }
        while (lower < blocks.size) {
            val block = blocks[lower]
            if (block.topPx >= clip.bottom) return
            action(block)
            lower += 1
        }
    }

    inline fun forEachFragmentIntersecting(clip: Rect, action: (PreparedProseFragment) -> Unit) =
        forEachBlockIntersecting(clip) { block -> block.fragments.filterTo(mutableListOf()) { Rect.intersects(it.bounds, clip) }.forEach(action) }

    companion object {
        fun error(key: ProseLayoutKey, widthPx: Int, error: ProseViewerError) =
            PreparedProseLayout(key, widthPx, 0, emptyList(), retainedBytes = 0, error = error)
    }
}
