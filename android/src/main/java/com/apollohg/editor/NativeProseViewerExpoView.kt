package com.apollohg.editor

import android.content.Context
import android.view.View
import android.view.ViewGroup
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView

/** Expo adapter over the public Android prose viewer. */
class NativeProseViewerExpoView(
    context: Context,
    appContext: AppContext
) : ExpoView(context, appContext), ProseViewerInteractionListener {
    private val viewer = ProseViewerView(context)
    private val onContentHeightChange by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    private val onPressLink by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    private val onPressMention by EventDispatcher<Map<String, Any>>()

    private var lastRenderJson: String? = null
    private var lastThemeJson: String? = null
    private var lastEmittedContentHeight = 0
    internal var suppressContentHeightEventsForTesting = false
    internal val viewerForTesting: ProseViewerView
        get() = viewer

    init {
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        viewer.interactionListener = this
        viewer.opensLinksAutomatically = true
        viewer.setCollapsesWhenEmpty(true)
        viewer.onContentHeightChange = { heightPx ->
            val heightWithHostPadding = if (viewer.isContentCollapsedForHost) {
                0
            } else {
                heightPx + paddingTop + paddingBottom
            }
            emitContentHeightIfNeeded(heightWithHostPadding, force = true)
        }
        addView(
            viewer,
            LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
        )
    }

    fun setRenderJson(renderJson: String?) {
        if (lastRenderJson == renderJson) return
        lastRenderJson = renderJson
        applyToViewer()
    }

    fun setThemeJson(themeJson: String?) {
        if (lastThemeJson == themeJson) return
        lastThemeJson = themeJson
        applyToViewer()
    }

    fun setImageLoadingPolicyJson(policyJson: String?) {
        viewer.setImageLoadingPolicyJson(policyJson)
    }

    fun setCollapsesWhenEmpty(collapsesWhenEmpty: Boolean?) {
        viewer.setCollapsesWhenEmpty(collapsesWhenEmpty ?: true)
    }

    fun setEnableLinkTaps(enableLinkTaps: Boolean?) {
        viewer.linkTapsEnabled = enableLinkTaps ?: true
    }

    fun setInterceptLinkTaps(interceptLinkTaps: Boolean?) {
        viewer.opensLinksAutomatically = !(interceptLinkTaps ?: false)
    }

    private fun applyToViewer() {
        viewer.apply(
            renderJson = lastRenderJson ?: "[]",
            themeJson = lastThemeJson ?: "{}"
        )
        requestLayout()
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val childWidthSpec = getChildMeasureSpec(
            widthMeasureSpec,
            paddingLeft + paddingRight,
            viewer.layoutParams.width
        )
        viewer.measure(
            childWidthSpec,
            View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        )
        val desiredWidth = viewer.measuredWidth + paddingLeft + paddingRight
        val desiredHeight = if (viewer.isContentCollapsedForHost) {
            0
        } else {
            viewer.measuredHeight + paddingTop + paddingBottom
        }
        setMeasuredDimension(
            resolveSize(desiredWidth, widthMeasureSpec),
            desiredHeight
        )
        emitContentHeightIfNeeded(desiredHeight)
    }

    override fun onLayout(
        changed: Boolean,
        left: Int,
        top: Int,
        right: Int,
        bottom: Int
    ) {
        val childTop = paddingTop
        viewer.layout(
            paddingLeft,
            childTop,
            right - left - paddingRight,
            childTop + viewer.measuredHeight
        )
        emitContentHeightIfNeeded(
            if (viewer.isContentCollapsedForHost) 0 else measuredHeight
        )
    }

    override fun onLinkTap(view: ProseViewerView, href: String, text: String) {
        onPressLink(mapOf("href" to href, "text" to text))
    }

    override fun onMentionTap(view: ProseViewerView, docPos: Long, label: String) {
        onPressMention(mapOf("docPos" to docPos, "label" to label))
    }

    private fun emitContentHeightIfNeeded(contentHeight: Int, force: Boolean = false) {
        if (contentHeight <= 0 && !viewer.isContentCollapsedForHost) return
        if (!force && contentHeight == lastEmittedContentHeight) return
        lastEmittedContentHeight = contentHeight
        if (!suppressContentHeightEventsForTesting) {
            onContentHeightChange(mapOf("contentHeight" to contentHeight))
        }
    }
}
