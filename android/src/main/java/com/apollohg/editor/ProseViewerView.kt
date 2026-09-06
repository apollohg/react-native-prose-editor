package com.apollohg.editor

import android.content.Context
import android.content.res.Configuration
import android.graphics.Rect
import android.graphics.Typeface
import android.os.Bundle
import android.util.AttributeSet
import android.view.View
import android.view.ViewGroup
import android.view.ViewTreeObserver
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeProvider
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import com.apollohg.editor.viewer.PreparedProseDrawingView
import com.apollohg.editor.viewer.PreparedProseAccessibilityNode
import com.apollohg.editor.viewer.PreparedProseInstrumentation
import com.apollohg.editor.viewer.PreparedProseInteraction
import com.apollohg.editor.viewer.PreparedProseLayout
import com.apollohg.editor.viewer.PreparedProseLayoutRegistry
import com.apollohg.editor.viewer.ProseViewerRequest
import com.apollohg.editor.viewer.ViewerDocument
import com.apollohg.editor.viewer.ViewerAttachmentRevisionState
import com.apollohg.editor.viewer.ViewerFontEnvironment
import com.apollohg.editor.viewer.ViewerImageAttachment
import com.apollohg.editor.viewer.ViewerImagePipeline
import com.apollohg.editor.viewer.accessibilityNodeVisibleOnScreen
import org.json.JSONArray
import org.json.JSONObject

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
    val collapsesWhenEmpty: Boolean = true,
)

@JvmInline
value class ProseViewerErrorCode(val value: String) {
    companion object {
        val INVALID_WIDTH = ProseViewerErrorCode("INVALID_WIDTH")
        val LAYOUT_FAILED = ProseViewerErrorCode("LAYOUT_FAILED")
        val RESOURCE_LOAD_FAILED = ProseViewerErrorCode("RESOURCE_LOAD_FAILED")
        val INVALID_MENTION_ATTRIBUTES = ProseViewerErrorCode("INVALID_MENTION_ATTRIBUTES")
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

/** A mention activated from an embedded prose viewer. */
data class ProseViewerMention(
    val docPos: Long,
    val label: String,
    val attrs: Map<String, Any?>,
)

/** Interaction callbacks for an embedded Android prose viewer. */
interface ProseViewerInteractionListener {
    fun onLinkTap(view: ProseViewerView, href: String, text: String)
    fun onMentionTap(view: ProseViewerView, mention: ProseViewerMention)
    fun onViewerError(view: ProseViewerView, error: ProseViewerError) = Unit
}

abstract class ProseViewerInteractionListenerAdapter : ProseViewerInteractionListener {
    override fun onLinkTap(view: ProseViewerView, href: String, text: String) = Unit
    override fun onMentionTap(view: ProseViewerView, mention: ProseViewerMention) = Unit
}

/**
 * Display-only prose viewer for Android View hosts.
 */
class ProseViewerView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0
) : ViewGroup(context, attrs, defStyleAttr) {
    private val accessibilityManager = context.getSystemService(AccessibilityManager::class.java)
    private val preparedInstrumentationOwner = "direct-${System.identityHashCode(this)}"
    internal constructor(context: Context, registry: PreparedProseLayoutRegistry) : this(context) {
        layoutRegistry = registry
    }

    var interactionListener: ProseViewerInteractionListener? = null
        set(value) {
            if (field === value) return
            clearVirtualAccessibilityFocus()
            field = value
            updatePreparedInteractionCapabilities()
            notifyAccessibilitySubtreeChanged()
        }

    private var layoutRegistry = PreparedProseLayoutRegistry.shared
    private val preparedDrawingView = PreparedProseDrawingView(context)
    private var preparedRequest: ProseViewerRequest? = null
    private var retainedDocument: ViewerDocument? = null
    private var preparedArtifact: PreparedProseLayout? = null
    // Detach drops the direct registration but deliberately retains the
    // immutable artifact for exact, no-recompile reattachment.
    private var directMountedArtifact: PreparedProseLayout? = null
    private var directError: ProseViewerError? = null
    private var reportedGenerationIdentity: String? = null
    private val attachmentRevisions = ViewerAttachmentRevisionState()
    private val fontEnvironment = ViewerFontEnvironment()
    private val viewerImagePipeline = ViewerImagePipeline()

    private var accessibilityFocusedNode: FocusedVirtualNode? = null
    private var preparedAccessibilityGeneration: String? = null
    private val scrollChangedListener = ViewTreeObserver.OnScrollChangedListener {
        reconcileVirtualAccessibilityFocus()
    }

    internal var linkTapsEnabled = true
        set(value) {
            if (field == value) return
            clearVirtualAccessibilityFocus()
            field = value
            updatePreparedInteractionCapabilities()
            notifyAccessibilitySubtreeChanged()
        }
    internal var onLinkTapForTesting: (() -> Unit)? = null
        set(value) {
            if (field === value) return
            clearVirtualAccessibilityFocus()
            field = value
            updatePreparedInteractionCapabilities()
            notifyAccessibilitySubtreeChanged()
        }
    internal var onMentionTapForTesting: (() -> Unit)? = null
        set(value) {
            if (field === value) return
            clearVirtualAccessibilityFocus()
            field = value
            updatePreparedInteractionCapabilities()
            notifyAccessibilitySubtreeChanged()
        }
    internal var accessibilityVisibilityForTesting: ((Rect) -> Boolean)? = null
    internal val preparedLayoutForTesting: PreparedProseLayout?
        get() = preparedArtifact
    /** Mounted host total; the shared layout cache excludes mutable sidecars. */
    internal val preparedSurfaceRetainedBytesForTesting: Long
        get() = saturatedRetainedBytes(
            preparedArtifact?.retainedBytes ?: 0L,
            attachmentRevisions.retainedPublicationBytesForTesting.toLong(),
            preparedDrawingView.retainedImagePixelsBytesForTesting,
        )

    init {
        DecodedBitmapBudget.shared(context)
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES

        // The public facade owns virtual accessibility. The drawing child is
        // still interactive, but must not expose a duplicate virtual subtree.
        preparedDrawingView.importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        preparedDrawingView.publishesAccessibilitySubtree = false
        updatePreparedInteractionCapabilities()
        preparedDrawingView.onCodeHighlightsReady = {
            preparedRequest?.let { preparedRequest = it.copy(nativeFontRevision = it.nativeFontRevision + 1) }
            requestLayout()
        }
        preparedDrawingView.onInteractionActivated = { activatePreparedInteraction(it) }
        viewerImagePipeline.onPixels = { attachment, lease ->
            val current = preparedRequest
            if (current != null && viewerImagePipeline.acceptsCompletion(current.semanticGenerationIdentity)) {
                preparedDrawingView.putImageLease(attachment.id, lease)
            } else {
                lease.close()
            }
        }
        viewerImagePipeline.onPixelsReleased = preparedDrawingView::removeImageLeases
        viewerImagePipeline.onIntrinsicMetadata = { attachment, width, height ->
            applyIntrinsicImageMetadata(attachment, width, height)
        }
        viewerImagePipeline.onResourceFailure = { attachment -> reportResourceFailureIfNeeded(attachment) }
        fontEnvironment.onInvalidated = { revision -> applyFontEnvironmentRevision(revision) }
        addView(
            preparedDrawingView,
            LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT)
        )

    }

    /**
     * Starts an immutable direct-content generation. Compilation is retained through the first
     * finite measurement even when the registry evicts its unmounted cache entry.
     */
    fun apply(source: ProseViewerSource, configuration: ProseViewerConfiguration): Boolean {
        fontEnvironment.activate()
        val currentFontRevision = fontEnvironment.revision
        preparedRequest?.let { current ->
            if (current.source == source && current.configuration == configuration && current.fontEnvironmentRevision == currentFontRevision) {
                return directError == null
            }
            if (current.source == source && current.configuration == configuration) {
                preparedRequest = current.copy(fontEnvironmentRevision = currentFontRevision)
                requestLayout()
                return directError == null
            }
        }
        val next = ProseViewerRequest(
            source,
            configuration,
            fontEnvironmentRevision = currentFontRevision,
            attachmentRevision = 0,
        )
        PreparedProseInstrumentation.invalidated(PreparedProseInstrumentation.InvalidationReason.CONTENT)
        attachmentRevisions.beginSemanticGeneration(next.semanticGenerationIdentity)
        clearVirtualAccessibilityFocus()
        preparedRequest = next
        retainedDocument = null
        preparedArtifact = null
        releaseDirectMountedArtifact()
        viewerImagePipeline.cancel()
        preparedDrawingView.clearImageLeases()
        directError = null
        reportedGenerationIdentity = null
        preparedAccessibilityGeneration = null
        // The replacement artifact owns the observable subtree transition.
        preparedDrawingView.install(null, announceAccessibilitySubtree = false)
        preparedDrawingView.visibility = View.VISIBLE
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
     * Clears content and pending work for a recycled host view.
     *
     * The interaction listener is retained so holders may assign it once.
     */
    fun prepareForReuse() {
        PreparedProseInstrumentation.invalidated(PreparedProseInstrumentation.InvalidationReason.REUSE)
        fontEnvironment.deactivate()
        clearDirectGeneration()
        clearVirtualAccessibilityFocus()
        requestLayout()
        notifyAccessibilitySubtreeChanged()
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
                measurementImageState = attachmentRevisions,
            )
            val artifactChanged = preparedArtifact !== artifact
            val accessibilityChanged =
                artifactChanged || preparedAccessibilityGeneration != artifact.key.generationIdentity
            if (accessibilityChanged) {
                clearVirtualAccessibilityFocus()
            }
            preparedArtifact = artifact
            registerDirectMountedArtifactIfAttached(artifact)
            preparedDrawingView.install(artifact)
            if (accessibilityChanged) {
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
        setMeasuredDimension(resolveSize(0, widthMeasureSpec), 0)
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
            reconcileVirtualAccessibilityFocus()
            requestVisibleImageAttachments()
            return
        }
    }

    override fun dispatchDraw(canvas: android.graphics.Canvas) {
        reconcileVirtualAccessibilityFocus()
        super.dispatchDraw(canvas)
    }

    override fun onDetachedFromWindow() {
        if (viewTreeObserver.isAlive) {
            viewTreeObserver.removeOnScrollChangedListener(scrollChangedListener)
        }
        preparePreparedHostForWindowDetachment()
        fontEnvironment.deactivate()
        super.onDetachedFromWindow()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        viewTreeObserver.addOnScrollChangedListener(scrollChangedListener)
        if (preparedRequest != null) fontEnvironment.activate(deliverPending = true)
        // Detachment only cancels image work. The immutable prepared artifact
        // remains installed, so a direct host can draw/measure immediately on
        // reattach without a semantic replacement or republish.
        if (preparedRequest != null) {
            preparedArtifact?.let(::registerDirectMountedArtifactIfAttached)
            requestLayout()
            preparedDrawingView.invalidate()
        }
        requestVisibleImageAttachments()
        reconcileVirtualAccessibilityFocus()
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
        releaseDirectMountedArtifact()
        if (preparedRequest != null) {
            viewerImagePipeline.cancel()
            // Cancellation/detachment does not create a new semantic source;
            // retain its artifact/publication bits and revision for a later
            // remount. Pixel ownership is released with the drawing map.
            preparedDrawingView.clearImageLeases()
        }
    }

    private fun reportDirectErrorIfNeeded(request: ProseViewerRequest, error: ProseViewerError?) {
        if (error == null || reportedGenerationIdentity == request.semanticGenerationIdentity) return
        reportedGenerationIdentity = request.semanticGenerationIdentity
        interactionListener?.onViewerError(this, error)
    }

    private fun reportResourceFailureIfNeeded(attachment: ViewerImageAttachment) {
        if (!attachmentRevisions.recordResourceFailure(attachment.ordinal)) return
        interactionListener?.onViewerError(this, ProseViewerError.resource())
    }

    private fun clearDirectGeneration() {
        clearVirtualAccessibilityFocus()
        preparedRequest = null
        retainedDocument = null
        preparedArtifact = null
        releaseDirectMountedArtifact()
        viewerImagePipeline.cancel()
        attachmentRevisions.reset()
        preparedDrawingView.clearImageLeases()
        directError = null
        reportedGenerationIdentity = null
        preparedDrawingView.install(null, announceAccessibilitySubtree = false)
        preparedDrawingView.visibility = View.GONE
        preparedAccessibilityGeneration = null
    }

    /**
     * Android can measure a View that never enters a window. Keep that
     * artifact on this View, but do not globally pin it as a direct mount
     * until attachment gives us a deterministic matching release callback.
     */
    private fun registerDirectMountedArtifactIfAttached(artifact: PreparedProseLayout) {
        if (!isAttachedToWindow) return
        if (directMountedArtifact === artifact) return
        releaseDirectMountedArtifact()
        layoutRegistry.registerDirectMounted(preparedInstrumentationOwner, artifact)
        directMountedArtifact = artifact
    }

    private fun releaseDirectMountedArtifact() {
        if (directMountedArtifact == null) return
        layoutRegistry.releaseDirectMounted(preparedInstrumentationOwner)
        directMountedArtifact = null
    }

    private fun configureImageGeneration(artifact: PreparedProseLayout) {
        val request = preparedRequest ?: return
        attachmentRevisions.beginSemanticGeneration(request.semanticGenerationIdentity)
        attachmentRevisions.admit(artifact.imageAttachments.size)
        viewerImagePipeline.begin(
            request.semanticGenerationIdentity,
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
        if (!viewerImagePipeline.acceptsCompletion(request.semanticGenerationIdentity)) return
        if (!attachmentRevisions.recordIntrinsicSize(attachment.id, attachment.ordinal, width, height, attachment.declaredSize)) return
        preparedRequest = request.copy(attachmentRevision = attachmentRevisions.revision)
        PreparedProseInstrumentation.invalidated(PreparedProseInstrumentation.InvalidationReason.ATTACHMENT)
        PreparedProseInstrumentation.retained(
            PreparedProseInstrumentation.Owner.SIDECARS,
            "direct-${System.identityHashCode(this)}",
            attachmentRevisions.retainedPublicationBytesForTesting.toLong(),
        )
        requestLayout()
    }

    private fun applyFontEnvironmentRevision(revision: Long) {
        val request = preparedRequest ?: return
        if (revision <= request.fontEnvironmentRevision) return
        preparedRequest = request.copy(fontEnvironmentRevision = revision)
        PreparedProseInstrumentation.invalidated(PreparedProseInstrumentation.InvalidationReason.FONT)
        requestLayout()
    }

    private fun saturatedRetainedBytes(vararg values: Long): Long = values.fold(0L) { total, value ->
        if (value > 0 && total > Long.MAX_VALUE - value) Long.MAX_VALUE else total + value
    }

    private fun activatePreparedInteraction(interaction: PreparedProseInteraction): Boolean {
        return when (interaction.kind) {
            PreparedProseInteraction.Kind.LINK -> {
                val href = interaction.href ?: return false
                if (!linkInteractionsActionable()) return false
                onLinkTapForTesting?.invoke()
                    ?: interactionListener?.onLinkTap(this, href, interaction.visibleText)
                true
            }
            PreparedProseInteraction.Kind.MENTION -> {
                if (!mentionInteractionsActionable()) return false
                val docPos = interaction.docPos ?: return false
                val attrs = interaction.attrsJson?.let(::parseMentionAttrs)
                if (attrs == null) {
                    interactionListener?.onViewerError(
                        this,
                        ProseViewerError.compiler(
                            "viewer",
                            ProseViewerErrorCode.INVALID_MENTION_ATTRIBUTES.value,
                            "The prepared mention attributes are not a JSON object.",
                        ),
                    )
                    return false
                }
                onMentionTapForTesting?.invoke()
                    ?: interactionListener?.onMentionTap(this, ProseViewerMention(docPos, interaction.label, attrs))
                true
            }
        }
    }

    private fun parseMentionAttrs(json: String): Map<String, Any?>? = runCatching {
        jsonObjectToMap(JSONObject(json))
    }.getOrNull()

    private fun jsonObjectToMap(value: JSONObject): Map<String, Any?> = buildMap {
        value.keys().forEach { key -> put(key, jsonValue(value.get(key))) }
    }

    private fun jsonArrayToList(value: JSONArray): List<Any?> =
        List(value.length()) { index -> jsonValue(value.get(index)) }

    private fun jsonValue(value: Any): Any? = when (value) {
        JSONObject.NULL -> null
        is JSONObject -> jsonObjectToMap(value)
        is JSONArray -> jsonArrayToList(value)
        else -> value
    }

    internal fun activatePreparedInteractionForTesting(interaction: PreparedProseInteraction): Boolean =
        activatePreparedInteraction(interaction)

    override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
        super.onInitializeAccessibilityNodeInfo(info)
        info.className = android.widget.TextView::class.java.name
        info.text = preparedAccessibleNodes().joinToString(" ") { it.label }
        repeat(preparedAccessibleNodes().size) { index ->
            info.addChild(this, index + FIRST_VIRTUAL_ANNOTATION_ID)
        }
    }

    override fun getAccessibilityNodeProvider(): AccessibilityNodeProvider =
        annotationNodeProvider

    // The replacement constructors are API 30 and this module's minSdk is 24;
    // on API 30+ obtain() only delegates to them.
    @Suppress("DEPRECATION")
    private val annotationNodeProvider = object : AccessibilityNodeProvider() {
        override fun createAccessibilityNodeInfo(virtualViewId: Int): AccessibilityNodeInfo? {
            if (virtualViewId == View.NO_ID) {
                return AccessibilityNodeInfo.obtain(this@ProseViewerView).also {
                    onInitializeAccessibilityNodeInfo(it)
                }
            }
            return preparedAccessibilityNodeInfo(virtualViewId)
        }

        override fun performAction(
            virtualViewId: Int,
            action: Int,
            arguments: Bundle?
        ): Boolean {
            return performPreparedAccessibilityAction(virtualViewId, action)
        }
    }

    private fun preparedAccessibleNodes() = preparedArtifact?.accessibilityNodes.orEmpty().filter {
        when (it.role) {
            PreparedProseAccessibilityNode.Role.LINK -> linkInteractionsActionable()
            PreparedProseAccessibilityNode.Role.MENTION -> mentionInteractionsActionable()
        }
    }

    // AccessibilityNodeInfo() is API 30; setBoundsInParent has no replacement
    // and API 24-28 services still read it.
    @Suppress("DEPRECATION")
    private fun preparedAccessibilityNodeInfo(virtualViewId: Int): AccessibilityNodeInfo? {
        val node = preparedAccessibleNodes().getOrNull(virtualViewId - FIRST_VIRTUAL_ANNOTATION_ID) ?: return null
        val parentBounds = Rect(node.bounds).apply { offset(preparedDrawingView.left, preparedDrawingView.top) }
        val screenBounds = preparedAccessibilityScreenBounds(node)
        val visibleToUser = accessibilityNodeVisibleOnScreen(screenBounds)
        reconcileVirtualAccessibilityFocus()
        val identity = accessibilityIdentity(node)
        return AccessibilityNodeInfo.obtain().apply {
            packageName = context.packageName
            className = android.widget.Button::class.java.name
            setSource(this@ProseViewerView, virtualViewId)
            setParent(this@ProseViewerView)
            text = node.label
            contentDescription = node.label
            isClickable = true
            isFocusable = true
            AndroidApiCompat.setScreenReaderFocusable(this, true)
            isAccessibilityFocused = accessibilityFocusedNode?.identity == identity
            isVisibleToUser = visibleToUser
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
            AccessibilityNodeInfo.ACTION_CLICK -> if (preparedAccessibilityNodeVisible(node)) {
                preparedArtifact?.interactions?.getOrNull(node.interactionIndex)
                    ?.let(::activatePreparedInteraction) ?: false
            } else {
                false
            }
            AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS -> requestVirtualAccessibilityFocus(virtualViewId)
            AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS -> clearVirtualAccessibilityFocus(virtualViewId)
            else -> false
        }
    }

    private fun requestVirtualAccessibilityFocus(virtualViewId: Int): Boolean {
        val node = preparedAccessibleNodes()
            .getOrNull(virtualViewId - FIRST_VIRTUAL_ANNOTATION_ID) ?: return false
        if (!preparedAccessibilityNodeVisible(node)) return false
        val identity = accessibilityIdentity(node)
        if (accessibilityFocusedNode?.identity == identity) return false
        accessibilityFocusedNode?.let { previous ->
            accessibilityFocusedNode = null
            sendVirtualAccessibilityEvent(
                previous.virtualId,
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED
            )
        }
        accessibilityFocusedNode = FocusedVirtualNode(virtualViewId, identity)
        invalidate()
        sendVirtualAccessibilityEvent(
            virtualViewId,
            AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED
        )
        return true
    }

    private fun clearVirtualAccessibilityFocus(
        virtualViewId: Int = accessibilityFocusedNode?.virtualId ?: View.NO_ID,
    ): Boolean {
        val focused = accessibilityFocusedNode ?: return false
        if (virtualViewId == View.NO_ID || virtualViewId != focused.virtualId) return false
        accessibilityFocusedNode = null
        invalidate()
        sendVirtualAccessibilityEvent(
            virtualViewId,
            AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED
        )
        return true
    }

    private fun reconcileVirtualAccessibilityFocus() {
        val focused = accessibilityFocusedNode ?: return
        val nodes = preparedAccessibleNodes()
        val index = nodes.indexOfFirst { accessibilityIdentity(it) == focused.identity }
        if (index < 0 || index + FIRST_VIRTUAL_ANNOTATION_ID != focused.virtualId) {
            clearVirtualAccessibilityFocus(focused.virtualId)
            return
        }
        if (!preparedAccessibilityNodeVisible(nodes[index])) {
            clearVirtualAccessibilityFocus(focused.virtualId)
        }
    }

    private fun preparedAccessibilityNodeVisible(node: PreparedProseAccessibilityNode): Boolean =
        preparedAccessibilityScreenBounds(node).let { bounds ->
            accessibilityVisibilityForTesting?.invoke(bounds)
                ?: accessibilityNodeVisibleOnScreen(bounds)
        }

    private fun preparedAccessibilityScreenBounds(node: PreparedProseAccessibilityNode): Rect {
        val bounds = Rect(node.bounds).apply {
            offset(preparedDrawingView.left, preparedDrawingView.top)
        }
        val location = IntArray(2)
        getLocationOnScreen(location)
        bounds.offset(location[0], location[1])
        return bounds
    }

    private fun accessibilityIdentity(node: PreparedProseAccessibilityNode) =
        AccessibilityNodeIdentity(
            preparedArtifact?.key?.generationIdentity,
            node.interactionIndex,
            node.role,
            node.label,
        )

    private fun updatePreparedInteractionCapabilities() {
        preparedDrawingView.linkInteractionsEnabled = linkInteractionsActionable()
        preparedDrawingView.mentionInteractionsEnabled = mentionInteractionsActionable()
    }

    private fun linkInteractionsActionable(): Boolean =
        linkTapsEnabled && (onLinkTapForTesting != null || interactionListener != null)

    private fun mentionInteractionsActionable(): Boolean =
        onMentionTapForTesting != null || interactionListener != null

    // AccessibilityEvent(Int) is API 30; see the node provider above.
    @Suppress("DEPRECATION")
    private fun sendVirtualAccessibilityEvent(virtualViewId: Int, eventType: Int) {
        if (!accessibilityManager.isEnabled) return
        val event = AccessibilityEvent.obtain(eventType).apply {
            packageName = context.packageName
            className = android.widget.Button::class.java.name
            setSource(this@ProseViewerView, virtualViewId)
        }
        parent?.requestSendAccessibilityEvent(this, event)
    }

    @Suppress("DEPRECATION")
    private fun notifyAccessibilitySubtreeChanged() {
        if (!accessibilityManager.isEnabled) return
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

    companion object {
        private const val FIRST_VIRTUAL_ANNOTATION_ID = 1

        /**
         * Explicit availability signal for custom-family loaders. Unknown
         * Typeface fallback is intentionally not treated as a missing font.
         */
        @JvmStatic
        fun registerAvailableFontFamily(family: String, typeface: Typeface) =
            ViewerFontEnvironment.registerAvailableFamily(family, typeface)

        @JvmStatic
        fun markFontFamilyUnavailable(family: String) =
            ViewerFontEnvironment.markFamilyUnavailable(family)

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
