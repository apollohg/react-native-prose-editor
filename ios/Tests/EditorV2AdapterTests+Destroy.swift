import XCTest
import UIKit

extension EditorV2AdapterTests {
    func testUndoRedoRoundTrip() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>ab</p>")
        _ = adapter.insertText("c", atScalar: 2)
        XCTAssertEqual(documentText(adapter), "abc")

        let undone = adapter.undo()
        XCTAssertEqual(renderedText(undone), "ab")
        XCTAssertEqual(historyState(undone)["canRedo"] as? Bool, true)

        let redone = adapter.redo()
        XCTAssertEqual(renderedText(redone), "abc")
    }

    func testDestroyMidOperationsYieldsStructuredFailureWithoutCrash() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>ab</p>")
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record

        adapter.destroy()

        XCTAssertNil(adapter.insertText("x", atScalar: 0))
        XCTAssertEqual(spy.last?.domain, "lifecycle")
        XCTAssertEqual(spy.last?.code, "ENGINE_DESTROYED")

        XCTAssertNil(adapter.refreshFromRustState(mirrorSelection: nil))
        XCTAssertEqual(spy.last?.code, "ENGINE_DESTROYED")

        // Repeated destroy is safe.
        adapter.destroy()
        adapter.destroy()
    }

    func testDestroyFailureRetainsThePairAndErrorOwnerUntilRetrySucceeds() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { editorId in
                destroyAttempts += 1
                if destroyAttempts == 1 {
                    return FfiUnitResult(
                        value: nil,
                        error: FfiError(
                            domain: "operation",
                            code: "OPERATION_INVALID",
                            message: "temporary destroy failure",
                            requestId: nil,
                            operationIndex: nil,
                            limit: nil,
                            actual: nil,
                            detailsJson: nil
                        )
                    )
                }
                return editorV2Destroy(editorId: editorId)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        defer { EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId) }

        let owner = UUID()
        var deliveredErrors: [FfiError] = []
        adapter.bindAutonomousErrorOwner(token: owner) { deliveredErrors.append($0) }

        let firstError = EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId)
        XCTAssertEqual(firstError?.code, "OPERATION_INVALID")
        XCTAssertFalse(adapter.isDestroyed)
        XCTAssertTrue(adapter.isAutonomousErrorOwner(token: owner))
        XCTAssertTrue(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId) === adapter)

        adapter.rejectExternalRenderEnvelope("pair must remain live after destroy failure")
        XCTAssertEqual(deliveredErrors.count, 1)

        XCTAssertNil(EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId))
        XCTAssertTrue(adapter.isDestroyed)
        XCTAssertFalse(adapter.isAutonomousErrorOwner(token: owner))
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))
        XCTAssertEqual(destroyAttempts, 2)

        adapter.rejectExternalRenderEnvelope("destroyed adapter must not deliver again")
        XCTAssertEqual(deliveredErrors.count, 1)
    }

    func testDestroyWithNeitherValueNorErrorRetainsThePairUntilRetrySucceeds() {
        assertMalformedDestroyResultRetainsPairUntilRetry(
            FfiUnitResult(value: nil, error: nil)
        )
    }

    func testDestroyWithBothValueAndErrorRetainsThePairUntilRetrySucceeds() {
        assertMalformedDestroyResultRetainsPairUntilRetry(
            FfiUnitResult(
                value: true,
                error: FfiError(
                    domain: "lifecycle",
                    code: "ENGINE_DESTROYED",
                    message: "malformed destroy result",
                    requestId: nil,
                    operationIndex: nil,
                    limit: nil,
                    actual: nil,
                    detailsJson: nil
                )
            )
        )
    }

    func testDestroyReservesTheViewBeforeFfiAndRollsBackAfterRetryableFailure() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        let registry = NativeEditorViewRegistry.shared
        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                destroyAttempts += 1
                XCTAssertTrue(registry.isDestroyed(editorId: handle.nativeViewId))
                XCTAssertTrue(
                    registry.prepareForCommandJSON(editorId: handle.nativeViewId)
                        .contains("\"ready\":false")
                )
                return FfiUnitResult(
                    value: nil,
                    error: FfiError(
                        domain: "operation",
                        code: "OPERATION_INVALID",
                        message: "retryable destroy failure",
                        requestId: nil,
                        operationIndex: nil,
                        limit: nil,
                        actual: nil,
                        detailsJson: nil
                    )
                )
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        registry.markEditorCreated(editorId: handle.nativeViewId)
        defer {
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            registry.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        let error = EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId)

        XCTAssertEqual(error?.code, "OPERATION_INVALID")
        XCTAssertEqual(destroyAttempts, 1)
        XCTAssertFalse(registry.isDestroyed(editorId: handle.nativeViewId))
        XCTAssertTrue(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId) === adapter)
        XCTAssertEqual(
            commandPreparation(registry.prepareForCommandJSON(editorId: handle.nativeViewId)),
            nil
        )
    }

    func testDestroyReservationContentionReturnsRetryableErrorThenOwnerSuccessFinalizesOnce() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        let firstFfiEntered = expectation(description: "first destroy ffi entered")
        let firstDestroyFinished = expectation(description: "first destroy finished")
        let releaseFirstFfi = DispatchSemaphore(value: 0)
        let attemptsLock = NSLock()
        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                attemptsLock.lock()
                destroyAttempts += 1
                let attempt = destroyAttempts
                attemptsLock.unlock()
                if attempt == 1 {
                    firstFfiEntered.fulfill()
                    _ = releaseFirstFfi.wait(timeout: .now() + 1)
                }
                return FfiUnitResult(value: true, error: nil)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        let viewRegistry = NativeEditorViewRegistry.shared
        viewRegistry.markEditorCreated(editorId: handle.nativeViewId)
        var finalizationChecks = 0
        viewRegistry.onFinalizeDestroyForTesting = { editorId in
            guard editorId == handle.nativeViewId else { return }
            finalizationChecks += 1
            XCTAssertNil(EditorV2Registry.adapter(forLegacyId: editorId))
            XCTAssertTrue(viewRegistry.isDestroyReserved(editorId: editorId))
            XCTAssertTrue(
                viewRegistry.prepareForCommandJSON(editorId: editorId)
                    .contains("\"ready\":false")
            )
        }
        defer {
            viewRegistry.onFinalizeDestroyForTesting = nil
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            viewRegistry.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        DispatchQueue.global().async {
            let ownerResult = destroyEditorV2FromModule(editorId: handle.handle)
            XCTAssertEqual(ownerResult.value, true)
            XCTAssertNil(ownerResult.error)
            firstDestroyFinished.fulfill()
        }
        wait(for: [firstFfiEntered], timeout: 1)

        let contentionResult = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertNil(contentionResult.value)
        XCTAssertEqual(contentionResult.error?.domain, "operation")
        XCTAssertEqual(contentionResult.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(contentionResult.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 1)
        releaseFirstFfi.signal()
        wait(for: [firstDestroyFinished], timeout: 1)

        XCTAssertEqual(destroyAttempts, 1)
        XCTAssertEqual(finalizationChecks, 1)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))
    }

    func testDestroyReservationContentionAllowsRetryAfterOwnerRollback() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        let firstFfiEntered = expectation(description: "first destroy ffi entered")
        let firstDestroyFinished = expectation(description: "first destroy rolled back")
        let releaseFirstFfi = DispatchSemaphore(value: 0)
        let attemptsLock = NSLock()
        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                attemptsLock.lock()
                destroyAttempts += 1
                let attempt = destroyAttempts
                attemptsLock.unlock()
                if attempt == 1 {
                    firstFfiEntered.fulfill()
                    _ = releaseFirstFfi.wait(timeout: .now() + 1)
                    return FfiUnitResult(
                        value: nil,
                        error: FfiError(
                            domain: "operation",
                            code: "OPERATION_INVALID",
                            message: "owner retryable destroy failure",
                            requestId: nil,
                            operationIndex: nil,
                            limit: nil,
                            actual: nil,
                            detailsJson: nil
                        )
                    )
                }
                return FfiUnitResult(value: true, error: nil)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: handle.nativeViewId)
        defer {
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        DispatchQueue.global().async {
            let ownerResult = destroyEditorV2FromModule(editorId: handle.handle)
            XCTAssertNil(ownerResult.value)
            XCTAssertEqual(ownerResult.error?.message, "owner retryable destroy failure")
            firstDestroyFinished.fulfill()
        }
        wait(for: [firstFfiEntered], timeout: 1)

        let contentionResult = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertNil(contentionResult.value)
        XCTAssertEqual(contentionResult.error?.domain, "operation")
        XCTAssertEqual(contentionResult.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(contentionResult.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 1)

        releaseFirstFfi.signal()
        wait(for: [firstDestroyFinished], timeout: 1)
        XCTAssertTrue(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId) === adapter)
        XCTAssertFalse(NativeEditorViewRegistry.shared.isDestroyed(editorId: handle.nativeViewId))
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))

        let retryResult = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(retryResult.value, true)
        XCTAssertNil(retryResult.error)
        XCTAssertEqual(destroyAttempts, 2)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))
    }

}
