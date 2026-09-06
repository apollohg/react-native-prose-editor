import XCTest
import ExpoModulesCore

extension EditorV2StagingViewTests {
    func testStagingTypingAppliesRenderPatchWithoutFullRerender() {
        let (view, adapter, window) = makeBoundView(html: "<p>Hello world, this is a long paragraph.</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        setCollapsedCaret(in: view.textView, utf16Offset: 5)
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.insertText("X")

        XCTAssertEqual(v2DocumentText(adapter), "HelloX world, this is a long paragraph.")
        XCTAssertEqual(
            view.textView.lastRenderAppliedPatchForTesting, true,
            "a single-character commit must render through the patch path, not a full re-render"
        )
        XCTAssertEqual(view.textView.textStorage.string, "HelloX world, this is a long paragraph.")
    }

    func testStagingSelectionSyncDeliversRustStatePositions() {
        let (view, adapter, window) = makeBoundView(html: "<p>abcdef</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        let delegate = EditorTextViewDelegateSpy()
        view.textView.editorDelegate = delegate
        setCollapsedCaret(in: view.textView, utf16Offset: 3)
        view.textView.delegate?.textViewDidChangeSelection?(view.textView)
        flushMain()

        // scalar 3 inside "abcdef" maps to doc position 4.
        XCTAssertEqual(delegate.selectionChanges.last?.anchor, 4)
        XCTAssertEqual(delegate.selectionChanges.last?.head, 4)
        _ = adapter
    }

    func testStagingReadOnlyRejectsAccessibilityStyleEditAtomically() {
        let (view, adapter, window) = makeBoundView(
            configJson: #"{"initialization":{"type":"localEmpty"},"policy":{"readOnly":true}}"#,
            html: "<p>ab</p>"
        )
        defer { view.removeFromSuperview(); window.isHidden = true }
        setCollapsedCaret(in: view.textView, utf16Offset: 2)
        flushMain()
        var errors: [FfiError] = []
        adapter.onAutonomousError = { errors.append($0) }

        // VoiceOver/dictation edits enter through the same UITextInput entry
        // points; the engine must reject them atomically even if UIKit lets
        // the call through.
        view.textView.insertText("z")
        view.textView.deleteBackward()

        XCTAssertEqual(v2DocumentText(adapter), "ab")
        XCTAssertEqual(view.textView.textStorage.string, "ab")
        XCTAssertEqual(errors.last?.code, "MUTATION_REJECTED")
    }

    func testStagingDestroyMidCompositionIsStructuredFailureWithoutPartialCommit() {
        let (view, adapter, window) = makeBoundView(html: "<p>ab</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        setCollapsedCaret(in: view.textView, utf16Offset: 2)
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())
        var errors: [FfiError] = []
        adapter.onAutonomousError = { errors.append($0) }

        view.textView.setMarkedText("xyz", selectedRange: NSRange(location: 1, length: 0))
        let revisionBeforeDestroy = adapter.baseDocumentRevision

        // The editor is destroyed mid-composition.
        adapter.destroy()

        // Finishing the composition must not crash, must not partially
        // commit, and must surface the structured lifecycle failure.
        view.textView.unmarkText()
        flushMain()

        XCTAssertEqual(errors.last?.domain, "lifecycle")
        XCTAssertEqual(errors.last?.code, "ENGINE_DESTROYED")
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBeforeDestroy)
    }

    func testStagingUndoRedoThroughToolbarPath() {
        let (view, adapter, window) = makeBoundView(html: "<p>ab</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        setCollapsedCaret(in: view.textView, utf16Offset: 2)
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.insertText("c")
        XCTAssertEqual(v2DocumentText(adapter), "abc")

        view.textView.performToolbarUndo()
        XCTAssertEqual(v2DocumentText(adapter), "ab")
        view.textView.performToolbarRedo()
        XCTAssertEqual(v2DocumentText(adapter), "abc")
    }

}
