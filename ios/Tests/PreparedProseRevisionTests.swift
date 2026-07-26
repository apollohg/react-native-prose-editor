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

    func testKnownImagePixelsInvalidateWithoutAttachmentRevision() {
        let state = ViewerAttachmentRevisionState()
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "known", declaredSize: CGSize(width: 40, height: 20)))
        XCTAssertEqual(state.revision, 0)
    }

    func testUnknownImageIntrinsicSizeAdvancesRevisionOnlyOnce() {
        let state = ViewerAttachmentRevisionState()
        XCTAssertTrue(state.recordIntrinsicSize(CGSize(width: 40, height: 20), for: "unknown", declaredSize: nil))
        XCTAssertFalse(state.recordIntrinsicSize(CGSize(width: 80, height: 40), for: "unknown", declaredSize: nil))
        XCTAssertEqual(state.revision, 1)
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
}
