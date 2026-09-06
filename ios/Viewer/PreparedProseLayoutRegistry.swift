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

    struct FabricMeasurementCancelled: Error {}

    /// This record exists only while a measurement is executing. A release
    /// marks it cancelled so post-layout ownership cannot be republished;
    /// `endFabricMeasure` removes it when the final in-flight callback exits.
    struct FabricMeasurementState {
        var count = 0
        var cancelled = false
    }

    struct FabricLeaseState {
        var active = true
        /// `nil` permits bounded pre-commit Yoga work before a component view
        /// has identified the canonical state/props generation.
        var permittedGenerationIdentity: String?
    }

    let lock = NSLock()
    let compiledCondition = NSCondition()
    var compiledDocuments: [String: ViewerDocument] = [:]
    /// Lazy access generations keep repeated compiled-document hits O(1).
    /// Stale tokens are skipped during eviction and bounded by compaction.
    var compiledAccessOrder: [(key: String, generation: UInt64)] = []
    var compiledAccessGenerations: [String: UInt64] = [:]
    var compiledAccessOrderHead = 0
    var nextCompiledAccessGeneration: UInt64 = 0
    private var compiledInFlight: [String: Compilation] = [:]
    var compilationFailures: [String: Error] = [:]
    var compilationFailureAccessOrder: [String] = []
    var documentsByFabricGeneration: [FabricGenerationToken: ViewerDocument] = [:]
    var failuresByFabricGeneration: [FabricGenerationToken: Error] = [:]
    var themesByGeneration: [String: PreparedProseTheme] = [:]
    var themeAccessOrder: [String] = []
    var themeOwners: [FabricGenerationToken: String] = [:]
    var themeOwnerCounts: [String: Int] = [:]
    var fabricMeasurementsInFlight: [FabricGenerationToken: FabricMeasurementState] = [:]
    /// Bounded to state-family handles that still have Fabric lifecycle
    /// ownership. Terminal lifecycle cleanup removes entries entirely.
    var fabricLeaseStates: [FabricLeaseOwner: FabricLeaseState] = [:]
    var fabricOwnershipRevisions: [FabricGenerationToken: UInt64] = [:]
    var themesRetainedBytes = 0
    var compiledRetainedBytes = 0
    let compiledByteBudget: Int
    let compilationFailureBudget: Int
    let themeByteBudget: Int
    let themeEntryBudget: Int
    let layoutCache: PreparedProseLayoutCache
    private let compile: DocumentCompiler
    let prepare: LayoutPreparation
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
    struct BenchmarkResidentCensus {
        let keys: [ProseLayoutKey]
        let count: Int
        let digest: String
    }
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
        compiledByteBudget: Int = 8 * 1024 * 1024,
        compilationFailureBudget: Int = 128,
        themeByteBudget: Int = 512 * 1024,
        themeEntryBudget: Int = 128,
        compile: @escaping DocumentCompiler,
        prepare: @escaping LayoutPreparation = PreparedProseLayoutRegistry.prepareWithCoreText
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
        let ownedGeneration = beganFabricMeasure ? generation : nil
        defer {
            if let generation, beganFabricMeasure { endFabricMeasure(generation) }
        }
        let canonicalWidth = ProseLayoutMetrics.canonicalWidth(widthPixels: widthPixels, scale: scale)
        // Yoga can prepare before a component view exists. Reset the matching
        // surface sidecar before Core Text asks for intrinsic fallback.
        let imageMeasurementState = ownedGeneration.flatMap { token -> ViewerAttachmentRevisionState? in
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
        } ?? measurementImageState ?? generation.map { _ in
            let state = ViewerAttachmentRevisionState()
            _ = state.beginSemanticGeneration(request.semanticGenerationIdentity)
            return state
        }
        do {
            if let ownedGeneration, !isFabricLeaseActive(ownedGeneration) {
                throw FabricMeasurementCancelled()
            }
            let document = try preparedDocument(
                request: request,
                compiledDocument: compiledDocument,
                fabricGeneration: ownedGeneration
            )
            let key = layoutKey(for: document, request: request, widthPixels: widthPixels, scale: scale)
            if let ownedGeneration, !isFabricLeaseActive(ownedGeneration) {
                throw FabricMeasurementCancelled()
            }
            let layout = try layoutCache.value(
                for: key,
                fabricSurface: ownedGeneration?.surface,
                fabricLeaseHandle: ownedGeneration?.leaseHandle,
                shouldCreateFabricLease: {
                    guard let ownedGeneration else { return true }
                    return self.isFabricLeaseActive(ownedGeneration)
                }
            ) {
                let layoutStarted = PreparedProseInstrumentation.now()
                self.lock.lock()
                self.layoutPreparationCount += 1
                self.lock.unlock()
                do {
                    var prepared: Result<PreparedProseLayout, Error>!
                    request.appearance.traits.performAsCurrent {
                        prepared = Result {
                            if let imageMeasurementState {
                                return try FabricAttachmentSidecars.withMeasurementState(imageMeasurementState) {
                                    try self.prepare(document, key, canonicalWidth, scale)
                                }
                            }
                            return try self.prepare(document, key, canonicalWidth, scale)
                        }
                    }
                    let artifact = try prepared.get()
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
            if let generation = ownedGeneration,
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
            if let ownedGeneration {
                discardCancelledFabricMeasurement(ownedGeneration, widthPixels: widthPixels, scale: scale)
            }
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
                fabricSurface: ownedGeneration?.surface,
                fabricGeneration: ownedGeneration,
                fabricLeaseHandle: ownedGeneration?.leaseHandle
            )
            if let generation = ownedGeneration,
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
                fabricSurface: ownedGeneration?.surface,
                fabricGeneration: ownedGeneration,
                fabricLeaseHandle: ownedGeneration?.leaseHandle
            )
            if let generation = ownedGeneration,
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

}
