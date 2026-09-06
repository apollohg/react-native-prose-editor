package com.apollohg.editor

data class CodeHighlightRange(val start: Int, val length: Int, val color: Long, val fontStyle: Int)

interface CodeHighlightingProvider {
    val id: String
    val version: Int
    fun highlight(text: String, language: String?, theme: String): List<CodeHighlightRange>
}

object CodeHighlightingRegistry {
    private val providers = mutableMapOf<String, CodeHighlightingProvider>()

    @Synchronized
    fun register(provider: CodeHighlightingProvider) {
        require(provider.id.isNotEmpty() && provider.version == 1) {
            "Code highlighting provider must have an ID and support version 1"
        }
        providers[provider.id] = provider
    }

    @Synchronized
    fun provider(id: String): CodeHighlightingProvider = requireNotNull(providers[id]) {
        "Code highlighting provider '$id' is unavailable. Install and import its native package, then rebuild the app."
    }

    internal fun validRanges(text: String, ranges: List<CodeHighlightRange>): Boolean {
        fun isBoundary(offset: Int): Boolean = offset == 0 || offset == text.length ||
            !Character.isHighSurrogate(text[offset - 1]) || !Character.isLowSurrogate(text[offset])
        var end = 0
        for (range in ranges) {
            if (range.start < end || range.length <= 0 || range.start > text.length ||
                range.length > text.length - range.start || range.color !in 0L..0xffffffffL ||
                range.fontStyle !in 0..7 || !isBoundary(range.start) || !isBoundary(range.start + range.length)) return false
            end = range.start + range.length
        }
        return true
    }
}

internal data class CodeHighlightBlock(val start: Int, val text: String, val language: String?)
internal data class HighlightedCodeBlock(val block: CodeHighlightBlock, val ranges: List<CodeHighlightRange>)

internal class CodeHighlightingSession {
    private data class Request(
        val generation: Long,
        val provider: CodeHighlightingProvider,
        val theme: String,
        val blocks: List<CodeHighlightBlock>,
        val completion: (Result<List<HighlightedCodeBlock>>) -> Unit,
    )

    companion object {
        private val executor = java.util.concurrent.Executors.newSingleThreadExecutor { task ->
            Thread(task, "editor.code-highlighting").apply { isDaemon = true }
        }
    }

    private val lock = Any()
    private val main = android.os.Handler(android.os.Looper.getMainLooper())
    private var generation = 0L
    private var pending: Request? = null
    private var running = false

    fun cancel() {
        check(android.os.Looper.myLooper() == android.os.Looper.getMainLooper()) {
            "Highlighting sessions must be updated on the main thread"
        }
        synchronized(lock) {
            generation++
            pending = null
        }
    }

    fun update(provider: String, theme: String, blocks: List<CodeHighlightBlock>,
               completion: (Result<List<HighlightedCodeBlock>>) -> Unit) {
        cancel()
        val resolved = CodeHighlightingRegistry.provider(provider)
        val schedule = synchronized(lock) {
            pending = Request(generation, resolved, theme, blocks.toList(), completion)
            (!running).also { running = true }
        }
        if (schedule) executor.execute { drain() }
    }

    private fun current(value: Long) = synchronized(lock) { generation == value }

    private fun takeRequest(): Request? = synchronized(lock) {
        pending.also {
            pending = null
            if (it == null) running = false
        }
    }

    private fun scheduleNext() {
        val schedule = synchronized(lock) {
            (pending != null).also { running = it }
        }
        if (schedule) executor.execute { drain() }
    }

    private fun drain() {
        val request = takeRequest() ?: return
        try {
            val result = runCatching {
                val output = mutableListOf<HighlightedCodeBlock>()
                for (block in request.blocks) {
                    if (!current(request.generation)) break
                    val ranges = request.provider.highlight(block.text, block.language, request.theme).toList()
                    require(CodeHighlightingRegistry.validRanges(block.text, ranges)) {
                        "Code highlighting provider returned invalid UTF-16 ranges"
                    }
                    output.add(HighlightedCodeBlock(block, ranges))
                }
                output.toList()
            }
            if (current(request.generation)) main.post {
                if (current(request.generation)) request.completion(result)
            }
        } finally {
            scheduleNext()
        }
    }
}
