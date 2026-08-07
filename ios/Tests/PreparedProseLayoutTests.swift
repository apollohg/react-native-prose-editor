import CoreText
import Foundation
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
        let first = viewer.sizeThatFits(CGSize(width: 160, height: CGFloat.greatestFiniteMagnitude))
        let second = viewer.sizeThatFits(CGSize(width: 160.1, height: CGFloat.greatestFiniteMagnitude))
        XCTAssertGreaterThan(first.height, 0)
        XCTAssertEqual(first.height, second.height)

        viewer.frame = CGRect(x: 0, y: 0, width: 160, height: 200)
        viewer.layoutIfNeeded()
        viewer.drawingViewForTesting.draw(viewer.drawingViewForTesting.bounds)

        XCTAssertEqual(preparations, 1)
        XCTAssertEqual(registry.layoutPreparationCount, 1)
    }

    func testFrameBasedLayoutPreparesAndInstallsWithoutPriorMeasurement() throws {
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
        viewer.frame = CGRect(x: 0, y: 0, width: 160, height: 200)
        viewer.setNeedsLayout()
        viewer.layoutIfNeeded()

        let first = try XCTUnwrap(viewer.drawingViewForTesting.layout)
        XCTAssertGreaterThan(first.size.height, 0)
        XCTAssertFalse(first.blocks.isEmpty)
        XCTAssertEqual(preparations, 1)

        viewer.setNeedsLayout()
        viewer.layoutIfNeeded()
        XCTAssertTrue(viewer.drawingViewForTesting.layout === first)
        XCTAssertEqual(preparations, 1)

        viewer.frame.size.width = 120
        viewer.setNeedsLayout()
        viewer.layoutIfNeeded()
        let second = try XCTUnwrap(viewer.drawingViewForTesting.layout)
        XCTAssertFalse(second === first)
        XCTAssertEqual(preparations, 2)

        viewer.frame.size.width = 160
        viewer.setNeedsLayout()
        viewer.layoutIfNeeded()
        XCTAssertTrue(viewer.drawingViewForTesting.layout === first)
        XCTAssertEqual(preparations, 2)
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
        _ = viewer.sizeThatFits(CGSize(width: 160, height: CGFloat.greatestFiniteMagnitude))
        _ = viewer.sizeThatFits(CGSize(width: 120, height: CGFloat.greatestFiniteMagnitude))
        _ = viewer.sizeThatFits(CGSize(width: 120.1, height: CGFloat.greatestFiniteMagnitude))

        XCTAssertEqual(preparations, 2)
        XCTAssertEqual(registry.layoutPreparationCount, 2)
    }

    func testMentionActivationPreservesAttributesAndRejectsANonObjectRoot() throws {
        let viewer = ProseViewerView(layoutRegistry: makeRegistry { document, key, width, scale in
            try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        })
        let delegate = FailureRecordingDelegate()
        viewer.interactionDelegate = delegate
        let interaction = PreparedProseInteraction(
            kind: .mention,
            rects: [CGRect(x: 0, y: 0, width: 20, height: 20)],
            href: nil,
            visibleText: "@alice",
            docPos: .max,
            label: "@alice",
            attrsJSON: #"{"id":"user-9","profile":{"kind":"clinician"}}"#
        )

        XCTAssertTrue(viewer.activatePreparedInteractionForTesting(interaction))
        XCTAssertEqual(delegate.mentions.count, 1)
        let mention = try XCTUnwrap(delegate.mentions.first)
        XCTAssertEqual(mention.docPos, UInt32.max)
        XCTAssertEqual(mention.label, "@alice")
        XCTAssertEqual(mention.attrs["id"] as? String, "user-9")
        XCTAssertEqual(
            (mention.attrs["profile"] as? [String: Any])?["kind"] as? String,
            "clinician"
        )

        let invalid = PreparedProseInteraction(
            kind: .mention,
            rects: interaction.rects,
            href: nil,
            visibleText: "@invalid",
            docPos: 9,
            label: "@invalid",
            attrsJSON: "[]"
        )
        XCTAssertFalse(viewer.activatePreparedInteractionForTesting(invalid))
        XCTAssertEqual(delegate.mentions.count, 1)
        XCTAssertEqual(delegate.errors.last?.code, "INVALID_MENTION_ATTRIBUTES")
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
            viewer.sizeThatFits(CGSize(width: 160, height: CGFloat.greatestFiniteMagnitude)).height,
            0
        )
        XCTAssertEqual(
            viewer.sizeThatFits(CGSize(width: 160, height: CGFloat.greatestFiniteMagnitude)).height,
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
        XCTAssertEqual(viewer.sizeThatFits(CGSize(width: CGFloat.infinity, height: 100)).height, 0)
        XCTAssertEqual(viewer.sizeThatFits(CGSize(width: CGFloat.infinity, height: 100)).height, 0)
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
                widthPoints: CGFloat.infinity,
                scale: 0
            )
        )
        XCTAssertTrue(drawingView.layout === mountedLayout)
        XCTAssertEqual(preparations, 1)

        let invalidYogaMeasurement = registry.measure(request: request, widthPoints: CGFloat.infinity, scale: 0)
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
            widthPoints: CGFloat.greatestFiniteMagnitude,
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
                widthPoints: CGFloat.greatestFiniteMagnitude,
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

    func testBenchmarkCensusIncludesOnlyObservedLiveArtifacts() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 1)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func layout(_ key: ProseLayoutKey, bytes: Int) -> PreparedProseLayout {
            PreparedProseLayout(
                key: key,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: bytes
            )
        }

        let directKey = key("direct")
        let pendingKey = key("pending")
        let mountedKey = key("mounted")
        let completedKey = key("completed")
        let replacementKey = key("replacement")
        let oversizedKey = key("oversized")
        let rejectedKey = key("rejected")
        let surface = FabricSurfaceToken(surfaceId: 88, componentTag: 880)

        cache.registerDirectMount("benchmark-direct", layout: layout(directKey, bytes: 2))
        _ = try cache.value(for: pendingKey, fabricSurface: surface, fabricLeaseHandle: 1) {
            layout(pendingKey, bytes: 2)
        }
        _ = try cache.value(for: mountedKey, fabricSurface: surface, fabricLeaseHandle: 2) {
            layout(mountedKey, bytes: 2)
        }
        XCTAssertTrue(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: mountedKey.generationIdentity,
            widthPixels: mountedKey.widthPixels,
            displayScale: 2,
            leaseHandle: 2
        ) != nil)
        _ = try cache.value(for: completedKey) { layout(completedKey, bytes: 1) }

        cache.beginBenchmarkCensus()
        _ = try cache.value(for: directKey) { XCTFail("direct owner should satisfy the lookup"); return layout(directKey, bytes: 2) }
        _ = try cache.value(for: pendingKey, fabricSurface: surface, fabricLeaseHandle: 1) { XCTFail("pending owner should satisfy the lookup"); return layout(pendingKey, bytes: 2) }
        _ = try cache.value(for: mountedKey, fabricSurface: surface, fabricLeaseHandle: 2) { XCTFail("mounted owner should satisfy the lookup"); return layout(mountedKey, bytes: 2) }
        _ = try cache.value(for: completedKey) { XCTFail("completed entry should satisfy the lookup"); return layout(completedKey, bytes: 1) }
        XCTAssertThrowsError(try cache.value(for: rejectedKey) { throw NSError(domain: "benchmark", code: 1) })
        XCTAssertEqual(
            Set(cache.endBenchmarkCensus()),
            Set([directKey, pendingKey, mountedKey, completedKey])
        )

        // Re-observe the completed entry, then evict it. The replacement is
        // live; the old key, a rejected lookup, and an oversized unowned
        // artifact are not.
        cache.beginBenchmarkCensus()
        _ = try cache.value(for: completedKey) { XCTFail("completed entry should satisfy the lookup"); return layout(completedKey, bytes: 1) }
        _ = try cache.value(for: replacementKey) { layout(replacementKey, bytes: 1) }
        XCTAssertThrowsError(try cache.value(for: rejectedKey) { throw NSError(domain: "benchmark", code: 2) })
        _ = try cache.value(for: oversizedKey) { layout(oversizedKey, bytes: 2) }
        XCTAssertEqual(
            Set(cache.endBenchmarkCensus()),
            Set([replacementKey]),
            "the observed completed key was evicted; rejected and oversized unowned lookups never became resident"
        )
    }

    func testBenchmarkCensusRetainsSeededLiveKeysWithoutReverseLookup() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 8)
        let key = ProseLayoutKey(
            semanticKey: "reverse-seeded",
            widthPixels: 320,
            themeDigest: "theme",
            nativeFontRevision: 0,
            fontEnvironmentRevision: 0,
            displayScale: 2,
            attachmentRevision: 0,
            generationIdentity: "reverse-seeded",
            semanticGenerationIdentity: "reverse-seeded"
        )
        let artifact = PreparedProseLayout(
            key: key,
            size: CGSize(width: 160, height: 20),
            blocks: [],
            retainedBytes: 1
        )

        cache.beginBenchmarkCensus()
        _ = try cache.value(for: key) { artifact }
        let forwardKeys = cache.endBenchmarkCensus()

        cache.beginBenchmarkCensus(seeding: forwardKeys)

        XCTAssertEqual(
            Set(cache.endBenchmarkCensus()),
            Set([key]),
            "a reverse pass must retain a prime-pass key that remains live even when UIKit does not rebind its only cell"
        )
    }

    func testReleasingDirectMountEnforcesBudgetBeforePublishingUnmountedCache() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 3)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func artifact(_ key: ProseLayoutKey) -> PreparedProseLayout {
            PreparedProseLayout(
                key: key,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: 2
            )
        }

        let directKey = key("direct")
        let unmountedKey = key("unmounted")
        let directArtifact = try cache.value(for: directKey) { artifact(directKey) }
        cache.registerDirectMount("direct-owner", layout: directArtifact)
        _ = try cache.value(for: unmountedKey) { artifact(unmountedKey) }

        cache.releaseDirectMount("direct-owner")

        XCTAssertLessThanOrEqual(cache.retainedBytesForTesting, 3)
        XCTAssertEqual(cache.countForTesting, 1)
    }

    func testLayoutCacheRepeatedHitsPreserveExactLRURecency() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 2)
        var builds: [String: Int] = [:]
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func resolve(_ name: String) throws -> PreparedProseLayout {
            let layoutKey = key(name)
            return try cache.value(for: layoutKey) {
                builds[name, default: 0] += 1
                return PreparedProseLayout(
                    key: layoutKey,
                    size: CGSize(width: 160, height: 20),
                    blocks: [],
                    retainedBytes: 1
                )
            }
        }

        _ = try resolve("first")
        _ = try resolve("second")
        for _ in 0..<128 { _ = try resolve("first") }
        _ = try resolve("third")
        _ = try resolve("first")
        _ = try resolve("second")

        XCTAssertEqual(builds["first"], 1, "repeated hits must keep the entry most-recent")
        XCTAssertEqual(builds["second"], 2, "the untouched oldest entry must be evicted first")
        XCTAssertEqual(cache.countForTesting, 2)
    }

    func testLayoutCacheCompactsConsumedAccessPrefixBeforeRepeatedHitsGrowStorage() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 1)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func resolve(_ name: String) throws -> PreparedProseLayout {
            let layoutKey = key(name)
            return try cache.value(for: layoutKey) {
                PreparedProseLayout(
                    key: layoutKey,
                    size: CGSize(width: 160, height: 20),
                    blocks: [],
                    retainedBytes: 1
                )
            }
        }

        for index in 0..<256 { _ = try resolve("evicted-\(index)") }
        for _ in 0..<128 { _ = try resolve("evicted-255") }

        XCTAssertLessThanOrEqual(
            cache.accessOrderTokenCountForTesting,
            65,
            "a large consumed prefix must not remain allocated while the live entry is repeatedly touched"
        )
    }

    func testMountedFabricLeaseReleaseEvictsReturnedCompletedEntryBeforePublishing() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 3)
        let surface = FabricSurfaceToken(surfaceId: 1, componentTag: 1)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func artifact(_ layoutKey: ProseLayoutKey) -> PreparedProseLayout {
            PreparedProseLayout(
                key: layoutKey,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: 2
            )
        }

        let mountedKey = key("mounted")
        let completedKey = key("completed")
        _ = try cache.value(for: mountedKey, fabricSurface: surface, fabricLeaseHandle: 1) { artifact(mountedKey) }
        XCTAssertNotNil(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: mountedKey.generationIdentity,
            widthPixels: mountedKey.widthPixels,
            displayScale: 2,
            leaseHandle: 1
        ))
        _ = try cache.value(for: completedKey) { artifact(completedKey) }

        cache.releaseLease(for: surface, generationIdentity: mountedKey.generationIdentity, leaseHandle: 1)

        XCTAssertEqual(cache.retainedBytesForTesting, 2)
        XCTAssertEqual(cache.countForTesting, 1)
        var mountedRebuilds = 0
        _ = try cache.value(for: mountedKey) {
            mountedRebuilds += 1
            return artifact(mountedKey)
        }
        XCTAssertEqual(mountedRebuilds, 1)
    }

    func testFabricMountReplacementEvictsStaleMountedCompletedEntryBeforePublishing() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 4)
        let surface = FabricSurfaceToken(surfaceId: 1, componentTag: 1)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func artifact(_ layoutKey: ProseLayoutKey, retainedBytes: Int) -> PreparedProseLayout {
            PreparedProseLayout(
                key: layoutKey,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: retainedBytes
            )
        }

        let staleMountedKey = key("stale-mounted")
        let replacementKey = key("replacement")
        let completedKey = key("completed")
        _ = try cache.value(for: staleMountedKey, fabricSurface: surface, fabricLeaseHandle: 1) { artifact(staleMountedKey, retainedBytes: 3) }
        XCTAssertNotNil(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: staleMountedKey.generationIdentity,
            widthPixels: staleMountedKey.widthPixels,
            displayScale: 2,
            leaseHandle: 1
        ))
        _ = try cache.value(for: replacementKey, fabricSurface: surface, fabricLeaseHandle: 1) { artifact(replacementKey, retainedBytes: 1) }
        _ = try cache.value(for: completedKey) { artifact(completedKey, retainedBytes: 3) }

        XCTAssertNotNil(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: replacementKey.generationIdentity,
            widthPixels: replacementKey.widthPixels,
            displayScale: 2,
            leaseHandle: 1
        ))

        XCTAssertEqual(cache.retainedBytesForTesting, 4)
        XCTAssertEqual(cache.countForTesting, 2)
        var staleRebuilds = 0
        _ = try cache.value(for: staleMountedKey) {
            staleRebuilds += 1
            return artifact(staleMountedKey, retainedBytes: 3)
        }
        XCTAssertEqual(staleRebuilds, 1)
    }

    func testReplacingDirectMountEvictsReturnedCompletedEntryBeforePublishing() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 3)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func artifact(_ layoutKey: ProseLayoutKey) -> PreparedProseLayout {
            PreparedProseLayout(
                key: layoutKey,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: 2
            )
        }

        let replacedKey = key("replaced")
        let replacementKey = key("replacement")
        let completedKey = key("completed")
        let replaced = try cache.value(for: replacedKey) { artifact(replacedKey) }
        let replacement = try cache.value(for: replacementKey) { artifact(replacementKey) }
        cache.registerDirectMount("first-owner", layout: replaced)
        cache.registerDirectMount("second-owner", layout: replacement)
        _ = try cache.value(for: completedKey) { artifact(completedKey) }

        cache.registerDirectMount("first-owner", layout: replacement)

        XCTAssertEqual(cache.retainedBytesForTesting, 4)
        XCTAssertEqual(cache.countForTesting, 2)
        var replacedRebuilds = 0
        _ = try cache.value(for: replacedKey) {
            replacedRebuilds += 1
            return artifact(replacedKey)
        }
        XCTAssertEqual(replacedRebuilds, 1)
    }

    func testLayoutCacheDeduplicatesMixedOwnersAndEvictsAfterDirectRelease() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 4)
        func key(_ name: String) -> ProseLayoutKey {
            ProseLayoutKey(
                semanticKey: name,
                widthPixels: 320,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: name,
                semanticGenerationIdentity: name
            )
        }
        func artifact(_ layoutKey: ProseLayoutKey) -> PreparedProseLayout {
            PreparedProseLayout(
                key: layoutKey,
                size: CGSize(width: 160, height: 20),
                blocks: [],
                retainedBytes: 2
            )
        }

        let directKey = key("direct")
        let pendingKey = key("pending")
        let mountedKey = key("mounted")
        let completedKey = key("completed")
        let surface = FabricSurfaceToken(surfaceId: 99, componentTag: 990)

        let direct = try cache.value(for: directKey) { artifact(directKey) }
        cache.registerDirectMount("direct-owner", layout: direct)
        _ = try cache.value(for: pendingKey, fabricSurface: surface, fabricLeaseHandle: 1) { artifact(pendingKey) }
        _ = try cache.value(for: mountedKey, fabricSurface: surface, fabricLeaseHandle: 2) { artifact(mountedKey) }
        XCTAssertNotNil(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: mountedKey.generationIdentity,
            widthPixels: mountedKey.widthPixels,
            displayScale: 2,
            leaseHandle: 2
        ))
        _ = try cache.value(for: completedKey) { artifact(completedKey) }

        XCTAssertEqual(cache.retainedBytesForTesting, 8, "each identity is retained once across completed, pending, mounted, and direct roles")
        XCTAssertEqual(cache.pendingLeaseCountForTesting, 1)
        XCTAssertEqual(cache.mountedLeaseCountForTesting, 1)
        XCTAssertEqual(cache.countForTesting, 4)

        cache.releaseDirectMount("direct-owner")

        XCTAssertEqual(cache.retainedBytesForTesting, 6)
        XCTAssertEqual(cache.pendingLeaseCountForTesting, 1)
        XCTAssertEqual(cache.mountedLeaseCountForTesting, 1)
        XCTAssertEqual(cache.countForTesting, 3, "direct release must evict its newly budgeted oldest completed entry")
        var directRebuilds = 0
        _ = try cache.value(for: directKey) {
            directRebuilds += 1
            return artifact(directKey)
        }
        XCTAssertEqual(directRebuilds, 1)
    }

    func testExactPendingLeasesSurviveBytePressureUntilTheirOwnersAcquire() throws {
        let cache = PreparedProseLayoutCache(byteBudget: 1)
        let key = ProseLayoutKey(
            semanticKey: String(repeating: "a", count: 64),
            widthPixels: 320,
            themeDigest: "theme",
            nativeFontRevision: 0,
            fontEnvironmentRevision: 0,
            displayScale: 2,
            attachmentRevision: 0,
            generationIdentity: "shared",
            semanticGenerationIdentity: "shared"
        )
        let artifact = PreparedProseLayout(
            key: key,
            size: CGSize(width: 160, height: 20),
            blocks: [],
            retainedBytes: 80
        )
        let surface = FabricSurfaceToken(surfaceId: 88, componentTag: 880)
        let mounted: UInt64 = 1
        let firstPending: UInt64 = 2
        let secondPending: UInt64 = 3
        let preferred: UInt64 = 4

        _ = try cache.value(for: key, fabricSurface: surface, fabricLeaseHandle: mounted) { artifact }
        XCTAssertTrue(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: key.generationIdentity,
            widthPixels: key.widthPixels,
            displayScale: 2,
            leaseHandle: mounted
        ) === artifact)
        for handle in [firstPending, secondPending, preferred] {
            _ = try cache.value(for: key, fabricSurface: surface, fabricLeaseHandle: handle) {
                XCTFail("A live immutable artifact must be reused.")
                return artifact
            }
        }

        XCTAssertEqual(cache.pendingLeaseCountForTesting, 3)
        XCTAssertEqual(cache.mountedLeaseCountForTesting, 1)
        XCTAssertTrue(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: key.generationIdentity,
            widthPixels: key.widthPixels,
            displayScale: 2,
            leaseHandle: firstPending
        ) === artifact)
        XCTAssertTrue(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: key.generationIdentity,
            widthPixels: key.widthPixels,
            displayScale: 2,
            leaseHandle: secondPending
        ) === artifact)
        XCTAssertTrue(cache.acquireForFabricMount(
            surface: surface,
            generationIdentity: key.generationIdentity,
            widthPixels: key.widthPixels,
            displayScale: 2,
            leaseHandle: preferred
        ) === artifact)
    }

    func testStaleFabricMountMissPreservesNewerPendingWidthAndMountedArtifact() {
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
        let mountedView = PreparedProseDrawingView(frame: .zero)

        let mounted = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface)
        XCTAssertTrue(install(request, in: mountedView, surface: surface, registry: registry))
        let replacement = registry.measure(request: request, widthPoints: 140, scale: 2, fabricSurface: surface)

        registry.releaseFabricMountMiss(
            FabricGenerationToken(
                surface: surface,
                generationIdentity: canonicalFabricGenerationIdentity(request, registry: registry)
            ),
            widthPoints: 160,
            scale: 2
        )

        XCTAssertTrue(mountedView.layout === mounted)
        XCTAssertEqual(registry.pendingFabricLeaseCountForTesting, 1)
        XCTAssertEqual(registry.mountedFabricLeaseCountForTesting, 1)
        XCTAssertTrue(install(request, in: mountedView, surface: surface, registry: registry, width: 140))
        XCTAssertTrue(mountedView.layout === replacement)
    }

    func testFabricLeaseHandlePreventsStaleLifecycleFromTouchingReplacement() {
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
        let generationIdentity = canonicalFabricGenerationIdentity(request, registry: registry)
        let h1 = FabricGenerationToken(surface: surface, generationIdentity: generationIdentity, leaseHandle: 101)
        let h2 = FabricGenerationToken(surface: surface, generationIdentity: generationIdentity, leaseHandle: 202)
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: h1.leaseHandle)
        registry.registerFabricLease(surfaceId: surface.surfaceId, componentTag: surface.componentTag, leaseHandle: h2.leaseHandle)

        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: h1.leaseHandle)
        _ = registry.measure(request: request, widthPoints: 160, scale: 2, fabricSurface: surface, fabricLeaseHandle: h2.leaseHandle)

        // A delayed H1 mount can consume only H1; it cannot select H2 by
        // surface/generation or by any newest-epoch policy.
        XCTAssertTrue(install(request, in: PreparedProseDrawingView(frame: .zero), surface: surface, registry: registry, leaseHandle: h1.leaseHandle))
        registry.releaseFabricMountMiss(h1, widthPoints: 160, scale: 2)
        registry.releaseFabricGeneration(h1)
        _ = registry.measure(request: request, widthPoints: 0, scale: 2, fabricSurface: surface, fabricLeaseHandle: h1.leaseHandle)

        let replacement = PreparedProseDrawingView(frame: .zero)
        XCTAssertTrue(install(request, in: replacement, surface: surface, registry: registry, leaseHandle: h2.leaseHandle))
        XCTAssertTrue(registry.hasFabricGenerationOwnershipForTesting(h2))

        // Width replacement stays within H2 and atomically replaces only H2's
        // mounted ownership.
        let replacementLayout = registry.measure(
            request: request,
            widthPoints: 140,
            scale: 2,
            fabricSurface: surface,
            fabricLeaseHandle: h2.leaseHandle
        )
        XCTAssertTrue(install(request, in: replacement, surface: surface, registry: registry, width: 140, leaseHandle: h2.leaseHandle))
        XCTAssertTrue(replacement.layout === replacementLayout)
    }

    func testFabricLeaseHandleBridgeStaticContract() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        func source(_ path: String) throws -> String {
            try String(contentsOf: root.appendingPathComponent(path), encoding: .utf8)
        }

        let state = try source("common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerState.h")
        let shadow = try source("common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerShadowNode.cpp")
        let manager = try source("ios/Viewer/Fabric/PreparedProseMeasurementsManager.mm")
        let registry = try source("ios/Viewer/PreparedProseLayoutRegistry.swift")
        let component = try source("ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm")
        let cache = try source("ios/Viewer/PreparedProseLayoutCache.swift")
        let android = try source("android/src/main/jni/PreparedProseViewerMeasurementsManager.cpp")

        XCTAssertTrue(state.contains("uint64_t leaseHandle{0}"))
        XCTAssertTrue(state.contains("previousState.leaseHandle"))
        XCTAssertTrue(state.contains("if (data.count(key) == 0)"))
        XCTAssertTrue(state.contains("return fallback;"))
        XCTAssertTrue(state.contains("leaseLifecycle"))
        XCTAssertTrue(state.contains("~PreparedProseViewerLeaseLifecycle"))
        XCTAssertTrue(state.contains("bindTerminalCleanup"))
        XCTAssertTrue(shadow.contains("NextFabricLeaseHandle"))
        XCTAssertTrue(shadow.contains("initialStateData"))
        XCTAssertTrue(shadow.contains("state.leaseHandle = NextFabricLeaseHandle()"))
        XCTAssertLessThan(
            shadow.range(of: "initialStateData")!.lowerBound,
            shadow.range(of: "measureContent")!.lowerBound
        )
        XCTAssertFalse(shadow.contains("updateState("))
        XCTAssertFalse(shadow.contains("PendingLeaseHandles"))
        XCTAssertFalse(shadow.contains("ShadowNodeFamily*"))
        XCTAssertTrue(manager.contains("leaseHandle:leaseHandle"))
        XCTAssertTrue(manager.contains("bindLeaseLifecycle"))
        XCTAssertTrue(manager.contains("registerFabricLeaseSurfaceId"))
        XCTAssertTrue(manager.contains("!leaseLifecycle->isActive()"))
        XCTAssertTrue(android.contains("beginNativeMeasure"))
        XCTAssertTrue(android.contains("static_cast<int64_t>(leaseHandle)"))
        XCTAssertTrue(registry.contains("leaseHandle: UInt64"))
        XCTAssertTrue(registry.contains("releaseFabricLeaseSurfaceId"))
        XCTAssertTrue(registry.contains("permittedGenerationIdentity"))
        XCTAssertTrue(registry.contains("activateFabricGeneration"))
        XCTAssertTrue(registry.contains("releaseFabricInvalidMeasurement"))
        XCTAssertFalse(registry.contains("retiredFabricLeaseHandles"))
        XCTAssertTrue(component.contains("installCachedLayoutInDrawingView"))
        XCTAssertTrue(component.contains("const auto leaseHandle = LeaseHandle(_viewerState)"))
        XCTAssertTrue(component.contains("leaseHandle:leaseHandle"))
        XCTAssertTrue(component.contains("beginNewGenerationTerminatingCurrentLease:NO"))
        XCTAssertTrue(component.contains("releaseFabricOwnershipTerminatingLease:YES"))
        XCTAssertTrue(component.contains("if (terminal) {"))
        XCTAssertTrue(component.contains("DeactivateLease(_viewerState, stateLeaseHandle)"))
        XCTAssertTrue(component.contains("releaseFabricMountMissSurfaceId"))
        XCTAssertTrue(component.contains("activateFabricGenerationSurfaceId"))
        XCTAssertTrue(component.contains("leaseHandle:_ownedLeaseHandle"))
        XCTAssertTrue(cache.contains("$0.leaseHandle == leaseHandle"))
        XCTAssertTrue(cache.contains("fabricGenerations(for surface"))
        XCTAssertFalse(cache.contains(".max(by: { $0.epoch < $1.epoch })"))
    }

    func testFabricStateLifecycleUsesOneHandleAcrossOrdinaryRevisionsStaticContract() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let component = try String(
            contentsOf: root.appendingPathComponent("ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm"),
            encoding: .utf8
        )
        let shadow = try String(
            contentsOf: root.appendingPathComponent("common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerShadowNode.cpp"),
            encoding: .utf8
        )
        XCTAssertTrue(component.contains("beginNewGenerationTerminatingCurrentLease:NO"))
        XCTAssertTrue(component.contains("leaseChanged"))
        XCTAssertTrue(component.contains("beginNewGenerationTerminatingCurrentLease:leaseChanged"))
        XCTAssertTrue(component.contains("releaseFabricOwnershipTerminatingLease:YES"))
        XCTAssertTrue(shadow.contains("std::numeric_limits<int64_t>::max()"))
        XCTAssertTrue(shadow.contains("for (;;)"))
        XCTAssertTrue(shadow.contains("compare_exchange_weak"))
        XCTAssertTrue(shadow.contains("Restart from that value"))
        XCTAssertTrue(shadow.contains("return 0;"))
    }

    func testFabricComponentRetriesInstallationAfterWindowAttachmentStaticContract() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let component = try String(
            contentsOf: root.appendingPathComponent("ios/Viewer/Fabric/PREPPreparedProseViewerComponentView.mm"),
            encoding: .utf8
        )
        let callback = try XCTUnwrap(component.range(of: "- (void)didMoveToWindow"))
        let installGate = try XCTUnwrap(
            component.range(
                of: "[self installMeasuredArtifactIfAttached]",
                range: callback.lowerBound ..< component.endIndex
            )
        )

        XCTAssertLessThan(callback.lowerBound, installGate.lowerBound)
    }

    func testNeverMountedFabricStateBindsExactTerminalCleanupStaticContract() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let state = try String(
            contentsOf: root.appendingPathComponent("common/cpp/react/renderer/components/PreparedProseViewer/PreparedProseViewerState.h"),
            encoding: .utf8
        )
        let bridge = try String(
            contentsOf: root.appendingPathComponent("ios/Viewer/Fabric/PreparedProseMeasurementsManager.mm"),
            encoding: .utf8
        )
        XCTAssertTrue(state.contains("final snapshot dies"))
        XCTAssertTrue(bridge.contains("releaseFabricLeaseSurfaceId"))
        XCTAssertTrue(bridge.contains("leaseLifecycle->bindTerminalCleanup"))
    }

    func testAndroidNativeStateKeepsExactLeaseHandleStaticContract() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let manager = try String(
            contentsOf: root.appendingPathComponent("android/src/main/java/com/apollohg/editor/viewer/PreparedProseViewerManager.kt"),
            encoding: .utf8
        )
        let cache = try String(
            contentsOf: root.appendingPathComponent("android/src/main/java/com/apollohg/editor/viewer/PreparedProseLayoutCache.kt"),
            encoding: .utf8
        )
        let jni = try String(
            contentsOf: root.appendingPathComponent("android/src/main/jni/PreparedProseViewerMeasurementsManager.cpp"),
            encoding: .utf8
        )
        XCTAssertTrue(manager.contains("stringOrNull(\"leaseHandle\")?.toLongOrNull()"))
        XCTAssertTrue(manager.contains("incoming.copy(leaseHandle = prior.leaseHandle)"))
        XCTAssertTrue(manager.contains("FabricLeaseHandleBridge.currentHandle()"))
        XCTAssertTrue(cache.contains("pendingLeases"))
        XCTAssertTrue(cache.contains("mountedLeases"))
        XCTAssertFalse(cache.contains("completed[mountIndex"))
        XCTAssertTrue(jni.contains("getStaticMethod<void(jlong)>(\"beginNativeMeasure\")"))
        XCTAssertTrue(jni.contains("registerNativeLease"))
        XCTAssertTrue(jni.contains("finalizeNativeLease"))
        XCTAssertTrue(jni.contains("global_ref<facebook::jni::JClass>"))
        XCTAssertFalse(jni.contains("alias_ref<facebook::jni::JClass>"))
        XCTAssertTrue(jni.contains("make_global"))
        XCTAssertTrue(jni.contains("processLifetime"))
        XCTAssertTrue(jni.contains("process-lifetime"))
        XCTAssertFalse(jni.contains("bridge.reset()"))
        XCTAssertTrue(jni.contains("facebook::jni::ThreadScope"))
        XCTAssertTrue(jni.contains("Every object allocation, class lookup, and Java invocation below can run"))
        XCTAssertTrue(jni.contains("Still inside ThreadScope"))
        XCTAssertTrue(jni.contains("std::to_string(static_cast<int64_t>(leaseHandle))"))
        XCTAssertTrue(jni.contains("folly::dynamic localData"))
        XCTAssertTrue(jni.contains("beginNativeMeasure_(bridgeClass_"))
        XCTAssertTrue(jni.contains("endNativeMeasure_(bridgeClass_)"))
    }

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
        _ = viewer.sizeThatFits(CGSize(width: 160, height: CGFloat.greatestFiniteMagnitude))
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
        width: CGFloat = 160,
        leaseHandle: UInt64 = 1
    ) -> Bool {
        registry.installCachedLayout(
            in: drawingView,
            surfaceId: surface.surfaceId,
            componentTag: surface.componentTag,
            leaseHandle: leaseHandle,
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

    private func canonicalFabricGenerationIdentity(
        _ request: ProseViewerRequest,
        registry: PreparedProseLayoutRegistry
    ) -> String {
        registry.fabricGenerationIdentity(
            sourceKind: "json",
            source: request.source.value as NSString,
            configJSON: request.configuration.configJSON as NSString,
            themeJSON: request.configuration.themeJSON as NSString?,
            imagePolicyJSON: request.configuration.imagePolicyJSON as NSString?,
            imagesEnabled: request.configuration.imagesEnabled,
            collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
            attachmentRevision: request.attachmentRevision,
            nativeFontRevision: request.nativeFontRevision,
            fontEnvironmentRevision: request.fontEnvironmentRevision
        ) as String
    }

    private func registerAndActivateFabricGeneration(
        _ request: ProseViewerRequest,
        surface: FabricSurfaceToken,
        registry: PreparedProseLayoutRegistry,
        leaseHandle: UInt64
    ) -> FabricGenerationToken {
        let generation = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(request, registry: registry),
            leaseHandle: leaseHandle
        )
        registry.registerFabricLease(
            surfaceId: surface.surfaceId,
            componentTag: surface.componentTag,
            leaseHandle: leaseHandle
        )
        registry.activateFabricGeneration(generation)
        return generation
    }

    private final class FailureRecordingDelegate: ProseViewerInteractionDelegate {
        var errors: [ProseViewerError] = []
        var mentions: [ProseViewerMention] = []

        func proseViewer(_ view: ProseViewerView, didTapLink href: String, text: String) {}
        func proseViewer(_ view: ProseViewerView, didTapMention mention: ProseViewerMention) {
            mentions.append(mention)
        }
        func proseViewer(_ view: ProseViewerView, didFail error: ProseViewerError) {
            errors.append(error)
        }
    }
}

// Direct registry tests model the C++ state-family bridge explicitly. Calls
// that omit an opaque handle use the test's canonical H1 family.
extension PreparedProseLayoutRegistry {
    func measure(
        request: ProseViewerRequest,
        widthPoints: CGFloat,
        scale: CGFloat,
        fabricSurface: FabricSurfaceToken
    ) -> PreparedProseLayout {
        registerFabricLease(
            surfaceId: fabricSurface.surfaceId,
            componentTag: fabricSurface.componentTag,
            leaseHandle: 1
        )
        return measure(
            request: request,
            widthPoints: widthPoints,
            scale: scale,
            fabricSurface: fabricSurface,
            fabricLeaseHandle: 1
        )
    }
}
