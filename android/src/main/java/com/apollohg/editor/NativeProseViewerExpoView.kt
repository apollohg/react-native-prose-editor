package com.apollohg.editor

import android.content.Intent
import android.content.Context
import android.graphics.Color
import android.graphics.Rect
import android.net.Uri
import android.os.Bundle
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeProvider
import android.view.accessibility.AccessibilityEvent
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import kotlin.math.abs
import org.json.JSONArray

private sealed interface TapTarget {
    val annotation: android.text.Annotation
    val start: Int
    val end: Int

    data class Mention(
        val docPos: Int,
        val label: String,
        override val annotation: android.text.Annotation,
        override val start: Int,
        override val end: Int
    ) : TapTarget

    data class Link(
        val href: String,
        val text: String,
        override val annotation: android.text.Annotation,
        override val start: Int,
        override val end: Int
    ) : TapTarget
}

private fun TapTarget.matches(other: TapTarget?): Boolean =
    other != null &&
        annotation === other.annotation &&
        start == other.start &&
        end == other.end &&
        this::class == other::class

private data class PendingTapGesture(
    val target: TapTarget,
    val pointerId: Int,
    val downX: Float,
    val downY: Float
)

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
    private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop.toFloat()
    private var pendingTapGesture: PendingTapGesture? = null
    private var accessibilityFocusedVirtualId = View.NO_ID
    internal var suppressContentHeightEventsForTesting = false
    internal var onLinkTapForTesting: (() -> Unit)? = null
    internal var onMentionTapForTesting: (() -> Unit)? = null

    init {
        proseView.setBaseStyle(
            proseView.textSize,
            proseView.currentTextColor,
            Color.TRANSPARENT
        )
        proseView.isEditable = false
        proseView.inputType = android.text.InputType.TYPE_CLASS_TEXT or
            android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE or
            android.text.InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        proseView.setImageResizingEnabled(false)
        proseView.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        proseView.isFocusable = false
        proseView.isFocusableInTouchMode = false
        proseView.isCursorVisible = false
        proseView.isLongClickable = false
        proseView.setTextIsSelectable(false)
        proseView.showSoftInputOnFocus = false
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
        proseView.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        proseView.setOnTouchListener { _, event ->
            handleProseTouch(event)
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
        clearVirtualAccessibilityFocus()
        lastRenderJson = renderJson
        applyRenderJson()
        requestLayout()
        notifyAccessibilitySubtreeChanged()
    }

    fun setThemeJson(themeJson: String?) {
        if (lastThemeJson == themeJson) return
        lastThemeJson = themeJson
        proseView.applyTheme(EditorTheme.fromJson(themeJson))
        applyRenderJson()
        requestLayout()
    }

    fun setImageLoadingPolicyJson(policyJson: String?) {
        proseView.setImageLoadingPolicyJson(policyJson)
        applyRenderJson()
        requestLayout()
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
        val nextValue = enableLinkTaps ?: true
        if (this.enableLinkTaps == nextValue) return
        clearVirtualAccessibilityFocus()
        this.enableLinkTaps = nextValue
        notifyAccessibilitySubtreeChanged()
    }

    fun setInterceptLinkTaps(interceptLinkTaps: Boolean?) {
        this.interceptLinkTaps = interceptLinkTaps ?: false
    }

    private fun handleProseTouch(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                pendingTapGesture = if (event.pointerCount == 1) {
                    tapTargetAt(event.x, event.y)?.let { target ->
                        PendingTapGesture(
                            target = target,
                            pointerId = event.getPointerId(event.actionIndex),
                            downX = event.x,
                            downY = event.y
                        )
                    }
                } else {
                    null
                }
                return false
            }
            MotionEvent.ACTION_MOVE -> {
                val gesture = pendingTapGesture
                if (
                    gesture != null &&
                    (event.pointerCount != 1 ||
                        event.findPointerIndex(gesture.pointerId) < 0 ||
                        movedBeyondTouchSlop(event, gesture))
                ) {
                    pendingTapGesture = null
                }
                return false
            }
            MotionEvent.ACTION_POINTER_DOWN,
            MotionEvent.ACTION_POINTER_UP,
            MotionEvent.ACTION_CANCEL -> {
                pendingTapGesture = null
                return false
            }
            MotionEvent.ACTION_UP -> {
                val gesture = pendingTapGesture
                pendingTapGesture = null
                if (
                    event.pointerCount != 1 ||
                    gesture == null ||
                    event.getPointerId(event.actionIndex) != gesture.pointerId ||
                    movedBeyondTouchSlop(event, gesture) ||
                    !gesture.target.matches(tapTargetAt(event.x, event.y))
                ) {
                    return false
                }
                return activateTapTarget(gesture.target)
            }
            else -> return false
        }
    }

    private fun movedBeyondTouchSlop(event: MotionEvent, gesture: PendingTapGesture): Boolean {
        val deltaX = event.x - gesture.downX
        val deltaY = event.y - gesture.downY
        return deltaX * deltaX + deltaY * deltaY > touchSlop * touchSlop
    }

    private fun tapTargetAt(x: Float, y: Float): TapTarget? {
        val hit = proseView.interactiveAnnotationHitAt(x, y) ?: return null
        return when (val target = hit.target) {
            is EditorEditText.AccessibleAnnotationTarget.Mention -> TapTarget.Mention(
                target.docPos,
                target.label,
                hit.annotation,
                hit.start,
                hit.end
            )
            is EditorEditText.AccessibleAnnotationTarget.Link -> {
                if (!enableLinkTaps) return null
                TapTarget.Link(
                    target.href,
                    target.text,
                    hit.annotation,
                    hit.start,
                    hit.end
                )
            }
        }
    }

    private fun activateTapTarget(target: TapTarget): Boolean {
        return when (target) {
            is TapTarget.Mention -> {
                onMentionTapForTesting?.invoke() ?: onPressMention(
                    mapOf("docPos" to target.docPos, "label" to target.label)
                )
                true
            }
            is TapTarget.Link -> {
                onLinkTapForTesting?.let {
                    it()
                    return true
                }
                if (interceptLinkTaps) {
                    onPressLink(mapOf("href" to target.href, "text" to target.text))
                    true
                } else {
                    openLink(target.href)
                }
            }
        }
    }

    override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
        super.onInitializeAccessibilityNodeInfo(info)
        info.className = android.widget.TextView::class.java.name
        info.text = proseView.text?.toString()?.replace(EMPTY_TEXT_BLOCK_PLACEHOLDER.toString(), "")
        accessibleAnnotations().indices.forEach { index ->
            info.addChild(this, index + FIRST_VIRTUAL_ANNOTATION_ID)
        }
    }

    override fun getAccessibilityNodeProvider(): AccessibilityNodeProvider = annotationNodeProvider

    private val annotationNodeProvider = object : AccessibilityNodeProvider() {
        override fun createAccessibilityNodeInfo(virtualViewId: Int): AccessibilityNodeInfo? {
            if (virtualViewId == View.NO_ID) {
                return AccessibilityNodeInfo.obtain(this@NativeProseViewerExpoView).also {
                    onInitializeAccessibilityNodeInfo(it)
                }
            }
            val annotation = accessibleAnnotations()
                .getOrNull(virtualViewId - FIRST_VIRTUAL_ANNOTATION_ID) ?: return null
            val bounds = Rect(annotation.bounds).apply {
                offset(proseView.left, proseView.top)
            }
            val screenBounds = Rect(bounds)
            val screenLocation = IntArray(2)
            getLocationOnScreen(screenLocation)
            screenBounds.offset(screenLocation[0], screenLocation[1])
            return AccessibilityNodeInfo.obtain().apply {
                packageName = context.packageName
                className = android.widget.Button::class.java.name
                setSource(this@NativeProseViewerExpoView, virtualViewId)
                setParent(this@NativeProseViewerExpoView)
                text = annotation.label
                contentDescription = annotation.label
                isEnabled = isAttachedToWindow || !isInEditMode
                isClickable = true
                isFocusable = true
                isAccessibilityFocused = virtualViewId == accessibilityFocusedVirtualId
                setBoundsInParent(bounds)
                setBoundsInScreen(screenBounds)
                addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_CLICK)
                addAction(
                    if (isAccessibilityFocused) {
                        AccessibilityNodeInfo.AccessibilityAction.ACTION_CLEAR_ACCESSIBILITY_FOCUS
                    } else {
                        AccessibilityNodeInfo.AccessibilityAction.ACTION_ACCESSIBILITY_FOCUS
                    }
                )
                AccessibilityNodeInfoCompat.wrap(this).roleDescription = annotation.role
            }
        }

        override fun performAction(
            virtualViewId: Int,
            action: Int,
            arguments: Bundle?
        ): Boolean {
            val annotation = accessibleAnnotations()
                .getOrNull(virtualViewId - FIRST_VIRTUAL_ANNOTATION_ID) ?: return false
            return when (action) {
                AccessibilityNodeInfo.ACTION_CLICK ->
                    activateTapTarget(annotation.toTapTarget())
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS ->
                    requestVirtualAccessibilityFocus(virtualViewId)
                AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS ->
                    clearVirtualAccessibilityFocus(virtualViewId)
                else -> false
            }
        }
    }

    private fun requestVirtualAccessibilityFocus(virtualViewId: Int): Boolean {
        if (accessibleAnnotations().getOrNull(virtualViewId - FIRST_VIRTUAL_ANNOTATION_ID) == null) {
            return false
        }
        if (accessibilityFocusedVirtualId == virtualViewId) return false
        if (accessibilityFocusedVirtualId != View.NO_ID) {
            val previousId = accessibilityFocusedVirtualId
            accessibilityFocusedVirtualId = View.NO_ID
            sendVirtualAccessibilityEvent(
                previousId,
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED
            )
        }
        accessibilityFocusedVirtualId = virtualViewId
        invalidate()
        sendVirtualAccessibilityEvent(
            virtualViewId,
            AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED
        )
        return true
    }

    private fun clearVirtualAccessibilityFocus(
        virtualViewId: Int = accessibilityFocusedVirtualId
    ): Boolean {
        if (
            virtualViewId == View.NO_ID ||
            virtualViewId != accessibilityFocusedVirtualId
        ) return false
        accessibilityFocusedVirtualId = View.NO_ID
        invalidate()
        sendVirtualAccessibilityEvent(
            virtualViewId,
            AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED
        )
        return true
    }

    private fun sendVirtualAccessibilityEvent(virtualViewId: Int, eventType: Int) {
        val event = AccessibilityEvent.obtain(eventType).apply {
            packageName = context.packageName
            className = android.widget.Button::class.java.name
            setSource(this@NativeProseViewerExpoView, virtualViewId)
        }
        parent?.requestSendAccessibilityEvent(this, event)
    }

    private fun notifyAccessibilitySubtreeChanged() {
        val event = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED).apply {
            packageName = context.packageName
            className = android.widget.TextView::class.java.name
            contentChangeTypes = AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE
            setSource(this@NativeProseViewerExpoView)
        }
        parent?.requestSendAccessibilityEvent(this, event)
    }

    private fun accessibleAnnotations(): List<EditorEditText.AccessibleAnnotation> =
        proseView.accessibleAnnotations().filter { annotation ->
            annotation.target !is EditorEditText.AccessibleAnnotationTarget.Link || enableLinkTaps
        }

    private fun EditorEditText.AccessibleAnnotation.toTapTarget(): TapTarget = when (val value = target) {
        is EditorEditText.AccessibleAnnotationTarget.Link ->
            TapTarget.Link(value.href, value.text, annotation, start, end)
        is EditorEditText.AccessibleAnnotationTarget.Mention ->
            TapTarget.Mention(value.docPos, value.label, annotation, start, end)
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        if (isCollapsedEmptyContent) {
            setMeasuredDimension(resolveSize(0, widthMeasureSpec), 0)
            emitContentHeightIfNeeded()
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

        val resolvedContentHeight = proseView.resolveAutoGrowHeight()
        val desiredWidth = proseView.measuredWidth + paddingLeft + paddingRight
        val desiredHeight = resolvedContentHeight + paddingTop + paddingBottom
        val measuredHeight = when (View.MeasureSpec.getMode(heightMeasureSpec)) {
            View.MeasureSpec.AT_MOST -> desiredHeight.coerceAtMost(
                View.MeasureSpec.getSize(heightMeasureSpec)
            )
            else -> desiredHeight
        }
        setMeasuredDimension(
            resolveSize(desiredWidth, widthMeasureSpec),
            measuredHeight
        )
        emitContentHeightIfNeeded(measuredContentHeight = desiredHeight)
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

    private fun emitContentHeightIfNeeded(
        force: Boolean = false,
        measuredContentHeight: Int? = null
    ) {
        val contentHeight = if (isCollapsedEmptyContent) {
            0
        } else {
            (
                measuredContentHeight ?: (measureContentHeightPx() + paddingTop + paddingBottom)
            ).coerceAtLeast(0)
        }
        if (contentHeight <= 0 && !isCollapsedEmptyContent) {
            return
        }
        if (!force && contentHeight == lastEmittedContentHeight) {
            return
        }
        lastEmittedContentHeight = contentHeight
        if (suppressContentHeightEventsForTesting) {
            return
        }
        onContentHeightChange(mapOf("contentHeight" to contentHeight))
    }

    private fun measureContentHeightPx(): Int {
        if (isCollapsedEmptyContent) {
            return 0
        }

        val availableWidthPx = resolveAvailableWidthPx()
        if (
            proseView.measuredWidth <= 0 ||
            abs(proseView.measuredWidth - availableWidthPx) > 1
        ) {
            val childWidthSpec = View.MeasureSpec.makeMeasureSpec(
                availableWidthPx,
                View.MeasureSpec.EXACTLY
            )
            val childHeightSpec = View.MeasureSpec.makeMeasureSpec(
                0,
                View.MeasureSpec.UNSPECIFIED
            )
            proseView.measure(childWidthSpec, childHeightSpec)
        }
        return proseView.resolveAutoGrowHeight()
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
        private const val FIRST_VIRTUAL_ANNOTATION_ID = 1

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
