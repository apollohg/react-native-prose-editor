package com.apollohg.editor

import android.content.Context
import android.graphics.Color
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView

class NativeProseViewerExpoView(
    context: Context,
    appContext: AppContext
) : ExpoView(context, appContext) {

    private val proseView = EditorEditText(context)
    private val onContentHeightChange by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    private val onPressMention by EventDispatcher<Map<String, Any>>()

    private var lastRenderJson: String? = null
    private var lastThemeJson: String? = null
    private var lastEmittedContentHeight = 0

    init {
        proseView.setBaseStyle(
            proseView.textSize,
            proseView.currentTextColor,
            Color.TRANSPARENT
        )
        proseView.isEditable = false
        proseView.setImageResizingEnabled(false)
        proseView.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        proseView.isFocusable = false
        proseView.isFocusableInTouchMode = false
        proseView.isCursorVisible = false
        proseView.isLongClickable = false
        proseView.setTextIsSelectable(false)
        proseView.showSoftInputOnFocus = false
        proseView.setOnTouchListener { _, event ->
            if (event.actionMasked != MotionEvent.ACTION_UP) {
                return@setOnTouchListener false
            }

            val mention = proseView.mentionHitAt(event.x, event.y) ?: return@setOnTouchListener false
            onPressMention(
                mapOf(
                    "docPos" to mention.docPos,
                    "label" to mention.label
                )
            )
            true
        }

        addView(
            proseView,
            LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
        )
    }

    fun setRenderJson(renderJson: String?) {
        if (lastRenderJson == renderJson) return
        lastRenderJson = renderJson
        proseView.applyRenderJSON(renderJson ?: "[]")
        post {
            requestLayout()
            emitContentHeightIfNeeded(force = true)
        }
    }

    fun setThemeJson(themeJson: String?) {
        if (lastThemeJson == themeJson) return
        lastThemeJson = themeJson
        proseView.applyTheme(EditorTheme.fromJson(themeJson))
        proseView.applyRenderJSON(lastRenderJson ?: "[]")
        post {
            requestLayout()
            emitContentHeightIfNeeded(force = true)
        }
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val childWidthSpec = getChildMeasureSpec(
            widthMeasureSpec,
            paddingLeft + paddingRight,
            proseView.layoutParams.width
        )
        val childHeightSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            0,
            android.view.View.MeasureSpec.UNSPECIFIED
        )
        proseView.measure(childWidthSpec, childHeightSpec)

        val desiredWidth = proseView.measuredWidth + paddingLeft + paddingRight
        val desiredHeight = proseView.measuredHeight + paddingTop + paddingBottom
        setMeasuredDimension(
            resolveSize(desiredWidth, widthMeasureSpec),
            resolveSize(desiredHeight, heightMeasureSpec)
        )
    }

    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
        val childLeft = paddingLeft
        val childTop = paddingTop
        proseView.layout(
            childLeft,
            childTop,
            right - left - paddingRight,
            childTop + proseView.measuredHeight
        )
        emitContentHeightIfNeeded()
    }

    private fun emitContentHeightIfNeeded(force: Boolean = false) {
        val contentHeight = (measureContentHeightPx() + paddingTop + paddingBottom)
            .coerceAtLeast(0)
        if (contentHeight <= 0) {
            return
        }
        if (!force && contentHeight == lastEmittedContentHeight) {
            return
        }
        lastEmittedContentHeight = contentHeight
        onContentHeightChange(mapOf("contentHeight" to contentHeight))
    }

    private fun measureContentHeightPx(): Int {
        val currentMeasuredHeight = proseView.measuredHeight
        if (currentMeasuredHeight > 0 && proseView.layout != null) {
            return currentMeasuredHeight
        }

        val availableWidthPx = resolveAvailableWidthPx()
        val childWidthSpec = View.MeasureSpec.makeMeasureSpec(availableWidthPx, View.MeasureSpec.EXACTLY)
        val childHeightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        proseView.measure(childWidthSpec, childHeightSpec)
        return proseView.measuredHeight
    }

    private fun resolveAvailableWidthPx(): Int {
        val localWidth = width - paddingLeft - paddingRight
        if (localWidth > 0) {
            return localWidth
        }

        val parentWidth = ((parent as? View)?.width ?: 0) - paddingLeft - paddingRight
        if (parentWidth > 0) {
            return parentWidth
        }

        return (resources.displayMetrics.widthPixels - paddingLeft - paddingRight).coerceAtLeast(1)
    }
}
