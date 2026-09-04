import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testBindingAdoptsExactlyOneAtomicSnapshotForViewAndToolbar() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected the v2 adapter paired to the native editor")
            return
        }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Bound</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        let renderCallsBefore = adapter.renderUpdateCallCountForTesting
        view.setEditorId(editorId)

        XCTAssertEqual(
            adapter.renderUpdateCallCountForTesting,
            renderCallsBefore + 1,
            "binding must use one atomic render snapshot for both cache adoption and toolbar state"
        )
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Bound")
    }

    func testPendingEditorUpdateValidatesBeforeSupersession() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>a</p>")
        let oldRender = try XCTUnwrap(editorV2RenderUpdate(
            editorId: String(editorId),
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value)
        let errors = AutonomousErrorEventSink()
        let view = NativeEditorExpoView()
        view.onEditorErrorForTesting = errors.record
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        let current = EditorV2Shadow.insertTextScalar(id: editorId, scalarPos: 1, text: "b")
        view.richTextView.textView.applyUpdateJSON(current)
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "ab")

        view.setPendingEditorUpdateJson(oldRender)
        view.setPendingEditorUpdateEditorId(String(editorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "ab")
        XCTAssertTrue(errors.errors.isEmpty)

        var malformed = parseJSONObject(oldRender)
        malformed.removeValue(forKey: "historyState")
        view.setPendingEditorUpdateJson(try encodedJSONObject(malformed))
        view.setPendingEditorUpdateEditorId(String(editorId))
        view.setPendingEditorUpdateRevision(2)
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "ab")
        XCTAssertEqual(errors.errors.count, 1)
        XCTAssertEqual(errors.errors.first?.code, "FFI_RESULT_INVALID")
    }

    func testPendingEditorUpdateAppliesEqualNewerAndReboundEditorSnapshots() throws {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>first</p>")
        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(firstEditorId)
        let current = EditorV2Shadow.insertTextScalar(
            id: firstEditorId,
            scalarPos: 5,
            text: "!"
        )
        view.richTextView.textView.applyUpdateJSON(current)

        _ = EditorV2Shadow.setSelectionScalar(
            id: firstEditorId,
            scalarAnchor: 2,
            scalarHead: 2
        )
        let equalRender = try XCTUnwrap(editorV2RenderUpdate(
            editorId: String(firstEditorId),
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value)
        view.setPendingEditorUpdateJson(equalRender)
        view.setPendingEditorUpdateEditorId(String(firstEditorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        XCTAssertEqual(PositionBridge.cursorScalarOffset(in: view.richTextView.textView), 2)

        _ = EditorV2Shadow.replaceHtml(id: firstEditorId, html: "<p>newer</p>")
        let newer = try XCTUnwrap(editorV2RenderUpdate(
            editorId: String(firstEditorId),
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value)
        view.setPendingEditorUpdateJson(newer)
        view.setPendingEditorUpdateEditorId(String(firstEditorId))
        view.setPendingEditorUpdateRevision(2)
        view.applyPendingEditorUpdateIfNeeded()
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "newer")

        view.setEditorId(secondEditorId)
        let rebound = try XCTUnwrap(editorV2RenderUpdate(
            editorId: String(secondEditorId),
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value)
        view.setPendingEditorUpdateJson(rebound)
        view.setPendingEditorUpdateEditorId(String(secondEditorId))
        view.setPendingEditorUpdateRevision(3)
        view.applyPendingEditorUpdateIfNeeded()
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "\u{200B}")
    }

    func testEqualDocumentRevisionCannotRollBackToOlderStateRevision() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>abcdef</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)

        _ = EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 1, scalarHead: 1)
        let olderState = try XCTUnwrap(editorV2RenderUpdate(
            editorId: String(editorId),
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value)
        _ = EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 4, scalarHead: 4)
        let newerState = try XCTUnwrap(editorV2RenderUpdate(
            editorId: String(editorId),
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value)
        let older = parseJSONObject(olderState)
        let newer = parseJSONObject(newerState)
        XCTAssertEqual(older["documentVersion"] as? String, newer["documentVersion"] as? String)
        XCTAssertLessThan(
            try XCTUnwrap(UInt64(older["stateRevision"] as? String ?? "")),
            try XCTUnwrap(UInt64(newer["stateRevision"] as? String ?? ""))
        )

        XCTAssertTrue(view.applyEditorUpdate(newerState))
        XCTAssertEqual(PositionBridge.cursorScalarOffset(in: view.richTextView.textView), 4)
        XCTAssertTrue(view.applyEditorUpdate(olderState))

        XCTAssertEqual(PositionBridge.cursorScalarOffset(in: view.richTextView.textView), 4)
    }

    func testPendingEditorUpdateRejectsStaleCrossEditorSourceWithoutRetrying() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>First</p>")
        _ = EditorV2Shadow.setHtml(id: secondEditorId, html: "<p>Second</p>")
        guard let secondAdapter = EditorV2Registry.adapter(forLegacyId: secondEditorId) else {
            XCTFail("expected second adapter")
            return
        }
        let errorSink = AutonomousErrorEventSink()

        let view = NativeEditorExpoView()
        view.onEditorErrorForTesting = errorSink.record
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(secondEditorId)

        _ = EditorV2Shadow.replaceHtml(id: firstEditorId, html: "<p>Remote first</p>")
        guard let firstAdapter = EditorV2Registry.adapter(forLegacyId: firstEditorId),
              let staleUpdate = editorV2RenderUpdate(
                editorId: firstAdapter.editorId,
                mirrorScalarAnchor: nil,
                mirrorScalarHead: nil
              ).value
        else {
            XCTFail("expected atomic render snapshot")
            return
        }
        view.setPendingEditorUpdateJson(staleUpdate)
        view.setPendingEditorUpdateEditorId(String(firstEditorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
        XCTAssertEqual(errorSink.errors.count, 1)
        XCTAssertEqual(errorSink.errors.first?.domain, "boundary")
        XCTAssertEqual(errorSink.errors.first?.code, "FFI_RESULT_INVALID")
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        XCTAssertEqual(errorSink.errors.count, 1, "a permanent source mismatch must not schedule another attempt")
        assertNoPendingEditorUpdate(in: view)
        XCTAssertEqual(internalEditorUpdateRejections(in: view), [])
    }

    func testPendingEditorUpdateAcceptsCanonicalSourceAndRejectsMalformedSourceOnce() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }
        let errorSink = AutonomousErrorEventSink()
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Initial</p>")

        let view = NativeEditorExpoView()
        view.onEditorErrorForTesting = errorSink.record
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Canonical</p>")
        guard let accepted = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value else {
            XCTFail("expected atomic render snapshot")
            return
        }
        view.setPendingEditorUpdateJson(accepted)
        view.setPendingEditorUpdateEditorId(String(editorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Canonical")

        view.setPendingEditorUpdateJson(accepted)
        view.setPendingEditorUpdateEditorId("00\(editorId)")
        XCTAssertNil(retainedPendingEditorUpdateSourceId(in: view))
        view.setPendingEditorUpdateRevision(2)
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Canonical")
        XCTAssertEqual(errorSink.errors.count, 1)
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        XCTAssertEqual(errorSink.errors.count, 1)
        assertNoPendingEditorUpdate(in: view)
        XCTAssertEqual(internalEditorUpdateRejections(in: view), [])
    }

    func testPendingEditorUpdateRetainsSourceAcrossSequentialExternalRenderRevisions() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }
        let errorSink = AutonomousErrorEventSink()

        let view = NativeEditorExpoView()
        view.onEditorErrorForTesting = errorSink.record
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Revision one</p>")
        guard let firstUpdate = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value else {
            XCTFail("expected first atomic render snapshot")
            return
        }
        view.setPendingEditorUpdateJson(firstUpdate)
        view.setPendingEditorUpdateEditorId(String(editorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Revision one")
        XCTAssertEqual(retainedPendingEditorUpdateSourceId(in: view), String(editorId))

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Revision two</p>")
        guard let secondUpdate = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value else {
            XCTFail("expected second atomic render snapshot")
            return
        }
        view.setPendingEditorUpdateJson(secondUpdate)
        view.setPendingEditorUpdateRevision(2)
        view.applyPendingEditorUpdateIfNeeded()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Revision two")
        XCTAssertEqual(retainedPendingEditorUpdateSourceId(in: view), String(editorId))
        flushMainQueue()
        flushMainQueue()
        XCTAssertTrue(errorSink.errors.isEmpty)
        XCTAssertEqual(internalEditorUpdateRejections(in: view), [])
    }

    func testPendingEditorUpdateClearsMismatchedRetainedSourceBeforeNextRevision() {
        let editorId = makeV2Editor()
        let differentEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: editorId)
            destroyV2Editor(id: differentEditorId)
        }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId),
              let differentAdapter = EditorV2Registry.adapter(forLegacyId: differentEditorId)
        else {
            XCTFail("expected adapters")
            return
        }
        let errorSink = AutonomousErrorEventSink()

        let view = NativeEditorExpoView()
        view.onEditorErrorForTesting = errorSink.record
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Canonical</p>")
        guard let canonicalUpdate = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value else {
            XCTFail("expected canonical atomic render snapshot")
            return
        }
        view.setPendingEditorUpdateJson(canonicalUpdate)
        view.setPendingEditorUpdateEditorId(String(editorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Canonical")

        _ = EditorV2Shadow.replaceHtml(id: differentEditorId, html: "<p>Different source</p>")
        guard let mismatchedUpdate = editorV2RenderUpdate(
            editorId: differentAdapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value else {
            XCTFail("expected mismatched atomic render snapshot")
            return
        }
        view.setPendingEditorUpdateJson(mismatchedUpdate)
        view.setPendingEditorUpdateEditorId(String(differentEditorId))
        view.setPendingEditorUpdateRevision(2)
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        flushMainQueue()
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Canonical")
        XCTAssertNil(retainedPendingEditorUpdateSourceId(in: view))
        XCTAssertEqual(errorSink.errors.count, 1)
        XCTAssertEqual(
            errorSink.errors.last?.message,
            "external editor update source does not match the bound canonical editor id"
        )

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Must not render</p>")
        guard let idOmittedUpdate = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value else {
            XCTFail("expected ID-omitted atomic render snapshot")
            return
        }
        view.setPendingEditorUpdateJson(idOmittedUpdate)
        view.setPendingEditorUpdateRevision(3)
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Canonical")
        XCTAssertEqual(errorSink.errors.count, 2)
        XCTAssertEqual(
            errorSink.errors.last?.message,
            "external editor update source id is missing or malformed"
        )
        XCTAssertEqual(internalEditorUpdateRejections(in: view), [])
    }

    func testMissingPendingEditorUpdateJSONReportsThroughAdapterOnceAndIsConsumed() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }
        let errorSink = AutonomousErrorEventSink()

        let view = NativeEditorExpoView()
        view.onEditorErrorForTesting = errorSink.record
        view.setEditorId(editorId)
        view.setPendingEditorUpdateEditorId(String(editorId))
        XCTAssertEqual(retainedPendingEditorUpdateSourceId(in: view), String(editorId))
        view.setPendingEditorUpdateJson(nil)
        XCTAssertNil(retainedPendingEditorUpdateSourceId(in: view))
        view.setPendingEditorUpdateRevision(1)

        view.applyPendingEditorUpdateIfNeeded()
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(errorSink.errors.count, 1)
        XCTAssertEqual(errorSink.errors.first?.domain, "boundary")
        XCTAssertEqual(errorSink.errors.first?.code, "FFI_RESULT_INVALID")
        XCTAssertEqual(internalEditorUpdateRejections(in: view), [])
        assertNoPendingEditorUpdate(in: view)
    }

    func testPermanentInvalidPendingEditorUpdateReportsOnceWithoutRetry() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }
        let errorSink = AutonomousErrorEventSink()
        let debugNotesBefore = adapter.debugNotes

        let view = NativeEditorExpoView()
        view.onEditorErrorForTesting = errorSink.record
        view.setEditorId(editorId)
        view.setPendingEditorUpdateJson("{malformed")
        view.setPendingEditorUpdateEditorId(String(editorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(errorSink.errors.count, 1)
        XCTAssertEqual(errorSink.errors.first?.code, "FFI_RESULT_INVALID")
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        XCTAssertEqual(errorSink.errors.count, 1)
        XCTAssertEqual(adapter.debugNotes, debugNotesBefore)
        XCTAssertEqual(internalEditorUpdateRejections(in: view), [])
        assertNoPendingEditorUpdate(in: view)
    }

    func testMalformedPendingEditorUpdateDuringCompositionRejectsOnceWithoutRetry() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }
        let errorSink = AutonomousErrorEventSink()
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>First</p>")

        let view = NativeEditorExpoView()
        view.onEditorErrorForTesting = errorSink.record
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 0)
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.setPendingEditorUpdateJson("{malformed")
        view.setPendingEditorUpdateEditorId(String(editorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()

        assertNoPendingEditorUpdate(in: view)

        flushMainQueue()
        flushMainQueue()
        XCTAssertEqual(errorSink.errors.count, 1)
        XCTAssertEqual(errorSink.errors.first?.code, "FFI_RESULT_INVALID")
        assertNoPendingEditorUpdate(in: view)
    }

    func testMissingPendingEditorUpdateAdapterRecordsOneInternalBoundaryRejectionAndConsumes() {
        let editorId = makeV2Editor()
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            destroyV2Editor(id: editorId)
            return
        }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Initial</p>")

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Remote</p>")
        guard let update = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value else {
            XCTFail("expected atomic render snapshot")
            destroyV2Editor(id: editorId)
            return
        }
        let removedAdapter = EditorV2Registry.removePairing(forLegacyId: editorId)
        defer { removedAdapter?.destroy() }

        view.setPendingEditorUpdateJson(update)
        view.setPendingEditorUpdateEditorId(String(editorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(
            internalEditorUpdateRejections(in: view),
            ["boundary/FFI_RESULT_INVALID/missingAdapter"]
        )
        assertNoPendingEditorUpdate(in: view)
    }

    func testDestroyedPendingEditorUpdateAdapterConsumesWithoutAnEvent() {
        let editorId = makeV2Editor()
        defer { EditorV2Registry.removePairing(forLegacyId: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }
        let errorSink = AutonomousErrorEventSink()
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Initial</p>")

        let view = NativeEditorExpoView()
        view.onEditorErrorForTesting = errorSink.record
        view.setEditorId(editorId)
        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Remote</p>")
        guard let update = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value else {
            XCTFail("expected atomic render snapshot")
            return
        }
        XCTAssertNil(adapter.destroy())

        view.setPendingEditorUpdateJson(update)
        view.setPendingEditorUpdateEditorId(String(editorId))
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()
        view.applyPendingEditorUpdateIfNeeded()
        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(errorSink.errors.isEmpty, "destroy clears the bound view owner before session release")
        XCTAssertEqual(internalEditorUpdateRejections(in: view), [])
        assertNoPendingEditorUpdate(in: view)
    }

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

    func testInputTraitChangesDrainPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesInputTraitChange {
            $0.setAutoCorrect(true)
        }
        assertPendingNativeAutocorrectSurvivesInputTraitChange {
            $0.setAutoCapitalize("characters")
        }
        assertPendingNativeAutocorrectSurvivesInputTraitChange {
            $0.setKeyboardType("email-address")
        }
    }

    func testInputTraitChangeFlushesActiveMarkedCompositionBeforeReload() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 6)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))

        view.textView.setKeyboardType("email-address")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello brave world")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlockedAutoCorrectRetryDoesNotOverrideNewerValue() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCorrect(true)
        view.textView.setAutoCorrect(false)
        flushMainQueue()

        XCTAssertEqual(view.textView.autocorrectionType, .no)
        XCTAssertEqual(view.textView.spellCheckingType, .no)
    }

    func testBlockedAutoCapitalizeRetryDoesNotOverrideNewerValue() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCapitalize("characters")
        view.textView.setAutoCapitalize("none")
        flushMainQueue()

        XCTAssertEqual(view.textView.autocapitalizationType, .none)
    }

    func testBlockedKeyboardTypeRetryDoesNotOverrideNewerValue() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setKeyboardType("email-address")
        view.textView.setKeyboardType("url")
        flushMainQueue()

        XCTAssertEqual(view.textView.keyboardType, .URL)
    }

    func testPendingAutoCorrectRetryIsInvalidatedAndDesiredTraitReplayedOnEditorRebind() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = firstEditorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCorrect(true)
        view.editorId = secondEditorId
        flushMainQueue()

        XCTAssertEqual(view.textView.autocorrectionType, .yes)
        XCTAssertEqual(view.textView.spellCheckingType, .default)
    }

    func testPendingAutoCapitalizeRetryIsInvalidatedAndDesiredTraitReplayedOnEditorRebind() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = firstEditorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCapitalize("characters")
        view.editorId = secondEditorId
        flushMainQueue()

        XCTAssertEqual(view.textView.autocapitalizationType, .allCharacters)
    }

    func testPendingKeyboardTypeRetryIsInvalidatedAndDesiredTraitReplayedOnEditorRebind() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = firstEditorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setKeyboardType("email-address")
        view.editorId = secondEditorId
        flushMainQueue()

        XCTAssertEqual(view.textView.keyboardType, .emailAddress)
    }

    func testAccessoryToolbarPlacementDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange { view in
            view.setToolbarPlacement("inline")
        } verify: { view, _, file, line in
            XCTAssertTrue(view.isUsingAccessoryPlaceholderForTesting(), file: file, line: line)
            XCTAssertFalse(view.isUsingAccessoryToolbarForTesting(), file: file, line: line)
        }
    }

    func testAccessoryToolbarVisibilityDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange { view in
            view.setShowToolbar(false)
        } verify: { view, _, file, line in
            XCTAssertTrue(view.isUsingAccessoryPlaceholderForTesting(), file: file, line: line)
            XCTAssertFalse(view.isUsingAccessoryToolbarForTesting(), file: file, line: line)
        }
    }

    func testThemeAccessoryReloadDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange { view in
            view.setThemeJson(#"{"toolbar":{"appearance":"native"}}"#)
        }
    }

    func testBlockedThemeRetryIsClearedWhenDesiredThemeRevertsBeforeRetry() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        let themeA = "{\"backgroundColor\":\"#101820\"}"
        let themeB = "{\"backgroundColor\":\"#ffeedd\"}"
        view.setEditorId(editorId)
        view.setThemeJson(themeA)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.setThemeJson(themeB)
        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))

        view.setThemeJson(themeA)
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))
        XCTAssertNotEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#ffeedd"))
    }

    func testBlockedThemeRetryAppliesDesiredThemeAfterEditorRebind() {
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

        let themeA = "{\"backgroundColor\":\"#101820\"}"
        let themeB = "{\"backgroundColor\":\"#ffeedd\"}"
        view.setEditorId(firstEditorId)
        view.setThemeJson(themeA)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setThemeJson(themeB)
        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))

        view.setEditorId(secondEditorId)
        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#ffeedd"))

        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#ffeedd"))
        XCTAssertNotEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))
    }

    func testBlockedAtomsPropRetriesAfterCompositionEndsWithoutPropRedelivery() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        flushMainQueue()
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.setAtomsJson(
            #"{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"#
        )
        XCTAssertNil(view.richTextView.textView.atomRenderConfiguration)

        view.richTextView.textView.unmarkText()
        let retried = expectation(description: "atoms configuration reapplied")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
            retried.fulfill()
        }
        wait(for: [retried], timeout: 1)

        XCTAssertEqual(
            view.richTextView.textView.atomRenderConfiguration?.registeredNodeTypes,
            ["counterCard"]
        )
    }

    func testBlockedAtomsPropRetryIsDelayedAndCapped() {
        let view = NativeEditorExpoView()
        view.blockAtomConfigurationApplyForTesting = true

        view.setAtomsJson(
            #"{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"#
        )
        let settled = expectation(description: "retry queue settles")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) {
            settled.fulfill()
        }
        wait(for: [settled], timeout: 1)

        XCTAssertEqual(view.atomsRetryAttemptsForTesting, 5)
        XCTAssertNil(view.richTextView.textView.atomRenderConfiguration)
    }

    func testBlockedAtomsPropWakesAfterRetryCapWhenCompositionEnds() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        view.blockAtomConfigurationApplyForTesting = true
        view.setAtomsJson(
            #"{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"#
        )

        let capped = expectation(description: "atom retries reach their cap")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) {
            capped.fulfill()
        }
        wait(for: [capped], timeout: 1)
        XCTAssertEqual(view.atomsRetryAttemptsForTesting, 5)
        XCTAssertNil(view.richTextView.textView.atomRenderConfiguration)

        view.blockAtomConfigurationApplyForTesting = false
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.richTextView.textView.unmarkText()
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(
            view.richTextView.textView.atomRenderConfiguration?.registeredNodeTypes,
            ["counterCard"]
        )
    }

    func testMentionAddonRefreshDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange(
            initialHTML: "<p>teh @al</p>",
            selectionOffset: 7
        ) { view in
            view.setAddonsJson(self.aliceMentionAddonsJson())
        } verify: { view, _, file, line in
            XCTAssertNotNil(
                view.currentMentionQueryStateForTesting(trigger: "@"),
                file: file,
                line: line
            )
        }
    }

    func testMentionAddonClearDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange(
            initialHTML: "<p>teh @al</p>",
            selectionOffset: 7,
            configure: { view, _ in
                view.setAddonsJson(self.aliceMentionAddonsJson())
            }
        ) { view in
            view.setAddonsJson(nil)
        }
    }

    func testStaleMentionClearRetryDoesNotHideFreshSuggestionsAfterRefreshSucceeds() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.setAddonsJson(aliceMentionAddonsJson())
        XCTAssertTrue(view.isShowingMentionSuggestionsForTesting())

        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(nil)
        view.setAddonsJson(aliceMentionAddonsJson())

        XCTAssertTrue(
            view.isShowingMentionSuggestionsForTesting(),
            "successful mention refresh should show suggestions before the stale clear retry runs"
        )

        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(
            view.isShowingMentionSuggestionsForTesting(),
            "stale clear retry should not hide suggestions from a later successful refresh"
        )
    }

    func testAccessoryRetryBatchKeepsNonConflictingToolbarVisibilityActionAfterMentionClear() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        view.setToolbarPlacement("keyboard")
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.setAddonsJson(aliceMentionAddonsJson())
        XCTAssertTrue(view.isUsingAccessoryToolbarForTesting())

        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(nil)
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setShowToolbar(false)

        XCTAssertTrue(
            view.isUsingAccessoryToolbarForTesting(),
            "toolbar visibility should remain unchanged while the accessory update is queued"
        )

        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(
            view.isUsingAccessoryPlaceholderForTesting(),
            "successful mention clear retry should not cancel a queued toolbar visibility retry"
        )
        XCTAssertFalse(view.isUsingAccessoryToolbarForTesting())
    }

    func testAccessoryRetryBatchKeepsRemainingActionsWhenFirstRetryRequeues() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        view.setToolbarPlacement("keyboard")
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.setAddonsJson(aliceMentionAddonsJson())
        XCTAssertTrue(view.isShowingMentionSuggestionsForTesting())
        XCTAssertTrue(view.isUsingAccessoryToolbarForTesting())

        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(nil)
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(aliceMentionAddonsJson())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setShowToolbar(false)

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(
            view.isShowingMentionSuggestionsForTesting(),
            "a refresh queued behind a requeued clear should still run"
        )
        XCTAssertTrue(
            view.isUsingAccessoryPlaceholderForTesting(),
            "toolbar visibility queued behind a requeued clear should still run"
        )
        XCTAssertFalse(view.isUsingAccessoryToolbarForTesting())
    }

    func testApplyEditorUpdateRetriesAfterBlockedCompositionOnSameEditor() {
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
              let updateJSON = editorV2RenderUpdate(
                editorId: adapter.editorId,
                mirrorScalarAnchor: nil,
                mirrorScalarHead: nil
              ).value
        else {
            XCTFail("expected atomic render snapshot")
            return
        }
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        XCTAssertFalse(view.applyEditorUpdate(updateJSON))
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "First")

        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Remote")
    }

    func testApplyEditorUpdateRetryIsDroppedAfterEditorRebind() {
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

        let staleUpdateJSON = EditorV2Shadow.replaceHtml(id: firstEditorId, html: "<p>Remote</p>")
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        XCTAssertFalse(view.applyEditorUpdate(staleUpdateJSON))
        view.setEditorId(secondEditorId)
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
    }

    func testSameEditorIdUpdateDoesNotDropPendingNativeAutocorrect() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>teh </p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.setEditorId(editorId)
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "the ")
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testPendingNativeAutocorrectIsDroppedAfterEditorRebind() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>teh </p>")
        _ = EditorV2Shadow.setHtml(id: secondEditorId, html: "<p>Second</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(firstEditorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.setEditorId(secondEditorId)
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: firstEditorId), "<p>teh </p>")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: secondEditorId), "<p>Second</p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
    }

    func testPrepareForCommandAfterEditorRebindDoesNotDrainPreviousEditorMutation() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>teh </p>")
        _ = EditorV2Shadow.setHtml(id: secondEditorId, html: "<p>Second</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            NativeEditorViewRegistry.shared.unregister(editorId: secondEditorId, view: view)
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(firstEditorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        view.setEditorId(secondEditorId)

        let preparationJSON = NativeEditorViewRegistry.shared.prepareForCommandJSON(
            editorId: firstEditorId
        )
        XCTAssertTrue(preparationJSON.contains("\"ready\":true"))
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: firstEditorId), "<p>teh </p>")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: secondEditorId), "<p>Second</p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
    }

    func testDestroyedEditorInvalidatesRegistryAndUnbindsView() {
        let editorId = makeV2Editor()
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        XCTAssertEqual(view.richTextView.editorId, editorId)
        XCTAssertEqual(view.richTextView.textView.editorId, editorId)
        view.setPendingEditorUpdateEditorId(String(editorId))
        XCTAssertEqual(retainedPendingEditorUpdateSourceId(in: view), String(editorId))

        NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: editorId)
        destroyV2Editor(id: editorId)
        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )

        XCTAssertEqual(preparation["ready"] as? Bool, false)
        XCTAssertEqual(preparation["blockedReason"] as? String, "destroyed")
        XCTAssertEqual(view.richTextView.editorId, 0)
        XCTAssertEqual(view.richTextView.textView.editorId, 0)
        XCTAssertNil(retainedPendingEditorUpdateSourceId(in: view))
    }

    func testMalformedEditorIdPropRetainsExistingBinding() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)

        applyNativeEditorIdProp("01", to: view)

        XCTAssertEqual(view.richTextView.editorId, editorId)
        XCTAssertEqual(view.richTextView.textView.editorId, editorId)
    }

    func testDestroyedEditorInvalidatesEveryBoundView() {
        let editorId = makeV2Editor()
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)
        let first = NativeEditorExpoView()
        let second = NativeEditorExpoView()
        first.setEditorId(editorId)
        second.setEditorId(editorId)

        NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: editorId)
        destroyV2Editor(id: editorId)

        XCTAssertEqual(first.richTextView.editorId, 0)
        XCTAssertEqual(second.richTextView.editorId, 0)
    }

    func testUnregisterRemovesOnlyCallingViewFromEditorRegistry() {
        let editorId = makeV2Editor()
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)
        let first = NativeEditorExpoView()
        let second = NativeEditorExpoView()
        first.setEditorId(editorId)
        second.setEditorId(editorId)

        NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: first)
        NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: editorId)
        destroyV2Editor(id: editorId)

        XCTAssertEqual(first.richTextView.editorId, editorId)
        XCTAssertEqual(second.richTextView.editorId, 0)
        first.setEditorId(0)
    }

    func testDetachedOwnerReelectsNewestAttachedSurvivorAndCatchesItUp() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Initial</p>")

        let first = NativeEditorExpoView()
        let second = NativeEditorExpoView()
        let third = NativeEditorExpoView()
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        [first, second, third].forEach {
            $0.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
            viewController.view.addSubview($0)
            $0.setEditorId(editorId)
        }
        defer {
            [first, second, third].forEach {
                $0.setEditorId(0)
                $0.removeFromSuperview()
            }
            window.isHidden = true
        }

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Updated</p>")
        third.removeFromSuperview()

        XCTAssertEqual(second.richTextView.textView.textStorage.string, "Updated")
        XCTAssertEqual(first.richTextView.textView.textStorage.string, "Initial")

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Latest</p>")
        second.removeFromSuperview()

        XCTAssertEqual(first.richTextView.textView.textStorage.string, "Latest")
    }

    func testDestroyBoundaryBlocksReentrantRegistrationAndCommandsUntilInvalidation() {
        let editorId = makeV2Editor()
        let registry = NativeEditorViewRegistry.shared
        registry.markEditorCreated(editorId: editorId)
        let first = NativeEditorExpoView()
        let second = NativeEditorExpoView()
        first.setEditorId(editorId)
        var nestedDestroyRan = false

        registry.destroy(editorId: editorId) {
            XCTAssertFalse(registry.register(editorId: editorId, view: second))
            XCTAssertTrue(
                registry.prepareForCommandJSON(editorId: editorId).contains("\"blockedReason\":\"destroying\"")
            )
            registry.destroy(editorId: editorId) { nestedDestroyRan = true }
            destroyV2Editor(id: editorId)
            XCTAssertEqual(first.richTextView.editorId, editorId)
        }

        XCTAssertFalse(nestedDestroyRan)
        XCTAssertEqual(first.richTextView.editorId, 0)
        XCTAssertEqual(second.richTextView.editorId, 0)
    }

    func testDestroyBoundaryInvalidatesViewsWhenDestroyOperationDoesNotRemoveEditor() {
        let editorId = makeV2Editor()
        let registry = NativeEditorViewRegistry.shared
        registry.markEditorCreated(editorId: editorId)
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)

        registry.destroy(editorId: editorId) {
            // Deterministically simulate a native destroy operation that returned
            // without removing the Rust editor.
        }

        XCTAssertEqual(view.richTextView.editorId, 0)
        XCTAssertFalse(EditorV2Shadow.getCurrentState(id: editorId).contains("editor not found"))
        destroyV2Editor(id: editorId)
    }

    func testDestroyedEditorIdCannotRegisterNewView() {
        let editorId = makeV2Editor()
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)
        NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: editorId)
        destroyV2Editor(id: editorId)

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )

        XCTAssertEqual(view.richTextView.editorId, 0)
        XCTAssertEqual(view.richTextView.textView.editorId, 0)
        XCTAssertEqual(preparation["ready"] as? Bool, false)
        XCTAssertEqual(preparation["blockedReason"] as? String, "destroyed")
    }

    func testPrepareForCommandReportsCompositionBlockedReasonWhenMarkedTextPreflightDefers() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: view)
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 0)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )

        XCTAssertEqual(preparation["ready"] as? Bool, false)
        XCTAssertEqual(preparation["blockedReason"] as? String, "composition")
        XCTAssertNil(preparation["updateJSON"])
    }

    func testPrepareForCommandIncludesUpdateJSONAfterNativeAutocorrectDrain() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>teh </p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: view)
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )
        let updateJSON = preparation["updateJSON"] as? String

        XCTAssertEqual(preparation["ready"] as? Bool, true)
        XCTAssertNil(preparation["blockedReason"])
        XCTAssertNotNil(updateJSON)
        XCTAssertTrue(updateJSON?.contains("the ") == true, "preflight update should include the drained correction")
        XCTAssertFalse(updateJSON?.contains("teh ") == true, "preflight update should not contain stale text")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
    }

    func testPrepareForCommandIncludesUpdateJSONAfterSameTextCompositionChangesSelectionState() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 0, scalarHead: 5)
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 5))

        textView.setMarkedText("Hello", selectedRange: NSRange(location: 5, length: 0))
        let preparation = textView.prepareForExternalEditorCommand()

        XCTAssertTrue(preparation.ready)
        XCTAssertNil(preparation.blockedReason)
        XCTAssertNotNil(
            preparation.updateJSON,
            "same-text composition commits should still forward selection/state changes"
        )
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello world")
    }


    private func assertPendingNativeAutocorrectSurvivesInputTraitChange(
        _ applyTraitChange: (EditorTextView) -> Void,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>teh </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder(), file: file, line: line)
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        applyTraitChange(view.textView)

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>", file: file, line: line)
        XCTAssertEqual(view.textView.textStorage.string, "the ", file: file, line: line)
        XCTAssertEqual(view.textView.reconciliationCount, 0, file: file, line: line)
    }

    private func beginEmptyMarkedComposition(
        in view: RichTextEditorView,
        utf16Offset: Int,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        setCollapsedSelection(in: view.textView, utf16Offset: utf16Offset)
        flushMainQueue()
        XCTAssertTrue(view.textView.becomeFirstResponder(), file: file, line: line)
        view.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
    }

    private func assertPendingNativeAutocorrectSurvivesAccessoryChange(
        initialHTML: String = "<p>teh </p>",
        selectionOffset: Int = 4,
        configure: ((NativeEditorExpoView, UInt64) -> Void)? = nil,
        _ applyAccessoryChange: (NativeEditorExpoView) -> Void,
        verify: ((NativeEditorExpoView, UInt64, StaticString, UInt) -> Void)? = nil,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: initialHTML)

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        configure?(view, editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: selectionOffset)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder(), file: file, line: line)
        let expectedText = view.richTextView.textView.textStorage.string.replacingOccurrences(
            of: "teh",
            with: "the"
        )
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        applyAccessoryChange(view)
        flushMainQueue()

        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            initialHTML.replacingOccurrences(of: "teh", with: "the"),
            file: file,
            line: line
        )
        XCTAssertEqual(view.richTextView.textView.textStorage.string, expectedText, file: file, line: line)
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0, file: file, line: line)
        verify?(view, editorId, file, line)
    }

    private func internalEditorUpdateRejections(in view: NativeEditorExpoView) -> [String] {
        Mirror(reflecting: view).children.first {
            $0.label == "editorUpdateInternalRejections"
        }?.value as? [String] ?? []
    }

    private func retainedPendingEditorUpdateSourceId(in view: NativeEditorExpoView) -> String? {
        Mirror(reflecting: view).children.first {
            $0.label == "pendingEditorUpdateEditorId"
        }?.value as? String
    }

    private func assertNoPendingEditorUpdate(
        in view: NativeEditorExpoView,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let state = Dictionary(uniqueKeysWithValues: Mirror(reflecting: view).children.compactMap {
            child -> (String, Any)? in
            guard let label = child.label else { return nil }
            return (label, child.value)
        })
        XCTAssertNil(state["pendingEditorUpdateJSON"] as? String, file: file, line: line)
        XCTAssertEqual(state["pendingEditorUpdateRevision"] as? Int, 0, file: file, line: line)
        XCTAssertEqual(state["pendingEditorUpdateRetryScheduled"] as? Bool, false, file: file, line: line)
    }

    private func encodedJSONObject(_ object: [String: Any]) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: object)
        return try XCTUnwrap(String(data: data, encoding: .utf8))
    }

}

private final class AutonomousErrorEventSink {
    private(set) var errors: [FfiError] = []

    func record(_ payload: [String: Any]) {
        guard let error = payload["error"] as? [String: Any],
              let domain = error["domain"] as? String,
              let code = error["code"] as? String,
              let message = error["message"] as? String
        else { return }
        func optionalString(_ key: String) -> String? {
            error[key] as? String
        }
        errors.append(FfiError(
            domain: domain,
            code: code,
            message: message,
            requestId: optionalString("requestId"),
            operationIndex: optionalString("operationIndex"),
            limit: optionalString("limit"),
            actual: optionalString("actual"),
            detailsJson: optionalString("detailsJson")
        ))
    }
}
