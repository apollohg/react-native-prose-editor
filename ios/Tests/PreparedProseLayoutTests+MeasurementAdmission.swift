import CoreText
import Foundation
import UIKit
import XCTest

extension PreparedProseLayoutTests {
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

}
