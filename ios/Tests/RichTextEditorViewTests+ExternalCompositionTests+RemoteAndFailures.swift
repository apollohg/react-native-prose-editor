import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testExternalTextCompositionMaximumLengthFailureCancelsAndRestores() throws {
        try assertExternalCompositionCommitFailureRestores(
            configJSON: #"{"initialization":{"type":"localEmpty"},"policy":{"maxLength":3}}"#,
            initialText: "ab",
            finalText: "long"
        )
    }

    func testExternalTextCompositionInputFilterFailureCancelsAndRestores() throws {
        let creation = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"},"policy":{"inputFilter":"[unclosed"}}"#,
            snapshotState: nil
        )
        let handle = try XCTUnwrap(creation.value.flatMap(createdV2TestEditorHandle))
        var collaborationWakes: [CollaborationWakeReason] = []
        let adapter = try XCTUnwrap(
            EditorV2Adapter.attach(
                editorId: handle.handle,
                roomBound: true,
                collaborationWake: { _, reason in
                    collaborationWakes.append(reason)
                }
            )
        )
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        defer { EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: handle.nativeViewId, initialHTML: "<p>12</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 2))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        let stateBefore = try XCTUnwrap(editorV2GetState(editorId: handle.handle).value)
        let revisionBefore = adapter.baseDocumentRevision
        let historyBefore = try XCTUnwrap(adapter.historyFlags())
        collaborationWakes.removeAll()

        _ = textView.beginExternalTextComposition(sessionId: "speech-filter")
        _ = textView.updateExternalTextComposition(
            sessionId: "speech-filter",
            text: "letters"
        )
        XCTAssertTrue(collaborationWakes.isEmpty)
        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-filter",
            finalText: "letters"
        )
        let result = parseJSONObject(resultJSON)
        let duplicate = textView.commitExternalTextComposition(
            sessionId: "speech-filter",
            finalText: "ignored"
        )

        XCTAssertEqual(result["outcome"] as? String, "cancelled")
        assertExternalCompositionError(result, code: "EXTERNAL_COMPOSITION_COMMIT_FAILED")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: handle.nativeViewId), "<p>12</p>")
        XCTAssertEqual(textView.textStorage.string, "12")
        XCTAssertEqual(editorV2GetState(editorId: handle.handle).value, stateBefore)
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore)
        XCTAssertEqual(adapter.historyFlags()?.canUndo, historyBefore.canUndo)
        XCTAssertEqual(adapter.historyFlags()?.canRedo, historyBefore.canRedo)
        XCTAssertTrue(collaborationWakes.isEmpty)
        XCTAssertEqual(duplicate, resultJSON)
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
        XCTAssertTrue(spy.receivedUpdates.isEmpty)
    }

    func testExternalTextCompositionMergesRemoteFirstMutationThroughDeferredRegistryRefresh() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>abc</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        let textView = view.richTextView.textView
        setSelection(in: textView, utf16Range: NSRange(location: 1, length: 1))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "X")

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991102","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"Z"}}"#
        )
        XCTAssertNil(external.error)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")

        NativeEditorViewRegistry.shared.applyRemoteCommitRefresh(editorId: editorId)

        XCTAssertEqual(textView.textStorage.string, "aXc")
        XCTAssertTrue(spy.receivedUpdates.isEmpty)

        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-1",
            finalText: "Y"
        )
        let result = parseJSONObject(resultJSON)

        XCTAssertEqual(result["outcome"] as? String, "committed")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        XCTAssertNil(result["error"])
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>ZaYc</p>")
        XCTAssertEqual(textView.textStorage.string, "ZaYc")
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
        XCTAssertEqual(spy.receivedUpdates.count, 1)
    }

    func testExternalTextCompositionRemoteFirstNoOpAdoptsRenderWithoutLocalUpdate() throws {
        let creation = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"},"policy":{"inputFilter":"[0-9]"}}"#,
            snapshotState: nil
        )
        let handle = try XCTUnwrap(creation.value.flatMap(createdV2TestEditorHandle))
        let adapter = try XCTUnwrap(
            EditorV2Adapter.attach(editorId: handle.handle, roomBound: false)
        )
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        defer { EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId) }
        _ = EditorV2Shadow.setHtml(id: handle.nativeViewId, html: "<p>123</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(handle.nativeViewId)
        let textView = view.richTextView.textView
        setSelection(in: textView, utf16Range: NSRange(location: 1, length: 1))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        _ = textView.beginExternalTextComposition(sessionId: "speech-noop")
        _ = textView.updateExternalTextComposition(sessionId: "speech-noop", text: "X")

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991104","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"4"}}"#
        )
        XCTAssertNil(external.error)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: handle.nativeViewId), "<p>4123</p>")
        let remoteOutcome = parseJSONObject(try XCTUnwrap(external.value))
        let remoteRevision = try XCTUnwrap(remoteOutcome["documentRevision"] as? String)
        NativeEditorViewRegistry.shared.applyRemoteCommitRefresh(editorId: handle.nativeViewId)
        XCTAssertTrue(spy.receivedUpdates.isEmpty)

        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-noop",
            finalText: "letters"
        )
        let result = parseJSONObject(resultJSON)

        XCTAssertEqual(result["outcome"] as? String, "committed")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        XCTAssertNil(result["error"])
        XCTAssertEqual(EditorV2Shadow.getHtml(id: handle.nativeViewId), "<p>4123</p>")
        XCTAssertEqual(
            parseJSONObject(EditorV2Shadow.getCurrentState(id: handle.nativeViewId))["documentVersion"] as? String,
            remoteRevision
        )
        XCTAssertEqual(textView.textStorage.string, "4123")
        XCTAssertEqual(
            spy.externalCompositionEnds,
            [resultJSON],
            "the successful no-op commit must emit one terminal event"
        )
        XCTAssertTrue(
            spy.receivedUpdates.isEmpty,
            "the remote render must not be reported as a local content update; got \(spy.receivedUpdates.count)"
        )
    }

    func testExternalTextCompositionReleasedPositionEpochCancelsAndRestoresAuthoritativeRender() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>abc</p>")
        let view = NativeEditorExpoView()
        var hostErrors: [[String: Any]] = []
        view.onEditorErrorForTesting = { hostErrors.append($0) }
        view.setEditorId(editorId)
        let textView = view.richTextView.textView
        setSelection(in: textView, utf16Range: NSRange(location: 1, length: 1))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        _ = textView.beginExternalTextComposition(sessionId: "speech-invalid-epoch")
        _ = textView.updateExternalTextComposition(
            sessionId: "speech-invalid-epoch",
            text: "X"
        )

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991105","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"Z"}}"#
        )
        XCTAssertNil(external.error)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")
        let authoritativeState = try XCTUnwrap(editorV2GetState(editorId: adapter.editorId).value)
        let requestBeforeCommit = adapter.lastRequestIdForTesting ?? 0
        XCTAssertTrue(adapter.releaseCurrentNativeOwnerInRustForTesting())

        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-invalid-epoch",
            finalText: "Y"
        )
        let result = parseJSONObject(resultJSON)

        XCTAssertEqual(result["outcome"] as? String, "cancelled")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        assertExternalCompositionError(result, code: "EXTERNAL_COMPOSITION_COMMIT_FAILED")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")
        XCTAssertEqual(textView.textStorage.string, "Zabc")
        XCTAssertEqual(adapter.lastRequestIdForTesting, requestBeforeCommit + 1)
        XCTAssertEqual(editorV2GetState(editorId: adapter.editorId).value, authoritativeState)
        XCTAssertTrue(spy.receivedUpdates.isEmpty)
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
        flushMainQueue()
        XCTAssertEqual(hostErrors.count, 1)
        XCTAssertEqual(hostErrors.first?["editorId"] as? String, adapter.editorId)
        let hostError = try XCTUnwrap(hostErrors.first?["error"] as? [String: Any])
        XCTAssertEqual(hostError["code"] as? String, "POSITION_EPOCH_INVALID")
    }

    func testExternalTextCompositionRemoteFirstCollapsedEmptyRemapsCaretWithoutLocalUpdate() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>abc</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        let textView = view.richTextView.textView
        setSelection(in: textView, utf16Range: NSRange(location: 2, length: 0))
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        _ = textView.beginExternalTextComposition(sessionId: "speech-remote-empty")
        _ = textView.updateExternalTextComposition(
            sessionId: "speech-remote-empty",
            text: "X"
        )

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991103","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"Z"}}"#
        )
        XCTAssertNil(external.error)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")
        let remoteOutcome = parseJSONObject(try XCTUnwrap(external.value))
        let revisionBeforeCommit = try XCTUnwrap(remoteOutcome["documentRevision"] as? String)
        let requestBeforeCommit = adapter.lastRequestIdForTesting ?? 0

        NativeEditorViewRegistry.shared.applyRemoteCommitRefresh(editorId: editorId)

        XCTAssertEqual(textView.textStorage.string, "abXc")
        spy.receivedUpdates.removeAll()
        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-remote-empty",
            finalText: ""
        )
        let result = parseJSONObject(resultJSON)

        XCTAssertEqual(result["outcome"] as? String, "committed")
        XCTAssertEqual(result["cause"] as? String, "consumer")
        XCTAssertNil(result["error"])
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Zabc</p>")
        XCTAssertEqual(textView.textStorage.string, "Zabc")
        XCTAssertEqual(textView.selectedRange, NSRange(location: 3, length: 0))
        XCTAssertEqual(adapter.lastRequestIdForTesting, requestBeforeCommit + 1)
        XCTAssertEqual(
            parseJSONObject(EditorV2Shadow.getCurrentState(id: editorId))["documentVersion"] as? String,
            revisionBeforeCommit
        )
        XCTAssertTrue(spy.receivedUpdates.isEmpty)
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON])
    }

}
