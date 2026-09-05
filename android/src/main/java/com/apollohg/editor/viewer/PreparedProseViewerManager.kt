package com.apollohg.editor.viewer

import android.content.Context
import android.graphics.Rect
import com.apollohg.editor.ProseViewerConfiguration
import com.apollohg.editor.ProseViewerError
import com.apollohg.editor.ProseViewerSource
import com.apollohg.editor.ImageLoadingPolicy
import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.ReadableMap
import com.facebook.react.bridge.WritableMap
import com.facebook.react.module.annotations.ReactModule
import com.facebook.react.uimanager.ReactStylesDiffMap
import com.facebook.react.uimanager.SimpleViewManager
import com.facebook.react.uimanager.StateWrapper
import com.facebook.react.uimanager.ThemedReactContext
import com.facebook.react.uimanager.UIManagerHelper
import com.facebook.react.uimanager.ViewManagerDelegate
import com.facebook.react.uimanager.events.Event
import com.facebook.react.viewmanagers.PreparedProseViewerManagerDelegate
import com.facebook.react.viewmanagers.PreparedProseViewerManagerInterface
import com.facebook.yoga.YogaMeasureMode
import com.facebook.yoga.YogaMeasureOutput
import java.util.WeakHashMap
import kotlin.math.roundToInt

internal fun fabricConstraintPixels(value: Float): Int? {
    if (!value.isFinite() || value <= 0f || value > Int.MAX_VALUE.toFloat()) return null
    return value.roundToInt().takeIf { it > 0 }
}

internal fun fabricPixelsToDp(value: Float, density: Float): Float? {
    if (!value.isFinite() || value < 0f || !density.isFinite() || density <= 0f) return null
    return value / density
}

/** Fabric ViewManager; Yoga measurement creates the artifact and mounting only acquires it. */
@ReactModule(name = PreparedProseViewerManager.REACT_CLASS)
internal class PreparedProseViewerManager :
    SimpleViewManager<PreparedProseDrawingView>(),
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
            state.fontEnvironment.activate(deliverPending = true)
            state.imagePipeline.onPixels = { attachment, lease ->
                val request = state.requestOrNull()
                if (request != null && state.imagePipeline.acceptsCompletion(request.semanticGenerationIdentity)) {
                    view.putImageLease(attachment.id, lease)
                } else {
                    lease.close()
                }
            }
            state.imagePipeline.onPixelsReleased = view::removeImageLeases
            state.imagePipeline.onIntrinsicMetadata = { attachment, width, height ->
                if (state.recordIntrinsicSize(attachment, width, height)) {
                    state.publishAttachmentRevision()
                }
            }
            state.imagePipeline.onResourceFailure = { attachment -> dispatchResourceError(view, attachment) }
        }

    override fun onDropViewInstance(view: PreparedProseDrawingView) {
        states.remove(view)?.let { state ->
            state.finishWithoutMountedReplacement(view)
            state.release()
        }
        view.onUsableMetricsChanged = null
        view.onVisibleRectChanged = null
        view.onFontConfigurationChanged = null
        view.onInteractionActivated = null
        view.clearImageLeases()
        view.install(null)
        super.onDropViewInstance(view)
    }

    override fun onSurfaceStopped(surfaceId: Int) {
        // Yoga can measure a component that never receives a mounted View, so
        // the weak view map is only supplemental lifecycle bookkeeping. The
        // registry cleanup is intentionally unconditional and surface-wide.
        PreparedProseLayoutRegistry.shared.deactivateFabricSurfaceId(surfaceId)
        states.entries.forEach { (view, state) ->
            if (state.ownsFabricSurface(surfaceId)) {
                state.release()
                view.clearImageLeases()
                view.install(null)
            }
        }
        super.onSurfaceStopped(surfaceId)
    }

    /** Test-only Fabric-side per-surface accounting; cache bytes stay separate. */
    internal fun retainedSurfaceBytesForTesting(view: PreparedProseDrawingView): Long =
        states[view]?.retainedSurfaceBytesForTesting(view)
            ?: PreparedProseDrawingView.retainedImagePixelsBytes(emptyMap<String, Any>())

    override fun updateState(
        view: PreparedProseDrawingView,
        props: ReactStylesDiffMap,
        stateWrapper: StateWrapper,
    ): Any? {
        val state = states.getOrPut(view, ::ViewState)
        state.fontEnvironment.activate(deliverPending = true)
        state.replaceStateWrapper(stateWrapper, stateWrapper.stateData?.fabricRevisionsOrNull())
        state.beginSemanticImageGeneration(view)
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
        // Capability only filters the installed interaction/accessibility
        // nodes. It must not reconcile, acquire, or replace the generation.
        view.linkInteractionsEnabled = value
    }

    override fun setMentionInteractionsEnabled(view: PreparedProseDrawingView, value: Boolean) {
        view.mentionInteractionsEnabled = value
    }

    override fun setFontEnvironmentRevision(view: PreparedProseDrawingView, value: Int) =
        update(view) { fontEnvironmentRevision = value.coerceAtLeast(0).toLong() }

    override fun onAfterUpdateTransaction(view: PreparedProseDrawingView) {
        super.onAfterUpdateTransaction(view)
        val state = states[view] ?: return
        state.beginSemanticImageGeneration(view)
        reconcile(view, state)
    }

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
        val finalLayout = FabricLeaseHandleBridge.currentFinalLayout()
        val leaseHandle = FabricLeaseHandleBridge.currentHandle()
        // A state snapshot can outlive its C++ family. Only the synchronous
        // native thread-local handle proves this Yoga callback still belongs
        // to an active exact state incarnation.
        if (surface != null && leaseHandle <= 0L) {
            PreparedProseInstrumentation.trace("measure") { "declined: $surface has no live native lease handle" }
            return YogaMeasureOutput.make(0f, 0f)
        }
        val request = requestFrom(props, state, leaseHandle)
        if (request == null) {
            // An incomplete/stale Yoga callback has not named an exact
            // generation. It must never tear down a newer incarnation on the
            // same surface; recycle and surface-stop own terminal cleanup.
            PreparedProseInstrumentation.trace("measure") { "declined: state named no exact generation (surface=$surface, handle=$leaseHandle)" }
            return YogaMeasureOutput.make(0f, 0f)
        }
        val widthPx = fabricConstraintPixels(width)
        // The registry begins and scopes this exact surface/component sidecar
        // around preparation, so an LRU miss cannot borrow another surface.
        val artifact = if (finalLayout != null && surface != null && widthPx != null) {
            PreparedProseLayoutRegistry.shared.prepareFinalLayout(
                request,
                widthPx,
                density,
                finalLayout.contentOriginXPx,
                finalLayout.contentOriginYPx,
                surface,
                leaseHandle,
                fontScale,
            )
        } else if (
            (widthMode != YogaMeasureMode.EXACTLY && widthMode != YogaMeasureMode.AT_MOST) ||
            widthPx == null
        ) {
            PreparedProseLayoutRegistry.shared.measure(request, 0, density, fabricSurface = surface, fabricLeaseHandle = leaseHandle, fontScale = fontScale)
        } else {
            PreparedProseLayoutRegistry.shared.measure(request, widthPx, density, fabricSurface = surface, fabricLeaseHandle = leaseHandle, fontScale = fontScale)
        }
        val measuredWidth = fabricPixelsToDp(artifact.widthPx.toFloat(), density) ?: 0f
        val intrinsicHeight = fabricPixelsToDp(artifact.heightPx.toFloat(), density) ?: 0f
        val constrainedHeight = fabricPixelsToDp(height, density)
        val measuredHeight = when (heightMode) {
            YogaMeasureMode.EXACTLY -> constrainedHeight ?: 0f
            YogaMeasureMode.AT_MOST -> constrainedHeight?.let { minOf(intrinsicHeight, it) } ?: intrinsicHeight
            else -> intrinsicHeight
        }
        PreparedProseInstrumentation.trace("measure") {
            "surface=$surface handle=$leaseHandle $widthMode widthPx=$widthPx $heightMode heightPx=$height " +
                "-> artifact ${artifact.widthPx}x${artifact.heightPx}px, yoga ${measuredWidth}x${measuredHeight}dp"
        }
        return YogaMeasureOutput.make(measuredWidth, measuredHeight)
    }

    private fun update(view: PreparedProseDrawingView, mutation: ViewState.() -> Unit) {
        val state = states.getOrPut(view, ::ViewState)
        state.mutation()
    }

    private fun installCachedLayout(view: PreparedProseDrawingView) {
        val state = states[view] ?: run {
            PreparedProseInstrumentation.trace("mount") { "declined: no view state (tag=${view.id})" }
            return
        }
        val request = state.requestOrNull() ?: run {
            PreparedProseInstrumentation.trace("mount") { "declined: props/state have not yet named a request (tag=${view.id})" }
            return
        }
        val surfaceId = UIManagerHelper.getSurfaceId(view)
        if (surfaceId < 0 || view.id <= 0) {
            PreparedProseInstrumentation.trace("mount") { "declined: surfaceId=$surfaceId tag=${view.id}" }
            state.finishWithoutMountedReplacement(view)
            return
        }
        val surface = FabricSurfaceToken(surfaceId, view.id)
        val generation = state.adopt(surface, request)
        state.bindFabricAttachmentState(generation)
        val ticket = PreparedProseLayoutRegistry.shared.acquirePreparedMountTicket(
            generation,
            request.nativeFontRevision,
        )
        if (ticket == null) {
            PreparedProseInstrumentation.trace("mount") { "miss: no final layout ticket for $generation" }
            PreparedProseLayoutRegistry.shared.prepareForFabricMount(generation) {
                view.post {
                    val current = states[view] ?: return@post
                    if (current.generation != generation) return@post
                    val currentRequest = current.requestOrNull() ?: return@post
                    val prepared = PreparedProseLayoutRegistry.shared.acquirePreparedMountTicket(
                        generation,
                        currentRequest.nativeFontRevision,
                    )
                        ?: return@post
                    installPreparedTicket(view, current, prepared)
                }
            }
            return
        }
        installPreparedTicket(view, state, ticket)
    }

    private fun installPreparedTicket(
        view: PreparedProseDrawingView,
        state: ViewState,
        ticket: PreparedMountTicket,
    ) {
        val currentRequest = state.requestOrNull() ?: return
        if (ticket.nativeFontRevision != currentRequest.nativeFontRevision) {
            prepareCurrentTicket(view, state)
            return
        }
        PreparedProseInstrumentation.trace("mount") {
            "installed: ${ticket.generation} widthPx=${ticket.contentWidthPx} heightPx=${ticket.artifact.heightPx}"
        }
        state.installMountedReplacement(view, ticket)
        state.beginImages(view, ticket.artifact, currentRequest)
        ViewerAtomConfiguration.parse(currentRequest.configuration.themeJson)?.let { atoms ->
            val density = view.resources.displayMetrics.density
            dispatchViewerEvent(view, "topAtomLayout", Arguments.createMap().apply {
                putString("generation", atoms.generation)
                putString("revision", atoms.revision)
                putDouble("layoutWidth", ticket.artifact.widthPx.toDouble() / density)
                putString("atomsJson", ticket.artifact.atomsJson(density, ticket.contentOriginXPx, ticket.contentOriginYPx))
            })
        }
        ticket.artifact.error?.let { dispatchError(view, currentRequest, it) }
    }

    private fun prepareCurrentTicket(view: PreparedProseDrawingView, state: ViewState) {
        val generation = state.generation ?: return
        PreparedProseLayoutRegistry.shared.prepareForFabricMount(generation) { prepared ->
            if (!prepared) return@prepareForFabricMount
            view.post {
                val current = states[view] ?: return@post
                if (current.generation != generation) return@post
                val request = current.requestOrNull() ?: return@post
                val ticket = PreparedProseLayoutRegistry.shared.acquirePreparedMountTicket(
                    generation,
                    request.nativeFontRevision,
                )
                    ?: return@post
                installPreparedTicket(view, current, ticket)
            }
        }
    }

    private fun dispatchError(
        view: PreparedProseDrawingView,
        request: ProseViewerRequest,
        error: ProseViewerError,
    ) {
        val state = states[view] ?: return
        if (!state.errorReporter.shouldReport(request.semanticGenerationIdentity)) return
        dispatchViewerEvent(view, EVENT_ERROR, Arguments.createMap().apply {
            putString("domain", error.domain)
            putString("code", error.code.value)
            putString("message", error.message)
            putBoolean("fatal", true)
        })
    }

    private fun dispatchResourceError(view: PreparedProseDrawingView, attachment: ViewerImageAttachment) {
        val state = states[view] ?: return
        state.requestOrNull() ?: return
        if (!state.recordResourceFailure(attachment.ordinal)) return
        dispatchViewerEvent(view, EVENT_ERROR, Arguments.createMap().apply {
            putString("domain", "viewer.resource")
            putString("code", "RESOURCE_LOAD_FAILED")
            putString("message", "An image resource could not be loaded.")
            putBoolean("fatal", false)
        })
    }

    private fun dispatchInteraction(view: PreparedProseDrawingView, interaction: PreparedProseInteraction): Boolean {
        val isLink = interaction.kind == PreparedProseInteraction.Kind.LINK
        val payload = Arguments.createMap().apply {
            if (isLink) {
                putString("href", interaction.href)
                putString("text", interaction.visibleText)
            } else {
                putDouble("docPos", (interaction.docPos ?: return false).toDouble())
                putString("label", interaction.label)
                putString("attrsJson", interaction.attrsJson ?: return false)
            }
        }
        dispatchViewerEvent(view, if (isLink) EVENT_PRESS_LINK else EVENT_PRESS_MENTION, payload)
        return true
    }

    private fun dispatchViewerEvent(view: PreparedProseDrawingView, eventName: String, payload: WritableMap) {
        val context = UIManagerHelper.getReactContext(view)
        UIManagerHelper.getEventDispatcherForReactTag(context, view.id)?.dispatchEvent(
            PreparedProseViewerEvent(UIManagerHelper.getSurfaceId(view), view.id, eventName, payload)
        )
    }

    private fun requestFrom(props: ReadableMap?, state: ReadableMap?, nativeLeaseHandle: Long = 0): ProseViewerRequest? {
        val revisions = state?.fabricRevisionsOrNull(nativeLeaseHandle) ?: return null
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
        // `adopt` is the authoritative commit boundary. Releasing G1 here
        // would race a G2 Yoga handoff and blank a valid mounted fallback.
        installCachedLayout(view)
    }

    private fun surfaceToken(data: ReadableMap): FabricSurfaceToken? {
        val surfaceId = data.longOrZero("surfaceId").toInt()
        val componentTag = data.longOrZero("componentTag").toInt()
        return if (surfaceId > 0 && componentTag > 0) FabricSurfaceToken(surfaceId, componentTag) else null
    }

    private fun ReadableMap.stringOrNull(key: String): String? =
        if (hasKey(key) && !isNull(key)) getString(key) else null

    private fun ReadableMap.booleanOrDefault(key: String, default: Boolean): Boolean =
        if (hasKey(key) && !isNull(key)) getBoolean(key) else default

    private fun ReadableMap.longOrZero(key: String): Long =
        if (!hasKey(key) || isNull(key)) 0 else getDouble(key).toLong().coerceAtLeast(0)

    private fun ReadableMap.fabricRevisionsOrNull(nativeLeaseHandle: Long = 0): FabricStateRevisions? {
        val attachmentRevision = longOrNull("attachmentRevision") ?: return null
        val nativeFontRevision = longOrNull("nativeFontRevision") ?: return null
        // State's optional decimal representation is exact. The hot native
        // measurement path receives this same handle as a JNI jlong above.
        val stateHandle = stringOrNull("leaseHandle")?.toLongOrNull()?.takeIf { it > 0 } ?: 0L
        val handle = stateHandle.takeIf { it > 0 } ?: nativeLeaseHandle
        if (nativeLeaseHandle > 0 && stateHandle > 0 && stateHandle != nativeLeaseHandle) return null
        return FabricStateRevisions(attachmentRevision, nativeFontRevision, handle)
    }

    private fun ReadableMap.longOrNull(key: String): Long? =
        if (!hasKey(key) || isNull(key)) null else getDouble(key).toLong().coerceAtLeast(0)

    internal data class FabricStateRevisions(
        val attachmentRevision: Long,
        val nativeFontRevision: Long,
        val leaseHandle: Long,
    )

    internal class ViewState(
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
        // This is independent from the generation lease: an accepted props
        // replacement can release the lease before the next Yoga pass, while
        // the mounted sidecar is still owned by this exact Fabric token.
        private var sidecarGeneration: FabricGenerationToken? = null,
        val errorReporter: FabricErrorReporter = FabricErrorReporter(),
        private val replacementAccessibilityTransaction: FabricReplacementAccessibilityTransaction =
            FabricReplacementAccessibilityTransaction(),
        private val createStateMap: () -> WritableMap = { Arguments.createMap() },
    ) {
        private var pendingFontRevision = 0L
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
            val prior = revisions
            revisions = nextRevisions?.let { incoming ->
                val leaseHandle = incoming.leaseHandle.takeIf { it > 0 }
                    ?: prior?.leaseHandle
                    ?: 0
                if (leaseHandle > 0 && prior?.leaseHandle == leaseHandle) {
                    incoming.copy(
                        attachmentRevision = maxOf(
                            incoming.attachmentRevision,
                            prior.attachmentRevision,
                        ),
                        nativeFontRevision = maxOf(
                            incoming.nativeFontRevision,
                            prior.nativeFontRevision,
                        ),
                        leaseHandle = leaseHandle,
                    )
                } else {
                    incoming.copy(leaseHandle = leaseHandle)
                }
            }
            if (pendingFontRevision > 0 && revisions != null) {
                pendingFontRevision = 0
                val current = requireNotNull(revisions)
                publishRevisions(
                    FabricStateRevisions(
                        current.attachmentRevision,
                        current.nativeFontRevision + 1,
                        current.leaseHandle,
                    )
                )
            }
        }

        private var attachmentRevisions = ViewerAttachmentRevisionState()
        val fontEnvironment = ViewerFontEnvironment()
        val imagePipeline = ViewerImagePipeline()
        private var visibleRect: android.graphics.Rect = android.graphics.Rect()

        fun beginImages(view: PreparedProseDrawingView, artifact: PreparedProseLayout, request: ProseViewerRequest) {
            // The semantic reset happens when props/state are accepted, before
            // Yoga can prepare a replacement. Mount binds only this artifact's
            // ordinal descriptors and must preserve a permitted revision.
            attachmentRevisions.admit(artifact.imageAttachments.size)
            fontEnvironment.activate()
            imagePipeline.begin(request.semanticGenerationIdentity, request.configuration.imagesEnabled, ImageLoadingPolicy.fromJson(request.configuration.imagePolicyJson))
        }

        /** Phase one of Fabric image setup: no artifact is required yet. */
        fun beginSemanticImageGeneration(view: PreparedProseDrawingView) {
            val request = requestOrNull() ?: return
            if (!attachmentRevisions.beginSemanticGeneration(request.semanticGenerationIdentity)) return
            imagePipeline.cancel()
            view.clearImageLeases()
        }

        fun bindFabricAttachmentState(generation: FabricGenerationToken) {
            attachmentRevisions = FabricAttachmentSidecars.state(generation) ?: attachmentRevisions
        }

        fun recordIntrinsicSize(attachment: ViewerImageAttachment, width: Int, height: Int): Boolean =
            attachmentRevisions.recordIntrinsicSize(
                attachment.id,
                attachment.ordinal,
                width,
                height,
                attachment.declaredSize,
            )

        fun recordResourceFailure(ordinal: Int): Boolean =
            attachmentRevisions.recordResourceFailure(ordinal)

        internal fun retainedSurfaceBytesForTesting(view: PreparedProseDrawingView): Long {
            val layout = view.preparedLayout?.retainedBytes ?: 0L
            val sidecar = attachmentRevisions.retainedPublicationBytesForTesting.toLong()
            val pixels = view.retainedImagePixelsBytesForTesting
            return listOf(layout, sidecar, pixels).fold(0L) { total, value ->
                if (value > 0 && total > Long.MAX_VALUE - value) Long.MAX_VALUE else total + value
            }
        }

        fun requestVisibleImages(view: PreparedProseDrawingView, visible: android.graphics.Rect) {
            visibleRect = android.graphics.Rect(visible)
            val artifact = view.preparedLayout ?: return
            imagePipeline.updateVisibleRect(visibleRect, artifact.imageAttachments)
        }

        fun publishAttachmentRevision() {
            val current = revisions ?: return
            publishRevisions(FabricStateRevisions(current.attachmentRevision + 1, current.nativeFontRevision, current.leaseHandle))
        }

        fun publishFontRevision(revision: Long) {
            if (revision <= 0) return
            val current = revisions
            if (current == null) {
                pendingFontRevision = maxOf(pendingFontRevision, revision)
                return
            }
            publishRevisions(FabricStateRevisions(current.attachmentRevision, current.nativeFontRevision + 1, current.leaseHandle))
        }

        private fun publishRevisions(next: FabricStateRevisions) {
            val current = revisions
            if (current == next) return
            revisions = next
            stateWrapper?.updateState(createStateMap().apply {
                putDouble("attachmentRevision", next.attachmentRevision.toDouble())
                putDouble("nativeFontRevision", next.nativeFontRevision.toDouble())
            })
        }

        fun releaseGeneration(
            view: PreparedProseDrawingView,
        ) {
            generation?.let(PreparedProseLayoutRegistry.shared::releaseFabricGeneration)
            generation = null
            releaseSidecarOwnership()
            imagePipeline.cancel()
            replacementAccessibilityTransaction.finishWithoutMountedReplacement(view)
            view.clearImageLeases()
            view.install(null)
        }

        fun installMountedReplacement(view: PreparedProseDrawingView, ticket: PreparedMountTicket) {
            replacementAccessibilityTransaction.installMountedReplacement(view, ticket)
        }

        fun finishWithoutMountedReplacement(view: PreparedProseDrawingView) {
            replacementAccessibilityTransaction.finishWithoutMountedReplacement(view)
        }

        fun adopt(
            surface: FabricSurfaceToken,
            request: ProseViewerRequest,
        ): FabricGenerationToken {
            val handle = revisions?.leaseHandle ?: 0L
            require(handle > 0) { "Fabric mount attempted without an exact native lease handle." }
            val next = FabricGenerationToken(surface, request.generationIdentity, handle)
            // This is the state/props commit boundary. Permit the canonical
            // incoming generation before releasing any prior owner or trying
            // to acquire: delayed G1 Yoga callbacks are now inert, while a
            // G2 measurement already in flight remains eligible to finish.
            PreparedProseLayoutRegistry.shared.activateFabricGeneration(next)
            val previousSidecar = sidecarGeneration
            if (previousSidecar != null && previousSidecar != next) {
                // The view may already carry another Fabric tag by the time
                // this callback runs. Remove only the sidecar token recorded
                // for its former owner, never one reconstructed from the view.
                PreparedProseLayoutRegistry.shared.releaseFabricGeneration(previousSidecar)
            }
            sidecarGeneration = next
            if (generation != null && generation != next) {
                PreparedProseLayoutRegistry.shared.releaseFabricGeneration(generation!!)
            }
            generation = next
            return next
        }

        fun release() {
            // A replacement keeps mounted G1 alive while G2 is pending or
            // failed. Recycling is terminal for the exact state-family
            // handle, so sweep every owner lease after marking it inactive
            // rather than releasing only G2 through active-generation checks.
            val terminalOwner = generation?.let { FabricLeaseOwner(it.surface, it.leaseHandle) }
                ?: sidecarGeneration?.let { FabricLeaseOwner(it.surface, it.leaseHandle) }
            terminalOwner?.let { owner ->
                PreparedProseLayoutRegistry.shared.deactivateFabricLease(owner.surface, owner.leaseHandle)
            }
            generation = null
            sidecarGeneration = null
            imagePipeline.cancel()
            fontEnvironment.deactivate()
            attachmentRevisions.reset()
            stateWrapper?.destroyState()
            stateWrapper = null
            revisions = null
            errorReporter.reset()
        }

        fun ownsFabricSurface(surfaceId: Int): Boolean =
            generation?.surface?.surfaceId == surfaceId ||
                sidecarGeneration?.surface?.surfaceId == surfaceId

        private fun releaseSidecarOwnership() {
            val sidecar = sidecarGeneration ?: return
            // Clear only the state incarnation recorded for this view. A
            // stale recycled H1 must not remove H2 merely because Fabric has
            // reused the same surface/component tag.
            sidecarGeneration = null
            PreparedProseLayoutRegistry.shared.releaseFabricGeneration(sidecar)
        }
    }

    companion object {
        const val REACT_CLASS = "PreparedProseViewer"
        private const val EVENT_ERROR = "topError"
        private const val EVENT_PRESS_LINK = "topPressLink"
        private const val EVENT_PRESS_MENTION = "topPressMention"
    }
}

/**
 * Fabric direct event carrying an already-built payload. Every viewer event
 * reports a discrete outcome, so coalescing stays off: two link presses or two
 * distinct resource failures on one view must both reach JS.
 */
private class PreparedProseViewerEvent(
    surfaceId: Int,
    viewTag: Int,
    private val name: String,
    private val payload: WritableMap,
) : Event<PreparedProseViewerEvent>(surfaceId, viewTag) {
    override fun getEventName(): String = name

    override fun getEventData(): WritableMap = payload

    override fun canCoalesce(): Boolean = false
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

    fun installMountedReplacement(view: PreparedProseDrawingView, ticket: PreparedMountTicket) {
        installMountedReplacement(
            view,
            ticket.artifact,
            ticket.contentOriginXPx,
            ticket.contentOriginYPx,
        )
    }

    fun installMountedReplacement(view: PreparedProseDrawingView, artifact: PreparedProseLayout) {
        installMountedReplacement(view, artifact, 0, 0)
    }

    private fun installMountedReplacement(
        view: PreparedProseDrawingView,
        artifact: PreparedProseLayout,
        contentOriginXPx: Int,
        contentOriginYPx: Int,
    ) {
        view.install(
            artifact,
            announceAccessibilitySubtree = notificationOwner != NotificationOwner.REMOVED_SUBTREE,
            contentOriginXPx = contentOriginXPx,
            contentOriginYPx = contentOriginYPx,
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
