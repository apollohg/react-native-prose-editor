import XCTest
import UIKit
@testable import NativeEditor

final class PreparedProseRevisionTests: XCTestCase {
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
                + ViewerAttachmentRevisionState.activeRegistrationRetainedBytes
                + semanticIdentity.utf8.count * 2
                + (0..<count).reduce(0) { $0 + "\($1):https://example.test/image".utf8.count * 2 }
        )
        XCTAssertEqual(state.intrinsicSize(for: count - 1), CGSize(width: 1, height: 1))
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 2, height: 2), for: "0:https://example.test/image", ordinal: 0, declaredSize: nil))
    }

    func testGlobalMetadataLRUEvictionFallsBackToActiveSidecarWithoutRepublishing() {
        let state = ViewerAttachmentRevisionState()
        ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting(1)
        defer { ViewerImageIntrinsicStore.shared.clearAndSetEntryLimitForTesting() }
        XCTAssertTrue(state.beginSemanticGeneration("semantic-a"))
        state.admit(attachmentCount: 1)
        XCTAssertTrue(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
        ViewerImageIntrinsicStore.shared.store(CGSize(width: 20, height: 40), for: "8:https://example.test/b")
        XCTAssertNil(ViewerImageIntrinsicStore.shared.globalSize(for: "7:https://example.test/a"))
        XCTAssertEqual(ViewerImageIntrinsicStore.shared.size(for: "7:https://example.test/a"), CGSize(width: 40, height: 20))
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
        XCTAssertEqual(state.revision, 1)
    }

    func testMountedPixelOwnershipCountsOnlySurfaceMapEntries() {
        let drawing = PreparedProseDrawingView()
        let image = paddedImage(bytesPerRow: 64, height: 2)
        drawing.imagePixels = ["first": image, "second": image]
        XCTAssertEqual(
            PreparedProseDrawingView.imagePixelMapRetainedBytes
                + PreparedProseDrawingView.imagePixelEntryRetainedBytes * 2,
            drawing.retainedImagePixelsBytesForTesting
        )
        drawing.imagePixels = ["replacement": paddedImage(bytesPerRow: 32, height: 3)]
        XCTAssertEqual(
            PreparedProseDrawingView.imagePixelMapRetainedBytes
                + PreparedProseDrawingView.imagePixelEntryRetainedBytes,
            drawing.retainedImagePixelsBytesForTesting
        )
        drawing.imagePixels = [:]
        XCTAssertEqual(drawing.retainedImagePixelsBytesForTesting, 0)
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

    private func paddedImage(bytesPerRow: Int, height: Int) -> UIImage {
        let data = Data(repeating: 0, count: bytesPerRow * height)
        let image = CGImage(
            width: 2,
            height: height,
            bitsPerComponent: 8,
            bitsPerPixel: 32,
            bytesPerRow: bytesPerRow,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
            provider: CGDataProvider(data: data as CFData)!,
            decode: nil,
            shouldInterpolate: false,
            intent: .defaultIntent
        )!
        return UIImage(cgImage: image)
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

    func testFabricUpdateThenRecycleBeforeNextMeasureRemovesOnlyItsPersistedSidecar() {
        let registry = PreparedProseLayoutRegistry(compile: { _ in
            ViewerDocument(semanticKey: String(repeating: "a", count: 64), paragraphs: [], isEmpty: true, retainedBytes: 0)
        })
        let surface = FabricSurfaceToken(surfaceId: 711, componentTag: 9)
        _ = FabricAttachmentSidecars.begin(surface, semanticIdentity: "semantic-a")
        // updateState releases the old layout generation before Yoga measures
        // the replacement; recycle must still know this stable sidecar token.
        registry.releaseFabricGeneration(.init(surface: surface, generationIdentity: "replacement-layout"))
        XCTAssertNotNil(FabricAttachmentSidecars.state(for: surface))

        registry.releaseFabricSurface(surface)
        XCTAssertNil(FabricAttachmentSidecars.state(for: surface))
        registry.releaseFabricSurface(surface)
        XCTAssertNil(FabricAttachmentSidecars.state(for: surface))
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

    func testFabricNativeRevisionUsesPublishedScaleForReplacementGeometry() {
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            paragraphs: [ViewerParagraph(text: "A prepared Fabric font scale must alter geometry.")],
            isEmpty: false,
            retainedBytes: 128
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })
        let base = registry.measure(
            surfaceId: 42, componentTag: 7,
            sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
            imagesEnabled: true, collapsesWhenEmpty: true,
            attachmentRevision: 0, nativeFontRevision: 1, nativeFontScale: 1,
            fontEnvironmentRevision: 0, widthPoints: 120, scale: 2
        )
        let replacement = registry.measure(
            surfaceId: 42, componentTag: 7,
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
}
