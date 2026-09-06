package com.apollohg.editor

import java.util.concurrent.atomic.AtomicLong

internal const val ATOM_NATIVE_ID_PREFIX = "prose-atom:"
internal val nextNativeEditorErrorCallbackToken = AtomicLong(0)

internal enum class NativeEditorOutsideTapDecision {
    IGNORE,
    PRESERVE_FOCUS,
    OUTSIDE_EDITOR
}

internal data class NativeEditorOutsideTapRouteTestState(
    val isRegistered: Boolean,
    val hasCallbackReconciler: Boolean
)

internal enum class PendingEditorUpdateApplyOutcome {
    APPLIED,
    RETRYABLE_DEFERRED,
    PERMANENTLY_REJECTED
}

internal enum class PendingEditorUpdateKind {
    ORDINARY,
    RESET
}
