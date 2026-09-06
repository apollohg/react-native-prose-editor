import XCTest
import ExpoModulesCore

extension EditorV2StagingViewTests {
    func testStagingBindRendersFromV2Session() {
        let (view, adapter, window) = makeBoundView()
        defer { view.removeFromSuperview(); window.isHidden = true }

        XCTAssertEqual(view.textView.textStorage.string, "Hello")
        XCTAssertEqual(v2DocumentText(adapter), "Hello")
        XCTAssertGreaterThan(adapter.baseDocumentRevision, 0)
    }

    func testStagingEditorOwnsTextDragAndDropDelegates() {
        let (view, _, window) = makeBoundView()
        defer { view.removeFromSuperview(); window.isHidden = true }

        XCTAssertTrue(view.textView.textDragDelegate === view.textView)
        XCTAssertTrue(view.textView.textDropDelegate === view.textView)
    }

    func testStagingTerminalAtomDoesNotExposeAdjacentCaretsOrExtraHeight() {
        let (view, _, window) = makeTerminalAtomView()
        defer { view.removeFromSuperview(); window.isHidden = true }
        let atomRect = terminalAtomRect(in: view.textView)
        for offset in [0, view.textView.textStorage.length] {
            setCollapsedCaret(in: view.textView, utf16Offset: offset)
            guard let position = view.textView.selectedTextRange?.start else {
                XCTFail("expected selected position")
                return
            }
            XCTAssertTrue(view.textView.caretRect(for: position).isEmpty)
        }
        let measuredHeight = view.textView.measuredAutoGrowHeightForTesting(
            width: view.textView.bounds.width
        )

        XCTAssertLessThanOrEqual(
            measuredHeight,
            ceil(atomRect.maxY + view.textView.textContainerInset.bottom) + 0.5
        )
    }

    func testStagingAtomBoundarySelectionRestoresLastParagraphCaret() {
        let (view, _, window) = makeTerminalAtomView(
            html: #"<p>Before</p><div data-type="counter-card" data-count="7"></div>"#
        )
        defer { view.removeFromSuperview(); window.isHidden = true }
        let validRange = NSRange(location: 3, length: 0)
        setCollapsedCaret(in: view.textView, utf16Offset: validRange.location)
        view.textView.textViewDidChangeSelection(view.textView)
        let atomOffset = view.textView.textStorage.length - 1

        for offset in [atomOffset, atomOffset + 1] {
            setCollapsedCaret(in: view.textView, utf16Offset: offset)
            view.textView.textViewDidChangeSelection(view.textView)
            XCTAssertEqual(view.textView.selectedRange, validRange)
        }
    }

    func testStagingTypingAtTerminalAtomBoundaryDoesNotChangeDocument() {
        let (view, _, window) = makeTerminalAtomView()
        defer { view.removeFromSuperview(); window.isHidden = true }
        let htmlBefore = EditorV2Shadow.getHtml(id: view.editorId)
        setCollapsedCaret(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMain()

        view.textView.insertText("x")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: view.editorId), htmlBefore)
    }

    func testStagingReturnAtTerminalAtomBoundaryDoesNotChangeDocument() {
        let (view, _, window) = makeTerminalAtomView()
        defer { view.removeFromSuperview(); window.isHidden = true }
        let htmlBefore = EditorV2Shadow.getHtml(id: view.editorId)
        setCollapsedCaret(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMain()

        view.textView.insertText("\n")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: view.editorId), htmlBefore)
    }

    func testStagingBackspaceInEmptyParagraphAfterTerminalAtomKeepsAtom() {
        let (view, _, window) = makeTerminalAtomView()
        defer { view.removeFromSuperview(); window.isHidden = true }
        let htmlBefore = EditorV2Shadow.getHtml(id: view.editorId)
        setCollapsedCaret(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMain()

        view.textView.insertText("\n")
        view.textView.deleteBackward()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: view.editorId), htmlBefore)
    }

    func testStagingAtomLineHitKeepsExistingParagraphPosition() throws {
        let (view, _, window) = makeTerminalAtomView(
            html: #"<p>Before</p><div data-type="counter-card" data-count="7"></div>"#
        )
        defer { view.removeFromSuperview(); window.isHidden = true }
        let textView = view.textView
        let atomRect = terminalAtomRect(in: textView)
        setCollapsedCaret(in: textView, utf16Offset: 3)
        textView.textViewDidChangeSelection(textView)

        let hitPosition = try XCTUnwrap(textView.closestPosition(
            to: CGPoint(x: atomRect.maxX - 1, y: atomRect.midY)
        ))
        let documentRange = try XCTUnwrap(textView.textRange(
            from: textView.beginningOfDocument,
            to: textView.endOfDocument
        ))
        let constrainedHitPosition = try XCTUnwrap(textView.closestPosition(
            to: CGPoint(x: atomRect.maxX - 1, y: atomRect.midY),
            within: documentRange
        ))

        XCTAssertEqual(
            textView.offset(from: textView.beginningOfDocument, to: hitPosition),
            3
        )
        XCTAssertEqual(
            textView.offset(from: textView.beginningOfDocument, to: constrainedHitPosition),
            3
        )
    }

    func testStagingFixedHeightParagraphHitBeforeOffscreenAtomMovesCaret() throws {
        let (view, _, window) = makeTerminalAtomView(
            html: #"""
            <p>First line</p><p>Second line</p><p>Third line</p><p>Fourth line</p>
            <p>Fifth line</p><p>Sixth line</p><p>Seventh line</p><p>Eighth line</p>
            <p>Ninth line</p><p>Tenth line</p>
            <div data-type="counter-card" data-count="7"></div>
            """#,
            initialFrame: .zero,
            finalFrame: CGRect(x: 0, y: 0, width: 320, height: 480)
        )
        defer { view.removeFromSuperview(); window.isHidden = true }
        let textView = view.textView
        view.applyTheme(EditorTheme(dictionary: [
            "contentInsets": [
                "top": 28,
                "right": 20,
                "bottom": 336,
                "left": 20,
            ],
        ]))
        view.layoutIfNeeded()
        XCTAssertTrue(textView.becomeFirstResponder())
        flushMain()
        XCTAssertEqual(textView.textContainer.size.height, CGFloat.greatestFiniteMagnitude)
        setCollapsedCaret(in: textView, utf16Offset: 3)
        textView.textViewDidChangeSelection(textView)
        let afterRange = (textView.textStorage.string as NSString).range(of: "Second")
        let glyphRange = textView.layoutManager.glyphRange(
            forCharacterRange: NSRange(location: afterRange.location + 2, length: 1),
            actualCharacterRange: nil
        )
        let glyphRect = textView.layoutManager.boundingRect(
            forGlyphRange: glyphRange,
            in: textView.textContainer
        )
        let point = CGPoint(
            x: glyphRect.midX + textView.textContainerInset.left,
            y: glyphRect.midY + textView.textContainerInset.top
        )

        let hitPosition = try XCTUnwrap(textView.closestPosition(to: point))
        let hitOffset = textView.offset(
            from: textView.beginningOfDocument,
            to: hitPosition
        )

        XCTAssertTrue(afterRange.location...NSMaxRange(afterRange) ~= hitOffset)
    }

}
