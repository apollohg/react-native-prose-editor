package com.apollohg.editor.viewer

import android.content.Context
import com.apollohg.editor.ProseViewerConfiguration
import com.apollohg.editor.ProseViewerError
import com.apollohg.editor.ProseViewerSource
import com.apollohg.editor.ImageLoadingPolicy
import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.ReadableMap
import com.facebook.react.module.annotations.ReactModule
import com.facebook.react.uimanager.BaseViewManager
import com.facebook.react.uimanager.LayoutShadowNode
import com.facebook.react.uimanager.ReactStylesDiffMap
import com.facebook.react.uimanager.StateWrapper
import com.facebook.react.uimanager.ThemedReactContext
import com.facebook.react.uimanager.UIManagerHelper
import com.facebook.react.uimanager.ViewManagerDelegate
import com.facebook.react.viewmanagers.PreparedProseViewerManagerDelegate
import com.facebook.react.viewmanagers.PreparedProseViewerManagerInterface
import com.facebook.yoga.YogaMeasureMode
import com.facebook.yoga.YogaMeasureOutput
import java.util.WeakHashMap
import kotlin.math.roundToInt

/** Fabric ViewManager; Yoga measurement creates the artifact and mounting only acquires it. */
@ReactModule(name = PreparedProseViewerManager.REACT_CLASS)
internal class PreparedProseViewerManager :
    BaseViewManager<PreparedProseDrawingView, LayoutShadowNode>(),
    PreparedProseViewerManagerInterface<PreparedProseDrawingView> {
    private val delegate: ViewManagerDelegate<PreparedProseDrawingView> =
        PreparedProseViewerManagerDelegate(this)
    private val states = WeakHashMap<PreparedProseDrawingView, ViewState>()

    override fun getName(): String = REACT_CLASS

    override fun getDelegate(): ViewManagerDelegate<PreparedProseDrawingView> = delegate

    override fun createViewInstance(context: ThemedReactContext): PreparedProseDrawingView =
        PreparedProseDrawingView(context).also { view ->
            val state = ViewState()
            states[view] = state
            view.onUsableMetricsChanged = { installCachedLayout(view) }
            view.onVisibleRectChanged = { visible -> state.requestVisibleImages(view, visible) }
            view.onFontConfigurationChanged = { configuration -> state.fontEnvironment.onConfigurationChanged(configuration) }
            view.onInteractionActivated = { interaction -> dispatchInteraction(view, interaction) }
            state.fontEnvironment.onInvalidated = { revision -> state.publishFontRevision(revision) }
            state.imagePipeline.onPixels = { attachment, bitmap ->
                val request = state.requestOrNull()
                if (request != null && state.imagePipeline.acceptsCompletion(request.semanticGenerationIdentity)) {
                    view.imagePixels = view.imagePixels + (attachment.id to bitmap)
                }
            }
            state.imagePipeline.onIntrinsicMetadata = { attachment, width, height ->
                if (state.attachmentRevisions.recordIntrinsicSize(attachment.id, attachment.ordinal, width, height, attachment.declaredSize)) {
                    state.publishAttachmentRevision()
                }
            }
            state.imagePipeline.onResourceFailure = { attachment -> dispatchResourceError(view, attachment) }
        }

    override fun createShadowNodeInstance(): LayoutShadowNode = LayoutShadowNode()

    override fun getShadowNodeClass(): Class<LayoutShadowNode> = LayoutShadowNode::class.java

    override fun updateExtraData(root: PreparedProseDrawingView, extraData: Any?) = Unit

    override fun onDropViewInstance(view: PreparedProseDrawingView) {
        states.remove(view)?.let { state ->
            state.finishWithoutMountedReplacement(view)
            state.release()
        }
        view.onUsableMetricsChanged = null
        view.onVisibleRectChanged = null
        view.onFontConfigurationChanged = null
        view.onInteractionActivated = null
        view.install(null)
        super.onDropViewInstance(view)
    }

    override fun onSurfaceStopped(surfaceId: Int) {
        // Yoga can measure a component that never receives a mounted View, so
        // the weak view map is only supplemental lifecycle bookkeeping. The
        // registry cleanup is intentionally unconditional and surface-wide.
        PreparedProseLayoutRegistry.shared.releaseFabricSurfaceId(surfaceId)
        states.values.forEach { state ->
            if (state.generation?.surface?.surfaceId == surfaceId) state.release()
        }
        super.onSurfaceStopped(surfaceId)
    }

    override fun updateState(
        view: PreparedProseDrawingView,
        props: ReactStylesDiffMap,
        stateWrapper: StateWrapper,
    ): Any? {
        val state = states.getOrPut(view, ::ViewState)
        state.replaceStateWrapper(stateWrapper, stateWrapper.stateData?.fabricRevisionsOrNull())
        reconcile(view, state)
        return null
    }

    override fun setSourceKind(view: PreparedProseDrawingView, value: String?) =
        update(view) { sourceKind = value ?: "json" }

    override fun setSource(view: PreparedProseDrawingView, value: String?) =
        update(view) { source = value.orEmpty() }

    override fun setConfigJson(view: PreparedProseDrawingView, value: String?) =
        update(view) { configJson = value ?: "{}" }

    override fun setThemeJson(view: PreparedProseDrawingView, value: String?) =
        update(view) { themeJson = value }

    override fun setImagePolicyJson(view: PreparedProseDrawingView, value: String?) =
        update(view) { imagePolicyJson = value }

    override fun setImagesEnabled(view: PreparedProseDrawingView, value: Boolean) =
        update(view) { imagesEnabled = value }

    override fun setCollapsesWhenEmpty(view: PreparedProseDrawingView, value: Boolean) =
        update(view) { collapsesWhenEmpty = value }

    override fun setEnableLinkTaps(view: PreparedProseDrawingView, value: Boolean) {
        // Permission only filters the installed interaction/accessibility
        // nodes. It must not reconcile, acquire, or replace the generation.
        view.linkInteractionsEnabled = value
    }

    override fun setFontEnvironmentRevision(view: PreparedProseDrawingView, value: Int) =
        update(view) { fontEnvironmentRevision = value.coerceAtLeast(0).toLong() }

    override fun measure(
        context: Context,
        localData: ReadableMap?,
        props: ReadableMap?,
        state: ReadableMap?,
        width: Float,
        widthMode: YogaMeasureMode,
        height: Float,
        heightMode: YogaMeasureMode,
        attachmentsPositions: FloatArray?,
    ): Long {
        val density = context.resources.displayMetrics.density
        val fontScale = context.resources.configuration.fontScale
        val surface = localData?.let(::surfaceToken)
        val request = requestFrom(props, state)
        if (request == null) {
            surface?.let(PreparedProseLayoutRegistry.shared::releaseFabricSurface)
            return YogaMeasureOutput.make(0f, 0f)
        }
        val widthPx = widthToPixels(width, density)
        val artifact = if (
            (widthMode != YogaMeasureMode.EXACTLY && widthMode != YogaMeasureMode.AT_MOST) ||
            widthPx == null
        ) {
            PreparedProseLayoutRegistry.shared.measure(request, 0, density, fabricSurface = surface, fontScale = fontScale)
        } else {
            PreparedProseLayoutRegistry.shared.measure(request, widthPx, density, fabricSurface = surface, fontScale = fontScale)
        }
        val measuredWidth = artifact.widthPx / density
        val intrinsicHeight = artifact.heightPx / density
        val measuredHeight = when (heightMode) {
            YogaMeasureMode.EXACTLY -> height
            YogaMeasureMode.AT_MOST -> minOf(intrinsicHeight, height)
            else -> intrinsicHeight
        }
        return YogaMeasureOutput.make(measuredWidth, measuredHeight)
    }

    private fun update(view: PreparedProseDrawingView, mutation: ViewState.() -> Unit) {
        val state = states.getOrPut(view, ::ViewState)
        state.mutation()
        reconcile(view, state)
    }

    private fun installCachedLayout(view: PreparedProseDrawingView) {
        val state = states[view] ?: return
        val request = state.requestOrNull() ?: return
        val surfaceId = UIManagerHelper.getSurfaceId(view)
        if (surfaceId < 0 || view.id <= 0 || view.width <= 0) {
            state.finishWithoutMountedReplacement(view)
            return
        }
        val density = view.resources.displayMetrics.density
        val widthPx = widthToPixels(view.width / density, density) ?: run {
            state.finishWithoutMountedReplacement(view)
            return
        }
        val surface = FabricSurfaceToken(surfaceId, view.id)
        val generation = state.adopt(surface, request)
        val artifact = PreparedProseLayoutRegistry.shared.acquireForFabricMount(surface, request, widthPx, density)
        if (artifact == null) {
            PreparedProseLayoutRegistry.shared.releaseFabricMountMiss(generation)
            state.finishWithoutMountedReplacement(view)
            return
        }
        state.installMountedReplacement(view, artifact)
        state.beginImages(view, artifact, request)
        artifact.error?.let { dispatchError(view, request, it) }
    }

    private fun dispatchError(
        view: PreparedProseDrawingView,
        request: ProseViewerRequest,
        error: ProseViewerError,
    ) {
        val state = states[view] ?: return
        if (!state.errorReporter.shouldReport(request.semanticGenerationIdentity)) return
        val context = UIManagerHelper.getReactContext(view)
        context.getJSModule(com.facebook.react.uimanager.events.RCTEventEmitter::class.java).receiveEvent(
            view.id,
            "topError",
            Arguments.createMap().apply {
                putString("domain", error.domain)
                putString("code", error.code.value)
                putString("message", error.message)
                putBoolean("fatal", true)
            },
        )
    }

    private fun dispatchResourceError(view: PreparedProseDrawingView, attachment: ViewerImageAttachment) {
        val state = states[view] ?: return
        val request = state.requestOrNull() ?: return
        if (!state.attachmentRevisions.recordResourceFailure(attachment.ordinal)) return
        UIManagerHelper.getReactContext(view)
            .getJSModule(com.facebook.react.uimanager.events.RCTEventEmitter::class.java)
            .receiveEvent(view.id, "topError", Arguments.createMap().apply {
                putString("domain", "viewer.resource")
                putString("code", "RESOURCE_LOAD_FAILED")
                putString("message", "An image resource could not be loaded.")
                putBoolean("fatal", false)
            })
    }

    private fun dispatchInteraction(view: PreparedProseDrawingView, interaction: PreparedProseInteraction): Boolean {
        val context = UIManagerHelper.getReactContext(view)
        context.getJSModule(com.facebook.react.uimanager.events.RCTEventEmitter::class.java).receiveEvent(
            view.id,
            if (interaction.kind == PreparedProseInteraction.Kind.LINK) "topPressLink" else "topPressMention",
            Arguments.createMap().apply {
                if (interaction.kind == PreparedProseInteraction.Kind.LINK) {
                    putString("href", interaction.href)
                    putString("text", interaction.visibleText)
                } else {
                    putDouble("docPos", (interaction.docPos ?: 0L).toDouble())
                    putString("label", interaction.label)
                }
            },
        )
        return true
    }

    private fun requestFrom(props: ReadableMap?, state: ReadableMap?): ProseViewerRequest? {
        val revisions = state?.fabricRevisionsOrNull() ?: return null
        return requestFrom(props, revisions)
    }

    private fun requestFrom(props: ReadableMap?, revisions: FabricStateRevisions): ProseViewerRequest {
        val sourceKind = props?.stringOrNull("sourceKind") ?: "json"
        val source = if (sourceKind == "html") {
            ProseViewerSource.Html(props?.stringOrNull("source").orEmpty())
        } else {
            ProseViewerSource.Json(props?.stringOrNull("source").orEmpty())
        }
        return ProseViewerRequest(
            source = source,
            configuration = ProseViewerConfiguration(
                configJson = props?.stringOrNull("configJson") ?: "{}",
                themeJson = props?.stringOrNull("themeJson"),
                imagePolicyJson = props?.stringOrNull("imagePolicyJson"),
                imagesEnabled = props?.booleanOrDefault("imagesEnabled", true) ?: true,
                collapsesWhenEmpty = props?.booleanOrDefault("collapsesWhenEmpty", true) ?: true,
            ),
            attachmentRevision = revisions.attachmentRevision,
            nativeFontRevision = revisions.nativeFontRevision,
            fontEnvironmentRevision = props?.longOrZero("fontEnvironmentRevision") ?: 0,
        )
    }

    private fun reconcile(view: PreparedProseDrawingView, state: ViewState) {
        val request = state.requestOrNull() ?: run {
            state.releaseGeneration(view)
            return
        }
        state.releaseReplacedGeneration(request, view)
        installCachedLayout(view)
    }

    private fun surfaceToken(data: ReadableMap): FabricSurfaceToken? {
        val surfaceId = data.longOrZero("surfaceId").toInt()
        val componentTag = data.longOrZero("componentTag").toInt()
        return if (surfaceId > 0 && componentTag > 0) FabricSurfaceToken(surfaceId, componentTag) else null
    }

    private fun widthToPixels(widthDip: Float, density: Float): Int? {
        if (!widthDip.isFinite() || widthDip <= 0f || !density.isFinite() || density <= 0f) return null
        val pixels = widthDip.toDouble() * density.toDouble()
        if (!pixels.isFinite() || pixels <= 0 || pixels > Int.MAX_VALUE.toDouble()) return null
        return pixels.roundToInt().takeIf { it > 0 }
    }

    private fun ReadableMap.stringOrNull(key: String): String? =
        if (hasKey(key) && !isNull(key)) getString(key) else null

    private fun ReadableMap.booleanOrDefault(key: String, default: Boolean): Boolean =
        if (hasKey(key) && !isNull(key)) getBoolean(key) else default

    private fun ReadableMap.longOrZero(key: String): Long =
        if (!hasKey(key) || isNull(key)) 0 else getDouble(key).toLong().coerceAtLeast(0)

    private fun ReadableMap.fabricRevisionsOrNull(): FabricStateRevisions? {
        val attachmentRevision = longOrNull("attachmentRevision") ?: return null
        val nativeFontRevision = longOrNull("nativeFontRevision") ?: return null
        return FabricStateRevisions(attachmentRevision, nativeFontRevision)
    }

    private fun ReadableMap.longOrNull(key: String): Long? =
        if (!hasKey(key) || isNull(key)) null else getDouble(key).toLong().coerceAtLeast(0)

    private data class FabricStateRevisions(
        val attachmentRevision: Long,
        val nativeFontRevision: Long,
    )

    private class ViewState(
        var sourceKind: String = "json",
        var source: String = "",
        var configJson: String = "{}",
        var themeJson: String? = null,
        var imagePolicyJson: String? = null,
        var imagesEnabled: Boolean = true,
        var collapsesWhenEmpty: Boolean = true,
        var fontEnvironmentRevision: Long = 0,
        var revisions: FabricStateRevisions? = null,
        var stateWrapper: StateWrapper? = null,
        var generation: FabricGenerationToken? = null,
        val errorReporter: FabricErrorReporter = FabricErrorReporter(),
        private val replacementAccessibilityTransaction: FabricReplacementAccessibilityTransaction =
            FabricReplacementAccessibilityTransaction(),
    ) {
        fun requestOrNull(): ProseViewerRequest? = revisions?.let { revisions ->
            ProseViewerRequest(
                source = if (sourceKind == "html") ProseViewerSource.Html(source) else ProseViewerSource.Json(source),
                configuration = ProseViewerConfiguration(
                    configJson,
                    themeJson,
                    imagePolicyJson,
                    imagesEnabled,
                    collapsesWhenEmpty,
                ),
                attachmentRevision = revisions.attachmentRevision,
                nativeFontRevision = revisions.nativeFontRevision,
                fontEnvironmentRevision = fontEnvironmentRevision,
            )
        }

        fun replaceStateWrapper(next: StateWrapper, nextRevisions: FabricStateRevisions?) {
            if (stateWrapper !== next) stateWrapper?.destroyState()
            stateWrapper = next
            revisions = nextRevisions
        }

        val attachmentRevisions = ViewerAttachmentRevisionState()
        val fontEnvironment = ViewerFontEnvironment()
        val imagePipeline = ViewerImagePipeline()
        private var visibleRect: android.graphics.Rect = android.graphics.Rect()

        fun beginImages(view: PreparedProseDrawingView, artifact: PreparedProseLayout, request: ProseViewerRequest) {
            attachmentRevisions.beginSemanticGeneration(request.semanticGenerationIdentity)
            attachmentRevisions.admit(artifact.imageAttachments.size)
            fontEnvironment.activate()
            imagePipeline.begin(request.semanticGenerationIdentity, request.configuration.imagesEnabled, ImageLoadingPolicy.fromJson(request.configuration.imagePolicyJson))
        }

        fun requestVisibleImages(view: PreparedProseDrawingView, visible: android.graphics.Rect) {
            visibleRect = android.graphics.Rect(visible)
            val artifact = view.preparedLayout ?: return
            imagePipeline.updateVisibleRect(visibleRect, artifact.imageAttachments)
        }

        fun publishAttachmentRevision() {
            val current = revisions ?: return
            publishRevisions(FabricStateRevisions(current.attachmentRevision + 1, current.nativeFontRevision))
        }

        fun publishFontRevision(revision: Long) {
            val current = revisions ?: return
            if (revision <= 0) return
            publishRevisions(FabricStateRevisions(current.attachmentRevision, current.nativeFontRevision + 1))
        }

        private fun publishRevisions(next: FabricStateRevisions) {
            val current = revisions
            if (current == next) return
            revisions = next
            stateWrapper?.updateState(Arguments.createMap().apply {
                putDouble("attachmentRevision", next.attachmentRevision.toDouble())
                putDouble("nativeFontRevision", next.nativeFontRevision.toDouble())
            })
        }

        fun releaseGeneration(
            view: PreparedProseDrawingView,
        ) {
            generation?.let(PreparedProseLayoutRegistry.shared::releaseFabricGeneration)
            generation = null
            imagePipeline.cancel()
            fontEnvironment.deactivate()
            replacementAccessibilityTransaction.finishWithoutMountedReplacement(view)
            view.install(null)
        }

        fun releaseReplacedGeneration(request: ProseViewerRequest, view: PreparedProseDrawingView) {
            val previous = generation ?: return
            if (previous.generationIdentity == request.generationIdentity) return
            PreparedProseLayoutRegistry.shared.releaseFabricGeneration(previous)
            generation = null
            imagePipeline.cancel()
            replacementAccessibilityTransaction.clearReplacing(view)
        }

        fun installMountedReplacement(view: PreparedProseDrawingView, artifact: PreparedProseLayout) {
            replacementAccessibilityTransaction.installMountedReplacement(view, artifact)
        }

        fun finishWithoutMountedReplacement(view: PreparedProseDrawingView) {
            replacementAccessibilityTransaction.finishWithoutMountedReplacement(view)
        }

        fun adopt(
            surface: FabricSurfaceToken,
            request: ProseViewerRequest,
        ): FabricGenerationToken {
            val next = FabricGenerationToken(surface, request.generationIdentity)
            if (generation != null && generation != next) {
                PreparedProseLayoutRegistry.shared.releaseFabricGeneration(generation!!)
            }
            generation = next
            return next
        }

        fun release() {
            generation?.let(PreparedProseLayoutRegistry.shared::releaseFabricGeneration)
            generation = null
            imagePipeline.cancel()
            fontEnvironment.deactivate()
            attachmentRevisions.reset()
            stateWrapper?.destroyState()
            stateWrapper = null
            revisions = null
            errorReporter.reset()
        }
    }

    companion object {
        const val REACT_CLASS = "PreparedProseViewer"
    }
}

/**
 * Makes replacement accessibility notification ownership explicit. A removed
 * old subtree is announced by either the final mounted artifact or, if Fabric
 * cannot mount it, by the removal itself—never by both.
 */
internal class FabricReplacementAccessibilityTransaction {
    private enum class NotificationOwner {
        NONE,
        FINAL_INSTALL,
        REMOVED_SUBTREE,
    }

    private var notificationOwner = NotificationOwner.NONE

    fun clearReplacing(view: PreparedProseDrawingView) {
        if (view.preparedLayout == null) return
        view.install(null, announceAccessibilitySubtree = false)
        notificationOwner = NotificationOwner.FINAL_INSTALL
    }

    fun installMountedReplacement(view: PreparedProseDrawingView, artifact: PreparedProseLayout) {
        view.install(
            artifact,
            announceAccessibilitySubtree = notificationOwner != NotificationOwner.REMOVED_SUBTREE,
        )
        notificationOwner = NotificationOwner.NONE
    }

    fun finishWithoutMountedReplacement(view: PreparedProseDrawingView) {
        if (notificationOwner != NotificationOwner.FINAL_INSTALL) return
        view.announceAccessibilitySubtreeChanged()
        notificationOwner = NotificationOwner.REMOVED_SUBTREE
    }
}

/** Small lifecycle seam that guarantees Fabric emits one error per request generation. */
internal class FabricErrorReporter {
    private var reportedGenerationIdentity: String? = null

    fun shouldReport(generationIdentity: String): Boolean {
        if (reportedGenerationIdentity == generationIdentity) return false
        reportedGenerationIdentity = generationIdentity
        return true
    }

    fun reset() {
        reportedGenerationIdentity = null
    }
}
