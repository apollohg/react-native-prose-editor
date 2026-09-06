import XCTest
import CoreText

extension RenderBridgeTests {
    func testNativeCollaborationSocketUsesRustHardMessageCeiling() {
        let socket = NativeCollaborationSocket(
            url: URL(string: "wss://example.com")!,
            protocols: [],
            callbacks: CollaborationSocketCallbacks(
                didOpen: { _ in },
                didClose: { _, _ in },
                didFail: {}
            )
        )
        defer { socket.cancel(code: .goingAway, reason: nil) }

        XCTAssertEqual(socket.maximumMessageSizeForTesting, 64 * 1_024 * 1_024)
    }

    func testNativeCollaborationSocketInvalidatesSessionAfterCleanClose() {
        var invalidatedSessions = 0
        var closeCallbacks = 0
        let socket = NativeCollaborationSocket(
            url: URL(string: "wss://example.com")!,
            protocols: [],
            callbacks: CollaborationSocketCallbacks(
                didOpen: { _ in },
                didClose: { _, _ in closeCallbacks += 1 },
                didFail: {}
            ),
            finishSession: { _ in invalidatedSessions += 1 }
        )
        defer { socket.cancel(code: .goingAway, reason: nil) }
        let delegateSession = URLSession(configuration: .ephemeral)
        let delegateTask = delegateSession.webSocketTask(with: URL(string: "wss://example.com")!)
        defer { delegateSession.invalidateAndCancel() }

        socket.urlSession(
            delegateSession,
            webSocketTask: delegateTask,
            didCloseWith: .normalClosure,
            reason: nil
        )

        XCTAssertEqual(invalidatedSessions, 1)
        XCTAssertEqual(closeCallbacks, 1)
    }

    func testStructuredEditorCreationIdUsesExactIntegerSemantics() {
        XCTAssertEqual(createdEditorId(#"{"editorId":"42"}"#), "42")
        XCTAssertEqual(createdEditorId(#"{"editorId":"18446744073709551615"}"#), "18446744073709551615")
        XCTAssertNil(createdEditorId(#"{"editorId":"+1"}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":"01"}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":" 1"}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":"1 "}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":"1e3"}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":true}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":-1}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":1.5}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":1e3}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":9223372036854775808}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":18446744073709551616}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":18446744073709551615.0}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":7,"editorId":8}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":1.5,"nested":{"editorId":7}}"#))
        XCTAssertNil(createdEditorId(#"{"error":{"code":"CONFIG_INVALID"},"editorId":7}"#))
    }

    func testV2UInt32AcceptsOnlyExactFiniteIntegralNSNumberValues() {
        XCTAssertEqual(v2ExactUInt32(NSNumber(value: UInt32.max)), UInt32.max)
        XCTAssertEqual(v2ExactUInt32(NSNumber(value: 0)), 0)
        XCTAssertEqual(v2ExactUInt32(NSDecimalNumber(string: "1")), 1)
        XCTAssertEqual(v2ExactUInt32(NSDecimalNumber(string: "4294967295")), UInt32.max)
        XCTAssertNil(v2ExactUInt32(NSDecimalNumber(string: "1.0000000000000000001")))

        for invalid: NSNumber in [
            NSNumber(value: -1),
            NSNumber(value: 1.5),
            NSNumber(value: Double.nan),
            NSNumber(value: Double.infinity),
            NSNumber(value: UInt64(UInt32.max) + 1),
            NSNumber(value: Double(UInt32.max) + 0.5),
            NSNumber(value: true),
        ] {
            XCTAssertNil(v2ExactUInt32(invalid), "must reject \(invalid)")
        }
    }

    func testCollaborationTickForwardsCanonicalMaximumAndRawJsonResult() {
        var forwardedEditorId: String?
        var forwardedNowMillis: String?
        let rawValue = #"{\"nextDeadlineMillis\":null,\"renewedLocal\":false,\"expiredPeers\":[],\"outboundChanged\":false,\"peersChanged\":false}"#

        let result = v2CollaborationTickResultDictionary(
            editorId: "editor-1",
            nowMillis: "18446744073709551615"
        ) { editorId, nowMillis in
            forwardedEditorId = editorId
            forwardedNowMillis = nowMillis
            return FfiJsonResult(value: rawValue, error: nil)
        }

        XCTAssertEqual(forwardedEditorId, "editor-1")
        XCTAssertEqual(forwardedNowMillis, "18446744073709551615")
        XCTAssertEqual(result["value"] as? String, rawValue)
        XCTAssertNil(result["error"])
    }

    func testCollaborationTickRejectsMalformedNowMillisBeforeBackend() {
        var called = false

        let result = v2CollaborationTickResultDictionary(editorId: "editor-1", nowMillis: "01") { _, _ in
            called = true
            return FfiJsonResult(value: "{}", error: nil)
        }

        XCTAssertFalse(called)
        XCTAssertEqual(
            (result["error"] as? [String: Any])?["code"] as? String,
            "FFI_RESULT_INVALID"
        )
    }

    func testCollaborationTickRejectsFfiResultWithBothValueAndError() {
        let result = v2CollaborationTickResultDictionary(editorId: "editor-1", nowMillis: "1") { _, _ in
            FfiJsonResult(value: "{}", error: ffiResultError())
        }

        assertFfiResultContractFailure(result)
    }

    func testCollaborationTickRejectsFfiResultWithNeitherValueNorError() {
        let result = v2CollaborationTickResultDictionary(editorId: "editor-1", nowMillis: "1") { _, _ in
            FfiJsonResult(value: nil, error: nil)
        }

        assertFfiResultContractFailure(result)
    }

    func testCreateWithBothValueAndErrorCleansExtractableSessionWithoutRegisteringIt() {
        var cleanupHandles: [String] = []
        let result = createEditorV2SessionFromModule(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil,
            create: { _, _ in
                FfiJsonResult(value: #"{"editorId":"900001"}"#, error: self.ffiResultError())
            },
            destroy: { editorId in
                cleanupHandles.append(editorId)
                return FfiUnitResult(value: true, error: nil)
            }
        )

        assertFfiResultContractFailure(result)
        XCTAssertEqual(cleanupHandles, ["900001"])
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: 900001))
        XCTAssertEqual(
            commandPreparation(result: NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: 900001)),
            "destroyed"
        )
    }

    func testCreateWithNeitherValueNorErrorLeavesNoPairingOrCleanupAttempt() {
        var cleanupHandles: [String] = []
        let result = createEditorV2SessionFromModule(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil,
            create: { _, _ in FfiJsonResult(value: nil, error: nil) },
            destroy: { editorId in
                cleanupHandles.append(editorId)
                return FfiUnitResult(value: true, error: nil)
            }
        )

        assertFfiResultContractFailure(result)
        XCTAssertTrue(cleanupHandles.isEmpty)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: 900002))
    }

    func testCreateWithInvalidValueCleansExtractableSessionWithoutRegisteringIt() {
        var cleanupHandles: [String] = []
        let result = createEditorV2SessionFromModule(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil,
            create: { _, _ in FfiJsonResult(value: #"{"editorId":"900003","unexpected":true}"#, error: nil) },
            destroy: { editorId in
                cleanupHandles.append(editorId)
                return FfiUnitResult(value: true, error: nil)
            }
        )

        XCTAssertEqual(
            ((result["error"] as? [String: Any])?["code"] as? String),
            "FFI_RESULT_INVALID"
        )
        XCTAssertEqual(cleanupHandles, ["900003"])
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: 900003))
    }

    func testModuleSessionOwnerDestroysEveryRemainingOwnedHandleOnce() {
        let owner = NativeEditorModuleSessionOwner()
        owner.insert("42")
        owner.insert("7")
        owner.insert("42")
        owner.remove("7")
        var destroyed: [String] = []

        owner.destroyAll { editorId in
            destroyed.append(editorId)
            return FfiUnitResult(value: true, error: nil)
        }

        XCTAssertEqual(destroyed, ["42"])
        XCTAssertEqual(owner.countForTesting, 0)
    }

    func testModuleSessionOwnerTeardownRemovesLiveAdapterPairing() {
        let created = createEditorV2SessionFromModule(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created["value"] as? String,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected module session creation")
            return
        }
        let owner = NativeEditorModuleSessionOwner()
        owner.insert(handle.handle)
        defer {
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        owner.destroyAll()

        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))
        XCTAssertEqual(editorV2GetState(editorId: handle.handle).error?.code, "ENGINE_DESTROYED")
        XCTAssertEqual(owner.countForTesting, 0)
    }

    func testDestroyPairedRoomRetiresTransportWithoutDeadlockingRuntimeGate() {
        let created = createEditorV2SessionFromModule(
            configJson: #"{"initialization":{"type":"room","documentId":"doc-runtime-gate","lineageId":"lineage-runtime-gate"}}"#,
            snapshotState: nil
        )
        guard let value = created["value"] as? String,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected room session creation")
            return
        }
        defer {
            NativeCollaborationTransportRegistry.destroy(editorId: handle.nativeViewId)
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }
        let collaborationOwner = UUID()
        XCTAssertNil(NativeCollaborationTransportRegistry.configure(
            owner: collaborationOwner,
            editorId: handle.nativeViewId,
            configJSON: #"{"url":"wss://example.com","connect":false}"#
        ))
        let finished = expectation(description: "destroy finished")
        let resultLock = NSLock()
        var result: FfiUnitResult?

        DispatchQueue.global().async {
            let destroyed = destroyEditorV2FromModule(editorId: handle.handle)
            resultLock.lock()
            result = destroyed
            resultLock.unlock()
            finished.fulfill()
        }

        wait(for: [finished], timeout: 1)
        resultLock.lock()
        let finalResult = result
        resultLock.unlock()
        XCTAssertEqual(finalResult?.value, true)
        XCTAssertNil(finalResult?.error)
    }

    func testRetiringModuleDoesNotClearReplacementCollaborationState() {
        let firstOwner = UUID()
        let replacementOwner = UUID()
        var deliveredOwners: [String] = []

        NativeCollaborationTransportRegistry.setEventEmitter(owner: firstOwner) { _ in
            deliveredOwners.append("first")
        }
        NativeCollaborationTransportRegistry.setEventEmitter(owner: replacementOwner) { _ in
            deliveredOwners.append("replacement")
        }

        NativeCollaborationTransportRegistry.setEventEmitter(owner: firstOwner, nil)
        NativeCollaborationTransportRegistry.destroyAll(owner: firstOwner)
        NativeCollaborationTransportRegistry.emitForTesting(["kind": "test"])

        XCTAssertEqual(deliveredOwners, ["replacement"])
        XCTAssertEqual(
            NativeCollaborationTransportRegistry.eventEmitterOwnerForTesting,
            replacementOwner
        )
        NativeCollaborationTransportRegistry.destroyAll(owner: replacementOwner)
    }

    func testUnpairedDestroyReservesBeforeFfiAndFinalizesLifecycleAlreadyDestroyed() {
        let editorId: UInt64 = 900004
        let registry = NativeEditorViewRegistry.shared
        registry.markEditorCreated(editorId: editorId)
        defer { registry.invalidateDestroyedEditor(editorId: editorId) }

        let result = destroyUnpairedEditorV2FromModule(
            editorId: String(editorId),
            nativeViewId: editorId,
            destroy: { _ in
                XCTAssertTrue(registry.isDestroyed(editorId: editorId))
                XCTAssertTrue(
                    registry.prepareForCommandJSON(editorId: editorId)
                        .contains("\"ready\":false")
                )
                return FfiUnitResult(
                    value: nil,
                    error: FfiError(
                        domain: "lifecycle",
                        code: "ENGINE_DESTROYED",
                        message: "already gone",
                        requestId: nil,
                        operationIndex: nil,
                        limit: nil,
                        actual: nil,
                        detailsJson: nil
                    )
                )
            }
        )

        XCTAssertNil(result.value)
        XCTAssertEqual(result.error?.code, "ENGINE_DESTROYED")
        XCTAssertTrue(registry.isDestroyed(editorId: editorId))
    }

    func testUnpairedDestroyContentionReturnsRetryableErrorThenOwnerSuccessFinalizesOnce() {
        let editorId: UInt64 = 900005
        let registry = NativeEditorViewRegistry.shared
        registry.markEditorCreated(editorId: editorId)
        defer { registry.invalidateDestroyedEditor(editorId: editorId) }

        let firstFfiEntered = expectation(description: "unpaired destroy ffi entered")
        let firstDestroyFinished = expectation(description: "unpaired destroy finished")
        let releaseFirstFfi = DispatchSemaphore(value: 0)
        let attemptsLock = NSLock()
        var destroyAttempts = 0
        let destroy: (String) -> FfiUnitResult = { _ in
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

        DispatchQueue.global().async {
            let ownerResult = destroyUnpairedEditorV2FromModule(
                editorId: String(editorId),
                nativeViewId: editorId,
                destroy: destroy
            )
            XCTAssertEqual(ownerResult.value, true)
            XCTAssertNil(ownerResult.error)
            firstDestroyFinished.fulfill()
        }
        wait(for: [firstFfiEntered], timeout: 1)

        let contentionResult = destroyUnpairedEditorV2FromModule(
            editorId: String(editorId),
            nativeViewId: editorId,
            destroy: destroy
        )
        XCTAssertNil(contentionResult.value)
        XCTAssertEqual(contentionResult.error?.domain, "operation")
        XCTAssertEqual(contentionResult.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(contentionResult.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 1)

        releaseFirstFfi.signal()
        wait(for: [firstDestroyFinished], timeout: 1)
        XCTAssertEqual(destroyAttempts, 1)
        XCTAssertTrue(registry.isDestroyed(editorId: editorId))
    }

    func testUnpairedDestroyContentionAllowsRetryAfterOwnerRollback() {
        let editorId: UInt64 = 900006
        let registry = NativeEditorViewRegistry.shared
        registry.markEditorCreated(editorId: editorId)
        defer { registry.invalidateDestroyedEditor(editorId: editorId) }

        let firstFfiEntered = expectation(description: "unpaired destroy ffi entered")
        let firstDestroyFinished = expectation(description: "unpaired destroy rolled back")
        let releaseFirstFfi = DispatchSemaphore(value: 0)
        let attemptsLock = NSLock()
        var destroyAttempts = 0
        let destroy: (String) -> FfiUnitResult = { _ in
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

        DispatchQueue.global().async {
            let ownerResult = destroyUnpairedEditorV2FromModule(
                editorId: String(editorId),
                nativeViewId: editorId,
                destroy: destroy
            )
            XCTAssertEqual(ownerResult.error?.message, "owner retryable destroy failure")
            firstDestroyFinished.fulfill()
        }
        wait(for: [firstFfiEntered], timeout: 1)

        let contentionResult = destroyUnpairedEditorV2FromModule(
            editorId: String(editorId),
            nativeViewId: editorId,
            destroy: destroy
        )
        XCTAssertNil(contentionResult.value)
        XCTAssertEqual(contentionResult.error?.domain, "operation")
        XCTAssertEqual(contentionResult.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(contentionResult.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 1)

        releaseFirstFfi.signal()
        wait(for: [firstDestroyFinished], timeout: 1)
        XCTAssertFalse(registry.isDestroyed(editorId: editorId))

        let retryResult = destroyUnpairedEditorV2FromModule(
            editorId: String(editorId),
            nativeViewId: editorId,
            destroy: destroy
        )
        XCTAssertEqual(retryResult.value, true)
        XCTAssertNil(retryResult.error)
        XCTAssertEqual(destroyAttempts, 2)
        XCTAssertTrue(registry.isDestroyed(editorId: editorId))
    }

    func testDestroyReservationAcquisitionClassifiesContentionAtomically() {
        let editorId: UInt64 = 900007
        let registry = NativeEditorViewRegistry.shared
        registry.markEditorCreated(editorId: editorId)
        defer {
            registry.rollbackDestroy(editorId: editorId)
            registry.invalidateDestroyedEditor(editorId: editorId)
        }

        XCTAssertEqual(
            registry.acquireDestroyReservation(editorId: editorId),
            .reserved
        )
        XCTAssertEqual(
            registry.acquireDestroyReservation(editorId: editorId),
            .alreadyInProgress
        )
        registry.rollbackDestroy(editorId: editorId)
        XCTAssertEqual(
            registry.acquireDestroyReservation(editorId: editorId),
            .reserved
        )
    }

    func testCollaborationDetachAndReattachBridgeRawUnitResults() {
        var invoked = [String]()

        let detach = v2CollaborationUnitResultDictionary(editorId: "editor-1") { editorId in
            invoked.append("detach:\(editorId)")
            return FfiUnitResult(value: true, error: nil)
        }
        let reattach = v2CollaborationUnitResultDictionary(editorId: "editor-1") { editorId in
            invoked.append("reattach:\(editorId)")
            return FfiUnitResult(value: true, error: nil)
        }

        XCTAssertEqual(invoked, ["detach:editor-1", "reattach:editor-1"])
        XCTAssertEqual(detach["value"] as? Bool, true)
        XCTAssertNil(detach["error"])
        XCTAssertEqual(reattach["value"] as? Bool, true)
        XCTAssertNil(reattach["error"])
    }

}
