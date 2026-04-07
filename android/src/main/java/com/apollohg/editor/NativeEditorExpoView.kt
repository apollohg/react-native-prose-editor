package com.apollohg.editor

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.graphics.Rect
import android.graphics.RectF
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.Window
import android.view.inputmethod.InputMethodManager
import android.widget.FrameLayout
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import org.json.JSONArray
import org.json.JSONObject
import uniffi.editor_core.*

/**
 * Expo Modules wrapper view that hosts a [RichTextEditorView] and bridges
 * editor events to React Native via [EventDispatcher].
 *
 * Registered as the native view component in [NativeEditorModule].
 */
class NativeEditorExpoView(
    context: Context,
    appContext: AppContext
) : ExpoView(context, appContext), EditorEditText.EditorListener {

    private enum class ToolbarPlacement {
        KEYBOARD,
        INLINE;

        companion object {
            fun fromRaw(raw: String?): ToolbarPlacement =
                if (raw == "inline") INLINE else KEYBOARD
        }
    }

    val richTextView: RichTextEditorView = RichTextEditorView(context)
    private val keyboardToolbarView = EditorKeyboardToolbarView(context)

    private val onEditorUpdate by EventDispatcher<Map<String, Any>>()
    private val onSelectionChange by EventDispatcher<Map<String, Any>>()
    private val onFocusChange by EventDispatcher<Map<String, Any>>()
    private val onContentHeightChange by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    private val onToolbarAction by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    private val onAddonEvent by EventDispatcher<Map<String, Any>>()

    /** Guard flag: when true, editor updates originated from JS and should not echo back. */
    var isApplyingJSUpdate = false
    private var didApplyAutoFocus = false
    private var heightBehavior = EditorHeightBehavior.FIXED
    private var lastEmittedContentHeight = 0
    private var outsideTapWindowCallback: Window.Callback? = null
    private var previousWindowCallback: Window.Callback? = null
    private var toolbarFrameInWindow: RectF? = null
    private var addons = NativeEditorAddons(null)
    private var mentionQueryState: MentionQueryState? = null
    private var lastMentionEventJson: String? = null
    private var toolbarState = NativeToolbarState.empty
    private var showsToolbar = true
    private var toolbarPlacement = ToolbarPlacement.KEYBOARD
    private var currentImeBottom = 0
    private var pendingEditorUpdateJson: String? = null
    private var pendingEditorUpdateRevision = 0
    private var appliedEditorUpdateRevision = 0

    init {
        addView(richTextView, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        richTextView.editorEditText.editorListener = this
        keyboardToolbarView.onPressItem = { item ->
            handleToolbarItemPress(item)
        }
        keyboardToolbarView.onSelectMentionSuggestion = { suggestion ->
            insertMentionSuggestion(suggestion)
        }
        keyboardToolbarView.applyState(toolbarState)
        ViewCompat.setOnApplyWindowInsetsListener(keyboardToolbarView) { _, insets ->
            currentImeBottom = insets.getInsets(WindowInsetsCompat.Type.ime()).bottom
            updateKeyboardToolbarLayout()
            updateKeyboardToolbarVisibility()
            insets
        }

        // Observe EditText focus changes.
        richTextView.editorEditText.setOnFocusChangeListener { _, hasFocus ->
            if (hasFocus) {
                installOutsideTapBlurHandlerIfNeeded()
                refreshMentionQuery()
            } else {
                uninstallOutsideTapBlurHandler()
                clearMentionQueryState()
            }
            updateKeyboardToolbarVisibility()
            val event = mapOf<String, Any>("isFocused" to hasFocus)
            onFocusChange(event)
        }
    }

    fun setEditorId(id: Long) {
        richTextView.editorId = id
    }

    fun setThemeJson(themeJson: String?) {
        val theme = EditorTheme.fromJson(themeJson)
        richTextView.applyTheme(theme)
        keyboardToolbarView.applyTheme(theme?.toolbar)
        keyboardToolbarView.applyMentionTheme(theme?.mentions ?: addons.mentions?.theme)
        updateKeyboardToolbarLayout()
    }

    fun setHeightBehavior(rawHeightBehavior: String) {
        val nextBehavior = EditorHeightBehavior.fromRaw(rawHeightBehavior)
        if (heightBehavior == nextBehavior) return
        heightBehavior = nextBehavior
        if (nextBehavior != EditorHeightBehavior.AUTO_GROW) {
            lastEmittedContentHeight = 0
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

    fun setAddonsJson(addonsJson: String?) {
        addons = NativeEditorAddons.fromJson(addonsJson)
        keyboardToolbarView.applyMentionTheme(richTextView.editorEditText.theme?.mentions ?: addons.mentions?.theme)
        refreshMentionQuery()
    }

    fun setRemoteSelectionsJson(remoteSelectionsJson: String?) {
        richTextView.setRemoteSelections(
            RemoteSelectionDecoration.fromJson(context, remoteSelectionsJson)
        )
    }

    fun setAutoFocus(autoFocus: Boolean) {
        if (!autoFocus || didApplyAutoFocus) {
            return
        }
        didApplyAutoFocus = true
        focus()
    }

    fun setShowToolbar(showToolbar: Boolean) {
        showsToolbar = showToolbar
        updateKeyboardToolbarVisibility()
    }

    fun setToolbarPlacement(rawToolbarPlacement: String?) {
        toolbarPlacement = ToolbarPlacement.fromRaw(rawToolbarPlacement)
        updateKeyboardToolbarVisibility()
    }

    fun setAllowImageResizing(allowImageResizing: Boolean) {
        richTextView.setImageResizingEnabled(allowImageResizing)
    }

    fun setToolbarItemsJson(toolbarItemsJson: String?) {
        keyboardToolbarView.setItems(NativeToolbarItem.fromJson(toolbarItemsJson))
    }

    fun setToolbarFrameJson(toolbarFrameJson: String?) {
        if (toolbarFrameJson.isNullOrBlank()) {
            toolbarFrameInWindow = null
            return
        }

        toolbarFrameInWindow = try {
            val json = JSONObject(toolbarFrameJson)
            RectF(
                json.optDouble("x").toFloat(),
                json.optDouble("y").toFloat(),
                (json.optDouble("x") + json.optDouble("width")).toFloat(),
                (json.optDouble("y") + json.optDouble("height")).toFloat()
            )
        } catch (_: Throwable) {
            null
        }
    }

    fun setPendingEditorUpdateJson(editorUpdateJson: String?) {
        pendingEditorUpdateJson = editorUpdateJson
    }

    fun setPendingEditorUpdateRevision(editorUpdateRevision: Int) {
        pendingEditorUpdateRevision = editorUpdateRevision
    }

    fun applyPendingEditorUpdateIfNeeded() {
        val updateJson = pendingEditorUpdateJson ?: return
        if (pendingEditorUpdateRevision == 0) return
        if (pendingEditorUpdateRevision == appliedEditorUpdateRevision) return
        appliedEditorUpdateRevision = pendingEditorUpdateRevision
        applyEditorUpdate(updateJson)
    }

    fun focus() {
        richTextView.editorEditText.requestFocus()
    }

    fun blur() {
        richTextView.editorEditText.clearFocus()
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
        imm?.hideSoftInputFromWindow(richTextView.editorEditText.windowToken, 0)
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        uninstallOutsideTapBlurHandler()
        detachKeyboardToolbarIfNeeded()
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        if (heightBehavior != EditorHeightBehavior.AUTO_GROW) {
            super.onMeasure(widthMeasureSpec, heightMeasureSpec)
            return
        }

        val childWidthSpec = getChildMeasureSpec(
            widthMeasureSpec,
            paddingLeft + paddingRight,
            richTextView.layoutParams.width
        )
        val childHeightSpec = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED)
        richTextView.measure(childWidthSpec, childHeightSpec)

        val measuredWidth = resolveSize(
            richTextView.measuredWidth + paddingLeft + paddingRight,
            widthMeasureSpec
        )
        val desiredHeight = richTextView.measuredHeight + paddingTop + paddingBottom
        val measuredHeight = when (MeasureSpec.getMode(heightMeasureSpec)) {
            MeasureSpec.AT_MOST -> desiredHeight.coerceAtMost(MeasureSpec.getSize(heightMeasureSpec))
            else -> desiredHeight
        }
        setMeasuredDimension(measuredWidth, measuredHeight)
        emitContentHeightIfNeeded(force = false)
    }

    private fun emitContentHeightIfNeeded(force: Boolean) {
        if (heightBehavior != EditorHeightBehavior.AUTO_GROW) return
        val editText = richTextView.editorEditText
        val resolvedEditHeight = editText.resolveAutoGrowHeight()
        val contentHeight = (
            when {
                editText.isLaidOut && (editText.layout?.height ?: 0) > 0 -> {
                    (editText.layout?.height ?: 0) +
                        editText.compoundPaddingTop +
                        editText.compoundPaddingBottom +
                        richTextView.paddingTop +
                        richTextView.paddingBottom
                }
                richTextView.measuredHeight > 0 -> {
                    richTextView.measuredHeight + paddingTop + paddingBottom
                }
                editText.measuredHeight > 0 -> {
                    editText.measuredHeight +
                        richTextView.paddingTop +
                        richTextView.paddingBottom +
                        paddingTop +
                        paddingBottom
                }
                else -> {
                    resolvedEditHeight +
                        richTextView.paddingTop +
                        richTextView.paddingBottom +
                        paddingTop +
                        paddingBottom
                }
            }
        ).coerceAtLeast(0)
        if (contentHeight <= 0) return
        if (!force && contentHeight == lastEmittedContentHeight) return
        lastEmittedContentHeight = contentHeight
        onContentHeightChange(mapOf("contentHeight" to contentHeight))
    }

    /** Applies an editor update from JS without echoing it back through events. */
    fun applyEditorUpdate(updateJson: String) {
        val apply = Runnable {
            isApplyingJSUpdate = true
            richTextView.editorEditText.applyUpdateJSON(updateJson)
            isApplyingJSUpdate = false
        }
        if (android.os.Looper.myLooper() == android.os.Looper.getMainLooper()) {
            apply.run()
        } else {
            val latch = java.util.concurrent.CountDownLatch(1)
            post {
                apply.run()
                latch.countDown()
            }
            latch.await()
        }
    }

    override fun onSelectionChanged(anchor: Int, head: Int) {
        refreshToolbarStateFromEditorSelection()
        refreshMentionQuery()
        richTextView.refreshRemoteSelections()
        val event = mapOf<String, Any>("anchor" to anchor, "head" to head)
        onSelectionChange(event)
    }

    override fun onEditorUpdate(updateJSON: String) {
        NativeToolbarState.fromUpdateJson(updateJSON)?.let { state ->
            toolbarState = state
            keyboardToolbarView.applyState(state)
        }
        refreshMentionQuery()
        richTextView.refreshRemoteSelections()
        if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
            post {
                requestLayout()
                emitContentHeightIfNeeded(force = false)
            }
        }
        if (isApplyingJSUpdate) return
        val event = mapOf<String, Any>("updateJson" to updateJSON)
        onEditorUpdate(event)
    }

    private fun installOutsideTapBlurHandlerIfNeeded() {
        val window = resolveActivity(context)?.window ?: return
        val currentCallback = window.callback ?: return
        if (currentCallback === outsideTapWindowCallback) return

        val wrappedCallback = object : Window.Callback by currentCallback {
            override fun dispatchTouchEvent(event: MotionEvent): Boolean {
                if (
                    event.action == MotionEvent.ACTION_DOWN &&
                    richTextView.editorEditText.hasFocus() &&
                    isTouchOutsideEditor(event)
                ) {
                    blur()
                }
                return currentCallback.dispatchTouchEvent(event)
            }
        }

        previousWindowCallback = currentCallback
        outsideTapWindowCallback = wrappedCallback
        window.callback = wrappedCallback
    }

    private fun uninstallOutsideTapBlurHandler() {
        val window = resolveActivity(context)?.window ?: return
        val callback = outsideTapWindowCallback ?: return
        if (window.callback === callback) {
            window.callback = previousWindowCallback ?: callback
        }
        outsideTapWindowCallback = null
        previousWindowCallback = null
    }

    private fun isTouchOutsideEditor(event: MotionEvent): Boolean {
        if (isTouchInsideKeyboardToolbar(event)) {
            return false
        }
        val toolbarFrame = toolbarFrameInWindow
        if (toolbarFrame != null && toolbarFrame.contains(event.rawX, event.rawY)) {
            return false
        }
        val rect = Rect()
        richTextView.editorEditText.getGlobalVisibleRect(rect)
        return !rect.contains(event.rawX.toInt(), event.rawY.toInt())
    }

    private fun isTouchInsideKeyboardToolbar(event: MotionEvent): Boolean {
        if (keyboardToolbarView.parent == null || keyboardToolbarView.visibility != View.VISIBLE) {
            return false
        }
        val rect = Rect()
        keyboardToolbarView.getGlobalVisibleRect(rect)
        return rect.contains(event.rawX.toInt(), event.rawY.toInt())
    }

    private fun resolveActivity(context: Context): Activity? {
        var current: Context? = context
        while (current is ContextWrapper) {
            if (current is Activity) return current
            current = current.baseContext
        }
        return null
    }

    private fun refreshMentionQuery() {
        val mentions = addons.mentions
        if (mentions == null || !richTextView.editorEditText.hasFocus()) {
            clearMentionQueryState()
            emitMentionQueryChange("", "@", 0, 0, false)
            return
        }

        val queryState = currentMentionQueryState(mentions.trigger)
        if (queryState == null) {
            clearMentionQueryState()
            emitMentionQueryChange("", mentions.trigger, 0, 0, false)
            return
        }

        mentionQueryState = queryState
        val suggestions = filteredMentionSuggestions(queryState, mentions)
        keyboardToolbarView.applyMentionTheme(richTextView.editorEditText.theme?.mentions ?: mentions.theme)
        syncKeyboardToolbarMentionSuggestions(suggestions)
        emitMentionQueryChange(
            queryState.query,
            queryState.trigger,
            queryState.anchor,
            queryState.head,
            true
        )
    }

    private fun clearMentionQueryState() {
        mentionQueryState = null
        syncKeyboardToolbarMentionSuggestions(emptyList())
    }

    private fun currentMentionQueryState(trigger: String): MentionQueryState? {
        val editor = richTextView.editorEditText
        if (editor.selectionStart != editor.selectionEnd) return null
        val text = editor.text?.toString() ?: return null
        val cursorUtf16 = editor.selectionStart
        val cursorScalar = PositionBridge.utf16ToScalar(cursorUtf16, text)
        return resolveMentionQueryState(
            text = text,
            cursorScalar = cursorScalar,
            trigger = trigger,
            isCaretInsideMention = isCaretInsideMention(cursorUtf16)
        )
    }

    private fun isCaretInsideMention(cursorUtf16: Int): Boolean {
        val editable = richTextView.editorEditText.text ?: return false
        val checkOffsets = listOf(cursorUtf16, (cursorUtf16 - 1).coerceAtLeast(0))
        return checkOffsets.any { offset ->
            editable.getSpans(offset, offset, android.text.Annotation::class.java).any { span ->
                span.key == "nativeVoidNodeType" && span.value == "mention"
            }
        }
    }

    private fun filteredMentionSuggestions(
        queryState: MentionQueryState,
        config: NativeMentionsAddonConfig
    ): List<NativeMentionSuggestion> {
        val normalizedQuery = queryState.query.trim().lowercase()
        if (normalizedQuery.isEmpty()) return config.suggestions
        return config.suggestions.filter { suggestion ->
            suggestion.title.lowercase().contains(normalizedQuery) ||
                suggestion.label.lowercase().contains(normalizedQuery) ||
                (suggestion.subtitle?.lowercase()?.contains(normalizedQuery) == true)
        }
    }

    private fun syncKeyboardToolbarMentionSuggestions(suggestions: List<NativeMentionSuggestion>) {
        keyboardToolbarView.setMentionSuggestions(suggestions)
        keyboardToolbarView.requestLayout()
        post {
            updateKeyboardToolbarLayout()
            updateEditorViewportInset()
        }
    }

    private fun emitMentionQueryChange(
        query: String,
        trigger: String,
        anchor: Int,
        head: Int,
        isActive: Boolean
    ) {
        val eventJson = JSONObject()
            .put("type", "mentionsQueryChange")
            .put("query", query)
            .put("trigger", trigger)
            .put("range", JSONObject().put("anchor", anchor).put("head", head))
            .put("isActive", isActive)
            .toString()
        if (eventJson == lastMentionEventJson) return
        lastMentionEventJson = eventJson
        onAddonEvent(mapOf("eventJson" to eventJson))
    }

    private fun emitMentionSelect(trigger: String, suggestion: NativeMentionSuggestion) {
        val eventJson = JSONObject()
            .put("type", "mentionsSelect")
            .put("trigger", trigger)
            .put("suggestionKey", suggestion.key)
            .put("attrs", suggestion.attrs)
            .toString()
        onAddonEvent(mapOf("eventJson" to eventJson))
    }

    private fun insertMentionSuggestion(suggestion: NativeMentionSuggestion) {
        val mentions = addons.mentions ?: return
        val queryState = mentionQueryState ?: return
        val attrs = JSONObject(suggestion.attrs.toString())
        if (!attrs.has("label")) {
            attrs.put("label", suggestion.label)
        }
        val docJson = JSONObject()
            .put("type", "doc")
            .put(
                "content",
                JSONArray().put(
                    JSONObject()
                        .put("type", "mention")
                        .put("attrs", attrs)
                )
            )

        val updateJson = editorInsertContentJsonAtSelectionScalar(
            richTextView.editorId.toULong(),
            queryState.anchor.toUInt(),
            queryState.head.toUInt(),
            docJson.toString()
        )
        richTextView.editorEditText.applyUpdateJSON(updateJson)
        emitMentionSelect(mentions.trigger, suggestion)
        lastMentionEventJson = null
        clearMentionQueryState()
    }

    private fun refreshToolbarStateFromEditorSelection() {
        if (richTextView.editorId == 0L) return
        val state = NativeToolbarState.fromUpdateJson(
            editorGetCurrentState(richTextView.editorId.toULong())
        ) ?: return
        toolbarState = state
        keyboardToolbarView.applyState(state)
    }

    private fun ensureKeyboardToolbarAttached() {
        val host = resolveActivity(context)?.findViewById<ViewGroup>(android.R.id.content) ?: return
        if (keyboardToolbarView.parent === host) {
            updateKeyboardToolbarLayout()
            return
        }
        detachKeyboardToolbarIfNeeded()
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

    private fun detachKeyboardToolbarIfNeeded() {
        (keyboardToolbarView.parent as? ViewGroup)?.removeView(keyboardToolbarView)
    }

    private fun updateKeyboardToolbarLayout() {
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

    private fun updateKeyboardToolbarVisibility() {
        val shouldAttach =
            showsToolbar &&
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

    private fun updateEditorViewportInset() {
        val shouldReserveToolbarSpace =
            heightBehavior == EditorHeightBehavior.FIXED &&
                showsToolbar &&
                toolbarPlacement == ToolbarPlacement.KEYBOARD &&
                richTextView.editorEditText.isEditable &&
                richTextView.editorEditText.hasFocus() &&
                currentImeBottom > 0

        if (!shouldReserveToolbarSpace) {
            richTextView.setViewportBottomInsetPx(0)
            return
        }

        val hostWidth = (resolveActivity(context)?.findViewById<ViewGroup>(android.R.id.content)?.width ?: width)
            .coerceAtLeast(0)
        val toolbarTheme = richTextView.editorEditText.theme?.toolbar
        val density = resources.displayMetrics.density
        val horizontalInsetPx = ((toolbarTheme?.resolvedHorizontalInset() ?: 0f) * density).toInt()
        if (keyboardToolbarView.measuredHeight == 0) {
            val availableWidth = (hostWidth - horizontalInsetPx * 2).coerceAtLeast(0)
            val widthSpec = MeasureSpec.makeMeasureSpec(availableWidth, MeasureSpec.AT_MOST)
            val heightSpec = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED)
            keyboardToolbarView.measure(widthSpec, heightSpec)
        }
        val toolbarHeight = keyboardToolbarView.measuredHeight.coerceAtLeast(keyboardToolbarView.height)
        richTextView.setViewportBottomInsetPx(toolbarHeight.coerceAtLeast(0))
    }

    private fun handleListToggle(listType: String) {
        val isActive = toolbarState.nodes[listType] == true
        richTextView.editorEditText.performToolbarToggleList(listType, isActive)
    }

    private fun handleToolbarItemPress(item: NativeToolbarItem) {
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
            ToolbarItemKind.action -> item.key?.let { onToolbarAction(mapOf("key" to it)) }
            ToolbarItemKind.group -> Unit
            ToolbarItemKind.separator -> Unit
        }
    }

}
