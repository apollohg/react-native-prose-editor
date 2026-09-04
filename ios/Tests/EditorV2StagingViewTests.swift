import XCTest
import ExpoModulesCore

@MainActor
private final class TestTextDragSession: NSObject, UIDragSession {
    let items: [UIDragItem]
    var localContext: Any?
    var allowsMoveOperation: Bool { true }
    var isRestrictedToDraggingApplication: Bool { true }

    init(items: [UIDragItem]) {
        self.items = items
    }

    func location(in view: UIView) -> CGPoint { .zero }

    func hasItemsConforming(toTypeIdentifiers typeIdentifiers: [String]) -> Bool {
        items.contains { item in
            typeIdentifiers.contains { item.itemProvider.hasItemConformingToTypeIdentifier($0) }
        }
    }

    func canLoadObjects(ofClass aClass: NSItemProviderReading.Type) -> Bool {
        items.contains { $0.itemProvider.canLoadObject(ofClass: aClass) }
    }
}

@MainActor
private final class TestTextDropSession: NSObject, UIDropSession {
    let items: [UIDragItem]
    let localDragSession: UIDragSession?
    var progressIndicatorStyle: UIDropSessionProgressIndicatorStyle = .default
    let progress = Progress(totalUnitCount: 1)
    var allowsMoveOperation: Bool { true }
    var isRestrictedToDraggingApplication: Bool { true }

    init(dragSession: UIDragSession) {
        localDragSession = dragSession
        items = dragSession.items
    }

    func location(in view: UIView) -> CGPoint { .zero }

    func hasItemsConforming(toTypeIdentifiers typeIdentifiers: [String]) -> Bool {
        items.contains { item in
            typeIdentifiers.contains { item.itemProvider.hasItemConformingToTypeIdentifier($0) }
        }
    }

    func canLoadObjects(ofClass aClass: NSItemProviderReading.Type) -> Bool {
        items.contains { $0.itemProvider.canLoadObject(ofClass: aClass) }
    }

    func loadObjects(
        ofClass aClass: NSItemProviderReading.Type,
        completion: @escaping ([NSItemProviderReading]) -> Void
    ) -> Progress {
        completion([])
        return progress
    }
}

@MainActor
private final class TestTextDragRequest: NSObject, UITextDragRequest {
    let dragRange: UITextRange
    let suggestedItems: [UIDragItem]
    let existingItems: [UIDragItem] = []
    let isSelected: Bool
    let dragSession: UIDragSession

    init(
        dragRange: UITextRange,
        suggestedItems: [UIDragItem],
        isSelected: Bool,
        dragSession: UIDragSession
    ) {
        self.dragRange = dragRange
        self.suggestedItems = suggestedItems
        self.isSelected = isSelected
        self.dragSession = dragSession
    }
}

@MainActor
private final class TestTextDropRequest: NSObject, UITextDropRequest {
    let dropPosition: UITextPosition
    let suggestedProposal: UITextDropProposal
    let isSameView: Bool
    let dropSession: UIDropSession

    init(
        dropPosition: UITextPosition,
        isSameView: Bool,
        dropSession: UIDropSession
    ) {
        self.dropPosition = dropPosition
        self.isSameView = isSameView
        self.dropSession = dropSession
        suggestedProposal = UITextDropProposal(operation: .copy)
    }
}

// MARK: - v2 view integration tests (formerly the staging-variant suite)
//
// The view is bound to a v2 session through the session pairing registry, so
// every interaction — typing, marked text, autocorrect, selection, toolbar,
// accessibility-style edits, render patches — flows through the typed v2
// transactions. This is the only engine path: no legacy runtime exists.
final class EditorV2StagingViewTests: XCTestCase {

    private var adapters: [EditorV2Adapter] = []
    private var syntheticIds: [UInt64] = []

    override func tearDown() {
        for id in syntheticIds {
            EditorV2Registry.destroyPair(forLegacyId: id)
        }
        syntheticIds = []
        adapters = []
        super.tearDown()
    }

    private func hostStagingView(_ view: RichTextEditorView) -> UIWindow {
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        view.layoutIfNeeded()
        return window
    }

    private func makeBoundView(
        configJson: String = #"{"initialization":{"type":"localEmpty"}}"#,
        html: String = "<p>Hello</p>",
        initialFrame: CGRect = CGRect(x: 0, y: 0, width: 320, height: 120),
        finalFrame: CGRect? = nil,
        file: StaticString = #filePath,
        line: UInt = #line
    ) -> (view: RichTextEditorView, adapter: EditorV2Adapter, window: UIWindow) {
        let syntheticId = makeV2Editor(configJson: configJson, file: file, line: line)
        guard let adapter = EditorV2Registry.adapter(forLegacyId: syntheticId) else {
            XCTFail("v2 adapter was not paired to its created handle", file: file, line: line)
            fatalError("unreachable")
        }
        adapters.append(adapter)
        syntheticIds.append(syntheticId)
        let view = RichTextEditorView(frame: initialFrame)
        let window = hostStagingView(view)
        view.editorId = syntheticId
        view.setContent(html: html)
        if let finalFrame {
            view.frame = finalFrame
            view.layoutIfNeeded()
        }
        return (view, adapter, window)
    }

    private func makeTerminalAtomView(
        html: String = #"<div data-type="counter-card" data-count="7"></div>"#,
        initialFrame: CGRect = CGRect(x: 0, y: 0, width: 320, height: 120),
        finalFrame: CGRect? = nil,
        file: StaticString = #filePath,
        line: UInt = #line
    ) -> (view: RichTextEditorView, adapter: EditorV2Adapter, window: UIWindow) {
        let configJson = #"""
        {
          "initialization":{"type":"localEmpty"},
          "schema":{
            "nodes":[
              {"name":"doc","content":"block+","role":"doc"},
              {"name":"paragraph","content":"text*","group":"block","role":"textBlock","htmlTag":"p"},
              {"name":"text","content":"","role":"text"},
              {
                "name":"counterCard",
                "content":"",
                "group":"block",
                "role":"block",
                "isVoid":true,
                "attrs":{"count":{"default":0}},
                "html":{
                  "tag":"div",
                  "staticAttrs":{"data-type":"counter-card"},
                  "attrMap":{"count":"data-count"}
                }
              }
            ],
            "marks":[]
          }
        }
        """#
        let bound = makeBoundView(
            configJson: configJson,
            html: html,
            initialFrame: initialFrame,
            finalFrame: finalFrame,
            file: file,
            line: line
        )
        XCTAssertTrue(
            bound.view.applyAtomRenderConfiguration(
                AtomRenderConfiguration(
                    registeredNodeTypes: ["counterCard"],
                    estimatedHeights: ["counterCard": 72],
                    measuredHeights: [:]
                )
            ),
            file: file,
            line: line
        )
        bound.view.layoutIfNeeded()
        return bound
    }

    private func terminalAtomRect(
        in textView: EditorTextView,
        file: StaticString = #filePath,
        line: UInt = #line
    ) -> CGRect {
        guard textView.textStorage.length > 0 else {
            XCTFail("expected terminal atom text", file: file, line: line)
            return .zero
        }
        let range = NSRange(location: textView.textStorage.length - 1, length: 1)
        let glyphRange = textView.layoutManager.glyphRange(
            forCharacterRange: range,
            actualCharacterRange: nil
        )
        let rect = textView.layoutManager.boundingRect(
            forGlyphRange: glyphRange,
            in: textView.textContainer
        )
        return rect.offsetBy(
            dx: textView.textContainerInset.left,
            dy: textView.textContainerInset.top
        )
    }

    private func flushMain() {
        let expectation = expectation(description: "flush main")
        DispatchQueue.main.async { expectation.fulfill() }
        wait(for: [expectation], timeout: 1.0)
    }

    private func flushMain(until condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(1.0)
        repeat {
            flushMain()
        } while !condition() && Date() < deadline
    }

    private func setCollapsedCaret(in textView: UITextView, utf16Offset: Int) {
        textView.selectedRange = NSRange(location: utf16Offset, length: 0)
    }

    private func v2DocumentText(_ adapter: EditorV2Adapter, file: StaticString = #filePath, line: UInt = #line) -> String {
        let result = editorV2GetDocumentJson(editorId: adapter.editorId)
        guard let value = result.value, result.error == nil else {
            XCTFail("getDocumentJson failed: \(String(describing: result.error))", file: file, line: line)
            return ""
        }
        guard let data = value.data(using: .utf8),
              let doc = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return "" }
        var pieces: [String] = []
        func walk(_ node: [String: Any]) {
            if let type = node["type"] as? String, type == "text", let text = node["text"] as? String {
                pieces.append(text)
            }
            for child in (node["content"] as? [[String: Any]]) ?? [] { walk(child) }
        }
        walk(doc)
        return pieces.joined()
    }

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
