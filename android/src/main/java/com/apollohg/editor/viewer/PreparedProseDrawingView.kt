package com.apollohg.editor.viewer

import android.content.Context
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import android.util.AttributeSet
import android.view.View
import android.view.MotionEvent
import android.view.ViewConfiguration
import android.os.Bundle
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeProvider
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import com.apollohg.editor.AndroidApiCompat

/** Rendering-only consumer of fully prepared StaticLayout and geometry fragments. */
internal class PreparedProseDrawingView @JvmOverloads constructor(context: Context, attrs: AttributeSet? = null) : View(context, attrs) {
    private val accessibilityManager = context.getSystemService(AccessibilityManager::class.java)
    var preparedLayout: PreparedProseLayout? = null
        private set
    var onUsableMetricsChanged: (() -> Unit)? = null
    var onVisibleRectChanged: ((Rect) -> Unit)? = null
    var onFontConfigurationChanged: ((Configuration) -> Unit)? = null
    var onInteractionActivated: ((PreparedProseInteraction) -> Boolean)? = null
    var imagePixels: Map<String, Bitmap> = emptyMap()
        set(value) {
            field = value
            PreparedProseInstrumentation.retained(
                PreparedProseInstrumentation.Owner.IMAGE,
                "drawing-${System.identityHashCode(this)}",
                retainedImagePixelsBytes(value),
            )
            invalidate()
        }
    /**
     * This surface owns only map/entry references. The shared native loader
     * cache is the sole owner charged for a decoded Bitmap allocation.
     */
    internal val retainedImagePixelsBytesForTesting: Long
        get() = retainedImagePixelsBytes(imagePixels)
    /** False when a public host owns this view's virtual subtree and notifications. */
    var publishesAccessibilitySubtree: Boolean = true
    var linkInteractionsEnabled: Boolean = true
        set(value) {
            if (field == value) return
            field = value
            clearVirtualAccessibilityFocus()
            announceAccessibilitySubtreeChanged()
        }
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop.toFloat()
    private var pendingTap: PendingTap? = null
    private var focusedVirtualId = View.NO_ID

    internal companion object {
        const val IMAGE_PIXEL_MAP_RETAINED_BYTES = 48L
        const val IMAGE_PIXEL_ENTRY_RETAINED_BYTES = 48L

        fun retainedImagePixelsBytes(pixels: Map<String, Bitmap>): Long {
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

    /**
     * Publishes a prepared artifact. Replacement owners suppress the transient
     * clear announcement and let the final install report the one logical
     * subtree transition; focus-cleared events remain immediate.
     */
    fun install(
        layout: PreparedProseLayout?,
        announceAccessibilitySubtree: Boolean = true,
    ) {
        if (preparedLayout === layout) return
        preparedLayout = layout
        clearVirtualAccessibilityFocus()
        if (announceAccessibilitySubtree) announceAccessibilitySubtreeChanged()
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val artifact = preparedLayout ?: return
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
    }

    private fun drawBackground(canvas: Canvas, fragment: PreparedProseFragment) {
        if (fragment.kind != PreparedProseFragmentKind.BACKGROUND && fragment.kind != PreparedProseFragmentKind.ATOM && fragment.kind != PreparedProseFragmentKind.IMAGE) return
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
                    canvas.restoreToCount(saved)
                } ?: if (fragment.kind == PreparedProseFragmentKind.MARKER) drawTaskMarker(canvas, fragment) else Unit
            }
            PreparedProseFragmentKind.ATOM -> fragment.labelLayout?.let { layout ->
                val saved = canvas.save()
                canvas.translate(fragment.labelX.toFloat(), fragment.labelY.toFloat())
                layout.draw(canvas)
                canvas.restoreToCount(saved)
            }
            PreparedProseFragmentKind.STRIKE -> {
                paint.style = Paint.Style.FILL
                paint.color = fragment.color ?: return
                canvas.drawRect(fragment.bounds, paint)
            }
            PreparedProseFragmentKind.IMAGE -> {
                val attachment = preparedLayout?.imageAttachments?.firstOrNull { it.bounds == fragment.bounds } ?: return
                val bitmap = imagePixels[attachment.id] ?: return
                canvas.drawBitmap(bitmap, null, fragment.bounds, paint)
            }
            else -> Unit
        }
    }

    private fun drawTaskMarker(canvas: Canvas, fragment: PreparedProseFragment) {
        val bounds = RectF(fragment.bounds)
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
        if (width > 0) onUsableMetricsChanged?.invoke()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        if (width > 0) onUsableMetricsChanged?.invoke()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        onFontConfigurationChanged?.invoke(newConfig)
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        fun targetAt(): PreparedProseInteraction? = preparedLayout?.interactions?.firstOrNull { interaction ->
            (linkInteractionsEnabled || interaction.kind != PreparedProseInteraction.Kind.LINK) && interaction.rects.any { it.contains(event.x.toInt(), event.y.toInt()) }
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
            val screen = RectF(node.bounds).let { rect -> android.graphics.Rect(rect.left.toInt(), rect.top.toInt(), rect.right.toInt(), rect.bottom.toInt()) }
            val location = IntArray(2)
            getLocationOnScreen(location)
            screen.offset(location[0], location[1])
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
                isAccessibilityFocused = id == focusedVirtualId
                setBoundsInParent(node.bounds)
                setBoundsInScreen(screen)
                addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_CLICK)
                addAction(if (isAccessibilityFocused) AccessibilityNodeInfo.AccessibilityAction.ACTION_CLEAR_ACCESSIBILITY_FOCUS else AccessibilityNodeInfo.AccessibilityAction.ACTION_ACCESSIBILITY_FOCUS)
                AccessibilityNodeInfoCompat.wrap(this).roleDescription = if (node.role == PreparedProseAccessibilityNode.Role.LINK) "link" else "mention"
            }
        }

        override fun performAction(id: Int, action: Int, arguments: Bundle?): Boolean {
            val node = nodes().getOrNull(id - 1) ?: return false
            return when (action) {
                AccessibilityNodeInfo.ACTION_CLICK -> preparedLayout?.interactions?.getOrNull(node.interactionIndex)?.let { onInteractionActivated?.invoke(it) ?: false } ?: false
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS -> requestVirtualAccessibilityFocus(id)
                AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS -> clearVirtualAccessibilityFocus(id)
                else -> false
            }
        }
    }

    private fun nodes(): List<PreparedProseAccessibilityNode> = preparedLayout?.accessibilityNodes.orEmpty().filter {
        linkInteractionsEnabled || it.role != PreparedProseAccessibilityNode.Role.LINK
    }

    private fun requestVirtualAccessibilityFocus(id: Int): Boolean {
        if (nodes().getOrNull(id - 1) == null || focusedVirtualId == id) return false
        clearVirtualAccessibilityFocus()
        focusedVirtualId = id
        invalidate()
        sendVirtualAccessibilityEvent(id, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED)
        return true
    }

    private fun clearVirtualAccessibilityFocus(id: Int = focusedVirtualId): Boolean {
        if (id == View.NO_ID || id != focusedVirtualId) return false
        focusedVirtualId = View.NO_ID
        invalidate()
        sendVirtualAccessibilityEvent(id, AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED)
        return true
    }

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
}
