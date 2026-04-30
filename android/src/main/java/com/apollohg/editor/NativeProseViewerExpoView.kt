package com.apollohg.editor

import android.content.Intent
import android.content.Context
import android.graphics.Color
import android.net.Uri
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import org.json.JSONArray

class NativeProseViewerExpoView(
    context: Context,
    appContext: AppContext
) : ExpoView(context, appContext) {

    private val proseView = EditorEditText(context)
    private val onContentHeightChange by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    private val onPressLink by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    private val onPressMention by EventDispatcher<Map<String, Any>>()

    private var lastRenderJson: String? = null
    private var lastThemeJson: String? = null
    private var lastEmittedContentHeight = 0
    private var collapsesWhenEmpty = true
    private var isCollapsedEmptyContent = false
    private var enableLinkTaps = true
    private var interceptLinkTaps = false

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

            proseView.mentionHitAt(event.x, event.y)?.let { mention ->
                onPressMention(mapOf("docPos" to mention.docPos, "label" to mention.label))
                return@setOnTouchListener true
            }

            if (!enableLinkTaps) {
                return@setOnTouchListener false
            }

            val link = proseView.linkHitAt(event.x, event.y) ?: return@setOnTouchListener false
            if (interceptLinkTaps) {
                onPressLink(
                    mapOf(
                        "href" to link.href,
                        "text" to link.text
                    )
                )
                return@setOnTouchListener true
            }

            return@setOnTouchListener openLink(link.href)
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
        applyRenderJson()
        post {
            requestLayout()
            emitContentHeightIfNeeded(force = true)
        }
    }

    fun setThemeJson(themeJson: String?) {
        if (lastThemeJson == themeJson) return
        lastThemeJson = themeJson
        proseView.applyTheme(EditorTheme.fromJson(themeJson))
        applyRenderJson()
        post {
            requestLayout()
            emitContentHeightIfNeeded(force = true)
        }
    }

    fun setCollapsesWhenEmpty(collapsesWhenEmpty: Boolean?) {
        val nextValue = collapsesWhenEmpty ?: true
        if (this.collapsesWhenEmpty == nextValue) return
        this.collapsesWhenEmpty = nextValue
        updateCollapsedEmptyState()
        requestLayout()
        emitContentHeightIfNeeded(force = true)
    }

    fun setEnableLinkTaps(enableLinkTaps: Boolean?) {
        this.enableLinkTaps = enableLinkTaps ?: true
    }

    fun setInterceptLinkTaps(interceptLinkTaps: Boolean?) {
        this.interceptLinkTaps = interceptLinkTaps ?: false
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        if (isCollapsedEmptyContent) {
            setMeasuredDimension(resolveSize(0, widthMeasureSpec), 0)
            return
        }

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
        if (isCollapsedEmptyContent) {
            proseView.layout(paddingLeft, paddingTop, right - left - paddingRight, paddingTop)
            emitContentHeightIfNeeded()
            return
        }

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

    private fun applyRenderJson() {
        updateCollapsedEmptyState()
        proseView.applyRenderJSON(lastRenderJson ?: "[]")
        proseView.visibility = if (isCollapsedEmptyContent) View.GONE else View.VISIBLE
    }

    private fun updateCollapsedEmptyState() {
        isCollapsedEmptyContent = collapsesWhenEmpty &&
            renderJsonContainsOnlyEmptyParagraphs(lastRenderJson ?: "[]")
        proseView.visibility = if (isCollapsedEmptyContent) View.GONE else View.VISIBLE
    }

    private fun emitContentHeightIfNeeded(force: Boolean = false) {
        val contentHeight = if (isCollapsedEmptyContent) {
            0
        } else {
            (measureContentHeightPx() + paddingTop + paddingBottom).coerceAtLeast(0)
        }
        if (contentHeight <= 0 && !isCollapsedEmptyContent) {
            return
        }
        if (!force && contentHeight == lastEmittedContentHeight) {
            return
        }
        lastEmittedContentHeight = contentHeight
        onContentHeightChange(mapOf("contentHeight" to contentHeight))
    }

    private fun measureContentHeightPx(): Int {
        if (isCollapsedEmptyContent) {
            return 0
        }

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

    private fun openLink(href: String): Boolean {
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse(href)).apply {
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        return runCatching {
            context.startActivity(intent)
            true
        }.getOrDefault(false)
    }

    companion object {
        private const val EMPTY_TEXT_BLOCK_PLACEHOLDER = '\u200B'

        internal fun renderJsonContainsOnlyEmptyParagraphs(renderJson: String): Boolean {
            val elements = try {
                JSONArray(renderJson)
            } catch (_: Exception) {
                return false
            }

            if (elements.length() == 0) {
                return true
            }

            var hasParagraph = false
            var paragraphIsOpen = false

            for (index in 0 until elements.length()) {
                val element = elements.optJSONObject(index) ?: return false
                when (element.optString("type", "")) {
                    "blockStart" -> {
                        if (
                            paragraphIsOpen ||
                            element.optString("nodeType", "") != "paragraph" ||
                            element.optInt("depth", 0) != 0
                        ) {
                            return false
                        }
                        paragraphIsOpen = true
                        hasParagraph = true
                    }

                    "textRun" -> {
                        val text = element.optString("text", "")
                        if (
                            !paragraphIsOpen ||
                            !text.all { it == EMPTY_TEXT_BLOCK_PLACEHOLDER }
                        ) {
                            return false
                        }
                    }

                    "blockEnd" -> {
                        if (!paragraphIsOpen) {
                            return false
                        }
                        paragraphIsOpen = false
                    }

                    else -> return false
                }
            }

            return hasParagraph && !paragraphIsOpen
        }
    }
}
