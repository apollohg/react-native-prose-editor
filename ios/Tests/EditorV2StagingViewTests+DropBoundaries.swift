import XCTest
import ExpoModulesCore

extension EditorV2StagingViewTests {
    @MainActor
    func testStagingSameViewTextDropAllowsSelectionSpanningCodeBlockNewline() throws {
        let configJson = #"""
        {
          "initialization":{"type":"localEmpty"},
          "schema":{
            "nodes":[
              {"name":"doc","content":"block+","role":"doc"},
              {"name":"paragraph","content":"text*","group":"block","role":"textBlock","htmlTag":"p"},
              {"name":"codeBlock","content":"text*","group":"block","role":"textBlock","htmlTag":"pre"},
              {"name":"text","content":"","role":"text"}
            ],
            "marks":[]
          }
        }
        """#
        let (view, _, window) = makeBoundView(configJson: configJson)
        defer { view.removeFromSuperview(); window.isHidden = true }
        view.setContent(json: """
        {
          "type":"doc",
          "content":[{"type":"codeBlock","content":[{"type":"text","text":"a\\nbcd"}]}]
        }
        """)
        XCTAssertEqual(view.textView.textStorage.string, "a\nbcd")
        XCTAssertNil(
            view.textView.textStorage.attribute(
                RenderBridgeAttributes.voidNodeType,
                at: 1,
                effectiveRange: nil
            )
        )
        XCTAssertEqual(
            view.textView.textStorage.attribute(
                RenderBridgeAttributes.blockNodeType,
                at: 1,
                effectiveRange: nil
            ) as? String,
            "codeBlock"
        )
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

    @MainActor
    func testStagingSameViewTextDropRejectsStaleSourceRevision() throws {
        let (view, _, window) = makeBoundView(html: "<p>abcd</p>")
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

        view.setContent(html: "<p>wxyz</p>")
        let dropRequest = TestTextDropRequest(
            dropPosition: view.textView.endOfDocument,
            isSameView: true,
            dropSession: TestTextDropSession(dragSession: dragSession)
        )

        XCTAssertEqual(
            view.textView.textDroppableView(view.textView, proposalForDrop: dropRequest).operation,
            .forbidden
        )
        view.textView.textDroppableView(view.textView, willPerformDrop: dropRequest)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: view.editorId), "<p>wxyz</p>")
    }

    @MainActor
    func testStagingSameViewTextDropRejectsRevisionAdvancedByPendingMutationFlush() throws {
        let (view, adapter, window) = makeBoundView(html: "<p>abcd</p>")
        defer { view.removeFromSuperview(); window.isHidden = true }
        XCTAssertTrue(view.textView.becomeFirstResponder())
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
        let revisionBefore = adapter.baseDocumentRevision
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 3, length: 1),
            with: "z"
        )
        let dropRequest = TestTextDropRequest(
            dropPosition: view.textView.endOfDocument,
            isSameView: true,
            dropSession: TestTextDropSession(dragSession: dragSession)
        )

        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore)
        XCTAssertEqual(
            view.textView.textDroppableView(view.textView, proposalForDrop: dropRequest).operation,
            .move
        )
        view.textView.textDroppableView(view.textView, willPerformDrop: dropRequest)

        XCTAssertEqual(v2DocumentText(adapter), "abcz")
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore + 1)
    }

    @MainActor
    func testStagingTextDropStartedBeforeRebindCannotMutateTheNewEditor() throws {
        let (view, firstAdapter, window) = makeBoundView(html: "<p>abcd</p>")
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

        let secondId = makeV2Editor()
        let secondAdapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: secondId))
        syntheticIds.append(secondId)
        adapters.append(secondAdapter)
        view.editorId = secondId
        view.setContent(html: "<p>wxyz</p>")
        let dropRequest = TestTextDropRequest(
            dropPosition: view.textView.endOfDocument,
            isSameView: true,
            dropSession: TestTextDropSession(dragSession: dragSession)
        )

        XCTAssertEqual(
            view.textView.textDroppableView(view.textView, proposalForDrop: dropRequest).operation,
            .copy
        )
        view.textView.textDroppableView(view.textView, willPerformDrop: dropRequest)
        XCTAssertEqual(v2DocumentText(firstAdapter), "abcd")
        XCTAssertEqual(v2DocumentText(secondAdapter), "wxyz")
    }

    @MainActor
    func testStagingAttachmentDragNarrowsSuggestedRangeToTheAtom() throws {
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let attributedText = NSMutableAttributedString(attachment: NSTextAttachment())
        attributedText.addAttribute(
            RenderBridgeAttributes.voidNodeType,
            value: "counterCard",
            range: NSRange(location: 0, length: 1)
        )
        attributedText.append(NSAttributedString(string: "\n"))
        textView.attributedText = attributedText
        let editorId = makeV2Editor()
        syntheticIds.append(editorId)
        textView.editorId = editorId
        let item = UIDragItem(itemProvider: NSItemProvider(object: "atom" as NSString))
        let dragSession = TestTextDragSession(items: [item])
        let source = try XCTUnwrap(textView.textRange(
            from: textView.beginningOfDocument,
            to: try XCTUnwrap(textView.position(from: textView.beginningOfDocument, offset: 2))
        ))
        _ = textView.textDraggableView(
            textView,
            itemsForDrag: TestTextDragRequest(
                dragRange: source,
                suggestedItems: [item],
                isSelected: false,
                dragSession: dragSession
            )
        )
        let dropRequest = TestTextDropRequest(
            dropPosition: textView.endOfDocument,
            isSameView: true,
            dropSession: TestTextDropSession(dragSession: dragSession)
        )

        XCTAssertEqual(
            textView.textDroppableView(textView, proposalForDrop: dropRequest).operation,
            .move
        )
    }

}
