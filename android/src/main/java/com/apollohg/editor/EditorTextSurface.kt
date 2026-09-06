package com.apollohg.editor

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.Rect
import android.graphics.Typeface
import android.graphics.drawable.Drawable
import android.os.Bundle
import android.text.Editable
import android.text.InputType
import android.text.Layout
import android.text.NoCopySpan
import android.text.Selection
import android.text.SpanWatcher
import android.text.Spannable
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.TextPaint
import android.text.TextUtils
import android.text.TextWatcher
import android.text.method.KeyListener
import android.text.method.TextKeyListener
import android.util.AttributeSet
import android.util.TypedValue
import android.view.ActionMode
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.widget.TextView.BufferType

open class EditorTextSurface @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0,
) : View(context, attrs, defStyleAttr) {
    val paint = TextPaint(Paint.ANTI_ALIAS_FLAG).apply {
        density = resources.displayMetrics.density
        textSize = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, 16f, resources.displayMetrics)
        color = Color.BLACK
    }
    private var buffer = SpannableStringBuilder()
    private val watchers = mutableListOf<TextWatcher>()
    private var cachedLayout: EditorDocumentLayout? = null
    private var layoutDirty = true
    private var layoutWidth = -1
    private var textChangeDepth = 0
    private var batchDepth = 0
    private var pendingInputState = false
    private var textBeforeAccessibilityChange: CharSequence = ""
    private var lastSelectionStart = -1
    private var lastSelectionEnd = -1
    private var replacingBuffer = false
    private var baseInputConnection: EditorSurfaceInputConnection? = null
    private var monitorCursor = false
    private var cursorPublicationPosted = false
    internal var documentLayoutBuildCount = 0
        private set
    private val publishCursor = Runnable {
        cursorPublicationPosted = false
        if (monitorCursor) publishSurfaceCursor()
    }
    private var blinkVisible = true
    internal var selectionActionMode: ActionMode? = null
    internal val interaction by lazy { EditorTextSurfaceInteraction(this) }
    private val dragDrop by lazy { EditorTextSurfaceDragDrop(this) }
    private val selectionPaint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val selectionPath = Path()

    var text: Editable
        get() = buffer
        set(value) { setText(value as CharSequence) }
    val editableText: Editable get() = buffer
    val selectionStart: Int get() = Selection.getSelectionStart(buffer)
    val selectionEnd: Int get() = Selection.getSelectionEnd(buffer)
    val layout: Layout
        get() {
            val available = ((if (measuredWidth > 0) measuredWidth else width) - compoundPaddingLeft - compoundPaddingRight).coerceAtLeast(1)
            if (layoutDirty || layoutWidth != available || cachedLayout == null) {
                documentLayoutBuildCount++
                cachedLayout = EditorDocumentLayout(buffer, paint, available, includeFontPadding, lineSpacingMultiplier, lineSpacingExtra, cachedLayout)
                layoutWidth = available
                layoutDirty = false
            }
            return requireNotNull(cachedLayout)
        }
    var textSize: Float
        get() = paint.textSize
        set(value) { paint.textSize = value; invalidateTextLayout() }
    var typeface: Typeface?
        get() = paint.typeface
        set(value) { paint.typeface = value; invalidateTextLayout() }
    var letterSpacing: Float
        get() = paint.letterSpacing
        set(value) { paint.letterSpacing = value; invalidateTextLayout() }
    var inputType: Int = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE
    var imeOptions: Int = EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_FLAG_NO_FULLSCREEN
    var privateImeOptions: String? = null
    var keyListener: KeyListener? = TextKeyListener.getInstance()
    var gravity: Int = android.view.Gravity.TOP or android.view.Gravity.START
    var linksClickable = false
    var showSoftInputOnFocus = true
    var includeFontPadding = false
        set(value) { if (field != value) { field = value; invalidateTextLayout() } }
    var isCursorVisible = true
        set(value) { field = value; invalidate() }
    var textCursorDrawable: Drawable? = null
    var highlightColor: Int = 0x6633B5E5
        set(value) { field = value; invalidate() }
    var textColors: ColorStateList = ColorStateList.valueOf(Color.BLACK)
        private set
    var hintTextColors: ColorStateList = ColorStateList.valueOf(Color.GRAY)
        private set
    val currentTextColor get() = textColors.getColorForState(drawableState, textColors.defaultColor)
    val currentHintTextColor get() = hintTextColors.getColorForState(drawableState, hintTextColors.defaultColor)
    var hint: CharSequence? = null
    var minHeight: Int
        get() = minimumHeight
        set(value) { minimumHeight = value }
    var minLines = 1
        set(value) { field = value.coerceAtLeast(0); requestLayout() }
    var maxLines = Int.MAX_VALUE
        set(value) { field = value.coerceAtLeast(1); requestLayout() }
    var lineSpacingMultiplier = 1f
        private set
    var lineSpacingExtra = 0f
        private set
    val lineHeight get() = (paint.fontMetricsInt.run { descent - ascent } * lineSpacingMultiplier + lineSpacingExtra).toInt().coerceAtLeast(1)
    val lineCount get() = layout.lineCount
    val compoundPaddingLeft get() = paddingLeft
    val compoundPaddingRight get() = paddingRight
    val compoundPaddingTop get() = paddingTop
    val compoundPaddingBottom get() = paddingBottom
    val totalPaddingLeft get() = paddingLeft
    val totalPaddingRight get() = paddingRight
    val totalPaddingTop get() = paddingTop
    val totalPaddingBottom get() = paddingBottom
    val extendedPaddingTop get() = paddingTop
    val extendedPaddingBottom get() = paddingBottom

    private val changeWatcher = object : TextWatcher, SpanWatcher, NoCopySpan {
        override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {
            textBeforeAccessibilityChange = s?.toString() ?: ""
            textChangeDepth++
            watchers.toList().forEach { it.beforeTextChanged(s, start, count, after) }
        }
        override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
            invalidateTextLayout()
            watchers.toList().forEach { it.onTextChanged(s, start, before, count) }
            val event = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED)
            event.beforeText = textBeforeAccessibilityChange
            event.fromIndex = start
            event.removedCount = before
            event.addedCount = count
            sendSurfaceAccessibilityEvent(event)
        }
        override fun afterTextChanged(s: Editable?) {
            try { watchers.toList().forEach { it.afterTextChanged(s) } }
            finally {
                textChangeDepth = (textChangeDepth - 1).coerceAtLeast(0)
                notifySelectionChanged()
                notifyInputStateChanged()
            }
        }
        override fun onSpanAdded(text: Spannable?, what: Any?, start: Int, end: Int) = spanChanged(text, what)
        override fun onSpanRemoved(text: Spannable?, what: Any?, start: Int, end: Int) = spanChanged(text, what)
        override fun onSpanChanged(text: Spannable?, what: Any?, oldStart: Int, oldEnd: Int, newStart: Int, newEnd: Int) = spanChanged(text, what)
        private fun spanChanged(text: Spannable?, what: Any?) {
            if (what === this || replacingBuffer) return
            if (what === Selection.SELECTION_START || what === Selection.SELECTION_END) {
                if ((text?.getSpanFlags(what) ?: 0) and Spanned.SPAN_INTERMEDIATE == 0) notifySelectionChanged()
            } else {
                invalidateTextLayout()
                notifyInputStateChanged()
            }
        }
    }

    private val blink = object : Runnable {
        override fun run() {
            blinkVisible = !blinkVisible
            if (hasFocus() && isCursorVisible) invalidate()
            if (isAttachedToWindow && hasFocus()) postDelayed(this, 500)
        }
    }

    init {
        isFocusable = true
        isFocusableInTouchMode = true
        isClickable = true
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES
        Selection.setSelection(buffer, 0)
        lastSelectionStart = 0
        lastSelectionEnd = 0
        buffer.setSpan(changeWatcher, 0, 0, Spanned.SPAN_INCLUSIVE_INCLUSIVE)
    }

    @JvmOverloads
    fun setText(value: CharSequence?, type: BufferType = BufferType.EDITABLE) {
        val previousText = buffer.toString()
        val next = SpannableStringBuilder(value ?: "")
        watchers.toList().forEach { it.beforeTextChanged(buffer, 0, buffer.length, next.length) }
        val before = buffer.length
        replacingBuffer = true
        try {
            buffer.removeSpan(changeWatcher)
            buffer = next
            buffer.setSpan(changeWatcher, 0, buffer.length, Spanned.SPAN_INCLUSIVE_INCLUSIVE)
            if (Selection.getSelectionStart(buffer) < 0) Selection.setSelection(buffer, 0)
        } finally { replacingBuffer = false }
        invalidateTextLayout()
        textChangeDepth++
        try {
            watchers.toList().forEach { it.onTextChanged(buffer, 0, before, buffer.length) }
            watchers.toList().forEach { it.afterTextChanged(buffer) }
        } finally { textChangeDepth-- }
        if (previousText != buffer.toString()) {
            val event = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED)
            event.beforeText = previousText
            event.fromIndex = 0
            event.removedCount = previousText.length
            event.addedCount = buffer.length
            sendSurfaceAccessibilityEvent(event)
        }
        notifySelectionChanged()
        notifyInputStateChanged()
    }

    fun length() = buffer.length
    fun setSelection(index: Int) = setSelection(index, index)
    fun setSelection(start: Int, end: Int) {
        require(start in 0..buffer.length && end in 0..buffer.length)
        Selection.setSelection(buffer, start, end)
    }
    fun selectAll() = setSelection(0, buffer.length)
    fun extendSelection(index: Int) = Selection.extendSelection(buffer, index)
    fun addTextChangedListener(watcher: TextWatcher) { if (watcher !in watchers) watchers += watcher }
    fun removeTextChangedListener(watcher: TextWatcher) { watchers -= watcher }
    fun setRawInputType(value: Int) { inputType = value }
    fun setTextColor(value: Int) { textColors = ColorStateList.valueOf(value); paint.color = value; invalidateTextLayout() }
    fun setTextColor(value: ColorStateList) { textColors = value; paint.color = currentTextColor; invalidateTextLayout() }
    fun setHintTextColor(value: Int) { hintTextColors = ColorStateList.valueOf(value); invalidate() }
    fun setTextSize(unit: Int, value: Float) { textSize = TypedValue.applyDimension(unit, value, resources.displayMetrics) }
    fun setTypeface(value: Typeface?, style: Int) { typeface = Typeface.create(value, style) }
    fun setLineSpacing(add: Float, mult: Float) { lineSpacingExtra = add; lineSpacingMultiplier = mult; invalidateTextLayout() }
    fun setCompoundDrawablesRelativeWithIntrinsicBounds(start: Drawable?, top: Drawable?, end: Drawable?, bottom: Drawable?) {
        require(start == null && top == null && end == null && bottom == null) { "Editor content insets own text padding." }
    }

    internal fun invalidateTextLayout() {
        layoutDirty = true
        requestLayout()
        invalidate()
    }

    private fun notifySelectionChanged() {
        if (replacingBuffer || textChangeDepth > 0) return
        val start = selectionStart
        val end = selectionEnd
        if (start == lastSelectionStart && end == lastSelectionEnd) return
        lastSelectionStart = start
        lastSelectionEnd = end
        blinkVisible = true
        onSelectionChanged(start, end)
        interaction.selectionChanged()
        notifyInputStateChanged()
        invalidate()
    }

    protected open fun onSelectionChanged(selStart: Int, selEnd: Int) = Unit
    protected open fun onSurfaceInputStateChanged() = Unit

    fun beginBatchEdit(): Boolean { batchDepth++; return true }
    fun endBatchEdit(): Boolean {
        if (batchDepth == 0) return false
        batchDepth--
        if (batchDepth == 0 && pendingInputState) notifyInputStateChanged()
        return batchDepth > 0
    }

    internal fun notifyInputStateChanged() {
        if (batchDepth > 0 || textChangeDepth > 0 || replacingBuffer) { pendingInputState = true; return }
        pendingInputState = false
        onSurfaceInputStateChanged()
        scheduleSurfaceCursorPublication()
    }

    internal fun requestSurfaceCursorUpdates(mode: Int): Boolean {
        if (mode and (InputConnection.CURSOR_UPDATE_IMMEDIATE or InputConnection.CURSOR_UPDATE_MONITOR).inv() != 0) return false
        monitorCursor = mode and InputConnection.CURSOR_UPDATE_MONITOR != 0
        if (mode and InputConnection.CURSOR_UPDATE_IMMEDIATE != 0) publishSurfaceCursor()
        return true
    }

    internal fun surfaceViewportChanged() {
        scheduleSurfaceCursorPublication()
        interaction.viewportChanged()
    }

    private fun scheduleSurfaceCursorPublication() {
        if (!monitorCursor || cursorPublicationPosted || !isAttachedToWindow) return
        cursorPublicationPosted = true
        postOnAnimation(publishCursor)
    }

    private fun publishSurfaceCursor() {
        if (!isAttachedToWindow || !hasFocus()) return
        val editor = this as? EditorEditText ?: return
        val info = editor.buildSurfaceCursorAnchorInfo()
        inputMethod.updateCursorAnchorInfo(this, info)
    }

    internal val inputMethod get() = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager

    override fun onCheckIsTextEditor() = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
        if (!isEnabled) return null
        baseInputConnection?.closeConnection()
        monitorCursor = false
        outAttrs.inputType = inputType
        outAttrs.imeOptions = imeOptions
        outAttrs.privateImeOptions = privateImeOptions
        outAttrs.initialSelStart = selectionStart.coerceAtLeast(0)
        outAttrs.initialSelEnd = selectionEnd.coerceAtLeast(0)
        outAttrs.initialCapsMode = TextUtils.getCapsMode(buffer, outAttrs.initialSelStart, inputType)
        outAttrs.packageName = context.packageName
        outAttrs.fieldId = id
        androidx.core.view.inputmethod.EditorInfoCompat.setInitialSurroundingText(outAttrs, buffer)
        return EditorSurfaceInputConnection(this).also { baseInputConnection = it }
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val measured = resolveSize(suggestedMinimumWidth.coerceAtLeast(1), widthMeasureSpec)
        val available = (measured - compoundPaddingLeft - compoundPaddingRight).coerceAtLeast(1)
        if (layoutDirty || layoutWidth != available || cachedLayout == null) {
            documentLayoutBuildCount++
            cachedLayout = EditorDocumentLayout(buffer, paint, available, includeFontPadding, lineSpacingMultiplier, lineSpacingExtra, cachedLayout)
            layoutDirty = false
            layoutWidth = available
        }
        val content = requireNotNull(cachedLayout)
        val textHeight = if (content.lineCount > maxLines) content.getLineTop(maxLines) else content.height
        val desired = maxOf(textHeight, minLines * lineHeight) + compoundPaddingTop + compoundPaddingBottom
        setMeasuredDimension(measured, resolveSize(maxOf(desired, suggestedMinimumHeight), heightMeasureSpec))
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val layout = layout
        val save = canvas.save()
        canvas.translate(compoundPaddingLeft.toFloat(), compoundPaddingTop.toFloat())
        val start = selectionStart
        val end = selectionEnd
        val selection = if (start >= 0 && end >= 0 && start != end && hasFocus()) {
            selectionPath.reset()
            layout.getSelectionPath(minOf(start, end), maxOf(start, end), selectionPath)
            selectionPaint.color = highlightColor
            selectionPath
        } else null
        layout.draw(canvas, selection, selectionPaint, 0)
        if (isCursorVisible && blinkVisible && start >= 0 && start == end && hasFocus() && hasWindowFocus()) {
            val editor = this as? EditorEditText
            editor?.nativeCursorDrawRect()?.let { rect ->
                selectionPaint.color = editor.caretColor
                canvas.drawRect(rect, selectionPaint)
            }
        }
        interaction.drawHandles(canvas)
        canvas.restoreToCount(save)
    }

    fun getOffsetForPosition(x: Float, y: Float): Int {
        val layout = layout
        val line = layout.getLineForVertical((y + scrollY - totalPaddingTop).toInt())
        return PositionBridge.snapToGraphemeBoundary(layout.getOffsetForHorizontal(line, x + scrollX - totalPaddingLeft), buffer.toString())
    }

    fun bringPointIntoView(offset: Int): Boolean {
        val layout = layout
        val safe = offset.coerceIn(0, buffer.length)
        val line = layout.getLineForOffset(safe)
        val x = layout.getPrimaryHorizontal(safe).toInt() + totalPaddingLeft
        val rect = Rect(x, layout.editorTextLineTop(line) + totalPaddingTop, x + 2, layout.editorTextLineBottom(line) + totalPaddingTop)
        var scrolled = false
        if (!interaction.hasScrollContainer() && height > 0) {
            val limit = (layout.height + totalPaddingTop + totalPaddingBottom - height).coerceAtLeast(0)
            val next = when {
                rect.top < scrollY + totalPaddingTop -> rect.top - totalPaddingTop
                rect.bottom > scrollY + height - totalPaddingBottom -> rect.bottom - height + totalPaddingBottom
                else -> scrollY
            }.coerceIn(0, limit)
            scrolled = next != scrollY
            if (scrolled) scrollTo(scrollX, next)
        }
        return requestRectangleOnScreen(rect) || scrolled
    }

    override fun onDragEvent(event: android.view.DragEvent) = dragDrop.onDragEvent(event)
    override fun onTouchEvent(event: MotionEvent) = interaction.onTouchEvent(event)
    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean = interaction.onKeyDown(keyCode, event) || super.onKeyDown(keyCode, event)
    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean = keyListener?.onKeyUp(this, buffer, keyCode, event) == true || super.onKeyUp(keyCode, event)
    override fun performClick(): Boolean { super.performClick(); return true }

    open fun onTextContextMenuItem(id: Int): Boolean {
        return when (id) {
            android.R.id.selectAll -> { selectAll(); true }
            android.R.id.copy, android.R.id.cut -> {
                val from = minOf(selectionStart, selectionEnd).coerceAtLeast(0)
                val to = maxOf(selectionStart, selectionEnd).coerceAtLeast(from)
                (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("", buffer.subSequence(from, to)))
                if (id == android.R.id.cut) buffer.delete(from, to)
                true
            }
            android.R.id.paste, android.R.id.pasteAsPlainText -> {
                val clip = (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).primaryClip
                val value = if (clip != null && clip.itemCount > 0) clip.getItemAt(0).coerceToText(context) else null
                if (value == null) false else {
                    val from = minOf(selectionStart, selectionEnd).coerceAtLeast(0)
                    val to = maxOf(selectionStart, selectionEnd).coerceAtLeast(from)
                    buffer.replace(from, to, value)
                    setSelection(from + value.length)
                    true
                }
            }
            else -> false
        }
    }

    internal fun sendSurfaceAccessibilityEvent(event: AccessibilityEvent) {
        val manager = context.getSystemService(Context.ACCESSIBILITY_SERVICE) as android.view.accessibility.AccessibilityManager
        if (manager.isEnabled) sendAccessibilityEventUnchecked(event) else event.recycle()
    }

    override fun onInitializeAccessibilityEvent(event: AccessibilityEvent) {
        super.onInitializeAccessibilityEvent(event)
        event.className = "android.widget.EditText"
        event.text.add(buffer.toString())
        if (event.eventType == AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED) {
            event.fromIndex = selectionStart
            event.toIndex = selectionEnd
            event.itemCount = buffer.length
        }
    }

    override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
        super.onInitializeAccessibilityNodeInfo(info)
        interaction.initializeAccessibility(info)
    }

    override fun performAccessibilityAction(action: Int, arguments: Bundle?): Boolean = interaction.performAccessibilityAction(action, arguments) || super.performAccessibilityAction(action, arguments)

    override fun onFocusChanged(gainFocus: Boolean, direction: Int, previouslyFocusedRect: Rect?) {
        super.onFocusChanged(gainFocus, direction, previouslyFocusedRect)
        blinkVisible = true
        removeCallbacks(blink)
        if (gainFocus && isAttachedToWindow) postDelayed(blink, 500)
        if (!gainFocus) interaction.endSelectionActionMode()
        invalidate()
    }

    override fun onAttachedToWindow() { super.onAttachedToWindow(); if (hasFocus()) postDelayed(blink, 500) }
    override fun onDetachedFromWindow() {
        removeCallbacks(blink)
        removeCallbacks(publishCursor)
        cursorPublicationPosted = false
        baseInputConnection?.closeConnection()
        baseInputConnection = null
        monitorCursor = false
        interaction.dispose()
        dragDrop.dispose()
        super.onDetachedFromWindow()
    }
    override fun onScrollChanged(l: Int, t: Int, oldl: Int, oldt: Int) {
        super.onScrollChanged(l, t, oldl, oldt)
        surfaceViewportChanged()
    }
    override fun computeVerticalScrollRange() = if (width <= 0 && measuredWidth <= 0) height else layout.height + totalPaddingTop + totalPaddingBottom
    override fun computeHorizontalScrollRange() = width
}
