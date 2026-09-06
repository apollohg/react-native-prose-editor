package com.apollohg.editor.prototype

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Rect
import android.os.Bundle
import android.text.InputType
import android.text.TextPaint
import android.view.GestureDetector
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CursorAnchorInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import com.apollohg.editor.PositionBridge

internal class PrototypeEditorView(context: Context, val session: PrototypeDocumentSession) : ViewGroup(context), PrototypeCursorReporter {
    private val density = resources.displayMetrics.density
    private val textPaint = TextPaint(Paint.ANTI_ALIAS_FLAG).apply {
        textSize = 19f * resources.displayMetrics.scaledDensity
        color = Color.rgb(27, 43, 51)
    }
    private val selectionPaint = Paint().apply { color = 0x55498DCA }
    private val cursorPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.rgb(20, 103, 113) }
    private var atom: View? = null
    private var atomHeight = 0
    private var cursorMonitoring = false
    private var cursorVisible = true
    private var draggingSelection = false
    private var dragAnchor = 0
    private var connection: PrototypeInputConnection? = null
    var onStateChanged: (() -> Unit)? = null
    var documentLayout = makeLayout(1)
        private set

    private val blink = object : Runnable {
        override fun run() {
            cursorVisible = !cursorVisible
            if (hasFocus()) invalidate()
            if (isAttachedToWindow) postDelayed(this, 500)
        }
    }
    private val gestures = GestureDetector(context, object : GestureDetector.SimpleOnGestureListener() {
        override fun onDown(event: MotionEvent) = true
        override fun onSingleTapUp(event: MotionEvent): Boolean {
            requestFocus()
            val offset = documentLayout.offsetAt(event.x, event.y)
            session.setSelection(offset, offset)
            inputMethod.showSoftInput(this@PrototypeEditorView, InputMethodManager.SHOW_IMPLICIT)
            performClick()
            return true
        }
        override fun onLongPress(event: MotionEvent) {
            requestFocus()
            draggingSelection = true
            dragAnchor = documentLayout.offsetAt(event.x, event.y)
            session.setSelection(dragAnchor, dragAnchor)
            parent?.requestDisallowInterceptTouchEvent(true)
        }
    })
    private val inputMethod get() = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager

    init {
        isFocusable = true
        isFocusableInTouchMode = true
        importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES
        setWillNotDraw(false)
        session.onChange = {
            documentLayout = makeLayout(width.coerceAtLeast(1))
            cursorVisible = true
            requestLayout()
            invalidate()
            publishInputState()
            onStateChanged?.invoke()
        }
    }

    fun mountAtom(child: View, heightPx: Int) {
        if (atom !== child) {
            atom?.let(::removeView)
            atom = child
            addView(child)
        }
        atomHeight = heightPx.coerceAtLeast(0)
        requestLayout()
    }

    private fun makeLayout(widthPx: Int) = PrototypeBlockLayout(
        session.editable, widthPx, textPaint,
        afterBlockHeight = { if (it == 0) atomHeight else 0 },
    ) { index ->
        PrototypeInsets((if (index % 2 == 0) 16 else 48).dp, (if (index % 2 == 0) 64 else 16).dp, 16.dp, 16.dp)
    }

    private val Int.dp get() = (this * density).toInt()

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val availableWidth = MeasureSpec.getSize(widthMeasureSpec).coerceAtLeast(1)
        documentLayout = makeLayout(availableWidth)
        atom?.let {
            val bounds = documentLayout.afterBlockBounds(0)
            it.measure(MeasureSpec.makeMeasureSpec(bounds.width().toInt(), MeasureSpec.EXACTLY), MeasureSpec.makeMeasureSpec(atomHeight, MeasureSpec.EXACTLY))
        }
        setMeasuredDimension(availableWidth, resolveSize(documentLayout.height, heightMeasureSpec))
    }

    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
        atom?.let {
            val bounds = documentLayout.afterBlockBounds(0)
            it.layout(bounds.left.toInt(), bounds.top.toInt(), bounds.right.toInt(), bounds.bottom.toInt())
        }
        publishInputState()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        documentLayout.draw(canvas)
        val start = session.selectionStart
        val end = session.selectionEnd
        if (start != end) {
            canvas.drawPath(documentLayout.selection(start, end), selectionPaint)
            for (offset in listOf(start, end)) {
                val caret = documentLayout.caret(offset)
                canvas.drawCircle(caret.left, caret.bottom + 5.dp, 6.dp.toFloat(), cursorPaint)
            }
        } else if (hasFocus() && cursorVisible) {
            canvas.drawRect(documentLayout.caret(end), cursorPaint)
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN && session.selectionStart != session.selectionEnd) {
            for ((offset, opposite) in listOf(session.selectionStart to session.selectionEnd, session.selectionEnd to session.selectionStart)) {
                val caret = documentLayout.caret(offset)
                if (kotlin.math.abs(event.x - caret.left) < 24.dp && kotlin.math.abs(event.y - caret.bottom) < 24.dp) {
                    draggingSelection = true
                    dragAnchor = opposite
                    parent?.requestDisallowInterceptTouchEvent(true)
                    return true
                }
            }
        }
        if (draggingSelection) {
            if (event.actionMasked == MotionEvent.ACTION_MOVE) {
                session.setSelection(dragAnchor, documentLayout.offsetAt(event.x, event.y))
                revealCaret()
            }
            if (event.actionMasked == MotionEvent.ACTION_UP || event.actionMasked == MotionEvent.ACTION_CANCEL) {
                draggingSelection = false
                parent?.requestDisallowInterceptTouchEvent(false)
            }
            return true
        }
        return gestures.onTouchEvent(event)
    }

    override fun performClick(): Boolean { super.performClick(); return true }
    override fun onCheckIsTextEditor() = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_FLAG_NO_FULLSCREEN
        outAttrs.initialSelStart = session.selectionStart
        outAttrs.initialSelEnd = session.selectionEnd
        return PrototypeInputConnection(this, session).also { connection = it }
    }

    fun activeConnection(): InputConnection = connection?.takeIf { it.isActive } ?: onCreateInputConnection(EditorInfo())

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        val input = activeConnection()
        when (keyCode) {
            KeyEvent.KEYCODE_DEL -> return if (session.selectionStart != session.selectionEnd) input.commitText("", 1) else input.deleteSurroundingTextInCodePoints(1, 0)
            KeyEvent.KEYCODE_ENTER -> return input.commitText("\n", 1)
            KeyEvent.KEYCODE_DPAD_LEFT, KeyEvent.KEYCODE_DPAD_RIGHT -> {
                val text = session.editable.toString()
                val current = session.selectionEnd
                val next = if (keyCode == KeyEvent.KEYCODE_DPAD_LEFT) {
                    if (current == 0) 0 else android.icu.text.BreakIterator.getCharacterInstance().run { setText(text); preceding(current) }
                } else PositionBridge.snapToGraphemeBoundary((current + 1).coerceAtMost(text.length), text)
                session.setSelection(if (event.isShiftPressed) session.selectionStart else next, next)
                revealCaret()
                return true
            }
            KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_DPAD_DOWN -> {
                val next = documentLayout.moveVertically(session.selectionEnd, keyCode == KeyEvent.KEYCODE_DPAD_DOWN)
                session.setSelection(if (event.isShiftPressed) session.selectionStart else next, next)
                revealCaret()
                return true
            }
        }
        val character = event.unicodeChar
        if (character > 0 && !event.isCtrlPressed && !event.isAltPressed) return input.commitText(String(Character.toChars(character)), 1)
        return super.onKeyDown(keyCode, event)
    }

    override fun requestCursorUpdates(mode: Int): Boolean {
        if (mode and (InputConnection.CURSOR_UPDATE_IMMEDIATE or InputConnection.CURSOR_UPDATE_MONITOR).inv() != 0) return false
        cursorMonitoring = mode and InputConnection.CURSOR_UPDATE_MONITOR != 0
        if (mode and InputConnection.CURSOR_UPDATE_IMMEDIATE != 0) publishCursor()
        return true
    }

    fun onViewportChanged() {
        if (cursorMonitoring) publishCursor()
    }

    private fun publishInputState() {
        val composingStart = BaseInputConnection.getComposingSpanStart(session.editable)
        val composingEnd = BaseInputConnection.getComposingSpanEnd(session.editable)
        inputMethod.updateSelection(this, session.selectionStart, session.selectionEnd, composingStart, composingEnd)
        if (cursorMonitoring) publishCursor()
        sendAccessibilityEvent(AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED)
    }

    private fun publishCursor() {
        if (!isAttachedToWindow) return
        val caret = documentLayout.caret(session.selectionEnd)
        val location = IntArray(2).also(::getLocationOnScreen)
        val builder = CursorAnchorInfo.Builder()
            .setMatrix(Matrix().apply { setTranslate(location[0].toFloat(), location[1].toFloat()) })
            .setSelectionRange(session.selectionStart, session.selectionEnd)
            .setInsertionMarkerLocation(caret.left, caret.top, caret.bottom + textPaint.fontMetrics.descent * -1, caret.bottom, CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION)
        val start = BaseInputConnection.getComposingSpanStart(session.editable)
        val end = BaseInputConnection.getComposingSpanEnd(session.editable)
        if (start >= 0 && end >= start) builder.setComposingText(start, session.editable.subSequence(start, end))
        inputMethod.updateCursorAnchorInfo(this, builder.build())
    }

    fun revealCaret() {
        val caret = documentLayout.caret(session.selectionEnd)
        val rect = Rect(caret.left.toInt() - 8.dp, caret.top.toInt() - 8.dp, caret.right.toInt() + 8.dp, caret.bottom.toInt() + 20.dp)
        requestRectangleOnScreen(rect, false)
    }

    override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
        super.onInitializeAccessibilityNodeInfo(info)
        info.className = "android.widget.EditText"
        info.text = session.editable
        info.isEditable = true
        info.isMultiLine = true
        info.setTextSelection(session.selectionStart, session.selectionEnd)
        info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_SELECTION)
        info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_SET_TEXT)
        info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_COPY)
        info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_PASTE)
    }

    override fun performAccessibilityAction(action: Int, arguments: Bundle?): Boolean {
        when (action) {
            AccessibilityNodeInfo.ACTION_SET_SELECTION -> {
                val start = arguments?.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, -1) ?: -1
                val end = arguments?.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, -1) ?: -1
                if (start !in 0..session.editable.length || end !in 0..session.editable.length) return false
                session.setSelection(start, end)
                return true
            }
            AccessibilityNodeInfo.ACTION_SET_TEXT -> {
                session.setSelection(0, session.editable.length)
                return activeConnection().commitText(arguments?.getCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE) ?: "", 1)
            }
            AccessibilityNodeInfo.ACTION_COPY -> {
                val from = minOf(session.selectionStart, session.selectionEnd)
                val to = maxOf(session.selectionStart, session.selectionEnd)
                (context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("Selection", session.editable.subSequence(from, to)))
                return true
            }
            AccessibilityNodeInfo.ACTION_PASTE -> {
                val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                val text = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context) ?: return false
                return activeConnection().commitText(text, 1)
            }
        }
        return super.performAccessibilityAction(action, arguments)
    }

    override fun onAttachedToWindow() { super.onAttachedToWindow(); post(blink) }
    override fun onDetachedFromWindow() {
        removeCallbacks(blink)
        connection?.closeConnection()
        connection = null
        super.onDetachedFromWindow()
    }
}
