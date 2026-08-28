import XCTest
import UIKit
import CoreText

final class PreparedProseRevisionTests: XCTestCase {
    private enum FixtureError: Error { case expected }

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

    private func imageLayout(attachments: [ViewerImageAttachment]) -> PreparedProseLayout {
        PreparedProseLayout(
            key: ProseLayoutKey(
                semanticKey: "image-layout",
                widthPixels: 400,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: "image-layout",
                semanticGenerationIdentity: "image-layout"
            ),
            size: CGSize(width: 200, height: 1_600),
            blocks: [],
            imageAttachments: attachments,
            retainedBytes: 0
        )
    }

    private func flushMain(until condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(1)
        repeat {
            let flushed = expectation(description: "flush main queue")
            DispatchQueue.main.async { flushed.fulfill() }
            wait(for: [flushed], timeout: 1)
        } while !condition() && Date() < deadline
    }

    private func imageDataURI() -> String {
        let image = UIGraphicsImageRenderer(size: CGSize(width: 1, height: 1)).image { context in
            UIColor.black.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 1, height: 1))
        }
        return "data:image/png;base64,\(image.pngData()!.base64EncodedString())"
    }

    func testFabricMeasurementScopeCannotConsultAnotherSurfaceSidecar() {
        let first = FabricSurfaceToken(surfaceId: 91, componentTag: 1)
        let second = FabricSurfaceToken(surfaceId: 92, componentTag: 1)
        let id = "7:https://example.test/a"
        defer {
            FabricAttachmentSidecars.remove(first)
            FabricAttachmentSidecars.remove(second)
        }
        let outgoing = FabricAttachmentSidecars.begin(first, semanticIdentity: "outgoing")
        outgoing.admit(attachmentCount: 1)
        XCTAssertTrue(outgoing.recordIntrinsicSize(CGSize(width: 10, height: 20), for: id, ordinal: 0, declaredSize: nil))
        let incoming = FabricAttachmentSidecars.begin(second, semanticIdentity: "incoming")
        incoming.admit(attachmentCount: 1)
        ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting(1)
        defer { ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting() }
        XCTAssertNil(FabricAttachmentSidecars.withMeasurementState(incoming) {
            ViewerImageIntrinsicStore.shared.size(for: id)
        })
    }

    func testConcurrentFabricMeasurementScopesKeepEvictedIntrinsicMetadataSurfaceLocalAndCleanUp() throws {
        let first = FabricSurfaceToken(surfaceId: 91, componentTag: 1)
        let second = FabricSurfaceToken(surfaceId: 92, componentTag: 1)
        // This deliberately collides an attachment identity across semantic
        // source/revision states; only the stable Fabric token may select it.
        let id = "7:https://example.test/shared"
        let ready = DispatchGroup()
        ready.enter()
        ready.enter()
        let proceed = DispatchSemaphore(value: 0)
        let completed = expectation(description: "concurrent Fabric measurements")
        completed.expectedFulfillmentCount = 2
        let resultLock = NSLock()
        var firstResult: CGSize?
        var secondResult: CGSize?
        ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting(1)
        defer {
            FabricAttachmentSidecars.remove(first)
            FabricAttachmentSidecars.remove(second)
            ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting()
        }

        let firstState = FabricAttachmentSidecars.begin(first, semanticIdentity: "source-a-revision-1")
        firstState.admit(attachmentCount: 1)
        XCTAssertTrue(firstState.recordIntrinsicSize(CGSize(width: 80, height: 40), for: id, ordinal: 0, declaredSize: nil))
        let secondState = FabricAttachmentSidecars.begin(second, semanticIdentity: "source-b-revision-2")
        secondState.admit(attachmentCount: 1)
        XCTAssertTrue(secondState.recordIntrinsicSize(CGSize(width: 30, height: 60), for: id, ordinal: 0, declaredSize: nil))
        XCTAssertEqual(firstState.revision, 1)
        XCTAssertEqual(secondState.revision, 1)
        ViewerImageIntrinsicStore.shared.store(CGSize(width: 1, height: 1), for: "8:https://example.test/evict")
        XCTAssertNil(ViewerImageIntrinsicStore.shared.globalSize(for: id))

        DispatchQueue.global().async {
            let size = FabricAttachmentSidecars.withMeasurementState(firstState) {
                ready.leave()
                _ = proceed.wait(timeout: .now() + 1)
                return ViewerImageIntrinsicStore.shared.size(for: id)
            }
            resultLock.lock()
            firstResult = size
            resultLock.unlock()
            completed.fulfill()
        }
        DispatchQueue.global().async {
            let size = FabricAttachmentSidecars.withMeasurementState(secondState) {
                ready.leave()
                _ = proceed.wait(timeout: .now() + 1)
                return ViewerImageIntrinsicStore.shared.size(for: id)
            }
            resultLock.lock()
            secondResult = size
            resultLock.unlock()
            completed.fulfill()
        }
        XCTAssertEqual(ready.wait(timeout: .now() + 1), .success)
        proceed.signal()
        proceed.signal()
        wait(for: [completed], timeout: 1)
        XCTAssertEqual(firstResult, CGSize(width: 80, height: 40))
        XCTAssertEqual(secondResult, CGSize(width: 30, height: 60))

        try FabricAttachmentSidecars.withMeasurementState(firstState) {
            XCTAssertThrowsError(try FabricAttachmentSidecars.withMeasurementState(secondState) {
                throw FixtureError.expected
            })
            XCTAssertTrue(FabricAttachmentSidecars.currentMeasurementState === firstState)
        }
        XCTAssertNil(FabricAttachmentSidecars.currentMeasurementState)

        FabricAttachmentSidecars.remove(first)
        FabricAttachmentSidecars.remove(second)
        XCTAssertNil(FabricAttachmentSidecars.state(for: first))
        XCTAssertNil(FabricAttachmentSidecars.state(for: second))
    }

    func testBoundedIntrinsicMetadataEvictsOldestEntryDeterministically() {
        let store = ViewerImageIntrinsicStore(entryLimit: 2)
        store.store(CGSize(width: 10, height: 10), for: "a")
        store.store(CGSize(width: 20, height: 20), for: "b")
        store.store(CGSize(width: 30, height: 30), for: "c")
        XCTAssertNil(store.size(for: "a"))
        XCTAssertEqual(store.size(for: "b"), CGSize(width: 20, height: 20))
        XCTAssertEqual(store.size(for: "c"), CGSize(width: 30, height: 30))
    }

    func testStaleImageGenerationCannotDeliverPixelsOrMetadata() {
        let pipeline = ViewerImagePipeline(policy: .default)
        pipeline.begin(generation: "first", imagesEnabled: true)
        pipeline.begin(generation: "second", imagesEnabled: true)
        XCTAssertFalse(pipeline.acceptsCompletion(generation: "first"))
        XCTAssertTrue(pipeline.acceptsCompletion(generation: "second"))
    }

    func testMissingFamilyWarningSurvivesFontEnvironmentReplacementButNewSemanticGenerationWarns() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        XCTAssertTrue(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "a"))
        XCTAssertFalse(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "a"))
        environment.invalidateRegisteredFonts()
        XCTAssertFalse(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "a"))
        XCTAssertTrue(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "b"))
    }

    func testGenericAliasesResolveSilentlyAndInlineCodeUsesMonospacedSystemFont() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        let semanticGeneration = "generic-fonts-\(UUID().uuidString)"
        let fallback = UIFont.systemFont(ofSize: 17)
        let inlineCode = environment.resolveFont(
            family: "monospace",
            size: 17,
            fallback: fallback,
            additionalTraits: [.traitBold, .traitItalic],
            semanticGeneration: semanticGeneration
        )

        XCTAssertTrue(inlineCode.fontDescriptor.symbolicTraits.contains(.traitMonoSpace))
        XCTAssertTrue(inlineCode.fontDescriptor.symbolicTraits.contains(.traitBold))
        XCTAssertTrue(inlineCode.fontDescriptor.symbolicTraits.contains(.traitItalic))
        XCTAssertFalse(
            environment.hasMissingFamilyWarning("monospace", semanticGeneration: semanticGeneration),
            "generic monospace must not enter the missing-family warning registry"
        )

        for alias in ["system-ui", "sans-serif", "serif", "ui-sans-serif", "sans-serif-condensed", "cursive"] {
            _ = environment.resolveFont(
                family: alias,
                size: 17,
                fallback: fallback,
                semanticGeneration: semanticGeneration
            )
            XCTAssertFalse(
                environment.hasMissingFamilyWarning(alias, semanticGeneration: semanticGeneration),
                "generic alias \(alias) must resolve without a missing-family warning"
            )
        }
    }

    func testHeadingInheritsCustomBaseFamilyWhileApplyingDefaultBold() {
        let base = UIFont(name: "Courier", size: 17)!
        let theme = PreparedProseTheme.resolve(
            themeJSON: #"{"text":{"fontFamily":"Courier"},"paragraph":{"fontSize":18},"blockquote":{"text":{"fontStyle":"italic"}},"codeBlock":{"text":{"fontSize":16}}}"#,
            semanticGeneration: "heading-inheritance-\(UUID().uuidString)"
        )
        let heading = theme.headings["h1"]!

        XCTAssertEqual(theme.paragraph.font.familyName, base.familyName)
        XCTAssertEqual(theme.blockquote.font.familyName, base.familyName)
        XCTAssertEqual(theme.code.font.familyName, base.familyName)
        XCTAssertEqual(heading.font.familyName, base.familyName)
        XCTAssertTrue(heading.font.fontDescriptor.symbolicTraits.contains(.traitBold))
    }

    func testExplicitHeadingFamilyOverridesInheritedBaseFamily() {
        let expected = UIFont(name: "Courier-Bold", size: 32)!
        let theme = PreparedProseTheme.resolve(
            themeJSON: #"{"text":{"fontFamily":"Courier"},"headings":{"h1":{"fontFamily":"Courier-Bold"}}}"#,
            semanticGeneration: "heading-override-\(UUID().uuidString)"
        )

        XCTAssertEqual(theme.headings["h1"]?.font.fontName, expected.fontName)
    }

    func testMissingLiteralFamilyWarnsOnceAndFallsBack() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        let family = "missing-literal-font-\(UUID().uuidString)"
        let semanticGeneration = "missing-literal-\(UUID().uuidString)"
        let fallback = UIFont(name: "Courier", size: 17)!

        let first = environment.resolveFont(
            family: family,
            size: 17,
            fallback: fallback,
            semanticGeneration: semanticGeneration
        )
        let second = environment.resolveFont(
            family: family,
            size: 17,
            fallback: fallback,
            semanticGeneration: semanticGeneration
        )

        XCTAssertEqual(first.fontName, fallback.fontName)
        XCTAssertEqual(second.fontName, fallback.fontName)
        XCTAssertTrue(
            environment.hasMissingFamilyWarning(family, semanticGeneration: semanticGeneration),
            "two literal-name resolutions in one semantic generation must emit only one warning"
        )
    }

    func testThemeFamiliesUseOneSemanticWarningAcrossStylesAndLayoutRevisions() {
        let family = "missing-theme-family-\(UUID().uuidString)"
        let semanticA = "theme-warning-a-\(UUID().uuidString)"
        let semanticB = "theme-warning-b-\(UUID().uuidString)"
        let theme = """
        {"text":{"fontFamily":"\(family)"},"paragraph":{"fontFamily":"\(family)"},"blockquote":{"text":{"fontFamily":"\(family)"}},"codeBlock":{"text":{"fontFamily":"\(family)"}},"headings":{"h1":{"fontFamily":"\(family)"},"h2":{"fontFamily":"\(family)"},"h3":{"fontFamily":"\(family)"},"h4":{"fontFamily":"\(family)"},"h5":{"fontFamily":"\(family)"},"h6":{"fontFamily":"\(family)"}},"links":{"fontFamily":"\(family)"}}
        """

        _ = PreparedProseTheme.resolve(themeJSON: theme, semanticGeneration: semanticA)
        XCTAssertFalse(ViewerFontEnvironment.shared.shouldWarnForMissingFamily(family, semanticGeneration: semanticA))
        _ = PreparedProseTheme.resolve(themeJSON: theme, fontScale: 1.4, semanticGeneration: semanticA)
        XCTAssertFalse(ViewerFontEnvironment.shared.shouldWarnForMissingFamily(family, semanticGeneration: semanticA))
        _ = PreparedProseTheme.resolve(themeJSON: theme, semanticGeneration: semanticB)
        XCTAssertFalse(ViewerFontEnvironment.shared.shouldWarnForMissingFamily(family, semanticGeneration: semanticB))
    }

    func testValidThemeFamilyRemainsSilentAndCustomFallbackPreservesBoldItalic() {
        let validSemantic = "valid-theme-family-\(UUID().uuidString)"
        _ = PreparedProseTheme.resolve(
            themeJSON: #"{"paragraph":{"fontFamily":"Courier"}}"#,
            semanticGeneration: validSemantic
        )
        XCTAssertTrue(ViewerFontEnvironment.shared.shouldWarnForMissingFamily("Courier", semanticGeneration: validSemantic))

        let missingFamily = "missing-custom-family-\(UUID().uuidString)"
        let fallback = UIFont(name: "Courier", size: 17)!
        let resolved = ViewerFontEnvironment.shared.resolveFont(
            family: missingFamily,
            size: 17,
            fallback: fallback,
            additionalTraits: [.traitBold, .traitItalic],
            semanticGeneration: "custom-fallback-\(UUID().uuidString)"
        )
        XCTAssertTrue(resolved.fontDescriptor.symbolicTraits.contains(.traitBold))
        XCTAssertTrue(resolved.fontDescriptor.symbolicTraits.contains(.traitItalic))
    }

    /// An inherited custom fallback may remain in use only when it supplies
    /// every requested emphasis trait. A face without those traits must take
    /// the deterministic system fallback instead of returning partial bold or
    /// italic styling.
    func testInheritedCustomFallbackMissingEmphasisUsesSystemFallback() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        let custom = UIFont(name: "AppleColorEmoji", size: 17)!
        let requested: UIFontDescriptor.SymbolicTraits = [.traitBold, .traitItalic]
        let resolved = environment.resolveFont(
            family: nil,
            size: 17,
            fallback: custom,
            additionalTraits: requested,
            semanticGeneration: "inherited-custom-missing-emphasis"
        )

        XCTAssertTrue(ViewerFontEnvironment.satisfiesRequestedEmphasis(resolved, requestedTraits: requested))
        XCTAssertNotEqual(resolved.familyName, custom.familyName)
    }

    /// A registered custom family remains authoritative when it can express
    /// the requested pair, while an explicit family that cannot does not
    /// silently return an incomplete face.
    func testExplicitCustomFamilyTraitResolutionPrefersCompleteFaceOrFallsBack() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        let requested: UIFontDescriptor.SymbolicTraits = [.traitBold, .traitItalic]
        let complete = environment.resolveFont(
            family: "Courier",
            size: 17,
            fallback: UIFont.systemFont(ofSize: 17),
            additionalTraits: requested,
            semanticGeneration: "explicit-custom-complete"
        )
        let incomplete = environment.resolveFont(
            family: "AppleColorEmoji",
            size: 17,
            fallback: UIFont.systemFont(ofSize: 17),
            additionalTraits: requested,
            semanticGeneration: "explicit-custom-incomplete"
        )

        XCTAssertEqual(complete.familyName, "Courier")
        XCTAssertTrue(ViewerFontEnvironment.satisfiesRequestedEmphasis(complete, requestedTraits: requested))
        XCTAssertNotEqual(incomplete.familyName, UIFont(name: "AppleColorEmoji", size: 17)!.familyName)
        XCTAssertTrue(ViewerFontEnvironment.satisfiesRequestedEmphasis(incomplete, requestedTraits: requested))
    }

    func testGenericMonospaceUsesSupportedFallbackWithoutMissingFamilyWarning() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        let requested: UIFontDescriptor.SymbolicTraits = [.traitBold, .traitItalic]
        let resolved = environment.resolveFont(
            family: "ui-monospace",
            size: 17,
            fallback: UIFont.systemFont(ofSize: 17),
            additionalTraits: requested,
            semanticGeneration: "generic-monospace-emphasis"
        )

        // Generic aliases are a supported semantic request. UIKit's
        // descriptor traits differ by OS/font runtime, so only assert the
        // portable contract: a sized fallback is resolved without recording
        // a missing-family warning.
        XCTAssertEqual(resolved.pointSize, 17)
        XCTAssertFalse(environment.hasMissingFamilyWarning("ui-monospace", semanticGeneration: "generic-monospace-emphasis"))
    }

    func testWarningContextUsesSemanticIdentityInsteadOfLayoutReplacementIdentity() {
        let request = ProseViewerRequest(source: .json("{\"type\":\"doc\"}"), configuration: .init())
        let replacement = ProseViewerRequest(
            source: request.source,
            configuration: request.configuration,
            nativeFontRevision: 1,
            nativeFontScale: 1.4,
            fontEnvironmentRevision: 2,
            attachmentRevision: 3
        )
        let baseKey = ProseLayoutKey(
            semanticKey: String(repeating: "a", count: 64),
            widthPixels: 200,
            themeDigest: request.themeDigest,
            nativeFontRevision: request.nativeFontRevision,
            fontEnvironmentRevision: request.fontEnvironmentRevision,
            displayScale: 2,
            attachmentRevision: request.attachmentRevision,
            generationIdentity: request.generationIdentity,
            semanticGenerationIdentity: request.semanticGenerationIdentity
        )
        let replacementKey = ProseLayoutKey(
            semanticKey: baseKey.semanticKey,
            widthPixels: 240,
            themeDigest: replacement.themeDigest,
            nativeFontRevision: replacement.nativeFontRevision,
            fontEnvironmentRevision: replacement.fontEnvironmentRevision,
            displayScale: 3,
            attachmentRevision: replacement.attachmentRevision,
            generationIdentity: replacement.generationIdentity,
            semanticGenerationIdentity: replacement.semanticGenerationIdentity
        )
        XCTAssertNotEqual(baseKey.generationIdentity, replacementKey.generationIdentity)
        XCTAssertEqual(baseKey.semanticGenerationIdentity, replacementKey.semanticGenerationIdentity)
    }

    func testFabricTerminalSidecarReleaseIsIdempotentAndCannotRemoveAnotherComponent() {
        let registry = PreparedProseLayoutRegistry(compile: { _ in
            ViewerDocument(semanticKey: String(repeating: "a", count: 64), paragraphs: [], isEmpty: true, retainedBytes: 0)
        })
        let surface = FabricSurfaceToken(surfaceId: 711, componentTag: 9)
        let sibling = FabricSurfaceToken(surfaceId: 711, componentTag: 10)
        _ = FabricAttachmentSidecars.begin(surface, semanticIdentity: "semantic-a")
        _ = FabricAttachmentSidecars.begin(sibling, semanticIdentity: "semantic-b")
        registry.releaseFabricGeneration(.init(surface: surface, generationIdentity: "replacement-layout"))
        XCTAssertNotNil(FabricAttachmentSidecars.state(for: surface))

        registry.releaseFabricSurface(surface)
        XCTAssertNil(FabricAttachmentSidecars.state(for: surface))
        XCTAssertNotNil(FabricAttachmentSidecars.state(for: sibling))
        registry.releaseFabricSurface(surface)
        XCTAssertNil(FabricAttachmentSidecars.state(for: surface))
        XCTAssertNotNil(FabricAttachmentSidecars.state(for: sibling))
        registry.releaseFabricSurface(sibling)
    }

    func testExplicitFontAvailabilityAndDynamicTypeEachPublishOneReplacementRevision() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        var revisions: [UInt64] = []
        environment.onInvalidated = { revisions.append($0) }
        environment.invalidateRegisteredFonts()
        NotificationCenter.default.post(name: UIContentSizeCategory.didChangeNotification, object: nil)
        XCTAssertEqual(revisions, [1, 2])
    }

    func testDocumentedCoreTextRegisteredFontNotificationIsWiredAndContentSizeDuplicatesAreIgnored() {
        let center = NotificationCenter()
        let environment = ViewerFontEnvironment(notificationCenter: center)
        center.post(name: ViewerFontEnvironment.registeredFontsDidChangeNotification, object: nil)
        center.post(name: UIContentSizeCategory.didChangeNotification, object: nil, userInfo: [
            UIContentSizeCategory.newValueUserInfoKey: UIContentSizeCategory.accessibilityLarge,
        ])
        center.post(name: UIContentSizeCategory.didChangeNotification, object: nil, userInfo: [
            UIContentSizeCategory.newValueUserInfoKey: UIContentSizeCategory.accessibilityLarge,
        ])
        XCTAssertEqual(environment.revision, 2)
    }

    func testFontScaleChangesResolvedGeometry() {
        let base = PreparedProseTheme.resolve(themeJSON: nil, fontScale: 1)
        let scaled = PreparedProseTheme.resolve(themeJSON: nil, fontScale: 1.4)
        XCTAssertGreaterThan(scaled.paragraph.font.pointSize, base.paragraph.font.pointSize)
    }

    func testPreparedLayoutCacheSeparatesCurrentLightAndDarkAppearance() throws {
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            paragraphs: [ViewerParagraph(text: "Appearance")],
            isEmpty: false,
            retainedBytes: 64
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })
        let lightTraits = UITraitCollection(userInterfaceStyle: .light)
        let darkTraits = UITraitCollection(userInterfaceStyle: .dark)
        var light: PreparedProseLayout!
        lightTraits.performAsCurrent {
            light = registry.measure(
                request: ProseViewerRequest(source: .json("{}"), configuration: .init()),
                widthPoints: 160,
                scale: 2
            )
        }
        var dark: PreparedProseLayout!
        darkTraits.performAsCurrent {
            dark = registry.measure(
                request: ProseViewerRequest(source: .json("{}"), configuration: .init()),
                widthPoints: 160,
                scale: 2
            )
        }

        XCTAssertNotEqual(light.key.generationIdentity, dark.key.generationIdentity)
        XCTAssertEqual(light.key.semanticGenerationIdentity, dark.key.semanticGenerationIdentity)
        XCTAssertEqual(try foregroundColor(in: light), UIColor.label.resolvedColor(with: lightTraits))
        XCTAssertEqual(try foregroundColor(in: dark), UIColor.label.resolvedColor(with: darkTraits))
    }

    func testFabricAppearanceSeparatesLayoutGenerationButNotImagePublication() {
        let registry = PreparedProseLayoutRegistry(compile: { _ in
            ViewerDocument(
                semanticKey: String(repeating: "a", count: 64),
                paragraphs: [],
                isEmpty: true,
                retainedBytes: 0
            )
        })
        func generation(style: UIUserInterfaceStyle) -> String {
            registry.fabricGenerationIdentity(
                sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil,
                imagePolicyJSON: nil, imagesEnabled: true, collapsesWhenEmpty: true,
                attachmentRevision: 0, nativeFontRevision: 0, nativeFontScale: 1,
                fontEnvironmentRevision: 0, userInterfaceStyle: style.rawValue
            ) as String
        }
        func semanticGeneration(style: UIUserInterfaceStyle) -> String {
            registry.fabricSemanticGenerationIdentity(
                sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil,
                imagePolicyJSON: nil, imagesEnabled: true, collapsesWhenEmpty: true,
                attachmentRevision: 0, nativeFontRevision: 0, nativeFontScale: 1,
                fontEnvironmentRevision: 0, userInterfaceStyle: style.rawValue
            ) as String
        }

        XCTAssertNotEqual(generation(style: .light), generation(style: .dark))
        XCTAssertEqual(semanticGeneration(style: .light), semanticGeneration(style: .dark))
    }

    func testDirectViewerRepreparesWhenItsColorAppearanceChanges() throws {
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            paragraphs: [ViewerParagraph(text: "Appearance")],
            isEmpty: false,
            retainedBytes: 64
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })
        let viewer = ProseViewerView(
            frame: CGRect(x: 0, y: 0, width: 160, height: 80),
            layoutRegistry: registry
        )
        let window = UIWindow(frame: viewer.bounds)
        let host = UIViewController()
        window.rootViewController = host
        host.view.addSubview(viewer)
        window.isHidden = false
        defer { window.isHidden = true }
        viewer.overrideUserInterfaceStyle = .light
        flushMain(until: { viewer.traitCollection.userInterfaceStyle == .light })
        XCTAssertEqual(viewer.traitCollection.userInterfaceStyle, .light)
        XCTAssertTrue(viewer.apply(source: .json("{}"), configuration: .init()))
        _ = viewer.sizeThatFits(CGSize(width: 160, height: 80))
        let light = try XCTUnwrap(viewer.drawingViewForTesting.layout)

        let previousTraits = UITraitCollection(userInterfaceStyle: .light)
        viewer.overrideUserInterfaceStyle = .dark
        flushMain(until: { viewer.traitCollection.userInterfaceStyle == .dark })
        viewer.traitCollectionDidChange(previousTraits)
        XCTAssertEqual(viewer.traitCollection.userInterfaceStyle, .dark)
        _ = viewer.sizeThatFits(CGSize(width: 160, height: 80))
        let dark = try XCTUnwrap(viewer.drawingViewForTesting.layout)

        XCTAssertFalse(light === dark)
        XCTAssertEqual(
            try foregroundColor(in: dark),
            UIColor.label.resolvedColor(with: UITraitCollection(userInterfaceStyle: .dark))
        )
    }

    func testFabricInitialFontSnapshotUsesPublishedScaleBeforeFirstNativeRevision() {
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            paragraphs: [ViewerParagraph(text: "The initial Fabric font snapshot must alter geometry.")],
            isEmpty: false,
            retainedBytes: 128
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })

        func measure(nativeFontScale: CGFloat, leaseHandle: UInt64) -> CGSize {
            let generation = registry.fabricGenerationIdentity(
                sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
                imagesEnabled: true, collapsesWhenEmpty: true,
                attachmentRevision: 0, nativeFontRevision: 0, nativeFontScale: nativeFontScale,
                fontEnvironmentRevision: 0
            )
            registry.registerFabricLease(surfaceId: 41, componentTag: 7, leaseHandle: leaseHandle)
            registry.activateFabricGeneration(
                surfaceId: 41, componentTag: 7, generationIdentity: generation, leaseHandle: leaseHandle
            )
            return registry.measure(
                surfaceId: 41, componentTag: 7, leaseHandle: leaseHandle,
                sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
                imagesEnabled: true, collapsesWhenEmpty: true,
                attachmentRevision: 0, nativeFontRevision: 0, nativeFontScale: nativeFontScale,
                fontEnvironmentRevision: 0, widthPoints: 120, scale: 2
            )
        }

        let base = measure(nativeFontScale: 1, leaseHandle: 1)
        let scaled = measure(nativeFontScale: 1.6, leaseHandle: 2)

        XCTAssertGreaterThan(scaled.height, base.height)
    }

    func testFabricNativeRevisionUsesPublishedScaleForReplacementGeometry() {
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            paragraphs: [ViewerParagraph(text: "A prepared Fabric font scale must alter geometry.")],
            isEmpty: false,
            retainedBytes: 128
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })
        let baseGeneration = registry.fabricGenerationIdentity(
            sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
            imagesEnabled: true, collapsesWhenEmpty: true,
            attachmentRevision: 0, nativeFontRevision: 1, nativeFontScale: 1,
            fontEnvironmentRevision: 0
        )
        registry.registerFabricLease(surfaceId: 42, componentTag: 7, leaseHandle: 1)
        registry.activateFabricGeneration(
            surfaceId: 42, componentTag: 7, generationIdentity: baseGeneration, leaseHandle: 1
        )
        let base = registry.measure(
            surfaceId: 42, componentTag: 7, leaseHandle: 1,
            sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
            imagesEnabled: true, collapsesWhenEmpty: true,
            attachmentRevision: 0, nativeFontRevision: 1, nativeFontScale: 1,
            fontEnvironmentRevision: 0, widthPoints: 120, scale: 2
        )
        let replacementGeneration = registry.fabricGenerationIdentity(
            sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
            imagesEnabled: true, collapsesWhenEmpty: true,
            attachmentRevision: 0, nativeFontRevision: 2, nativeFontScale: 1.6,
            fontEnvironmentRevision: 0
        )
        registry.registerFabricLease(surfaceId: 42, componentTag: 7, leaseHandle: 2)
        registry.activateFabricGeneration(
            surfaceId: 42, componentTag: 7, generationIdentity: replacementGeneration, leaseHandle: 2
        )
        let replacement = registry.measure(
            surfaceId: 42, componentTag: 7, leaseHandle: 2,
            sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
            imagesEnabled: true, collapsesWhenEmpty: true,
            attachmentRevision: 0, nativeFontRevision: 2, nativeFontScale: 1.6,
            fontEnvironmentRevision: 0, widthPoints: 120, scale: 2
        )
        XCTAssertGreaterThan(replacement.height, base.height)
    }

    func testResourceFailureIsPublishedOncePerGenerationAndAttachmentWithoutSource() {
        let state = ViewerAttachmentRevisionState()
        XCTAssertTrue(state.beginSemanticGeneration("resource"))
        state.admit(attachmentCount: 1)
        XCTAssertTrue(state.recordResourceFailure(for: 0))
        XCTAssertFalse(state.recordResourceFailure(for: 0))
        // A Fabric attachment-revision reinstall cancels/reconfigures requests,
        // but remains in the same semantic generation.
        XCTAssertFalse(state.beginSemanticGeneration("resource"))
        XCTAssertFalse(state.recordResourceFailure(for: 0))
    }

    private func foregroundColor(in layout: PreparedProseLayout) throws -> UIColor {
        let line = try XCTUnwrap(layout.blocks.flatMap(\.fragments).compactMap(\.line).first)
        let run = try XCTUnwrap((CTLineGetGlyphRuns(line) as? [CTRun])?.first)
        let attributes = CTRunGetAttributes(run) as? [NSAttributedString.Key: Any]
        let color = try XCTUnwrap(
            attributes?[kCTForegroundColorAttributeName as NSAttributedString.Key]
        )
        return UIColor(cgColor: color as! CGColor)
    }
}
