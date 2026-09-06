import XCTest
import ExpoModulesCore

extension EditorV2StagingViewTests {
    func testStagingMarkedTextTransientNeverReachesRustAndCommitsOnce() {
        let (view, adapter, window) = makeBoundView(html: "<p>ab</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        setCollapsedCaret(in: view.textView, utf16Offset: 2)
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())

        let revisionBefore = adapter.baseDocumentRevision
        view.textView.setMarkedText("n", selectedRange: NSRange(location: 1, length: 0))

        // Transient IME state stays native-only: no v2 traffic, no revision
        // movement, document untouched.
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore)
        XCTAssertEqual(v2DocumentText(adapter), "ab")

        view.textView.unmarkText()

        // The final composition commit is exactly one typed local-input
        // transaction: one revision step, one undo removes it.
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore + 1)
        XCTAssertEqual(v2DocumentText(adapter), "abn")
        _ = adapter.undo()
        XCTAssertEqual(v2DocumentText(adapter), "ab")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testStagingAutocorrectAcceptCommitsOneTransaction() {
        let (view, adapter, window) = makeBoundView(html: "<p>teh </p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        setCollapsedCaret(in: view.textView, utf16Offset: 4)
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())

        let revisionBefore = adapter.baseDocumentRevision
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        flushMain()

        XCTAssertEqual(v2DocumentText(adapter), "the ")
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore + 1)
        XCTAssertEqual(view.textView.textStorage.string, "the ")
    }

    func testStagingNativeCorrectionUpdateCarriesAuthoritativePostSelection() throws {
        let (view, _, window) = makeBoundView(html: "<p></p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        let delegate = EditorTextViewDelegateSpy()
        view.textView.editorDelegate = delegate
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())
        for character in "teh " {
            view.textView.insertText(String(character))
        }
        delegate.receivedUpdates.removeAll()
        delegate.selectionChanges.removeAll()

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        flushMain()

        let updateJSON = try XCTUnwrap(delegate.receivedUpdates.last)
        let updateData = try XCTUnwrap(updateJSON.data(using: .utf8))
        let update = try XCTUnwrap(
            JSONSerialization.jsonObject(with: updateData) as? [String: Any]
        )
        let selection = try XCTUnwrap(update["selection"] as? [String: Any])
        XCTAssertEqual((selection["anchorScalar"] as? NSNumber)?.uint32Value, 4)
        XCTAssertEqual((selection["headScalar"] as? NSNumber)?.uint32Value, 4)
        XCTAssertEqual(view.textView.selectedRange, NSRange(location: 4, length: 0))
        let expectedDocPosition = EditorV2Shadow.scalarToDoc(id: view.editorId, scalar: 4)
        XCTAssertEqual(delegate.selectionChanges.last?.anchor, expectedDocPosition)
        XCTAssertEqual(delegate.selectionChanges.last?.head, expectedDocPosition)
    }

    func testNativeCorrectionProjectsRustCaretAfterLengthChange() {
        let (view, adapter, window) = makeBoundView(html: "<p></p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())
        for character in "thueh" {
            view.textView.insertText(String(character))
        }

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 4),
            with: "thrus"
        )
        setCollapsedCaret(in: view.textView, utf16Offset: 6)
        flushMain()

        XCTAssertEqual(v2DocumentText(adapter), "thrush")
        XCTAssertEqual(view.textView.textStorage.string, "thrush")
        XCTAssertEqual(view.textView.selectedRange, NSRange(location: 6, length: 0))
        XCTAssertEqual(view.textView.currentLogicalScalarSelection()?.head, 6)
    }

    func testNativeCorrectionDoesNotAdoptTransientBeginningSelection() {
        let (view, adapter, window) = makeBoundView(html: "<p></p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())
        for character in "thueh" {
            view.textView.insertText(String(character))
        }

        view.textView.textStorage.beginEditing()
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 4),
            with: "thrus"
        )
        view.textView.selectedRange = NSRange(location: 0, length: 0)
        view.textView.textStorage.endEditing()
        flushMain()

        XCTAssertEqual(v2DocumentText(adapter), "thrush")
        XCTAssertEqual(view.textView.textStorage.string, "thrush")
        XCTAssertEqual(view.textView.selectedRange, NSRange(location: 6, length: 0))
        XCTAssertEqual(view.textView.currentLogicalScalarSelection()?.head, 6)
    }

    func testNativeCorrectionAdoptsSubsequentBeginningSelection() {
        let (view, adapter, window) = makeBoundView(html: "<p></p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())
        for character in "thueh" {
            view.textView.insertText(String(character))
        }

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 4),
            with: "thrus"
        )
        setCollapsedCaret(in: view.textView, utf16Offset: 0)
        flushMain()

        XCTAssertEqual(v2DocumentText(adapter), "thrush")
        XCTAssertEqual(view.textView.textStorage.string, "thrush")
        XCTAssertEqual(view.textView.selectedRange, NSRange(location: 0, length: 0))
        XCTAssertEqual(view.textView.currentLogicalScalarSelection()?.head, 0)
    }

    func testStagingAutocorrectPreservesAcceptedSpaceWhenNativeReplacementConsumesIt() {
        let (view, adapter, window) = makeBoundView(html: "<p></p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())

        for character in "teh " {
            view.textView.insertText(String(character))
        }
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 4),
            with: "the"
        )
        setCollapsedCaret(in: view.textView, utf16Offset: 3)

        view.textView.insertText("n")

        XCTAssertEqual(v2DocumentText(adapter), "the n")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        XCTAssertEqual(view.textView.selectedRange, NSRange(location: 5, length: 0))
    }

    func testStagingAutocorrectQueuesSpaceDeliveredDuringRustRender() {
        let (view, adapter, window) = makeBoundView(html: "<p></p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())

        for character in "thueh" {
            view.textView.insertText(String(character))
        }

        view.textView.onApplyingRustTextForTesting = { [weak textView = view.textView] in
            textView?.onApplyingRustTextForTesting = nil
            textView?.insertText(" ")
        }
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 4),
            with: "thrus"
        )
        setCollapsedCaret(in: view.textView, utf16Offset: 6)
        flushMain {
            v2DocumentText(adapter) == "thrush "
                && view.textView.textStorage.string == "thrush "
                && view.textView.selectedRange == NSRange(location: 7, length: 0)
        }

        XCTAssertEqual(v2DocumentText(adapter), "thrush ")
        XCTAssertEqual(view.textView.textStorage.string, "thrush ")
        XCTAssertEqual(view.textView.selectedRange, NSRange(location: 7, length: 0))
    }

    func testStagingAutocorrectDoesNotLetFollowingInputOvertakeDeferredSpace() {
        let (view, adapter, window) = makeBoundView(html: "<p></p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())

        for character in "thueh" {
            view.textView.insertText(String(character))
        }

        view.textView.onApplyingRustTextForTesting = { [weak textView = view.textView] in
            textView?.onApplyingRustTextForTesting = nil
            textView?.insertText(" ")
        }
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 4),
            with: "thrus"
        )
        setCollapsedCaret(in: view.textView, utf16Offset: 6)

        view.textView.insertText("x")
        view.textView.insertText("y")
        flushMain {
            v2DocumentText(adapter) == "thrush xy"
                && view.textView.textStorage.string == "thrush xy"
                && view.textView.selectedRange == NSRange(location: 9, length: 0)
        }

        XCTAssertEqual(v2DocumentText(adapter), "thrush xy")
        XCTAssertEqual(view.textView.textStorage.string, "thrush xy")
        XCTAssertEqual(view.textView.selectedRange, NSRange(location: 9, length: 0))
    }

    func testStagingAutocorrectReplacePreservesAcceptedSpaceImmediately() throws {
        let (view, adapter, window) = makeBoundView(html: "<p></p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())

        for character in "teh " {
            view.textView.insertText(String(character))
        }
        let start = try XCTUnwrap(
            view.textView.position(from: view.textView.beginningOfDocument, offset: 0)
        )
        let end = try XCTUnwrap(view.textView.position(from: start, offset: 4))
        let correctionRange = try XCTUnwrap(view.textView.textRange(from: start, to: end))

        view.textView.replace(correctionRange, withText: "the")

        XCTAssertEqual(v2DocumentText(adapter), "the ")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.selectedRange, NSRange(location: 4, length: 0))
    }

    func testStagingSelectedReplacementCanRemoveTrailingSpace() throws {
        let (view, adapter, window) = makeBoundView(html: "<p></p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        flushMain()
        XCTAssertTrue(view.textView.becomeFirstResponder())

        for character in "teh " {
            view.textView.insertText(String(character))
        }

        let start = try XCTUnwrap(
            view.textView.position(from: view.textView.beginningOfDocument, offset: 0)
        )
        let end = try XCTUnwrap(view.textView.position(from: start, offset: 4))
        let replacementRange = try XCTUnwrap(view.textView.textRange(from: start, to: end))
        view.textView.selectedTextRange = replacementRange

        view.textView.replace(replacementRange, withText: "the")

        XCTAssertEqual(v2DocumentText(adapter), "the")
        XCTAssertEqual(view.textView.textStorage.string, "the")
        XCTAssertEqual(view.textView.selectedRange, NSRange(location: 3, length: 0))
    }

}
