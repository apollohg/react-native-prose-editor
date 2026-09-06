package com.apollohg.editor

import android.text.style.AbsoluteSizeSpan
import android.text.style.ForegroundColorSpan
import android.text.style.StyleSpan
import android.text.style.TypefaceSpan

internal data class HardwareKeyEventSignature(
    val keyCode: Int,
    val downTime: Long,
    val repeatCount: Int
)

internal data class ParsedRenderPatch(
    val baseDocumentVersion: String?,
    val startIndex: Int,
    val deleteCount: Int,
    val renderBlocks: org.json.JSONArray
)

internal data class RenderReplaceRange(
    val start: Int,
    val endExclusive: Int
)

internal data class ParagraphSpanSnapshot(
    val span: Any,
    val start: Int,
    val end: Int,
    val flags: Int
)

internal data class PatchApplyTrace(
    val applied: Boolean,
    val eligibilityNanos: Long,
    val buildRenderNanos: Long,
    val applyRenderNanos: Long
)

internal data class ImageSelectionRange(
    val start: Int,
    val end: Int
)

internal data class LogicalSelectionSnapshot(
    val scalarAnchor: Int,
    val scalarHead: Int,
    val utf16Anchor: Int,
    val utf16Head: Int,
    val documentVersion: String?
)

internal data class ImageSpanHit(
    val span: BlockImageSpan,
    val start: Int,
    val end: Int
)

internal data class ImageGesture(
    val target: BlockImageSpan,
    val pointerId: Int,
    val downX: Float,
    val downY: Float
)

internal data class LocalTextDrag(
    val scalarFrom: Int,
    val scalarTo: Int,
    val documentVersion: String?,
    val editorId: Long,
)

internal data class NativeTextMutation(
    val scalarFrom: Int,
    val scalarTo: Int,
    val replacementText: String,
    val resultingText: String,
    val replacementStartUtf16: Int,
    val replacementEndUtf16: Int,
    val selectionScalarAnchor: Int?,
    val selectionScalarHead: Int?
)

internal data class NativeTextMutationAfterBlurWindow(
    val editorId: Long,
    val authorizedTextRevision: Long,
    val deadlineMs: Long,
    var didAdoptMutation: Boolean = false
)

internal data class NativeTextMutationAdoptionSuppression(
    val editorId: Long,
    val authorizedTextRevision: Long
)

internal data class ExternalTextCompositionState(
    val sessionId: String,
    var latestText: String,
    val replacementStartUtf16: Int,
    val replacementEndUtf16: Int,
    val startingAuthorizedText: String,
    val startingAuthorizedRenderedText: CharSequence?
)

internal interface TransientComposingTextStyleSpan

internal class TransientComposingSizeSpan(sizePx: Int) :
    AbsoluteSizeSpan(sizePx, false),
    TransientComposingTextStyleSpan

internal class TransientComposingColorSpan(color: Int) :
    ForegroundColorSpan(color),
    TransientComposingTextStyleSpan

internal class TransientComposingTypefaceSpan(family: String) :
    TypefaceSpan(family),
    TransientComposingTextStyleSpan

internal class TransientComposingStyleSpan(style: Int) :
    StyleSpan(style),
    TransientComposingTextStyleSpan

internal data class OptimisticInlineSpan(
    val span: Any,
    val flags: Int
)
