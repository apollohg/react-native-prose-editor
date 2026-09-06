package com.apollohg.editor

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.view.accessibility.AccessibilityNodeInfo

internal fun EditorEditText.isMutatingContextMenuItem(id: Int): Boolean =
    id == android.R.id.paste ||
        id == android.R.id.pasteAsPlainText ||
        id == android.R.id.cut

internal fun EditorEditText.prepareForExternalInteractionMutation(): Boolean =
    commitExternalTextCompositionBeforeInteractionIfNeeded() &&
        prepareForExternalEditorUpdate()

internal fun EditorEditText.handlePaste(plainTextOnly: Boolean) {
    val selectionRange = normalizedUtf16SelectionRange()
    if (selectionRange != null && isCollapsedAtomBoundarySelection(selectionRange.first, selectionRange.second)) {
        return
    }
    if (editorId == 0L) {
        // Dev mode: default paste behavior.
        baseTextContextMenuItem(
            if (plainTextOnly) android.R.id.pasteAsPlainText else android.R.id.paste
        )
        return
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) return
    if (!prepareForExternalInteractionMutation()) return

    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
        ?: return
    val clip = clipboard.primaryClip ?: return
    if (clip.itemCount == 0) return

    val item = clip.getItemAt(0)

    // Try HTML first for rich paste.
    val htmlText = item.htmlText
    if (!plainTextOnly && htmlText != null) {
        pasteHTML(htmlText)
        return
    }

    // Fallback to plain text.
    val plainText = item.text?.toString() ?: item.coerceToText(context)?.toString()
    if (plainText != null) {
        pastePlainText(plainText)
    }
}

internal fun EditorEditText.handleCut() {
    if (editorId == 0L) {
        baseTextContextMenuItem(android.R.id.cut)
        return
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) return
    if (!prepareForExternalInteractionMutation()) return

    val currentText = text?.toString() ?: return
    val (selectionStart, selectionEnd) = normalizedUtf16SelectionRange(currentText) ?: return
    if (selectionStart == selectionEnd) return

    val (utf16Start, utf16End) = PositionBridge.snapRangeToScalarBoundaries(
        selectionStart,
        selectionEnd,
        currentText
    )
    if (utf16Start >= utf16End) return

    val selectedText = currentText.substring(utf16Start, utf16End)
    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
    clipboard?.setPrimaryClip(ClipData.newPlainText(null, selectedText))

    val scalarStart = PositionBridge.utf16ToScalar(utf16Start, currentText)
    val scalarEnd = PositionBridge.utf16ToScalar(utf16End, currentText)
    deleteRangeInRust(scalarStart, scalarEnd)
}

internal fun EditorEditText.handleAccessibilitySetText(arguments: android.os.Bundle?): Boolean {
    val replacement = arguments
        ?.getCharSequence(
            android.view.accessibility.AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE
        )
        ?.toString()
        ?: return false
    if (editorId == 0L) {
        return baseAccessibilityAction(
            android.view.accessibility.AccessibilityNodeInfo.ACTION_SET_TEXT,
            arguments
        )
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) return false
    if (!prepareForExternalInteractionMutation()) return false

    val currentText = text?.toString() ?: return false
    val scalarStart = 0
    val scalarEnd = currentText.codePointCount(0, currentText.length)
    insertPlainTextRangeInRust(scalarStart, scalarEnd, replacement)
    return true
}
