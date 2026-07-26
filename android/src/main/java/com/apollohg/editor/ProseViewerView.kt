package com.apollohg.editor

import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.graphics.Color
import android.graphics.Rect
import android.net.Uri
import android.os.Bundle
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewGroup
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeProvider
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import com.apollohg.editor.viewer.PreparedProseDrawingView
import com.apollohg.editor.viewer.PreparedProseInteraction
import com.apollohg.editor.viewer.PreparedProseLayout
import com.apollohg.editor.viewer.PreparedProseLayoutRegistry
import com.apollohg.editor.viewer.ProseViewerRequest
import com.apollohg.editor.viewer.ViewerDocument
import com.apollohg.editor.viewer.ViewerAttachmentRevisionState
import com.apollohg.editor.viewer.ViewerFontEnvironment
import com.apollohg.editor.viewer.ViewerImageAttachment
import com.apollohg.editor.viewer.ViewerImagePipeline
import kotlin.math.abs
import kotlin.math.ceil
import org.json.JSONArray

sealed interface ProseViewerSource {
    val value: String
    val kind: String

    data class Json(override val value: String) : ProseViewerSource {
        override val kind: String = "json"
    }

    data class Html(override val value: String) : ProseViewerSource {
        override val kind: String = "html"
    }
}

data class ProseViewerConfiguration(
    val configJson: String,
    val themeJson: String? = null,
    val imagePolicyJson: String? = null,
    val imagesEnabled: Boolean = true,
    val collapsesWhenEmpty: Boolean = false,
)

@JvmInline
value class ProseViewerErrorCode(val value: String) {
    companion object {
        val INVALID_WIDTH = ProseViewerErrorCode("INVALID_WIDTH")
        val LAYOUT_FAILED = ProseViewerErrorCode("LAYOUT_FAILED")
        val RESOURCE_LOAD_FAILED = ProseViewerErrorCode("RESOURCE_LOAD_FAILED")
    }
}

data class ProseViewerError(
    val domain: String,
    val code: ProseViewerErrorCode,
    override val message: String,
) : RuntimeException(message) {
    companion object {
        fun compiler(domain: String, code: String, message: String) =
            ProseViewerError(domain, ProseViewerErrorCode(code), message)

        fun invalidWidth() = ProseViewerError(
            "viewer.host",
            ProseViewerErrorCode.INVALID_WIDTH,
            "A finite positive width is required for prose measurement.",
        )

        fun layout(message: String) =
            ProseViewerError("viewer.layout", ProseViewerErrorCode.LAYOUT_FAILED, message)

        fun resource() = ProseViewerError(
            "viewer.resource",
            ProseViewerErrorCode.RESOURCE_LOAD_FAILED,
            "An image resource could not be loaded.",
        )
    }
}

/** Interaction callbacks for an embedded Android prose viewer. */
interface ProseViewerInteractionListener {
    fun onLinkTap(view: ProseViewerView, href: String, text: String)
    fun onMentionTap(view: ProseViewerView, docPos: Long, label: String)
    fun onViewerError(view: ProseViewerView, error: ProseViewerError) = Unit
}

abstract class ProseViewerInteractionListenerAdapter : ProseViewerInteractionListener {
    override fun onLinkTap(view: ProseViewerView, href: String, text: String) = Unit
    override fun onMentionTap(view: ProseViewerView, docPos: Long, label: String) = Unit
}

private sealed interface ProseViewerTapTarget {
    val annotation: android.text.Annotation
    val start: Int
    val end: Int

    data class Mention(
        /** A Rust u32 retained in a signed [Long] without narrowing. */
        val docPos: Long,
        val label: String,
        override val annotation: android.text.Annotation,
        override val start: Int,
        override val end: Int
    ) : ProseViewerTapTarget

    data class Link(
        val href: String,
        val text: String,
        override val annotation: android.text.Annotation,
        override val start: Int,
        override val end: Int
    ) : ProseViewerTapTarget
}

private fun ProseViewerTapTarget.matches(other: ProseViewerTapTarget?): Boolean =
    other != null &&
        annotation === other.annotation &&
        start == other.start &&
        end == other.end &&
        this::class == other::class

private data class PendingProseViewerTap(
    val target: ProseViewerTapTarget,
    val pointerId: Int,
    val downX: Float,
    val downY: Float
)


/**
 * Display-only prose viewer for Android View hosts.
 *
 * Input is the flat render-ops JSON array produced by the package render
 * bridge. This view does not create or retain an editor handle.
 */
class ProseViewerView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0
) : ViewGroup(context, attrs, defStyleAttr) {
    internal constructor(context: Context, registry: PreparedProseLayoutRegistry) : this(context) {
        layoutRegistry = registry
    }

    var interactionListener: ProseViewerInteractionListener? = null

    private var layoutRegistry = PreparedProseLayoutRegistry.shared
    private val preparedDrawingView = PreparedProseDrawingView(context)
    private var preparedRequest: ProseViewerRequest? = null
    private var retainedDocument: ViewerDocument? = null
    private var preparedArtifact: PreparedProseLayout? = null
    private var directError: ProseViewerError? = null
    private var reportedGenerationIdentity: String? = null
    private val reportedResourceFailures = mutableSetOf<String>()
    private val attachmentRevisions = ViewerAttachmentRevisionState()
    private val fontEnvironment = ViewerFontEnvironment()
    private val viewerImagePipeline = ViewerImagePipeline()

    private val proseView = EditorEditText(context)
    private var lastRenderJson = "[]"
    private var lastThemeJson: String? = null
    private var collapsesWhenEmpty = false
    private var isCollapsedEmptyContent = false
    private var touchSlop = ViewConfiguration.get(context).scaledTouchSlop.toFloat()
    private var pendingTapGesture: PendingProseViewerTap? = null
    private var accessibilityFocusedVirtualId = View.NO_ID
    private var preparedAccessibilityGeneration: String? = null

    internal var onContentHeightChange: ((Int) -> Unit)? = null
    internal var opensLinksAutomatically = false
    internal var linkTapsEnabled = true
        set(value) {
            if (field == value) return
            clearVirtualAccessibilityFocus()
            field = value
            preparedDrawingView.linkInteractionsEnabled = value
            notifyAccessibilitySubtreeChanged()
        }
    internal val isContentCollapsedForHost: Boolean
        get() = isCollapsedEmptyContent
    internal val renderedTextForTesting: String
        get() = proseView.text?.toString()?.replace(EMPTY_TEXT_BLOCK_PLACEHOLDER.toString(), "")
            ?: ""
    internal val proseViewForTesting: EditorEditText
        get() = proseView
    internal var touchSlopForTesting: Float
        get() = touchSlop
        set(value) {
            touchSlop = value
        }
    internal var onLinkTapForTesting: (() -> Unit)? = null
    internal var onMentionTapForTesting: (() -> Unit)? = null
    internal val preparedLayoutForTesting: PreparedProseLayout?
        get() = preparedArtifact

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
        proseView.setOnTouchListener { _, event -> handleProseTouch(event) }

        preparedDrawingView.visibility = View.GONE
        // The public facade owns virtual accessibility. The drawing child is
        // still interactive, but must not expose a duplicate virtual subtree.
        preparedDrawingView.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        preparedDrawingView.publishesAccessibilitySubtree = false
        preparedDrawingView.linkInteractionsEnabled = linkTapsEnabled
        preparedDrawingView.onInteractionActivated = { activatePreparedInteraction(it) }
        viewerImagePipeline.onPixels = { attachment, bitmap ->
            val current = preparedRequest
            if (current != null && viewerImagePipeline.acceptsCompletion(current.generationIdentity)) {
                preparedDrawingView.imagePixels = preparedDrawingView.imagePixels + (attachment.id to bitmap)
            }
        }
        viewerImagePipeline.onIntrinsicMetadata = { attachment, width, height ->
            applyIntrinsicImageMetadata(attachment, width, height)
        }
        viewerImagePipeline.onResourceFailure = { attachment -> reportResourceFailureIfNeeded(attachment) }
        fontEnvironment.onInvalidated = { revision -> applyFontEnvironmentRevision(revision) }
        addView(
            preparedDrawingView,
            LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT)
        )

        addView(
            proseView,
            LayoutParams(
                LayoutParams.MATCH_PARENT,
                LayoutParams.WRAP_CONTENT
            )
        )
    }

    /**
     * Starts an immutable direct-content generation. Compilation is retained through the first
     * finite measurement even when the registry evicts its unmounted cache entry.
     */
    fun apply(source: ProseViewerSource, configuration: ProseViewerConfiguration): Boolean {
        val next = ProseViewerRequest(
            source,
            configuration,
            fontEnvironmentRevision = fontEnvironment.revision,
            attachmentRevision = attachmentRevisions.revision,
        )
        if (preparedRequest == next) return directError == null
        preparedRequest = next
        retainedDocument = null
        preparedArtifact = null
        viewerImagePipeline.cancel()
        preparedDrawingView.imagePixels = emptyMap()
        directError = null
        reportedGenerationIdentity = null
        reportedResourceFailures.clear()
        clearVirtualAccessibilityFocus()
        preparedAccessibilityGeneration = null
        // The replacement artifact owns the observable subtree transition.
        preparedDrawingView.install(null, announceAccessibilitySubtree = false)
        preparedDrawingView.visibility = View.VISIBLE
        proseView.visibility = View.GONE
        return try {
            retainedDocument = layoutRegistry.compileDocument(next)
            requestLayout()
            true
        } catch (error: ProseViewerError) {
            directError = error
            requestLayout()
            false
        }
    }

    /**
     * Applies render-ops and theme JSON. Invalid render input clears the view.
     */
    fun apply(renderJson: String, themeJson: String): Boolean {
        clearDirectGeneration()
        val accepted = isRenderOpsArray(renderJson)
        val normalizedRenderJson = if (accepted) renderJson else "[]"
        if (normalizedRenderJson == lastRenderJson && themeJson == lastThemeJson) {
            return accepted
        }

        clearVirtualAccessibilityFocus()
        pendingTapGesture = null
        lastRenderJson = normalizedRenderJson
        if (lastThemeJson != themeJson) {
            lastThemeJson = themeJson
            proseView.applyTheme(EditorTheme.fromJson(themeJson))
        }
        renderCurrentContent()
        notifyAccessibilitySubtreeChanged()
        return accepted
    }

    /** Updates the bounded image-loading policy from its serialized form. */
    fun setImageLoadingPolicyJson(policyJson: String?) {
        val previousPolicy = proseView.imageLoadingPolicy
        proseView.setImageLoadingPolicyJson(policyJson)
        if (previousPolicy != proseView.imageLoadingPolicy) {
            requestLayout()
        }
    }

    /**
     * Clears content and pending work for a recycled host view.
     *
     * The interaction listener is retained so holders may assign it once.
     */
    fun prepareForReuse() {
        clearDirectGeneration()
        clearVirtualAccessibilityFocus()
        pendingTapGesture = null
        lastRenderJson = "[]"
        lastThemeJson = null
        collapsesWhenEmpty = false
        isCollapsedEmptyContent = false
        proseView.applyTheme(null)
        proseView.applyRenderJSON("[]")
        proseView.setImageLoadingPolicyJson(null)
        proseView.visibility = View.VISIBLE
        lastReportedContentHeight = 0
        requestLayout()
        notifyAccessibilitySubtreeChanged()
    }

    /** Returns the current content height for an Android width in pixels. */
    fun measuredHeightForWidth(widthPx: Int): Int {
        if (isCollapsedEmptyContent || widthPx <= 0) return 0
        val childWidthSpec = MeasureSpec.makeMeasureSpec(widthPx, MeasureSpec.EXACTLY)
        val childHeightSpec = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED)
        proseView.measure(childWidthSpec, childHeightSpec)
        return proseView.resolveAutoGrowHeight()
    }

    internal fun setCollapsesWhenEmpty(collapses: Boolean) {
        if (collapsesWhenEmpty == collapses) return
        collapsesWhenEmpty = collapses
        updateCollapsedEmptyState()
        requestLayout()
        emitContentHeightIfNeeded(force = true)
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        preparedRequest?.let { request ->
            val widthMode = MeasureSpec.getMode(widthMeasureSpec)
            val availableWidth = (MeasureSpec.getSize(widthMeasureSpec) - paddingLeft - paddingRight)
            val artifact = layoutRegistry.measure(
                request = request,
                widthPx = if (widthMode == MeasureSpec.UNSPECIFIED) 0 else availableWidth,
                density = resources.displayMetrics.density,
                compiledDocument = retainedDocument,
                fontScale = resources.configuration.fontScale,
            )
            val artifactChanged = preparedArtifact !== artifact
            preparedArtifact = artifact
            preparedDrawingView.install(artifact)
            if (artifactChanged || preparedAccessibilityGeneration != artifact.key.generationIdentity) {
                clearVirtualAccessibilityFocus()
                preparedAccessibilityGeneration = artifact.key.generationIdentity
                notifyAccessibilitySubtreeChanged()
            }
            reportDirectErrorIfNeeded(request, artifact.error ?: directError)
            val desiredWidth = artifact.widthPx + paddingLeft + paddingRight
            val intrinsicHeight = artifact.heightPx + paddingTop + paddingBottom
            val measuredWidth = resolveSize(desiredWidth, widthMeasureSpec)
            val measuredHeight = when (MeasureSpec.getMode(heightMeasureSpec)) {
                MeasureSpec.EXACTLY -> MeasureSpec.getSize(heightMeasureSpec)
                MeasureSpec.AT_MOST -> intrinsicHeight.coerceAtMost(MeasureSpec.getSize(heightMeasureSpec))
                else -> intrinsicHeight
            }
            setMeasuredDimension(measuredWidth, measuredHeight)
            return
        }
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
        val childHeightSpec = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED)
        proseView.measure(childWidthSpec, childHeightSpec)

        val resolvedContentHeight = proseView.resolveAutoGrowHeight()
        val desiredWidth = proseView.measuredWidth + paddingLeft + paddingRight
        val desiredHeight = resolvedContentHeight + paddingTop + paddingBottom
        val measuredHeight = when (MeasureSpec.getMode(heightMeasureSpec)) {
            MeasureSpec.AT_MOST -> desiredHeight.coerceAtMost(
                MeasureSpec.getSize(heightMeasureSpec)
            )
            else -> desiredHeight
        }
        setMeasuredDimension(resolveSize(desiredWidth, widthMeasureSpec), measuredHeight)
        emitContentHeightIfNeeded(measuredContentHeight = desiredHeight)
    }

    override fun onLayout(
        changed: Boolean,
        left: Int,
        top: Int,
        right: Int,
        bottom: Int
    ) {
        if (preparedRequest != null) {
            preparedDrawingView.layout(
                paddingLeft,
                paddingTop,
                (right - left - paddingRight).coerceAtLeast(paddingLeft),
                (bottom - top - paddingBottom).coerceAtLeast(paddingTop),
            )
            requestVisibleImageAttachments()
            return
        }
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

    override fun onDetachedFromWindow() {
        preparePreparedHostForWindowDetachment()
        pendingTapGesture = null
        super.onDetachedFromWindow()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        requestVisibleImageAttachments()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        fontEnvironment.onConfigurationChanged(newConfig)
    }

    /** Explicit hook for React Native font loaders. */
    fun invalidateFontEnvironment() = fontEnvironment.invalidateRegisteredFonts()

    /**
     * Removes a directly owned prepared subtree. Virtual focus must clear
     * before the artifact disappears and before its subtree notification.
     */
    internal fun preparePreparedHostForWindowDetachment() {
        clearVirtualAccessibilityFocus()
        if (preparedRequest != null) {
            preparedArtifact = null
            preparedDrawingView.install(null)
            viewerImagePipeline.cancel()
            preparedDrawingView.imagePixels = emptyMap()
            preparedAccessibilityGeneration = null
            notifyAccessibilitySubtreeChanged()
        }
    }

    private fun reportDirectErrorIfNeeded(request: ProseViewerRequest, error: ProseViewerError?) {
        if (error == null || reportedGenerationIdentity == request.generationIdentity) return
        reportedGenerationIdentity = request.generationIdentity
        interactionListener?.onViewerError(this, error)
    }

    private fun reportResourceFailureIfNeeded(attachment: ViewerImageAttachment) {
        val generation = preparedRequest?.generationIdentity ?: return
        if (!reportedResourceFailures.add("$generation\u001f${attachment.id}")) return
        interactionListener?.onViewerError(this, ProseViewerError.resource())
    }

    private fun clearDirectGeneration() {
        clearVirtualAccessibilityFocus()
        preparedRequest = null
        retainedDocument = null
        preparedArtifact = null
        viewerImagePipeline.cancel()
        preparedDrawingView.imagePixels = emptyMap()
        directError = null
        reportedGenerationIdentity = null
        reportedResourceFailures.clear()
        // apply(renderJson, themeJson) publishes the replacement subtree.
        preparedDrawingView.install(null, announceAccessibilitySubtree = false)
        preparedDrawingView.visibility = View.GONE
        preparedAccessibilityGeneration = null
        proseView.visibility = View.VISIBLE
    }

    private fun renderCurrentContent() {
        updateCollapsedEmptyState()
        proseView.applyRenderJSON(lastRenderJson)
        proseView.visibility = if (isCollapsedEmptyContent) View.GONE else View.VISIBLE
        requestLayout()
    }

    private fun configureImageGeneration(artifact: PreparedProseLayout) {
        val request = preparedRequest ?: return
        viewerImagePipeline.begin(
            request.generationIdentity,
            request.configuration.imagesEnabled,
            ImageLoadingPolicy.fromJson(request.configuration.imagePolicyJson),
        )
    }

    private fun requestVisibleImageAttachments() {
        val artifact = preparedArtifact ?: return
        if (!isAttachedToWindow || preparedDrawingView.visibility != View.VISIBLE || !preparedDrawingView.isShown) return
        val visible = Rect()
        if (!preparedDrawingView.getGlobalVisibleRect(visible) || visible.isEmpty) return
        val location = IntArray(2)
        preparedDrawingView.getLocationOnScreen(location)
        visible.offset(-location[0], -location[1])
        if (!visible.intersect(Rect(0, 0, preparedDrawingView.width, preparedDrawingView.height)) || visible.isEmpty) return
        configureImageGeneration(artifact)
        viewerImagePipeline.updateVisibleRect(
            visible,
            artifact.imageAttachments,
        )
    }

    private fun applyIntrinsicImageMetadata(attachment: ViewerImageAttachment, width: Int, height: Int) {
        val request = preparedRequest ?: return
        if (!viewerImagePipeline.acceptsCompletion(request.generationIdentity)) return
        if (!attachmentRevisions.recordIntrinsicSize(attachment.id, width, height, attachment.declaredSize)) return
        preparedRequest = request.copy(attachmentRevision = attachmentRevisions.revision)
        reportedResourceFailures.clear()
        requestLayout()
    }

    private fun applyFontEnvironmentRevision(revision: Long) {
        val request = preparedRequest ?: return
        if (revision <= request.fontEnvironmentRevision) return
        preparedRequest = request.copy(fontEnvironmentRevision = revision)
        reportedResourceFailures.clear()
        requestLayout()
    }

    private fun updateCollapsedEmptyState() {
        isCollapsedEmptyContent = collapsesWhenEmpty &&
            renderJsonContainsOnlyEmptyParagraphs(lastRenderJson)
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
                measuredContentHeight ?: (
                    measureContentHeightPx() + paddingTop + paddingBottom
                    )
                ).coerceAtLeast(0)
        }
        if (contentHeight <= 0 && !isCollapsedEmptyContent) return
        if (!force && contentHeight == lastReportedContentHeight) return
        lastReportedContentHeight = contentHeight
        onContentHeightChange?.invoke(contentHeight)
    }

    private var lastReportedContentHeight = 0

    private fun measureContentHeightPx(): Int {
        if (isCollapsedEmptyContent) return 0

        val availableWidthPx = resolveAvailableWidthPx()
        if (
            proseView.measuredWidth <= 0 ||
            abs(proseView.measuredWidth - availableWidthPx) > 1
        ) {
            val childWidthSpec = MeasureSpec.makeMeasureSpec(
                availableWidthPx,
                MeasureSpec.EXACTLY
            )
            val childHeightSpec = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED)
            proseView.measure(childWidthSpec, childHeightSpec)
        }
        return proseView.resolveAutoGrowHeight()
    }

    private fun resolveAvailableWidthPx(): Int {
        val localWidth = width - paddingLeft - paddingRight
        if (localWidth > 0) return localWidth

        val parentWidth = ((parent as? View)?.width ?: 0) - paddingLeft - paddingRight
        if (parentWidth > 0) return parentWidth

        return (resources.displayMetrics.widthPixels - paddingLeft - paddingRight)
            .coerceAtLeast(1)
    }

    private fun handleProseTouch(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                pendingTapGesture = if (event.pointerCount == 1) {
                    tapTargetAt(event.x, event.y)?.let { target ->
                        PendingProseViewerTap(
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
                    (
                        event.pointerCount != 1 ||
                            event.findPointerIndex(gesture.pointerId) < 0 ||
                            movedBeyondTouchSlop(event, gesture)
                        )
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

    private fun activatePreparedInteraction(interaction: PreparedProseInteraction): Boolean = when (interaction.kind) {
        PreparedProseInteraction.Kind.LINK -> {
            val href = interaction.href ?: return false
            if (!linkTapsEnabled) false else if (opensLinksAutomatically) openLink(href) else {
                onLinkTapForTesting?.invoke() ?: interactionListener?.onLinkTap(this, href, interaction.visibleText)
                true
            }
        }
        PreparedProseInteraction.Kind.MENTION -> {
            val docPos = interaction.docPos ?: return false
            onMentionTapForTesting?.invoke() ?: interactionListener?.onMentionTap(this, docPos, interaction.label)
            true
        }
    }

    private fun movedBeyondTouchSlop(
        event: MotionEvent,
        gesture: PendingProseViewerTap
    ): Boolean {
        val deltaX = event.x - gesture.downX
        val deltaY = event.y - gesture.downY
        return deltaX * deltaX + deltaY * deltaY > touchSlop * touchSlop
    }

    private fun tapTargetAt(x: Float, y: Float): ProseViewerTapTarget? {
        val hit = proseView.interactiveAnnotationHitAt(x, y) ?: return null
        return when (val target = hit.target) {
            is EditorEditText.AccessibleAnnotationTarget.Mention ->
                ProseViewerTapTarget.Mention(
                    target.docPos,
                    target.label,
                    hit.annotation,
                    hit.start,
                    hit.end
                )
            is EditorEditText.AccessibleAnnotationTarget.Link -> {
                if (!linkTapsEnabled) return null
                ProseViewerTapTarget.Link(
                    target.href,
                    target.text,
                    hit.annotation,
                    hit.start,
                    hit.end
                )
            }
        }
    }

    private fun activateTapTarget(target: ProseViewerTapTarget): Boolean {
        return when (target) {
            is ProseViewerTapTarget.Mention -> {
                onMentionTapForTesting?.invoke()
                    ?: interactionListener?.onMentionTap(
                        this,
                        target.docPos,
                        target.label
                    )
                true
            }
            is ProseViewerTapTarget.Link -> {
                onLinkTapForTesting?.let {
                    it()
                    return true
                }
                if (opensLinksAutomatically) {
                    openLink(target.href)
                } else {
                    interactionListener?.onLinkTap(this, target.href, target.text)
                    true
                }
            }
        }
    }

    override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
        super.onInitializeAccessibilityNodeInfo(info)
        info.className = android.widget.TextView::class.java.name
        info.text = if (preparedRequest != null) preparedAccessibleNodes().joinToString(" ") { it.label } else renderedTextForTesting
        val count = if (preparedRequest != null) preparedAccessibleNodes().size else accessibleAnnotations().size
        repeat(count) { index ->
            info.addChild(this, index + FIRST_VIRTUAL_ANNOTATION_ID)
        }
    }

    override fun getAccessibilityNodeProvider(): AccessibilityNodeProvider =
        annotationNodeProvider

    private val annotationNodeProvider = object : AccessibilityNodeProvider() {
        override fun createAccessibilityNodeInfo(virtualViewId: Int): AccessibilityNodeInfo? {
            if (virtualViewId == View.NO_ID) {
                return AccessibilityNodeInfo.obtain(this@ProseViewerView).also {
                    onInitializeAccessibilityNodeInfo(it)
                }
            }
            if (preparedRequest != null) return preparedAccessibilityNodeInfo(virtualViewId)
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
                setSource(this@ProseViewerView, virtualViewId)
                setParent(this@ProseViewerView)
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
            if (preparedRequest != null) return performPreparedAccessibilityAction(virtualViewId, action)
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

    private fun preparedAccessibleNodes() = preparedArtifact?.accessibilityNodes.orEmpty().filter {
        linkTapsEnabled || it.role != com.apollohg.editor.viewer.PreparedProseAccessibilityNode.Role.LINK
    }

    private fun preparedAccessibilityNodeInfo(virtualViewId: Int): AccessibilityNodeInfo? {
        val node = preparedAccessibleNodes().getOrNull(virtualViewId - FIRST_VIRTUAL_ANNOTATION_ID) ?: return null
        val parentBounds = Rect(node.bounds).apply { offset(preparedDrawingView.left, preparedDrawingView.top) }
        val screenBounds = Rect(parentBounds)
        val location = IntArray(2)
        getLocationOnScreen(location)
        screenBounds.offset(location[0], location[1])
        return AccessibilityNodeInfo.obtain().apply {
            packageName = context.packageName
            className = android.widget.Button::class.java.name
            setSource(this@ProseViewerView, virtualViewId)
            setParent(this@ProseViewerView)
            text = node.label
            contentDescription = node.label
            isClickable = true
            isFocusable = true
            isScreenReaderFocusable = true
            isAccessibilityFocused = virtualViewId == accessibilityFocusedVirtualId
            setBoundsInParent(parentBounds)
            setBoundsInScreen(screenBounds)
            addAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_CLICK)
            addAction(if (isAccessibilityFocused) AccessibilityNodeInfo.AccessibilityAction.ACTION_CLEAR_ACCESSIBILITY_FOCUS else AccessibilityNodeInfo.AccessibilityAction.ACTION_ACCESSIBILITY_FOCUS)
            AccessibilityNodeInfoCompat.wrap(this).roleDescription = if (node.role == com.apollohg.editor.viewer.PreparedProseAccessibilityNode.Role.LINK) "link" else "mention"
        }
    }

    private fun performPreparedAccessibilityAction(virtualViewId: Int, action: Int): Boolean {
        val node = preparedAccessibleNodes().getOrNull(virtualViewId - FIRST_VIRTUAL_ANNOTATION_ID) ?: return false
        return when (action) {
            AccessibilityNodeInfo.ACTION_CLICK -> preparedArtifact?.interactions?.getOrNull(node.interactionIndex)?.let(::activatePreparedInteraction) ?: false
            AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS -> requestVirtualAccessibilityFocus(virtualViewId)
            AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS -> clearVirtualAccessibilityFocus(virtualViewId)
            else -> false
        }
    }

    private fun requestVirtualAccessibilityFocus(virtualViewId: Int): Boolean {
        val exists = if (preparedRequest != null) {
            preparedAccessibleNodes().getOrNull(virtualViewId - FIRST_VIRTUAL_ANNOTATION_ID) != null
        } else {
            accessibleAnnotations().getOrNull(virtualViewId - FIRST_VIRTUAL_ANNOTATION_ID) != null
        }
        if (!exists) {
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
        ) {
            return false
        }
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
            setSource(this@ProseViewerView, virtualViewId)
        }
        parent?.requestSendAccessibilityEvent(this, event)
    }

    private fun notifyAccessibilitySubtreeChanged() {
        val event = AccessibilityEvent.obtain(
            AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED
        ).apply {
            packageName = context.packageName
            className = android.widget.TextView::class.java.name
            contentChangeTypes = AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE
            setSource(this@ProseViewerView)
        }
        parent?.requestSendAccessibilityEvent(this, event)
    }

    private fun accessibleAnnotations(): List<EditorEditText.AccessibleAnnotation> =
        proseView.accessibleAnnotations().filter { annotation ->
            annotation.target !is EditorEditText.AccessibleAnnotationTarget.Link ||
                linkTapsEnabled
        }

    private fun EditorEditText.AccessibleAnnotation.toTapTarget(): ProseViewerTapTarget =
        when (val value = target) {
            is EditorEditText.AccessibleAnnotationTarget.Link ->
                ProseViewerTapTarget.Link(
                    value.href,
                    value.text,
                    annotation,
                    start,
                    end
                )
            is EditorEditText.AccessibleAnnotationTarget.Mention ->
                ProseViewerTapTarget.Mention(
                    value.docPos,
                    value.label,
                    annotation,
                    start,
                    end
                )
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

        /** Measures valid render-ops at an Android width in pixels. */
        @JvmStatic
        fun measureHeight(
            context: Context,
            renderJson: String,
            themeJson: String,
            widthPx: Int
        ): Int? {
            if (!isRenderOpsArray(renderJson)) return null
            if (widthPx <= 0) return 0
            return ceil(
                RenderBridge.measureHeight(
                    json = renderJson,
                    themeJson = themeJson,
                    width = widthPx.toFloat(),
                    density = context.resources.displayMetrics.density
                )
            ).toInt()
        }

        private fun isRenderOpsArray(renderJson: String): Boolean = runCatching {
            val elements = JSONArray(renderJson)
            (0 until elements.length()).all { elements.optJSONObject(it) != null }
        }.getOrDefault(false)

        internal fun renderJsonContainsOnlyEmptyParagraphs(renderJson: String): Boolean {
            val elements = try {
                JSONArray(renderJson)
            } catch (_: Exception) {
                return false
            }

            if (elements.length() == 0) return true

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
                        if (!paragraphIsOpen) return false
                        paragraphIsOpen = false
                    }
                    else -> return false
                }
            }

            return hasParagraph && !paragraphIsOpen
        }
    }
}
