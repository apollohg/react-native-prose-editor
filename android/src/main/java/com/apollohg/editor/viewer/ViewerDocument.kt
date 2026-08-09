package com.apollohg.editor.viewer

import com.apollohg.editor.ProseViewerConfiguration
import com.apollohg.editor.ProseViewerError
import com.apollohg.editor.ProseViewerSource
import java.security.MessageDigest
import org.json.JSONObject
import uniffi.editor_core.FfiViewerCompileRequest
import uniffi.editor_core.FfiViewerElement
import uniffi.editor_core.FfiViewerMark
import uniffi.editor_core.FfiViewerSourceKind
import uniffi.editor_core.viewerCompile

/** A typed, width-independent projection of Task 1's immutable compiler stream. */
internal data class ViewerListContext(
    val ordered: Boolean,
    /** Rust u32 list index retained exactly for interaction/accessibility consumers. */
    val index: Long,
    val kind: String?,
    val checked: Boolean,
    val isLast: Boolean,
)

/** Identifies the nearest list item and its first/final renderable leaf. */
internal data class ViewerListItemBoundary(
    val identity: Int,
    val nestingDepth: Int,
    val isFirstRenderableLeaf: Boolean,
    val isFinalRenderableLeaf: Boolean,
)

/**
 * One list-item owner on a leaf's full semantic path, ordered outermost first.
 * Every owner keeps its own marker reservation even when a nested descendant is
 * the nearest list item exposed to interaction/accessibility consumers.
 */
internal data class ViewerListItemAncestor(
    val identity: Int,
    val context: ViewerListContext,
    val nestingDepth: Int,
    val isFirstRenderableLeaf: Boolean,
    val isFinalRenderableLeaf: Boolean,
)

internal sealed interface ViewerInline {
    data class Text(val text: String, val marks: List<FfiViewerMark>) : ViewerInline
    /** Rust u32 document position retained exactly; drawing spans never own it. */
    data class Atom(val nodeType: String, val docPos: Long, val attrsJson: String, val label: String) : ViewerInline
}

internal data class ViewerBlock(
    val nodeType: String,
    val depth: Int,
    val inBlockquote: Boolean,
    val listContext: ViewerListContext?,
    val listItemBoundary: ViewerListItemBoundary?,
    val inlines: List<ViewerInline>,
    val listItemAncestors: List<ViewerListItemAncestor> = emptyList(),
    val outermostListItemIdentity: Int? = null,
    val outermostListItemIsLast: Boolean = false,
)

/** Semantic positions live only in [ViewerInline.Atom], never in Android drawing spans. */
internal data class ViewerDocument(
    val semanticKey: String,
    val blocks: List<ViewerBlock>,
    val isEmpty: Boolean,
    val retainedBytes: Long,
    val trailingEmptyTextBlockCount: Int = 0,
)

internal data class ProseViewerRequest(
    val source: ProseViewerSource,
    val configuration: ProseViewerConfiguration,
    val nativeFontRevision: Long = 0,
    val fontEnvironmentRevision: Long = 0,
    val attachmentRevision: Long = 0,
) {
    val compiledCacheKey: String by lazy {
        sha256(listOf(source.value, configuration.configJson, configuration.imagePolicyJson.orEmpty(), if (configuration.imagesEnabled) "1" else "0", mentionPrefix(configuration.configJson).orEmpty(), source.kind).joinToString("\u001f"))
    }
    val themeDigest: String by lazy { sha256(configuration.themeJson.orEmpty()) }
    /** Semantic publication identity; layout/font revisions deliberately do not enter it. */
    val semanticGenerationIdentity: String by lazy {
        sha256(listOf(
            source.kind,
            source.value,
            configuration.configJson,
            configuration.themeJson.orEmpty(),
            configuration.imagePolicyJson.orEmpty(),
            if (configuration.imagesEnabled) "1" else "0",
            if (configuration.collapsesWhenEmpty) "1" else "0",
            mentionPrefix.orEmpty(),
        ).joinToString("\u001f"))
    }
    /** Immutable layout/cache identity including permitted state-only revisions. */
    val generationIdentity: String by lazy {
        sha256(listOf(semanticGenerationIdentity, attachmentRevision.toString(), nativeFontRevision.toString(), fontEnvironmentRevision.toString()).joinToString("\u001f"))
    }
    val mentionPrefix: String? get() = mentionPrefix(configuration.configJson)
}

internal typealias DocumentCompiler = (ProseViewerRequest) -> ViewerDocument

internal fun compileWithRust(request: ProseViewerRequest): ViewerDocument {
    val result = viewerCompile(
        FfiViewerCompileRequest(
            sourceKind = if (request.source is ProseViewerSource.Html) FfiViewerSourceKind.HTML else FfiViewerSourceKind.JSON,
            source = request.source.value,
            configJson = request.configuration.configJson,
            imagesEnabled = request.configuration.imagesEnabled,
            mentionPrefix = request.mentionPrefix,
        )
    )
    try {
        result.error?.let { throw ProseViewerError.compiler(it.domain, it.code, it.message) }
        val compiled = result.value ?: throw ProseViewerError.compiler("viewer", "MISSING_COMPILED_DOCUMENT", "The compiler returned neither a document nor an error.")
        val semanticKey = compiled.semanticKey()
        if (!semanticKey.matches(Regex("[0-9a-f]{64}"))) {
            throw ProseViewerError.compiler("viewer", "INVALID_SEMANTIC_KEY", "The compiler returned an invalid semantic key.")
        }

        data class Builder(
            val nodeType: String,
            val depth: Int,
            val listContext: ViewerListContext?,
            val listItemIdentity: Int?,
            val listItemContext: ViewerListContext?,
            val inlines: MutableList<ViewerInline> = mutableListOf(),
        )

        val stack = mutableListOf<Builder>()
        val rendered = mutableListOf<ViewerBlock>()
        // A list item's terminal spacing belongs after its own direct leaves,
        // before a child list begins. Descendant leaves are retained only as a
        // fallback for an item whose sole renderable content is nested.
        val directLeavesByListItem = mutableMapOf<Int, MutableList<Int>>()
        val descendantLeavesByListItem = mutableMapOf<Int, MutableList<Int>>()
        val listItemDepths = mutableMapOf<Int, Int>()
        var nextListItemIdentity = 0

        fun nearestListContext(builders: List<Builder>): ViewerListContext? = builders.asReversed().firstNotNullOfOrNull { it.listContext }
        fun listItemAncestors(builders: List<Builder>): List<ViewerListItemAncestor> = builders.mapNotNull { builder ->
            val identity = builder.listItemIdentity ?: return@mapNotNull null
            val context = builder.listItemContext ?: return@mapNotNull null
            ViewerListItemAncestor(identity, context, builder.depth, false, false)
        }
        fun appendLeaf(nodeType: String, depth: Int, inlines: List<ViewerInline>, ancestors: List<Builder>) {
            val itemAncestors = listItemAncestors(ancestors)
            rendered += ViewerBlock(
                nodeType = nodeType,
                depth = depth,
                inBlockquote = ancestors.any { it.nodeType == "blockquote" },
                listContext = nearestListContext(ancestors),
                listItemBoundary = null,
                inlines = inlines,
                listItemAncestors = itemAncestors,
                outermostListItemIdentity = itemAncestors.firstOrNull()?.identity,
                outermostListItemIsLast = itemAncestors.firstOrNull()?.context?.isLast == true,
            )
            itemAncestors.forEach { ancestor ->
                descendantLeavesByListItem.getOrPut(ancestor.identity) { mutableListOf() } += rendered.lastIndex
            }
            itemAncestors.lastOrNull()?.let { nearest ->
                directLeavesByListItem.getOrPut(nearest.identity) { mutableListOf() } += rendered.lastIndex
            }
        }

        compiled.elements().forEach { element ->
            when (element) {
                is FfiViewerElement.BlockStart -> {
                    val context = listContext(element.listContextJson)
                    val identity = if (context != null) nextListItemIdentity++ else null
                    identity?.let { listItemDepths[it] = element.depth.toInt() }
                    stack += Builder(
                        element.nodeType,
                        element.depth.toInt(),
                        context,
                        identity,
                        if (identity == null) null else context,
                    )
                }
                is FfiViewerElement.TextRun -> stack.lastOrNull()?.inlines?.add(ViewerInline.Text(element.text, element.marks))
                is FfiViewerElement.InlineAtom -> stack.lastOrNull()?.inlines?.add(
                    ViewerInline.Atom(element.nodeType, u32(element.docPos), element.attrsJson, element.label)
                )
                is FfiViewerElement.BlockAtom -> appendLeaf(
                    element.nodeType,
                    stack.lastOrNull()?.depth ?: 0,
                    listOf(ViewerInline.Atom(element.nodeType, u32(element.docPos), element.attrsJson, element.label)),
                    stack,
                )
                FfiViewerElement.BlockEnd -> {
                    val builder = stack.removeLastOrNull() ?: return@forEach
                    // Containers are represented by inherited context. Every text block,
                    // including an empty paragraph, remains a leaf for list boundaries.
                    if (builder.nodeType !in CONTAINER_BLOCKS && builder.listItemIdentity == null) {
                        appendLeaf(builder.nodeType, builder.depth, builder.inlines, stack + builder)
                    }
                }
            }
        }
        descendantLeavesByListItem.forEach { (identity, descendantLeaves) ->
            val leaves = directLeavesByListItem[identity]?.takeIf { it.isNotEmpty() } ?: descendantLeaves
            val first = leaves.firstOrNull() ?: return@forEach
            val final = leaves.last()
            leaves.forEach { index ->
                val updatedAncestors = rendered[index].listItemAncestors.map { ancestor ->
                    if (ancestor.identity == identity) {
                        ancestor.copy(
                            nestingDepth = listItemDepths[identity] ?: ancestor.nestingDepth,
                            isFirstRenderableLeaf = index == first,
                            isFinalRenderableLeaf = index == final,
                        )
                    } else ancestor
                }
                val nearest = updatedAncestors.lastOrNull()
                rendered[index] = rendered[index].copy(
                    listItemBoundary = nearest?.let {
                        ViewerListItemBoundary(it.identity, it.nestingDepth, it.isFirstRenderableLeaf, it.isFinalRenderableLeaf)
                    },
                    listItemAncestors = updatedAncestors,
                )
            }
        }
        val fallback = if (rendered.isEmpty() && !compiled.isEmpty()) listOf(ViewerBlock("paragraph", 0, false, null, null, emptyList())) else rendered
        val admittedAttachmentCount = fallback.count { block ->
            block.nodeType == "image" && ViewerImageAttachment.sourceAndDeclaredSize(block) != null
        }
        if (admittedAttachmentCount > ViewerImageAttachment.MAXIMUM_ADMITTED_ATTACHMENTS) {
            throw ProseViewerError.compiler(
                "viewer",
                "ATTACHMENT_LIMIT_EXCEEDED",
                "The document exceeds the maximum admitted image attachment count.",
            )
        }
        return ViewerDocument(
            semanticKey = semanticKey,
            blocks = fallback,
            isEmpty = compiled.isEmpty(),
            retainedBytes = compiled.retainedBytesDecimal().toLongOrNull() ?: 0,
            trailingEmptyTextBlockCount = compiled.trailingEmptyTextBlockCount().toInt(),
        )
    } finally {
        result.destroy()
    }
}

private val CONTAINER_BLOCKS = setOf("doc", "blockquote", "bulletList", "orderedList", "taskList", "listItem")

internal fun listContext(json: String?): ViewerListContext? = runCatching {
    json ?: return@runCatching null
    val value = JSONObject(json)
    val index = if (value.has("index")) u32(value.opt("index")) else 1L
    ViewerListContext(
        value.optBoolean("ordered"),
        index,
        value.optionalString("kind"),
        value.optBoolean("checked"),
        value.optBoolean("isLast"),
    )
}.getOrNull()

/**
 * `optString(key, null)` cannot express an absent value: its fallback is
 * declared non-null, and a present JSON null coerces to the string "null".
 */
internal fun JSONObject.optionalString(key: String): String? =
    if (isNull(key)) null else optString(key)

/** JSON and UniFFI may expose Rust u32 values through different Kotlin number types. */
private fun u32(value: Any?): Long {
    val parsed = value?.toString()?.toLongOrNull()
        ?: throw IllegalArgumentException("Expected an unsigned 32-bit semantic value.")
    require(parsed in 0L..0xFFFF_FFFFL) { "Semantic value is outside Rust u32 range." }
    return parsed
}

private fun mentionPrefix(configJson: String): String? = runCatching {
    val root = JSONObject(configJson)
    root.optJSONObject("mentions")?.optionalString("prefix") ?: root.optionalString("mentionPrefix")
}.getOrNull()

internal fun sha256(value: String): String = MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }
