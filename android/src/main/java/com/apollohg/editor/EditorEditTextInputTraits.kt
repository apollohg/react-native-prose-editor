package com.apollohg.editor

import com.apollohg.editor.EditorEditText.ImeInitialSurroundingText
import com.apollohg.editor.EditorEditText.Companion.DEFAULT_KEYBOARD_TYPE
import com.apollohg.editor.EditorEditText.Companion.DEFAULT_AUTO_CORRECT
import com.apollohg.editor.EditorEditText.Companion.DEFAULT_AUTO_CAPITALIZE
import android.os.Build
import android.provider.Settings
import android.text.Annotation
import android.text.InputType
import android.text.Spanned
import android.text.SpannableStringBuilder
import android.view.inputmethod.InputConnection
import android.view.inputmethod.EditorInfo

internal fun EditorEditText.setAutoCapitalizeImpl(autoCapitalize: String?) {
    val next = when (autoCapitalize) {
        "none",
        "sentences",
        "words",
        "characters" -> autoCapitalize
        else -> DEFAULT_AUTO_CAPITALIZE
    }
    if (nativeAutoCapitalize == next) return
    nativeAutoCapitalize = next
    applyInputTraits()
}

internal fun EditorEditText.setAutoCorrectImpl(autoCorrect: Boolean?) {
    val next = autoCorrect ?: DEFAULT_AUTO_CORRECT
    if (nativeAutoCorrect == next) return
    nativeAutoCorrect = next
    applyInputTraits()
}

internal fun EditorEditText.setKeyboardTypeImpl(keyboardType: String?) {
    val next = when (keyboardType) {
        "default",
        "email-address",
        "numeric",
        "phone-pad",
        "ascii-capable",
        "numbers-and-punctuation",
        "url",
        "number-pad",
        "name-phone-pad",
        "decimal-pad",
        "twitter",
        "web-search",
        "visible-password",
        "ascii-capable-number-pad" -> keyboardType
        else -> DEFAULT_KEYBOARD_TYPE
    }
    if (nativeKeyboardType == next) return
    nativeKeyboardType = next
    applyInputTraits()
}

internal fun EditorEditText.setPrivateImeOptionsForEditorImpl(value: String?) {
    if (privateImeOptions == value) return
    privateImeOptions = value
    restartInputForEditorIfFocused("privateImeOptions")
}

internal fun EditorEditText.applyInputTraits() {
    val nextInputType = resolvedInputType()
    if (inputType == nextInputType) return

    val currentStart = selectionStart
    val currentEnd = selectionEnd
    val authorizedSelection = authorizedSelectionForTransientInputRestore(
        currentStart,
        currentEnd
    )
    discardTransientInputAndRestoreAuthorizedTextForEditor()
    setRawInputType(nextInputType)

    val editable = text
    if (editable != null && authorizedSelection != null) {
        setSelection(
            authorizedSelection.first.coerceIn(0, editable.length),
            authorizedSelection.second.coerceIn(0, editable.length)
        )
    }

    if (hasFocus()) {
        restartInputForEditor()
    }
}

internal fun EditorEditText.resolvedInputType(): Int {
    var nextInputType = when (nativeKeyboardType) {
        "email-address" -> InputType.TYPE_CLASS_TEXT or
            InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS
        "url" -> InputType.TYPE_CLASS_TEXT or
            InputType.TYPE_TEXT_VARIATION_URI
        "phone-pad" -> InputType.TYPE_CLASS_PHONE
        "number-pad" -> InputType.TYPE_CLASS_NUMBER
        "decimal-pad" -> InputType.TYPE_CLASS_NUMBER or
            InputType.TYPE_NUMBER_FLAG_DECIMAL
        "numeric" -> InputType.TYPE_CLASS_NUMBER or
            InputType.TYPE_NUMBER_FLAG_DECIMAL or
            InputType.TYPE_NUMBER_FLAG_SIGNED
        "visible-password" -> InputType.TYPE_CLASS_TEXT or
            InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
        else -> InputType.TYPE_CLASS_TEXT
    }

    if ((nextInputType and InputType.TYPE_MASK_CLASS) == InputType.TYPE_CLASS_TEXT) {
        nextInputType = nextInputType or InputType.TYPE_TEXT_FLAG_MULTI_LINE
        nextInputType = nextInputType or when (nativeAutoCapitalize) {
            "none" -> 0
            "words" -> InputType.TYPE_TEXT_FLAG_CAP_WORDS
            "characters" -> InputType.TYPE_TEXT_FLAG_CAP_CHARACTERS
            else -> InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        }
        nextInputType = nextInputType or if (nativeAutoCorrect) {
            InputType.TYPE_TEXT_FLAG_AUTO_CORRECT
        } else {
            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        }
    }

    return nextInputType
}

internal fun EditorEditText.applyInitialSurroundingTextForIme(
    outAttrs: EditorInfo,
    mapper: ImeTextCoordinateMapper,
): ImeInitialSurroundingText? {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return null
    val initialText = initialSurroundingTextForImeForEditor(mapper) ?: return null

    outAttrs.initialSelStart = initialText.selectionStart
    outAttrs.initialSelEnd = initialText.selectionEnd
    outAttrs.setInitialSurroundingText(initialText.text)
    return initialText
}

internal fun ImeInitialSurroundingText.textBeforeSelectionTailForImeLog(limit: Int = 24): String {
    val end = selectionStart.coerceIn(0, text.length)
    val start = maxOf(0, end - limit)
    return text.substring(start, end).toImeTraceSnippet()
}

private fun String.toImeTraceSnippet(): String {
    val builder = StringBuilder(length)
    forEach { ch ->
        when (ch) {
            '\n' -> builder.append("\\n")
            '\r' -> builder.append("\\r")
            '\t' -> builder.append("\\t")
            '\\' -> builder.append("\\\\")
            '"' -> builder.append("\\\"")
            else -> {
                if (ch.code < 0x20 || ch == LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER[0]) {
                    builder.append("\\u")
                    builder.append(ch.code.toString(16).padStart(4, '0'))
                } else {
                    builder.append(ch)
                }
            }
        }
    }
    return builder.toString()
}

internal fun EditorEditText.samsungSentenceCapsComposingTextForEditorImpl(composingText: String?): String? {
    if (composingText.isNullOrEmpty()) return composingText
    if (!isSamsungKeyboardActiveForEditor()) return composingText
    if ((inputType and InputType.TYPE_TEXT_FLAG_CAP_SENTENCES) != InputType.TYPE_TEXT_FLAG_CAP_SENTENCES) {
        return composingText
    }
    val (replacementStart, replacementEnd) = compositionReplacementRange() ?: return composingText
    if (replacementStart != replacementEnd) return composingText
    val authorizedSpanned = lastAuthorizedRenderedText as? Spanned
        ?: SpannableStringBuilder(lastAuthorizedText)
    if (!isRenderedLineStartForSentenceCaps(authorizedSpanned, replacementStart)) {
        return composingText
    }

    val firstCodePoint = Character.codePointAt(composingText, 0)
    if (!Character.isLowerCase(firstCodePoint)) return composingText
    val adjusted = buildString(composingText.length) {
        appendCodePoint(Character.toTitleCase(firstCodePoint))
        append(composingText.substring(Character.charCount(firstCodePoint)))
    }
    recordImeTraceForTesting(
        "samsungSentenceCapsFallback",
        "range=$replacementStart..$replacementEnd textLength=${composingText.length}"
    )
    return adjusted
}

internal fun EditorEditText.cursorCapsModeForEditorImpl(reqModes: Int, baseCapsMode: Int): Int {
    val sentenceCapsMode = InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
    if ((reqModes and sentenceCapsMode) != sentenceCapsMode) return baseCapsMode
    if ((baseCapsMode and sentenceCapsMode) == sentenceCapsMode) return baseCapsMode
    if (!isCursorAtRenderedLineStartForSentenceCaps()) return baseCapsMode
    return baseCapsMode or sentenceCapsMode
}

internal fun EditorEditText.initialSurroundingTextForImeForEditorImpl(
    mapper: ImeTextCoordinateMapper? = null,
): ImeInitialSurroundingText? {
    val coordinateMapper = mapper ?: imeTextCoordinateMapperForEditor() ?: return null
    val rawText = text ?: return null
    val removedCount = rawText.length - coordinateMapper.visibleText.length
    if (removedCount == 0) return null
    val start = selectionStart
    val end = selectionEnd
    if (start < 0 || end < 0) return null
    val rawSelectionStart = start.coerceIn(0, rawText.length)
    val rawSelectionEnd = end.coerceIn(0, rawText.length)

    return ImeInitialSurroundingText(
        text = coordinateMapper.visibleText.toString(),
        selectionStart = coordinateMapper.rawToIme(rawSelectionStart),
        selectionEnd = coordinateMapper.rawToIme(rawSelectionEnd),
        originalSelectionStart = rawSelectionStart,
        originalSelectionEnd = rawSelectionEnd,
        removedPlaceholderCount = removedCount
    )
}

internal fun EditorEditText.isCursorAtRenderedLineStartForSentenceCaps(): Boolean {
    val currentText = text ?: return false
    val start = selectionStart
    val end = selectionEnd
    if (start < 0 || end < 0 || start != end) return false

    val cursor = end.coerceIn(0, currentText.length)
    return isRenderedLineStartForSentenceCaps(currentText, cursor)
}

internal fun EditorEditText.isRenderedLineStartForSentenceCaps(text: Spanned, cursor: Int): Boolean {
    val cursor = cursor.coerceIn(0, text.length)
    if (cursor == 0) return true

    val lineStart = lastRenderedLineBreakBefore(text, cursor) + 1
    var index = lineStart
    while (index < cursor && isIgnoredSentenceCapsLinePrefix(text[index])) {
        index += 1
    }
    if (index == cursor) return true

    val markerEnd = renderedListMarkerEnd(text, index, cursor) ?: return false
    index = markerEnd
    while (index < cursor && isIgnoredSentenceCapsLinePrefix(text[index])) {
        index += 1
    }
    return index == cursor
}

internal fun EditorEditText.isSamsungKeyboardActiveForEditor(): Boolean {
    val inputMethodId = Settings.Secure.getString(
        context.contentResolver,
        Settings.Secure.DEFAULT_INPUT_METHOD
    ) ?: return false
    return inputMethodId.contains("samsung", ignoreCase = true) ||
        inputMethodId.contains("honeyboard", ignoreCase = true)
}

internal fun EditorEditText.lastRenderedLineBreakBefore(text: CharSequence, cursor: Int): Int {
    var index = cursor.coerceAtMost(text.length) - 1
    while (index >= 0) {
        when (text[index]) {
            '\n', '\r' -> return index
        }
        index -= 1
    }
    return -1
}

internal fun EditorEditText.isIgnoredSentenceCapsLinePrefix(ch: Char): Boolean =
    ch == ' ' ||
        ch == '\t' ||
        ch == '\u00A0' ||
        ch == LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER[0]

internal fun EditorEditText.renderedListMarkerEnd(text: Spanned, start: Int, endExclusive: Int): Int? {
    if (start >= endExclusive) return null
    if (renderedTaskListMarkerEnd(text, start, endExclusive) != null) {
        return start + 2
    }
    if (text[start] == LayoutConstants.UNORDERED_LIST_BULLET[0]) {
        return start + 1
    }

    var index = start
    while (index < endExclusive && text[index].isDigit()) {
        index += 1
    }
    if (index == start || index >= endExclusive) return null
    return when (text[index]) {
        '.', ')' -> index + 1
        else -> null
    }
}

internal fun EditorEditText.renderedTaskListMarkerEnd(text: Spanned, start: Int, endExclusive: Int): Int? {
    if (start + 1 >= endExclusive) return null
    val marker = text[start]
    if (marker != LayoutConstants.TASK_LIST_MARKER_UNCHECKED[0] &&
        marker != LayoutConstants.TASK_LIST_MARKER_CHECKED[0]
    ) return null
    if (text[start + 1] != ' ') return null
    val isMarker = text.getSpans(start, start + 1, Annotation::class.java)
        .any { it.key == RenderBridge.NATIVE_TASK_LIST_MARKER_ANNOTATION }
    return if (isMarker) start + 2 else null
}

internal fun EditorEditText.configureInputConnection(
    baseConnection: InputConnection,
    outAttrs: EditorInfo,
): InputConnection? {
    val originalInitialCapsMode = outAttrs.initialCapsMode
    outAttrs.initialCapsMode = cursorCapsModeForEditor(
        reqModes = outAttrs.inputType,
        baseCapsMode = outAttrs.initialCapsMode
    )
    val generation = nextInputConnectionGenerationForEditor()
    val mapper = imeTextCoordinateMapperForEditor(generation) ?: return null
    val initialSurroundingText = applyInitialSurroundingTextForIme(outAttrs, mapper)
    NativeEditorViewRegistry.registerInputView(editorId, this)
    recordImeTraceForTesting(
        "createInputConnection",
        "boundEditor=$editorId boundGen=$generation inputType=$inputType initialCaps=$originalInitialCapsMode->${outAttrs.initialCapsMode} " +
            "imeContextPlaceholdersRemoved=${initialSurroundingText?.removedPlaceholderCount ?: 0} " +
            "imeContextSel=${initialSurroundingText?.selectionStart ?: outAttrs.initialSelStart}..${initialSurroundingText?.selectionEnd ?: outAttrs.initialSelEnd} " +
            "imeContextRawSel=${initialSurroundingText?.originalSelectionStart ?: selectionStart}..${initialSurroundingText?.originalSelectionEnd ?: selectionEnd} " +
            "imeContextBeforeTail=\"${initialSurroundingText?.textBeforeSelectionTailForImeLog() ?: ""}\""
    )
    return EditorInputConnection(
        this,
        baseConnection,
        editorId,
        generation,
        mapper.generation,
    ).also {
        activeInputConnection = it
    }
}
