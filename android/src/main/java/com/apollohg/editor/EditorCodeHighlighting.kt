package com.apollohg.editor

import android.graphics.Color
import android.graphics.Typeface
import android.text.Spanned
import android.text.TextPaint
import android.text.style.MetricAffectingSpan
import org.json.JSONObject

data class NativeCodeHighlightingConfig(val provider: String, val theme: String) {
    companion object {
        fun fromJson(json: JSONObject?): NativeCodeHighlightingConfig? = json?.let {
            val provider = it.optNullableString("provider")
            val theme = it.optNullableString("theme")
            require(!provider.isNullOrBlank() && !theme.isNullOrBlank()) { "Code highlighting requires provider and theme." }
            NativeCodeHighlightingConfig(provider, theme)
        }
    }
}

internal class CodeBlockMetadataSpan(val language: String?)

internal class EditorCodeHighlightSpan(private val range: CodeHighlightRange) : MetricAffectingSpan() {
    override fun updateMeasureState(paint: TextPaint) {
        val traits = (if (range.fontStyle and 1 != 0) Typeface.BOLD else 0) or (if (range.fontStyle and 4 != 0) Typeface.ITALIC else 0)
        if (traits != 0) paint.typeface = Typeface.create(paint.typeface, (paint.typeface?.style ?: 0) or traits)
    }
    override fun updateDrawState(paint: TextPaint) {
        updateMeasureState(paint)
        val rgba = range.color
        paint.color = Color.argb((rgba and 255).toInt(), ((rgba shr 24) and 255).toInt(), ((rgba shr 16) and 255).toInt(), ((rgba shr 8) and 255).toInt())
        if (range.fontStyle and 2 != 0) paint.isUnderlineText = true
    }
}

internal fun EditorEditText.setCodeHighlighting(configuration: NativeCodeHighlightingConfig?) {
    configuration?.let { CodeHighlightingRegistry.provider(it.provider) }
    if (codeHighlightingConfiguration == configuration) return
    codeHighlightingConfiguration = configuration
    refreshCodeHighlighting()
}

internal fun EditorEditText.refreshCodeHighlighting() {
    codeHighlightingSession.cancel()
    val content = text ?: return
    content.getSpans(0, content.length, EditorCodeHighlightSpan::class.java).forEach(content::removeSpan)
    val config = codeHighlightingConfiguration ?: run { invalidate(); return }
    if (!isAttachedToWindow) return
    val snapshot = content.toString()
    val blocks = content.getSpans(0, content.length, CodeBlockMetadataSpan::class.java).mapNotNull {
        val start = content.getSpanStart(it)
        val end = content.getSpanEnd(it)
        if (start < 0 || end <= start) null else CodeHighlightBlock(start, snapshot.substring(start, end), it.language)
    }
    if (blocks.isEmpty()) return
    val revision = lastAuthorizedTextRevision
    codeHighlightingSession.update(config.provider, config.theme, blocks) { result ->
        if (!isAttachedToWindow || config != codeHighlightingConfiguration || revision != lastAuthorizedTextRevision || text?.toString() != snapshot) return@update
        result.onSuccess { highlighted ->
            val current = text ?: return@onSuccess
            current.getSpans(0, current.length, EditorCodeHighlightSpan::class.java).forEach(current::removeSpan)
            highlighted.forEach { block -> block.ranges.forEach { range ->
                current.setSpan(EditorCodeHighlightSpan(range), block.block.start + range.start, block.block.start + range.start + range.length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            } }
            requestLayout()
            invalidate()
        }.onFailure { android.util.Log.w("NativeEditor", "Code highlighting failed: ${it.message}") }
    }
}
