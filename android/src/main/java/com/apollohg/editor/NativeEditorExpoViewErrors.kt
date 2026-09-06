package com.apollohg.editor

import com.apollohg.editor.NativeEditorExpoView.PendingEditorErrorEvent
import com.apollohg.editor.NativeEditorExpoView.EditorErrorBinding
import android.os.Looper

internal fun NativeEditorExpoView.bindEditorErrorCallbackIfLive(viewToken: Long) {
    if (!isAttachedToNativeWindow || richTextView.editorId != viewToken ||
        richTextView.editorEditText.editorId != viewToken
    ) return
    val adapter = EditorV2Registry.adapterForViewToken(viewToken) ?: return
    val editorId = publicHandleForViewToken(viewToken) ?: return
    val existing = editorErrorBinding
    if (existing?.adapter === adapter && existing.viewToken == viewToken &&
        existing.editorId == editorId
    ) return
    clearEditorErrorBinding("claim")
    val binding = EditorErrorBinding(
        adapter = adapter,
        editorId = editorId,
        viewToken = viewToken,
        callbackToken = nextNativeEditorErrorCallbackToken.incrementAndGet(),
        generation = ++nextEditorErrorBindingGeneration,
    )
    editorErrorBinding = binding
    adapter.bindAutonomousErrorOwner(
        binding.callbackToken,
        callback = { error -> queueEditorError(binding, error) },
        onReleased = { releaseEditorErrorBinding(binding) },
    )
}

internal fun NativeEditorExpoView.releaseEditorErrorBinding(binding: EditorErrorBinding) {
    if (Looper.myLooper() != Looper.getMainLooper()) {
        mainHandler.post { releaseEditorErrorBinding(binding) }
        return
    }
    if (editorErrorBinding != binding) return
    editorErrorBinding = null
    nextEditorErrorBindingGeneration += 1
    clearPendingEditorErrorDispatchQueue("ownerReleased")
}

internal fun NativeEditorExpoView.clearEditorErrorBinding(reason: String) {
    val binding = editorErrorBinding
    editorErrorBinding = null
    nextEditorErrorBindingGeneration += 1
    binding?.adapter?.clearAutonomousErrorOwner(binding.callbackToken)
    clearPendingEditorErrorDispatchQueue(reason)
}

internal fun NativeEditorExpoView.queueEditorError(binding: EditorErrorBinding, error: EditorV2Error) {
    val event = PendingEditorErrorEvent(
        adapter = binding.adapter,
        editorId = binding.editorId,
        viewToken = binding.viewToken,
        callbackToken = binding.callbackToken,
        bindingGeneration = binding.generation,
        error = error,
    )
    if (Looper.myLooper() == Looper.getMainLooper()) {
        enqueueEditorError(event)
    } else {
        mainHandler.post { enqueueEditorError(event) }
    }
}

internal fun NativeEditorExpoView.enqueueEditorError(event: PendingEditorErrorEvent) {
    if (!isLiveEditorErrorBinding(event)) return
    pendingEditorErrorEvents.addLast(event)
    if (!pendingEditorErrorDispatchScheduled) {
        pendingEditorErrorDispatchScheduled = true
        val generation = ++pendingEditorErrorDispatchGeneration
        mainHandler.post {
            if (generation != pendingEditorErrorDispatchGeneration) return@post
            pendingEditorErrorDispatchScheduled = false
            drainPendingEditorErrorEvents()
        }
    }
}

internal fun NativeEditorExpoView.drainPendingEditorErrorEvents() {
    while (pendingEditorErrorEvents.isNotEmpty()) {
        // Remove before dispatch so a reentrant lifecycle transition cannot deliver it twice.
        val event = pendingEditorErrorEvents.removeFirst()
        if (!isLiveEditorErrorBinding(event)) continue
        val payload = mapOf<String, Any>(
            "editorId" to event.editorId,
            "error" to event.error.toJSMap(),
        )
        dispatchEditorError(payload)
    }
}

internal fun NativeEditorExpoView.dispatchEditorError(payload: Map<String, Any>) {
    onEditorErrorForTesting?.let { callback ->
        callback(payload)
        return
    }
    if (!appContext.hasActiveReactInstance) return
    onEditorError(payload)
}

internal fun NativeEditorExpoView.isLiveEditorErrorBinding(event: PendingEditorErrorEvent): Boolean {
    val binding = editorErrorBinding ?: return false
    return isAttachedToNativeWindow &&
        !NativeEditorViewRegistry.isDestroyed(event.viewToken) &&
        binding.adapter === event.adapter &&
        binding.editorId == event.editorId &&
        binding.viewToken == event.viewToken &&
        binding.callbackToken == event.callbackToken &&
        binding.generation == event.bindingGeneration &&
        richTextView.editorId == event.viewToken &&
        richTextView.editorEditText.editorId == event.viewToken &&
        publicHandleForViewToken(event.viewToken) == event.editorId &&
        EditorV2Registry.adapterForViewToken(event.viewToken) === event.adapter &&
        event.adapter.ownsAutonomousErrorOwner(event.callbackToken)
}

internal fun NativeEditorExpoView.clearPendingEditorErrorDispatchQueue(reason: String) {
    val clearedCount = pendingEditorErrorEvents.size
    pendingEditorErrorEvents.clear()
    pendingEditorErrorDispatchScheduled = false
    pendingEditorErrorDispatchGeneration += 1
    if (clearedCount > 0) {
        richTextView.editorEditText.recordImeTraceForTesting(
            "nativeViewEditorErrorQueueCleared",
            "reason=$reason count=$clearedCount"
        )
    }
}

internal fun NativeEditorExpoView.pendingEditorErrorEventCountForTestingImpl(): Int = pendingEditorErrorEvents.size

internal fun NativeEditorExpoView.editorErrorCallbackTokenForTestingImpl(): Long? = editorErrorBinding?.callbackToken
