package com.apollohg.editor.viewer

import com.apollohg.editor.ProseViewerError
import java.util.concurrent.CompletableFuture
import java.util.concurrent.ConcurrentHashMap

/** Shared, thread-safe compiler and prepared-layout registry for View and Fabric hosts. */
internal class PreparedProseLayoutRegistry(
    private val compiler: DocumentCompiler = ::compileWithRust,
    private val layoutEngine: AndroidProseLayoutEngine = StaticLayoutAndroidProseLayoutEngine(),
    byteBudget: Long = 32L * 1024L * 1024L,
    private val compiledByteBudget: Long = 8L * 1024L * 1024L,
    private val compilationFailureBudget: Int = 128,
    private val themeEntryBudget: Int = 128,
) {
    private sealed interface Compilation {
        data class Document(val value: ViewerDocument) : Compilation
        data class Failure(val error: ProseViewerError) : Compilation
    }

    private val compilerLock = Any()
    private val compiled = LinkedHashMap<String, ViewerDocument>(16, 0.75f, true)
    private val compilationFailures = LinkedHashMap<String, ProseViewerError>(16, 0.75f, true)
    /** Resolved once per generation and bounded independently of layout entries. */
    private val themes = LinkedHashMap<String, PreparedProseTheme>(16, 0.75f, true)
    private val compilationInFlight = ConcurrentHashMap<String, CompletableFuture<Compilation>>()
    private val documentsByFabricGeneration = mutableMapOf<FabricGenerationToken, ViewerDocument>()
    private val failuresByFabricGeneration = mutableMapOf<FabricGenerationToken, ProseViewerError>()
    private val layoutCache = PreparedProseLayoutCache(byteBudget = byteBudget)
    private var compiledRetainedBytes = 0L
    private var themeRetainedBytes = 0L
    private val themeByteBudget = 512L * 1024L

    @Volatile internal var layoutPreparationCount = 0
        private set

    fun compileDocument(request: ProseViewerRequest): ViewerDocument {
        val cacheKey = request.compiledCacheKey
        synchronized(compilerLock) {
            compiled[cacheKey]?.let { return it }
            compilationFailures[cacheKey]?.let { throw it }
        }
        val fresh = CompletableFuture<Compilation>()
        val existing = compilationInFlight.putIfAbsent(cacheKey, fresh)
        if (existing != null) return existing.join().documentOrThrow()
        val compileStarted = PreparedProseInstrumentation.now()
        val result = try {
            Compilation.Document(compiler(request).also { document ->
                if (!document.semanticKey.matches(Regex("[0-9a-f]{64}"))) {
                    throw ProseViewerError.compiler(
                        "viewer",
                        "INVALID_SEMANTIC_KEY",
                        "The compiler returned an invalid semantic key.",
                    )
                }
            })
        } catch (error: ProseViewerError) {
            Compilation.Failure(error)
        } catch (throwable: Throwable) {
            Compilation.Failure(ProseViewerError.layout(throwable.message ?: "Document compilation failed."))
        }
        PreparedProseInstrumentation.compiled(compileStarted, request.generationIdentity)
        synchronized(compilerLock) {
            when (result) {
                is Compilation.Document -> {
                    compiled[cacheKey] = result.value
                    compiledRetainedBytes += result.value.retainedBytes
                    trimCompiledLocked()
                    PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.COMPILED, "registry", compiledRetainedBytes)
                }
                is Compilation.Failure -> {
                    compilationFailures[cacheKey] = result.error
                    while (compilationFailures.size > compilationFailureBudget) {
                        compilationFailures.remove(compilationFailures.entries.first().key)
                    }
                }
            }
        }
        fresh.complete(result)
        compilationInFlight.remove(cacheKey, fresh)
        return result.documentOrThrow()
    }

    fun measure(
        request: ProseViewerRequest,
        widthPx: Int,
        density: Float,
        fabricSurface: FabricSurfaceToken? = null,
        fabricLeaseHandle: Long = 0,
        compiledDocument: ViewerDocument? = null,
        fontScale: Float = 1f,
        measurementImageState: ViewerAttachmentRevisionState? = null,
    ): PreparedProseLayout {
        val generation = fabricSurface?.takeIf { fabricLeaseHandle > 0 }
            ?.let { FabricGenerationToken(it, request.generationIdentity, fabricLeaseHandle) }
        if (!isValidMeasurement(widthPx, density)) {
            // An old zero-width pass is not a lifecycle event.  Retire only
            // this handle's pending handoffs and leave every mounted layout,
            // including a newer H2, untouched.
            generation?.let { layoutCache.releasePendingLease(it) }
            return invalidWidthArtifact(request)
        }
        val densityBits = density.toRawBits().toLong()
        // Fabric resolves exactly its stable surface/component sidecar; direct
        // hosts pass their own mounted state. Neither path scans other hosts.
        val imageMeasurementState = generation?.let {
            FabricAttachmentSidecars.begin(it, request.semanticGenerationIdentity)
        } ?: measurementImageState
        return try {
            val document = preparedDocument(request, generation, compiledDocument)
            val theme = resolveTheme(request, density, fontScale)
            val key = layoutKey(document, request, widthPx, densityBits)
            layoutCache.value(key, generation) {
                val layoutStarted = PreparedProseInstrumentation.now()
                layoutPreparationCount += 1
                try {
                    val prepare = {
                        layoutEngine.prepare(
                            document,
                            key,
                            theme,
                            widthPx,
                            density,
                            request.configuration.collapsesWhenEmpty,
                            request.semanticGenerationIdentity,
                        )
                    }
                    val artifact = if (imageMeasurementState != null) {
                        FabricAttachmentSidecars.withMeasurementState(imageMeasurementState, prepare)
                    } else prepare()
                    PreparedProseInstrumentation.laidOut(layoutStarted, request.generationIdentity)
                    artifact
                } catch (error: ProseViewerError) {
                    PreparedProseLayout.error(key, widthPx, error)
                } catch (throwable: Throwable) {
                    PreparedProseLayout.error(key, widthPx, ProseViewerError.layout(throwable.message ?: "Layout preparation failed."))
                }
            }
        } catch (error: ProseViewerError) {
            cachedErrorArtifact(request, widthPx, densityBits, error, generation)
        }
    }

    /** Mount acquisition intentionally cannot invoke Rust or StaticLayout.Builder. */
    fun acquireForFabricMount(
        generation: FabricGenerationToken,
        request: ProseViewerRequest,
        widthPx: Int,
        density: Float,
    ): PreparedProseLayout? {
        if (!isValidMeasurement(widthPx, density)) return null
        return layoutCache.acquireForFabricMount(
            generation,
            widthPx,
            density.toRawBits().toLong(),
        )
    }

    fun releaseFabricGeneration(generation: FabricGenerationToken) {
        layoutCache.releaseLease(generation)
        FabricAttachmentSidecars.remove(generation)
        synchronized(compilerLock) {
            documentsByFabricGeneration.remove(generation)
            failuresByFabricGeneration.remove(generation)
        }
    }

    fun releaseFabricSurface(surface: FabricSurfaceToken) {
        layoutCache.releaseSurface(surface)
        FabricAttachmentSidecars.remove(surface)
        synchronized(compilerLock) {
            documentsByFabricGeneration.keys.removeAll { it.surface == surface }
            failuresByFabricGeneration.keys.removeAll { it.surface == surface }
        }
    }

    /**
     * Surface shutdown is broader than per-view recycling: Yoga may have
     * measured components that never mounted, so no ViewState remains to name
     * their exact component token. Clear every lease and generation pin for
     * this Fabric surface unconditionally.
     */
    fun releaseFabricSurfaceId(surfaceId: Int) {
        layoutCache.releaseSurfaceId(surfaceId)
        FabricAttachmentSidecars.removeSurface(surfaceId)
        synchronized(compilerLock) {
            documentsByFabricGeneration.keys.removeAll { it.surface.surfaceId == surfaceId }
            failuresByFabricGeneration.keys.removeAll { it.surface.surfaceId == surfaceId }
        }
    }

    /** A stale mount can retire only the exact unmounted handoff it requested. */
    fun releaseFabricMountMiss(generation: FabricGenerationToken, widthPx: Int, density: Float) {
        if (!isValidMeasurement(widthPx, density)) return
        layoutCache.releasePendingLease(generation, widthPx, density.toRawBits().toLong())
        synchronized(compilerLock) {
            if (!hasFabricLease(generation)) {
                documentsByFabricGeneration.remove(generation)
                failuresByFabricGeneration.remove(generation)
            }
        }
    }

    fun registerDirectMounted(owner: String, layout: PreparedProseLayout) = layoutCache.registerDirectMount(owner, layout)
    fun releaseDirectMounted(owner: String) = layoutCache.releaseDirectMount(owner)

    fun didReceiveMemoryWarning() {
        PreparedProseInstrumentation.invalidated(PreparedProseInstrumentation.InvalidationReason.MEMORY_PRESSURE)
        layoutCache.removeAllUnmounted()
        synchronized(compilerLock) {
            compiled.clear()
            compilationFailures.clear()
            themes.clear()
            documentsByFabricGeneration.clear()
            failuresByFabricGeneration.clear()
            compiledRetainedBytes = 0
            themeRetainedBytes = 0
        }
        PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.COMPILED, "registry", 0L)
    }

    internal val preparedLayoutCacheCountForTesting: Int get() = layoutCache.completedCountForTesting
    internal val layoutRetainedBytesForTesting: Long get() = layoutCache.retainedBytesForTesting
    internal val fabricLeaseCountForTesting: Int get() = layoutCache.leaseCountForTesting
    internal val fabricGenerationPinCountForTesting: Int get() = synchronized(compilerLock) {
        documentsByFabricGeneration.size + failuresByFabricGeneration.size
    }
    internal val preparedThemeCountForTesting: Int get() = synchronized(compilerLock) { themes.size }
    private fun hasFabricLease(generation: FabricGenerationToken): Boolean = layoutCache.hasLease(generation)

    private fun preparedDocument(
        request: ProseViewerRequest,
        generation: FabricGenerationToken?,
        suppliedDocument: ViewerDocument?,
    ): ViewerDocument {
        if (generation != null) synchronized(compilerLock) {
            documentsByFabricGeneration[generation]?.let { return it }
            failuresByFabricGeneration[generation]?.let { throw it }
        }
        return try {
            // Pins deliberately retain only compiler semantics. Theme paints are
            // density-local measurement inputs and must never cross a density change.
            val semanticDocument = suppliedDocument ?: compileDocument(request)
            if (generation != null) synchronized(compilerLock) { documentsByFabricGeneration[generation] = semanticDocument }
            semanticDocument
        } catch (error: ProseViewerError) {
            if (generation != null) synchronized(compilerLock) { failuresByFabricGeneration[generation] = error }
            throw error
        }
    }

    private fun cachedErrorArtifact(
        request: ProseViewerRequest,
        widthPx: Int,
        densityBits: Long,
        error: ProseViewerError,
        fabricGeneration: FabricGenerationToken?,
    ): PreparedProseLayout {
        val key = ProseLayoutKey(
            semanticKey = "error:${request.compiledCacheKey}",
            widthPx = widthPx,
            themeDigest = request.themeDigest,
            nativeFontRevision = request.nativeFontRevision,
            fontEnvironmentRevision = request.fontEnvironmentRevision,
            densityBits = densityBits,
            attachmentRevision = request.attachmentRevision,
            generationIdentity = request.generationIdentity,
            semanticGenerationIdentity = request.semanticGenerationIdentity,
        )
        return layoutCache.value(key, fabricGeneration) { PreparedProseLayout.error(key, widthPx, error) }
    }

    private fun invalidWidthArtifact(request: ProseViewerRequest): PreparedProseLayout {
        val key = ProseLayoutKey(
            semanticKey = "error:${request.compiledCacheKey}",
            widthPx = 0,
            themeDigest = request.themeDigest,
            nativeFontRevision = request.nativeFontRevision,
            fontEnvironmentRevision = request.fontEnvironmentRevision,
            densityBits = 0,
            attachmentRevision = request.attachmentRevision,
            generationIdentity = request.generationIdentity,
            semanticGenerationIdentity = request.semanticGenerationIdentity,
        )
        return PreparedProseLayout.error(key, 0, ProseViewerError.invalidWidth())
    }

    private fun layoutKey(document: ViewerDocument, request: ProseViewerRequest, widthPx: Int, densityBits: Long) =
        ProseLayoutKey(
            semanticKey = document.semanticKey,
            widthPx = widthPx,
            themeDigest = request.themeDigest,
            nativeFontRevision = request.nativeFontRevision,
            fontEnvironmentRevision = request.fontEnvironmentRevision,
            densityBits = densityBits,
            attachmentRevision = request.attachmentRevision,
            generationIdentity = request.generationIdentity,
            semanticGenerationIdentity = request.semanticGenerationIdentity,
        )

    private fun trimCompiledLocked() {
        while (compiledRetainedBytes > compiledByteBudget && compiled.isNotEmpty()) {
            val oldest = compiled.entries.first()
            compiled.remove(oldest.key)
            compiledRetainedBytes -= oldest.value.retainedBytes
        }
        PreparedProseInstrumentation.retained(PreparedProseInstrumentation.Owner.COMPILED, "registry", compiledRetainedBytes)
    }

    private fun resolveTheme(request: ProseViewerRequest, density: Float, fontScale: Float): PreparedProseTheme = synchronized(compilerLock) {
        val key = "${request.generationIdentity}:${density.toRawBits()}:${fontScale.toRawBits()}"
        themes[key]?.let { return@synchronized it }
        val resolved = PreparedProseTheme.resolve(
            request.configuration.themeJson,
            density,
            fontScale,
            request.semanticGenerationIdentity,
        )
        themes[key] = resolved
        themeRetainedBytes += resolved.retainedBytes
        while ((themeRetainedBytes > themeByteBudget || themes.size > themeEntryBudget) && themes.isNotEmpty()) {
            val oldest = themes.entries.first()
            themes.remove(oldest.key)
            themeRetainedBytes -= oldest.value.retainedBytes
        }
        resolved
    }


    private fun Compilation.documentOrThrow(): ViewerDocument = when (this) {
        is Compilation.Document -> value
        is Compilation.Failure -> throw error
    }

    private fun isValidMeasurement(widthPx: Int, density: Float): Boolean =
        widthPx > 0 && density.isFinite() && density > 0f

    companion object {
        val shared = PreparedProseLayoutRegistry()
    }
}
