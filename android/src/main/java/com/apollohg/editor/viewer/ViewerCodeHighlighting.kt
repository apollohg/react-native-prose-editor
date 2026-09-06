package com.apollohg.editor.viewer

import com.apollohg.editor.CodeHighlightBlock
import com.apollohg.editor.CodeHighlightRange
import com.apollohg.editor.CodeHighlightingSession
import com.apollohg.editor.NativeCodeHighlightingConfig

internal object ViewerCodeHighlightCache {
    private val values = LinkedHashMap<String, List<CodeHighlightRange>>(16, .75f, true)
    private var bytes = 0L
    internal fun key(config: NativeCodeHighlightingConfig, block: CodeHighlightBlock) = sha256(org.json.JSONArray(listOf(config.provider, config.theme, block.language, block.text)).toString())

    fun get(config: NativeCodeHighlightingConfig, block: CodeHighlightBlock): List<CodeHighlightRange>? {
        val key = key(config, block)
        return synchronized(values) { values[key] }
    }

    fun store(config: NativeCodeHighlightingConfig, block: CodeHighlightBlock, ranges: List<CodeHighlightRange>) {
        val key = key(config, block)
        val copy = ranges.toList()
        synchronized(values) {
            values.put(key, copy)?.let { bytes -= 128L + it.size * 32L }
            bytes += 128L + copy.size * 32L
            while (bytes > 8L * 1024 * 1024 || values.size > 512) {
                val oldest = values.entries.first()
                bytes -= 128L + oldest.value.size * 32L
                values.remove(oldest.key)
            }
        }
    }
}

internal class ViewerCodeHighlighting(private val view: PreparedProseDrawingView) {
    private val session = CodeHighlightingSession()
    private var semanticGeneration: String? = null
    private val completed = mutableSetOf<String>()
    private var notifiedArtifact = java.lang.ref.WeakReference<PreparedProseLayout>(null)
    fun cancel() = session.cancel()
    fun update() {
        session.cancel()
        val artifact = view.preparedLayout ?: return
        val config = artifact.codeHighlighting ?: return
        if (semanticGeneration != artifact.key.semanticGenerationIdentity) { semanticGeneration = artifact.key.semanticGenerationIdentity; completed.clear() }
        if (!view.isAttachedToWindow) return
        val hasUnconsumedCache = artifact.codeHighlightBlocks.any {
            ViewerCodeHighlightCache.key(config, it) !in artifact.highlightedCodeKeys && ViewerCodeHighlightCache.get(config, it) != null
        }
        if (hasUnconsumedCache && notifiedArtifact.get() !== artifact) {
            notifiedArtifact = java.lang.ref.WeakReference(artifact)
            view.onCodeHighlightsReady?.invoke()
            return
        }
        val pending = artifact.codeHighlightBlocks.filter { ViewerCodeHighlightCache.get(config, it) == null && ViewerCodeHighlightCache.key(config, it) !in completed }
        if (pending.isEmpty()) return
        session.update(config.provider, config.theme, pending) { result ->
            if (!view.isAttachedToWindow || view.preparedLayout !== artifact) return@update
            result.onSuccess { blocks ->
                blocks.forEach { ViewerCodeHighlightCache.store(config, it.block, it.ranges); completed += ViewerCodeHighlightCache.key(config, it.block) }
                view.onCodeHighlightsReady?.invoke()
            }.onFailure { android.util.Log.w("NativeProseViewer", "Code highlighting failed: ${it.message}") }
        }
    }
}
