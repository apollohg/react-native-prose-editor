import CoreText
import Foundation
import UIKit
import XCTest

extension PreparedProseLayoutTests {
    @MainActor
    func testExactFabricMountMissCleanupPreservesOtherSurfaceAndGenerationLeases() {
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
        let first = request(source: "first")
        let second = request(source: "second")
        let firstSurface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let secondSurface = FabricSurfaceToken(surfaceId: 12, componentTag: 102)

        let firstGeneration = registerAndActivateFabricGeneration(
            first, surface: firstSurface, registry: registry, leaseHandle: 101
        )
        let otherSurfaceGeneration = registerAndActivateFabricGeneration(
            first, surface: secondSurface, registry: registry, leaseHandle: 201
        )
        let replacementGeneration = registerAndActivateFabricGeneration(
            second, surface: firstSurface, registry: registry, leaseHandle: 102
        )

        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: firstSurface, fabricLeaseHandle: firstGeneration.leaseHandle)
        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: secondSurface, fabricLeaseHandle: otherSurfaceGeneration.leaseHandle)
        _ = registry.measure(request: second, widthPoints: 160, scale: 2, fabricSurface: firstSurface, fabricLeaseHandle: replacementGeneration.leaseHandle)
        registry.releaseFabricMountMiss(firstGeneration, widthPoints: 160, scale: 2)

        XCTAssertFalse(install(first, in: PreparedProseDrawingView(frame: .zero), surface: firstSurface, registry: registry, leaseHandle: firstGeneration.leaseHandle))
        XCTAssertTrue(install(first, in: PreparedProseDrawingView(frame: .zero), surface: secondSurface, registry: registry, leaseHandle: otherSurfaceGeneration.leaseHandle))
        XCTAssertTrue(install(second, in: PreparedProseDrawingView(frame: .zero), surface: firstSurface, registry: registry, leaseHandle: replacementGeneration.leaseHandle))
    }

    @MainActor
    func testFabricWidthLeaseReplacementLeavesOtherSurfacesAndGenerationsOwned() {
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
        let firstGeneration = request(source: "first")
        let secondGeneration = request(source: "second")
        let firstSurface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let secondSurface = FabricSurfaceToken(surfaceId: 12, componentTag: 102)

        let firstSurfaceGeneration = registerAndActivateFabricGeneration(
            firstGeneration, surface: firstSurface, registry: registry, leaseHandle: 101
        )
        let secondSurfaceGeneration = registerAndActivateFabricGeneration(
            firstGeneration, surface: secondSurface, registry: registry, leaseHandle: 201
        )
        let replacementGeneration = registerAndActivateFabricGeneration(
            secondGeneration, surface: firstSurface, registry: registry, leaseHandle: 102
        )
        _ = registry.measure(request: firstGeneration, widthPoints: 160, scale: 2, fabricSurface: firstSurface, fabricLeaseHandle: firstSurfaceGeneration.leaseHandle)
        XCTAssertTrue(
            install(
                firstGeneration,
                in: PreparedProseDrawingView(frame: .zero),
                surface: firstSurface,
                registry: registry,
                leaseHandle: firstSurfaceGeneration.leaseHandle
            )
        )
        _ = registry.measure(request: firstGeneration, widthPoints: 120, scale: 2, fabricSurface: secondSurface, fabricLeaseHandle: secondSurfaceGeneration.leaseHandle)
        let replacement = registry.measure(
            request: secondGeneration,
            widthPoints: 140,
            scale: 2,
            fabricSurface: firstSurface,
            fabricLeaseHandle: replacementGeneration.leaseHandle
        )

        let replacementView = PreparedProseDrawingView(frame: .zero)
        XCTAssertTrue(
            install(
                secondGeneration,
                in: replacementView,
                surface: firstSurface,
                registry: registry,
                width: 140,
                leaseHandle: replacementGeneration.leaseHandle
            )
        )
        XCTAssertTrue(replacementView.layout === replacement)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 2)
        XCTAssertEqual(registry.fabricLeaseCountForTesting, 3)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 420)

        XCTAssertTrue(
            install(
                firstGeneration,
                in: PreparedProseDrawingView(frame: .zero),
                surface: secondSurface,
                registry: registry,
                width: 120,
                leaseHandle: secondSurfaceGeneration.leaseHandle
            )
        )
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 0)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 3)
        XCTAssertEqual(registry.fabricLeaseCountForTesting, 3)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 420)
    }

    func testReleasingSurfaceLeasePreventsStaleGenerationMount() {
        let registry = makeRegistry { document, key, width, scale in
            try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let request = request()
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)

        registry.releaseFabricSurface(surface)
        registry.didReceiveMemoryWarning()

        XCTAssertFalse(install(request, in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry))
    }

    func testSurfaceStopKeepsOldFamilyInactiveUntilTerminalCleanupAndAllowsNewHandle() {
        let registry = PreparedProseLayoutRegistry(
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                PreparedProseLayout(key: key, size: CGSize(width: width, height: 20), blocks: [], retainedBytes: 1)
            }
        )
        let request = request()
        let surface = FabricSurfaceToken(surfaceId: 89, componentTag: 890)
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: 1)
        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: 1)

        registry.releaseFabricSurface(surface)
        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: 1)
        XCTAssertEqual(registry.fabricLeaseCountForTesting, 0)
        XCTAssertNil(FabricAttachmentSidecars.state(for: surface, leaseHandle: 1))

        registry.releaseFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: 1)
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: 2)
        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: 2)
        XCTAssertEqual(registry.fabricLeaseCountForTesting, 1)
    }

    func testMemoryWarningThenSurfaceReleaseRemovesCacheDiscoveredMountedLease() {
        let registry = makeRegistry { document, key, width, scale in
            try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let request = request()
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)
        XCTAssertTrue(install(request, in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry))

        registry.didReceiveMemoryWarning()
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        registry.releaseFabricSurface(surface)

        XCTAssertEqual(registry.fabricLeaseCountForTesting, 0)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 0)
    }

    func testLeaseBudgetRetainsBothExactOversizedPendingHandoffsUntilMount() {
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 10,
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: 11
                )
            }
        )
        let request = request()
        let firstSurface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let secondSurface = FabricSurfaceToken(surfaceId: 12, componentTag: 102)

        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: firstSurface)
        _ = registry.measure(request: request, widthPoints: 120, scale: 2, fabricSurface: secondSurface)

        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 22)
        XCTAssertEqual(registry.oversizedLeaseCountForTesting, 2)
        XCTAssertEqual(registry.fabricLeaseCountForTesting, 2)
        XCTAssertTrue(install(request, in: PreparedProseDrawingView(frame: .zero), surface: firstSurface, registry: registry))
        XCTAssertTrue(install(request, in: PreparedProseDrawingView(frame: .zero), surface: secondSurface, registry: registry, width: 120))
    }

    func testFabricMountMissNeverCompiles() {
        var compilations = 0
        let registry = PreparedProseLayoutRegistry(
            compile: { [document = self.document] _ in
                compilations += 1
                return document
            }
        )
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)

        XCTAssertFalse(install(request(), in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry))
        XCTAssertEqual(compilations, 0)
    }

    func testFabricMountMissCleanupReleasesOnlyThePersistedGenerationPin() {
        var compilations = 0
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compiledByteBudget: 1,
            compile: { request in
                compilations += 1
                return ViewerDocument(
                    semanticKey: String(repeating: request.source.value == "first" ? "a" : "b", count: 64),
                    paragraphs: [ViewerParagraph(text: request.source.value)],
                    isEmpty: false,
                    retainedBytes: 2
                )
            },
            prepare: { _, key, width, _ in
                PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: 2
                )
            }
        )
        let first = request(source: "first")
        let second = request(source: "second")
        let firstSurface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let secondSurface = FabricSurfaceToken(surfaceId: 12, componentTag: 102)

        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: firstSurface)
        _ = registry.measure(request: second, widthPoints: 160, scale: 2, fabricSurface: secondSurface)

        registry.releaseFabricMountMiss(
            FabricGenerationToken(
                surface: firstSurface,
                generationIdentity: canonicalFabricGenerationIdentity(first, registry: registry),
                leaseHandle: 1
            ),
            widthPoints: 160,
            scale: 2
        )
        XCTAssertFalse(install(first, in: PreparedProseDrawingView(frame: .zero), surface: firstSurface, registry: registry))
        XCTAssertTrue(install(second, in: PreparedProseDrawingView(frame: .zero), surface: secondSurface, registry: registry))
        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: firstSurface)

        XCTAssertEqual(compilations, 3)
    }

    func testCanonicalGenerationReleaseClearsOversizedLeaseAndCompilerPinForRecycle() {
        var compilations = 0
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compiledByteBudget: 1,
            compile: { [document = self.document] _ in
                compilations += 1
                return document
            },
            prepare: { _, key, width, _ in
                PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: 2
                )
            }
        )
        let request = request()
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)

        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)
        registry.releaseFabricGeneration(
            FabricGenerationToken(
                surface: surface,
                generationIdentity: canonicalFabricGenerationIdentity(request, registry: registry),
                leaseHandle: 1
            )
        )

        XCTAssertFalse(install(request, in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry))
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: 2)
        _ = registry.measure(
            request: request,
            widthPoints: 160,
            scale: 2,
            fabricSurface: surface,
            fabricLeaseHandle: 2
        )
        XCTAssertEqual(compilations, 2)
    }

    func testUIKitApplyRetainsCompiledDocumentThroughRegistryEviction() throws {
        var compilations = 0
        let registry = PreparedProseLayoutRegistry(
            compiledByteBudget: 1,
            compile: { request in
                compilations += 1
                return ViewerDocument(
                    semanticKey: String(repeating: request.source.value == "first" ? "a" : "b", count: 64),
                    paragraphs: [ViewerParagraph(text: request.source.value)],
                    isEmpty: false,
                    retainedBytes: 2
                )
            }
        )
        let viewer = ProseViewerView(layoutRegistry: registry)

        XCTAssertTrue(viewer.apply(source: .json("first"), configuration: configuration()))
        _ = try registry.compileDocument(request: request(source: "second"))
        _ = viewer.sizeThatFits(CGSize(width: 160, height: CGFloat.greatestFiniteMagnitude))

        XCTAssertEqual(compilations, 2)
    }

    func testSurfaceGenerationPinsCompilerFailuresUntilReleased() {
        var compilations = 0
        let registry = PreparedProseLayoutRegistry(
            compilationFailureBudget: 1,
            compile: { request in
                compilations += 1
                throw ProseViewerError.compiler(
                    domain: "viewer",
                    code: request.source.value,
                    message: "Malformed content"
                )
            }
        )
        let surface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let otherSurface = FabricSurfaceToken(surfaceId: 12, componentTag: 102)
        let first = request(source: "first")
        let second = request(source: "second")
        let firstGeneration = registerAndActivateFabricGeneration(
            first, surface: surface, registry: registry, leaseHandle: 101
        )
        let otherGeneration = registerAndActivateFabricGeneration(
            second, surface: otherSurface, registry: registry, leaseHandle: 201
        )

        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: firstGeneration.leaseHandle)
        _ = registry.measure(request: second, widthPoints: 160, scale: 2, fabricSurface: otherSurface, fabricLeaseHandle: otherGeneration.leaseHandle)
        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: firstGeneration.leaseHandle)
        XCTAssertEqual(compilations, 2)

        registry.releaseFabricSurface(surface)
        registry.didReceiveMemoryWarning()
        let restartedGeneration = registerAndActivateFabricGeneration(
            first, surface: surface, registry: registry, leaseHandle: 102
        )
        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: restartedGeneration.leaseHandle)
        XCTAssertEqual(compilations, 3)
    }

}
