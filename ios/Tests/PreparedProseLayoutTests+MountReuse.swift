import CoreText
import Foundation
import UIKit
import XCTest

extension PreparedProseLayoutTests {
    func testOversizedMeasurementLeaseSurvivesEvictionUntilFabricMount() {
        var compilations = 0
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
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
        let drawingView = PreparedProseDrawingView(frame: .zero)

        let measured = registry.measure(
            request: request,
            widthPoints: 160,
            scale: 2,
            fabricSurface: surface
        )

        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
        XCTAssertTrue(install(request, in: drawingView, surface: surface, registry: registry))
        XCTAssertTrue(drawingView.layout === measured)
        XCTAssertEqual(compilations, 1)
        XCTAssertFalse(
            install(
                request,
                in: PreparedProseDrawingView(frame: .zero),
                surface: surface,
                registry: registry
            )
        )
    }

    func testFabricMountAcceptsAOnePixelGridRoundingDifference() {
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
        let drawingView = PreparedProseDrawingView(frame: .zero)

        let measured = registry.measure(
            request: request,
            widthPoints: 160,
            scale: 2,
            fabricSurface: surface
        )

        XCTAssertTrue(
            install(request, in: drawingView, surface: surface, registry: registry, width: 160.5)
        )
        XCTAssertTrue(drawingView.layout === measured)
    }

    func testFabricMountRejectsAWidthBeyondThePixelGridRoundingSlack() {
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

        _ = registry.measure(
            request: request,
            widthPoints: 160,
            scale: 2,
            fabricSurface: surface
        )

        XCTAssertFalse(
            install(
                request,
                in: PreparedProseDrawingView(frame: .zero),
                surface: surface,
                registry: registry,
                width: 161
            )
        )
    }

    func testIdenticalFabricGenerationsKeepSeparateSurfaceLeases() {
        let registry = makeRegistry { document, key, width, scale in
            try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let request = request()
        let firstSurface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let secondSurface = FabricSurfaceToken(surfaceId: 12, componentTag: 102)

        let measured = registry.measure(
            request: request,
            widthPoints: 160,
            scale: 2,
            fabricSurface: firstSurface
        )
        _ = registry.measure(
            request: request,
            widthPoints: 160,
            scale: 2,
            fabricSurface: secondSurface
        )

        let firstDrawingView = PreparedProseDrawingView(frame: .zero)
        let secondDrawingView = PreparedProseDrawingView(frame: .zero)
        XCTAssertTrue(install(request, in: firstDrawingView, surface: firstSurface, registry: registry))
        XCTAssertTrue(install(request, in: secondDrawingView, surface: secondSurface, registry: registry))
        XCTAssertTrue(firstDrawingView.layout === measured)
        XCTAssertTrue(secondDrawingView.layout === measured)
    }

    func testFabricWidthChurnRetiresStalePendingLeaseAndPreservesMountedArtifactUntilReplacement() {
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
        let drawingView = PreparedProseDrawingView(frame: .zero)

        let first = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)
        XCTAssertTrue(install(request, in: drawingView, surface: surface, registry: registry))
        XCTAssertTrue(drawingView.layout === first)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 0)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 160)

        _ = registry.measure(request: request, widthPoints: 120, scale: 2, fabricSurface: surface)
        _ = registry.measure(request: request, widthPoints: 140, scale: 2, fabricSurface: surface)

        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.fabricLeaseCountForTesting, 2)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 300)
        XCTAssertTrue(drawingView.layout === first)
        XCTAssertFalse(
            install(
                request,
                in: PreparedProseDrawingView(frame: .zero),
                surface: surface,
                registry: registry,
                width: 120
            )
        )

        registry.didReceiveMemoryWarning()
        XCTAssertTrue(drawingView.layout === first)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 0)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.fabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 160)
        XCTAssertFalse(
            install(
                request,
                in: PreparedProseDrawingView(frame: .zero),
                surface: surface,
                registry: registry,
                width: 140
            )
        )
        let replacement = registry.measure(request: request, widthPoints: 140, scale: 2, fabricSurface: surface)

        XCTAssertTrue(install(request, in: drawingView, surface: surface, registry: registry, width: 140))
        XCTAssertTrue(drawingView.layout === replacement)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 0)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.fabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 140)
        XCTAssertFalse(
            install(
                request,
                in: PreparedProseDrawingView(frame: .zero),
                surface: surface,
                registry: registry,
                width: 160
            )
        )
    }

    func testExactFabricRemeasureKeepsPendingWidthReplacementUntilItMounts() {
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
        let drawingView = PreparedProseDrawingView(frame: .zero)

        let mounted = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)
        XCTAssertTrue(install(request, in: drawingView, surface: surface, registry: registry))
        let replacement = registry.measure(request: request, widthPoints: 140, scale: 2, fabricSurface: surface)

        let exactRemeasure = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)

        XCTAssertTrue(exactRemeasure === mounted)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 300)
        XCTAssertTrue(install(request, in: drawingView, surface: surface, registry: registry, width: 140))
        XCTAssertTrue(drawingView.layout === replacement)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 0)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 140)
    }

    func testFabricRemeasureAfterMemoryWarningReusesExactMountedArtifact() {
        var preparations = 0
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                preparations += 1
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
        let drawingView = PreparedProseDrawingView(frame: .zero)

        let mounted = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)
        XCTAssertTrue(install(request, in: drawingView, surface: surface, registry: registry))
        registry.didReceiveMemoryWarning()

        let exactRemeasure = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)

        XCTAssertTrue(exactRemeasure === mounted)
        XCTAssertEqual(preparations, 1)
        XCTAssertEqual(registry.layoutPreparationCount, 1)
        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 0)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 160)
    }

    func testFabricSurfaceReusesGloballyMountedArtifactAfterMemoryWarning() {
        var preparations = 0
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                preparations += 1
                return PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: Int(width)
                )
            }
        )
        let request = request()
        let firstSurface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let secondSurface = FabricSurfaceToken(surfaceId: 12, componentTag: 102)

        let first = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: firstSurface)
        XCTAssertTrue(install(request, in: PreparedProseDrawingView(frame: .zero), surface: firstSurface, registry: registry))
        registry.didReceiveMemoryWarning()

        let second = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: secondSurface)

        XCTAssertTrue(second === first)
        XCTAssertEqual(preparations, 1)
        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 160)
        let secondView = PreparedProseDrawingView(frame: .zero)
        XCTAssertTrue(install(request, in: secondView, surface: secondSurface, registry: registry))
        XCTAssertTrue(secondView.layout === first)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 0)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 2)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 160)
    }

    func testFabricSurfaceReusesLivePendingArtifactAfterCompletedEviction() {
        var preparations = 0
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                preparations += 1
                return PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: 2
                )
            }
        )
        let request = request()
        let firstSurface = FabricSurfaceToken(surfaceId: 11, componentTag: 101)
        let secondSurface = FabricSurfaceToken(surfaceId: 12, componentTag: 102)

        let first = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: firstSurface)
        let second = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: secondSurface)

        XCTAssertTrue(second === first)
        XCTAssertEqual(preparations, 1)
        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 2)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 2)
        XCTAssertTrue(install(request, in: PreparedProseDrawingView(frame: .zero), surface: firstSurface, registry: registry))
        XCTAssertTrue(install(request, in: PreparedProseDrawingView(frame: .zero), surface: secondSurface, registry: registry))
    }

    func testDirectMountedArtifactReusesAfterCompletedEvictionAndMemoryWarning() {
        var preparations = 0
        let registry = PreparedProseLayoutRegistry(
            byteBudget: 1,
            compile: { [document = self.document] _ in document },
            prepare: { _, key, width, _ in
                preparations += 1
                return PreparedProseLayout(
                    key: key,
                    size: CGSize(width: width, height: 20),
                    blocks: [],
                    retainedBytes: 2
                )
            }
        )
        let first = ProseViewerView(layoutRegistry: registry)
        let second = ProseViewerView(layoutRegistry: registry)

        XCTAssertTrue(first.apply(source: .json("{\"type\":\"doc\"}"), configuration: configuration()))
        _ = first.sizeThatFits(CGSize(width: 160, height: CGFloat.greatestFiniteMagnitude))
        guard let mounted = first.drawingViewForTesting.layout else {
            return XCTFail("The first direct viewer should retain its prepared artifact.")
        }
        registry.didReceiveMemoryWarning()

        XCTAssertTrue(second.apply(source: .json("{\"type\":\"doc\"}"), configuration: configuration()))
        _ = second.sizeThatFits(CGSize(width: 160, height: CGFloat.greatestFiniteMagnitude))

        XCTAssertTrue(second.drawingViewForTesting.layout === mounted)
        XCTAssertEqual(preparations, 1)
        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 2)
    }

}
