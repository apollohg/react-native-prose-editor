package com.apollohg.editor

import android.content.Context
import android.graphics.Color
import android.graphics.RectF
import android.graphics.drawable.GradientDrawable
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import kotlin.math.roundToInt

/** Container view that owns the native editor text field. */
class RichTextEditorView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0
) : LinearLayout(context, attrs, defStyleAttr) {
    val editorViewport: FrameLayout
    val editorContentFrame: FrameLayout

    private class EditorScrollView(context: Context) : ScrollView(context) {
        private fun updateParentIntercept(action: Int) {
            val canScroll = canScrollVertically(-1) || canScrollVertically(1)
            if (!canScroll) return
            when (action) {
                MotionEvent.ACTION_DOWN,
                MotionEvent.ACTION_MOVE -> parent?.requestDisallowInterceptTouchEvent(true)
                MotionEvent.ACTION_UP,
                MotionEvent.ACTION_CANCEL -> parent?.requestDisallowInterceptTouchEvent(false)
            }
        }

        override fun onInterceptTouchEvent(ev: MotionEvent): Boolean {
            updateParentIntercept(ev.actionMasked)
            return super.onInterceptTouchEvent(ev)
        }

        override fun onTouchEvent(ev: MotionEvent): Boolean {
            updateParentIntercept(ev.actionMasked)
            return super.onTouchEvent(ev)
        }
    }

    private inner class EditorContentFrame(context: Context) : FrameLayout(context) {
        override fun measureChildWithMargins(
            child: View,
            parentWidthMeasureSpec: Int,
            widthUsed: Int,
            parentHeightMeasureSpec: Int,
            heightUsed: Int,
        ) {
            if (child === editorEditText) {
                super.measureChildWithMargins(
                    child,
                    parentWidthMeasureSpec,
                    widthUsed,
                    parentHeightMeasureSpec,
                    heightUsed,
                )
                return
            }
            val width = child.measuredWidth.takeIf { it > 0 } ?: child.width.coerceAtLeast(0)
            val height = child.measuredHeight.takeIf { it > 0 } ?: child.height.coerceAtLeast(0)
            child.measure(
                MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY),
                MeasureSpec.makeMeasureSpec(height, MeasureSpec.EXACTLY),
            )
        }
    }

    val editorEditText: EditorEditText
    val editorScrollView: ScrollView
    private val remoteSelectionOverlayView: RemoteSelectionOverlayView
    private val imageResizeOverlayView: ImageResizeOverlayView

    private var heightBehavior: EditorHeightBehavior = EditorHeightBehavior.FIXED
    private var imageResizingEnabled = true
    private var theme: EditorTheme? = null
    private var baseBackgroundColor: Int = Color.WHITE
    private var viewportBottomInsetPx: Int = 0
    var onAtomContentWidthChange: ((Float) -> Unit)? = null
    private var atomRenderConfiguration: AtomRenderConfiguration? = null
    private val atomHostViews = linkedMapOf<String, View>()
    private val atomLayoutListeners = mutableMapOf<View, View.OnLayoutChangeListener>()
    private val measuredAtomHeightsPx = mutableMapOf<String, Int>()
    private var lastAtomContentWidthPx = -1
    internal var appliedCornerRadiusPx: Float = 0f
    internal var appliedBackgroundColorForTesting: Int = Color.WHITE

    private var currentEditorId: Long = 0
    private var deferEditorUnbindOnDetach = false
    internal var onBeforeDetachedFromWindow: (() -> Unit)? = null

    /** Binds or unbinds the Rust editor instance. */
    var editorId: Long
        get() = currentEditorId
        set(value) {
            setEditorId(value, bindEditor = true)
        }

    init {
        orientation = VERTICAL

        editorEditText = EditorEditText(context)
        editorContentFrame = EditorContentFrame(context).apply {
            setOnTouchListener { _, event ->
                if (event.actionMasked == MotionEvent.ACTION_DOWN) {
                    editorEditText.isFocusableInTouchMode = true
                    editorEditText.requestFocus()
                }
                true
            }
        }
        editorScrollView = EditorScrollView(context).apply {
            clipToPadding = false
            // Short content must still fill the viewport, or taps below the
            // last line land on the scroll container and never focus.
            isFillViewport = true
        }
        editorViewport = FrameLayout(context)
        remoteSelectionOverlayView = RemoteSelectionOverlayView(context)
        imageResizeOverlayView = ImageResizeOverlayView(context)
        editorContentFrame.addView(editorEditText, createEditorLayoutParams())
        editorScrollView.addView(
            editorContentFrame,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
        )
        editorViewport.addView(
            editorScrollView,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT
            )
        )
        editorViewport.addView(
            remoteSelectionOverlayView,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT
            )
        )
        editorViewport.addView(
            imageResizeOverlayView,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT
            )
        )
        remoteSelectionOverlayView.bind(this)
        imageResizeOverlayView.bind(this)
        editorEditText.onBeforeRenderRefresh = imageResizeOverlayView::cancelActiveResize
        editorScrollView.setOnScrollChangeListener { _, _, _, _, _ ->
            refreshOverlays()
        }
        editorEditText.onSelectionOrContentMayChange = { refreshOverlays() }

        addView(editorViewport, createContainerLayoutParams())
        updateScrollContainerAppearance()
        updateScrollContainerInsets()
    }

    fun configure(
        textSizePx: Float = 16f * resources.displayMetrics.density,
        textColor: Int = Color.BLACK,
        backgroundColor: Int = Color.WHITE
    ) {
        baseBackgroundColor = backgroundColor
        editorEditText.setBaseStyle(textSizePx, textColor, backgroundColor)
        updateScrollContainerAppearance()
        refreshOverlays()
    }

    fun applyTheme(theme: EditorTheme?) {
        this.theme = theme
        val previousScrollY = editorScrollView.scrollY
        editorEditText.applyTheme(theme)
        updateScrollContainerAppearance()
        updateScrollContainerInsets()
        if (heightBehavior == EditorHeightBehavior.FIXED) {
            editorScrollView.post {
                val childHeight = editorScrollView.getChildAt(0)?.height ?: 0
                val maxScrollY = maxOf(
                    0,
                    childHeight + editorScrollView.paddingTop + editorScrollView.paddingBottom - editorScrollView.height
                )
                editorScrollView.scrollTo(0, previousScrollY.coerceIn(0, maxScrollY))
                refreshOverlays()
            }
        }
        refreshOverlays()
    }

    fun applyAtomRenderConfiguration(configuration: AtomRenderConfiguration?): Boolean {
        val previous = atomRenderConfiguration
        atomRenderConfiguration = configuration
        val applied = editorEditText.applyAtomRenderConfiguration(resolvedAtomRenderConfiguration())
        if (!applied) atomRenderConfiguration = previous
        if (applied) refreshOverlays()
        return applied
    }

    internal fun mountAtomChild(child: View, atomKey: String) {
        atomHostViews.entries.firstOrNull { it.value === child }?.let { existing ->
            if (existing.key != atomKey) atomHostViews.remove(existing.key)
        }
        atomHostViews[atomKey]?.takeIf { it !== child }?.let(::detachAtomView)
        atomHostViews[atomKey] = child
        (child.parent as? ViewGroup)?.removeView(child)
        editorContentFrame.addView(child)
        val listener = View.OnLayoutChangeListener { _, _, top, _, bottom, _, oldTop, _, oldBottom ->
            val height = bottom - top
            val oldHeight = oldBottom - oldTop
            if (height > 0 && height != oldHeight) setAtomHeight(atomKey, height)
        }
        atomLayoutListeners.remove(child)?.let(child::removeOnLayoutChangeListener)
        atomLayoutListeners[child] = listener
        child.addOnLayoutChangeListener(listener)
        if (child.height > 0) setAtomHeight(atomKey, child.height)
        layoutAtomHostViews()
    }

    internal fun unmountAtomChild(child: View): Boolean {
        val entry = atomHostViews.entries.firstOrNull { it.value === child } ?: return false
        atomHostViews.remove(entry.key)
        detachAtomView(child)
        measuredAtomHeightsPx.remove(entry.key)
        val fallbackHeight = atomRenderConfiguration?.measuredHeightsPx?.get(entry.key)
            ?: atomRenderConfiguration?.let { configuration ->
                val nodeType = atomSpan(entry.key)?.nodeType
                nodeType?.let { type ->
                    ((configuration.estimatedHeightsDp[type] ?: 0f) * resources.displayMetrics.density)
                        .roundToInt()
                }
            }
            ?: 0
        editorEditText.applyAtomHeight(
            entry.key,
            fallbackHeight,
            resolvedAtomRenderConfiguration()
        )
        layoutAtomHostViews()
        return true
    }

    private fun detachAtomView(child: View) {
        atomLayoutListeners.remove(child)?.let(child::removeOnLayoutChangeListener)
        (child.parent as? ViewGroup)?.removeView(child)
    }

    private fun setAtomHeight(atomKey: String, heightPx: Int) {
        if (heightPx <= 0 || measuredAtomHeightsPx[atomKey] == heightPx) return
        val span = atomSpan(atomKey) ?: return
        measuredAtomHeightsPx[atomKey] = heightPx
        editorEditText.applyAtomHeight(
            atomKey,
            heightPx,
            resolvedAtomRenderConfiguration()
        )
        span.reservedHeightPx = heightPx
    }

    private fun resolvedAtomRenderConfiguration(): AtomRenderConfiguration? {
        val configuration = atomRenderConfiguration ?: return null
        return configuration.copy(
            measuredHeightsPx = configuration.measuredHeightsPx + measuredAtomHeightsPx
        )
    }

    private fun atomSpan(atomKey: String): AtomBlockSpan? {
        val content = editorEditText.text as? android.text.Spanned ?: return null
        return content.getSpans(0, content.length, AtomBlockSpan::class.java)
            .firstOrNull { it.atomKey == atomKey }
    }

    internal fun layoutAtomHostViews() {
        emitAtomContentWidthIfAvailable()
        if (atomHostViews.isEmpty()) return
        val content = editorEditText.text as? android.text.Spanned ?: return
        val textLayout = editorEditText.layout ?: return
        val spans = content.getSpans(0, content.length, AtomBlockSpan::class.java)
            .associateBy { it.atomKey }
        for ((atomKey, child) in atomHostViews) {
            val span = spans[atomKey]
            val spanStart = span?.let(content::getSpanStart) ?: -1
            if (spanStart < 0) {
                child.visibility = View.INVISIBLE
                continue
            }
            val line = textLayout.getLineForOffset(spanStart)
            child.translationX = (
                editorEditText.left + editorEditText.compoundPaddingLeft
            ).toFloat()
            child.translationY = (
                editorEditText.top +
                    editorEditText.totalPaddingTop +
                    textLayout.getLineTop(line) -
                    editorEditText.scrollY
            ).toFloat()
            child.visibility = View.VISIBLE
            editorContentFrame.bringChildToFront(child)
        }
    }

    internal fun emitAtomContentWidthIfAvailable(force: Boolean = false) {
        if (atomRenderConfiguration == null) return
        val contentWidth = (
            editorEditText.width -
                editorEditText.compoundPaddingLeft -
                editorEditText.compoundPaddingRight
        ).coerceAtLeast(0)
        if (contentWidth > 0 && (force || contentWidth != lastAtomContentWidthPx)) {
            lastAtomContentWidthPx = contentWidth
            onAtomContentWidthChange?.invoke(contentWidth.toFloat())
        }
    }

    internal fun measuredAtomHeightForTesting(atomKey: String): Int? =
        measuredAtomHeightsPx[atomKey]

    internal fun atomHeightRenderApplyCountForTesting(): Int =
        editorEditText.atomHeightRenderApplyCountForTesting()

    fun setHeightBehavior(heightBehavior: EditorHeightBehavior) {
        if (this.heightBehavior == heightBehavior) return
        this.heightBehavior = heightBehavior
        editorEditText.setHeightBehavior(heightBehavior)
        editorEditText.layoutParams = createEditorLayoutParams()
        editorViewport.layoutParams = createContainerLayoutParams()
        editorScrollView.isVerticalScrollBarEnabled = heightBehavior == EditorHeightBehavior.FIXED
        editorScrollView.overScrollMode = if (heightBehavior == EditorHeightBehavior.FIXED) {
            OVER_SCROLL_IF_CONTENT_SCROLLS
        } else {
            OVER_SCROLL_NEVER
        }
        updateScrollContainerInsets()
        refreshOverlays()
        requestLayout()
    }

    fun setImageResizingEnabled(enabled: Boolean) {
        if (imageResizingEnabled == enabled) return
        imageResizingEnabled = enabled
        editorEditText.setImageResizingEnabled(enabled)
        refreshOverlays()
    }

    fun setViewportBottomInsetPx(bottomInsetPx: Int) {
        val clampedInset = bottomInsetPx.coerceAtLeast(0)
        if (viewportBottomInsetPx == clampedInset) return
        viewportBottomInsetPx = clampedInset
        updateScrollContainerInsets()
        editorEditText.setViewportBottomInsetPx(clampedInset)
        refreshOverlays()
        requestLayout()
    }

    fun setViewportBottomOcclusionTopOnScreenPx(topPx: Int?) {
        editorEditText.setViewportBottomOcclusionTopOnScreenPx(topPx)
    }

    internal fun viewportBottomInsetPxForTesting(): Int = viewportBottomInsetPx

    fun setRemoteSelections(selections: List<RemoteSelectionDecoration>) {
        remoteSelectionOverlayView.setRemoteSelections(selections)
    }

    fun refreshRemoteSelections() {
        if (!remoteSelectionOverlayView.hasSelectionsOrCachedGeometry()) return
        remoteSelectionOverlayView.refreshGeometry()
    }

    fun imageResizeOverlayRectForTesting(): android.graphics.RectF? =
        imageResizeOverlayView.visibleRectForTesting()

    fun resizeSelectedImageForTesting(widthPx: Float, heightPx: Float) {
        imageResizeOverlayView.simulateResizeForTesting(widthPx, heightPx)
    }

    internal fun dispatchImageResizeTouchForTesting(event: MotionEvent): Boolean =
        imageResizeOverlayView.onTouchEvent(event)

    fun remoteSelectionDebugSnapshotsForTesting(): List<RemoteSelectionDebugSnapshot> =
        remoteSelectionOverlayView.debugSnapshotsForTesting()

    fun setRemoteSelectionScalarResolverForTesting(resolver: (Long, Int) -> Int) {
        remoteSelectionOverlayView.docToScalarResolver = resolver
    }

    fun setRemoteSelectionEditorIdForTesting(editorId: Long) {
        remoteSelectionOverlayView.editorIdOverrideForTesting = editorId
    }

    fun setContent(html: String) {
        if (editorId == 0L) return
        val driver = editorEditText.v2Driver ?: return
        driver.setContentHtml(html)?.let {
            editorEditText.applyUpdateJSON(
                it,
                notifyListener = false,
                refreshInputConnectionForExternalUpdate = true
            )
        }
    }

    fun setContent(json: org.json.JSONObject) {
        if (editorId == 0L) return
        val driver = editorEditText.v2Driver ?: return
        driver.setContentJson(json.toString())?.let {
            editorEditText.applyUpdateJSON(
                it,
                notifyListener = false,
                refreshInputConnectionForExternalUpdate = true
            )
        }
    }

    internal fun rebindEditorIfNeeded(notifyListener: Boolean = true) {
        if (editorId != 0L && editorEditText.editorId != editorId) {
            setEditorId(editorId, bindEditor = true, notifyListener = notifyListener)
        }
    }

    internal fun setEditorIdWhileDetached(value: Long) {
        setEditorId(value, bindEditor = false)
    }

    internal fun deferEditorUnbindOnNextDetach() {
        deferEditorUnbindOnDetach = true
    }

    internal fun clearDeferredEditorUnbind() {
        deferEditorUnbindOnDetach = false
    }

    internal fun unbindEditorForDetachedViewIfNeeded() {
        if (isAttachedToWindow) return
        deferEditorUnbindOnDetach = false
        if (editorId != 0L) {
            editorEditText.unbindEditor()
        }
    }

    private fun setEditorId(value: Long, bindEditor: Boolean, notifyListener: Boolean = true) {
        val targetBoundEditorId = if (bindEditor) value else 0L
        if (currentEditorId == value && editorEditText.editorId == targetBoundEditorId) return
        imageResizeOverlayView.cancelActiveResize()
        if (currentEditorId != value || editorEditText.editorId != targetBoundEditorId) {
            editorEditText.discardTransientNativeInputForEditorRebind()
        }
        currentEditorId = value
        if (bindEditor && value != 0L) {
            editorEditText.bindEditor(value, notifyListener = notifyListener)
        } else {
            editorEditText.unbindEditor()
        }
        refreshOverlays()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        clearDeferredEditorUnbind()
        rebindEditorIfNeeded()
    }

    override fun onDetachedFromWindow() {
        onBeforeDetachedFromWindow?.invoke()
        imageResizeOverlayView.cancelActiveResize()
        super.onDetachedFromWindow()
        if (editorId != 0L && !deferEditorUnbindOnDetach) {
            editorEditText.unbindEditor()
        }
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        if (heightBehavior != EditorHeightBehavior.AUTO_GROW) {
            super.onMeasure(widthMeasureSpec, heightMeasureSpec)
            return
        }

        val childWidthSpec = getChildMeasureSpec(
            widthMeasureSpec,
            paddingLeft + paddingRight,
            editorViewport.layoutParams.width
        )
        val childHeightSpec = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED)
        editorViewport.measure(childWidthSpec, childHeightSpec)

        val measuredWidth = resolveSize(
            editorViewport.measuredWidth + paddingLeft + paddingRight,
            widthMeasureSpec
        )
        val desiredHeight = editorViewport.measuredHeight + paddingTop + paddingBottom
        val measuredHeight = when (MeasureSpec.getMode(heightMeasureSpec)) {
            MeasureSpec.AT_MOST -> desiredHeight.coerceAtMost(MeasureSpec.getSize(heightMeasureSpec))
            else -> desiredHeight
        }
        setMeasuredDimension(measuredWidth, measuredHeight)
    }

    /** Fills a host-imposed floor so its extra space stays tappable. */
    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
        super.onLayout(changed, left, top, right, bottom)
        if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
            val available = (bottom - top) - paddingTop - paddingBottom
            if (available > editorViewport.height) {
                editorViewport.measure(
                    MeasureSpec.makeMeasureSpec(editorViewport.width, MeasureSpec.EXACTLY),
                    MeasureSpec.makeMeasureSpec(available, MeasureSpec.EXACTLY),
                )
                editorViewport.layout(
                    editorViewport.left,
                    paddingTop,
                    editorViewport.right,
                    paddingTop + available,
                )
            }
        }
        layoutAtomHostViews()
    }

    private fun updateScrollContainerAppearance() {
        val cornerRadiusPx = (theme?.borderRadius ?: 0f) * resources.displayMetrics.density
        val backgroundColor = theme?.backgroundColor ?: baseBackgroundColor
        editorViewport.background = GradientDrawable().apply {
            cornerRadius = cornerRadiusPx
            setColor(backgroundColor)
        }
        editorViewport.clipToOutline = cornerRadiusPx > 0f
        editorScrollView.setBackgroundColor(Color.TRANSPARENT)
        appliedCornerRadiusPx = cornerRadiusPx
        appliedBackgroundColorForTesting = backgroundColor
    }

    private fun updateScrollContainerInsets() {
        if (heightBehavior != EditorHeightBehavior.FIXED) {
            editorScrollView.setPadding(0, 0, 0, 0)
            return
        }

        val density = resources.displayMetrics.density
        val topInset = ((theme?.contentInsets?.top ?: 0f) * density).toInt()
        val bottomInset = ((theme?.contentInsets?.bottom ?: 0f) * density).toInt()
        editorScrollView.setPadding(0, topInset, 0, bottomInset + viewportBottomInsetPx)
    }

    private fun createContainerLayoutParams(): LayoutParams =
        if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
            LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT)
        } else {
            LayoutParams(LayoutParams.MATCH_PARENT, 0, 1f)
        }

    private fun createEditorLayoutParams(): FrameLayout.LayoutParams =
        FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.MATCH_PARENT
        )

    internal fun selectedImageGeometry(): EditorEditText.SelectedImageGeometry? {
        val geometry = editorEditText.selectedImageGeometry() ?: return null
        return EditorEditText.SelectedImageGeometry(
            docPos = geometry.docPos,
            rect = RectF(
                editorViewport.left + editorScrollView.left + editorEditText.left + geometry.rect.left,
                editorViewport.top + editorScrollView.top + editorEditText.top + geometry.rect.top - editorScrollView.scrollY,
                editorViewport.left + editorScrollView.left + editorEditText.left + geometry.rect.right,
                editorViewport.top + editorScrollView.top + editorEditText.top + geometry.rect.bottom - editorScrollView.scrollY
            )
        )
    }

    internal fun caretRect(): RectF? {
        val rect = editorEditText.caretRect() ?: return null
        return RectF(
            editorViewport.left + editorScrollView.left + editorEditText.left + rect.left,
            editorViewport.top + editorScrollView.top + editorEditText.top + rect.top - editorScrollView.scrollY,
            editorViewport.left + editorScrollView.left + editorEditText.left + rect.right,
            editorViewport.top + editorScrollView.top + editorEditText.top + rect.bottom - editorScrollView.scrollY
        )
    }

    internal fun maximumImageWidthPx(): Float {
        val availableWidth =
            maxOf(editorEditText.width, editorEditText.measuredWidth) -
                editorEditText.compoundPaddingLeft -
                editorEditText.compoundPaddingRight
        return availableWidth.coerceAtLeast(48).toFloat()
    }

    internal fun clampImageSize(
        widthPx: Float,
        heightPx: Float,
        maximumWidthPx: Float = maximumImageWidthPx()
    ): Pair<Float, Float> {
        val aspectRatio = maxOf(widthPx / maxOf(heightPx, 1f), 0.1f)
        val clampedWidth = minOf(maxOf(48f, maximumWidthPx), maxOf(48f, widthPx))
        val clampedHeight = maxOf(48f, clampedWidth / aspectRatio)
        return clampedWidth to clampedHeight
    }

    internal fun resizeImage(docPos: Int, widthPx: Float, heightPx: Float) {
        val (clampedWidth, clampedHeight) = clampImageSize(widthPx, heightPx)
        editorEditText.resizeImageAtDocPos(docPos, clampedWidth, clampedHeight)
    }

    private fun refreshOverlays() {
        layoutAtomHostViews()
        remoteSelectionOverlayView.refreshGeometry()
        imageResizeOverlayView.refresh()
    }
}
