package com.apollohg.editor

import android.content.ClipData
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.Point
import android.text.StaticLayout
import android.text.TextPaint
import android.os.Bundle
import android.text.Selection
import android.view.ActionMode
import android.view.GestureDetector
import android.view.KeyEvent
import android.view.Menu
import android.view.MenuItem
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.InputMethodManager
import java.text.BreakIterator
import kotlin.math.abs

internal class EditorTextSurfaceInteraction(private val view: EditorTextSurface) {
    private val density get() = view.resources.displayMetrics.density
    private val handlePaint = Paint(Paint.ANTI_ALIAS_FLAG)
    private var draggingHandle = 0
    private var insertionHandleVisible = false
    private var handleDragOffsetY = 0f
    private var downX = 0f
    private var downY = 0f
    private var lastY = 0f
    private var moved = false
    private var longPressed = false
    private var selectionAnchor = 0
    private val slop = ViewConfiguration.get(view.context).scaledTouchSlop
    private val gestures = GestureDetector(view.context, object : GestureDetector.SimpleOnGestureListener() {
        override fun onDown(e: MotionEvent) = true
        override fun onSingleTapUp(e: MotionEvent): Boolean {
            if (moved || draggingHandle != 0) return true
            view.requestFocus()
            view.setSelection(view.getOffsetForPosition(e.x, e.y))
            endSelectionActionMode()
            insertionHandleVisible = true
            showKeyboard()
            view.performClick()
            return true
        }
        override fun onDoubleTap(e: MotionEvent): Boolean { selectWord(e.x, e.y); return true }
        override fun onLongPress(e: MotionEvent) {
            if (draggingHandle != 0 || moved) return
            val offset = view.getOffsetForPosition(e.x, e.y)
            val from = minOf(view.selectionStart, view.selectionEnd)
            val to = maxOf(view.selectionStart, view.selectionEnd)
            if (from >= 0 && from < to && offset in from until to && (view as? EditorEditText)?.isEditable != false) {
                val clip = ClipData.newPlainText("", view.text.subSequence(from, to))
                if (view.startDragAndDrop(clip, selectionDragShadow(from, to), view, 0)) {
                    endSelectionActionMode()
                    return
                }
            }
            longPressed = true
            selectWord(e.x, e.y)
            selectionAnchor = view.selectionStart
            view.parent?.requestDisallowInterceptTouchEvent(true)
        }
    })

    fun onTouchEvent(event: MotionEvent): Boolean {
        if (!view.isEnabled) return false
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = event.x; downY = event.y; lastY = event.y
                moved = false; longPressed = false
                draggingHandle = hitHandle(event.x, event.y)
                if (draggingHandle != 0) {
                    val offset = if (draggingHandle == 1) view.selectionStart else view.selectionEnd
                    handleDragOffsetY = event.y + view.scrollY - view.totalPaddingTop - handlePosition(offset).second
                    view.parent?.requestDisallowInterceptTouchEvent(true)
                }
            }
            MotionEvent.ACTION_MOVE -> {
                if (abs(event.x - downX) > slop || abs(event.y - downY) > slop) moved = true
                if (draggingHandle != 0) {
                    val offset = view.getOffsetForPosition(event.x, event.y - handleDragOffsetY - 1f)
                    if (draggingHandle == 3) view.setSelection(offset)
                    else if (draggingHandle == 1) view.setSelection(offset, view.selectionEnd.coerceAtLeast(0))
                    else view.setSelection(view.selectionStart.coerceAtLeast(0), offset)
                    view.bringPointIntoView(offset)
                } else if (longPressed && moved) {
                    val offset = view.getOffsetForPosition(event.x, event.y)
                    view.setSelection(selectionAnchor, offset)
                    view.bringPointIntoView(offset)
                } else if (moved && !hasScrollContainer() && (view as? EditorEditText)?.heightBehavior == EditorHeightBehavior.FIXED) {
                    val limit = (view.layout.height + view.totalPaddingTop + view.totalPaddingBottom - view.height).coerceAtLeast(0)
                    view.scrollTo(0, (view.scrollY + lastY - event.y).toInt().coerceIn(0, limit))
                }
                lastY = event.y
            }
        }
        gestures.onTouchEvent(event)
        if (event.actionMasked == MotionEvent.ACTION_UP || event.actionMasked == MotionEvent.ACTION_CANCEL) {
            if (draggingHandle != 0 && (view.selectionStart != view.selectionEnd || (!moved && event.actionMasked == MotionEvent.ACTION_UP))) startSelectionActionMode()
            draggingHandle = 0
            view.parent?.requestDisallowInterceptTouchEvent(false)
        }
        return true
    }

    internal fun hasScrollContainer(): Boolean {
        var parent = view.parent
        while (parent != null) {
            if (parent is android.widget.ScrollView) return true
            parent = parent.parent
        }
        return false
    }

    private fun showKeyboard() {
        if (view.showSoftInputOnFocus && (view as? EditorEditText)?.isEditable != false) view.inputMethod.showSoftInput(view, InputMethodManager.SHOW_IMPLICIT)
    }

    private fun selectWord(x: Float, y: Float) {
        view.requestFocus()
        val text = view.text.toString()
        if (text.isEmpty()) { view.setSelection(0); startSelectionActionMode(); showKeyboard(); return }
        val offset = view.getOffsetForPosition(x, y).coerceAtMost(text.length - 1)
        val iterator = BreakIterator.getWordInstance().apply { setText(text) }
        val start = if (iterator.isBoundary(offset)) offset else iterator.preceding(offset).coerceAtLeast(0)
        val end = iterator.following(offset).let { if (it == BreakIterator.DONE) text.length else it }
        view.setSelection(start, end)
        view.performHapticFeedback(android.view.HapticFeedbackConstants.LONG_PRESS)
        startSelectionActionMode()
    }

    private fun selectionDragShadow(from: Int, to: Int): View.DragShadowBuilder {
        val text = view.text.subSequence(from, minOf(to, from + 256))
        val layout = StaticLayout.Builder.obtain(text, 0, text.length, TextPaint(view.paint), minOf(view.width, (240 * density).toInt()).coerceAtLeast(1))
            .setMaxLines(3)
            .setIncludePad(false)
            .build()
        return object : View.DragShadowBuilder(view) {
            override fun onProvideShadowMetrics(size: Point, touch: Point) {
                size.set(layout.width, layout.height)
                touch.set(layout.width / 2, layout.height / 2)
            }
            override fun onDrawShadow(canvas: Canvas) { layout.draw(canvas) }
        }
    }

    private fun handlePosition(offset: Int): Pair<Float, Float> {
        val layout = view.layout
        val safe = offset.coerceIn(0, view.text.length)
        val line = layout.getLineForOffset(safe)
        return layout.getPrimaryHorizontal(safe) to layout.editorTextLineBottom(line).toFloat()
    }

    private fun handleCenter(offset: Int): Pair<Float, Float> {
        val (x, bottom) = handlePosition(offset)
        val radius = 6f * density
        val maxY = view.layout.height + view.totalPaddingBottom - radius
        val centerY = (bottom + 10f * density).coerceAtMost(maxY).coerceAtLeast(radius)
        return x.coerceIn(radius - view.totalPaddingLeft, maxOf(radius - view.totalPaddingLeft, view.width - view.totalPaddingLeft - radius)) to centerY
    }

    private fun hitHandle(x: Float, y: Float): Int {
        if (!view.hasFocus() || view.selectionStart < 0 || (view.selectionStart == view.selectionEnd && !insertionHandleVisible)) return 0
        val localX = x + view.scrollX - view.totalPaddingLeft
        val localY = y + view.scrollY - view.totalPaddingTop
        val radius = 24f * density
        val offsets = if (view.selectionStart == view.selectionEnd) listOf(view.selectionEnd) else listOf(view.selectionStart, view.selectionEnd)
        for ((index, offset) in offsets.withIndex()) {
            val (hx, hy) = handleCenter(offset)
            if (abs(localX - hx) <= radius && abs(localY - hy) <= radius) return if (offsets.size == 1) 3 else index + 1
        }
        return 0
    }

    fun drawHandles(canvas: Canvas) {
        if (!view.hasFocus() || view.selectionStart < 0 || (view.selectionStart == view.selectionEnd && !insertionHandleVisible)) return
        handlePaint.color = (view as? EditorEditText)?.caretColor ?: view.currentTextColor
        for (offset in listOf(view.selectionStart, view.selectionEnd).distinct()) {
            val (x, y) = handleCenter(offset)
            val lineBottom = handlePosition(offset).second
            canvas.drawRect(x - density, minOf(lineBottom, y), x + density, maxOf(lineBottom, y), handlePaint)
            canvas.drawCircle(x, y, 6f * density, handlePaint)
        }
    }

    fun selectionChanged() {
        if (view.selectionStart == view.selectionEnd) {
            endSelectionActionMode()
            if (draggingHandle != 3) insertionHandleVisible = false
        }
        else view.selectionActionMode?.invalidateContentRect()
        view.sendAccessibilityEvent(AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED)
    }
    fun viewportChanged() { view.selectionActionMode?.invalidateContentRect() }
    fun dispose() { endSelectionActionMode(); draggingHandle = 0; insertionHandleVisible = false }
    fun endSelectionActionMode() { view.selectionActionMode?.finish(); view.selectionActionMode = null }

    private fun startSelectionActionMode() {
        if (view.selectionActionMode != null) { view.selectionActionMode?.invalidate(); return }
        view.selectionActionMode = view.startActionMode(object : ActionMode.Callback2() {
            override fun onCreateActionMode(mode: ActionMode, menu: Menu): Boolean {
                menu.add(0, android.R.id.copy, 0, android.R.string.copy).setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM)
                if ((view as? EditorEditText)?.isEditable != false) {
                    menu.add(0, android.R.id.cut, 1, android.R.string.cut).setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM)
                    menu.add(0, android.R.id.paste, 2, android.R.string.paste).setShowAsAction(MenuItem.SHOW_AS_ACTION_IF_ROOM)
                }
                menu.add(0, android.R.id.selectAll, 3, android.R.string.selectAll)
                return true
            }
            override fun onPrepareActionMode(mode: ActionMode, menu: Menu) = false
            override fun onActionItemClicked(mode: ActionMode, item: MenuItem): Boolean {
                val handled = view.onTextContextMenuItem(item.itemId)
                if (item.itemId != android.R.id.selectAll) mode.finish()
                return handled
            }
            override fun onDestroyActionMode(mode: ActionMode) { view.selectionActionMode = null }
            override fun onGetContentRect(mode: ActionMode, target: View, outRect: Rect) {
                val layout = view.layout
                val from = minOf(view.selectionStart, view.selectionEnd).coerceIn(0, view.text.length)
                val to = maxOf(view.selectionStart, view.selectionEnd).coerceIn(from, view.text.length)
                val bounds = android.graphics.RectF()
                if (from != to) {
                    val path = android.graphics.Path()
                    layout.getSelectionPath(from, to, path)
                    path.computeBounds(bounds, true)
                } else {
                    val line = layout.getLineForOffset(from)
                    val x = layout.getPrimaryHorizontal(from)
                    bounds.set(x, layout.editorTextLineTop(line).toFloat(), x + 1, layout.editorTextLineBottom(line).toFloat())
                }
                bounds.offset((view.totalPaddingLeft - view.scrollX).toFloat(), (view.totalPaddingTop - view.scrollY).toFloat())
                bounds.roundOut(outRect)
            }
        }, ActionMode.TYPE_FLOATING)
    }

    fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        if (event.isCtrlPressed) {
            val action = when (keyCode) {
                KeyEvent.KEYCODE_A -> android.R.id.selectAll
                KeyEvent.KEYCODE_C -> android.R.id.copy
                KeyEvent.KEYCODE_X -> android.R.id.cut
                KeyEvent.KEYCODE_V -> android.R.id.paste
                else -> 0
            }
            if (action != 0) return view.onTextContextMenuItem(action)
        }
        val content = view.editableText
        val layout = view.layout
        if (keyCode == KeyEvent.KEYCODE_DPAD_UP || keyCode == KeyEvent.KEYCODE_DPAD_DOWN) {
            if (event.isShiftPressed) {
                if (keyCode == KeyEvent.KEYCODE_DPAD_UP) Selection.extendUp(content, layout) else Selection.extendDown(content, layout)
            } else {
                if (keyCode == KeyEvent.KEYCODE_DPAD_UP) Selection.moveUp(content, layout) else Selection.moveDown(content, layout)
            }
            view.bringPointIntoView(view.selectionEnd.coerceAtLeast(0))
            return true
        }
        if (keyCode == KeyEvent.KEYCODE_DPAD_LEFT || keyCode == KeyEvent.KEYCODE_DPAD_RIGHT) {
            if (event.isShiftPressed) {
                if (keyCode == KeyEvent.KEYCODE_DPAD_LEFT) Selection.extendLeft(content, layout) else Selection.extendRight(content, layout)
            } else {
                if (keyCode == KeyEvent.KEYCODE_DPAD_LEFT) Selection.moveLeft(content, layout) else Selection.moveRight(content, layout)
            }
            view.bringPointIntoView(view.selectionEnd.coerceAtLeast(0))
            return true
        }
        val offset = view.selectionEnd.coerceAtLeast(0)
        val line = layout.getLineForOffset(offset)
        val next = when (keyCode) {
            KeyEvent.KEYCODE_MOVE_HOME -> if (event.isCtrlPressed) 0 else layout.getLineStart(line)
            KeyEvent.KEYCODE_MOVE_END -> if (event.isCtrlPressed) content.length else layout.getLineVisibleEnd(line)
            else -> null
        }
        if (next != null) {
            if (event.isShiftPressed) Selection.extendSelection(content, next) else view.setSelection(next)
            view.bringPointIntoView(next)
            return true
        }
        if ((view as? EditorEditText)?.isEditable == false) return false
        return view.keyListener?.onKeyDown(view, content, keyCode, event) == true
    }

    fun initializeAccessibility(info: AccessibilityNodeInfo) {
        info.className = "android.widget.EditText"
        info.text = view.text
        info.isEditable = (view as? EditorEditText)?.isEditable != false
        info.isMultiLine = true
        info.isClickable = view.isClickable
        if (view.isEnabled) info.addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_CLICK)
        info.setTextSelection(view.selectionStart, view.selectionEnd)
        info.movementGranularities = AccessibilityNodeInfo.MOVEMENT_GRANULARITY_CHARACTER or AccessibilityNodeInfo.MOVEMENT_GRANULARITY_WORD or AccessibilityNodeInfo.MOVEMENT_GRANULARITY_LINE or AccessibilityNodeInfo.MOVEMENT_GRANULARITY_PARAGRAPH
        info.addAction(AccessibilityNodeInfo.ACTION_SET_SELECTION)
        info.addAction(AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY)
        info.addAction(AccessibilityNodeInfo.ACTION_PREVIOUS_AT_MOVEMENT_GRANULARITY)
        info.addAction(AccessibilityNodeInfo.ACTION_COPY)
        if (info.isEditable) {
            info.addAction(AccessibilityNodeInfo.ACTION_SET_TEXT)
            info.addAction(AccessibilityNodeInfo.ACTION_CUT)
            info.addAction(AccessibilityNodeInfo.ACTION_PASTE)
        }
    }

    fun performAccessibilityAction(action: Int, arguments: Bundle?): Boolean {
        when (action) {
            AccessibilityNodeInfo.ACTION_CLICK -> {
                view.requestFocus()
                showKeyboard()
                view.performClick()
                return true
            }
            AccessibilityNodeInfo.ACTION_SET_SELECTION -> {
                val start = arguments?.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, -1) ?: -1
                val end = arguments?.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, -1) ?: -1
                if (start !in 0..view.text.length || end !in 0..view.text.length) return false
                view.setSelection(start, end)
                return true
            }
            AccessibilityNodeInfo.ACTION_SET_TEXT -> {
                val replacement = arguments?.getCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE) ?: return false
                view.editableText.replace(0, view.text.length, replacement)
                view.setSelection(view.text.length)
                return true
            }
            AccessibilityNodeInfo.ACTION_COPY -> return view.onTextContextMenuItem(android.R.id.copy)
            AccessibilityNodeInfo.ACTION_CUT -> return view.onTextContextMenuItem(android.R.id.cut)
            AccessibilityNodeInfo.ACTION_PASTE -> return view.onTextContextMenuItem(android.R.id.paste)
            AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY,
            AccessibilityNodeInfo.ACTION_PREVIOUS_AT_MOVEMENT_GRANULARITY -> {
                val forward = action == AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY
                val text = view.text.toString()
                val offset = view.selectionEnd.coerceAtLeast(0)
                val granularity = arguments?.getInt(AccessibilityNodeInfo.ACTION_ARGUMENT_MOVEMENT_GRANULARITY_INT) ?: return false
                val next = when (granularity) {
                    AccessibilityNodeInfo.MOVEMENT_GRANULARITY_CHARACTER,
                    AccessibilityNodeInfo.MOVEMENT_GRANULARITY_WORD -> {
                        val iterator = (if (granularity == AccessibilityNodeInfo.MOVEMENT_GRANULARITY_CHARACTER) BreakIterator.getCharacterInstance() else BreakIterator.getWordInstance()).apply { setText(text) }
                        var next = if (forward) iterator.following(offset) else iterator.preceding(offset)
                        if (granularity == AccessibilityNodeInfo.MOVEMENT_GRANULARITY_WORD) {
                            while (next != BreakIterator.DONE && text.substring(minOf(offset, next), maxOf(offset, next)).isBlank()) {
                                next = if (forward) iterator.following(next) else iterator.preceding(next)
                            }
                        }
                        next
                    }
                    AccessibilityNodeInfo.MOVEMENT_GRANULARITY_LINE -> {
                        val line = view.layout.getLineForOffset(offset) + if (forward) 1 else -1
                        if (line in 0 until view.layout.lineCount) view.layout.getLineStart(line) else BreakIterator.DONE
                    }
                    AccessibilityNodeInfo.MOVEMENT_GRANULARITY_PARAGRAPH -> if (forward) text.indexOf('\n', offset).let { if (it < 0) text.length else it + 1 } else text.lastIndexOf('\n', (offset - 2).coerceAtLeast(0)) + 1
                    else -> return false
                }
                if (next == BreakIterator.DONE || next == offset) return false
                if (arguments.getBoolean(AccessibilityNodeInfo.ACTION_ARGUMENT_EXTEND_SELECTION_BOOLEAN)) view.extendSelection(next) else view.setSelection(next)
                view.bringPointIntoView(next)
                val event = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_VIEW_TEXT_TRAVERSED_AT_MOVEMENT_GRANULARITY)
                event.action = action
                event.movementGranularity = granularity
                event.fromIndex = minOf(offset, next)
                event.toIndex = maxOf(offset, next)
                view.sendSurfaceAccessibilityEvent(event)
                return true
            }
        }
        return false
    }
}
