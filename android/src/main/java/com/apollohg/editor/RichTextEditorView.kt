package com.apollohg.editor

import android.content.Context
import android.graphics.Color
import android.graphics.RectF
import android.graphics.drawable.GradientDrawable
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView

/** Container view that owns the native editor text field. */
class RichTextEditorView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0
) : LinearLayout(context, attrs, defStyleAttr) {
    val editorViewport: FrameLayout

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

    val editorEditText: EditorEditText
    val editorScrollView: ScrollView
    private val remoteSelectionOverlayView: RemoteSelectionOverlayView
    private val imageResizeOverlayView: ImageResizeOverlayView

    private var heightBehavior: EditorHeightBehavior = EditorHeightBehavior.FIXED
    private var imageResizingEnabled = true
    private var theme: EditorTheme? = null
    private var baseBackgroundColor: Int = Color.WHITE
    private var viewportBottomInsetPx: Int = 0
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
        editorScrollView = EditorScrollView(context).apply {
            clipToPadding = false
            isFillViewport = false
        }
        editorViewport = FrameLayout(context)
        remoteSelectionOverlayView = RemoteSelectionOverlayView(context)
        imageResizeOverlayView = ImageResizeOverlayView(context)
        editorScrollView.addView(editorEditText, createEditorLayoutParams())
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
            ViewGroup.LayoutParams.WRAP_CONTENT
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
        remoteSelectionOverlayView.refreshGeometry()
        imageResizeOverlayView.refresh()
    }
}
