import XCTest
import ExpoModulesCore

extension EditorV2StagingViewTests {
    func testStagingReturnReplacementUsesCollapsedSuppliedRange() throws {
        for returnText in ["\n", "\r"] {
            let (view, _, window) = makeBoundView(html: "<p>abcd</p>")
            defer { view.removeFromSuperview(); window.isHidden = true }
            setCollapsedCaret(in: view.textView, utf16Offset: 0)
            let position = try XCTUnwrap(
                view.textView.position(from: view.textView.beginningOfDocument, offset: 2)
            )
            let replacementRange = try XCTUnwrap(
                view.textView.textRange(from: position, to: position)
            )

            view.textView.replace(replacementRange, withText: returnText)

            XCTAssertEqual(
                EditorV2Shadow.getHtml(id: view.editorId),
                "<p>ab</p><p>cd</p>"
            )
        }
    }

    func testStagingReturnReplacementUsesNoncollapsedSuppliedRange() throws {
        let (view, _, window) = makeBoundView(html: "<p>abcd</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        setCollapsedCaret(in: view.textView, utf16Offset: 0)
        let start = try XCTUnwrap(
            view.textView.position(from: view.textView.beginningOfDocument, offset: 1)
        )
        let end = try XCTUnwrap(view.textView.position(from: start, offset: 2))
        let replacementRange = try XCTUnwrap(
            view.textView.textRange(from: start, to: end)
        )

        view.textView.replace(replacementRange, withText: "\n")

        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: view.editorId),
            "<p>a</p><p>d</p>"
        )
    }

    func testStagingBackspaceAtTerminalAtomBoundaryDoesNotChangeDocument() {
        let (view, _, window) = makeTerminalAtomView()
        defer { view.removeFromSuperview(); window.isHidden = true }
        let htmlBefore = EditorV2Shadow.getHtml(id: view.editorId)
        setCollapsedCaret(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMain()

        view.textView.deleteBackward()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: view.editorId), htmlBefore)
    }

    func testStagingMoveSelectionCommandReordersText() {
        let (view, adapter, window) = makeBoundView(html: "<p>abcd</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }

        guard let updateJSON = adapter.moveSelection(anchor: 0, head: 2, to: 4) else {
            XCTFail("expected move selection update")
            return
        }
        XCTAssertTrue(view.textView.applyUpdateJSON(updateJSON))
        XCTAssertEqual(v2DocumentText(adapter), "cdab")
    }

    @MainActor
    func testStagingSameViewTextDropRestoresCleanupAndAcceptsTypingBeforeSessionEnd() throws {
        let (view, adapter, window) = makeBoundView(html: "<p>abcd</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        let delegate = EditorTextViewDelegateSpy()
        view.textView.editorDelegate = delegate
        delegate.receivedUpdates.removeAll()
        let item = UIDragItem(itemProvider: NSItemProvider(object: "ab" as NSString))
        let dragSession = TestTextDragSession(items: [item])
        let dropSession = TestTextDropSession(dragSession: dragSession)
        let source = try XCTUnwrap(view.textView.textRange(
            from: try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 0)),
            to: try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 2))
        ))
        let dragRequest = TestTextDragRequest(
            dragRange: source,
            suggestedItems: [item],
            isSelected: true,
            dragSession: dragSession
        )
        _ = view.textView.textDraggableView(view.textView, itemsForDrag: dragRequest)
        let dropRequest = TestTextDropRequest(
            dropPosition: try XCTUnwrap(
                view.textView.position(from: view.textView.beginningOfDocument, offset: 4)
            ),
            isSameView: true,
            dropSession: dropSession
        )

        let proposal = view.textView.textDroppableView(
            view.textView,
            proposalForDrop: dropRequest
        )
        XCTAssertEqual(proposal.operation, .move)
        XCTAssertEqual(proposal.dropPerformer, .delegate)
        XCTAssertFalse(proposal.useFastSameViewOperations)

        view.textView.textDroppableView(view.textView, willPerformDrop: dropRequest)
        XCTAssertEqual(v2DocumentText(adapter), "cdab")
        XCTAssertEqual(view.textView.textStorage.string, "cdab")
        XCTAssertEqual(delegate.receivedUpdates.count, 1)

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 2),
            with: ""
        )
        XCTAssertEqual(view.textView.textStorage.string, "cdab")
        setCollapsedCaret(in: view.textView, utf16Offset: view.textView.textStorage.length)
        view.textView.insertText("x")
        flushMain()

        XCTAssertEqual(v2DocumentText(adapter), "cdabx")
        XCTAssertEqual(view.textView.textStorage.string, "cdabx")

        view.textView.textDraggableView(
            view.textView,
            dragSessionDidEnd: dragSession,
            with: .move
        )
        flushMain()

        XCTAssertEqual(v2DocumentText(adapter), "cdabx")
        XCTAssertEqual(view.textView.textStorage.string, "cdabx")
        XCTAssertEqual(delegate.receivedUpdates.count, 2)

        _ = adapter.undo()
        XCTAssertEqual(v2DocumentText(adapter), "cdab")
        _ = adapter.undo()
        XCTAssertEqual(v2DocumentText(adapter), "abcd")
    }

    @MainActor
    func testStagingSameViewTextDropIgnoresCleanupAfterARenderMutation() throws {
        let (view, adapter, window) = makeBoundView(html: "<p>abcd</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        let item = UIDragItem(itemProvider: NSItemProvider(object: "ab" as NSString))
        let dragSession = TestTextDragSession(items: [item])
        let source = try XCTUnwrap(view.textView.textRange(
            from: view.textView.beginningOfDocument,
            to: try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 2))
        ))
        _ = view.textView.textDraggableView(
            view.textView,
            itemsForDrag: TestTextDragRequest(
                dragRange: source,
                suggestedItems: [item],
                isSelected: true,
                dragSession: dragSession
            )
        )
        let drop = TestTextDropRequest(
            dropPosition: try XCTUnwrap(
                view.textView.position(from: view.textView.beginningOfDocument, offset: 4)
            ),
            isSameView: true,
            dropSession: TestTextDropSession(dragSession: dragSession)
        )
        view.textView.textDroppableView(view.textView, willPerformDrop: drop)
        XCTAssertEqual(v2DocumentText(adapter), "cdab")

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: view.textView.textStorage.length),
            with: "cdab"
        )
        view.textView.textStorage.replaceCharacters(in: NSRange(location: 0, length: 2), with: "")
        flushMain()

        XCTAssertEqual(v2DocumentText(adapter), "cdab")
        XCTAssertEqual(view.textView.textStorage.string, "cdab")
    }

    @MainActor
    func testStagingSameViewAtomDropPreservesAttributesAndUndoesInOneStep() throws {
        let htmlBefore = #"<div data-type="counter-card" data-count="7"></div><p>x</p>"#
        let (view, adapter, window) = makeTerminalAtomView(html: htmlBefore)
        defer { view.removeFromSuperview(); window.isHidden = true }
        let item = UIDragItem(itemProvider: NSItemProvider(object: "atom" as NSString))
        let dragSession = TestTextDragSession(items: [item])
        let source = try XCTUnwrap(view.textView.textRange(
            from: view.textView.beginningOfDocument,
            to: try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 1))
        ))
        _ = view.textView.textDraggableView(
            view.textView,
            itemsForDrag: TestTextDragRequest(
                dragRange: source,
                suggestedItems: [item],
                isSelected: false,
                dragSession: dragSession
            )
        )
        let dropRequest = TestTextDropRequest(
            dropPosition: view.textView.endOfDocument,
            isSameView: true,
            dropSession: TestTextDropSession(dragSession: dragSession)
        )

        XCTAssertEqual(
            view.textView.textDroppableView(view.textView, proposalForDrop: dropRequest).operation,
            .move
        )
        view.textView.textDroppableView(view.textView, willPerformDrop: dropRequest)
        view.textView.textDraggableView(
            view.textView,
            dragSessionDidEnd: dragSession,
            with: .move
        )
        flushMain()

        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: view.editorId),
            #"<p>x</p><div data-type="counter-card" data-count="7"></div>"#
        )
        _ = adapter.undo()
        XCTAssertEqual(EditorV2Shadow.getHtml(id: view.editorId), htmlBefore)
    }

    @MainActor
    func testStagingSameViewTextDropRejectsDestinationInsideSourceRange() throws {
        let (view, _, window) = makeBoundView(html: "<p>abcd</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        let item = UIDragItem(itemProvider: NSItemProvider(object: "ab" as NSString))
        let dragSession = TestTextDragSession(items: [item])
        let source = try XCTUnwrap(view.textView.textRange(
            from: try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 0)),
            to: try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 2))
        ))
        _ = view.textView.textDraggableView(
            view.textView,
            itemsForDrag: TestTextDragRequest(
                dragRange: source,
                suggestedItems: [item],
                isSelected: true,
                dragSession: dragSession
            )
        )
        let dropRequest = TestTextDropRequest(
            dropPosition: try XCTUnwrap(
                view.textView.position(from: view.textView.beginningOfDocument, offset: 1)
            ),
            isSameView: true,
            dropSession: TestTextDropSession(dragSession: dragSession)
        )

        XCTAssertEqual(
            view.textView.textDroppableView(view.textView, proposalForDrop: dropRequest).operation,
            .forbidden
        )
        view.textView.textDraggableView(
            view.textView,
            dragSessionDidEnd: dragSession,
            with: .cancel
        )
    }

    @MainActor
    func testStagingSameViewTextDropRejectsCrossParagraphSelection() throws {
        let (view, _, window) = makeBoundView(html: "<p>ab</p><p>cd</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        let item = UIDragItem(itemProvider: NSItemProvider(object: "ab\nc" as NSString))
        let dragSession = TestTextDragSession(items: [item])
        let source = try XCTUnwrap(view.textView.textRange(
            from: view.textView.beginningOfDocument,
            to: try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 4))
        ))
        _ = view.textView.textDraggableView(
            view.textView,
            itemsForDrag: TestTextDragRequest(
                dragRange: source,
                suggestedItems: [item],
                isSelected: true,
                dragSession: dragSession
            )
        )
        let dropRequest = TestTextDropRequest(
            dropPosition: view.textView.endOfDocument,
            isSameView: true,
            dropSession: TestTextDropSession(dragSession: dragSession)
        )

        XCTAssertEqual(
            view.textView.textDroppableView(view.textView, proposalForDrop: dropRequest).operation,
            .forbidden
        )
    }

    @MainActor
    func testStagingSameViewTextDropRejectsSelectionEndingAtNextParagraphStart() throws {
        let (view, _, window) = makeBoundView(html: "<p>ab</p><p>cd</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        let item = UIDragItem(itemProvider: NSItemProvider(object: "ab\n" as NSString))
        let dragSession = TestTextDragSession(items: [item])
        let source = try XCTUnwrap(view.textView.textRange(
            from: view.textView.beginningOfDocument,
            to: try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 3))
        ))
        _ = view.textView.textDraggableView(
            view.textView,
            itemsForDrag: TestTextDragRequest(
                dragRange: source,
                suggestedItems: [item],
                isSelected: true,
                dragSession: dragSession
            )
        )
        let dropRequest = TestTextDropRequest(
            dropPosition: view.textView.endOfDocument,
            isSameView: true,
            dropSession: TestTextDropSession(dragSession: dragSession)
        )

        XCTAssertEqual(
            view.textView.textDroppableView(view.textView, proposalForDrop: dropRequest).operation,
            .forbidden
        )
    }

    @MainActor
    func testStagingSameViewTextDropAllowsSelectionSpanningInlineHardBreak() throws {
        let (view, _, window) = makeBoundView(html: "<p>a<br>bcd</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        let item = UIDragItem(itemProvider: NSItemProvider(object: "a\nb" as NSString))
        let dragSession = TestTextDragSession(items: [item])
        let source = try XCTUnwrap(view.textView.textRange(
            from: view.textView.beginningOfDocument,
            to: try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 3))
        ))
        _ = view.textView.textDraggableView(
            view.textView,
            itemsForDrag: TestTextDragRequest(
                dragRange: source,
                suggestedItems: [item],
                isSelected: true,
                dragSession: dragSession
            )
        )
        let dropRequest = TestTextDropRequest(
            dropPosition: view.textView.endOfDocument,
            isSameView: true,
            dropSession: TestTextDropSession(dragSession: dragSession)
        )

        XCTAssertEqual(
            view.textView.textDroppableView(view.textView, proposalForDrop: dropRequest).operation,
            .move
        )
    }

}
