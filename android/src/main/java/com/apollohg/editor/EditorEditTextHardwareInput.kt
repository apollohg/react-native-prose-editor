package com.apollohg.editor

import com.apollohg.editor.EditorEditText.Companion.RECENT_HANDLED_HARDWARE_KEY_DOWN_WINDOW_MS
import android.os.SystemClock
import android.view.KeyEvent

internal fun EditorEditText.handleCompositionKeyEventImpl(event: KeyEvent, applyBaseEvent: () -> Boolean): Boolean {
    val inputConnection = activeInputConnection ?: return false
    if (!inputConnection.hasPendingComposition()) return false
    if (!isCompositionKeyCode(event.keyCode)) return false
    if (event.action == KeyEvent.ACTION_DOWN) {
        val signature = hardwareKeyEventSignature(event)
        if (
            lastHandledHardwareKeySignature == signature ||
            didRecentlyHandleHardwareKeyDown(signature)
        ) {
            return true
        }
        markHandledHardwareKeyDown(signature)
        runWithTransientInputMutationGuard {
            when (event.keyCode) {
                KeyEvent.KEYCODE_DEL,
                KeyEvent.KEYCODE_FORWARD_DEL -> inputConnection.deleteTransientTextForHardwareKeyEvent(event)
                else -> applyBaseEvent()
            }
        }
        inputConnection.refreshComposingTextFromEditableForEditor()
        return true
    }
    if (event.action == KeyEvent.ACTION_UP) {
        if (lastHandledHardwareKeySignature?.let {
                it.keyCode == event.keyCode && it.downTime == event.downTime
            } == true) {
            lastHandledHardwareKeySignature = null
        }
        return true
    }
    return false
}

internal fun EditorEditText.isCompositionKeyCode(keyCode: Int): Boolean =
    when (keyCode) {
        KeyEvent.KEYCODE_DEL,
        KeyEvent.KEYCODE_FORWARD_DEL,
        KeyEvent.KEYCODE_ENTER,
        KeyEvent.KEYCODE_NUMPAD_ENTER,
        KeyEvent.KEYCODE_TAB -> true
        else -> false
    }

internal fun EditorEditText.handleHardwareKeyDownImpl(keyCode: Int, shiftPressed: Boolean): Boolean {
    if (!isEditable || isApplyingRustState) return false
    return when (keyCode) {
        KeyEvent.KEYCODE_DEL -> {
            handleBackspace()
            true
        }
        KeyEvent.KEYCODE_FORWARD_DEL -> {
            handleForwardDelete()
            true
        }
        KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> {
            if (shiftPressed) {
                handleHardBreak()
            } else {
                handleReturnKey()
            }
            true
        }
        KeyEvent.KEYCODE_TAB -> handleTab(shiftPressed)
        else -> false
    }
}

internal fun EditorEditText.isSupportedHardwareMutationKey(keyCode: Int): Boolean =
    when (keyCode) {
        KeyEvent.KEYCODE_DEL,
        KeyEvent.KEYCODE_FORWARD_DEL,
        KeyEvent.KEYCODE_ENTER,
        KeyEvent.KEYCODE_NUMPAD_ENTER,
        KeyEvent.KEYCODE_TAB -> true
        else -> false
    }

internal fun EditorEditText.isReadOnlyTextMutationKeyEventImpl(event: KeyEvent): Boolean {
    if (isSupportedHardwareMutationKey(event.keyCode) ||
        event.keyCode == KeyEvent.KEYCODE_FORWARD_DEL
    ) {
        return true
    }
    if (event.keyCode == KeyEvent.KEYCODE_INSERT && event.isShiftPressed) {
        return true
    }
    if (event.isCtrlPressed || event.isMetaPressed) {
        return when (event.keyCode) {
            KeyEvent.KEYCODE_V,
            KeyEvent.KEYCODE_X,
            KeyEvent.KEYCODE_Z,
            KeyEvent.KEYCODE_Y -> true
            else -> false
        }
    }
    if (!keyEventCharacters(event).isNullOrEmpty()) return true
    return event.unicodeChar != 0
}

internal fun EditorEditText.handleHardwareKeyEventImpl(event: KeyEvent?): Boolean {
    if (event == null || !isEditable || isApplyingRustState) return false

    return when (event.action) {
        KeyEvent.ACTION_DOWN -> {
            if (!isSupportedHardwareMutationKey(event.keyCode)) return false

            val signature = hardwareKeyEventSignature(event)
            if (
                lastHandledHardwareKeySignature == signature ||
                didRecentlyHandleHardwareKeyDown(signature)
            ) {
                return true
            }

            if (handleHardwareKeyDown(event.keyCode, event.isShiftPressed)) {
                markHandledHardwareKeyDown(signature)
                true
            } else {
                false
            }
        }

        KeyEvent.ACTION_UP -> {
            if (lastHandledHardwareKeySignature?.let {
                    it.keyCode == event.keyCode && it.downTime == event.downTime
                } == true) {
                lastHandledHardwareKeySignature = null
                true
            } else {
                false
            }
        }

        else -> false
    }
}

internal fun EditorEditText.handlePrintableHardwareKeyEventImpl(
    event: KeyEvent,
    applyBaseEvent: () -> Boolean
): Boolean {
    if (!isEditable || isApplyingRustState || !isPrintableHardwareMutationKey(event)) {
        return false
    }
    val signature = hardwareKeyEventSignature(event)
    return when (event.action) {
        KeyEvent.ACTION_DOWN -> {
            if (
                lastHandledHardwareKeySignature == signature ||
                didRecentlyHandleHardwareKeyDown(signature)
            ) {
                true
            } else {
                val inputConnection = activeInputConnection?.takeIf {
                    it.hasPendingComposition()
                }
                if (inputConnection != null) {
                    var didMutate = false
                    runWithTransientInputMutationGuard {
                        didMutate = insertTransientHardwareText(keyEventText(event))
                        didMutate
                    }
                    if (!didMutate) return false
                    inputConnection.refreshComposingTextFromEditableForEditor()
                } else {
                    applyBaseEvent()
                }
                markHandledHardwareKeyDown(signature)
                true
            }
        }
        KeyEvent.ACTION_UP -> {
            if (lastHandledHardwareKeySignature?.let {
                    it.keyCode == event.keyCode && it.downTime == event.downTime
                } == true) {
                lastHandledHardwareKeySignature = null
            }
            false
        }
        else -> false
    }
}

internal fun EditorEditText.isPrintableHardwareMutationKey(event: KeyEvent): Boolean {
    if (isSupportedHardwareMutationKey(event.keyCode)) return false
    if (event.isCtrlPressed || event.isMetaPressed) return false
    return !keyEventText(event).isNullOrEmpty()
}

internal fun EditorEditText.hardwareKeyEventSignature(event: KeyEvent): HardwareKeyEventSignature =
    HardwareKeyEventSignature(
        keyCode = event.keyCode,
        downTime = event.downTime,
        repeatCount = event.repeatCount
    )

    @Suppress("DEPRECATION")
internal fun EditorEditText.keyEventCharacters(event: KeyEvent): String? = event.characters

internal fun EditorEditText.keyEventText(event: KeyEvent): String? {
    val characters = keyEventCharacters(event)
    if (!characters.isNullOrEmpty()) return characters
    val unicodeChar = event.unicodeChar
    if (unicodeChar == 0) return null
    return runCatching {
        String(Character.toChars(unicodeChar))
    }.getOrNull()
}

internal fun EditorEditText.insertTransientHardwareText(insertedText: String?): Boolean {
    if (insertedText.isNullOrEmpty()) return false
    val editable = text ?: return false
    val currentText = editable.toString()
    val rawStart = selectionStart
    val rawEnd = selectionEnd
    if (rawStart < 0 || rawEnd < 0) return false
    val start = rawStart.coerceIn(0, editable.length)
    val end = rawEnd.coerceIn(0, editable.length)
    val normalizedStart = minOf(start, end)
    val normalizedEnd = maxOf(start, end)
    val (replaceStart, replaceEnd) = PositionBridge.snapRangeToScalarBoundaries(
        normalizedStart,
        normalizedEnd,
        currentText
    )
    if (isCollapsedAtomBoundarySelection(replaceStart, replaceEnd)) return false
    editable.replace(replaceStart, replaceEnd, insertedText)
    val cursor = (replaceStart + insertedText.length).coerceIn(0, editable.length)
    setSelection(cursor)
    return true
}

internal fun EditorEditText.markHandledHardwareKeyDown(signature: HardwareKeyEventSignature) {
    lastHandledHardwareKeySignature = signature
    recentHandledHardwareKeyDownSignature = signature
    recentHandledHardwareKeyDownUptimeMs = SystemClock.uptimeMillis()
}

internal fun EditorEditText.didRecentlyHandleHardwareKeyDown(signature: HardwareKeyEventSignature): Boolean {
    val recentSignature = recentHandledHardwareKeyDownSignature ?: return false
    val elapsedMs = SystemClock.uptimeMillis() - recentHandledHardwareKeyDownUptimeMs
    if (elapsedMs > RECENT_HANDLED_HARDWARE_KEY_DOWN_WINDOW_MS) {
        recentHandledHardwareKeyDownSignature = null
        recentHandledHardwareKeyDownUptimeMs = 0L
        return false
    }
    return recentSignature == signature
}
