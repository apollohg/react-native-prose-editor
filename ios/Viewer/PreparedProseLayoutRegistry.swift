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

    private struct FabricMeasurementCancelled: Error {}

    /// This record exists only while a measurement is executing. A release
    /// marks it cancelled so post-layout ownership cannot be republished;
    /// `endFabricMeasure` removes it when the final in-flight callback exits.
    private struct FabricMeasurementState {
        var count = 0
        var cancelled = false
    }

    private struct FabricLeaseState {
        var active = true
        /// `nil` permits bounded pre-commit Yoga work before a component view
        /// has identified the canonical state/props generation.
        var permittedGenerationIdentity: String?
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
    private var fabricMeasurementsInFlight: [FabricGenerationToken: FabricMeasurementState] = [:]
    /// Bounded to state-family handles that still have Fabric lifecycle
    /// ownership. Terminal lifecycle cleanup removes entries entirely.
    private var fabricLeaseStates: [FabricLeaseOwner: FabricLeaseState] = [:]
    private var fabricOwnershipRevisions: [FabricGenerationToken: UInt64] = [:]
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

    // XCTest-only lock-step hook for the mount-miss/measure ownership race.
    // It is deliberately absent from release binaries.
#if DEBUG
    var fabricMountMissAfterExactLeaseCleanupForTesting: (() -> Void)?
    var fabricSidecarRegisteredForTesting: (() -> Void)?
#endif

    var preparedLayoutCacheCountForTesting: Int { layoutCache.countForTesting }
    var compiledDocumentBytesForTesting: Int {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        return compiledRetainedBytes
    }
    var layoutRetainedBytesForTesting: Int { layoutCache.retainedBytesForTesting }
    var oversizedLeaseCountForTesting: Int { layoutCache.oversizedLeaseCountForTesting }
    var pendingFabricLeaseCountForTesting: Int { layoutCache.pendingLeaseCountForTesting }
    var mountedFabricLeaseCountForTesting: Int { layoutCache.mountedLeaseCountForTesting }
    var fabricLeaseCountForTesting: Int { layoutCache.leaseCountForTesting }
    func permittedFabricGenerationForTesting(_ owner: FabricLeaseOwner) -> String? {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        return fabricLeaseStates[owner]?.permittedGenerationIdentity
    }
    func hasFabricGenerationOwnershipForTesting(_ generation: FabricGenerationToken) -> Bool {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        if documentsByFabricGeneration[generation] != nil {
            return themeOwners[generation] != nil
        }
        return failuresByFabricGeneration[generation] != nil
    }
    var preparedThemeCountForTesting: Int {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        return themesByGeneration.count
    }
    func hasFabricThemeOwnershipForTesting(_ generation: FabricGenerationToken) -> Bool {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        return themeOwners[generation] != nil
    }

    override convenience init() {
        self.init(compile: Self.compileWithRust, prepare: Self.prepareWithCoreText)
    }

    init(
        byteBudget: Int = 32 * 1024 * 1024,
        pendingLeaseBudget: Int = 256,
        compiledByteBudget: Int = 8 * 1024 * 1024,
        compilationFailureBudget: Int = 128,
        themeByteBudget: Int = 512 * 1024,
        themeEntryBudget: Int = 128,
        compile: @escaping DocumentCompiler,
        prepare: @escaping LayoutPreparation = PreparedProseLayoutRegistry.prepareWithCoreText
    ) {
        self.compile = compile
        self.prepare = prepare
        layoutCache = PreparedProseLayoutCache(
            byteBudget: byteBudget,
            pendingLeaseBudget: pendingLeaseBudget
        )
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
        PreparedProseInstrumentation.compiled(compileStarted, generation: request.generationIdentity)

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
        fabricLeaseHandle: UInt64 = 1,
        measurementImageState: ViewerAttachmentRevisionState? = nil
    ) -> PreparedProseLayout {
        let generation: FabricGenerationToken?
        if let fabricSurface, fabricLeaseHandle != 0 {
            generation = FabricGenerationToken(
                surface: fabricSurface,
                generationIdentity: request.generationIdentity,
                leaseHandle: fabricLeaseHandle
            )
        } else {
            generation = nil
        }
        guard let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: widthPoints, scale: scale) else {
            if let generation {
                releaseFabricInvalidMeasurement(
                    generation,
                    widthPoints: widthPoints,
                    scale: scale
                )
            }
            return invalidWidthArtifact(
                request: request,
                scale: scale,
                error: .hostContract(message: "A finite positive width is required for prose measurement.")
            )
        }
        let beganFabricMeasure = generation.map(beginFabricMeasure) ?? false
        guard generation == nil || beganFabricMeasure else {
            return invalidWidthArtifact(
                request: request,
                scale: scale,
                error: .layout(message: "A retired or superseded Fabric lease attempted another measurement.")
            )
        }
        defer {
            if let generation, beganFabricMeasure { endFabricMeasure(generation) }
        }
        let canonicalWidth = ProseLayoutMetrics.canonicalWidth(widthPixels: widthPixels, scale: scale)
        // Yoga can prepare before a component view exists. Reset the matching
        // surface sidecar before Core Text asks for intrinsic fallback.
        let imageMeasurementState = generation.flatMap { token -> ViewerAttachmentRevisionState? in
            guard isFabricLeaseActive(token) else { return nil }
            let state = FabricAttachmentSidecars.begin(
                token.surface,
                leaseHandle: fabricLeaseHandle,
                semanticIdentity: request.semanticGenerationIdentity
            )
#if DEBUG
            fabricSidecarRegisteredForTesting?()
#endif
            // Release may race validation and create a stale exact sidecar.
            // Remove only this handle; a replacement family has another one.
            guard isFabricLeaseActive(token) else {
                FabricAttachmentSidecars.remove(token.surface, leaseHandle: token.leaseHandle)
                return nil
            }
            return state
        } ?? measurementImageState
        if generation != nil && imageMeasurementState == nil {
            return invalidWidthArtifact(
                request: request,
                scale: scale,
                error: .layout(message: "A superseded Fabric generation attempted sidecar publication.")
            )
        }
        do {
            if let generation, !isFabricLeaseActive(generation) {
                throw FabricMeasurementCancelled()
            }
            let document = try preparedDocument(
                request: request,
                compiledDocument: compiledDocument,
                fabricGeneration: generation
            )
            let key = layoutKey(for: document, request: request, widthPixels: widthPixels, scale: scale)
            if let generation, !isFabricLeaseActive(generation) {
                throw FabricMeasurementCancelled()
            }
            let layout = try layoutCache.value(
                for: key,
                fabricSurface: fabricSurface,
                fabricLeaseHandle: generation?.leaseHandle,
                shouldCreateFabricLease: {
                    guard let generation else { return true }
                    return self.isFabricLeaseActive(generation)
                }
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
                    PreparedProseInstrumentation.laidOut(layoutStarted, generation: request.generationIdentity)
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
            if let generation,
               !retainFabricGenerationOwnership(
                    generation,
                    document: document,
                    request: request
               ) {
                discardCancelledFabricMeasurement(
                    generation,
                    widthPixels: widthPixels,
                    scale: scale
                )
            }
            return layout
        } catch is FabricMeasurementCancelled {
            if let generation { discardCancelledFabricMeasurement(generation, widthPixels: widthPixels, scale: scale) }
            return invalidWidthArtifact(
                request: request,
                scale: scale,
                error: .layout(message: "A released Fabric measurement completed after its lifecycle ended.")
            )
        } catch let error as ProseViewerError {
            let layout = cachedErrorArtifact(
                request: request,
                widthPixels: widthPixels,
                scale: scale,
                error: error,
                fabricSurface: fabricSurface,
                fabricGeneration: generation,
                fabricLeaseHandle: generation?.leaseHandle
            )
            if let generation,
               !retainFabricGenerationFailure(generation, error: error) {
                discardCancelledFabricMeasurement(
                    generation,
                    widthPixels: widthPixels,
                    scale: scale
                )
            }
            return layout
        } catch {
            let layout = cachedErrorArtifact(
                request: request,
                widthPixels: widthPixels,
                scale: scale,
                error: .layout(message: String(describing: error)),
                fabricSurface: fabricSurface,
                fabricGeneration: generation,
                fabricLeaseHandle: generation?.leaseHandle
            )
            if let generation,
               !retainFabricGenerationFailure(generation, error: error) {
                discardCancelledFabricMeasurement(
                    generation,
                    widthPixels: widthPixels,
                    scale: scale
                )
            }
            return layout
        }
    }

    @objc(measureSurfaceId:componentTag:leaseHandle:sourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:attachmentRevision:nativeFontRevision:nativeFontScale:fontEnvironmentRevision:widthPoints:scale:)
    public func measure(
        surfaceId: Int64,
        componentTag: Int64,
        leaseHandle: UInt64,
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
            fabricSurface: leaseHandle == 0
                ? nil
                : FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag),
            fabricLeaseHandle: leaseHandle
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

    @objc(installCachedLayoutInDrawingView:surfaceId:componentTag:leaseHandle:sourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:attachmentRevision:nativeFontRevision:nativeFontScale:fontEnvironmentRevision:widthPoints:scale:)
    public func installCachedLayout(
        in drawingView: PreparedProseDrawingView,
        surfaceId: Int64,
        componentTag: Int64,
        leaseHandle: UInt64,
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
            displayScale: scale,
            leaseHandle: leaseHandle
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
            leaseHandle: 0,
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
        let cachedGenerations = layoutCache.fabricGenerations(for: surface)
        compiledCondition.lock()
        let generations = Set(fabricMeasurementsInFlight.keys)
            .union(fabricOwnershipRevisions.keys)
            .union(documentsByFabricGeneration.keys)
            .union(failuresByFabricGeneration.keys)
            .union(cachedGenerations)
            .filter { $0.surface == surface }
        for generation in generations {
            cancelFabricMeasurementLocked(generation)
        }
        // Keep inactive state-family records until their C++ terminal guards
        // unregister. Delayed Yoga work must observe cancellation, not create
        // a fresh owner after surface teardown.
        for owner in fabricLeaseStates.keys.filter({ $0.surface == surface }) {
            var state = fabricLeaseStates[owner]!
            state.active = false
            fabricLeaseStates[owner] = state
        }
        // Keep cancelled in-flight callbacks until their defer path exits.
        // Removing them here would let an already-running stale measure pin
        // compiler/theme ownership again after surface shutdown.
        fabricOwnershipRevisions = fabricOwnershipRevisions.filter { $0.key.surface != surface }
        documentsByFabricGeneration = documentsByFabricGeneration.filter { $0.key.surface != surface }
        failuresByFabricGeneration = failuresByFabricGeneration.filter { $0.key.surface != surface }
        for generation in themeOwners.keys.filter({ $0.surface == surface }) {
            releaseThemeOwnership(for: generation)
        }
        compiledCondition.unlock()
        layoutCache.releaseLease(for: surface)
        FabricAttachmentSidecars.remove(surface)
    }

    func releaseFabricGeneration(_ generation: FabricGenerationToken) {
        compiledCondition.lock()
        guard isFabricLeaseActiveLocked(generation) else {
            compiledCondition.unlock()
            return
        }
        cancelFabricMeasurementLocked(generation)
        fabricOwnershipRevisions.removeValue(forKey: generation)
        documentsByFabricGeneration.removeValue(forKey: generation)
        failuresByFabricGeneration.removeValue(forKey: generation)
        releaseThemeOwnership(for: generation)
        compiledCondition.unlock()
        layoutCache.releaseLease(
            for: generation.surface,
            generationIdentity: generation.generationIdentity,
            leaseHandle: generation.leaseHandle
        )
        FabricAttachmentSidecars.remove(generation.surface, leaseHandle: generation.leaseHandle)
    }

    /// Commits the single generation allowed to publish for this state-family
    /// lease. Existing G2 work remains valid; all prior G1 work is cancelled
    /// before it can recreate pins, sidecars, or a pending handoff.
    func activateFabricGeneration(_ generation: FabricGenerationToken) {
        let owner = FabricLeaseOwner(surface: generation.surface, leaseHandle: generation.leaseHandle)
        compiledCondition.lock()
        guard var lease = fabricLeaseStates[owner], lease.active else {
            compiledCondition.unlock()
            return
        }
        lease.permittedGenerationIdentity = generation.generationIdentity
        fabricLeaseStates[owner] = lease
        let stale = Set(fabricMeasurementsInFlight.keys)
            .union(fabricOwnershipRevisions.keys)
            .union(documentsByFabricGeneration.keys)
            .union(failuresByFabricGeneration.keys)
            .union(themeOwners.keys)
            .filter {
                FabricLeaseOwner(surface: $0.surface, leaseHandle: $0.leaseHandle) == owner &&
                    $0 != generation
            }
        for token in stale {
            cancelFabricMeasurementLocked(token)
            fabricOwnershipRevisions.removeValue(forKey: token)
            documentsByFabricGeneration.removeValue(forKey: token)
            failuresByFabricGeneration.removeValue(forKey: token)
            releaseThemeOwnership(for: token)
        }
        compiledCondition.unlock()
        // Do not hold the registry lock while entering cache/sidecar locks.
        // A mounted G1 remains displayed until G2 consumes its handoff.
        layoutCache.activateFabricGeneration(
            surface: generation.surface,
            generationIdentity: generation.generationIdentity,
            leaseHandle: generation.leaseHandle
        )
    }

    /// Called by the Objective-C++ measurement bridge when a state-owned
    /// lifecycle is cancelled while its synchronous registry call is running.
    /// It intentionally identifies ownership by the opaque handle alone: the
    /// stale callback may have completed after a new revision changed the
    /// generation digest, but it can never affect another handle.
    @objc(releaseFabricLeaseSurfaceId:componentTag:leaseHandle:)
    public func releaseFabricLease(
        surfaceId: Int64,
        componentTag: Int64,
        leaseHandle: UInt64
    ) {
        guard leaseHandle != 0 else { return }
        let surface = FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag)
        compiledCondition.lock()
        let owner = FabricLeaseOwner(surface: surface, leaseHandle: leaseHandle)
        // Terminal family cleanup is the only path that drops this bounded
        // inactive record after marking it cancelled for concurrent readers.
        if var state = fabricLeaseStates[owner] {
            state.active = false
            fabricLeaseStates[owner] = state
        }
        fabricLeaseStates.removeValue(forKey: owner)
        let generations = Set(fabricMeasurementsInFlight.keys)
            .union(fabricOwnershipRevisions.keys)
            .union(documentsByFabricGeneration.keys)
            .union(failuresByFabricGeneration.keys)
            .filter { $0.surface == surface && $0.leaseHandle == leaseHandle }
        for generation in generations {
            cancelFabricMeasurementLocked(generation)
            fabricOwnershipRevisions.removeValue(forKey: generation)
            documentsByFabricGeneration.removeValue(forKey: generation)
            failuresByFabricGeneration.removeValue(forKey: generation)
            releaseThemeOwnership(for: generation)
        }
        compiledCondition.unlock()
        layoutCache.releaseLease(for: surface, leaseHandle: leaseHandle)
        FabricAttachmentSidecars.remove(surface, leaseHandle: leaseHandle)
    }

    /// State-family bridge is the only owner creation path. Measurement and
    /// activation reject absent/inactive records so delayed Yoga cannot revive
    /// a released surface.
    @objc(registerFabricLeaseSurfaceId:componentTag:leaseHandle:)
    public func registerFabricLease(
        surfaceId: Int64,
        componentTag: Int64,
        leaseHandle: UInt64
    ) {
        guard leaseHandle != 0 else { return }
        let owner = FabricLeaseOwner(
            surface: FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag),
            leaseHandle: leaseHandle
        )
        compiledCondition.lock()
        if fabricLeaseStates[owner] == nil {
            fabricLeaseStates[owner] = FabricLeaseState()
        }
        compiledCondition.unlock()
    }

    /// An invalid Yoga pass is not a lifecycle event. It may retire only an
    /// exact physical pending handoff; an actually invalid width has no such
    /// key, so it must not infer one from a prior measurement. In particular,
    /// it preserves mounted rendering, compiler/theme pins, permitted
    /// generation ownership, and attachment sidecar state for a later valid
    /// measure in this same state-family lifecycle.
    private func releaseFabricInvalidMeasurement(
        _ generation: FabricGenerationToken,
        widthPoints: CGFloat,
        scale: CGFloat
    ) {
        guard let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: widthPoints, scale: scale) else {
            return
        }
        compiledCondition.lock()
        guard isFabricLeaseActiveLocked(generation) else {
            compiledCondition.unlock()
            return
        }
        compiledCondition.unlock()
        let hasSurvivingLease = layoutCache.releasePendingLease(
            for: generation.surface,
            generationIdentity: generation.generationIdentity,
            widthPixels: widthPixels,
            displayScale: scale,
            leaseHandle: generation.leaseHandle
        )
        guard !hasSurvivingLease else { return }

        // This branch is defensive: current validation reaches this helper
        // only when no physical key exists. If validation evolves, compiler
        // pins can be released only after the exact pending handoff is gone
        // and no displayed/in-flight ownership remains. Sidecars are never
        // reset here because they may back the currently displayed artifact.
        compiledCondition.lock()
        guard (fabricMeasurementsInFlight[generation]?.count ?? 0) == 0,
              isFabricLeaseActiveLocked(generation)
        else {
            compiledCondition.unlock()
            return
        }
        documentsByFabricGeneration.removeValue(forKey: generation)
        failuresByFabricGeneration.removeValue(forKey: generation)
        releaseThemeOwnership(for: generation)
        compiledCondition.unlock()
    }

    /// Fabric records an owner before it tries to consume Yoga's lease. A
    /// stale mount callback may only retire its exact unmounted handoff; a
    /// newer width or a displayed mounted artifact for the same generation
    /// must remain owned. Release the compiler pin only when no lease for the
    /// surface/generation survives that exact cleanup.
    func releaseFabricMountMiss(
        _ generation: FabricGenerationToken,
        widthPoints: CGFloat,
        scale: CGFloat
    ) {
        guard let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: widthPoints, scale: scale) else { return }
        compiledCondition.lock()
        guard isFabricLeaseActiveLocked(generation) else {
            compiledCondition.unlock()
            return
        }
        let ownershipRevision = fabricOwnershipRevisions[generation, default: 0]
        compiledCondition.unlock()
        let hasSurvivingLease = layoutCache.releasePendingLease(
            for: generation.surface,
            generationIdentity: generation.generationIdentity,
            widthPixels: widthPixels,
            displayScale: scale,
            leaseHandle: generation.leaseHandle
        )
#if DEBUG
        fabricMountMissAfterExactLeaseCleanupForTesting?()
#endif

        // A measure already in preparation will retain ownership once it
        // reaches its post-cache decision. Do not let a stale mount callback
        // clear that future pin. Conversely, a later measure begins after we
        // release the condition and establishes its own ownership afresh.
        compiledCondition.lock()
        guard !hasSurvivingLease,
              (fabricMeasurementsInFlight[generation]?.count ?? 0) == 0,
              fabricOwnershipRevisions[generation, default: 0] == ownershipRevision,
              isFabricLeaseActiveLocked(generation)
        else {
            compiledCondition.unlock()
            return
        }
        documentsByFabricGeneration.removeValue(forKey: generation)
        failuresByFabricGeneration.removeValue(forKey: generation)
        releaseThemeOwnership(for: generation)
        compiledCondition.unlock()
    }

    func registerDirectMounted(_ owner: String, layout: PreparedProseLayout) {
        layoutCache.registerDirectMount(owner, layout: layout)
    }

    func releaseDirectMounted(_ owner: String) {
        layoutCache.releaseDirectMount(owner)
    }

    @objc(releaseFabricSurfaceId:componentTag:)
    public func releaseFabricSurface(surfaceId: Int64, componentTag: Int64) {
        releaseFabricSurface(FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag))
    }

    @objc(releaseFabricGenerationSurfaceId:componentTag:generationIdentity:leaseHandle:)
    public func releaseFabricGeneration(
        surfaceId: Int64,
        componentTag: Int64,
        generationIdentity: NSString,
        leaseHandle: UInt64
    ) {
        releaseFabricGeneration(
            FabricGenerationToken(
                surface: FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag),
                generationIdentity: generationIdentity as String,
                leaseHandle: leaseHandle
            )
        )
    }

    @objc(activateFabricGenerationSurfaceId:componentTag:generationIdentity:leaseHandle:)
    public func activateFabricGeneration(
        surfaceId: Int64,
        componentTag: Int64,
        generationIdentity: NSString,
        leaseHandle: UInt64
    ) {
        guard leaseHandle != 0 else { return }
        activateFabricGeneration(
            FabricGenerationToken(
                surface: FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag),
                generationIdentity: generationIdentity as String,
                leaseHandle: leaseHandle
            )
        )
    }

    @objc(releaseFabricMountMissSurfaceId:componentTag:generationIdentity:leaseHandle:widthPoints:scale:)
    public func releaseFabricMountMiss(
        surfaceId: Int64,
        componentTag: Int64,
        generationIdentity: NSString,
        leaseHandle: UInt64,
        widthPoints: CGFloat,
        scale: CGFloat
    ) {
        releaseFabricMountMiss(
            FabricGenerationToken(
                surface: FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag),
                generationIdentity: generationIdentity as String,
                leaseHandle: leaseHandle
            ),
            widthPoints: widthPoints,
            scale: scale
        )
    }

    @objc func didReceiveMemoryWarning() {
        PreparedProseInstrumentation.invalidated(.memoryPressure)
        layoutCache.removeAllUnmounted()
        compiledCondition.lock()
        fabricOwnershipRevisions.removeAll()
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

    private func beginFabricMeasure(_ generation: FabricGenerationToken) -> Bool {
        compiledCondition.lock()
        guard isFabricLeaseActiveLocked(generation) else {
            compiledCondition.unlock()
            return false
        }
        var state = fabricMeasurementsInFlight[generation] ?? FabricMeasurementState()
        guard !state.cancelled else {
            compiledCondition.unlock()
            return false
        }
        state.count += 1
        fabricMeasurementsInFlight[generation] = state
        compiledCondition.unlock()
        return true
    }

    private func endFabricMeasure(_ generation: FabricGenerationToken) {
        compiledCondition.lock()
        guard var state = fabricMeasurementsInFlight[generation] else {
            compiledCondition.unlock()
            return
        }
        let remaining = max(0, state.count - 1)
        if remaining == 0 {
            fabricMeasurementsInFlight.removeValue(forKey: generation)
        } else {
            state.count = remaining
            fabricMeasurementsInFlight[generation] = state
        }
        compiledCondition.unlock()
    }

    private func isFabricLeaseActive(_ generation: FabricGenerationToken) -> Bool {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        return isFabricLeaseActiveLocked(generation)
    }

    /// Caller must hold `compiledCondition`.
    private func isFabricLeaseActiveLocked(_ generation: FabricGenerationToken) -> Bool {
        guard generation.leaseHandle != 0,
              let lease = fabricLeaseStates[FabricLeaseOwner(
                  surface: generation.surface,
                  leaseHandle: generation.leaseHandle
              )],
              lease.active,
              lease.permittedGenerationIdentity == nil ||
                  lease.permittedGenerationIdentity == generation.generationIdentity
        else { return false }
        return !(fabricMeasurementsInFlight[generation]?.cancelled ?? false)
    }

    /// Caller must hold `compiledCondition`.
    private func cancelFabricMeasurementLocked(_ generation: FabricGenerationToken) {
        guard var state = fabricMeasurementsInFlight[generation] else { return }
        state.cancelled = true
        fabricMeasurementsInFlight[generation] = state
    }

    private func retireStaleFabricLease(
        _ generation: FabricGenerationToken,
        widthPixels: Int,
        scale: CGFloat
    ) {
        _ = layoutCache.releasePendingLease(
            for: generation.surface,
            generationIdentity: generation.generationIdentity,
            widthPixels: widthPixels,
            displayScale: scale,
            leaseHandle: generation.leaseHandle
        )
    }

    private func discardCancelledFabricMeasurement(
        _ generation: FabricGenerationToken,
        widthPixels: Int,
        scale: CGFloat
    ) {
        retireStaleFabricLease(generation, widthPixels: widthPixels, scale: scale)
        // Release can win after sidecar begin but before cache publication.
        // This exact handle cannot clear a concurrently-created replacement.
        FabricAttachmentSidecars.remove(
            generation.surface,
            leaseHandle: generation.leaseHandle
        )
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
        fabricSurface: FabricSurfaceToken?,
        fabricGeneration: FabricGenerationToken?,
        fabricLeaseHandle: UInt64?
    ) -> PreparedProseLayout {
        let key = errorLayoutKey(request: request, widthPixels: widthPixels, scale: scale)
        let width = ProseLayoutMetrics.canonicalWidth(widthPixels: widthPixels, scale: scale)
        return (try? layoutCache.value(
            for: key,
            fabricSurface: fabricSurface,
            fabricLeaseHandle: fabricLeaseHandle,
            shouldCreateFabricLease: {
                guard let fabricGeneration else { return true }
                return self.isFabricLeaseActive(fabricGeneration)
            }
        ) {
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
        fabricGeneration: FabricGenerationToken?
    ) throws -> ViewerDocument {
        compiledCondition.lock()
        if let fabricGeneration,
           !isFabricLeaseActiveLocked(fabricGeneration) {
            compiledCondition.unlock()
            throw FabricMeasurementCancelled()
        }
        if let fabricGeneration, let document = documentsByFabricGeneration[fabricGeneration] {
            compiledCondition.unlock()
            return documentForEmptyContentPolicy(document, request: request)
                .withPreparedTheme(try preparedTheme(for: request, generation: fabricGeneration))
        }
        if let fabricGeneration, let failure = failuresByFabricGeneration[fabricGeneration] {
            compiledCondition.unlock()
            throw failure
        }
        compiledCondition.unlock()

        do {
            let document: ViewerDocument
            if let compiledDocument {
                document = compiledDocument
            } else {
                document = try compileDocument(request: request)
            }
            if let fabricGeneration {
                compiledCondition.lock()
                guard isFabricLeaseActiveLocked(fabricGeneration) else {
                    compiledCondition.unlock()
                    throw FabricMeasurementCancelled()
                }
                documentsByFabricGeneration[fabricGeneration] = document
                compiledCondition.unlock()
            }
            return documentForEmptyContentPolicy(document, request: request)
                .withPreparedTheme(try preparedTheme(for: request, generation: fabricGeneration))
        } catch {
            if error is FabricMeasurementCancelled {
                throw error
            }
            if let fabricGeneration {
                compiledCondition.lock()
                if isFabricLeaseActiveLocked(fabricGeneration) {
                    failuresByFabricGeneration[fabricGeneration] = error
                }
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
    ) throws -> PreparedProseTheme {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        if let generation,
           !isFabricLeaseActiveLocked(generation) {
            throw FabricMeasurementCancelled()
        }
        return preparedThemeLocked(for: request, generation: generation)
    }

    private func retainFabricGenerationOwnership(
        _ generation: FabricGenerationToken,
        document: ViewerDocument,
        request: ProseViewerRequest
    ) -> Bool {
        compiledCondition.lock()
        guard isFabricLeaseActiveLocked(generation) else {
            compiledCondition.unlock()
            return false
        }
        documentsByFabricGeneration[generation] = document
        failuresByFabricGeneration.removeValue(forKey: generation)
        _ = preparedThemeLocked(for: request, generation: generation)
        fabricOwnershipRevisions[generation, default: 0] &+= 1
        compiledCondition.unlock()
        return true
    }

    private func retainFabricGenerationFailure(
        _ generation: FabricGenerationToken,
        error: Error
    ) -> Bool {
        compiledCondition.lock()
        guard isFabricLeaseActiveLocked(generation) else {
            compiledCondition.unlock()
            return false
        }
        failuresByFabricGeneration[generation] = error
        documentsByFabricGeneration.removeValue(forKey: generation)
        fabricOwnershipRevisions[generation, default: 0] &+= 1
        compiledCondition.unlock()
        return true
    }

    /// Caller must hold `compiledCondition`.
    private func preparedThemeLocked(
        for request: ProseViewerRequest,
        generation: FabricGenerationToken?
    ) -> PreparedProseTheme {
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
