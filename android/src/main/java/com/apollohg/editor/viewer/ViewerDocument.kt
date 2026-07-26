package com.apollohg.editor.viewer

import com.apollohg.editor.ProseViewerConfiguration
import com.apollohg.editor.ProseViewerError
import com.apollohg.editor.ProseViewerSource
import java.security.MessageDigest
import uniffi.editor_core.FfiViewerCompileRequest
import uniffi.editor_core.FfiViewerElement
import uniffi.editor_core.FfiViewerSourceKind
import uniffi.editor_core.viewerCompile

/** Immutable, width-independent projection of the Rust compiled document. */
internal data class ViewerDocument(
    val semanticKey: String,
    val paragraphs: List<String>,
    val isEmpty: Boolean,
    val retainedBytes: Long,
)

internal data class ProseViewerRequest(
    val source: ProseViewerSource,
    val configuration: ProseViewerConfiguration,
    val nativeFontRevision: Long = 0,
    val fontEnvironmentRevision: Long = 0,
    val attachmentRevision: Long = 0,
) {
    val compiledCacheKey: String by lazy {
        sha256(
            listOf(
                source.value,
                configuration.configJson,
                configuration.imagePolicyJson.orEmpty(),
                if (configuration.imagesEnabled) "1" else "0",
                mentionPrefix(configuration.configJson).orEmpty(),
                source.kind,
            ).joinToString("\u001f")
        )
    }

    val themeDigest: String by lazy { sha256(configuration.themeJson.orEmpty()) }

    val generationIdentity: String by lazy {
        sha256(
            listOf(
                compiledCacheKey,
                themeDigest,
                if (configuration.collapsesWhenEmpty) "1" else "0",
                attachmentRevision.toString(),
                nativeFontRevision.toString(),
                fontEnvironmentRevision.toString(),
            ).joinToString("\u001f")
        )
    }

    val mentionPrefix: String?
        get() = mentionPrefix(configuration.configJson)
}

internal typealias DocumentCompiler = (ProseViewerRequest) -> ViewerDocument

internal fun compileWithRust(request: ProseViewerRequest): ViewerDocument {
    val result = viewerCompile(
        FfiViewerCompileRequest(
            sourceKind = if (request.source is ProseViewerSource.Html) {
                FfiViewerSourceKind.HTML
            } else {
                FfiViewerSourceKind.JSON
            },
            source = request.source.value,
            configJson = request.configuration.configJson,
            imagesEnabled = request.configuration.imagesEnabled,
            mentionPrefix = request.mentionPrefix,
        )
    )
    result.error?.let { error ->
        throw ProseViewerError.compiler(error.domain, error.code, error.message)
    }
    val compiled = result.value ?: throw ProseViewerError.compiler(
        domain = "viewer",
        code = "MISSING_COMPILED_DOCUMENT",
        message = "The compiler returned neither a document nor an error.",
    )
    val semanticKey = compiled.semanticKey()
    if (!semanticKey.matches(Regex("[0-9a-f]{64}"))) {
        throw ProseViewerError.compiler(
            domain = "viewer",
            code = "INVALID_SEMANTIC_KEY",
            message = "The compiler returned an invalid semantic key.",
        )
    }

    val paragraphs = mutableListOf<String>()
    val text = StringBuilder()
    var inBlock = false
    compiled.elements().forEach { element ->
        when (element) {
            is FfiViewerElement.BlockStart -> {
                if (inBlock) {
                    paragraphs += text.toString()
                    text.clear()
                }
                inBlock = true
            }
            is FfiViewerElement.TextRun -> text.append(element.text)
            is FfiViewerElement.InlineAtom -> text.append(element.label)
            is FfiViewerElement.BlockAtom -> text.append(element.label)
            FfiViewerElement.BlockEnd -> {
                paragraphs += text.toString()
                text.clear()
                inBlock = false
            }
        }
    }
    if (inBlock || (text.isNotEmpty() && paragraphs.isEmpty())) paragraphs += text.toString()
    return ViewerDocument(
        semanticKey = semanticKey,
        paragraphs = if (paragraphs.isEmpty()) listOf("") else paragraphs.toList(),
        isEmpty = compiled.isEmpty(),
        retainedBytes = compiled.retainedBytesDecimal().toLongOrNull() ?: 0,
    )
}

private fun mentionPrefix(configJson: String): String? = runCatching {
    val mentions = org.json.JSONObject(configJson).optJSONObject("mentions")
    mentions?.optString("prefix", null)
        ?: org.json.JSONObject(configJson).optString("mentionPrefix", null)
}.getOrNull()

internal fun sha256(value: String): String = MessageDigest.getInstance("SHA-256")
    .digest(value.toByteArray(Charsets.UTF_8))
    .joinToString("") { "%02x".format(it) }
