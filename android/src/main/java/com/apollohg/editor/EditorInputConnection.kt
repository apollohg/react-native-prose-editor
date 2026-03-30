package com.apollohg.editor

import android.view.KeyEvent
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputConnectionWrapper

/**
 * Custom [InputConnectionWrapper] that intercepts all text input from the soft keyboard
 * and routes it through the Rust editor-core engine via the hosting [EditorEditText].
 *
 * Instead of letting Android's EditText text storage handle insertions and deletions
 * directly, this class captures the user's intent (typing, deleting, IME composition)
 * and delegates to the Rust editor. The Rust editor returns render elements, which are
 * converted to [android.text.SpannableStringBuilder] via [RenderBridge] and applied
 * back to the EditText.
 *
 * ## Composition (IME) Handling
 *
 * For CJK input methods (and swipe keyboards), [setComposingText] and
 * [finishComposingText] are used. During composition, we let the base [InputConnection]
 * handle composing text normally so the user sees their in-progress input with the
 * composing underline. When composition finalizes ([finishComposingText]), we capture
 * the result and route it through Rust.
 *
 * ## Key Events
 *
 * Hardware keyboard events (backspace, enter) arrive via [sendKeyEvent]. We intercept
 * DEL and ENTER to route through the Rust editor.
 */
class EditorInputConnection(
    private val editorView: EditorEditText,
    baseConnection: InputConnection
) : InputConnectionWrapper(baseConnection, true) {

    companion object {
        internal fun codePointsToUtf16Length(
            text: String,
            fromUtf16Offset: Int,
            codePointCount: Int,
            forward: Boolean
        ): Int {
            if (codePointCount <= 0 || text.isEmpty()) return 0

            var remaining = codePointCount
            var utf16Length = 0

            if (forward) {
                var index = fromUtf16Offset.coerceIn(0, text.length)
                while (index < text.length && remaining > 0) {
                    val codePoint = Character.codePointAt(text, index)
                    val charCount = Character.charCount(codePoint)
                    utf16Length += charCount
                    index += charCount
                    remaining--
                }
            } else {
                var index = fromUtf16Offset.coerceIn(0, text.length)
                while (index > 0 && remaining > 0) {
                    val codePoint = Character.codePointBefore(text, index)
                    val charCount = Character.charCount(codePoint)
                    utf16Length += charCount
                    index -= charCount
                    remaining--
                }
            }

            return utf16Length
        }
    }

    /** Tracks the current composing text for CJK/swipe input. */
    private var composingText: String? = null

    /**
     * Called when the IME commits finalized text (single character, word,
     * autocomplete selection, etc.).
     *
     * Routes the text through Rust instead of directly inserting into the EditText.
     */
    override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
        if (!editorView.isEditable) return false
        if (editorView.isApplyingRustState) {
            return super.commitText(text, newCursorPosition)
        }
        text?.toString()?.let { editorView.handleTextCommit(it) }
        return true
    }

    /**
     * Called when the IME requests deletion of text surrounding the cursor.
     *
     * Routes the deletion through Rust instead of directly modifying the EditText.
     *
     * @param beforeLength Number of UTF-16 code units to delete before the cursor.
     * @param afterLength Number of UTF-16 code units to delete after the cursor.
     */
    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
        if (!editorView.isEditable) return false
        if (editorView.isApplyingRustState) {
            return super.deleteSurroundingText(beforeLength, afterLength)
        }
        editorView.handleDelete(beforeLength, afterLength)
        return true
    }

    override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean {
        if (!editorView.isEditable) return false
        if (editorView.isApplyingRustState) {
            return super.deleteSurroundingTextInCodePoints(beforeLength, afterLength)
        }

        val currentText = editorView.text?.toString().orEmpty()
        val cursor = editorView.selectionStart.coerceAtLeast(0)
        val beforeUtf16Length = codePointsToUtf16Length(
            text = currentText,
            fromUtf16Offset = cursor,
            codePointCount = beforeLength,
            forward = false
        )
        val afterUtf16Length = codePointsToUtf16Length(
            text = currentText,
            fromUtf16Offset = editorView.selectionEnd.coerceAtLeast(cursor),
            codePointCount = afterLength,
            forward = true
        )
        editorView.handleDelete(beforeUtf16Length, afterUtf16Length)
        return true
    }

    /**
     * Called when the IME sets composing (in-progress) text for CJK/swipe input.
     *
     * We let the base InputConnection handle this normally so the user sees
     * the composing text with its underline decoration. The text is NOT sent
     * to Rust during composition — only when [finishComposingText] is called.
     */
    override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
        if (!editorView.isEditable) return super.setComposingText(text, newCursorPosition)
        composingText = text?.toString()
        return super.setComposingText(text, newCursorPosition)
    }

    /**
     * Called when IME composition is finalized (user selects a candidate or
     * presses space/enter to commit the composing text).
     *
     * At this point, the composed text is final. We notify the [EditorEditText]
     * so it can capture the result and send it to Rust.
     */
    override fun finishComposingText(): Boolean {
        if (!editorView.isEditable) return super.finishComposingText()
        val composed = composingText
        composingText = null

        // Prevent selection sync while the base connection commits the composed
        // text, since the Rust document doesn't have it yet.
        editorView.isApplyingRustState = true
        val result = super.finishComposingText()
        editorView.isApplyingRustState = false

        // Now route the composed text through Rust.
        if (!editorView.isApplyingRustState) {
            editorView.handleCompositionFinished(composed)
        }
        return result
    }

    /**
     * Called for hardware keyboard key events.
     *
     * Intercepts DEL (backspace) and ENTER to route through Rust. Other key
     * events are passed through to the base connection.
     */
    override fun sendKeyEvent(event: KeyEvent?): Boolean {
        if (!editorView.isEditable) return false
        if (event != null && event.action == KeyEvent.ACTION_DOWN) {
            if (editorView.handleHardwareKeyDown(event.keyCode, event.isShiftPressed)) {
                return true
            }
        }
        if (event != null && event.action == KeyEvent.ACTION_UP) {
            when (event.keyCode) {
                KeyEvent.KEYCODE_DEL,
                KeyEvent.KEYCODE_ENTER,
                KeyEvent.KEYCODE_NUMPAD_ENTER,
                KeyEvent.KEYCODE_TAB -> return true
            }
        }
        return super.sendKeyEvent(event)
    }
}
