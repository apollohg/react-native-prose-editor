import CryptoKit
import Foundation
import UIKit

extension PreparedProseLayoutRegistry {
    @objc(measureSurfaceId:componentTag:leaseHandle:sourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:attachmentRevision:nativeFontRevision:nativeFontScale:fontEnvironmentRevision:userInterfaceStyle:accessibilityContrast:widthPoints:scale:)
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
        userInterfaceStyle: Int = 0,
        accessibilityContrast: Int = 0,
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
            fontEnvironmentRevision: fontEnvironmentRevision,
            userInterfaceStyle: userInterfaceStyle,
            accessibilityContrast: accessibilityContrast
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
    @objc(fabricGenerationIdentitySourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:attachmentRevision:nativeFontRevision:nativeFontScale:fontEnvironmentRevision:userInterfaceStyle:accessibilityContrast:)
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
        fontEnvironmentRevision: UInt64,
        userInterfaceStyle: Int = 0,
        accessibilityContrast: Int = 0
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
            fontEnvironmentRevision: fontEnvironmentRevision,
            userInterfaceStyle: userInterfaceStyle,
            accessibilityContrast: accessibilityContrast
        ).generationIdentity as NSString
    }

    /// Fabric image publication/error state must use the same canonical
    /// semantic key as direct UIKit. Layout revisions are intentionally
    /// excluded so attachment/font reinstallation preserves that state.
    @objc(fabricSemanticGenerationIdentitySourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:attachmentRevision:nativeFontRevision:nativeFontScale:fontEnvironmentRevision:userInterfaceStyle:accessibilityContrast:)
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
        fontEnvironmentRevision: UInt64,
        userInterfaceStyle: Int = 0,
        accessibilityContrast: Int = 0
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
            fontEnvironmentRevision: fontEnvironmentRevision,
            userInterfaceStyle: userInterfaceStyle,
            accessibilityContrast: accessibilityContrast
        ).semanticGenerationIdentity as NSString
    }

    @objc(installCachedLayoutInDrawingView:surfaceId:componentTag:leaseHandle:sourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:attachmentRevision:nativeFontRevision:nativeFontScale:fontEnvironmentRevision:userInterfaceStyle:accessibilityContrast:widthPoints:scale:)
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
        userInterfaceStyle: Int = 0,
        accessibilityContrast: Int = 0,
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
            fontEnvironmentRevision: fontEnvironmentRevision,
            userInterfaceStyle: userInterfaceStyle,
            accessibilityContrast: accessibilityContrast
        )
        guard let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: widthPoints, scale: scale) else {
            return false
        }
        let fabricSurface = FabricSurfaceToken(surfaceId: surfaceId, componentTag: componentTag)
        let generation = FabricGenerationToken(
            surface: fabricSurface,
            generationIdentity: request.generationIdentity,
            leaseHandle: leaseHandle
        )
        if let artifact = layoutCache.acquireForFabricMount(
            surface: fabricSurface,
            generationIdentity: request.generationIdentity,
            widthPixels: widthPixels,
            displayScale: scale,
            leaseHandle: leaseHandle,
            allowCompletedFallback: true,
            shouldAcquire: { self.isFabricLeaseActive(generation) }
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
        userInterfaceStyle: Int = 0,
        accessibilityContrast: Int = 0,
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
            userInterfaceStyle: userInterfaceStyle,
            accessibilityContrast: accessibilityContrast,
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
    func releaseFabricInvalidMeasurement(
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

    func beginBenchmarkResidentCensus(seeding keys: [ProseLayoutKey] = []) {
        layoutCache.beginBenchmarkCensus(seeding: keys)
    }

    func endBenchmarkResidentCensus() -> BenchmarkResidentCensus {
        let keys = layoutCache.endBenchmarkCensus()
        let material = keys
            .map { String(describing: $0) }
            .sorted()
            .joined(separator: "\n")
        let digest = SHA256.hash(data: Data(material.utf8)).map { String(format: "%02x", $0) }.joined()
        return .init(keys: keys, count: keys.count, digest: digest)
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
        PreparedProseInstrumentation.capturePreResetSnapshot()
        layoutCache.removeAllUnmounted()
        compiledCondition.lock()
        fabricOwnershipRevisions.removeAll()
        compiledDocuments.removeAll()
        compiledAccessOrder.removeAll()
        compiledAccessGenerations.removeAll()
        compiledAccessOrderHead = 0
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
        PreparedProseInstrumentation.cacheUpdated(compiledBytes: 0, compiledResidentCount: 0)
        PreparedProseInstrumentation.capturePostResetSnapshot()
    }

}
