package com.apollohg.editor

import android.view.View.MeasureSpec

import com.apollohg.editor.NativeEditorExpoView.PreflightUpdateEvent
import com.apollohg.editor.NativeEditorExpoView.PendingNativeAction
import com.apollohg.editor.NativeEditorExpoView.ToolbarPlacement
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.ScrollView
import androidx.core.widget.NestedScrollView
import androidx.core.view.ViewCompat

internal fun NativeEditorExpoView.refreshToolbarStateFromEditorSelection(): String? {
    if (richTextView.editorId == 0L) return null
    if (handleDestroyedCurrentEditorIfNeeded()) return null
    onRefreshToolbarStateFromEditorSelectionForTesting?.let { callback ->
        val stateJson = callback()
        noteDocumentVersionFromUpdateJSON(stateJson)
        return stateJson
    }
    val stateJson = richTextView.editorEditText.v2Driver?.currentStateJson() ?: return null
    noteDocumentVersionFromUpdateJSON(stateJson)
    val state = NativeToolbarState.fromUpdateJson(stateJson) ?: return null
    toolbarState = state
    keyboardToolbarView.applyState(state)
    return stateJson
}

internal fun NativeEditorExpoView.ensureKeyboardToolbarAttached() {
    val host = resolveActivity(context)?.findViewById<ViewGroup>(android.R.id.content) ?: return
    pendingKeyboardToolbarDetachGeneration += 1
    if (keyboardToolbarView.parent === host) {
        updateKeyboardToolbarLayout()
        return
    }
    (keyboardToolbarView.parent as? ViewGroup)?.removeView(keyboardToolbarView)
    host.addView(
        keyboardToolbarView,
        FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.MATCH_PARENT,
            FrameLayout.LayoutParams.WRAP_CONTENT,
            Gravity.BOTTOM or Gravity.START
        )
    )
    updateKeyboardToolbarLayout()
    ViewCompat.requestApplyInsets(keyboardToolbarView)
}

internal fun NativeEditorExpoView.detachKeyboardToolbarIfNeeded() {
    pendingKeyboardToolbarDetachGeneration += 1
    val generation = pendingKeyboardToolbarDetachGeneration
    val parent = keyboardToolbarView.parent as? ViewGroup ?: return
    keyboardToolbarImeAnimationController.cancel()
    keyboardToolbarView.visibility = View.GONE
    parent.post {
        if (generation != pendingKeyboardToolbarDetachGeneration) return@post
        if (keyboardToolbarView.parent === parent) {
            parent.removeView(keyboardToolbarView)
        }
    }
}

internal fun NativeEditorExpoView.updateKeyboardToolbarLayout() {
    val params = keyboardToolbarView.layoutParams as? FrameLayout.LayoutParams ?: return
    val toolbarTheme = richTextView.editorEditText.theme?.toolbar
    val density = resources.displayMetrics.density
    params.gravity = Gravity.BOTTOM or Gravity.START
    val horizontalInsetPx = ((toolbarTheme?.resolvedHorizontalInset() ?: 0f) * density).toInt()
    val keyboardOffsetPx = ((toolbarTheme?.resolvedKeyboardOffset() ?: 0f) * density).toInt()
    params.leftMargin = horizontalInsetPx
    params.rightMargin = horizontalInsetPx
    params.bottomMargin = currentImeBottom + keyboardOffsetPx
    keyboardToolbarView.layoutParams = params
}

internal fun NativeEditorExpoView.updateAttachedKeyboardToolbarForInsets() {
    if (currentImeBottom <= 0) {
        clearPendingNativeActionRetry()
    }
    keyboardToolbarView.visibility = if (currentImeBottom > 0) View.VISIBLE else View.INVISIBLE
    updateEditorViewportInset()
}

internal fun NativeEditorExpoView.updateKeyboardToolbarVisibility() {
    val shouldAttach =
        showsToolbar &&
            canFocusCurrentEditor() &&
            toolbarPlacement == ToolbarPlacement.KEYBOARD &&
            richTextView.editorEditText.isEditable &&
            richTextView.editorEditText.hasFocus()

    if (!shouldAttach) {
        keyboardToolbarView.visibility = View.GONE
        detachKeyboardToolbarIfNeeded()
        updateEditorViewportInset()
        return
    }

    ensureKeyboardToolbarAttached()
    keyboardToolbarView.visibility = if (currentImeBottom > 0) View.VISIBLE else View.INVISIBLE
    updateEditorViewportInset()
}

internal fun NativeEditorExpoView.updateEditorViewportInset(forceMeasureToolbar: Boolean = false) {
    val shouldReserveToolbarSpace =
        showsToolbar &&
            toolbarPlacement == ToolbarPlacement.KEYBOARD &&
            richTextView.editorEditText.isEditable &&
            richTextView.editorEditText.hasFocus() &&
            currentImeBottom > 0

    if (!shouldReserveToolbarSpace) {
        richTextView.setViewportBottomOcclusionTopOnScreenPx(null)
        richTextView.setViewportBottomInsetPx(0)
        return
    }

    val hostWidth = (resolveActivity(context)?.findViewById<ViewGroup>(android.R.id.content)?.width ?: width)
        .coerceAtLeast(0)
    val toolbarTheme = richTextView.editorEditText.theme?.toolbar
    val density = resources.displayMetrics.density
    val horizontalInsetPx = ((toolbarTheme?.resolvedHorizontalInset() ?: 0f) * density).toInt()
    if (forceMeasureToolbar || keyboardToolbarView.measuredHeight == 0) {
        val availableWidth = (hostWidth - horizontalInsetPx * 2).coerceAtLeast(0)
        val widthSpec = MeasureSpec.makeMeasureSpec(availableWidth, MeasureSpec.AT_MOST)
        val heightSpec = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED)
        keyboardToolbarView.measure(widthSpec, heightSpec)
    }
    val toolbarHeight = keyboardToolbarView.measuredHeight.coerceAtLeast(keyboardToolbarView.height)
    val keyboardOffsetPx = ((toolbarTheme?.resolvedKeyboardOffset() ?: 0f) * density).toInt()
    val toolbarTopOnScreenPx = resolveToolbarTopOnScreenPx(
        toolbarHeight = toolbarHeight,
        keyboardOffsetPx = keyboardOffsetPx
    )
    richTextView.setViewportBottomOcclusionTopOnScreenPx(toolbarTopOnScreenPx)
    richTextView.setViewportBottomInsetPx(
        resolveToolbarViewportInsetPx(
            toolbarHeight = toolbarHeight,
            keyboardOffsetPx = keyboardOffsetPx,
            toolbarTopOnScreenPx = toolbarTopOnScreenPx
        )
    )
}

internal fun NativeEditorExpoView.resolveToolbarTopOnScreenPx(
    toolbarHeight: Int,
    keyboardOffsetPx: Int
): Int? {
    val host = resolveActivity(context)?.findViewById<ViewGroup>(android.R.id.content)
        ?: return null
    if (host.height <= 0) return null
    val hostLocation = IntArray(2)
    host.getLocationOnScreen(hostLocation)
    return hostLocation[1] + host.height - currentImeBottom - keyboardOffsetPx - toolbarHeight
}

internal fun NativeEditorExpoView.resolveToolbarViewportInsetPx(
    toolbarHeight: Int,
    keyboardOffsetPx: Int,
    toolbarTopOnScreenPx: Int?
): Int {
    val fallbackInset = (toolbarHeight + keyboardOffsetPx).coerceAtLeast(0)
    val toolbarTop = toolbarTopOnScreenPx ?: return fallbackInset
    var foundScrollViewport = false
    var viewportInset = 0

    fun includeScrollViewport(view: View) {
        if (view.height <= 0) return
        val location = IntArray(2)
        view.getLocationOnScreen(location)
        foundScrollViewport = true
        viewportInset = maxOf(viewportInset, location[1] + view.height - toolbarTop)
    }

    if (heightBehavior == EditorHeightBehavior.FIXED) {
        includeScrollViewport(richTextView.editorScrollView)
    } else {
        var ancestor = parent
        while (ancestor is View) {
            if (ancestor is ScrollView || ancestor is NestedScrollView) {
                includeScrollViewport(ancestor)
            }
            ancestor = (ancestor as View).parent
        }
    }

    return if (foundScrollViewport) viewportInset.coerceAtLeast(0) else fallbackInset
}

internal fun NativeEditorExpoView.handleListToggle(listType: String) {
    val isActive = toolbarState.nodes[listType] == true
    richTextView.editorEditText.performToolbarToggleList(listType, isActive)
}

internal fun NativeEditorExpoView.handleToolbarItemPress(
    item: NativeToolbarItem,
    allowPreflightRetry: Boolean = true
) {
    if (handleDestroyedCurrentEditorIfNeeded()) return
    if (!richTextView.editorEditText.isEditable) {
        clearPendingNativeActionRetry()
        return
    }
    var preflightUpdate: PreflightUpdateEvent? = null
    val needsEditorPreflight = when (item.type) {
        ToolbarItemKind.mark,
        ToolbarItemKind.heading,
        ToolbarItemKind.blockquote,
        ToolbarItemKind.list,
        ToolbarItemKind.command,
        ToolbarItemKind.node,
        ToolbarItemKind.action -> true
        ToolbarItemKind.group,
        ToolbarItemKind.separator -> false
    }
    if (needsEditorPreflight) {
        if (shouldBlockEditorCommandForPendingUpdate()) {
            if (allowPreflightRetry) {
                schedulePendingNativeActionRetry(PendingNativeAction.ToolbarItemPress(item))
            }
            return
        }
        val preparation = richTextView.editorEditText.prepareForExternalEditorCommand()
        if (!preparation.ready) {
            if (allowPreflightRetry) {
                schedulePendingNativeActionRetry(PendingNativeAction.ToolbarItemPress(item))
            }
            return
        }
        preflightUpdate = preflightUpdateEventFromJSON(preparation.updateJSON)
        preflightUpdate?.let { lastDocumentVersion = it.documentRevision }
        clearPendingNativeActionRetry()
    }
    if (handleDestroyedCurrentEditorIfNeeded()) return
    when (item.type) {
        ToolbarItemKind.mark -> item.mark?.let { richTextView.editorEditText.performToolbarToggleMark(it) }
        ToolbarItemKind.heading -> item.headingLevel?.let { richTextView.editorEditText.performToolbarToggleHeading(it) }
        ToolbarItemKind.blockquote -> richTextView.editorEditText.performToolbarToggleBlockquote()
        ToolbarItemKind.list -> item.listType?.name?.let { handleListToggle(it) }
        ToolbarItemKind.command -> when (item.command) {
            ToolbarCommand.indentList -> richTextView.editorEditText.performToolbarIndentListItem()
            ToolbarCommand.outdentList -> richTextView.editorEditText.performToolbarOutdentListItem()
            ToolbarCommand.undo -> richTextView.editorEditText.performToolbarUndo()
            ToolbarCommand.redo -> richTextView.editorEditText.performToolbarRedo()
            null -> Unit
        }
        ToolbarItemKind.node -> item.nodeType?.let { richTextView.editorEditText.performToolbarInsertNode(it) }
        ToolbarItemKind.action -> item.key?.let {
            if (handleDestroyedCurrentEditorIfNeeded()) return
            val payload = mutableMapOf<String, Any>(
                "key" to it,
                "editorId" to eventEditorId(richTextView.editorId)
            )
            addPreflightUpdateToEvent(payload, preflightUpdate)
            onToolbarActionForTesting?.invoke(payload) ?: onToolbarAction(payload)
        }
        ToolbarItemKind.group -> Unit
        ToolbarItemKind.separator -> Unit
    }
}
