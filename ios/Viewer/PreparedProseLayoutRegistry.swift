import CryptoKit
import Foundation
import UIKit

/// Shared compiler and prepared-layout owner for UIKit and Fabric.
@objc(PREPPreparedProseLayoutRegistry)
public final class PreparedProseLayoutRegistry: NSObject {
    typealias DocumentCompiler = (ProseViewerRequest) throws -> ViewerDocument
    typealias LayoutPreparation = (ViewerDocument, ProseLayoutKey, CGFloat, CGFloat) throws -> PreparedProseLayout

    @objc public class var sharedRegistry: PreparedProseLayoutRegistry { shared }
    static let shared = PreparedProseLayoutRegistry()

    private final class Compilation {
        var result: Result<ViewerDocument, Error>?
    }

    private let lock = NSLock()
    private let compiledCondition = NSCondition()
    private var compiledDocuments: [String: ViewerDocument] = [:]
    private var compiledAccessOrder: [String] = []
    private var compiledInFlight: [String: Compilation] = [:]
    private var compilationFailures: [String: Error] = [:]
    private var compilationFailureAccessOrder: [String] = []
    private var documentsByFabricGeneration: [FabricGenerationToken: ViewerDocument] = [:]
    private var failuresByFabricGeneration: [FabricGenerationToken: Error] = [:]
    private var themesByGeneration: [String: PreparedProseTheme] = [:]
    private var themeAccessOrder: [String] = []
    private var themeOwners: [FabricGenerationToken: String] = [:]
    private var themeOwnerCounts: [String: Int] = [:]
    private var themesRetainedBytes = 0
    private var compiledRetainedBytes = 0
    private let compiledByteBudget: Int
    private let compilationFailureBudget: Int
    private let themeByteBudget: Int
    private let themeEntryBudget: Int
    private let layoutCache: PreparedProseLayoutCache
    private let compile: DocumentCompiler
    private let prepare: LayoutPreparation
    private(set) var layoutPreparationCount = 0

    var preparedLayoutCacheCountForTesting: Int { layoutCache.countForTesting }
    var compiledDocumentBytesForTesting: Int {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        return compiledRetainedBytes
    }
    var layoutRetainedBytesForTesting: Int { layoutCache.retainedBytesForTesting }
    var oversizedLeaseCountForTesting: Int { layoutCache.oversizedLeaseCountForTesting }
    var preparedThemeCountForTesting: Int {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        return themesByGeneration.count
    }

    override convenience init() {
        self.init(compile: Self.compileWithRust, prepare: Self.prepareWithCoreText)
    }

    init(
        byteBudget: Int = 32 * 1024 * 1024,
        compiledByteBudget: Int = 8 * 1024 * 1024,
        compilationFailureBudget: Int = 128,
        themeByteBudget: Int = 512 * 1024,
        themeEntryBudget: Int = 128,
        compile: @escaping DocumentCompiler,
        prepare: @escaping LayoutPreparation = Self.prepareWithCoreText
    ) {
        self.compile = compile
        self.prepare = prepare
        layoutCache = PreparedProseLayoutCache(byteBudget: byteBudget)
        self.compiledByteBudget = compiledByteBudget
        self.compilationFailureBudget = compilationFailureBudget
        self.themeByteBudget = themeByteBudget
        self.themeEntryBudget = themeEntryBudget
        super.init()
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(didReceiveMemoryWarning),
            name: UIApplication.didReceiveMemoryWarningNotification,
            object: nil
        )
    }

    deinit { NotificationCenter.default.removeObserver(self) }

    func compileDocument(request: ProseViewerRequest) throws -> ViewerDocument {
        let cacheKey = request.compiledCacheKey
        compiledCondition.lock()
        if let document = compiledDocuments[cacheKey] {
            touchCompiled(cacheKey)
            compiledCondition.unlock()
            return document
        }
        if let failure = compilationFailures[cacheKey] {
            touchCompilationFailure(cacheKey)
            compiledCondition.unlock()
            throw failure
        }
        if let compilation = compiledInFlight[cacheKey] {
            while compilation.result == nil { compiledCondition.wait() }
            let result = compilation.result!
            compiledCondition.unlock()
            return try result.get()
        }
        let compilation = Compilation()
        compiledInFlight[cacheKey] = compilation
        compiledCondition.unlock()

        let compileStarted = PreparedProseInstrumentation.now()
        let result = Result<ViewerDocument, Error> {
            let compiled = try compile(request)
            guard compiled.semanticKey.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil else {
                throw ProseViewerError.compiler(
                    domain: "viewer",
                    code: "INVALID_SEMANTIC_KEY",
                    message: "The compiler returned an invalid semantic key."
                )
            }
            return compiled
        }
        PreparedProseInstrumentation.compiled(compileStarted)

        compiledCondition.lock()
        if case let .success(document) = result {
            compiledDocuments[cacheKey] = document
            compiledRetainedBytes += document.retainedBytes
            touchCompiled(cacheKey)
            trimCompiledToBudget()
            PreparedProseInstrumentation.retained(.compiled, scope: "registry", bytes: compiledRetainedBytes)
        } else if case let .failure(error) = result {
            compilationFailures[cacheKey] = error
            touchCompilationFailure(cacheKey)
            trimCompilationFailuresToBudget()
        }
        compilation.result = result
        compiledInFlight.removeValue(forKey: cacheKey)
        compiledCondition.broadcast()
        compiledCondition.unlock()
        return try result.get()
    }

    func measure(
        request: ProseViewerRequest,
        widthPoints: CGFloat,
        scale: CGFloat,
        compiledDocument: ViewerDocument? = nil,
        fabricSurface: FabricSurfaceToken? = nil,
        measurementImageState: ViewerAttachmentRevisionState? = nil
    ) -> PreparedProseLayout {
        guard let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: widthPoints, scale: scale) else {
            if let fabricSurface { layoutCache.releaseLease(for: fabricSurface) }
            return invalidWidthArtifact(
                request: request,
                scale: scale,
                error: .hostContract(message: "A finite positive width is required for prose measurement.")
            )
        }
        let canonicalWidth = ProseLayoutMetrics.canonicalWidth(widthPixels: widthPixels, scale: scale)
        // Yoga can prepare before a component view exists. Reset the matching
        // surface sidecar before Core Text asks for intrinsic fallback.
        let imageMeasurementState = fabricSurface.map {
            FabricAttachmentSidecars.begin($0, semanticIdentity: request.semanticGenerationIdentity)
        } ?? measurementImageState
        do {
            let document = try preparedDocument(
                request: request,
                compiledDocument: compiledDocument,
                fabricSurface: fabricSurface
            )
            let key = layoutKey(for: document, request: request, widthPixels: widthPixels, scale: scale)
            let layout = try layoutCache.value(
                for: key,
                fabricSurface: fabricSurface
            ) {
                let layoutStarted = PreparedProseInstrumentation.now()
                self.lock.lock()
                self.layoutPreparationCount += 1
                self.lock.unlock()
                do {
                    let artifact: PreparedProseLayout
                    if let imageMeasurementState {
                        artifact = try FabricAttachmentSidecars.withMeasurementState(imageMeasurementState) {
                            try self.prepare(document, key, canonicalWidth, scale)
                        }
                    } else {
                        artifact = try self.prepare(document, key, canonicalWidth, scale)
                    }
                    PreparedProseInstrumentation.laidOut(layoutStarted)
                    return artifact
                } catch let error as ProseViewerError {
                    return self.errorArtifact(key: key, width: canonicalWidth, error: error)
                } catch {
                    return self.errorArtifact(
                        key: key,
                        width: canonicalWidth,
                        error: .layout(message: String(describing: error))
                    )
                }
            }
            return layout
        } catch let error as ProseViewerError {
            let layout = cachedErrorArtifact(
                request: request,
                widthPixels: widthPixels,
                scale: scale,
                error: error,
                fabricSurface: fabricSurface
            )
            return layout
        } catch {
            let layout = cachedErrorArtifact(
                request: request,
                widthPixels: widthPixels,
                scale: scale,
                error: .layout(message: String(describing: error)),
                fabricSurface: fabricSurface
            )
            return layout
        }
    }

    @objc(measureSurfaceId:componentTag:sourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:attachmentRevision:nativeFontRevision:nativeFontScale:fontEnvironmentRevision:widthPoints:scale:)
    public func measure(
        surfaceId: Int64,
        componentTag: Int64,
        sourceKind: NSString,
        source: NSString,
        configJSON: NSString,
        themeJSON: NSString?,
        imagePolicyJSON: NSString?,
        imagesEnabled: Bool,
        collapsesWhenEmpty: Bool,
        attachmentRevision: UInt64,
        nativeFontRevision: UInt64,
        nativeFontScale: CGFloat = 1,
        fontEnvironmentRevision: UInt64,
        widthPoints: CGFloat,
        scale: CGFloat
    ) -> CGSize {
        let request = makeRequest(
            sourceKind: sourceKind,
            source: source,
            configJSON: configJSON,
            themeJSON: themeJSON,
            imagePolicyJSON: imagePolicyJSON,
            imagesEnabled: imagesEnabled,
            collapsesWhenEmpty: collapsesWhenEmpty,
            attachmentRevision: attachmentRevision,
            nativeFontRevision: nativeFontRevision,
            nativeFontScale: nativeFontScale,
            fontEnvironmentRevision: fontEnvironmentRevision
        )
        return measure(
            request: request,
            widthPoints: widthPoints,
            scale: scale,
            fabricSurface: FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag)
        ).size
    }

    /// The only Objective-C-visible generation identity boundary. Fabric must
    /// retain this exact SHA-256 key for every later release of its lease and
    /// compiler pin; native callers must not reconstruct it from props.
    @objc(fabricGenerationIdentitySourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:attachmentRevision:nativeFontRevision:nativeFontScale:fontEnvironmentRevision:)
    public func fabricGenerationIdentity(
        sourceKind: NSString,
        source: NSString,
        configJSON: NSString,
        themeJSON: NSString?,
        imagePolicyJSON: NSString?,
        imagesEnabled: Bool,
        collapsesWhenEmpty: Bool,
        attachmentRevision: UInt64,
        nativeFontRevision: UInt64,
        nativeFontScale: CGFloat = 1,
        fontEnvironmentRevision: UInt64
    ) -> NSString {
        makeRequest(
            sourceKind: sourceKind,
            source: source,
            configJSON: configJSON,
            themeJSON: themeJSON,
            imagePolicyJSON: imagePolicyJSON,
            imagesEnabled: imagesEnabled,
            collapsesWhenEmpty: collapsesWhenEmpty,
            attachmentRevision: attachmentRevision,
            nativeFontRevision: nativeFontRevision,
            nativeFontScale: nativeFontScale,
            fontEnvironmentRevision: fontEnvironmentRevision
        ).generationIdentity as NSString
    }

    /// Fabric image publication/error state must use the same canonical
    /// semantic key as direct UIKit. Layout revisions are intentionally
    /// excluded so attachment/font reinstallation preserves that state.
    @objc(fabricSemanticGenerationIdentitySourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:attachmentRevision:nativeFontRevision:nativeFontScale:fontEnvironmentRevision:)
    public func fabricSemanticGenerationIdentity(
        sourceKind: NSString,
        source: NSString,
        configJSON: NSString,
        themeJSON: NSString?,
        imagePolicyJSON: NSString?,
        imagesEnabled: Bool,
        collapsesWhenEmpty: Bool,
        attachmentRevision: UInt64,
        nativeFontRevision: UInt64,
        nativeFontScale: CGFloat = 1,
        fontEnvironmentRevision: UInt64
    ) -> NSString {
        makeRequest(
            sourceKind: sourceKind,
            source: source,
            configJSON: configJSON,
            themeJSON: themeJSON,
            imagePolicyJSON: imagePolicyJSON,
            imagesEnabled: imagesEnabled,
            collapsesWhenEmpty: collapsesWhenEmpty,
            attachmentRevision: attachmentRevision,
            nativeFontRevision: nativeFontRevision,
            nativeFontScale: nativeFontScale,
            fontEnvironmentRevision: fontEnvironmentRevision
        ).semanticGenerationIdentity as NSString
    }

    @objc(installCachedLayoutInDrawingView:surfaceId:componentTag:sourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:attachmentRevision:nativeFontRevision:nativeFontScale:fontEnvironmentRevision:widthPoints:scale:)
    public func installCachedLayout(
        in drawingView: PreparedProseDrawingView,
        surfaceId: Int64,
        componentTag: Int64,
        sourceKind: NSString,
        source: NSString,
        configJSON: NSString,
        themeJSON: NSString?,
        imagePolicyJSON: NSString?,
        imagesEnabled: Bool,
        collapsesWhenEmpty: Bool,
        attachmentRevision: UInt64,
        nativeFontRevision: UInt64,
        nativeFontScale: CGFloat = 1,
        fontEnvironmentRevision: UInt64,
        widthPoints: CGFloat,
        scale: CGFloat
    ) -> Bool {
        let request = makeRequest(
            sourceKind: sourceKind,
            source: source,
            configJSON: configJSON,
            themeJSON: themeJSON,
            imagePolicyJSON: imagePolicyJSON,
            imagesEnabled: imagesEnabled,
            collapsesWhenEmpty: collapsesWhenEmpty,
            attachmentRevision: attachmentRevision,
            nativeFontRevision: nativeFontRevision,
            nativeFontScale: nativeFontScale,
            fontEnvironmentRevision: fontEnvironmentRevision
        )
        guard let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: widthPoints, scale: scale) else {
            return false
        }
        let fabricSurface = FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag)
        if let artifact = layoutCache.acquireForFabricMount(
            surface: fabricSurface,
            generationIdentity: request.generationIdentity,
            widthPixels: widthPixels,
            displayScale: scale
        ) {
            drawingView.install(layout: artifact)
            return true
        }
        return false
    }

    // XCTest and UIKit-only callers have no Fabric owner. Production Fabric
    // mounting always uses the surface/component overload above.
    func installCachedLayout(
        in drawingView: PreparedProseDrawingView,
        sourceKind: NSString,
        source: NSString,
        configJSON: NSString,
        themeJSON: NSString?,
        imagePolicyJSON: NSString?,
        imagesEnabled: Bool,
        collapsesWhenEmpty: Bool,
        attachmentRevision: UInt64,
        nativeFontRevision: UInt64,
        nativeFontScale: CGFloat = 1,
        fontEnvironmentRevision: UInt64,
        widthPoints: CGFloat,
        scale: CGFloat
    ) -> Bool {
        installCachedLayout(
            in: drawingView,
            surfaceId: 0,
            componentTag: 0,
            sourceKind: sourceKind,
            source: source,
            configJSON: configJSON,
            themeJSON: themeJSON,
            imagePolicyJSON: imagePolicyJSON,
            imagesEnabled: imagesEnabled,
            collapsesWhenEmpty: collapsesWhenEmpty,
            attachmentRevision: attachmentRevision,
            nativeFontRevision: nativeFontRevision,
            nativeFontScale: nativeFontScale,
            fontEnvironmentRevision: fontEnvironmentRevision,
            widthPoints: widthPoints,
            scale: scale
        )
    }

    func releaseFabricSurface(_ surface: FabricSurfaceToken) {
        layoutCache.releaseLease(for: surface)
        FabricAttachmentSidecars.remove(surface)
        compiledCondition.lock()
        documentsByFabricGeneration = documentsByFabricGeneration.filter { $0.key.surface != surface }
        failuresByFabricGeneration = failuresByFabricGeneration.filter { $0.key.surface != surface }
        for generation in themeOwners.keys.filter({ $0.surface == surface }) {
            releaseThemeOwnership(for: generation)
        }
        compiledCondition.unlock()
    }

    func releaseFabricGeneration(_ generation: FabricGenerationToken) {
        layoutCache.releaseLease(for: generation.surface, generationIdentity: generation.generationIdentity)
        compiledCondition.lock()
        documentsByFabricGeneration.removeValue(forKey: generation)
        failuresByFabricGeneration.removeValue(forKey: generation)
        releaseThemeOwnership(for: generation)
        compiledCondition.unlock()
    }

    /// Fabric records an owner before it tries to consume Yoga's lease. If
    /// acquisition misses, this deterministic cleanup drops that generation's
    /// lease and compiler pin without touching a newly recycled view's token.
    func releaseFabricMountMiss(_ generation: FabricGenerationToken) {
        releaseFabricGeneration(generation)
    }

    @objc(releaseFabricSurfaceId:componentTag:)
    public func releaseFabricSurface(surfaceId: Int64, componentTag: Int64) {
        releaseFabricSurface(FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag))
    }

    @objc(releaseFabricGenerationSurfaceId:componentTag:generationIdentity:)
    public func releaseFabricGeneration(
        surfaceId: Int64,
        componentTag: Int64,
        generationIdentity: NSString
    ) {
        releaseFabricGeneration(
            FabricGenerationToken(
                surface: FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag),
                generationIdentity: generationIdentity as String
            )
        )
    }

    @objc(releaseFabricMountMissSurfaceId:componentTag:generationIdentity:)
    public func releaseFabricMountMiss(
        surfaceId: Int64,
        componentTag: Int64,
        generationIdentity: NSString
    ) {
        releaseFabricMountMiss(
            FabricGenerationToken(
                surface: FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag),
                generationIdentity: generationIdentity as String
            )
        )
    }

    @objc func didReceiveMemoryWarning() {
        PreparedProseInstrumentation.invalidated(.memoryPressure)
        layoutCache.removeAllUnmounted()
        compiledCondition.lock()
        compiledDocuments.removeAll()
        compiledAccessOrder.removeAll()
        compilationFailures.removeAll()
        compilationFailureAccessOrder.removeAll()
        documentsByFabricGeneration.removeAll()
        failuresByFabricGeneration.removeAll()
        themesByGeneration.removeAll()
        themeAccessOrder.removeAll()
        themeOwners.removeAll()
        themeOwnerCounts.removeAll()
        themesRetainedBytes = 0
        compiledRetainedBytes = 0
        compiledCondition.unlock()
        PreparedProseInstrumentation.retained(.compiled, scope: "registry", bytes: 0)
    }

    private func layoutKey(
        for document: ViewerDocument,
        request: ProseViewerRequest,
        widthPixels: Int,
        scale: CGFloat
    ) -> ProseLayoutKey {
        return ProseLayoutKey(
            semanticKey: document.semanticKey,
            widthPixels: widthPixels,
            themeDigest: request.themeDigest,
            nativeFontRevision: request.nativeFontRevision,
            fontEnvironmentRevision: request.fontEnvironmentRevision,
            displayScale: scale,
            attachmentRevision: request.attachmentRevision,
            generationIdentity: request.generationIdentity,
            semanticGenerationIdentity: request.semanticGenerationIdentity
        )
    }

    private func errorArtifact(
        key: ProseLayoutKey,
        width: CGFloat,
        error: ProseViewerError
    ) -> PreparedProseLayout {
        .error(key: key, width: width, error: error)
    }

    private func cachedErrorArtifact(
        request: ProseViewerRequest,
        widthPixels: Int,
        scale: CGFloat,
        error: ProseViewerError,
        fabricSurface: FabricSurfaceToken?
    ) -> PreparedProseLayout {
        let key = errorLayoutKey(request: request, widthPixels: widthPixels, scale: scale)
        let width = ProseLayoutMetrics.canonicalWidth(widthPixels: widthPixels, scale: scale)
        return (try? layoutCache.value(for: key, fabricSurface: fabricSurface) {
            self.errorArtifact(key: key, width: width, error: error)
        }) ?? errorArtifact(key: key, width: width, error: error)
    }

    private func errorLayoutKey(
        request: ProseViewerRequest,
        widthPixels: Int,
        scale: CGFloat
    ) -> ProseLayoutKey {
        ProseLayoutKey(
            semanticKey: "error:" + request.compiledCacheKey,
            widthPixels: widthPixels,
            themeDigest: request.themeDigest,
            nativeFontRevision: request.nativeFontRevision,
            fontEnvironmentRevision: request.fontEnvironmentRevision,
            displayScale: scale,
            attachmentRevision: request.attachmentRevision,
            generationIdentity: request.generationIdentity,
            semanticGenerationIdentity: request.semanticGenerationIdentity
        )
    }

    private func invalidWidthArtifact(
        request: ProseViewerRequest,
        scale: CGFloat,
        error: ProseViewerError
    ) -> PreparedProseLayout {
        let safeScale = scale.isFinite && scale > 0 ? scale : 1
        return errorArtifact(
            key: errorLayoutKey(request: request, widthPixels: 0, scale: safeScale),
            width: 0,
            error: error
        )
    }

    private func preparedDocument(
        request: ProseViewerRequest,
        compiledDocument: ViewerDocument?,
        fabricSurface: FabricSurfaceToken?
    ) throws -> ViewerDocument {
        let generation = fabricSurface.map {
            FabricGenerationToken(surface: $0, generationIdentity: request.generationIdentity)
        }
        compiledCondition.lock()
        if let generation {
            documentsByFabricGeneration = documentsByFabricGeneration.filter {
                $0.key.surface != generation.surface || $0.key == generation
            }
            failuresByFabricGeneration = failuresByFabricGeneration.filter {
                $0.key.surface != generation.surface || $0.key == generation
            }
            for staleGeneration in themeOwners.keys.filter({
                $0.surface == generation.surface && $0 != generation
            }) {
                releaseThemeOwnership(for: staleGeneration)
            }
        }
        if let generation, let document = documentsByFabricGeneration[generation] {
            compiledCondition.unlock()
            return documentForEmptyContentPolicy(document, request: request).withPreparedTheme(preparedTheme(for: request, generation: generation))
        }
        if let generation, let failure = failuresByFabricGeneration[generation] {
            compiledCondition.unlock()
            throw failure
        }
        compiledCondition.unlock()

        do {
            let document = compiledDocument ?? (try compileDocument(request: request))
            if let generation {
                compiledCondition.lock()
                documentsByFabricGeneration[generation] = document
                compiledCondition.unlock()
            }
            return documentForEmptyContentPolicy(document, request: request).withPreparedTheme(preparedTheme(for: request, generation: generation))
        } catch {
            if let generation {
                compiledCondition.lock()
                failuresByFabricGeneration[generation] = error
                compiledCondition.unlock()
            }
            throw error
        }
    }

    private func documentForEmptyContentPolicy(
        _ compiledDocument: ViewerDocument,
        request: ProseViewerRequest
    ) -> ViewerDocument {
        var document = compiledDocument
        if document.isEmpty && !request.configuration.collapsesWhenEmpty {
            document = ViewerDocument(
                semanticKey: document.semanticKey,
                blocks: document.blocks,
                isEmpty: false,
                retainedBytes: document.retainedBytes
            )
        }
        return document
    }

    /// Parsed paint values are immutable and shared across all width-specific
    /// layouts for the same semantic generation.
    private func preparedTheme(
        for request: ProseViewerRequest,
        generation: FabricGenerationToken?
    ) -> PreparedProseTheme {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        if let generation, themeOwners[generation] == nil {
            themeOwners[generation] = request.generationIdentity
            themeOwnerCounts[request.generationIdentity, default: 0] += 1
        }
        if let theme = themesByGeneration[request.generationIdentity] {
            touchTheme(request.generationIdentity)
            return theme
        }
        let theme = PreparedProseTheme.resolve(
            themeJSON: request.configuration.themeJSON,
            fontScale: request.nativeFontRevision > 0
                ? request.nativeFontScale
                : ViewerFontEnvironment.shared.fontScale(for: request.fontEnvironmentRevision),
            semanticGeneration: request.semanticGenerationIdentity
        )
        themesByGeneration[request.generationIdentity] = theme
        themesRetainedBytes += theme.estimatedRetainedBytes
        touchTheme(request.generationIdentity)
        trimThemesToBudget()
        return theme
    }

    private func touchTheme(_ generationIdentity: String) {
        themeAccessOrder.removeAll { $0 == generationIdentity }
        themeAccessOrder.append(generationIdentity)
    }

    /// Pinned Fabric generations own their exact resolved value until the
    /// matching release callback. Unowned values form a byte/count-bounded
    /// LRU, so background source churn cannot grow this cache without limit.
    private func trimThemesToBudget() {
        while (themesByGeneration.count > themeEntryBudget || themesRetainedBytes > themeByteBudget),
              let oldest = themeAccessOrder.first(where: { themeOwnerCounts[$0, default: 0] == 0 }) {
            themeAccessOrder.removeAll { $0 == oldest }
            if let theme = themesByGeneration.removeValue(forKey: oldest) {
                themesRetainedBytes -= theme.estimatedRetainedBytes
            }
        }
    }

    private func releaseThemeOwnership(for generation: FabricGenerationToken) {
        guard let generationIdentity = themeOwners.removeValue(forKey: generation) else { return }
        let remaining = max(0, (themeOwnerCounts[generationIdentity] ?? 1) - 1)
        if remaining == 0 {
            themeOwnerCounts.removeValue(forKey: generationIdentity)
        } else {
            themeOwnerCounts[generationIdentity] = remaining
        }
        trimThemesToBudget()
    }

    private func makeRequest(
        sourceKind: NSString,
        source: NSString,
        configJSON: NSString,
        themeJSON: NSString?,
        imagePolicyJSON: NSString?,
        imagesEnabled: Bool,
        collapsesWhenEmpty: Bool,
        attachmentRevision: UInt64,
        nativeFontRevision: UInt64,
        nativeFontScale: CGFloat,
        fontEnvironmentRevision: UInt64
    ) -> ProseViewerRequest {
        ProseViewerRequest(
            source: sourceKind == "html" ? .html(source as String) : .json(source as String),
            configuration: ProseViewerConfiguration(
                configJSON: configJSON as String,
                themeJSON: themeJSON as String?,
                imagePolicyJSON: imagePolicyJSON as String?,
                imagesEnabled: imagesEnabled,
                collapsesWhenEmpty: collapsesWhenEmpty
            ),
            nativeFontRevision: nativeFontRevision,
            nativeFontScale: nativeFontScale,
            fontEnvironmentRevision: fontEnvironmentRevision,
            attachmentRevision: attachmentRevision
        )
    }

    private func touchCompiled(_ cacheKey: String) {
        compiledAccessOrder.removeAll { $0 == cacheKey }
        compiledAccessOrder.append(cacheKey)
    }

    private func trimCompiledToBudget() {
        while compiledRetainedBytes > compiledByteBudget, let oldest = compiledAccessOrder.first {
            compiledAccessOrder.removeFirst()
            if let removed = compiledDocuments.removeValue(forKey: oldest) {
                compiledRetainedBytes -= removed.retainedBytes
            }
        }
        PreparedProseInstrumentation.retained(.compiled, scope: "registry", bytes: compiledRetainedBytes)
    }

    private func touchCompilationFailure(_ cacheKey: String) {
        compilationFailureAccessOrder.removeAll { $0 == cacheKey }
        compilationFailureAccessOrder.append(cacheKey)
    }

    private func trimCompilationFailuresToBudget() {
        while compilationFailures.count > compilationFailureBudget,
              let oldest = compilationFailureAccessOrder.first {
            compilationFailureAccessOrder.removeFirst()
            compilationFailures.removeValue(forKey: oldest)
        }
    }

    private static func prepareWithCoreText(
        document: ViewerDocument,
        key: ProseLayoutKey,
        widthPoints: CGFloat,
        scale: CGFloat
    ) throws -> PreparedProseLayout {
        try CoreTextProseLayoutEngine().prepare(
            document: document,
            key: key,
            widthPoints: widthPoints,
            displayScale: scale,
            semanticGenerationIdentity: key.semanticGenerationIdentity
        )
    }

    private static func compileWithRust(request: ProseViewerRequest) throws -> ViewerDocument {
        let result = viewerCompile(
            request: FfiViewerCompileRequest(
                sourceKind: request.source.kind,
                source: request.source.value,
                configJson: request.configuration.configJSON,
                imagesEnabled: request.configuration.imagesEnabled,
                mentionPrefix: request.mentionPrefix
            )
        )
        if let error = result.error {
            throw ProseViewerError.compiler(domain: error.domain, code: error.code, message: error.message)
        }
        guard let compiled = result.value else {
            throw ProseViewerError.compiler(
                domain: "viewer",
                code: "MISSING_COMPILED_DOCUMENT",
                message: "The compiler returned neither a document nor an error."
            )
        }
        return try ViewerDocument(compiled: compiled)
    }
}
