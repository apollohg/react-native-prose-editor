import XCTest
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
        state.admit(attachmentCount: 1)
        XCTAssertTrue(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
        // Fabric installs the replacement artifact after its state revision.
        // That is not a new semantic source and must preserve publication.
        state.admit(attachmentCount: 1)
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
        XCTAssertEqual(state.revision, 1)
    }

    func testAllAdmittedUnknownAttachmentsBeyond256PublishOnceWithCompactBitset() {
        let state = ViewerAttachmentRevisionState()
        let count = 513
        state.admit(attachmentCount: count)
        for index in 0..<count {
            XCTAssertTrue(state.recordIntrinsicSize(CGSize(width: 1, height: 1), for: "\(index):https://example.test/image", ordinal: index, declaredSize: nil))
        }
        XCTAssertEqual(state.revision, UInt64(count))
        XCTAssertEqual(state.retainedPublicationBytesForTesting, (count + 7) / 8 + count * (MemoryLayout<CGSize>.stride + MemoryLayout<String?>.stride))
        XCTAssertEqual(state.intrinsicSize(for: count - 1), CGSize(width: 1, height: 1))
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 2, height: 2), for: "0:https://example.test/image", ordinal: 0, declaredSize: nil))
    }

    func testGlobalMetadataLRUEvictionDoesNotRepublishTheSameSemanticGeneration() {
        let state = ViewerAttachmentRevisionState()
        let evictedMetadata = ViewerImageIntrinsicStore(entryLimit: 1)
        state.admit(attachmentCount: 1)
        XCTAssertTrue(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
        evictedMetadata.store(CGSize(width: 40, height: 20), for: "7:https://example.test/a")
        evictedMetadata.store(CGSize(width: 20, height: 40), for: "8:https://example.test/b")
        XCTAssertNil(evictedMetadata.size(for: "7:https://example.test/a"))
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "7:https://example.test/a", ordinal: 0, declaredSize: nil))
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

    func testMissingFamilyWarnsOnceUntilFontEnvironmentRevision() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        XCTAssertTrue(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "a"))
        XCTAssertFalse(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "a"))
        environment.invalidateRegisteredFonts()
        XCTAssertTrue(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "a"))
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
        let pipeline = ViewerImagePipeline(policy: .default)
        var failures = 0
        pipeline.onResourceFailure = { _ in failures += 1 }
        pipeline.begin(generation: "resource", imagesEnabled: true)
        pipeline.reportFailureForTesting(ViewerImageAttachment(id: "secret", source: "https://user:credential@example.test/a", bounds: .zero, declaredSize: nil))
        pipeline.reportFailureForTesting(ViewerImageAttachment(id: "secret", source: "https://user:credential@example.test/a", bounds: .zero, declaredSize: nil))
        XCTAssertEqual(failures, 1)
    }
}
