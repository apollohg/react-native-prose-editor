package com.apollohg.editor

import android.widget.LinearLayout.LayoutParams

import com.apollohg.editor.NativeEditorExpoView.ToolbarPlacement
import android.graphics.RectF
import android.os.Build
import org.json.JSONObject

internal fun NativeEditorExpoView.setThemeJsonImpl(themeJson: String?) {
    if (lastThemeJson == themeJson && !hasPendingTheme) return
    pendingThemeJson = themeJson
    hasPendingTheme = true
    pendingThemeRetry.bind(richTextView.editorId)
    pendingThemeRetry.resetAttempts()
    applyPendingThemeIfNeeded()
}

internal fun NativeEditorExpoView.setImageLoadingPolicyJsonImpl(policyJson: String?) {
    richTextView.editorEditText.setImageLoadingPolicyJson(policyJson)
}

internal fun NativeEditorExpoView.applyThemeJson(themeJson: String?) {
    if (lastThemeJson == themeJson) return
    lastThemeJson = themeJson
    val theme = EditorTheme.fromJson(themeJson)
    richTextView.applyTheme(theme)
    keyboardToolbarView.applyTheme(theme?.toolbar)
    keyboardToolbarView.applyMentionTheme(theme?.mentions ?: addons.mentions?.theme)
    keyboardToolbarView.requestLayout()
    updateKeyboardToolbarLayout()
    updateEditorViewportInset(forceMeasureToolbar = true)
    post {
        updateKeyboardToolbarLayout()
        updateEditorViewportInset(forceMeasureToolbar = true)
    }
}

internal fun NativeEditorExpoView.setHeightBehaviorImpl(rawHeightBehavior: String) {
    val nextBehavior = EditorHeightBehavior.fromRaw(rawHeightBehavior)
    if (heightBehavior == nextBehavior) return
    heightBehavior = nextBehavior
    if (nextBehavior != EditorHeightBehavior.AUTO_GROW) {
        lastEmittedContentHeight = 0
        lastEmittedContentHeightEditorId = null
        publishAutoGrowStyleHeight(null)
    }
    richTextView.setHeightBehavior(nextBehavior)
    val params = richTextView.layoutParams as LayoutParams
    params.width = LayoutParams.MATCH_PARENT
    params.height = if (nextBehavior == EditorHeightBehavior.AUTO_GROW) {
        LayoutParams.WRAP_CONTENT
    } else {
        LayoutParams.MATCH_PARENT
    }
    richTextView.layoutParams = params
    requestLayout()
    if (nextBehavior == EditorHeightBehavior.AUTO_GROW) {
        post { emitContentHeightIfNeeded(force = true) }
    }
    updateEditorViewportInset()
}

internal fun NativeEditorExpoView.invalidateAutoGrowContentHeightEmission() {
    if (heightBehavior != EditorHeightBehavior.AUTO_GROW) return
    lastEmittedContentHeight = 0
    lastEmittedContentHeightEditorId = null
    requestLayout()
}

internal fun NativeEditorExpoView.setAddonsJsonImpl(addonsJson: String?) {
    if (lastAddonsJson == addonsJson) return
    clearPendingNativeActionRetry()
    lastAddonsJson = addonsJson
    addons = NativeEditorAddons.fromJson(addonsJson)
    keyboardToolbarView.applyMentionTheme(richTextView.editorEditText.theme?.mentions ?: addons.mentions?.theme)
    refreshMentionQuery()
}

internal fun NativeEditorExpoView.setAtomsJsonImpl(atomsJson: String?) {
    if (lastAtomsJson == atomsJson && !hasPendingAtoms) return
    pendingAtomsJson = atomsJson
    hasPendingAtoms = true
    pendingAtomsRetry.bind(richTextView.editorId)
    pendingAtomsRetry.resetAttempts()
    applyPendingAtomsIfNeeded()
}

internal fun NativeEditorExpoView.setRemoteSelectionsJsonImpl(remoteSelectionsJson: String?) {
    if (lastRemoteSelectionsJson == remoteSelectionsJson) return
    lastRemoteSelectionsJson = remoteSelectionsJson
    richTextView.setRemoteSelections(
        RemoteSelectionDecoration.fromJson(context, remoteSelectionsJson)
    )
}

internal fun NativeEditorExpoView.setAutoFocusImpl(autoFocus: Boolean) {
    autoFocusRequested = autoFocus
    applyAutoFocusIfNeeded()
}

internal fun NativeEditorExpoView.applyAutoFocusIfNeeded() {
    if (!autoFocusRequested || didApplyAutoFocus || !canFocusCurrentEditor()) return
    didApplyAutoFocus = true
    focus()
}

internal fun NativeEditorExpoView.setAutoCapitalizeImpl(autoCapitalize: String?) {
    richTextView.editorEditText.setAutoCapitalize(autoCapitalize)
}

internal fun NativeEditorExpoView.setAutoCorrectImpl(autoCorrect: Boolean?) {
    richTextView.editorEditText.setAutoCorrect(autoCorrect)
}

internal fun NativeEditorExpoView.setKeyboardTypeImpl(keyboardType: String?) {
    richTextView.editorEditText.setKeyboardType(keyboardType)
}

internal fun NativeEditorExpoView.setAndroidInputOptionsJsonImpl(optionsJson: String?) {
    val options = optionsJson?.let { runCatching { JSONObject(it) }.getOrNull() }
    val privateImeOptions = options?.opt("privateImeOptions") as? String
    richTextView.editorEditText.setPrivateImeOptionsForEditor(privateImeOptions)
}

internal fun NativeEditorExpoView.setEditableImpl(editable: Boolean) {
    if (richTextView.editorEditText.isEditable == editable) return
    if (!editable) {
        cancelActiveExternalTextComposition("lifecycle")
        cancelPendingToolbarRefocus()
        clearPendingNativeActionRetry()
    }
    richTextView.editorEditText.isEditable = editable
    updateKeyboardToolbarVisibility()
}

internal fun NativeEditorExpoView.setAccessibilityLabelImpl(label: String?) {
    richTextView.editorEditText.contentDescription = label
}

internal fun NativeEditorExpoView.setAccessibilityHintImpl(hint: String?) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        richTextView.editorEditText.tooltipText = null
    }
    richTextView.editorEditText.setEditorAccessibilityHint(hint)
}

internal fun NativeEditorExpoView.setShowToolbarImpl(showToolbar: Boolean) {
    if (showsToolbar == showToolbar) return
    if (!showToolbar) {
        cancelPendingToolbarRefocus()
        clearPendingNativeActionRetry()
    }
    showsToolbar = showToolbar
    updateKeyboardToolbarVisibility()
}

internal fun NativeEditorExpoView.setToolbarPlacementImpl(rawToolbarPlacement: String?) {
    val nextPlacement = ToolbarPlacement.fromRaw(rawToolbarPlacement)
    if (toolbarPlacement == nextPlacement) return
    if (nextPlacement != ToolbarPlacement.KEYBOARD) {
        cancelPendingToolbarRefocus()
        clearPendingNativeActionRetry()
    }
    toolbarPlacement = nextPlacement
    updateKeyboardToolbarVisibility()
}

internal fun NativeEditorExpoView.setAllowImageResizingImpl(allowImageResizing: Boolean) {
    richTextView.setImageResizingEnabled(allowImageResizing)
}

internal fun NativeEditorExpoView.setToolbarItemsJsonImpl(toolbarItemsJson: String?) {
    if (lastToolbarItemsJson == toolbarItemsJson) return
    clearPendingNativeActionRetry()
    lastToolbarItemsJson = toolbarItemsJson
    keyboardToolbarView.setItems(NativeToolbarItem.fromJson(toolbarItemsJson))
}

internal fun NativeEditorExpoView.setToolbarFrameJsonImpl(toolbarFrameJson: String?) {
    if (lastToolbarFrameJson == toolbarFrameJson) return
    lastToolbarFrameJson = toolbarFrameJson
    if (toolbarFrameJson.isNullOrBlank()) {
        toolbarFramesInWindow = emptyList()
        return
    }

    toolbarFramesInWindow = try {
        val json = JSONObject(toolbarFrameJson)
        val frames = json.optJSONArray("frames")
        if (frames != null) {
            buildList {
                for (index in 0 until frames.length()) {
                    frames.optJSONObject(index)?.toToolbarFrame()?.let { add(it) }
                }
            }
        } else {
            listOfNotNull(json.toToolbarFrame())
        }
    } catch (_: Throwable) {
        emptyList()
    }
}

private fun JSONObject.toToolbarFrame(): RectF? {
    val x = optDouble("x", Double.NaN)
    val y = optDouble("y", Double.NaN)
    val width = optDouble("width", Double.NaN)
    val height = optDouble("height", Double.NaN)
    if (
        x.isNaN() || x.isInfinite() ||
        y.isNaN() || y.isInfinite() ||
        width.isNaN() || width.isInfinite() ||
        height.isNaN() || height.isInfinite()
    ) {
        return null
    }
    if (width <= 0.0 || height <= 0.0) {
        return null
    }

    return RectF(
        x.toFloat(),
        y.toFloat(),
        (x + width).toFloat(),
        (y + height).toFloat()
    )
}
