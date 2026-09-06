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

}
