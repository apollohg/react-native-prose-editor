import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testNativeCommitEventPayloadKeepsTheCommittedUpdateSourceAndRevision() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>native</p>")
        let updateJSON = EditorV2Shadow.getCurrentState(id: editorId)

        let event = try XCTUnwrap(
            NativeEditorExpoView.nativeCommitEventPayload(
                originatingEditorId: String(editorId),
                updateJSON: updateJSON
            )
        )
        let update = parseJSONObject(updateJSON)
        let emittedUpdateJSON = try XCTUnwrap(event["updateJson"] as? String)
        let emittedUpdate = parseJSONObject(emittedUpdateJSON)

        XCTAssertEqual(event["editorId"] as? String, String(editorId))
        XCTAssertEqual(event["documentRevision"] as? String, update["documentVersion"] as? String)
        XCTAssertEqual(emittedUpdate["documentVersion"] as? String, update["documentVersion"] as? String)
    }

    func testNativeCommitEventPayloadPublishesTheCompleteAtomicSnapshot() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>a</p>")
        let viewUpdateJSON = EditorV2Shadow.insertTextScalar(id: editorId, scalarPos: 1, text: "b")
        let viewUpdate = parseJSONObject(viewUpdateJSON)
        XCTAssertNil(viewUpdate["scalarLength"])

        let event = try XCTUnwrap(
            NativeEditorExpoView.nativeCommitEventPayload(
                originatingEditorId: String(editorId),
                updateJSON: viewUpdateJSON
            )
        )
        let emittedUpdateJSON = try XCTUnwrap(event["updateJson"] as? String)
        let emittedUpdate = parseJSONObject(emittedUpdateJSON)

        XCTAssertNotNil(emittedUpdate["scalarLength"])
        XCTAssertNotNil(emittedUpdate["selection"])
        XCTAssertEqual(
            emittedUpdate["documentVersion"] as? String,
            event["documentRevision"] as? String
        )
    }

    func testEditorScopedNonCommitEventPayloadCapturesOnlyCanonicalOriginatingEditorIds() throws {
        let event = try XCTUnwrap(
            NativeEditorExpoView.editorScopedEventPayload(
                ["anchor": 2, "head": 4],
                originatingEditorId: 42
            )
        )

        XCTAssertEqual(event["editorId"] as? String, "42")
        XCTAssertEqual(event["anchor"] as? Int, 2)
        XCTAssertEqual(event["head"] as? Int, 4)
        XCTAssertNil(
            NativeEditorExpoView.editorScopedEventPayload(
                ["isFocused": true],
                originatingEditorId: 0
            )
        )
    }

    func testNonTextSelectionApplicationsClearBackwardTextDirection() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>ab</p>")

        func applyBackwardTextSelection(anchor: UInt32, head: UInt32) {
            EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: anchor, scalarHead: head)
            textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
            XCTAssertEqual(PositionBridge.cursorScalarOffset(in: textView), head)
        }

        func applySelection(_ selection: [String: Any]) throws {
            var update = parseJSONObject(EditorV2Shadow.getCurrentState(id: editorId))
            update["selection"] = selection
            let data = try JSONSerialization.data(withJSONObject: update)
            let json = try XCTUnwrap(String(data: data, encoding: .utf8))
            textView.applyUpdateJSON(json, notifyDelegate: false)
        }

        applyBackwardTextSelection(anchor: 2, head: 1)
        try applySelection([
            "type": "node",
            "pos": EditorV2Shadow.scalarToDoc(id: editorId, scalar: 1),
            "posScalar": 1,
        ])
        XCTAssertEqual(PositionBridge.cursorScalarOffset(in: textView), 2)

        applyBackwardTextSelection(anchor: 2, head: 0)
        try applySelection(["type": "all"])
        XCTAssertEqual(PositionBridge.cursorScalarOffset(in: textView), 2)
    }

    func testUnbindClearsBackwardTextDirection() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>ab</p>")
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 2, scalarHead: 1)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        XCTAssertEqual(PositionBridge.cursorScalarOffset(in: textView), 1)

        textView.unbindEditor()

        XCTAssertEqual(PositionBridge.cursorScalarOffset(in: textView), 2)
    }

    func testBindEditorFailedInitialHTMLRebindFallsBackToNewEditorState() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor(
            configJson: #"{"initialization":{"type":"localEmpty"},"policy":{"maxLength":3}}"#
        )
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: firstEditorId, initialHTML: "<p>first</p>")

        textView.bindEditor(id: secondEditorId, initialHTML: "<p>too long</p>")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: secondEditorId), "<p></p>")
        XCTAssertEqual(textView.textStorage.string, "\u{200B}")
    }

    func testBackwardSelectionRoundTripsLogicalAnchorHeadAndUsesHeadAsCaretEdge() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let delegate = EditorTextViewDelegateSpy()
        textView.editorDelegate = delegate
        textView.bindEditor(id: editorId, initialHTML: "<p>abcdef</p>")
        delegate.selectionChanges.removeAll()

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 5, scalarHead: 1)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        XCTAssertEqual(PositionBridge.cursorScalarOffset(in: textView), 1)
        textView.delegate?.textViewDidChangeSelection?(textView)
        flushMainQueue()

        XCTAssertEqual(delegate.selectionChanges.last?.anchor, EditorV2Shadow.scalarToDoc(id: editorId, scalar: 5))
        XCTAssertEqual(delegate.selectionChanges.last?.head, EditorV2Shadow.scalarToDoc(id: editorId, scalar: 1))
        let selection = currentSelection(in: editorId)
        XCTAssertEqual((selection["anchor"] as? NSNumber)?.uint32Value, EditorV2Shadow.scalarToDoc(id: editorId, scalar: 5))
        XCTAssertEqual((selection["head"] as? NSNumber)?.uint32Value, EditorV2Shadow.scalarToDoc(id: editorId, scalar: 1))
    }

    func testNativeTextMutationPreservesAuthorizedBackwardSelectionDirection() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>abcdef</p>")
        XCTAssertTrue(view.textView.becomeFirstResponder())
        _ = EditorV2Shadow.setSelectionScalar(
            id: editorId,
            scalarAnchor: 5,
            scalarHead: 1
        )
        view.textView.applyUpdateJSON(
            EditorV2Shadow.getCurrentState(id: editorId),
            notifyDelegate: false
        )

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 1),
            with: "A"
        )
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Abcdef</p>")
        let selection = currentSelection(in: editorId)
        XCTAssertEqual(
            (selection["anchor"] as? NSNumber)?.uint32Value,
            EditorV2Shadow.scalarToDoc(id: editorId, scalar: 5)
        )
        XCTAssertEqual(
            (selection["head"] as? NSNumber)?.uint32Value,
            EditorV2Shadow.scalarToDoc(id: editorId, scalar: 1)
        )
    }

    func testLengthChangingNativeTextMutationPreservesAuthorizedBackwardSelectionDirection() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>abcdef</p>")
        XCTAssertTrue(view.textView.becomeFirstResponder())
        _ = EditorV2Shadow.setSelectionScalar(
            id: editorId,
            scalarAnchor: 5,
            scalarHead: 1
        )
        view.textView.applyUpdateJSON(
            EditorV2Shadow.getCurrentState(id: editorId),
            notifyDelegate: false
        )

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 1),
            with: "AA"
        )
        view.textView.selectedRange = NSRange(location: 2, length: 4)
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>AAbcdef</p>")
        let selection = currentSelection(in: editorId)
        XCTAssertEqual(
            (selection["anchor"] as? NSNumber)?.uint32Value,
            EditorV2Shadow.scalarToDoc(id: editorId, scalar: 6)
        )
        XCTAssertEqual(
            (selection["head"] as? NSNumber)?.uint32Value,
            EditorV2Shadow.scalarToDoc(id: editorId, scalar: 2)
        )
    }

    func testSelectionMismatchPublishesAuthoritativeRefreshedMapping() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>base</p>")
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let delegate = EditorTextViewDelegateSpy()
        textView.editorDelegate = delegate
        textView.bindEditor(id: editorId, initialHTML: "<p>base</p>")
        setCollapsedSelection(in: textView, utf16Offset: 0)
        textView.delegate?.textViewDidChangeSelection?(textView)
        flushMainQueue()
        delegate.selectionChanges.removeAll()

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"990014","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"EXT"}}"#
        )
        XCTAssertNil(external.error)
        setCollapsedSelection(in: textView, utf16Offset: 2)
        textView.delegate?.textViewDidChangeSelection?(textView)
        flushMainQueue()

        XCTAssertEqual(textView.textStorage.string, "EXTbase")
        let authoritative = try XCTUnwrap(editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value)
        let state = parseJSONObject(authoritative)
        let selection = try XCTUnwrap(state["selection"] as? [String: Any])
        XCTAssertEqual(
            delegate.selectionChanges.last?.anchor,
            (selection["anchor"] as? NSNumber)?.uint32Value
        )
        XCTAssertEqual(
            delegate.selectionChanges.last?.head,
            (selection["head"] as? NSNumber)?.uint32Value
        )
    }

}
