import XCTest
import UIKit
import CoreText

extension PreparedProseRevisionTests {
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

}
