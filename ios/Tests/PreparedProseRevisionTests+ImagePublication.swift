import XCTest
import UIKit
import CoreText

extension PreparedProseRevisionTests {
    func testDisabledImagesDoNotCreateAttachmentsOrRequests() {
        let pipeline = ViewerImagePipeline(policy: .default)
        pipeline.begin(generation: "disabled", imagesEnabled: false)
        pipeline.updateVisibleRect(.zero, attachments: [
            ViewerImageAttachment(id: "image", source: "https://example.test/image.png", bounds: CGRect(x: 0, y: 0, width: 20, height: 20), declaredSize: nil)
        ])
        XCTAssertEqual(pipeline.requestCountForTesting, 0)
    }

    func testImagePipelineDoesNotAcquireUntilMountedVisibleRectIsSupplied() {
        let pipeline = ViewerImagePipeline(policy: .default)
        pipeline.begin(generation: "mounted", imagesEnabled: true)
        XCTAssertEqual(pipeline.requestCountForTesting, 0)
    }

    func testZeroAndOffscreenVisibleRectsDoNotAcquireImages() {
        let pipeline = ViewerImagePipeline(policy: .default)
        pipeline.begin(generation: "visible", imagesEnabled: true)
        let attachment = ViewerImageAttachment(id: "image", source: "data:image/png;base64,", bounds: CGRect(x: 1000, y: 1000, width: 20, height: 20), declaredSize: nil)
        pipeline.updateVisibleRect(.zero, attachments: [attachment])
        pipeline.updateVisibleRect(CGRect(x: 0, y: 0, width: 20, height: 20), attachments: [attachment])
        XCTAssertEqual(pipeline.requestCountForTesting, 0)
    }

    func testImageLeavingViewportCanBeRequestedAgainWhenItReturns() {
        let pipeline = ViewerImagePipeline(policy: .default)
        pipeline.begin(generation: "viewport-return", imagesEnabled: true)
        let attachment = ViewerImageAttachment(
            ordinal: 0,
            id: "image",
            source: imageDataURI(),
            bounds: CGRect(x: 0, y: 0, width: 20, height: 20),
            declaredSize: nil
        )

        pipeline.updateVisibleRect(CGRect(x: 0, y: 0, width: 20, height: 20), attachments: [attachment])
        pipeline.updateVisibleRect(CGRect(x: 2_000, y: 2_000, width: 20, height: 20), attachments: [attachment])
        pipeline.updateVisibleRect(CGRect(x: 0, y: 0, width: 20, height: 20), attachments: [attachment])

        XCTAssertEqual(pipeline.requestCountForTesting, 2)
    }

    func testAncestorScrollRequestsNewlyVisibleImageWithoutManualRefresh() {
        let attachment = ViewerImageAttachment(
            ordinal: 0,
            id: "scroll-image",
            source: imageDataURI(),
            bounds: CGRect(x: 0, y: 1_200, width: 20, height: 20),
            declaredSize: CGSize(width: 20, height: 20)
        )
        let drawing = PreparedProseDrawingView(frame: CGRect(x: 0, y: 0, width: 200, height: 1_600))
        drawing.install(layout: imageLayout(attachments: [attachment]))
        drawing.configureImages(generation: "ancestor-scroll", imagesEnabled: true, policyJSON: nil)
        let scrollView = UIScrollView(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
        scrollView.contentSize = drawing.bounds.size
        scrollView.addSubview(drawing)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
        window.addSubview(scrollView)
        window.isHidden = false
        defer {
            drawing.cancelConfiguredImages()
            window.isHidden = true
        }

        drawing.updateConfiguredImagesForVisibleWindow()
        XCTAssertNil(drawing.imagePixels[attachment.id])
        scrollView.contentOffset.y = 1_100
        flushMain(until: { drawing.imagePixels[attachment.id] != nil })

        XCTAssertNotNil(drawing.imagePixels[attachment.id])
    }

    func testDirectViewerAncestorScrollRequestsNewlyVisibleImage() {
        let attachment = ViewerImageAttachment(
            ordinal: 0,
            id: "direct-scroll-image",
            source: imageDataURI(),
            bounds: CGRect(x: 0, y: 1_200, width: 20, height: 20),
            declaredSize: CGSize(width: 20, height: 20)
        )
        let registry = PreparedProseLayoutRegistry(
            compile: { _ in
                ViewerDocument(
                    semanticKey: String(repeating: "a", count: 64),
                    paragraphs: [],
                    isEmpty: false,
                    retainedBytes: 0
                )
            },
            prepare: { _, key, _, _ in
                PreparedProseLayout(
                    key: key,
                    size: CGSize(width: 200, height: 1_600),
                    blocks: [],
                    imageAttachments: [attachment],
                    retainedBytes: 0
                )
            }
        )
        let viewer = ProseViewerView(
            frame: CGRect(x: 0, y: 0, width: 200, height: 1_600),
            layoutRegistry: registry
        )
        let scrollView = UIScrollView(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
        scrollView.contentSize = viewer.bounds.size
        scrollView.addSubview(viewer)
        let window = UIWindow(frame: scrollView.bounds)
        window.addSubview(scrollView)
        window.isHidden = false
        defer {
            viewer.prepareForReuse()
            window.isHidden = true
        }
        XCTAssertTrue(viewer.apply(source: .json("{}"), configuration: .init()))
        viewer.setNeedsLayout()
        viewer.layoutIfNeeded()
        XCTAssertNil(viewer.drawingViewForTesting.imagePixels[attachment.id])

        scrollView.contentOffset.y = 1_100
        flushMain(until: { viewer.drawingViewForTesting.imagePixels[attachment.id] != nil })

        XCTAssertNotNil(viewer.drawingViewForTesting.imagePixels[attachment.id])
    }

    func testVisibleWindowUpdateReleasesPixelsOutsidePrefetchRange() {
        let visible = ViewerImageAttachment(
            ordinal: 1,
            id: "visible",
            source: "visible",
            bounds: CGRect(x: 0, y: 1_100, width: 20, height: 20),
            declaredSize: nil
        )
        let offscreen = ViewerImageAttachment(
            ordinal: 0,
            id: "offscreen",
            source: "offscreen",
            bounds: CGRect(x: 0, y: 0, width: 20, height: 20),
            declaredSize: nil
        )
        let drawing = PreparedProseDrawingView(frame: CGRect(x: 0, y: 0, width: 200, height: 1_600))
        drawing.install(layout: imageLayout(attachments: [offscreen, visible]))
        let scrollView = UIScrollView(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
        scrollView.contentSize = drawing.bounds.size
        scrollView.addSubview(drawing)
        scrollView.contentOffset.y = 1_100
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
        window.addSubview(scrollView)
        window.isHidden = false
        defer { window.isHidden = true }
        drawing.imagePixels = [
            offscreen.id: UIImage(),
            visible.id: UIImage(),
        ]

        drawing.updateConfiguredImagesForVisibleWindow()

        XCTAssertEqual(Set(drawing.imagePixels.keys), [visible.id])
    }

    func testWindowDetachmentReleasesMountedImagePixels() {
        let drawing = PreparedProseDrawingView(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
        drawing.install(layout: imageLayout(attachments: []))
        let window = UIWindow(frame: drawing.bounds)
        window.addSubview(drawing)
        window.isHidden = false
        drawing.imagePixels = ["mounted": UIImage()]

        drawing.removeFromSuperview()

        XCTAssertTrue(drawing.imagePixels.isEmpty)
        window.isHidden = true
    }

    func testKnownImagePixelsInvalidateWithoutAttachmentRevision() {
        let state = ViewerAttachmentRevisionState()
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "known", ordinal: 0, declaredSize: CGSize(width: 40, height: 20)))
        XCTAssertEqual(state.revision, 0)
    }

    func testUnknownImageIntrinsicSizeAdvancesRevisionOnlyOnce() {
        let state = ViewerAttachmentRevisionState()
        state.admit(attachmentCount: 1)
        XCTAssertTrue(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "unknown", ordinal: 0, declaredSize: nil))
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 80, height: 40), for: "unknown", ordinal: 0, declaredSize: nil))
        XCTAssertEqual(state.revision, 1)
    }

    func testIntrinsicMetadataDoesNotReopenAcrossAttachmentRevisionReinstall() {
        let state = ViewerAttachmentRevisionState()
        XCTAssertTrue(state.beginSemanticGeneration("semantic-a"))
        state.admit(attachmentCount: 1)
        XCTAssertTrue(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
        // Fabric installs the replacement artifact after its state revision.
        // That is not a new semantic source and must preserve publication.
        state.admit(attachmentCount: 1)
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
        XCTAssertEqual(state.revision, 1)
    }

    func testSemanticIdentityIncludesAllPublicationInputsButExcludesStateRevisions() {
        let base = ProseViewerRequest(
            source: .json("{\"type\":\"doc\"}"),
            configuration: .init(
                configJSON: "{\"mentions\":{\"prefix\":\"@\"},\"maxLines\":2,\"overflow\":\"clip\"}",
                themeJSON: "{\"paragraph\":{\"fontSize\":16}}",
                imagePolicyJSON: "{\"maxDecodedBytes\":1024}",
                imagesEnabled: true,
                collapsesWhenEmpty: true
            )
        )
        let stateRevision = ProseViewerRequest(
            source: base.source,
            configuration: base.configuration,
            nativeFontRevision: 3,
            nativeFontScale: 1.4,
            fontEnvironmentRevision: 4,
            attachmentRevision: 5
        )
        XCTAssertEqual(base.semanticGenerationIdentity, stateRevision.semanticGenerationIdentity)
        XCTAssertNotEqual(base.generationIdentity, stateRevision.generationIdentity)

        let variants = [
            ProseViewerRequest(source: .html(base.source.value), configuration: base.configuration),
            ProseViewerRequest(source: base.source, configuration: .init(configJSON: "{\"mentions\":{\"prefix\":\"#\"},\"maxLines\":2,\"overflow\":\"clip\"}", themeJSON: base.configuration.themeJSON, imagePolicyJSON: base.configuration.imagePolicyJSON, imagesEnabled: true, collapsesWhenEmpty: true)),
            ProseViewerRequest(source: base.source, configuration: .init(configJSON: base.configuration.configJSON, themeJSON: "{\"paragraph\":{\"fontSize\":18}}", imagePolicyJSON: base.configuration.imagePolicyJSON, imagesEnabled: true, collapsesWhenEmpty: true)),
            ProseViewerRequest(source: base.source, configuration: .init(configJSON: base.configuration.configJSON, themeJSON: base.configuration.themeJSON, imagePolicyJSON: "{\"maxDecodedBytes\":2048}", imagesEnabled: true, collapsesWhenEmpty: true)),
            ProseViewerRequest(source: base.source, configuration: .init(configJSON: base.configuration.configJSON, themeJSON: base.configuration.themeJSON, imagePolicyJSON: base.configuration.imagePolicyJSON, imagesEnabled: false, collapsesWhenEmpty: true)),
            ProseViewerRequest(source: base.source, configuration: .init(configJSON: base.configuration.configJSON, themeJSON: base.configuration.themeJSON, imagePolicyJSON: base.configuration.imagePolicyJSON, imagesEnabled: true, collapsesWhenEmpty: false)),
        ]
        variants.forEach { XCTAssertNotEqual(base.semanticGenerationIdentity, $0.semanticGenerationIdentity) }
    }

    func testSemanticReplacementResetsPublicationAndResourceErrorBitsExactlyOnce() {
        let state = ViewerAttachmentRevisionState()
        XCTAssertTrue(state.beginSemanticGeneration("semantic-a"))
        state.admit(attachmentCount: 1)
        XCTAssertTrue(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
        XCTAssertTrue(state.recordResourceFailure(for: 0))
        XCTAssertFalse(state.beginSemanticGeneration("semantic-a"))
        XCTAssertFalse(state.recordResourceFailure(for: 0))
        XCTAssertTrue(state.beginSemanticGeneration("semantic-b"))
        state.admit(attachmentCount: 1)
        XCTAssertEqual(state.revision, 0)
        XCTAssertTrue(state.recordResourceFailure(for: 0))
    }

    func testAllAdmittedUnknownAttachmentsBeyond256PublishOnceWithCompactBitset() {
        let state = ViewerAttachmentRevisionState()
        let count = 513
        let semanticIdentity = "semantic-byte-fixture"
        XCTAssertTrue(state.beginSemanticGeneration(semanticIdentity))
        state.admit(attachmentCount: count)
        for index in 0..<count {
            XCTAssertTrue(state.recordIntrinsicSize(CGSize(width: 1, height: 1), for: "\(index):https://example.test/image", ordinal: index, declaredSize: nil))
        }
        XCTAssertEqual(state.revision, UInt64(count))
        XCTAssertEqual(
            state.retainedPublicationBytesForTesting,
            ViewerAttachmentRevisionState.fixedRetainedBytes
                + ViewerAttachmentRevisionState.collectionRetainedBytes * 5
                + (count + 7) / 8 * 2
                + count * (MemoryLayout<CGSize>.stride + MemoryLayout<String?>.stride + MemoryLayout<Int>.stride)
                + semanticIdentity.utf8.count * 2
                + (0..<count).reduce(0) { $0 + "\($1):https://example.test/image".utf8.count * 2 }
        )
        XCTAssertEqual(state.intrinsicSize(for: count - 1), CGSize(width: 1, height: 1))
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 2, height: 2), for: "0:https://example.test/image", ordinal: 0, declaredSize: nil))
    }

    func testGlobalMetadataLRUEvictionFallsBackToOwnMeasurementSidecarWithoutRepublishing() {
        let state = ViewerAttachmentRevisionState()
        ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting(1)
        defer { ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting() }
        XCTAssertTrue(state.beginSemanticGeneration("semantic-a"))
        state.admit(attachmentCount: 1)
        XCTAssertTrue(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
        ViewerImageIntrinsicStore.shared.store(CGSize(width: 20, height: 40), for: "8:https://example.test/b")
        XCTAssertNil(ViewerImageIntrinsicStore.shared.globalSize(for: "7:https://example.test/a"))
        XCTAssertEqual(FabricAttachmentSidecars.withMeasurementState(state) {
            ViewerImageIntrinsicStore.shared.size(for: "7:https://example.test/a")
        }, CGSize(width: 40, height: 20))
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
        XCTAssertEqual(state.revision, 1)
    }

    func testMountedPixelOwnershipCountsOnlySurfaceMapEntries() {
        XCTAssertEqual(
            PreparedProseImagePixelMapAccounting.mapRetainedBytes
                + PreparedProseImagePixelMapAccounting.entryRetainedBytes * 2,
            PreparedProseImagePixelMapAccounting.retainedBytes(entryCount: 2)
        )
        XCTAssertEqual(
            PreparedProseImagePixelMapAccounting.mapRetainedBytes
                + PreparedProseImagePixelMapAccounting.entryRetainedBytes,
            PreparedProseImagePixelMapAccounting.retainedBytes(entryCount: 1)
        )
        XCTAssertEqual(PreparedProseImagePixelMapAccounting.retainedBytes(entryCount: 0), 0)
    }

}
