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
    val index: Int,
    val kind: String?,
    val checked: Boolean,
)

/** Identifies the nearest list item and its first/final renderable leaf. */
internal data class ViewerListItemBoundary(
    val identity: Int,
    val nestingDepth: Int,
    val isFirstRenderableLeaf: Boolean,
    val isFinalRenderableLeaf: Boolean,
)

internal sealed interface ViewerInline {
    data class Text(val text: String, val marks: List<FfiViewerMark>) : ViewerInline
    data class Atom(val nodeType: String, val docPos: Int, val attrsJson: String, val label: String) : ViewerInline
}

internal data class ViewerBlock(
    val nodeType: String,
    val depth: Int,
    val inBlockquote: Boolean,
    val listContext: ViewerListContext?,
    val listItemBoundary: ViewerListItemBoundary?,
    val inlines: List<ViewerInline>,
)

/** Semantic positions live only in [ViewerInline.Atom], never in Android drawing spans. */
internal data class ViewerDocument(
    val semanticKey: String,
    val blocks: List<ViewerBlock>,
    val isEmpty: Boolean,
    val retainedBytes: Long,
    val preparedTheme: PreparedProseTheme? = null,
) {
    fun withPreparedTheme(theme: PreparedProseTheme): ViewerDocument = copy(preparedTheme = theme)
}

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
    val generationIdentity: String by lazy {
        sha256(listOf(compiledCacheKey, themeDigest, if (configuration.collapsesWhenEmpty) "1" else "0", attachmentRevision.toString(), nativeFontRevision.toString(), fontEnvironmentRevision.toString()).joinToString("\u001f"))
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
            val inlines: MutableList<ViewerInline> = mutableListOf(),
        )

        val stack = mutableListOf<Builder>()
        val rendered = mutableListOf<ViewerBlock>()
        val leavesByListItem = mutableMapOf<Int, MutableList<Int>>()
        val listItemDepths = mutableMapOf<Int, Int>()
        var nextListItemIdentity = 0

        fun nearestListContext(builders: List<Builder>): ViewerListContext? = builders.asReversed().firstNotNullOfOrNull { it.listContext }
        fun nearestListItem(builders: List<Builder>): Int? = builders.asReversed().firstNotNullOfOrNull { it.listItemIdentity }
        fun appendLeaf(nodeType: String, depth: Int, inlines: List<ViewerInline>, ancestors: List<Builder>) {
            val listItemIdentity = nearestListItem(ancestors)
            rendered += ViewerBlock(
                nodeType = nodeType,
                depth = depth,
                inBlockquote = ancestors.any { it.nodeType == "blockquote" },
                listContext = nearestListContext(ancestors),
                listItemBoundary = null,
                inlines = inlines,
            )
            listItemIdentity?.let { leavesByListItem.getOrPut(it) { mutableListOf() } += rendered.lastIndex }
        }

        compiled.elements().forEach { element ->
            when (element) {
                is FfiViewerElement.BlockStart -> {
                    val identity = if (element.nodeType == "listItem") nextListItemIdentity++ else null
                    identity?.let { listItemDepths[it] = element.depth.toInt() }
                    stack += Builder(element.nodeType, element.depth.toInt(), listContext(element.listContextJson), identity)
                }
                is FfiViewerElement.TextRun -> stack.lastOrNull()?.inlines?.add(ViewerInline.Text(element.text, element.marks))
                is FfiViewerElement.InlineAtom -> stack.lastOrNull()?.inlines?.add(ViewerInline.Atom(element.nodeType, element.docPos.toInt(), element.attrsJson, element.label))
                is FfiViewerElement.BlockAtom -> appendLeaf(
                    element.nodeType,
                    stack.lastOrNull()?.depth ?: 0,
                    listOf(ViewerInline.Atom(element.nodeType, element.docPos.toInt(), element.attrsJson, element.label)),
                    stack,
                )
                FfiViewerElement.BlockEnd -> {
                    val builder = stack.removeLastOrNull() ?: return@forEach
                    // Containers are represented by inherited context. Every text block,
                    // including an empty paragraph, remains a leaf for list boundaries.
                    if (builder.nodeType !in CONTAINER_BLOCKS) appendLeaf(builder.nodeType, builder.depth, builder.inlines, stack + builder)
                }
            }
        }
        leavesByListItem.forEach { (identity, leaves) ->
            val first = leaves.firstOrNull() ?: return@forEach
            val final = leaves.last()
            leaves.forEach { index ->
                rendered[index] = rendered[index].copy(
                    listItemBoundary = ViewerListItemBoundary(
                        identity = identity,
                        nestingDepth = listItemDepths[identity] ?: 0,
                        isFirstRenderableLeaf = index == first,
                        isFinalRenderableLeaf = index == final,
                    )
                )
            }
        }
        val fallback = if (rendered.isEmpty() && !compiled.isEmpty()) listOf(ViewerBlock("paragraph", 0, false, null, null, emptyList())) else rendered
        return ViewerDocument(semanticKey, fallback, compiled.isEmpty(), compiled.retainedBytesDecimal().toLongOrNull() ?: 0)
    } finally {
        result.destroy()
    }
}

private val CONTAINER_BLOCKS = setOf("doc", "blockquote", "bulletList", "orderedList", "taskList", "listItem")

private fun listContext(json: String?): ViewerListContext? = runCatching {
    json ?: return@runCatching null
    val value = JSONObject(json)
    ViewerListContext(value.optBoolean("ordered"), value.optInt("index", 1), value.optString("kind", null), value.optBoolean("checked"))
}.getOrNull()

private fun mentionPrefix(configJson: String): String? = runCatching {
    val root = JSONObject(configJson)
    root.optJSONObject("mentions")?.optString("prefix", null) ?: root.optString("mentionPrefix", null)
}.getOrNull()

internal fun sha256(value: String): String = MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }
