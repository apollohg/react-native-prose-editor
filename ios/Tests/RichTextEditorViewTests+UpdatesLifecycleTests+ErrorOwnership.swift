import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testPendingEditorUpdateRetriesCompositionDeferralThenApplies() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>First</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 0)

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Remote</p>")
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId),
              let update = editorV2RenderUpdate(
                editorId: adapter.editorId,
                mirrorScalarAnchor: nil,
                mirrorScalarHead: nil
              ).value
        else {
            XCTFail("expected atomic render snapshot")
            return
        }
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setPendingEditorUpdateJson(update)
        view.setPendingEditorUpdateEditorId(String(editorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "First")

        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Remote")
        XCTAssertEqual(internalEditorUpdateRejections(in: view), [])
        assertNoPendingEditorUpdate(in: view)
    }

    func testTask15EditorErrorEventIsExposedByTheView() {
        let eventNames = Set(Mirror(reflecting: NativeEditorExpoView()).children.compactMap(\.label))

        XCTAssertTrue(eventNames.contains("onEditorError"))
    }

    func testTask15BoundViewRoutesOneAdapterFailureWithCompleteNullFilledRecord() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let view = NativeEditorExpoView()
        var events: [[String: Any]] = []
        view.onEditorErrorForTesting = { events.append($0) }
        view.setEditorId(editorId)

        XCTAssertFalse(view.applyEditorUpdate("{malformed"))
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(events.count, 1)
        let event = events[0]
        XCTAssertEqual(event["editorId"] as? String, String(editorId))
        let error = event["error"] as? [String: Any]
        XCTAssertEqual(
            Set(error?.keys ?? Dictionary<String, Any>().keys),
            Set(["domain", "code", "message", "requestId", "operationIndex", "limit", "actual", "detailsJson"])
        )
        XCTAssertEqual(error?["domain"] as? String, "boundary")
        XCTAssertEqual(error?["code"] as? String, "FFI_RESULT_INVALID")
        XCTAssertFalse((error?["message"] as? String)?.isEmpty ?? true)
        for key in ["requestId", "operationIndex", "limit", "actual", "detailsJson"] {
            XCTAssertTrue(error?[key] is NSNull, "\(key) must be explicitly null")
        }
    }

    func testDestroyReservationBlocksAutonomousErrorCallbackEligibility() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }

        let view = NativeEditorExpoView()
        var events: [[String: Any]] = []
        view.onEditorErrorForTesting = { events.append($0) }
        view.setEditorId(editorId)

        XCTAssertTrue(NativeEditorViewRegistry.shared.reserveDestroy(editorId: editorId))
        defer { NativeEditorViewRegistry.shared.rollbackDestroy(editorId: editorId) }

        adapter.rejectExternalRenderEnvelope("destroy reservation must block autonomous callback delivery")
        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(events.isEmpty)
    }

    func testTask15EqualDistinctAdapterFailuresEachDeliverExactlyOnce() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let view = NativeEditorExpoView()
        var events: [[String: Any]] = []
        view.onEditorErrorForTesting = { events.append($0) }
        view.setEditorId(editorId)

        XCTAssertFalse(view.applyEditorUpdate("{malformed"))
        XCTAssertFalse(view.applyEditorUpdate("{malformed"))
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(events.count, 2)
        XCTAssertEqual(events[0]["editorId"] as? String, String(editorId))
        XCTAssertEqual(events[1]["editorId"] as? String, String(editorId))
        let firstError = events[0]["error"] as? [String: Any]
        let secondError = events[1]["error"] as? [String: Any]
        XCTAssertEqual(firstError?["domain"] as? String, secondError?["domain"] as? String)
        XCTAssertEqual(firstError?["code"] as? String, secondError?["code"] as? String)
        XCTAssertEqual(firstError?["message"] as? String, secondError?["message"] as? String)
    }

    func testTask15RebindAToBToACancelsLateErrorBeforeDispatch() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        let view = NativeEditorExpoView()
        var events: [[String: Any]] = []
        view.onEditorErrorForTesting = { events.append($0) }
        view.setEditorId(firstEditorId)

        XCTAssertFalse(view.applyEditorUpdate("{malformed"))
        view.setEditorId(secondEditorId)
        view.setEditorId(firstEditorId)
        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(events.isEmpty, "the old A generation must not leak through A→B→A")
        XCTAssertFalse(view.applyEditorUpdate("{malformed"))
        flushMainQueue()
        flushMainQueue()
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events[0]["editorId"] as? String, String(firstEditorId))
    }

    func testTask15DetachAndDestroyCancelQueuedAdapterFailures() {
        let detachEditorId = makeV2Editor()
        let destroyEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: detachEditorId)
            destroyV2Editor(id: destroyEditorId)
        }

        let detachedView = NativeEditorExpoView()
        var detachedEvents: [[String: Any]] = []
        detachedView.onEditorErrorForTesting = { detachedEvents.append($0) }
        let window = hostNativeEditorExpoView(detachedView)
        detachedView.setEditorId(detachEditorId)
        XCTAssertFalse(detachedView.applyEditorUpdate("{malformed"))
        detachedView.removeFromSuperview()
        window.isHidden = true

        let destroyedView = NativeEditorExpoView()
        var destroyedEvents: [[String: Any]] = []
        destroyedView.onEditorErrorForTesting = { destroyedEvents.append($0) }
        destroyedView.setEditorId(destroyEditorId)
        XCTAssertFalse(destroyedView.applyEditorUpdate("{malformed"))
        destroyV2Editor(id: destroyEditorId)
        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(detachedEvents.isEmpty)
        XCTAssertTrue(destroyedEvents.isEmpty)
    }

    func testTask15OlderViewTokenCannotClearNewerViewOwner() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let firstView = NativeEditorExpoView()
        let secondView = NativeEditorExpoView()
        var firstEvents: [[String: Any]] = []
        var secondEvents: [[String: Any]] = []
        firstView.onEditorErrorForTesting = { firstEvents.append($0) }
        secondView.onEditorErrorForTesting = { secondEvents.append($0) }
        firstView.setEditorId(editorId)
        secondView.setEditorId(editorId)

        firstView.setEditorId(0)
        XCTAssertFalse(secondView.applyEditorUpdate("{malformed"))
        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(firstEvents.isEmpty)
        XCTAssertEqual(secondEvents.count, 1)
    }

    func testOnlyCurrentNativeOwnerConsumesRemoteCommitRefresh() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let firstView = NativeEditorExpoView()
        let secondView = NativeEditorExpoView()
        firstView.setEditorId(editorId)
        secondView.setEditorId(editorId)
        let firstInitialText = firstView.richTextView.textView.textStorage.string

        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }
        let remoteMutation = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991106","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"Remote"}}"#
        )
        XCTAssertNil(remoteMutation.error)
        NativeEditorViewRegistry.shared.applyRemoteCommitRefresh(editorId: editorId)

        XCTAssertEqual(firstView.richTextView.textView.textStorage.string, firstInitialText)
        XCTAssertEqual(secondView.richTextView.textView.textStorage.string, "Remote")
    }

    func testRebindClearsPendingEditorUpdateSourceAndPayload() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>First</p>")
        _ = EditorV2Shadow.setHtml(id: secondEditorId, html: "<p>Second</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(firstEditorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 0)
        _ = EditorV2Shadow.replaceHtml(id: firstEditorId, html: "<p>Remote</p>")
        guard let firstAdapter = EditorV2Registry.adapter(forLegacyId: firstEditorId),
              let update = editorV2RenderUpdate(
                editorId: firstAdapter.editorId,
                mirrorScalarAnchor: nil,
                mirrorScalarHead: nil
              ).value
        else {
            XCTFail("expected atomic render snapshot")
            return
        }
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setPendingEditorUpdateJson(update)
        view.setPendingEditorUpdateEditorId(String(firstEditorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        XCTAssertEqual(retainedPendingEditorUpdateSourceId(in: view), String(firstEditorId))

        view.setEditorId(secondEditorId)
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
        XCTAssertNil(retainedPendingEditorUpdateSourceId(in: view))
    }

}
