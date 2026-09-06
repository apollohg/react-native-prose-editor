package com.apollohg.editor.viewer

import android.content.Context
import android.content.res.Configuration
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import android.util.AttributeSet
import android.view.View
import android.view.MotionEvent
import android.view.ViewConfiguration
import android.view.ViewTreeObserver
import android.os.Bundle
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeProvider
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import com.apollohg.editor.AndroidApiCompat
import com.apollohg.editor.DecodedBitmapLease
import com.apollohg.editor.DecodedBitmapBudget

/** Rendering-only consumer of fully prepared StaticLayout and geometry fragments. */
internal class PreparedProseDrawingView @JvmOverloads constructor(context: Context, attrs: AttributeSet? = null) : View(context, attrs) {
    private val accessibilityManager = context.getSystemService(AccessibilityManager::class.java)
    var preparedLayout: PreparedProseLayout? = null
        private set
    var onCodeHighlightsReady: (() -> Unit)? = null
    private val codeHighlighting = ViewerCodeHighlighting(this)
    var onUsableMetricsChanged: (() -> Unit)? = null
    var onVisibleRectChanged: ((Rect) -> Unit)? = null
    var onFontConfigurationChanged: ((Configuration) -> Unit)? = null
    var onInteractionActivated: ((PreparedProseInteraction) -> Boolean)? = null
    private val imagePixelsLock = Any()
    private val imagePixels = mutableMapOf<String, DecodedBitmapLease>()
    /** Map overhead only; decoded allocation bytes are charged by the shared lease budget. */
    internal val retainedImagePixelsBytesForTesting: Long
        get() = synchronized(imagePixelsLock) { retainedImagePixelsBytes(imagePixels) }
    /** False when a public host owns this view's virtual subtree and notifications. */
    var publishesAccessibilitySubtree: Boolean = true
    internal var accessibilityVisibilityForTesting: ((Rect) -> Boolean)? = null
    var linkInteractionsEnabled: Boolean = true
        set(value) {
            if (field == value) return
            clearVirtualAccessibilityFocus()
            field = value
            announceAccessibilitySubtreeChanged()
        }
    var mentionInteractionsEnabled: Boolean = false
        set(value) {
            if (field == value) return
            clearVirtualAccessibilityFocus()
            field = value
            announceAccessibilitySubtreeChanged()
        }
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop.toFloat()
    private var pendingTap: PendingTap? = null
    private var focusedVirtualNode: FocusedVirtualNode? = null
    private var contentOriginXPx = 0
    private var contentOriginYPx = 0
    private val scrollChangedListener = ViewTreeObserver.OnScrollChangedListener {
        reconcileVirtualAccessibilityFocus()
    }

    init {
        DecodedBitmapBudget.shared(context)
    }

    internal companion object {
        const val IMAGE_PIXEL_MAP_RETAINED_BYTES = 48L
        const val IMAGE_PIXEL_ENTRY_RETAINED_BYTES = 48L

        fun retainedImagePixelsBytes(pixels: Map<String, *>): Long {
            if (pixels.isEmpty()) return 0L
            var retained = IMAGE_PIXEL_MAP_RETAINED_BYTES
            pixels.values.forEach {
                retained = saturatingAdd(retained, IMAGE_PIXEL_ENTRY_RETAINED_BYTES)
            }
            return retained
        }

        private fun saturatingAdd(left: Long, right: Long): Long =
            if (right > 0 && left > Long.MAX_VALUE - right) Long.MAX_VALUE else left + right

        private fun saturatingMultiply(left: Long, right: Long): Long = when {
            left <= 0L || right <= 0L -> 0L
            left > Long.MAX_VALUE / right -> Long.MAX_VALUE
            else -> left * right
        }
    }

    fun putImageLease(id: String, lease: DecodedBitmapLease) {
        synchronized(imagePixelsLock) { imagePixels.put(id, lease) }?.close()
        reportRetainedImagePixels()
        postInvalidate()
    }

    fun removeImageLeases(ids: Set<String>) {
        val released = synchronized(imagePixelsLock) { ids.mapNotNull(imagePixels::remove) }
        if (released.isEmpty()) return
        released.forEach(DecodedBitmapLease::close)
        reportRetainedImagePixels()
        postInvalidate()
    }

    fun clearImageLeases() {
        val released = synchronized(imagePixelsLock) {
            imagePixels.values.toList().also { imagePixels.clear() }
        }
        if (released.isEmpty()) return
        released.forEach(DecodedBitmapLease::close)
        reportRetainedImagePixels()
        postInvalidate()
    }

    private fun reportRetainedImagePixels() {
        PreparedProseInstrumentation.retained(
            PreparedProseInstrumentation.Owner.IMAGE,
            "drawing-${System.identityHashCode(this)}",
            synchronized(imagePixelsLock) { retainedImagePixelsBytes(imagePixels) },
        )
    }

    /**
     * Publishes a prepared artifact. Replacement owners suppress the transient
     * clear announcement and let the final install report the one logical
     * subtree transition; focus-cleared events remain immediate.
     */
    fun install(
        layout: PreparedProseLayout?,
        announceAccessibilitySubtree: Boolean = true,
        contentOriginXPx: Int = 0,
        contentOriginYPx: Int = 0,
    ) {
        if (
            preparedLayout === layout &&
            this.contentOriginXPx == contentOriginXPx &&
            this.contentOriginYPx == contentOriginYPx
        ) return
        clearVirtualAccessibilityFocus()
        preparedLayout = layout
        codeHighlighting.update()
        this.contentOriginXPx = contentOriginXPx
        this.contentOriginYPx = contentOriginYPx
        if (announceAccessibilitySubtree) announceAccessibilitySubtreeChanged()
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        reconcileVirtualAccessibilityFocus()
        val artifact = preparedLayout ?: return
        val saved = canvas.save()
        canvas.translate(contentOriginXPx.toFloat(), contentOriginYPx.toFloat())
        canvas.clipRect(0, 0, artifact.widthPx, artifact.heightPx)
        try {
            artifact.contentBox?.let { com.apollohg.editor.EditorBoxDrawing.draw(canvas, RectF(0f, 0f, artifact.widthPx.toFloat(), artifact.heightPx.toFloat()), it) }
            recordPreparedProseDraw {
                onVisibleRectChanged?.invoke(Rect(canvas.clipBounds))
                val visible = mutableListOf<PreparedProseFragment>()
                var visibleBlockCount = 0
                artifact.forEachBlockIntersecting(canvas.clipBounds) { block -> visible += block.fragments; visibleBlockCount += 1 }
                // Phases stay global across blocks: later code backgrounds cannot cover
                // an earlier quote border, and text/labels always remain foreground.
                visible.forEach { drawBackground(canvas, it) }
                visible.forEach { drawBorderOrRule(canvas, it) }
                visible.forEach { drawForeground(canvas, it) }
                visibleBlockCount
            }
        } finally {
            canvas.restoreToCount(saved)
        }
    }

    private fun drawBackground(canvas: Canvas, fragment: PreparedProseFragment) {
        if (fragment.kind != PreparedProseFragmentKind.BACKGROUND && fragment.kind != PreparedProseFragmentKind.ATOM && fragment.kind != PreparedProseFragmentKind.IMAGE) return
        fragment.box?.let { box ->
            val saved = canvas.save()
            canvas.clipRect(fragment.bounds)
            com.apollohg.editor.EditorBoxDrawing.draw(canvas, RectF(fragment.decorationBounds ?: fragment.bounds), box)
            canvas.restoreToCount(saved)
            return
        }
        paint.style = Paint.Style.FILL
        paint.color = fragment.color ?: return
        canvas.drawRoundRect(RectF(fragment.bounds), fragment.cornerRadius, fragment.cornerRadius, paint)
    }

    private fun drawBorderOrRule(canvas: Canvas, fragment: PreparedProseFragment) {
        when (fragment.kind) {
            PreparedProseFragmentKind.BORDER, PreparedProseFragmentKind.RULE -> {
                paint.style = Paint.Style.FILL
                paint.color = fragment.color ?: return
                canvas.drawRect(fragment.bounds, paint)
            }
            PreparedProseFragmentKind.ATOM -> if (fragment.strokeWidth > 0f) {
                paint.style = Paint.Style.STROKE
                paint.strokeWidth = fragment.strokeWidth
                paint.color = fragment.borderColor ?: fragment.color ?: return
                val inset = fragment.strokeWidth / 2f
                canvas.drawRoundRect(RectF(fragment.bounds).apply { inset(inset, inset) }, maxOf(0f, fragment.cornerRadius - inset), maxOf(0f, fragment.cornerRadius - inset), paint)
            }
            else -> Unit
        }
    }

    private fun drawForeground(canvas: Canvas, fragment: PreparedProseFragment) {
        when (fragment.kind) {
            PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.MARKER -> {
                fragment.layout?.let { layout ->
                    val saved = canvas.save()
                    canvas.translate(fragment.layoutX.toFloat(), fragment.layoutY.toFloat())
                    layout.draw(canvas)
                    com.apollohg.editor.EditorTextDecorationDrawing.draw(canvas, layout)
                    canvas.restoreToCount(saved)
                } ?: if (fragment.kind == PreparedProseFragmentKind.MARKER) drawTaskMarker(canvas, fragment) else Unit
            }
            PreparedProseFragmentKind.ATOM -> fragment.labelLayout?.let { layout ->
                val saved = canvas.save()
                canvas.translate(fragment.labelX.toFloat(), fragment.labelY.toFloat())
                layout.draw(canvas)
                com.apollohg.editor.EditorTextDecorationDrawing.draw(canvas, layout)
                canvas.restoreToCount(saved)
            }
            PreparedProseFragmentKind.STRIKE -> {
                paint.style = Paint.Style.FILL
                paint.color = fragment.color ?: return
                canvas.drawRect(fragment.bounds, paint)
            }
            PreparedProseFragmentKind.IMAGE -> {
                val attachment = preparedLayout?.imageAttachments?.firstOrNull { it.bounds == fragment.bounds } ?: return
                val bitmap = synchronized(imagePixelsLock) { imagePixels[attachment.id]?.bitmap } ?: return
                fragment.box?.let {
                    com.apollohg.editor.EditorBoxDrawing.drawImage(canvas, bitmap, RectF(fragment.bounds), it, fragment.resizeMode)
                } ?: canvas.drawBitmap(bitmap, null, fragment.bounds, paint)
            }
            else -> Unit
        }
    }

    private fun drawTaskMarker(canvas: Canvas, fragment: PreparedProseFragment) {
        val bounds = RectF(fragment.bounds)
        fragment.box?.let {
            com.apollohg.editor.drawCheckbox(canvas, bounds, it, fragment.checked, fragment.borderColor ?: fragment.color ?: android.graphics.Color.BLACK)
            return
        }
        val inset = maxOf(1f, bounds.height() * 0.2f)
        val box = RectF(bounds).apply { inset(inset, inset) }
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = maxOf(1f, box.width() * 0.1f)
        paint.color = fragment.color ?: return
        canvas.drawRoundRect(box, box.width() * 0.2f, box.width() * 0.2f, paint)
        if (!fragment.checked) return
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = maxOf(1.4f, box.width() * 0.12f)
        paint.strokeCap = Paint.Cap.ROUND
        paint.strokeJoin = Paint.Join.ROUND
        val path = android.graphics.Path().apply {
            moveTo(box.left + box.width() * 0.2f, box.centerY())
            lineTo(box.left + box.width() * 0.43f, box.bottom - box.height() * 0.2f)
            lineTo(box.right - box.width() * 0.16f, box.top + box.height() * 0.2f)
        }
        canvas.drawPath(path, paint)
        paint.strokeCap = Paint.Cap.BUTT
        paint.strokeJoin = Paint.Join.MITER
    }

    override fun onSizeChanged(width: Int, height: Int, oldWidth: Int, oldHeight: Int) {
        super.onSizeChanged(width, height, oldWidth, oldHeight)
        reconcileVirtualAccessibilityFocus()
        if (width > 0) onUsableMetricsChanged?.invoke()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        codeHighlighting.update()
        viewTreeObserver.addOnScrollChangedListener(scrollChangedListener)
        reconcileVirtualAccessibilityFocus()
        if (width > 0) onUsableMetricsChanged?.invoke()
    }

    override fun onDetachedFromWindow() {
        codeHighlighting.cancel()
        if (viewTreeObserver.isAlive) {
            viewTreeObserver.removeOnScrollChangedListener(scrollChangedListener)
        }
        clearVirtualAccessibilityFocus()
        super.onDetachedFromWindow()
    }

    override fun onVisibilityChanged(changedView: View, visibility: Int) {
        super.onVisibilityChanged(changedView, visibility)
        reconcileVirtualAccessibilityFocus()
    }

    override fun onWindowVisibilityChanged(visibility: Int) {
        super.onWindowVisibilityChanged(visibility)
        reconcileVirtualAccessibilityFocus()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        onFontConfigurationChanged?.invoke(newConfig)
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        val contentX = event.x - contentOriginXPx
        val contentY = event.y - contentOriginYPx
        fun targetAt(): PreparedProseInteraction? = preparedLayout?.interactions?.firstOrNull { interaction ->
            if (contentX < 0f || contentY < 0f) return@firstOrNull false
            interactionEnabled(interaction.kind) &&
                interaction.rects.any { it.contains(contentX.toInt(), contentY.toInt()) }
        }
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                pendingTap = if (event.pointerCount == 1) targetAt()?.let { PendingTap(it, event.getPointerId(event.actionIndex), event.x, event.y) } else null
                return pendingTap != null
            }
            MotionEvent.ACTION_MOVE -> {
                pendingTap?.let { tap -> if (event.pointerCount != 1 || event.findPointerIndex(tap.pointerId) < 0 || exceedsSlop(event, tap)) pendingTap = null }
                return pendingTap != null
            }
            MotionEvent.ACTION_CANCEL, MotionEvent.ACTION_POINTER_DOWN, MotionEvent.ACTION_POINTER_UP -> { pendingTap = null; return false }
            MotionEvent.ACTION_UP -> {
                val tap = pendingTap
                pendingTap = null
                if (tap != null && event.pointerCount == 1 && event.getPointerId(event.actionIndex) == tap.pointerId && !exceedsSlop(event, tap) && targetAt() == tap.target) {
                    return onInteractionActivated?.invoke(tap.target) ?: false
                }
            }
        }
        return false
    }

    private fun exceedsSlop(event: MotionEvent, tap: PendingTap): Boolean {
        val dx = event.x - tap.downX
        val dy = event.y - tap.downY
        return dx * dx + dy * dy > touchSlop * touchSlop
    }

    private data class PendingTap(val target: PreparedProseInteraction, val pointerId: Int, val downX: Float, val downY: Float)

    override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
        super.onInitializeAccessibilityNodeInfo(info)
        info.className = android.widget.TextView::class.java.name
        nodes().indices.forEach { info.addChild(this, it + 1) }
    }

    override fun getAccessibilityNodeProvider(): AccessibilityNodeProvider = provider

    // The replacement constructors are API 30 and this module's minSdk is 24;
    // on API 30+ obtain() only delegates to them. setBoundsInParent has no
    // replacement at all, and API 24-28 services still read it.
    @Suppress("DEPRECATION")
    private val provider = object : AccessibilityNodeProvider() {
        override fun createAccessibilityNodeInfo(id: Int): AccessibilityNodeInfo? {
            if (id == View.NO_ID) return AccessibilityNodeInfo.obtain(this@PreparedProseDrawingView).also(::onInitializeAccessibilityNodeInfo)
            val node = nodes().getOrNull(id - 1) ?: return null
            val parentBounds = Rect(node.bounds).apply { offset(contentOriginXPx, contentOriginYPx) }
            val screen = accessibilityScreenBounds(node)
            val visibleToUser = accessibilityNodeVisibleOnScreen(screen)
            reconcileVirtualAccessibilityFocus()
            val identity = identity(node)
            return AccessibilityNodeInfo.obtain().apply {
                packageName = context.packageName
                className = android.widget.Button::class.java.name
                setSource(this@PreparedProseDrawingView, id)
                setParent(this@PreparedProseDrawingView)
                text = node.label
                contentDescription = node.label
                isClickable = true
                isFocusable = true
                AndroidApiCompat.setScreenReaderFocusable(this, true)
                isAccessibilityFocused = focusedVirtualNode?.identity == identity
                isVisibleToUser = visibleToUser
                setBoundsInParent(parentBounds)
                setBoundsInScreen(screen)
                addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_CLICK)
                addAction(if (isAccessibilityFocused) AccessibilityNodeInfo.AccessibilityAction.ACTION_CLEAR_ACCESSIBILITY_FOCUS else AccessibilityNodeInfo.AccessibilityAction.ACTION_ACCESSIBILITY_FOCUS)
                AccessibilityNodeInfoCompat.wrap(this).roleDescription = if (node.role == PreparedProseAccessibilityNode.Role.LINK) "link" else "mention"
            }
        }

        override fun performAction(id: Int, action: Int, arguments: Bundle?): Boolean {
            val node = nodes().getOrNull(id - 1) ?: return false
            return when (action) {
                AccessibilityNodeInfo.ACTION_CLICK -> if (accessibilityNodeVisible(node)) {
                    preparedLayout?.interactions?.getOrNull(node.interactionIndex)?.let {
                        onInteractionActivated?.invoke(it) ?: false
                    } ?: false
                } else {
                    false
                }
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS -> requestVirtualAccessibilityFocus(id)
                AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS -> clearVirtualAccessibilityFocus(id)
                else -> false
            }
        }
    }

    private fun nodes(): List<PreparedProseAccessibilityNode> =
        preparedLayout?.accessibilityNodes.orEmpty().filter { node ->
            when (node.role) {
                PreparedProseAccessibilityNode.Role.LINK -> linkInteractionsEnabled
                PreparedProseAccessibilityNode.Role.MENTION -> mentionInteractionsEnabled
            }
        }

    private fun interactionEnabled(kind: PreparedProseInteraction.Kind): Boolean = when (kind) {
        PreparedProseInteraction.Kind.LINK -> linkInteractionsEnabled
        PreparedProseInteraction.Kind.MENTION -> mentionInteractionsEnabled
    }

    private fun requestVirtualAccessibilityFocus(id: Int): Boolean {
        val node = nodes().getOrNull(id - 1) ?: return false
        if (!accessibilityNodeVisible(node)) return false
        val identity = identity(node)
        if (focusedVirtualNode?.identity == identity) return false
        clearVirtualAccessibilityFocus()
        focusedVirtualNode = FocusedVirtualNode(id, identity)
        invalidate()
        sendVirtualAccessibilityEvent(id, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED)
        return true
    }

    private fun clearVirtualAccessibilityFocus(
        id: Int = focusedVirtualNode?.virtualId ?: View.NO_ID,
    ): Boolean {
        val focused = focusedVirtualNode ?: return false
        if (id == View.NO_ID || id != focused.virtualId) return false
        focusedVirtualNode = null
        invalidate()
        sendVirtualAccessibilityEvent(id, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED)
        return true
    }

    private fun reconcileVirtualAccessibilityFocus() {
        val focused = focusedVirtualNode ?: return
        val nodes = nodes()
        val index = nodes.indexOfFirst { identity(it) == focused.identity }
        if (index < 0 || index + 1 != focused.virtualId) {
            clearVirtualAccessibilityFocus(focused.virtualId)
            return
        }
        if (!accessibilityNodeVisible(nodes[index])) {
            clearVirtualAccessibilityFocus(focused.virtualId)
        }
    }

    private fun accessibilityNodeVisible(node: PreparedProseAccessibilityNode): Boolean =
        accessibilityScreenBounds(node).let { bounds ->
            accessibilityVisibilityForTesting?.invoke(bounds)
                ?: accessibilityNodeVisibleOnScreen(bounds)
        }

    private fun accessibilityScreenBounds(node: PreparedProseAccessibilityNode): Rect {
        val bounds = Rect(node.bounds).apply { offset(contentOriginXPx, contentOriginYPx) }
        val location = IntArray(2)
        getLocationOnScreen(location)
        bounds.offset(location[0], location[1])
        return bounds
    }

    private fun identity(node: PreparedProseAccessibilityNode) = AccessibilityNodeIdentity(
        preparedLayout?.key?.generationIdentity,
        node.interactionIndex,
        node.role,
        node.label,
    )

    // AccessibilityEvent(Int) is API 30; see the node provider above.
    @Suppress("DEPRECATION")
    private fun sendVirtualAccessibilityEvent(id: Int, type: Int) {
        if (!accessibilityManager.isEnabled) return
        val event = AccessibilityEvent.obtain(type).apply {
            packageName = context.packageName
            className = android.widget.Button::class.java.name
            setSource(this@PreparedProseDrawingView, id)
        }
        parent?.requestSendAccessibilityEvent(this, event)
    }

    /** Publishes a logical prepared-subtree transition without changing its artifact. */
    @Suppress("DEPRECATION")
    internal fun announceAccessibilitySubtreeChanged() {
        if (!publishesAccessibilitySubtree || !accessibilityManager.isEnabled) return
        val event = AccessibilityEvent.obtain(AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED).apply {
            packageName = context.packageName
            className = android.widget.TextView::class.java.name
            contentChangeTypes = AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE
            setSource(this@PreparedProseDrawingView)
        }
        parent?.requestSendAccessibilityEvent(this, event)
    }

    private data class AccessibilityNodeIdentity(
        val generation: String?,
        val interactionIndex: Int,
        val role: PreparedProseAccessibilityNode.Role,
        val label: String,
    )

    private data class FocusedVirtualNode(
        val virtualId: Int,
        val identity: AccessibilityNodeIdentity,
    )
}

internal fun View.accessibilityNodeVisibleOnScreen(screenBounds: Rect): Boolean {
    val visibleBounds = Rect()
    val hasGlobalVisibleBounds = getGlobalVisibleRect(visibleBounds)
    return accessibilityBoundsVisible(
        screenBounds,
        visibleBounds.takeIf { hasGlobalVisibleBounds },
        isShown,
        windowVisibility == View.VISIBLE,
        hasVisibleAlpha(),
    )
}

internal fun accessibilityBoundsVisible(
    screenBounds: Rect,
    globalVisibleBounds: Rect?,
    shown: Boolean,
    windowVisible: Boolean,
    alphaVisible: Boolean,
): Boolean {
    if (!shown || !windowVisible || !alphaVisible || globalVisibleBounds?.isEmpty != false) {
        return false
    }
    return Rect(globalVisibleBounds).run {
        intersect(screenBounds) && !isEmpty
    }
}

private fun View.hasVisibleAlpha(): Boolean {
    var current: View? = this
    while (current != null) {
        if (current.alpha <= 0f) return false
        current = current.parent as? View
    }
    return true
}
