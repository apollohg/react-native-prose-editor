import CoreText
import XCTest

final class PreparedProseLayoutTests: XCTestCase {
    private let document = ViewerDocument(
        semanticKey: String(repeating: "a", count: 64),
        paragraphs: [ViewerParagraph(text: "One prepared paragraph")],
        isEmpty: false,
        retainedBytes: 64
    )

    func testRepeatedFittingAndDrawingAtSamePhysicalWidthPrepareExactlyOnce() {
        var preparations = 0
        let registry = makeRegistry { document, key, width, scale in
            preparations += 1
            return try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let viewer = ProseViewerView(layoutRegistry: registry)

        XCTAssertTrue(viewer.apply(source: .json("{\"type\":\"doc\"}"), configuration: configuration()))
        let first = viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude))
        let second = viewer.sizeThatFits(CGSize(width: 160.1, height: .greatestFiniteMagnitude))
        XCTAssertGreaterThan(first.height, 0)
        XCTAssertEqual(first.height, second.height)

        viewer.frame = CGRect(x: 0, y: 0, width: 160, height: 200)
        viewer.layoutIfNeeded()
        viewer.drawingViewForTesting.draw(viewer.drawingViewForTesting.bounds)

        XCTAssertEqual(preparations, 1)
        XCTAssertEqual(registry.layoutPreparationCount, 1)
    }

    func testChangedPhysicalWidthPreparesOneAdditionalArtifact() {
        var preparations = 0
        let registry = makeRegistry { document, key, width, scale in
            preparations += 1
            return try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let viewer = ProseViewerView(layoutRegistry: registry)

        XCTAssertTrue(viewer.apply(source: .json("{\"type\":\"doc\"}"), configuration: configuration()))
        _ = viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude))
        _ = viewer.sizeThatFits(CGSize(width: 120, height: .greatestFiniteMagnitude))
        _ = viewer.sizeThatFits(CGSize(width: 120.1, height: .greatestFiniteMagnitude))

        XCTAssertEqual(preparations, 2)
        XCTAssertEqual(registry.layoutPreparationCount, 2)
    }

    func testMalformedInputProducesOneZeroHeightErrorArtifactAndOneDelegateEvent() {
        let registry = PreparedProseLayoutRegistry(
            compile: { _ in
                throw ProseViewerError.compiler(
                    domain: "viewer",
                    code: "MALFORMED_INPUT",
                    message: "Malformed content"
                )
            }
        )
        let viewer = ProseViewerView(layoutRegistry: registry)
        let delegate = FailureRecordingDelegate()
        viewer.interactionDelegate = delegate

        XCTAssertFalse(viewer.apply(source: .json("not valid"), configuration: configuration()))
        XCTAssertEqual(
            viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude)).height,
            0
        )
        XCTAssertEqual(
            viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude)).height,
            0
        )

        XCTAssertEqual(delegate.errors.count, 1)
        XCTAssertEqual(delegate.errors.first?.code, "MALFORMED_INPUT")
        XCTAssertEqual(registry.layoutPreparationCount, 0)
    }

    func testInvalidWidthProducesAnUncachedErrorAndReportsOnceForTheGeneration() {
        var preparations = 0
        let registry = makeRegistry { document, key, width, scale in
            preparations += 1
            return try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let viewer = ProseViewerView(layoutRegistry: registry)
        let delegate = FailureRecordingDelegate()
        viewer.interactionDelegate = delegate

        XCTAssertTrue(viewer.apply(source: .json("{\"type\":\"doc\"}"), configuration: configuration()))
        XCTAssertEqual(viewer.sizeThatFits(CGSize(width: .infinity, height: 100)).height, 0)
        XCTAssertEqual(viewer.sizeThatFits(CGSize(width: .infinity, height: 100)).height, 0)
        XCTAssertEqual(delegate.errors.map(\.code), ["INVALID_WIDTH"])
        XCTAssertEqual(preparations, 0)
        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
    }

    func testInvalidMetricsAfterValidMountLeaveDrawingUntouchedUntilValidMetricsReturn() {
        var preparations = 0
        let registry = makeRegistry { document, key, width, scale in
            preparations += 1
            return try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let request = request()
        let drawingView = PreparedProseDrawingView(frame: .zero)

        _ = registry.measure(request: request, widthPoints: 160, scale: 2)
        XCTAssertTrue(
            registry.installCachedLayout(
                in: drawingView,
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: 160,
                scale: 2
            )
        )
        guard let mountedLayout = drawingView.layout else {
            return XCTFail("A cached artifact should install once usable metrics arrive.")
        }

        XCTAssertFalse(
            registry.installCachedLayout(
                in: drawingView,
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: .infinity,
                scale: 0
            )
        )
        XCTAssertTrue(drawingView.layout === mountedLayout)
        XCTAssertEqual(preparations, 1)

        let invalidYogaMeasurement = registry.measure(request: request, widthPoints: .infinity, scale: 0)
        XCTAssertEqual(invalidYogaMeasurement.error?.code, "INVALID_WIDTH")
        XCTAssertTrue(
            registry.installCachedLayout(
                in: drawingView,
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: 160,
                scale: 2
            )
        )
        XCTAssertTrue(drawingView.layout === mountedLayout)
    }

    func testValidWidthPreparationFailureIsCachedAndInstalledWithoutRebuilding() {
        var preparations = 0
        let registry = makeRegistry { _, _, _, _ in
            preparations += 1
            throw ProseViewerError.layout(message: "Core Text preparation failed")
        }
        let request = request()
        let drawingView = PreparedProseDrawingView(frame: .zero)

        let first = registry.measure(request: request, widthPoints: 160, scale: 2)
        let second = registry.measure(request: request, widthPoints: 160, scale: 2)

        XCTAssertEqual(first.error?.code, "LAYOUT_FAILED")
        XCTAssertTrue(first === second)
        XCTAssertEqual(preparations, 1)
        XCTAssertEqual(registry.layoutPreparationCount, 1)
        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 1)
        XCTAssertTrue(
            registry.installCachedLayout(
                in: drawingView,
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: 160,
                scale: 2
            )
        )
        XCTAssertTrue(drawingView.layout === first)
    }

    func testOverflowingFiniteMetricsReturnUncachedInvalidWidthWithoutReplacingMountedArtifact() {
        let registry = makeRegistry { document, key, width, scale in
            try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let request = request()
        let drawingView = PreparedProseDrawingView(frame: .zero)

        _ = registry.measure(request: request, widthPoints: 160, scale: 2)
        XCTAssertTrue(
            registry.installCachedLayout(
                in: drawingView,
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: 160,
                scale: 2
            )
        )
        guard let mountedLayout = drawingView.layout else {
            return XCTFail("A valid measurement should mount its cached artifact.")
        }

        let overflowing = registry.measure(
            request: request,
            widthPoints: .greatestFiniteMagnitude,
            scale: 2
        )

        XCTAssertEqual(overflowing.error?.code, "INVALID_WIDTH")
        XCTAssertEqual(overflowing.size.width, 0)
        XCTAssertFalse(
            registry.installCachedLayout(
                in: drawingView,
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: .greatestFiniteMagnitude,
                scale: 2
            )
        )
        XCTAssertTrue(drawingView.layout === mountedLayout)
    }

    func testNegativeWidthAndScaleAreInvalidAndDoNotReplaceMountedArtifact() {
        var preparations = 0
        let registry = makeRegistry { document, key, width, scale in
            preparations += 1
            return try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let request = request()
        let drawingView = PreparedProseDrawingView(frame: .zero)

        _ = registry.measure(request: request, widthPoints: 160, scale: 2)
        XCTAssertTrue(
            registry.installCachedLayout(
                in: drawingView,
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: 160,
                scale: 2
            )
        )
        guard let mountedLayout = drawingView.layout else {
            return XCTFail("A valid measurement should mount its cached artifact.")
        }

        let invalidMeasurement = registry.measure(request: request, widthPoints: -160, scale: -2)

        XCTAssertNil(ProseLayoutMetrics.widthPixels(widthPoints: -160, scale: -2))
        XCTAssertEqual(invalidMeasurement.error?.code, "INVALID_WIDTH")
        XCTAssertFalse(
            registry.installCachedLayout(
                in: drawingView,
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: -160,
                scale: -2
            )
        )
        XCTAssertTrue(drawingView.layout === mountedLayout)
        XCTAssertEqual(preparations, 1)
    }

    func testCompilerFailureIsPreparedOnceAndMountAcquiresItsErrorArtifact() {
        var compilations = 0
        let registry = PreparedProseLayoutRegistry(
            compile: { _ in
                compilations += 1
                throw ProseViewerError.compiler(
                    domain: "viewer",
                    code: "MALFORMED_INPUT",
                    message: "Malformed content"
                )
            }
        )
        let request = request()
        let drawingView = PreparedProseDrawingView(frame: .zero)

        let first = registry.measure(request: request, widthPoints: 160, scale: 2)
        let second = registry.measure(request: request, widthPoints: 160, scale: 2)

        XCTAssertEqual(first.error?.code, "MALFORMED_INPUT")
        XCTAssertTrue(first === second)
        XCTAssertEqual(compilations, 1)
        XCTAssertTrue(
            registry.installCachedLayout(
                in: drawingView,
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: 160,
                scale: 2
            )
        )
        XCTAssertTrue(drawingView.layout === first)
        XCTAssertEqual(compilations, 1)
    }

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
        let drawingView = PreparedProseDrawingView(frame: .zero)

        let measured = registry.measure(request: request, widthPoints: 160, scale: 2)

        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
        XCTAssertTrue(
            registry.installCachedLayout(
                in: drawingView,
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: 160,
                scale: 2
            )
        )
        XCTAssertTrue(drawingView.layout === measured)
        XCTAssertEqual(compilations, 1)
        XCTAssertFalse(
            registry.installCachedLayout(
                in: PreparedProseDrawingView(frame: .zero),
                sourceKind: "json",
                source: request.source.value as NSString,
                configJSON: request.configuration.configJSON as NSString,
                themeJSON: nil,
                imagePolicyJSON: nil,
                imagesEnabled: request.configuration.imagesEnabled,
                collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
                attachmentRevision: request.attachmentRevision,
                nativeFontRevision: request.nativeFontRevision,
                fontEnvironmentRevision: request.fontEnvironmentRevision,
                widthPoints: 160,
                scale: 2
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

    func testLeaseBudgetCountsHandoffArtifactsAndKeepsOnlyOneOversizedLease() {
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

        XCTAssertEqual(registry.layoutRetainedBytesForTesting, 11)
        XCTAssertEqual(registry.oversizedLeaseCountForTesting, 1)
        XCTAssertFalse(install(request, in: PreparedProseDrawingView(frame: .zero), surface: firstSurface, registry: registry))
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
        XCTAssertFalse(install(first, in: PreparedProseDrawingView(frame: .zero), surface: firstSurface, registry: registry))

        registry.releaseFabricMountMiss(
            FabricGenerationToken(surface: firstSurface, generationIdentity: first.generationIdentity)
        )
        XCTAssertTrue(install(second, in: PreparedProseDrawingView(frame: .zero), surface: secondSurface, registry: registry))
        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: firstSurface)

        XCTAssertEqual(compilations, 3)
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
        _ = viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude))

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

        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: surface)
        _ = registry.measure(request: second, widthPoints: 160, scale: 2, fabricSurface: otherSurface)
        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: surface)
        XCTAssertEqual(compilations, 2)

        registry.releaseFabricSurface(surface)
        registry.didReceiveMemoryWarning()
        _ = registry.measure(request: first, widthPoints: 160, scale: 2, fabricSurface: surface)
        XCTAssertEqual(compilations, 3)
    }

    func testRevisionVariantsUseDistinctPreparedArtifacts() {
        var preparations = 0
        let registry = makeRegistry { document, key, width, scale in
            preparations += 1
            return try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let base = request()
        let attachmentChange = request(attachmentRevision: 1)
        let nativeFontChange = request(nativeFontRevision: 1)
        let environmentChange = request(fontEnvironmentRevision: 1)

        _ = registry.measure(request: base, widthPoints: 160, scale: 2)
        _ = registry.measure(request: attachmentChange, widthPoints: 160, scale: 2)
        _ = registry.measure(request: nativeFontChange, widthPoints: 160, scale: 2)
        _ = registry.measure(request: environmentChange, widthPoints: 160, scale: 2)

        XCTAssertEqual(preparations, 4)
        XCTAssertEqual(registry.layoutPreparationCount, 4)
    }

    func testClampedDrawingUsesViewBoundsForTheCoreTextCoordinateSystem() {
        let artifactHeight: CGFloat = 120
        let baselineFromArtifactTop: CGFloat = 18
        let clampedBounds = CGRect(x: 0, y: 0, width: 160, height: 40)

        XCTAssertEqual(
            PreparedProseDrawingView.textPosition(
                baselineFromArtifactTop: baselineFromArtifactTop,
                in: clampedBounds,
                artifactHeight: artifactHeight
            ).y,
            22
        )
    }

    func testCompiledDocumentsAreBudgetedAndEvictedInAccessOrder() throws {
        var compilations = 0
        let registry = PreparedProseLayoutRegistry(
            compiledByteBudget: 100,
            compile: { request in
                compilations += 1
                return ViewerDocument(
                    semanticKey: String(repeating: request.source.value == "first" ? "a" : "b", count: 64),
                    paragraphs: [ViewerParagraph(text: request.source.value)],
                    isEmpty: false,
                    retainedBytes: 40
                )
            }
        )
        let first = ProseViewerRequest(source: .json("first"), configuration: configuration())
        let second = ProseViewerRequest(source: .json("second"), configuration: configuration())
        let third = ProseViewerRequest(source: .json("third"), configuration: configuration())

        _ = try registry.compileDocument(request: first)
        _ = try registry.compileDocument(request: second)
        _ = try registry.compileDocument(request: first)
        _ = try registry.compileDocument(request: third)
        _ = try registry.compileDocument(request: first)
        _ = try registry.compileDocument(request: second)

        XCTAssertEqual(compilations, 4)
        XCTAssertEqual(registry.compiledDocumentBytesForTesting, 80)
    }

    func testMemoryWarningReleasesCacheOwnershipWithoutReleasingMountedArtifact() {
        let registry = makeRegistry { document, key, width, scale in
            try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let viewer = ProseViewerView(layoutRegistry: registry)

        XCTAssertTrue(viewer.apply(source: .json("{\"type\":\"doc\"}"), configuration: configuration()))
        _ = viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude))
        guard let mountedArtifact = viewer.drawingViewForTesting.layout else {
            return XCTFail("Measurement should install the prepared artifact in the drawing view.")
        }

        registry.didReceiveMemoryWarning()

        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
        XCTAssertTrue(viewer.drawingViewForTesting.layout === mountedArtifact)
    }

    private func configuration() -> ProseViewerConfiguration {
        ProseViewerConfiguration(configJSON: "{}", collapsesWhenEmpty: true)
    }

    private func request(
        source: String = "{\"type\":\"doc\"}",
        attachmentRevision: UInt64 = 0,
        nativeFontRevision: UInt64 = 0,
        fontEnvironmentRevision: UInt64 = 0
    ) -> ProseViewerRequest {
        ProseViewerRequest(
            source: .json(source),
            configuration: configuration(),
            nativeFontRevision: nativeFontRevision,
            fontEnvironmentRevision: fontEnvironmentRevision,
            attachmentRevision: attachmentRevision
        )
    }

    private func makeRegistry(
        prepare: @escaping PreparedProseLayoutRegistry.LayoutPreparation
    ) -> PreparedProseLayoutRegistry {
        PreparedProseLayoutRegistry(
            compile: { [document = self.document] _ in document },
            prepare: prepare
        )
    }

    private func install(
        _ request: ProseViewerRequest,
        in drawingView: PreparedProseDrawingView,
        surface: FabricSurfaceToken,
        registry: PreparedProseLayoutRegistry,
        width: CGFloat = 160
    ) -> Bool {
        registry.installCachedLayout(
            in: drawingView,
            surfaceId: surface.surfaceId,
            componentTag: surface.componentTag,
            sourceKind: "json",
            source: request.source.value as NSString,
            configJSON: request.configuration.configJSON as NSString,
            themeJSON: nil,
            imagePolicyJSON: nil,
            imagesEnabled: request.configuration.imagesEnabled,
            collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
            attachmentRevision: request.attachmentRevision,
            nativeFontRevision: request.nativeFontRevision,
            fontEnvironmentRevision: request.fontEnvironmentRevision,
            widthPoints: width,
            scale: 2
        )
    }

    private final class FailureRecordingDelegate: ProseViewerInteractionDelegate {
        var errors: [ProseViewerError] = []

        func proseViewer(_ view: ProseViewerView, didTapLink href: String, text: String) {}
        func proseViewer(_ view: ProseViewerView, didTapMention docPos: Int, label: String) {}
        func proseViewer(_ view: ProseViewerView, didFail error: ProseViewerError) {
            errors.append(error)
        }
    }
}
