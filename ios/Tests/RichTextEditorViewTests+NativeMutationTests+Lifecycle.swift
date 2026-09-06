import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testBlurTimeAutocorrectAfterResignStillCommitsToRust() {
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

        XCTAssertTrue(view.textView.becomeFirstResponder())
        XCTAssertTrue(view.textView.resignFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlurTimeAutocorrectAfterNextMainQueueTurnStillCommitsToRust() {
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

        XCTAssertTrue(view.textView.becomeFirstResponder())
        XCTAssertTrue(view.textView.resignFirstResponder())
        flushMainQueue()

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlurTimeAutocorrectAfterGracePeriodReconcilesInsteadOfCommitting() {
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

        XCTAssertTrue(view.textView.becomeFirstResponder())
        XCTAssertTrue(view.textView.resignFirstResponder())
        view.textView.expireNativeTextMutationAfterBlurDeadlineForTesting()

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>teh </p>")
        XCTAssertEqual(view.textView.textStorage.string, "teh ")
        XCTAssertEqual(view.textView.reconciliationCount, 1)
    }

    func testRejectedBlurredMutationCannotBecomeAuthorizedAfterRefocus() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 5)
        flushMainQueue()
        XCTAssertTrue(view.textView.becomeFirstResponder())
        XCTAssertTrue(view.textView.resignFirstResponder())
        view.textView.expireNativeTextMutationAfterBlurDeadlineForTesting()

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 5, length: 0),
            with: "x"
        )
        XCTAssertEqual(view.textView.textStorage.string, "Hellox")
        XCTAssertEqual(view.textView.reconciliationCount, 1)

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.insertText("!")
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello!</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello!")
    }

    func testBlurTimeAutocorrectAfterContentReplacementReconcilesInsteadOfCommitting() {
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

        XCTAssertTrue(view.textView.becomeFirstResponder())
        XCTAssertTrue(view.textView.resignFirstResponder())

        view.setContent(html: "<p>Remote</p>")
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: view.textView.textStorage.length),
            with: "the "
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Remote</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Remote")
        XCTAssertEqual(view.textView.reconciliationCount, 1)
    }

    func testBlurTimeAutocorrectGraceWindowIsConsumedAfterCommit() {
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

        XCTAssertTrue(view.textView.becomeFirstResponder())
        XCTAssertTrue(view.textView.resignFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "xxx"
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 1)
    }

    func testThemeRefreshDrainsPendingNativeAutocorrectBeforeApplyingRustState() {
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

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.textView.applyTheme(EditorTheme(dictionary: [
            "textColor": "#123456",
        ]))

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testSetEditableFalseDrainsPendingNativeAutocorrectBeforeReadOnly() {
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

        view.setEditable(false)

        XCTAssertFalse(view.richTextView.textView.isEditable)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "the ")
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testExternalAtomicRenderAdoptionLetsTheFirstKeystrokeCommitAtTheNextRevision() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected the v2 adapter paired to the native editor")
            return
        }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>base</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)

        let revisionN = adapter.baseDocumentRevision
        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"991001","baseDocumentRevision":"\#(revisionN)","command":{"type":"insertText","text":"EXT"}}"#
        )
        XCTAssertNil(external.error, "external mutation failed: \(String(describing: external.error))")
        let snapshot = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        )
        guard let externalRender = snapshot.value, snapshot.error == nil else {
            XCTFail("external render failed: \(String(describing: snapshot.error))")
            return
        }

        XCTAssertTrue(view.applyEditorUpdate(externalRender))
        XCTAssertEqual(adapter.baseDocumentRevision, revisionN + 1)

        view.richTextView.textView.insertText("!")

        XCTAssertEqual(adapter.baseDocumentRevision, revisionN + 2)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>EXT!base</p>")
        XCTAssertFalse(
            adapter.debugNotes.contains(where: { $0.contains("mismatch-refresh") }),
            "the first keystroke after an adopted external render must not race its own cache"
        )
    }

    func testExternalRenderCapturedBeforeMarkedTextPreflightRefreshesBeforeNextKeystroke() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected the v2 adapter paired to the native editor")
            return
        }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>base</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())

        let revisionN = adapter.baseDocumentRevision
        let snapshot = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        )
        guard let externalRenderAtN = snapshot.value, snapshot.error == nil else {
            XCTFail("external render failed: \(String(describing: snapshot.error))")
            return
        }

        view.richTextView.textView.setMarkedText("IME", selectedRange: NSRange(location: 3, length: 0))

        let renderCallsBeforePreflight = adapter.renderUpdateCallCountForTesting
        XCTAssertTrue(view.applyEditorUpdate(externalRenderAtN))
        XCTAssertEqual(adapter.baseDocumentRevision, revisionN + 1)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>baseIME</p>")
        XCTAssertEqual(
            adapter.renderUpdateCallCountForTesting,
            renderCallsBeforePreflight + 1,
            "the composition preflight commit must supply its already-adopted atomic render without a second refresh"
        )

        view.richTextView.textView.insertText("!")

        XCTAssertEqual(adapter.baseDocumentRevision, revisionN + 2)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>baseIME!</p>")
        XCTAssertFalse(
            adapter.debugNotes.contains(where: { $0.contains("mismatch-refresh") }),
            "the stale external render must not overwrite the post-preflight adapter revision"
        )
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

}
