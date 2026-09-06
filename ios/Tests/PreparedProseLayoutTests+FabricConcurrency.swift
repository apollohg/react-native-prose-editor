import CoreText
import Foundation
import UIKit
import XCTest

extension PreparedProseLayoutTests {
    func testFabricCommitPermitsOnlyItsCanonicalGeneration() {
        let registry = PreparedProseLayoutRegistry(
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                PreparedProseLayout(key: key, size: CGSize(width: width, height: 20), blocks: [], retainedBytes: 1)
            }
        )
        let first = request(source: "{\"type\":\"doc\",\"content\":[]}")
        let second = request(source: "{\"type\":\"doc\",\"content\":[{\"type\":\"paragraph\"}]}")
        let surface = FabricSurfaceToken(surfaceId: 41, componentTag: 410)
        let handle: UInt64 = 41
        let g1 = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(first, registry: registry),
            leaseHandle: handle
        )
        let g2 = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(second, registry: registry),
            leaseHandle: handle
        )
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: handle)

        // Both may prepare before a component commit has selected a winner.
        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: handle)
        _ = registry.measure(request: second, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: handle)
        registry.activateFabricGeneration(g2)

        XCTAssertEqual(
            registry.permittedFabricGenerationForTesting(FabricLeaseOwner(surface: surface, leaseHandle: handle)),
            g2.generationIdentity
        )
        XCTAssertFalse(registry.hasFabricGenerationOwnershipForTesting(g1))
        XCTAssertFalse(install(first, in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry, leaseHandle: handle))
        XCTAssertTrue(install(second, in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry, leaseHandle: handle))

        // A late G1 callback cannot recreate ownership after G2 commits.
        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: handle)
        XCTAssertFalse(registry.hasFabricGenerationOwnershipForTesting(g1))
    }

    func testCommittedFabricGenerationDoesNotCollapseProspectiveMeasurement() {
        let registry = PreparedProseLayoutRegistry(
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                PreparedProseLayout(key: key, size: CGSize(width: width, height: 20), blocks: [], retainedBytes: 1)
            }
        )
        let first = request(source: "first")
        let second = request(source: "second")
        let surface = FabricSurfaceToken(surfaceId: 44, componentTag: 440)
        let handle: UInt64 = 44
        let g1 = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(first, registry: registry),
            leaseHandle: handle
        )
        let g2 = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(second, registry: registry),
            leaseHandle: handle
        )
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: handle)

        XCTAssertEqual(
            registry.measure(
                request: first,
                widthPoints: 160,
                scale: 2,
                fabricSurface: surface,
                fabricLeaseHandle: handle
            ).size.height,
            20
        )
        registry.activateFabricGeneration(g1)
        XCTAssertTrue(install(first, in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry, leaseHandle: handle))

        XCTAssertEqual(
            registry.measure(
                request: second,
                widthPoints: 160,
                scale: 2,
                fabricSurface: surface,
                fabricLeaseHandle: handle
            ).size.height,
            20
        )
        registry.activateFabricGeneration(g2)
        XCTAssertTrue(install(second, in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry, leaseHandle: handle))
    }

    func testTerminalFabricOwnerSweepRemovesRetainedMountedReplacementAndIsExact() {
        let registry = PreparedProseLayoutRegistry(
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                PreparedProseLayout(key: key, size: CGSize(width: width, height: 20), blocks: [], retainedBytes: Int(width))
            }
        )
        let first = request(source: "mounted G1")
        let second = request(source: "failed G2")
        let surface = FabricSurfaceToken(surfaceId: 63, componentTag: 630)
        let isolatedSurface = FabricSurfaceToken(surfaceId: 63, componentTag: 631)
        let handle: UInt64 = 63
        let isolatedHandle: UInt64 = 64
        let g1 = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(first, registry: registry),
            leaseHandle: handle
        )
        let g2 = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(second, registry: registry),
            leaseHandle: handle
        )
        let isolated = FabricGenerationToken(
            surface: isolatedSurface,
            generationIdentity: canonicalFabricGenerationIdentity(first, registry: registry),
            leaseHandle: isolatedHandle
        )
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: handle)
        registry.registerFabricLease(surfaceId: isolatedSurface.surfaceId, componentTag: isolatedSurface.componentTag, leaseHandle: isolatedHandle)

        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: handle)
        XCTAssertTrue(install(first, in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry, leaseHandle: handle))
        registry.activateFabricGeneration(g2)
        _ = registry.measure(request: second, widthPoints: 140, scale: 2, fabricSurface: surface, fabricLeaseHandle: handle)
        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: isolatedSurface, fabricLeaseHandle: isolatedHandle)

        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 2)
        registry.releaseFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: handle)

        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 0)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 1)
        XCTAssertFalse(registry.hasFabricGenerationOwnershipForTesting(g1))
        XCTAssertFalse(registry.hasFabricGenerationOwnershipForTesting(g2))
        XCTAssertFalse(registry.hasFabricThemeOwnershipForTesting(g1))
        XCTAssertFalse(registry.hasFabricThemeOwnershipForTesting(g2))
        XCTAssertNil(FabricAttachmentSidecars.state(for: surface, leaseHandle: handle))
        XCTAssertTrue(registry.hasFabricGenerationOwnershipForTesting(isolated))
        XCTAssertNotNil(FabricAttachmentSidecars.state(for: isolatedSurface, leaseHandle: isolatedHandle))

        // The C++ family guard may release after the UIView has already swept.
        registry.releaseFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: handle)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 0)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 1)
    }

#if DEBUG
    func testTerminalReleaseAfterSidecarRegistrationRemovesOnlyItsExactSidecar() {
        let registry = PreparedProseLayoutRegistry(
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                PreparedProseLayout(key: key, size: CGSize(width: width, height: 20), blocks: [], retainedBytes: 1)
            }
        )
        let request = request()
        let surface = FabricSurfaceToken(surfaceId: 91, componentTag: 910)
        let h1: UInt64 = 1
        let h2: UInt64 = 2
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: h1)
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: h2)
        registry.fabricSidecarRegisteredForTesting = {
            registry.releaseFabricLease(
                surfaceId: surface.surfaceId,
                componentTag: surface.componentTag,
                leaseHandle: h1
            )
        }

        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: h1)

        XCTAssertNil(FabricAttachmentSidecars.state(for: surface, leaseHandle: h1))
        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: h2)
        XCTAssertNotNil(FabricAttachmentSidecars.state(for: surface, leaseHandle: h2))
    }
#endif

#if DEBUG
    func testConcurrentFabricMeasureRetainsGenerationPinAfterStaleMountMissCleanup() {
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: Int(width)
                )
            }
        )
        let request = request()
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let generation = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(request, registry: registry)
        )
        let exactCleanupReached = DispatchSemaphore(value: 0)
        let allowPinDecision = DispatchSemaphore(value: 0)
        let mountMissFinished = DispatchSemaphore(value: 0)

        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)
        registry.fabricMountMissAfterExactLeaseCleanupForTesting = {
            exactCleanupReached.signal()
            _ = allowPinDecision.wait(timeout: .now() + 1)
        }
        DispatchQueue.global().async {
            registry.releaseFabricMountMiss(generation, widthPoints: 160, scale: 2)
            mountMissFinished.signal()
        }

        XCTAssertEqual(exactCleanupReached.wait(timeout: .now() + 1), .success)
        let replacement = registry.measure(request: request, widthPoints: 140, scale: 2, fabricSurface: surface)
        allowPinDecision.signal()
        XCTAssertEqual(mountMissFinished.wait(timeout: .now() + 1), .success)

        XCTAssertTrue(registry.hasFabricGenerationOwnershipForTesting(generation))
        let drawingView = PreparedProseDrawingView(frame: .zero)
        XCTAssertTrue(install(request, in: drawingView, surface: surface, registry: registry, width: 140))
        XCTAssertTrue(drawingView.layout === replacement)
    }
#endif

    func testReleasedFabricPreparationCannotResurrectItsGenerationAndNewHandleCanMount() {
        let preparationStarted = DispatchSemaphore(value: 0)
        let allowPreparation = DispatchSemaphore(value: 0)
        let staleMeasureFinished = DispatchSemaphore(value: 0)
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                preparationStarted.signal()
                _ = allowPreparation.wait(timeout: .now() + 1)
                return PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: Int(width)
                )
            }
        )
        let request = request()
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let generation = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(request, registry: registry),
            leaseHandle: 1
        )

        DispatchQueue.global().async {
            _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)
            staleMeasureFinished.signal()
        }
        XCTAssertEqual(preparationStarted.wait(timeout: .now() + 1), .success)

        registry.releaseFabricGeneration(generation)
        allowPreparation.signal()
        XCTAssertEqual(staleMeasureFinished.wait(timeout: .now() + 1), .success)

        XCTAssertEqual(registry.fabricLeaseCountForTesting, 0)
        XCTAssertFalse(registry.hasFabricGenerationOwnershipForTesting(generation))
        XCTAssertFalse(registry.hasFabricThemeOwnershipForTesting(generation))
        XCTAssertNil(FabricAttachmentSidecars.state(for: surface, leaseHandle: generation.leaseHandle))

        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: 2)
        let fresh = registry.measure(
            request: request,
            widthPoints: 160,
            scale: 2,
            fabricSurface: surface,
            fabricLeaseHandle: 2
        )
        let drawingView = PreparedProseDrawingView(frame: .zero)
        XCTAssertTrue(install(request, in: drawingView, surface: surface, registry: registry, leaseHandle: 2))
        XCTAssertTrue(drawingView.layout === fresh)
        let freshGeneration = FabricGenerationToken(
            surface: surface,
            generationIdentity: generation.generationIdentity,
            leaseHandle: 2
        )
        XCTAssertTrue(registry.hasFabricGenerationOwnershipForTesting(freshGeneration))
        XCTAssertTrue(registry.hasFabricThemeOwnershipForTesting(freshGeneration))
    }

    @MainActor
    func testInvalidFabricWidthPreservesMountedOwnershipAndLaterValidReplacement() {
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: Int(width)
                )
            }
        )
        let primaryRequest = request()
        let otherRequest = request(source: "other")
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let otherSurface = FabricSurfaceToken(surfaceId: 12, componentTag: 102)
        let generation = registerAndActivateFabricGeneration(
            primaryRequest, surface: surface, registry: registry, leaseHandle: 101
        )
        let otherGeneration = registerAndActivateFabricGeneration(
            otherRequest, surface: otherSurface, registry: registry, leaseHandle: 201
        )

        let mounted = registry.measure(
            request: primaryRequest, widthPoints: 160, scale: 2,
            fabricSurface: surface, fabricLeaseHandle: generation.leaseHandle
        )
        let mountedView = PreparedProseDrawingView(frame: .zero)
        XCTAssertTrue(install(primaryRequest, in: mountedView, surface: surface, registry: registry, leaseHandle: generation.leaseHandle))
        XCTAssertTrue(mountedView.layout === mounted)
        guard let sidecar = FabricAttachmentSidecars.state(
            for: generation.surface,
            leaseHandle: generation.leaseHandle
        ) else {
            return XCTFail("The mounted Fabric generation should retain its attachment sidecar.")
        }
        _ = registry.measure(
            request: otherRequest, widthPoints: 120, scale: 2,
            fabricSurface: otherSurface, fabricLeaseHandle: otherGeneration.leaseHandle
        )
        let invalid = registry.measure(
            request: primaryRequest, widthPoints: 0, scale: 2,
            fabricSurface: surface, fabricLeaseHandle: generation.leaseHandle
        )

        XCTAssertEqual(invalid.error?.code, "INVALID_WIDTH")
        XCTAssertTrue(mountedView.layout === mounted)
        XCTAssertTrue(registry.hasFabricGenerationOwnershipForTesting(generation))
        XCTAssertTrue(registry.hasFabricThemeOwnershipForTesting(generation))
        XCTAssertTrue(
            FabricAttachmentSidecars.state(
                for: generation.surface,
                leaseHandle: generation.leaseHandle
            ) === sidecar
        )
        let replacement = registry.measure(
            request: primaryRequest, widthPoints: 140, scale: 2,
            fabricSurface: surface, fabricLeaseHandle: generation.leaseHandle
        )
        XCTAssertTrue(install(primaryRequest, in: mountedView, surface: surface, registry: registry, width: 140, leaseHandle: generation.leaseHandle))
        XCTAssertTrue(mountedView.layout === replacement)
        XCTAssertTrue(registry.hasFabricGenerationOwnershipForTesting(otherGeneration))
        XCTAssertTrue(registry.hasFabricThemeOwnershipForTesting(otherGeneration))
        XCTAssertTrue(install(otherRequest, in: PreparedProseDrawingView(frame: .zero), surface: otherSurface, registry: registry, width: 120, leaseHandle: otherGeneration.leaseHandle))
    }

    func testReleasedFabricCompileFailureCannotResurrectFailureOwnership() {
        let compilationStarted = DispatchSemaphore(value: 0)
        let allowCompilation = DispatchSemaphore(value: 0)
        let staleMeasureFinished = DispatchSemaphore(value: 0)
        let registry = PreparedProseLayoutRegistry(
            compile: { _ in
                compilationStarted.signal()
                _ = allowCompilation.wait(timeout: .now() + 1)
                throw ProseViewerError.compiler(
                    domain: "viewer",
                    code: "MALFORMED_INPUT",
                    message: "Malformed content"
                )
            }
        )
        let request = request()
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let generation = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(request, registry: registry)
        )

        DispatchQueue.global().async {
            _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)
            staleMeasureFinished.signal()
        }
        XCTAssertEqual(compilationStarted.wait(timeout: .now() + 1), .success)

        registry.releaseFabricGeneration(generation)
        allowCompilation.signal()
        XCTAssertEqual(staleMeasureFinished.wait(timeout: .now() + 1), .success)

        XCTAssertEqual(registry.fabricLeaseCountForTesting, 0)
        XCTAssertFalse(registry.hasFabricGenerationOwnershipForTesting(generation))
        XCTAssertFalse(registry.hasFabricThemeOwnershipForTesting(generation))
    }

}
