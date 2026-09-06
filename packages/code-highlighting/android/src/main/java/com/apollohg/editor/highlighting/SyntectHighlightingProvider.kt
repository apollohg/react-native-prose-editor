package com.apollohg.editor.highlighting

import com.apollohg.editor.CodeHighlightingProvider
import com.apollohg.editor.CodeHighlightRange
import uniffi.native_editor_highlighting.highlightCode

internal class SyntectHighlightingProvider : CodeHighlightingProvider {
    override val id = "syntect"
    override val version = 1

    override fun highlight(text: String, language: String?, theme: String): List<CodeHighlightRange> =
        highlightCode(text, language, theme).map {
            CodeHighlightRange(it.start.toInt(), it.length.toInt(), it.color.toLong(), it.fontStyle.toInt())
        }
}
