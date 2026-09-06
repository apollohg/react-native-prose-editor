package com.apollohg.editor

import com.apollohg.editor.EditorEditText.ApplyUpdateTrace
import com.apollohg.editor.EditorEditText.Companion.IME_TRACE_LOG_TAG
import com.apollohg.editor.EditorEditText.Companion.IME_TRACE_LIMIT_FOR_TESTING
import android.os.SystemClock
import android.util.Log

internal fun EditorEditText.ownsNativeBindingImpl(adapter: EditorV2Adapter): Boolean =
    adapter.isNativeBindingOwner(nativeBindingToken)

internal fun EditorEditText.lastRenderAppliedPatchImpl(): Boolean = lastRenderAppliedPatchForTesting

internal fun EditorEditText.lastApplyUpdateTraceImpl(): ApplyUpdateTrace? = lastApplyUpdateTraceForTesting

internal fun EditorEditText.hasDeferredRustUpdateApplicationForTestingImpl(): Boolean = deferredRustUpdateJSON != null

internal fun EditorEditText.inputConnectionGenerationForTestingImpl(): Long = inputConnectionGeneration

internal fun EditorEditText.authorizedTextForTestingImpl(): String = lastAuthorizedText

internal fun EditorEditText.applyRustUpdateJSONForTestingImpl(updateJSON: String) {
    applyRustUpdateJSON(updateJSON)
}

internal fun EditorEditText.recordImeTraceForTestingImpl(event: String, details: String = "") {
    if (imeTraceForTesting.size >= IME_TRACE_LIMIT_FOR_TESTING) {
        imeTraceForTesting.removeFirst()
    }
    imeTraceForTesting.addLast(
        if (details.isEmpty()) event else "$event:$details"
    )
    if (Log.isLoggable(IME_TRACE_LOG_TAG, Log.VERBOSE)) {
        val now = SystemClock.uptimeMillis()
        val deltaMs = if (lastImeTraceUptimeMs == 0L) 0L else now - lastImeTraceUptimeMs
        lastImeTraceUptimeMs = now
        imeTraceSequence += 1L
        val textLength = text?.length ?: -1
        val selection = "${selectionStart}..${selectionEnd}"
        val composingRange = "${composingReplacementStartUtf16 ?: -1}.." +
            "${composingReplacementEndUtf16 ?: -1}"
        val composingRevisionMatches =
            composingReplacementAuthorizedTextRevision == lastAuthorizedTextRevision
        val message = buildString {
            append("#").append(imeTraceSequence)
            append(" +").append(deltaMs).append("ms ")
            append(event)
            if (details.isNotEmpty()) {
                append(" ").append(details)
            }
            append(" editor=").append(editorId)
            append(" gen=").append(inputConnectionGeneration)
            append(" activeIc=").append(activeInputConnection != null)
            append(" focus=").append(hasFocus())
            append(" applying=").append(isApplyingRustState)
            append(" editable=").append(isEditable)
            append(" textLen=").append(textLength)
            append(" authLen=").append(lastAuthorizedText.length)
            append(" sel=").append(selection)
            append(" composingTextLen=").append(composingText?.length ?: -1)
            append(" composingRange=").append(composingRange)
            append(" composingRevOk=").append(composingRevisionMatches)
            append(" invalidComp=").append(didInvalidateCompositionReplacementRange)
            append(" deferredRustUpdate=").append(deferredRustUpdateJSON != null)
            append(" scroll=").append(scrollX).append(",").append(scrollY)
        }
        Log.v(IME_TRACE_LOG_TAG, message)
    }
}

internal fun EditorEditText.clearImeTraceForTestingImpl() {
    imeTraceForTesting.clear()
    imeTraceSequence = 0L
    lastImeTraceUptimeMs = 0L
}

internal fun EditorEditText.imeTraceSnapshotForTestingImpl(): List<String> =
    imeTraceForTesting.toList()

internal fun EditorEditText.nanosToMicros(nanos: Long): Long = nanos / 1_000L
