package com.apollohg.editor.prototype

import android.text.Selection
import android.text.SpannableStringBuilder
import android.view.inputmethod.BaseInputConnection
import com.apollohg.editor.EditorV2Adapter
import com.apollohg.editor.EditorV2CallResult
import com.apollohg.editor.PositionBridge
import com.apollohg.editor.UniffiEditorV2Backend
import org.json.JSONArray
import org.json.JSONObject

internal class PrototypeDocumentSession(initialParagraphs: List<String>) : AutoCloseable {
    val editable = SpannableStringBuilder()
    var onChange: (() -> Unit)? = null
    val selectionStart: Int get() = Selection.getSelectionStart(editable).coerceAtLeast(0)
    val selectionEnd: Int get() = Selection.getSelectionEnd(editable).coerceAtLeast(0)
    var committedText: String = ""
        private set
    var lastError: String? = null
        private set
    private var committedJson = ""
    private val adapter: EditorV2Adapter
    private var closed = false
    private var connectionGeneration = 0L
    private var batchDepth = 0
    private var changed = false
    private var commitPending = false

    init {
        val created = UniffiEditorV2Backend.create("""{"initialization":{"type":"localEmpty"}}""", null)
        check(created is EditorV2CallResult.Ok) { "Could not create prototype Rust document: $created" }
        val editorId = JSONObject(created.value).getString("editorId")
        adapter = requireNotNull(EditorV2Adapter.attach(UniffiEditorV2Backend, editorId, false))
        try {
            val paragraphs = JSONArray()
            initialParagraphs.ifEmpty { listOf("") }.flatMap { it.split('\n') }.forEach { text ->
                val content = JSONArray()
                if (text.isNotEmpty()) content.put(JSONObject().put("type", "text").put("text", text))
                paragraphs.put(JSONObject().put("type", "paragraph").put("content", content))
            }
            check(adapter.setContentJson(JSONObject().put("type", "doc").put("content", paragraphs).toString()) != null)
            readCommittedDocument()
            editable.append(committedText)
            Selection.setSelection(editable, 0)
        } catch (error: Throwable) {
            adapter.destroy()
            throw error
        }
    }

    fun committedDocumentJson(): String = committedJson

    fun setSelection(anchor: Int, head: Int) {
        if (closed) return
        val start = boundary(anchor)
        val end = boundary(head)
        if (start == selectionStart && end == selectionEnd) return
        Selection.setSelection(editable, start, end)
        changed(commit = false)
    }

    internal fun acquireConnection(): Long {
        check(!closed) { "Prototype document is closed." }
        connectionGeneration++
        cancelTransient()
        return connectionGeneration
    }

    internal fun isCurrent(generation: Long): Boolean = !closed && generation == connectionGeneration

    internal fun retireConnection(generation: Long) {
        if (!isCurrent(generation)) return
        connectionGeneration++
        cancelTransient()
    }

    internal fun beginBatch(): Boolean {
        if (closed) return false
        batchDepth++
        return true
    }

    internal fun endBatch(): Boolean {
        if (closed || batchDepth == 0) return false
        batchDepth--
        if (batchDepth == 0) flush()
        return true
    }

    internal fun changed(commit: Boolean): Boolean {
        if (closed) return false
        Selection.setSelection(editable, boundary(selectionStart), boundary(selectionEnd))
        changed = true
        commitPending = commitPending || commit
        return batchDepth > 0 || flush()
    }

    private fun flush(): Boolean {
        if (!changed) return true
        changed = false
        val composing = BaseInputConnection.getComposingSpanStart(editable) >= 0
        val shouldCommit = commitPending && !composing
        commitPending = false
        val result = if (shouldCommit) reconcile() else true
        if (!composing && editable.toString() == committedText) syncCoreSelection()
        onChange?.invoke()
        return result
    }

    private fun reconcile(): Boolean {
        val desired = editable.toString()
        if (desired == committedText) return true
        val anchor = selectionStart
        val head = selectionEnd
        return runCatching {
            var prefix = 0
            while (prefix < committedText.length && prefix < desired.length && committedText[prefix] == desired[prefix]) prefix++
            prefix = boundaryIn(committedText, prefix)
            var oldEnd = committedText.length
            var newEnd = desired.length
            while (oldEnd > prefix && newEnd > prefix && committedText[oldEnd - 1] == desired[newEnd - 1]) { oldEnd--; newEnd-- }
            if (oldEnd < committedText.length && oldEnd > 0 && Character.isLowSurrogate(committedText[oldEnd])) { oldEnd++; newEnd++ }
            val from = PositionBridge.utf16ToScalar(prefix, committedText)
            val to = PositionBridge.utf16ToScalar(oldEnd, committedText)
            val inserted = desired.substring(prefix, newEnd)
            if ('\n' !in inserted) {
                check(adapter.replaceTextRange(from, to, inserted) != null) { "Rust rejected text replacement." }
            } else {
                if (to > from) check(adapter.deleteScalarRange(from, to) != null) { "Rust rejected range deletion." }
                var caret = from
                inserted.split('\n').forEachIndexed { index, text ->
                    if (index > 0) { check(adapter.splitBlockAt(caret) != null) { "Rust rejected paragraph split." }; caret++ }
                    if (text.isNotEmpty()) {
                        check(adapter.insertText(text, caret) != null) { "Rust rejected text insertion." }
                        caret += text.codePointCount(0, text.length)
                    }
                }
            }
            readCommittedDocument()
            check(committedText == desired) { "Rust reconciliation differs: expected $desired, got $committedText" }
            Selection.setSelection(editable, boundary(anchor), boundary(head))
            lastError = null
            true
        }.getOrElse {
            lastError = it.message
            readCommittedDocument()
            editable.replace(0, editable.length, committedText)
            BaseInputConnection.removeComposingSpans(editable)
            Selection.setSelection(editable, boundary(anchor), boundary(head))
            false
        }
    }

    private fun syncCoreSelection() {
        adapter.syncSelection(PositionBridge.utf16ToScalar(selectionStart, committedText), PositionBridge.utf16ToScalar(selectionEnd, committedText))
    }

    private fun readCommittedDocument() {
        val result = UniffiEditorV2Backend.getDocumentJson(adapter.editorId)
        check(result is EditorV2CallResult.Ok) { "Could not read prototype Rust document: $result" }
        committedJson = result.value
        val paragraphs = JSONObject(committedJson).getJSONArray("content")
        committedText = (0 until paragraphs.length()).joinToString("\n") { index ->
            val nodes = paragraphs.getJSONObject(index).optJSONArray("content") ?: JSONArray()
            (0 until nodes.length()).joinToString("") { nodes.getJSONObject(it).optString("text", "") }
        }
    }

    private fun cancelTransient() {
        val hadChanges = editable.toString() != committedText || BaseInputConnection.getComposingSpanStart(editable) >= 0
        val anchor = selectionStart
        val head = selectionEnd
        batchDepth = 0
        changed = false
        commitPending = false
        BaseInputConnection.removeComposingSpans(editable)
        if (editable.toString() != committedText) editable.replace(0, editable.length, committedText)
        Selection.setSelection(editable, boundary(anchor), boundary(head))
        if (hadChanges) onChange?.invoke()
    }

    internal fun boundary(offset: Int, forward: Boolean = false): Int = boundaryIn(editable, offset, forward)

    private fun boundaryIn(text: CharSequence, offset: Int, forward: Boolean = false): Int {
        val clamped = offset.coerceIn(0, text.length)
        return if (clamped > 0 && clamped < text.length && Character.isHighSurrogate(text[clamped - 1]) && Character.isLowSurrogate(text[clamped])) clamped + if (forward) 1 else -1 else clamped
    }

    override fun close() {
        if (closed) return
        closed = true
        connectionGeneration++
        cancelTransient()
        onChange = null
        adapter.destroy()
    }
}
