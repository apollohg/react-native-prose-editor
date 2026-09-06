import XCTest
import UIKit

extension EditorV2AdapterTests {
    func testThrowingHandleReservationHookDoesNotStrandDestroyRetry() {
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
            destroySession: { _ in
                destroyAttempts += 1
                return destroyAttempts == 1
                    ? FfiUnitResult(
                        value: nil,
                        error: FfiError(
                            domain: "operation",
                            code: "OPERATION_INVALID",
                            message: "retryable",
                            requestId: nil,
                            operationIndex: nil,
                            limit: nil,
                            actual: nil,
                            detailsJson: nil
                        )
                    )
                    : FfiUnitResult(value: true, error: nil)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: handle.nativeViewId)
        let throwingHook: (UInt64) throws -> Void = { editorId in
            if editorId == handle.nativeViewId { throw TestHookError.failed }
        }
        EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = throwingHook
        defer {
            EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = nil
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        let first = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(first.error?.message, "retryable")
        XCTAssertTrue(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId) === adapter)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
        XCTAssertFalse(NativeEditorViewRegistry.shared.isDestroyReserved(editorId: handle.nativeViewId))

        EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = nil
        let retry = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(retry.value, true)
        XCTAssertNil(retry.error)
        XCTAssertEqual(destroyAttempts, 2)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
    }

    func testThrowingPairRemovalHookPreservesTerminalResultAndFinalizesDestroy() {
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
            destroySession: { _ in
                destroyAttempts += 1
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
        let throwingHook: (UInt64) throws -> Void = { editorId in
            if editorId == handle.nativeViewId { throw TestHookError.failed }
        }
        EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = throwingHook
        defer {
            EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = nil
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        let result = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(result.value, true)
        XCTAssertNil(result.error)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
        XCTAssertTrue(NativeEditorViewRegistry.shared.isDestroyed(editorId: handle.nativeViewId))

        EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = nil
        let subsequent = destroyEditorV2FromModule(
            editorId: handle.handle,
            destroy: { _ in
                destroyAttempts += 1
                return FfiUnitResult(value: true, error: nil)
            }
        )
        XCTAssertEqual(subsequent.value, true)
        XCTAssertNil(subsequent.error)
        XCTAssertEqual(destroyAttempts, 2)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
    }

    func testHandleTransactionBlocksContenderBeforePairLookup() {
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

        let reservationAcquired = expectation(description: "handle reservation acquired before pair lookup")
        let ownerFinished = expectation(description: "owner finished")
        let releaseOwner = DispatchSemaphore(value: 0)
        let attemptsLock = NSLock()
        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                attemptsLock.lock()
                destroyAttempts += 1
                attemptsLock.unlock()
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
        EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = { editorId in
            guard editorId == handle.nativeViewId else { return }
            reservationAcquired.fulfill()
            _ = releaseOwner.wait(timeout: .now() + 1)
        }
        defer {
            EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = nil
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        DispatchQueue.global().async {
            let result = destroyEditorV2FromModule(editorId: handle.handle)
            XCTAssertEqual(result.value, true)
            XCTAssertNil(result.error)
            ownerFinished.fulfill()
        }
        wait(for: [reservationAcquired], timeout: 1)

        let contender = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertNil(contender.value)
        XCTAssertEqual(contender.error?.domain, "operation")
        XCTAssertEqual(contender.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(contender.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 0)

        releaseOwner.signal()
        wait(for: [ownerFinished], timeout: 1)
        XCTAssertEqual(destroyAttempts, 1)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
    }

    func testHandleTransactionBlocksContenderAfterFfiAndAfterPairRemoval() {
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

        let ffiReturned = expectation(description: "destroy ffi returned")
        let pairRemoved = expectation(description: "pair removed before finalization")
        let ownerFinished = expectation(description: "owner finalized")
        let releaseAfterFfi = DispatchSemaphore(value: 0)
        let releaseAfterPairRemoval = DispatchSemaphore(value: 0)
        let attemptsLock = NSLock()
        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                attemptsLock.lock()
                destroyAttempts += 1
                attemptsLock.unlock()
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
        EditorV2Registry.onDestroyFfiResultReceivedForTesting = { editorId in
            guard editorId == handle.nativeViewId else { return }
            ffiReturned.fulfill()
            _ = releaseAfterFfi.wait(timeout: .now() + 1)
        }
        EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = { editorId in
            guard editorId == handle.nativeViewId else { return }
            pairRemoved.fulfill()
            _ = releaseAfterPairRemoval.wait(timeout: .now() + 1)
        }
        defer {
            EditorV2Registry.onDestroyFfiResultReceivedForTesting = nil
            EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = nil
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        DispatchQueue.global().async {
            let result = destroyEditorV2FromModule(editorId: handle.handle)
            XCTAssertEqual(result.value, true)
            XCTAssertNil(result.error)
            ownerFinished.fulfill()
        }
        wait(for: [ffiReturned], timeout: 1)

        let afterFfi = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(afterFfi.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(afterFfi.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 1)

        releaseAfterFfi.signal()
        wait(for: [pairRemoved], timeout: 1)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))

        let afterPairRemoval = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(afterPairRemoval.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(afterPairRemoval.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 1)

        releaseAfterPairRemoval.signal()
        wait(for: [ownerFinished], timeout: 1)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
    }

    func testHandleTransactionReturnsOriginalLifecycleTerminalResultForPairedAndUnpairedEditors() {
        let paired = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let pairedValue = paired.value,
              paired.error == nil,
              let pairedHandle = createdV2TestEditorHandle(pairedValue)
        else {
            XCTFail("expected paired v2 editor creation to succeed")
            return
        }
        let lifecycle = FfiError(
            domain: "lifecycle",
            code: "ENGINE_DESTROYED",
            message: "already destroyed by the engine",
            requestId: "request-7",
            operationIndex: "3",
            limit: nil,
            actual: nil,
            detailsJson: #"{"source":"test"}"#
        )
        guard let adapter = EditorV2Adapter.attach(
            editorId: pairedHandle.handle,
            roomBound: false,
            destroySession: { _ in FfiUnitResult(value: nil, error: lifecycle) }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: pairedHandle.nativeViewId)
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: pairedHandle.nativeViewId)
        defer {
            EditorV2Registry.removePairing(forLegacyId: pairedHandle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: pairedHandle.nativeViewId)
            _ = editorV2Destroy(editorId: pairedHandle.handle)
        }

        let pairedResult = destroyEditorV2FromModule(editorId: pairedHandle.handle)
        XCTAssertNil(pairedResult.value)
        XCTAssertEqual(pairedResult.error, lifecycle)
        XCTAssertTrue(adapter.isDestroyed)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: pairedHandle.nativeViewId))

        let unpairedId: UInt64 = 9_000_111
        let unpairedResult = destroyEditorV2FromModule(
            editorId: String(unpairedId),
            destroy: { _ in FfiUnitResult(value: nil, error: lifecycle) }
        )
        XCTAssertNil(unpairedResult.value)
        XCTAssertEqual(unpairedResult.error, lifecycle)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(unpairedId))
    }

}
