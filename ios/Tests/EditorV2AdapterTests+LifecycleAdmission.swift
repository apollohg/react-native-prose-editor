import XCTest
import UIKit

extension EditorV2AdapterTests {
    func testAttachRejectsNonCanonicalAndUnknownEditorIds() {
        XCTAssertNil(EditorV2Adapter.attach(editorId: "01", roomBound: false))
        XCTAssertNil(EditorV2Adapter.attach(editorId: "not-an-editor", roomBound: false))
        XCTAssertNil(EditorV2Adapter.attach(editorId: "999999", roomBound: false))
    }

    func testRequestIdExhaustionEmitsMaxOnceThenRejectsLocally() {
        let adapter = makeAdapter()
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        adapter.setNextRequestIdForTesting(UInt64.max - 1)

        let backendCallsBefore = adapter.backendEnvelopeCallCountForTesting
        XCTAssertNotNil(adapter.setContentHtml("<p>max</p>"))
        XCTAssertEqual(adapter.lastRequestIdForTesting, UInt64.max)
        XCTAssertEqual(adapter.backendEnvelopeCallCountForTesting, backendCallsBefore + 1)

        XCTAssertNil(adapter.setContentHtml("<p>must not reach backend</p>"))
        XCTAssertEqual(adapter.lastRequestIdForTesting, UInt64.max)
        XCTAssertEqual(adapter.backendEnvelopeCallCountForTesting, backendCallsBefore + 1)
        XCTAssertEqual(spy.last?.domain, "boundary")
        XCTAssertEqual(spy.last?.code, "CONFIG_INVALID")
        XCTAssertEqual(spy.last?.requestId, String(UInt64.max))
        XCTAssertEqual(documentText(adapter), "max")
    }

    func testTask15AutonomousErrorOwnerTokensProtectNewerOwnersFromStaleClears() {
        let adapter = makeAdapter()
        let firstOwner = UUID()
        let secondOwner = UUID()
        var firstErrors: [FfiError] = []
        var secondErrors: [FfiError] = []

        adapter.bindAutonomousErrorOwner(token: firstOwner) { firstErrors.append($0) }
        adapter.bindAutonomousErrorOwner(token: secondOwner) { secondErrors.append($0) }
        adapter.clearAutonomousErrorOwner(token: firstOwner)
        adapter.rejectExternalRenderEnvelope("first real adapter failure")

        XCTAssertTrue(firstErrors.isEmpty)
        XCTAssertEqual(secondErrors.count, 1)
        XCTAssertTrue(adapter.isAutonomousErrorOwner(token: secondOwner))

        adapter.clearAutonomousErrorOwner(token: secondOwner)
        adapter.rejectExternalRenderEnvelope("cleared owner must not receive failures")
        XCTAssertEqual(secondErrors.count, 1)
    }

    func testAdapterEmitsCanonicalDecimalDocumentVersion() {
        let adapter = makeAdapter()
        let update = parseObject(adapter.currentStateJSON())

        XCTAssertEqual(update["documentVersion"] as? String, "0")
        XCTAssertFalse(update["documentVersion"] is NSNumber)
    }

    func testDestroyWaitsForInFlightAdapterOperation() {
        let awarenessEntered = expectation(description: "awareness operation entered")
        let operationFinished = expectation(description: "awareness operation finished")
        let destroyEntered = expectation(description: "destroy entered")
        let destroyFinished = expectation(description: "destroy finished")
        let releaseAwareness = DispatchSemaphore(value: 0)
        let destroyEnteredSignal = DispatchSemaphore(value: 0)
        let adapter = makeAttachedAdapter(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            roomBound: true,
            destroySession: { editorId in
                destroyEnteredSignal.signal()
                destroyEntered.fulfill()
                return editorV2Destroy(editorId: editorId)
            },
            setAwarenessSelection: { _, _ in
                awarenessEntered.fulfill()
                _ = releaseAwareness.wait(timeout: .now() + 1)
                return FfiJsonResult(value: #"{"changed":false}"#, error: nil)
            },
            file: #filePath,
            line: #line
        )

        DispatchQueue.global().async {
            _ = adapter.syncSelection(anchor: 0, head: 0)
            operationFinished.fulfill()
        }
        wait(for: [awarenessEntered], timeout: 1)

        DispatchQueue.global().async {
            _ = adapter.destroyForModuleTransaction()
            destroyFinished.fulfill()
        }
        XCTAssertEqual(destroyEnteredSignal.wait(timeout: .now() + 0.05), .timedOut)

        releaseAwareness.signal()
        wait(for: [operationFinished, destroyEntered, destroyFinished], timeout: 1)
    }

    func testAdapterOperationStartedDuringDestroyDoesNotReachRust() {
        let destroyEntered = expectation(description: "destroy entered")
        let destroyFinished = expectation(description: "destroy finished")
        let operationFinished = expectation(description: "operation finished")
        let releaseDestroy = DispatchSemaphore(value: 0)
        let operationFinishedSignal = DispatchSemaphore(value: 0)
        let adapter = makeAttachedAdapter(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            roomBound: false,
            destroySession: { editorId in
                destroyEntered.fulfill()
                _ = releaseDestroy.wait(timeout: .now() + 1)
                return editorV2Destroy(editorId: editorId)
            },
            file: #filePath,
            line: #line
        )
        let backendCallsBefore = adapter.backendEnvelopeCallCountForTesting
        let resultLock = NSLock()
        var operationResult: String?

        DispatchQueue.global().async {
            _ = adapter.destroyForModuleTransaction()
            destroyFinished.fulfill()
        }
        wait(for: [destroyEntered], timeout: 1)

        DispatchQueue.global().async {
            let result = adapter.setContentHtml("<p>late</p>")
            resultLock.lock()
            operationResult = result
            resultLock.unlock()
            operationFinishedSignal.signal()
            operationFinished.fulfill()
        }
        XCTAssertEqual(operationFinishedSignal.wait(timeout: .now() + 0.05), .timedOut)

        releaseDestroy.signal()
        wait(for: [destroyFinished, operationFinished], timeout: 1)
        resultLock.lock()
        let finalResult = operationResult
        resultLock.unlock()
        XCTAssertNil(finalResult)
        XCTAssertEqual(adapter.backendEnvelopeCallCountForTesting, backendCallsBefore)
    }

}
