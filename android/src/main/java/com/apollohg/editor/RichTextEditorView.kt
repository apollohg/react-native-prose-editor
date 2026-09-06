package com.apollohg.editor

import android.content.Context
import android.graphics.Color
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.drawable.GradientDrawable
import android.os.Handler
import android.os.Looper
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.ViewConfiguration
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import kotlin.math.roundToInt
import com.facebook.react.uimanager.PointerEvents
import com.facebook.react.uimanager.ReactPointerEventsView

internal data class AtomLayoutPosition(
    val key: String,
    val xPx: Int,
    val yPx: Int,
    val heightPx: Int,
    val widthPx: Int,
)

/** Container view that owns the native editor text field. */
class RichTextEditorView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0
) : LinearLayout(context, attrs, defStyleAttr) {
    val editorViewport: FrameLayout
    val editorContentFrame: FrameLayout

    private class DecorationLayer(context: Context) : FrameLayout(context), ReactPointerEventsView {
        override val pointerEvents = PointerEvents.NONE
    }

    private inner class EditorScrollView(context: Context) : ScrollView(context) {
        private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop
        private var atomDown = false
        private var downX = 0f
        private var downY = 0f
        private var horizontalAtomGesture = false
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
            if (ev.actionMasked == MotionEvent.ACTION_DOWN) {
                downX = ev.x
                downY = ev.y
                horizontalAtomGesture = false
                atomDown = atomHostViews.values.any { child ->
                    val x = ev.x + scrollX - editorContentFrame.left
                    val y = ev.y + scrollY - editorContentFrame.top
                    child.visibility == View.VISIBLE && x >= child.left && x < child.right && y >= child.top && y < child.bottom
                }
                pendingAtomAnchor = null
            }
            if (atomDown && ev.actionMasked == MotionEvent.ACTION_MOVE) {
                val dx = kotlin.math.abs(ev.x - downX)
                val dy = kotlin.math.abs(ev.y - downY)
                if (dx > touchSlop && dx > dy) horizontalAtomGesture = true
            }
            if (horizontalAtomGesture) return false
            updateParentIntercept(ev.actionMasked)
            return super.onInterceptTouchEvent(ev)
        }

        override fun onTouchEvent(ev: MotionEvent): Boolean {
            if (ev.actionMasked == MotionEvent.ACTION_DOWN) pendingAtomAnchor = null
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
            val width = atomHostViews.entries.firstOrNull { it.value === child }?.key?.let(::atomWidthPx)
                ?: child.measuredWidth.takeIf { it > 0 }
                ?: child.width.coerceAtLeast(0)
            val height = renderedAtomHeightPx(child)
                ?: atomHostViews.entries.firstOrNull { it.value === child }?.key?.let(::atomSpan)?.reservedHeightPx
                ?: child.measuredHeight.takeIf { it > 0 }
                ?: child.height.coerceAtLeast(0)
            child.measure(
                MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY),
                MeasureSpec.makeMeasureSpec(height, MeasureSpec.EXACTLY),
            )
        }

        override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
            super.onLayout(changed, left, top, right, bottom)
            restoreAtomScrollAnchor()
            layoutAtomHostViews()
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
    private var lastAtomViewportHeight = -1
    internal var onAtomLayoutChange: ((Float, List<AtomLayoutPosition>) -> Unit)? = null
    private var atomRenderConfiguration: AtomRenderConfiguration? = null
    private val atomHostViews = linkedMapOf<String, View>()
    private val atomLayoutListeners = mutableMapOf<View, View.OnLayoutChangeListener>()
    private val atomMeasurementHandler = Handler(Looper.getMainLooper())
    private val pendingAtomMeasurements = mutableMapOf<View, Runnable>()
    private val measuredAtomHeightsPx = mutableMapOf<String, Int>()
    private var atomMeasurementGeneration = 0L
    private var atomMeasurementWidth = -1
    private var positioningAtoms = false
    private data class AtomScrollAnchor(val offset: Int, val top: Int, val scrollY: Int)
    private var pendingAtomAnchor: AtomScrollAnchor? = null
    private var lastAtomContentWidthPx = -1
    private var lastAtomLayoutPositions = emptyList<AtomLayoutPosition>()
    internal var appliedCornerRadiusPx: Float = 0f
    internal var appliedBackgroundColorForTesting: Int = Color.WHITE
    internal var onAutoGrowHeightMayChange: (() -> Unit)? = null

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
        val decorationLayer = DecorationLayer(context)
        editorViewport.addView(decorationLayer, FrameLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
        decorationLayer.addView(
            remoteSelectionOverlayView,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT
            )
        )
        decorationLayer.addView(
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
            editorEditText.surfaceViewportChanged()
            refreshOverlays()
            emitAtomLayoutIfAvailable(force = true)
        }
        editorEditText.onSelectionOrContentMayChange = { refreshOverlays() }
        editorEditText.onContentSizeMayChange = {
            if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
                onAutoGrowHeightMayChange?.invoke()
            }
        }

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
        cancelAtomMeasurement(child)
        atomHostViews.entries.firstOrNull { it.value === child }?.let { existing ->
            if (existing.key != atomKey) {
                atomHostViews.remove(existing.key)
                restoreConfiguredAtomHeight(existing.key)
            }
        }
        atomHostViews[atomKey]?.takeIf { it !== child }?.let(::detachAtomView)
        atomHostViews[atomKey] = child
        if (child.parent !== editorContentFrame) {
            (child.parent as? ViewGroup)?.removeView(child)
            editorContentFrame.addView(child)
        }
        val listener = View.OnLayoutChangeListener { view, _, _, _, _, _, _, _, _ ->
            if (!positioningAtoms) {
                val observedHeight = view.height
                val observedWidth = view.width
                layoutAtomHostViews()
                scheduleAtomMeasurement(view, atomKey, observedWidth, observedHeight)
            }
        }
        atomLayoutListeners.remove(child)?.let(child::removeOnLayoutChangeListener)
        atomLayoutListeners[child] = listener
        child.addOnLayoutChangeListener(listener)
        val initialHeight = renderedAtomHeightPx(child) ?: child.height
        if (initialHeight > 0 || (initialHeight == 0 && child.isLaidOut)) {
            constrainAtomHostBounds(child, child.width, initialHeight)
            setAtomHeight(atomKey, initialHeight)
        }
        layoutAtomHostViews()
    }

    internal fun orderAtomChildren(children: List<View>) {
        val keys = atomHostViews.entries.associate { it.value to it.key }
        children.forEach { child ->
            keys[child]?.let { key -> atomHostViews.remove(key); atomHostViews[key] = child }
        }
        ensureAtomHostZOrder()
    }

    private fun invalidateAtomMeasurements(clearHeights: Boolean = false) {
        atomMeasurementGeneration += 1
        pendingAtomMeasurements.values.forEach(atomMeasurementHandler::removeCallbacks)
        pendingAtomMeasurements.clear()
        pendingAtomAnchor = null
        if (clearHeights) measuredAtomHeightsPx.clear()
    }

    private fun scheduleAtomMeasurement(child: View, atomKey: String, observedWidth: Int = child.width, observedHeight: Int = child.height) {
        if (pendingAtomMeasurements.containsKey(child)) return
        val generation = atomMeasurementGeneration
        val width = atomWidthPx(atomKey) ?: observedWidth.takeIf { it > 0 } ?: return
        val owner = currentEditorId
        // Fabric commits host bounds before descendant bounds.
        val measurement = Runnable {
            pendingAtomMeasurements.remove(child)
            if (generation != atomMeasurementGeneration || owner != currentEditorId ||
                atomHostViews[atomKey] !== child || child.parent !== editorContentFrame || (atomWidthPx(atomKey) ?: width) != width) return@Runnable
            val height = if (child !is ViewGroup || child.childCount == 0) {
                observedHeight.takeIf { observedWidth == width && it >= 0 }
            } else renderedAtomHeightPx(child, width)
            if (height == null) return@Runnable
            constrainAtomHostBounds(child, width, height)
            setAtomHeight(atomKey, height)
            layoutAtomHostViews()
        }
        pendingAtomMeasurements[child] = measurement
        atomMeasurementHandler.post(measurement)
    }

    private fun cancelAtomMeasurement(child: View) {
        pendingAtomMeasurements.remove(child)?.let(atomMeasurementHandler::removeCallbacks)
    }

    private fun atomContentWidthPx(): Int? = (
        (editorEditText.width.takeIf { it > 0 } ?: editorEditText.measuredWidth) -
            editorEditText.compoundPaddingLeft - editorEditText.compoundPaddingRight
    ).takeIf { it > 0 }

    private fun renderedAtomHeightPx(host: View, expectedWidth: Int? = null): Int? {
        val group = host as? ViewGroup
        if (group == null || group.childCount == 0) {
            return host.height.takeIf { host.isLaidOut && (expectedWidth == null || host.width == expectedWidth) }
        }
        val measurementRoot = (0 until group.childCount).asSequence().map(group::getChildAt).firstOrNull {
            (it.getTag(com.facebook.react.R.id.view_tag_native_id) as? String)?.startsWith("prose-atom-content:") == true
        }
        if (measurementRoot != null) {
            return measurementRoot.height.takeIf {
                measurementRoot.isLaidOut && (expectedWidth == null || measurementRoot.width == expectedWidth)
            }
        }
        var bottom = 0
        var hasLaidOutChild = false
        for (index in 0 until group.childCount) {
            val child = group.getChildAt(index)
            if (child.visibility != View.GONE) {
                if (expectedWidth != null && child.width != expectedWidth) return null
                bottom = maxOf(bottom, child.bottom)
                hasLaidOutChild = hasLaidOutChild || child.isLaidOut
            }
        }
        return bottom.takeIf { it > 0 || hasLaidOutChild }
    }

    private fun constrainAtomHostBounds(host: View, width: Int, height: Int) {
        if (width <= 0 || height < 0) return
        positioningAtoms = true
        try {
            host.measure(MeasureSpec.makeMeasureSpec(width, MeasureSpec.EXACTLY),
                MeasureSpec.makeMeasureSpec(height, MeasureSpec.EXACTLY))
            host.layout(host.left, host.top, host.left + width, host.top + height)
        } finally { positioningAtoms = false }
    }

    internal fun unmountAtomChild(child: View): Boolean {
        val entry = atomHostViews.entries.firstOrNull { it.value === child } ?: return false
        atomHostViews.remove(entry.key)
        detachAtomView(child)
        restoreConfiguredAtomHeight(entry.key)
        layoutAtomHostViews()
        return true
    }

    private fun restoreConfiguredAtomHeight(atomKey: String) {
        measuredAtomHeightsPx.remove(atomKey)
        val nodeType = atomSpan(atomKey)?.nodeType
        val fallbackHeight = atomRenderConfiguration?.let { configuration ->
            nodeType?.let { configuration.reservedHeightPx(atomKey, it, resources.displayMetrics.density) }
        } ?: 0
        captureAtomScrollAnchor()
        editorEditText.applyAtomHeight(atomKey, fallbackHeight, resolvedAtomRenderConfiguration())
    }

    private fun detachAtomView(child: View) {
        cancelAtomMeasurement(child)
        atomLayoutListeners.remove(child)?.let(child::removeOnLayoutChangeListener)
        (child.parent as? ViewGroup)?.removeView(child)
    }

    private fun setAtomHeight(atomKey: String, heightPx: Int) {
        if (heightPx < 0 || measuredAtomHeightsPx[atomKey] == heightPx) return
        measuredAtomHeightsPx[atomKey] = heightPx
        captureAtomScrollAnchor()
        editorEditText.applyAtomHeight(atomKey, heightPx, resolvedAtomRenderConfiguration())
    }

    private fun captureAtomScrollAnchor() {
        if (pendingAtomAnchor != null || editorScrollView.scrollY <= 0) return
        val layout = editorEditText.layout ?: return
        val top = editorScrollView.scrollY - editorEditText.top - editorEditText.totalPaddingTop
        val line = layout.getLineForVertical(top.coerceAtLeast(0))
        pendingAtomAnchor = AtomScrollAnchor(layout.getLineStart(line), layout.getLineTop(line), editorScrollView.scrollY)
    }

    private fun restoreAtomScrollAnchor() {
        val anchor = pendingAtomAnchor ?: return
        pendingAtomAnchor = null
        val layout = editorEditText.layout ?: return
        val offset = anchor.offset.coerceIn(0, layout.text.length)
        val delta = layout.getLineTop(layout.getLineForOffset(offset)) - anchor.top
        editorScrollView.scrollTo(editorScrollView.scrollX, (anchor.scrollY + delta).coerceAtLeast(0))
    }

    private fun resolvedAtomRenderConfiguration(): AtomRenderConfiguration? {
        val configuration = atomRenderConfiguration ?: return null
        return configuration.copy(measuredHeightsPx = configuration.measuredHeightsPx + measuredAtomHeightsPx)
    }

    private fun atomSpan(atomKey: String): AtomBlockSpan? {
        val content = editorEditText.text as? android.text.Spanned ?: return null
        return content.getSpans(0, content.length, AtomBlockSpan::class.java).firstOrNull { it.atomKey == atomKey }
    }

    private fun atomBounds(span: AtomBlockSpan, content: android.text.Spanned, layout: android.text.Layout): Rect? {
        val offset = content.getSpanStart(span)
        if (offset < 0) return null
        val line = layout.getLineForOffset(offset)
        val composite = layout as? EditorDocumentLayout
        val left = composite?.contentLeft(line) ?: layout.getParagraphLeft(line).toFloat()
        val right = composite?.contentRight(line) ?: layout.getParagraphRight(line).toFloat()
        val top = composite?.textLineTop(line) ?: layout.getLineTop(line)
        val x = editorEditText.left + editorEditText.compoundPaddingLeft
        val y = editorEditText.top + editorEditText.totalPaddingTop
        return Rect(x + left.roundToInt(), y + top, x + right.roundToInt(), y + top + span.reservedHeightPx)
    }

    private fun atomWidthPx(atomKey: String): Int? {
        if (atomContentWidthPx() == null) return null
        val content = editorEditText.text as? android.text.Spanned ?: return atomContentWidthPx()
        val layout = editorEditText.layout ?: return atomContentWidthPx()
        return atomSpan(atomKey)?.let { atomBounds(it, content, layout)?.width() } ?: atomContentWidthPx()
    }

    internal fun layoutAtomHostViews() {
        if (positioningAtoms) return
        val width = atomContentWidthPx() ?: return
        val content = editorEditText.text as? android.text.Spanned ?: return
        val textLayout = editorEditText.layout
        if (atomMeasurementWidth != width) {
            atomMeasurementWidth = width
            invalidateAtomMeasurements()
        }
        val spanList = content.getSpans(0, content.length, AtomBlockSpan::class.java).toList()
        emitAtomLayoutIfAvailable(content, textLayout, spanList)
        val spans = spanList.associateBy { it.atomKey }
        positioningAtoms = true
        try {
            for ((key, child) in atomHostViews) {
                val bounds = spans[key]?.let { atomBounds(it, content, textLayout) }
                if (bounds == null) { child.visibility = View.INVISIBLE; continue }
                child.translationX = 0f
                child.translationY = 0f
                if (child.measuredWidth != bounds.width() || child.measuredHeight != bounds.height()) {
                    child.measure(MeasureSpec.makeMeasureSpec(bounds.width().coerceAtLeast(0), MeasureSpec.EXACTLY),
                        MeasureSpec.makeMeasureSpec(bounds.height().coerceAtLeast(0), MeasureSpec.EXACTLY))
                }
                child.layout(bounds.left, bounds.top, bounds.right, bounds.bottom)
                child.visibility = View.VISIBLE
            }
        } finally { positioningAtoms = false }
        ensureAtomHostZOrder()
    }

    private fun ensureAtomHostZOrder() {
        val atoms = atomHostViews.values.filter { it.parent === editorContentFrame }
        val first = editorContentFrame.childCount - atoms.size
        if (first >= 0 && atoms.indices.all { editorContentFrame.getChildAt(first + it) === atoms[it] }) return
        atoms.forEach(editorContentFrame::bringChildToFront)
    }

    internal fun emitAtomLayoutIfAvailable(force: Boolean = false) {
        if (atomRenderConfiguration == null || atomContentWidthPx() == null) return
        val content = editorEditText.text as? android.text.Spanned ?: return
        val layout = editorEditText.layout ?: return
        emitAtomLayoutIfAvailable(content, layout, content.getSpans(0, content.length, AtomBlockSpan::class.java).toList(), force)
    }

    private fun emitAtomLayoutIfAvailable(content: android.text.Spanned, layout: android.text.Layout,
                                         spans: List<AtomBlockSpan>, force: Boolean = false) {
        if (atomRenderConfiguration == null) return
        val width = atomContentWidthPx() ?: return
        val positions = spans.mapNotNull { span ->
            atomBounds(span, content, layout)?.let { AtomLayoutPosition(span.atomKey, it.left, it.top, it.height(), it.width()) }
        }
        if (force || editorScrollView.height != lastAtomViewportHeight || width != lastAtomContentWidthPx || positions != lastAtomLayoutPositions) {
            lastAtomViewportHeight = editorScrollView.height
            lastAtomContentWidthPx = width
            lastAtomLayoutPositions = positions
            onAtomLayoutChange?.invoke(width.toFloat(), positions)
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
        invalidateAtomMeasurements(clearHeights = true)
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
        atomHostViews.forEach { (key, child) -> scheduleAtomMeasurement(child, key) }
    }

    override fun onDetachedFromWindow() {
        invalidateAtomMeasurements()
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
        val editorOrigin = Rect()
        editorViewport.offsetDescendantRectToMyCoords(editorEditText, editorOrigin)
        return EditorEditText.SelectedImageGeometry(
            docPos = geometry.docPos,
            rect = RectF(geometry.rect).apply {
                offset(editorOrigin.left.toFloat(), editorOrigin.top.toFloat())
            }
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
