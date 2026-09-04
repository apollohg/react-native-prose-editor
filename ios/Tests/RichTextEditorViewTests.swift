import XCTest
import ExpoModulesCore

final class RichTextEditorViewTests: XCTestCase {
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

    func testImageAttachmentLoadNotificationOnlyInvalidatesOwningEditor() {
        let first = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let second = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let firstAttachment = NSTextAttachment()
        let secondAttachment = NSTextAttachment()
        first.attributedText = NSAttributedString(attachment: firstAttachment)
        second.attributedText = NSAttributedString(attachment: secondAttachment)
        var firstInvalidations = 0
        var secondInvalidations = 0
        first.onSelectionOrContentMayChange = { firstInvalidations += 1 }
        second.onSelectionOrContentMayChange = { secondInvalidations += 1 }

        NotificationCenter.default.post(
            name: .editorImageAttachmentDidLoad,
            object: firstAttachment
        )

        XCTAssertEqual(firstInvalidations, 1)
        XCTAssertEqual(secondInvalidations, 0)
    }

    func testRegistryLifecycleStateContainsOnlyLiveEditors() {
        let registry = NativeEditorViewRegistry.shared
        let stateBefore = Mirror(reflecting: registry).children.first {
            $0.label == "activeEditorIds"
        }?.value as? Set<UInt64>
        XCTAssertNotNil(stateBefore, "registry should track active editors instead of destroyed-ID tombstones")

        let destroyedEditorIds = (0..<128).map { UInt64(9_000_000_000 + $0) }
        for editorId in destroyedEditorIds {
            registry.markEditorCreated(editorId: editorId)
            registry.invalidateDestroyedEditor(editorId: editorId)
        }
        let stateAfterDestroyedEditors = Mirror(reflecting: registry).children.first {
            $0.label == "activeEditorIds"
        }?.value as? Set<UInt64>
        XCTAssertEqual(stateAfterDestroyedEditors, stateBefore)

        let liveEditorId: UInt64 = 9_000_001_000
        registry.markEditorCreated(editorId: liveEditorId)
        defer { registry.invalidateDestroyedEditor(editorId: liveEditorId) }
        let stateWithLiveEditor = Mirror(reflecting: registry).children.first {
            $0.label == "activeEditorIds"
        }?.value as? Set<UInt64>

        XCTAssertTrue(stateWithLiveEditor?.contains(liveEditorId) ?? false)
    }

    func testEditorTextViewDisablesNativeUndoManager() {
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))

        XCTAssertNil(
            textView.undoManager,
            "native UIKit undo should stay disabled because editor history is owned by Rust"
        )
    }

    func testEditorTextViewUsesRichTextKeyboardDefaults() {
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))

        XCTAssertEqual(textView.autocapitalizationType, .sentences)
        XCTAssertEqual(textView.autocorrectionType, .no)
        XCTAssertEqual(textView.spellCheckingType, .no)
        XCTAssertEqual(textView.keyboardType, .default)
    }

    func testEditorTextViewAppliesReactKeyboardProps() {
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))

        textView.setAutoCapitalize("characters")
        textView.setAutoCorrect(true)
        textView.setKeyboardType("email-address")

        XCTAssertEqual(textView.autocapitalizationType, .allCharacters)
        XCTAssertEqual(textView.autocorrectionType, .yes)
        XCTAssertEqual(textView.spellCheckingType, .default)
        XCTAssertEqual(textView.keyboardType, .emailAddress)
    }

    func testPlaceholderShowsForRenderedEmptyParagraph() {
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.placeholder = "Type here"
        textView.applyRenderJSON("""
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"\\u200B","marks":[]},
          {"type":"blockEnd"}
        ]
        """)

        XCTAssertTrue(textView.isPlaceholderVisibleForTesting())
    }

    /// An empty bullet is content the user can see, so the placeholder must go.
    ///
    /// The document renders no characters at all — the bullet marker comes from
    /// block structure, never from stored text — so the view cannot work this
    /// out by inspecting its own text storage. It has to take the core's
    /// `documentIsEmpty` from the update, which is what this drives.
    func testPlaceholderHidesWhenTheCoreReportsAnEmptyListItemAsContent() {
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.placeholder = "Type here"
        textView.applyUpdateJSON("""
        {
          "renderBlocks": [[
            {"type":"blockStart","nodeType":"bulletList","depth":0},
            {"type":"blockStart","nodeType":"listItem","depth":1,
             "listContext":{"ordered":false,"index":0,"total":1,"start":1,
                            "isFirst":true,"isLast":true}},
            {"type":"blockStart","nodeType":"paragraph","depth":2},
            {"type":"textRun","text":"\\u200B","marks":[]},
            {"type":"blockEnd"},
            {"type":"blockEnd"},
            {"type":"blockEnd"}
          ]],
          "documentIsEmpty": false
        }
        """)

        XCTAssertFalse(
            textView.isPlaceholderVisibleForTesting(),
            "a document containing an empty bullet is not an empty editor"
        )
    }

    /// The companion: the core reporting a genuinely empty document keeps the
    /// placeholder up, so the fix cannot be "never show the placeholder".
    func testPlaceholderShowsWhenTheCoreReportsAnEmptyDocument() {
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.placeholder = "Type here"
        textView.applyUpdateJSON("""
        {
          "renderBlocks": [[
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"\\u200B","marks":[]},
            {"type":"blockEnd"}
          ]],
          "documentIsEmpty": true
        }
        """)

        XCTAssertTrue(
            textView.isPlaceholderVisibleForTesting(),
            "an editor the core reports as empty must still show its placeholder"
        )
    }

    func testPlaceholderHidesForRenderedNonEmptyParagraph() {
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.placeholder = "Type here"
        textView.applyRenderJSON("""
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"Hello","marks":[]},
          {"type":"blockEnd"}
        ]
        """)

        XCTAssertFalse(textView.isPlaceholderVisibleForTesting())
    }

    func testPlaceholderStaysTopAlignedInTallEditor() {
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        textView.placeholder = "Line 1\nLine 2"
        textView.applyRenderJSON("""
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"\\u200B","marks":[]},
          {"type":"blockEnd"}
        ]
        """)
        textView.layoutIfNeeded()

        let placeholderFrame = textView.placeholderFrameForTesting()
        XCTAssertEqual(placeholderFrame.minY, textView.textContainerInset.top, accuracy: 0.1)
        XCTAssertLessThan(placeholderFrame.height, 200)
    }

    func testEmptyDocumentSelectionStaysBeforePlaceholderForAutocapitalization() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId)

        XCTAssertEqual(textView.text, "\u{200B}")
        XCTAssertEqual(
            textView.offset(
                from: textView.beginningOfDocument,
                to: textView.selectedTextRange?.start ?? textView.endOfDocument
            ),
            0,
            "empty single-block documents should keep the caret at paragraph start so UIKit auto-capitalization still applies"
        )
    }

    func testEmptyDocumentFocusRepositionsCaretBeforePlaceholderForAutocapitalization() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId

        setCollapsedSelection(in: view.textView, utf16Offset: 1)
        XCTAssertTrue(view.textView.becomeFirstResponder())

        XCTAssertEqual(
            view.textView.offset(
                from: view.textView.beginningOfDocument,
                to: view.textView.selectedTextRange?.start ?? view.textView.endOfDocument
            ),
            0,
            "focus should keep the caret before the empty-paragraph placeholder so UIKit sentence capitalization still treats the editor as empty"
        )
    }

    func testFirstCharacterEmojiInsertedIntoEmptyDocumentRendersVisibleGlyph() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId

        setCollapsedSelection(in: view.textView, utf16Offset: 0)
        view.textView.insertText("😀")
        flushMainQueue()
        view.layoutIfNeeded()
        view.textView.layoutIfNeeded()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>😀</p>")
        XCTAssertEqual(view.textView.textStorage.string, "😀")

        let nsString = view.textView.textStorage.string as NSString
        let emojiRange = nsString.rangeOfComposedCharacterSequence(at: 0)
        view.textView.layoutManager.ensureLayout(for: view.textView.textContainer)
        let rect = renderedRect(in: view.textView, utf16Range: emojiRange)

        XCTAssertGreaterThan(emojiRange.length, 1, "test must cover a surrogate-pair emoji")
        XCTAssertGreaterThan(rect.width, 0, "leading emoji should have a visible glyph width")
        XCTAssertGreaterThan(rect.height, 0, "leading emoji should have a visible glyph height")
    }

    func testCurrentCaretRectReportsEditorLocalCoordinates() throws {
        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.textView.applyRenderJSON("""
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"Hello world","marks":[]},
          {"type":"blockEnd"}
        ]
        """)
        view.layoutIfNeeded()
        view.textView.layoutIfNeeded()
        setCollapsedSelection(in: view.textView, utf16Offset: 5)

        let selectedTextRange = try XCTUnwrap(view.textView.selectedTextRange)
        let expected = view.textView.convert(
            view.textView.caretRect(for: selectedTextRange.end),
            to: view
        )
        let actual = try XCTUnwrap(view.currentCaretRect())

        XCTAssertEqual(actual.minX, expected.minX, accuracy: 0.1)
        XCTAssertEqual(actual.minY, expected.minY, accuracy: 0.1)
        XCTAssertEqual(actual.width, expected.width, accuracy: 0.1)
        XCTAssertEqual(actual.height, expected.height, accuracy: 0.1)
        XCTAssertGreaterThan(actual.height, 0)
    }

    func testEmptyDocumentSelectionDriftSnapsBackBeforePlaceholderForAutocapitalization() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId)

        setCollapsedSelection(in: textView, utf16Offset: 1)
        textView.refreshSelectionVisualState()

        XCTAssertEqual(
            textView.offset(
                from: textView.beginningOfDocument,
                to: textView.selectedTextRange?.start ?? textView.endOfDocument
            ),
            0,
            "selection refreshes should snap a collapsed caret off the synthetic empty-block placeholder back to the paragraph start"
        )
    }

    func testNativeEditReclaimsKeyboardProviderTextViewDelegateBeforeRustUpdate() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 5, scalarHead: 5)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        let delegateSpy = KeyboardProviderTextViewDelegateSpy(textViewDelegate: textView.delegate)
        textView.delegate = delegateSpy
        XCTAssertFalse(textView.isUsingInternalTextViewDelegateForTesting())

        textView.insertText("!")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello!</p>")
        XCTAssertEqual(
            delegateSpy.selectionChangeCount,
            0,
            "KeyboardProvider-style delegates should not inspect transient selection while Rust applies an edit"
        )
        XCTAssertEqual(delegateSpy.textChangeCount, 0)
        XCTAssertTrue(textView.isUsingInternalTextViewDelegateForTesting())
    }

    func testInternalTextViewDelegateDoesNotEchoPrivateUITextViewSelectorsThroughDelegateProxies() {
        // APOLLO-REACT-56: react-native-keyboard-controller wraps the focused
        // text view's delegate in a composite that forwards unhandled selectors
        // to the wrapped delegate. UIKit invokes the private
        // `keyboardInputChangedSelection:` on UITextView, which relays it to the
        // delegate when `respondsToSelector:` says yes. If the wrapped delegate
        // is the text view itself, the relay bounces text view -> proxy -> text
        // view until the stack overflows (EXC_BAD_ACCESS).
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))

        let keyboardInputChangedSelection = NSSelectorFromString("keyboardInputChangedSelection:")
        XCTAssertTrue(
            textView.responds(to: keyboardInputChangedSelection),
            "expected UITextView to implement the private keyboardInputChangedSelection: selector; if UIKit removed it, this regression test needs a new recursion-prone selector"
        )

        XCTAssertNotNil(textView.delegate, "EditorTextView should install its internal delegate on init")
        XCTAssertFalse(
            (textView.delegate as AnyObject?) === (textView as AnyObject),
            "EditorTextView must not be its own UITextViewDelegate: delegate-proxy keyboard integrations forward UITextView's private selectors back to the wrapped delegate, recursing forever when that delegate is the text view itself"
        )

        let composite = ForwardingCompositeTextViewDelegateSpy(wrappedDelegate: textView.delegate)
        textView.delegate = composite
        XCTAssertFalse(
            composite.responds(to: keyboardInputChangedSelection),
            "a KCTextInputCompositeDelegate-style proxy wrapping the editor's delegate must not claim to handle keyboardInputChangedSelection:, otherwise UIKit forwards it and the call recurses back into the text view"
        )
    }

    /// Return in an empty editor must add a blank line.
    ///
    /// The engine handles this (see the typing regression suite), so anything
    /// that swallows the keystroke does so on the way in: an empty document
    /// renders as a single zero-width placeholder, and input paths that reason
    /// about the text storage can mistake that for "nothing to split".
    func testReturnInAnEmptyEditorAddsABlankLine() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "")

        textView.insertText("\n")

        XCTAssertEqual(
            textView.textStorage.string,
            "\u{200B}\n\u{200B}",
            "Return on an empty line must leave two blank lines, each rendering "
                + "its own empty-block placeholder"
        )
    }

    /// The caret has to land on the blank line Return created, otherwise the
    /// next character typed goes back onto the first line.
    func testReturnInAnEmptyEditorLeavesTheCaretOnTheSecondLine() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "")

        textView.insertText("\n")
        textView.insertText("x")

        XCTAssertEqual(
            textView.textStorage.string,
            "\u{200B}\nx",
            "typing after Return must land on the second line, not the first"
        )
    }

    /// In an empty editor UIKit's caret is deliberately parked ahead of the
    /// block placeholder so autocapitalization engages. The engine's caret sits
    /// after it, and the engine's is the one commands must be positioned from —
    /// otherwise Return splits before the block and the caret is left on the
    /// first line, and backspace computes a range over structure instead of
    /// text.
    func testEmptyBlockCaretReportsTheEnginePositionDespiteTheUIKitNudge() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "")
        // Drive the real selection path, which applies the nudge.
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        XCTAssertEqual(
            textView.selectedRange,
            NSRange(location: 0, length: 0),
            "precondition: UIKit's caret is nudged ahead of the placeholder"
        )
        XCTAssertEqual(
            textView.currentLogicalScalarSelection()?.head,
            1,
            "commands must be positioned from the engine's caret, which sits "
                + "after the empty block placeholder"
        )
    }

    /// Backspacing an empty bullet must remove it.
    ///
    /// There is nothing to walk back over: the item renders only its block
    /// placeholder, so a range computed from the caret spans list structure and
    /// deletes nothing. The keystroke has to reach the engine's backspace
    /// planner, which knows to leave the list.
    func testBackspaceInAnEmptyListItemLeavesTheList() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<ul><li><p></p></li></ul>")
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected an adapter for the bound editor")
            return
        }
        let documentBefore = editorV2GetDocumentJson(editorId: adapter.editorId).value ?? ""
        XCTAssertTrue(
            documentBefore.contains("bullet_list"),
            "precondition: the document holds an empty bullet, got \(documentBefore)"
        )
        textView.deleteBackward()

        let documentAfter = editorV2GetDocumentJson(editorId: adapter.editorId).value ?? ""
        XCTAssertFalse(
            documentAfter.contains("bullet_list"),
            "backspace in an empty bullet must leave the list, got \(documentAfter)"
        )
    }

    /// The whole point of the nudge fix: Return in an empty editor has to split
    /// inside the block, leaving the caret on the new line.
    func testReturnAfterTheEmptyBlockNudgeLeavesTheCaretOnTheSecondLine() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "")
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        textView.insertText("\n")
        textView.insertText("x")

        XCTAssertEqual(
            textView.textStorage.string,
            "\u{200B}\nx",
            "Return must split inside the empty block so the next character "
                + "lands on the second line"
        )
    }

    func testParagraphSplitAppliesTopLevelRenderPatch() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")

        let betaRange = (textView.text as NSString).range(of: "Beta")
        XCTAssertNotEqual(betaRange.location, NSNotFound)
        let splitOffset = UInt32(betaRange.location + betaRange.length)
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: splitOffset, scalarHead: splitOffset)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        XCTAssertEqual(textView.currentLogicalScalarSelection()?.head, splitOffset)

        textView.insertText("\n")

        XCTAssertTrue(
            textView.lastRenderAppliedPatch(),
            "splitting a middle paragraph should use the native top-level patch path"
        )
        XCTAssertEqual(
            textView.textStorage.string,
            "Alpha\nBeta\n\u{200B}\nGamma",
            "split patches must replace the full structural block region so the new paragraph separator renders correctly"
        )
        let selectedOffset = textView.offset(
            from: textView.beginningOfDocument,
            to: textView.selectedTextRange?.start ?? textView.endOfDocument
        )
        let gammaRange = (textView.text as NSString).range(of: "Gamma")
        XCTAssertGreaterThanOrEqual(
            selectedOffset,
            betaRange.location + betaRange.length + 1,
            "after splitting at the end of a paragraph, the caret should land inside the inserted empty paragraph"
        )
        XCTAssertLessThan(
            selectedOffset,
            gammaRange.location,
            "after splitting at the end of a paragraph, the caret must stay before the following paragraph"
        )
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>Alpha</p><p>Beta</p><p></p><p>Gamma</p>"
        )
    }

    func testSequentialParagraphSplitsKeepUsingTopLevelRenderPatch() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 180))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")

        let betaRange = (textView.text as NSString).range(of: "Beta")
        XCTAssertNotEqual(betaRange.location, NSNotFound)
        let firstSplitOffset = UInt32(betaRange.location + betaRange.length)
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: firstSplitOffset, scalarHead: firstSplitOffset)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.insertText("\n")

        XCTAssertTrue(textView.lastRenderAppliedPatch())

        let gammaRange = (textView.text as NSString).range(of: "Gamma")
        XCTAssertNotEqual(gammaRange.location, NSNotFound)
        let secondSplitOffset = UInt32(gammaRange.location + gammaRange.length)
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: secondSplitOffset, scalarHead: secondSplitOffset)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.insertText("\n")

        XCTAssertTrue(
            textView.lastRenderAppliedPatch(),
            "top-level metadata cache should remain valid across consecutive structural edits"
        )
        XCTAssertEqual(
            textView.textStorage.string,
            "Alpha\nBeta\n\u{200B}\nGamma\n\u{200B}"
        )
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>Alpha</p><p>Beta</p><p></p><p>Gamma</p><p></p>"
        )
    }

    func testFullAtomicRenderRefreshesShiftedImageDocPosBeforeResizeAction() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: """
        <p>A</p><img src="https://example.com/target.png" width="140" height="80"><img src="https://example.com/control.png" width="90" height="60"><p></p>
        """)
        view.layoutIfNeeded()

        guard let originalTargetRange = firstImageRange(in: view.textView),
              let originalDocPos = (view.textView.textStorage.attributes(
                  at: originalTargetRange.location,
                  effectiveRange: nil
              )[RenderBridgeAttributes.docPos] as? NSNumber)?.uint32Value
        else {
            XCTFail("expected the target image and its document position")
            return
        }

        let fullAtomicRender = EditorV2Shadow.replaceHtml(
            id: editorId,
            html: """
            <p>Preceding text is now longer</p><img src="https://example.com/target.png" width="140" height="80"><img src="https://example.com/control.png" width="90" height="60"><p></p>
            """
        )
        view.textView.applyUpdateJSON(fullAtomicRender, notifyDelegate: false)
        view.layoutIfNeeded()

        guard let targetRange = firstImageRange(in: view.textView),
              let refreshedDocPos = (view.textView.textStorage.attributes(
                  at: targetRange.location,
                  effectiveRange: nil
              )[RenderBridgeAttributes.docPos] as? NSNumber)?.uint32Value
        else {
            XCTFail("expected the target image after the atomic render")
            return
        }

        XCTAssertNotEqual(
            refreshedDocPos,
            originalDocPos,
            "a preceding extent change must refresh retained atom document positions"
        )

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setSelection(in: view.textView, utf16Range: targetRange)
        flushMainQueue()
        view.layoutIfNeeded()
        view.resizeSelectedImageForTesting(width: 200, height: 100)
        flushMainQueue()

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(
            html.contains("target.png\" width=\"200\""),
            "the selected target image must receive the resize action, got: \(html)"
        )
        XCTAssertTrue(
            html.contains("control.png\" width=\"90\""),
            "resizing the target must not affect the adjacent control image, got: \(html)"
        )
    }

    func testPrependingTopLevelChildRefreshesRetainedChildIndexes() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<hr><hr>")

        let fullAtomicRender = EditorV2Shadow.replaceHtml(
            id: editorId,
            html: "<p>Prelude</p><hr><hr>"
        )
        textView.applyUpdateJSON(fullAtomicRender, notifyDelegate: false)

        let horizontalRuleRanges = (0..<textView.textStorage.length).compactMap { index -> NSRange? in
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            return (attrs[RenderBridgeAttributes.voidNodeType] as? String)
                .map(EditorNodeTypes.isHorizontalRule) == true
                ? NSRange(location: index, length: 1)
                : nil
        }
        XCTAssertEqual(horizontalRuleRanges.count, 2)
        guard horizontalRuleRanges.count == 2 else { return }
        XCTAssertEqual(
            (textView.textStorage.attributes(
                at: horizontalRuleRanges[0].location,
                effectiveRange: nil
            )[RenderBridgeAttributes.topLevelChildIndex] as? NSNumber)?.intValue,
            1,
            "the first retained atom must receive its shifted top-level child index"
        )
        XCTAssertEqual(
            (textView.textStorage.attributes(
                at: horizontalRuleRanges[1].location,
                effectiveRange: nil
            )[RenderBridgeAttributes.topLevelChildIndex] as? NSNumber)?.intValue,
            2,
            "every retained sibling after a prepend must receive its shifted index"
        )
    }

    func testExplicitPrependRenderPatchRefreshesRetainedAtomMetadata() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(id: editorId, initialHTML: "<hr><hr>")

        let finalUpdateJSON = EditorV2Shadow.replaceHtml(
            id: editorId,
            html: "<p>Prelude</p><hr><hr>"
        )
        var explicitPatchUpdate = parseJSONObject(finalUpdateJSON)
        let finalRenderBlocks = try XCTUnwrap(
            (explicitPatchUpdate["renderPatch"] as? [String: Any])?["renderBlocks"]
                as? [[[String: Any]]]
        )
        explicitPatchUpdate["renderBlocks"] = finalRenderBlocks
        explicitPatchUpdate["renderPatch"] = [
            "startIndex": 0,
            "deleteCount": 0,
            "renderBlocks": [finalRenderBlocks[0]],
        ]
        let explicitPatchData = try JSONSerialization.data(withJSONObject: explicitPatchUpdate)
        let explicitPatchJSON = try XCTUnwrap(String(data: explicitPatchData, encoding: .utf8))

        textView.applyUpdateJSON(explicitPatchJSON, notifyDelegate: false)

        let expected = RenderBridge.renderBlocks(
            fromArray: finalRenderBlocks,
            baseFont: textView.baseFont,
            textColor: textView.baseTextColor
        )
        let trace = try XCTUnwrap(textView.lastApplyUpdateTrace())
        XCTAssertTrue(
            trace.attemptedPatch,
            "the update must exercise the explicit renderPatch path"
        )
        XCTAssertTrue(
            trace.usedPatch,
            "the explicit prepend must retain the compact renderPatch path"
        )
        XCTAssertEqual(textView.textStorage.string, expected.string, "the prepend must not duplicate content")

        let actualAtomOffsets = (0..<textView.textStorage.length).filter { index in
            textView.textStorage.attributes(at: index, effectiveRange: nil)[RenderBridgeAttributes.voidNodeType] != nil
        }
        let expectedAtomOffsets = (0..<expected.length).filter { index in
            expected.attributes(at: index, effectiveRange: nil)[RenderBridgeAttributes.voidNodeType] != nil
        }
        XCTAssertEqual(actualAtomOffsets.count, 2)
        XCTAssertEqual(actualAtomOffsets.count, expectedAtomOffsets.count)

        for (actualOffset, expectedOffset) in zip(actualAtomOffsets, expectedAtomOffsets) {
            let actualAttributes = textView.textStorage.attributes(at: actualOffset, effectiveRange: nil)
            let expectedAttributes = expected.attributes(at: expectedOffset, effectiveRange: nil)
            XCTAssertEqual(
                (actualAttributes[RenderBridgeAttributes.topLevelChildIndex] as? NSNumber)?.intValue,
                (expectedAttributes[RenderBridgeAttributes.topLevelChildIndex] as? NSNumber)?.intValue
            )
            XCTAssertEqual(
                (actualAttributes[RenderBridgeAttributes.docPos] as? NSNumber)?.uint32Value,
                (expectedAttributes[RenderBridgeAttributes.docPos] as? NSNumber)?.uint32Value
            )
        }
    }

    func testWrongRenderPatchBaseRecoversAFullNativeSnapshot() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Alpha</p>")
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p>")
        let revision = adapter.baseDocumentRevision
        let wrongBase = revision == 0 ? UInt64(1) : UInt64(0)
        let stalePatch: [String: Any] = [
            "documentVersion": String(revision),
            "renderPatch": [
                "baseDocumentVersion": String(wrongBase),
                "startIndex": 0,
                "deleteCount": 1,
                "renderBlocks": [[
                    ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
                    ["type": "textRun", "text": "Corrupt", "marks": []],
                    ["type": "blockEnd"],
                ]],
            ],
        ]
        let staleData = try JSONSerialization.data(withJSONObject: stalePatch)
        let staleJSON = try XCTUnwrap(String(data: staleData, encoding: .utf8))

        XCTAssertTrue(textView.applyUpdateJSON(staleJSON, notifyDelegate: false))
        XCTAssertEqual(textView.textStorage.string, "Alpha")
        XCTAssertFalse(textView.lastRenderAppliedPatch())

        let nextUpdate = EditorV2Shadow.insertTextScalar(
            id: editorId,
            scalarPos: 5,
            text: "!"
        )
        XCTAssertTrue(textView.applyUpdateJSON(nextUpdate, notifyDelegate: false))
        XCTAssertEqual(textView.textStorage.string, "Alpha!")
    }

    func testTypingInsideListItemFallsBackToFullRenderAndPreservesTextOrder() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(
            id: editorId,
            initialHTML: "<ul><li><p>Alpha</p></li><li><p>Beta</p></li></ul>"
        )

        let alphaRange = (textView.text as NSString).range(of: "Alpha")
        XCTAssertNotEqual(alphaRange.location, NSNotFound)
        setCollapsedSelection(in: textView, utf16Offset: alphaRange.location + alphaRange.length)
        flushMainQueue()

        textView.insertText("!")

        XCTAssertFalse(
            textView.lastRenderAppliedPatch(),
            "list items should bypass the top-level render patch path until list marker patching is made safe"
        )
        XCTAssertEqual(textView.textStorage.string, "Alpha!\nBeta")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>Alpha!</p></li><li><p>Beta</p></li></ul>"
        )
    }

    func testReturnInsideListItemFallsBackToFullRenderAndKeepsTypingInNewItem() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 180))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(
            id: editorId,
            initialHTML: "<ul><li><p>Alpha</p></li><li><p>Beta</p></li></ul>"
        )

        let alphaRange = (textView.text as NSString).range(of: "Alpha")
        XCTAssertNotEqual(alphaRange.location, NSNotFound)
        setCollapsedSelection(in: textView, utf16Offset: alphaRange.location + alphaRange.length)
        flushMainQueue()

        textView.insertText("\n")

        XCTAssertFalse(
            textView.lastRenderAppliedPatch(),
            "splitting list items should use the full render path to keep caret mapping stable"
        )
        textView.insertText("B")

        XCTAssertEqual(textView.textStorage.string, "Alpha\nB\nBeta")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>Alpha</p></li><li><p>B</p></li><li><p>Beta</p></li></ul>"
        )
    }

    func testFullCurrentStateLocalEditUsesSynthesizedTopLevelPatch() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")

        let updatedDocument = """
        {
          "type": "doc",
          "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "Alpha"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "Better"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "Gamma"}]}
          ]
        }
        """
        let update = EditorV2Shadow.setJson(id: editorId, json: updatedDocument)
        textView.applyUpdateJSON(update, notifyDelegate: false)

        XCTAssertTrue(
            textView.lastRenderAppliedPatch(),
            "full current-state updates should synthesize a top-level patch when only a local block range changes"
        )
        XCTAssertEqual(textView.textStorage.string, "Alpha\nBetter\nGamma")
    }

    func testIdenticalFullCurrentStateSkipsNativeTextReapply() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")
        textView.captureApplyUpdateTraceForTesting = true

        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        let trace = try XCTUnwrap(textView.lastApplyUpdateTrace())
        XCTAssertFalse(textView.lastRenderAppliedPatch())
        XCTAssertEqual(trace.buildRenderNanos, 0)
        XCTAssertEqual(trace.applyRenderNanos, 0)
        XCTAssertEqual(trace.applyRenderTextMutationNanos, 0)
        XCTAssertEqual(textView.textStorage.string, "Alpha\nBeta\nGamma")
    }

    func testRustDrivenSelectionApplyDoesNotNotifySelectionDelegate() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let delegate = EditorTextViewDelegateSpy()
        textView.editorDelegate = delegate
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p>")
        delegate.selectionChanges.removeAll()
        delegate.receivedUpdates.removeAll()

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 8, scalarHead: 8)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        flushMainQueue()

        XCTAssertEqual(delegate.selectionChanges.count, 0)
        XCTAssertEqual(delegate.receivedUpdates.count, 0)
    }

    func testEditorThemeContentInsetsApplyToTextView() {
        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        let defaultInset = view.textView.textContainerInset
        let theme = EditorTheme(dictionary: [
            "contentInsets": [
                "top": 12,
                "right": 16,
                "bottom": 20,
                "left": 24,
            ],
        ])

        view.applyTheme(theme)

        XCTAssertEqual(view.textView.textContainerInset.top, 12, accuracy: 0.1)
        XCTAssertEqual(view.textView.textContainerInset.left, 24, accuracy: 0.1)
        XCTAssertEqual(view.textView.textContainerInset.bottom, 20, accuracy: 0.1)
        XCTAssertEqual(view.textView.textContainerInset.right, 16, accuracy: 0.1)

        view.applyTheme(nil)

        XCTAssertEqual(view.textView.textContainerInset.top, defaultInset.top, accuracy: 0.1)
        XCTAssertEqual(view.textView.textContainerInset.left, defaultInset.left, accuracy: 0.1)
        XCTAssertEqual(view.textView.textContainerInset.bottom, defaultInset.bottom, accuracy: 0.1)
        XCTAssertEqual(view.textView.textContainerInset.right, defaultInset.right, accuracy: 0.1)
    }

    func testEditorThemeZeroContentInsetsRemoveLeadingTextGutter() {
        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        view.textView.placeholder = "Type here"
        view.textView.applyRenderJSON("""
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"\\u200B","marks":[]},
          {"type":"blockEnd"}
        ]
        """)

        view.applyTheme(EditorTheme(dictionary: [
            "contentInsets": [
                "top": 0,
                "right": 0,
                "bottom": 0,
                "left": 0,
            ],
        ]))
        view.layoutIfNeeded()
        view.textView.layoutIfNeeded()

        XCTAssertEqual(view.textView.textContainer.lineFragmentPadding, 0, accuracy: 0.1)
        XCTAssertEqual(view.textView.placeholderFrameForTesting().minX, 0, accuracy: 0.1)
    }

    func testEditorThemeBorderRadiusAppliesToEditorContainer() {
        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        let theme = EditorTheme(dictionary: [
            "backgroundColor": "#d7e4ff",
            "borderRadius": 18,
        ])

        view.applyTheme(theme)

        XCTAssertEqual(view.layer.cornerRadius, 18, accuracy: 0.1)
        XCTAssertTrue(view.clipsToBounds)

        view.applyTheme(nil)

        XCTAssertEqual(view.layer.cornerRadius, 0, accuracy: 0.1)
        XCTAssertFalse(view.clipsToBounds)
    }

    func testRemoteSelectionOverlayShowsFocusedCaretWithoutBadge() {
        let editorId = makeV2Editor(
            configJson: #"{"initialization":{"type":"localEmpty"},"policy":{"allowBase64Images":true}}"#
        )
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        view.layoutIfNeeded()

        let docPos = EditorV2Shadow.scalarToDoc(id: editorId, scalar: 6)
        view.setRemoteSelections([
            RemoteSelectionDecoration(
                clientId: "7",
                anchor: docPos,
                head: docPos,
                color: .systemOrange,
                name: "Alice",
                isFocused: true
            ),
        ])
        view.layoutIfNeeded()

        let overlaySubviews = view.remoteSelectionOverlaySubviewsForTesting()
        let labels = overlaySubviews.compactMap { $0 as? UILabel }
        let nonLabels = overlaySubviews.filter { !($0 is UILabel) }
        let caretViews = nonLabels.filter { $0.bounds.height > 0 && $0.bounds.width > 0 }

        XCTAssertTrue(labels.isEmpty)
        XCTAssertEqual(nonLabels.count, 1, "expected one caret view for a collapsed focused remote selection")
        XCTAssertEqual(caretViews.count, 1, "expected the collapsed remote caret view to have a visible frame")
    }

    func testRemoteSelectionOverlayShowsFocusedCaretAtEndOfDocument() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        view.layoutIfNeeded()

        let endDocPos = EditorV2Shadow.scalarToDoc(id: editorId, scalar: 11)
        view.setRemoteSelections([
            RemoteSelectionDecoration(
                clientId: "9",
                anchor: endDocPos,
                head: endDocPos,
                color: .systemGreen,
                name: "Bob",
                isFocused: true
            ),
        ])
        view.layoutIfNeeded()

        let caretViews = view.remoteSelectionOverlaySubviewsForTesting()
            .filter { !($0 is UILabel) && $0.bounds.height > 0 && $0.bounds.width > 0 }
        XCTAssertEqual(caretViews.count, 1, "expected a visible caret view at the end of the document")
    }

    func testRemoteSelectionOverlayUsesCorrectWrappedVisualLine() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 140, height: 220))
        view.editorId = editorId
        view.setContent(html: "<p>Hello world from remote carets</p>")
        view.layoutIfNeeded()

        let targetScalar: UInt32 = 15
        let expectedCaretRect = view.textView.convert(
            view.textView.caretRect(
                for: PositionBridge.scalarToTextView(targetScalar, in: view.textView)
            ),
            to: view
        )
        XCTAssertGreaterThan(expectedCaretRect.minY, 0, "expected the target caret to be on a wrapped visual line")

        let docPos = EditorV2Shadow.scalarToDoc(id: editorId, scalar: targetScalar)
        view.setRemoteSelections([
            RemoteSelectionDecoration(
                clientId: "10",
                anchor: docPos,
                head: docPos,
                color: .systemPurple,
                name: "Wrapped",
                isFocused: true
            ),
        ])
        view.layoutIfNeeded()

        let caretView = view.remoteSelectionOverlaySubviewsForTesting()
            .first { !($0 is UILabel) && $0.bounds.height > 0 && $0.bounds.width > 0 }
        XCTAssertNotNil(caretView)
        XCTAssertEqual(caretView?.frame.minY ?? 0, round(expectedCaretRect.minY), accuracy: 1)
    }

    func testRemoteSelectionOverlayHidesCaretAndBadgeForUnfocusedCollapsedSelection() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        view.layoutIfNeeded()

        let docPos = EditorV2Shadow.scalarToDoc(id: editorId, scalar: 6)
        view.setRemoteSelections([
            RemoteSelectionDecoration(
                clientId: "8",
                anchor: docPos,
                head: docPos,
                color: .systemBlue,
                name: "Alice",
                isFocused: false
            ),
        ])
        view.layoutIfNeeded()

        XCTAssertTrue(view.remoteSelectionOverlaySubviewsForTesting().isEmpty)
    }

    func testAccessoryToolbarSwitchesToMentionSuggestionMode() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let baseHeight = toolbar.intrinsicContentSize.height

        toolbar.apply(mentionTheme: EditorMentionTheme(dictionary: [
            "suggestions": [
                "option": [
                    "backgroundColor": "#d7e4ff",
                    "textColor": "#1a2c48",
                ],
            ],
        ]))

        let didChange = toolbar.setMentionSuggestions([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "alice",
                "attrs": ["label": "alice"],
            ])!,
            NativeMentionSuggestion(dictionary: [
                "key": "ben",
                "title": "Ben Ortiz",
                "subtitle": "Engineering",
                "label": "ben",
                "attrs": ["label": "ben"],
            ])!,
        ], trigger: "@")

        XCTAssertTrue(didChange)
        XCTAssertEqual(toolbar.intrinsicContentSize.height, baseHeight + 2)
        XCTAssertTrue(toolbar.isShowingMentionSuggestions)
        XCTAssertEqual(toolbar.mentionButtonAtForTesting(0)?.titleTextForTesting(), "@alice")
    }

    func testAccessoryToolbarKeepsRetainedMentionButtonsMountedWhileQueryNarrows() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let alice = NativeMentionSuggestion(dictionary: [
            "key": "alice",
            "title": "Alice Chen",
            "subtitle": "Design",
            "label": "alice",
            "attrs": ["label": "alice"],
        ])!
        let ben = NativeMentionSuggestion(dictionary: [
            "key": "ben",
            "title": "Ben Ortiz",
            "subtitle": "Engineering",
            "label": "ben",
            "attrs": ["label": "ben"],
        ])!

        _ = toolbar.setMentionSuggestions([alice, ben], trigger: "@")
        let retainedButton = toolbar.mentionButtonAtForTesting(0)

        _ = toolbar.setMentionSuggestions([alice], trigger: "@")

        XCTAssertTrue(toolbar.mentionButtonAtForTesting(0) === retainedButton)
    }

    func testNativeEditorUsesZeroHeightAccessoryPlaceholderWhenToolbarIsInline() {
        let view = NativeEditorExpoView()

        view.setToolbarPlacement("inline")

        XCTAssertTrue(view.isUsingAccessoryPlaceholderForTesting())
        XCTAssertFalse(view.isUsingAccessoryToolbarForTesting())
        XCTAssertNotNil(view.inputAccessoryViewForTesting())
        XCTAssertEqual(view.inputAccessoryViewForTesting()?.intrinsicContentSize.height ?? -1, 0)
    }

    func testNativeEditorRestoresToolbarAccessoryWhenSwitchingBackToKeyboardPlacement() {
        let view = NativeEditorExpoView()

        view.setToolbarPlacement("inline")
        XCTAssertTrue(view.isUsingAccessoryPlaceholderForTesting())

        view.setToolbarPlacement("keyboard")

        XCTAssertTrue(view.isUsingAccessoryToolbarForTesting())
        XCTAssertFalse(view.isUsingAccessoryPlaceholderForTesting())
    }

    func testNativeEditorRemovesAccessoryPlaceholderWhenNotEditable() {
        let view = NativeEditorExpoView()

        view.setToolbarPlacement("inline")
        view.setEditable(false)

        XCTAssertNil(view.inputAccessoryViewForTesting())
    }

    func testNativeEditorToolbarFrameTapPreservesNextBlurOnce() {
        let view = NativeEditorExpoView()
        view.setToolbarFrameJson(#"{"x":20,"y":40,"width":100,"height":32}"#)

        XCTAssertFalse(view.shouldPreserveFocusAfterToolbarTouchForTesting())
        XCTAssertFalse(
            view.prepareOutsideTapForFocusHandlingForTesting(
                locationInWindow: CGPoint(x: 30, y: 50)
            )
        )
        XCTAssertTrue(view.shouldPreserveFocusAfterToolbarTouchForTesting())
        XCTAssertTrue(view.consumeToolbarFocusPreservationForTesting())
        XCTAssertFalse(view.shouldPreserveFocusAfterToolbarTouchForTesting())
        XCTAssertFalse(view.consumeToolbarFocusPreservationForTesting())
    }

    func testNativeEditorOutsideTapClearsToolbarPreservation() {
        let view = NativeEditorExpoView()

        view.markRecentToolbarTouchForTesting()
        XCTAssertTrue(view.shouldPreserveFocusAfterToolbarTouchForTesting())

        XCTAssertTrue(
            view.prepareOutsideTapForFocusHandlingForTesting(
                locationInWindow: CGPoint(x: 240, y: 260)
            )
        )
        XCTAssertFalse(view.shouldPreserveFocusAfterToolbarTouchForTesting())
    }

    func testInlineAccessoryPlaceholderRemainsAttachedAfterNativeEdit() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello</p>")
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 5, scalarHead: 5)

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        view.setToolbarPlacement("inline")
        view.richTextView.textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        view.richTextView.textView.insertText("!")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello!</p>")
        XCTAssertTrue(view.isUsingAccessoryPlaceholderForTesting())
    }

    func testToolbarThemeParsesNativeAppearance() {
        let theme = EditorTheme(dictionary: [
            "toolbar": [
                "appearance": "native",
                "height": 44,
            ],
        ])

        XCTAssertEqual(theme.toolbar?.appearance, .native)
        XCTAssertEqual(theme.toolbar?.height ?? 0, 44, accuracy: 0.1)
        XCTAssertEqual(theme.toolbar?.resolvedKeyboardOffset ?? 0, 6, accuracy: 0.1)
        XCTAssertEqual(theme.toolbar?.resolvedHorizontalInset ?? -1, 10, accuracy: 0.1)
        XCTAssertEqual(theme.toolbar?.resolvedBorderRadius ?? -1, 20, accuracy: 0.1)
    }

    func testToolbarThemeHonorsExplicitInsetAndBorderRadius() {
        let theme = EditorTheme(dictionary: [
            "toolbar": [
                "appearance": "native",
                "horizontalInset": 10,
                "borderRadius": 22,
            ],
        ])

        XCTAssertEqual(theme.toolbar?.resolvedHorizontalInset ?? -1, 10, accuracy: 0.1)
        XCTAssertEqual(theme.toolbar?.resolvedBorderRadius ?? -1, 22, accuracy: 0.1)
    }

    func testAccessoryToolbarAppliesNativeAppearanceChrome() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
            "height": 44,
        ]))
        XCTAssertTrue(toolbar.usesNativeAppearanceForTesting)
        if #available(iOS 26.0, *) {
#if compiler(>=6.2)
            XCTAssertTrue(toolbar.usesUIGlassEffectForTesting)
#else
            XCTAssertFalse(toolbar.usesUIGlassEffectForTesting)
#endif
            XCTAssertEqual(toolbar.chromeBorderWidthForTesting, 1 / UIScreen.main.scale, accuracy: 0.1)
        } else {
            XCTAssertEqual(toolbar.chromeBorderWidthForTesting, 1 / UIScreen.main.scale, accuracy: 0.1)
        }
        XCTAssertEqual(toolbar.intrinsicContentSize.height, 50, accuracy: 0.1)
    }

    func testAccessoryToolbarAppliesSelectedStateForActiveNativeButton() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.applyBoldStateForTesting(active: true, enabled: true)

        XCTAssertEqual(toolbar.selectedButtonCountForTesting, 1)
    }

    func testDefaultAccessoryToolbarUsesProseMirrorNodeNames() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.applyStateJSONForTesting("""
        {
          "activeState": {
            "marks": {},
            "nodes": { "bullet_list": true, "list_item": true },
            "commands": { "wrapBulletList": true, "wrapOrderedList": true },
            "allowedMarks": [],
            "insertableNodes": ["hard_break", "horizontal_rule"]
          },
          "historyState": { "canUndo": false, "canRedo": false }
        }
        """)

        XCTAssertEqual(toolbar.buttonLabelForTesting(5), "Bullet List")
        XCTAssertEqual(toolbar.selectedButtonCountForTesting, 1)
        XCTAssertEqual(toolbar.buttonIsEnabledForTesting(9), true)
        XCTAssertEqual(toolbar.buttonIsEnabledForTesting(10), true)
    }

    func testNativeToolbarCascadesGlobalAndPerButtonStyles() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "action",
            "key": "global-idle",
            "label": "Global Idle",
            "icon": { "type": "glyph", "text": "G" }
          },
          {
            "type": "action",
            "key": "idle",
            "label": "Idle",
            "icon": { "type": "glyph", "text": "I" },
            "buttonStyle": { "backgroundColor": "#121212" }
          },
          {
            "type": "action",
            "key": "global-disabled",
            "label": "Global Disabled",
            "icon": { "type": "glyph", "text": "E" },
            "isActive": true,
            "isDisabled": true
          },
          {
            "type": "action",
            "key": "disabled",
            "label": "Disabled",
            "icon": { "type": "glyph", "text": "D" },
            "isActive": true,
            "isDisabled": true,
            "buttonStyle": {
              "disabledColor": "#444444",
              "disabledBackgroundColor": "#555555"
            }
          },
          {
            "type": "action",
            "key": "global-active",
            "label": "Global Active",
            "icon": { "type": "glyph", "text": "T" },
            "isActive": true
          },
          {
            "type": "action",
            "key": "active",
            "label": "Active",
            "icon": { "type": "glyph", "text": "A" },
            "isActive": true,
            "buttonStyle": {
              "iconSize": 26,
              "activeColor": "#555555",
              "activeBackgroundColor": "#666666",
              "borderRadius": 12
            }
          }
        ]
        """)
        let theme = EditorToolbarTheme(dictionary: [
            "appearance": "native",
            "buttonIconSize": 18,
            "buttonColor": "#111111",
            "buttonBackgroundColor": "#050505",
            "buttonActiveColor": "#222222",
            "buttonDisabledColor": "#333333",
            "buttonActiveBackgroundColor": "#777777",
            "buttonDisabledBackgroundColor": "#888888",
            "buttonBorderRadius": 9,
        ])
        let buttonStyle = EditorToolbarButtonStyle(dictionary: [
            "backgroundColor": "#121212",
            "disabledBackgroundColor": "#555555",
        ])

        XCTAssertEqual(theme.buttonBackgroundColor, EditorTheme.color(from: "#050505"))
        XCTAssertEqual(theme.buttonDisabledBackgroundColor, EditorTheme.color(from: "#888888"))
        XCTAssertEqual(buttonStyle.backgroundColor, EditorTheme.color(from: "#121212"))
        XCTAssertEqual(buttonStyle.disabledBackgroundColor, EditorTheme.color(from: "#555555"))

        toolbar.apply(theme: theme)

        XCTAssertEqual(toolbar.buttonTintColorForTesting(0), EditorTheme.color(from: "#111111"))
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(0),
            EditorTheme.color(from: "#050505")
        )
        XCTAssertEqual(toolbar.buttonTintColorForTesting(1), EditorTheme.color(from: "#111111"))
        XCTAssertEqual(toolbar.buttonFontSizeForTesting(1) ?? -1, 18, accuracy: 0.1)
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(1),
            EditorTheme.color(from: "#121212")
        )
        XCTAssertEqual(toolbar.buttonCornerRadiusForTesting(1) ?? -1, 9, accuracy: 0.1)
        XCTAssertEqual(toolbar.buttonTintColorForTesting(2), EditorTheme.color(from: "#333333"))
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(2),
            EditorTheme.color(from: "#888888")
        )
        XCTAssertEqual(toolbar.buttonTintColorForTesting(3), EditorTheme.color(from: "#444444"))
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(3),
            EditorTheme.color(from: "#555555")
        )
        XCTAssertEqual(toolbar.buttonTintColorForTesting(4), EditorTheme.color(from: "#222222"))
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(4),
            EditorTheme.color(from: "#777777")
        )
        XCTAssertEqual(toolbar.buttonTintColorForTesting(5), EditorTheme.color(from: "#555555"))
        XCTAssertEqual(toolbar.buttonFontSizeForTesting(5) ?? -1, 26, accuracy: 0.1)
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(5),
            EditorTheme.color(from: "#666666")
        )
        XCTAssertEqual(toolbar.buttonCornerRadiusForTesting(5) ?? -1, 12, accuracy: 0.1)
    }

    /// A configured `UIButton` resolves its own selected-state background, so
    /// filling `backgroundColor` as well stacks two shapes into a double halo.
    func testActiveButtonPaintsExactlyOneBackground() {
        for appearance in ["native", "custom"] {
            let toolbar = EditorAccessoryToolbarView(frame: .zero)
            toolbar.apply(theme: EditorToolbarTheme(dictionary: [
                "appearance": appearance,
            ]))

            toolbar.applyBoldStateForTesting(active: true, enabled: true)
            XCTAssertEqual(
                toolbar.buttonBackgroundSourceCountForTesting(0),
                1,
                "an active \(appearance) button must paint one background, not stack two"
            )

            toolbar.applyBoldStateForTesting(active: false, enabled: true)
            XCTAssertEqual(
                toolbar.buttonBackgroundSourceCountForTesting(0),
                0,
                "an inactive \(appearance) button must paint no background at all"
            )
        }
    }

    func testAccessoryToolbarExpandsGroupedButtonsInline() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "group",
            "key": "headings",
            "label": "Headings",
            "icon": { "type": "glyph", "text": "H" },
            "presentation": "expand",
            "items": [
              {
                "type": "heading",
                "level": 1,
                "label": "Heading 1",
                "icon": { "type": "default", "id": "h1" }
              },
              {
                "type": "heading",
                "level": 2,
                "label": "Heading 2",
                "icon": { "type": "default", "id": "h2" }
              }
            ]
          }
        ]
        """)
        toolbar.applyStateJSONForTesting("""
        {
          "activeState": {
            "marks": {},
            "nodes": {},
            "commands": {
              "toggleHeading1": true,
              "toggleHeading2": true
            },
            "allowedMarks": [],
            "insertableNodes": []
          },
          "historyState": {
            "canUndo": false,
            "canRedo": false
          }
        }
        """)

        XCTAssertEqual(toolbar.buttonCountForTesting(), 1)

        toolbar.triggerButtonTapForTesting(0)

        XCTAssertEqual(toolbar.buttonCountForTesting(), 3)
        XCTAssertEqual(toolbar.buttonLabelForTesting(1), "Heading 1")
        XCTAssertEqual(toolbar.buttonLabelForTesting(2), "Heading 2")
    }

    func testAccessoryToolbarMenuGroupUsesEditMenuWithoutAttachingMenuToVisibleButton() {
        let toolbar = EditorAccessoryToolbarView(frame: CGRect(x: 0, y: 0, width: 320, height: 56))
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(toolbar)
        defer {
            toolbar.removeFromSuperview()
            window.isHidden = true
        }
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "group",
            "key": "headings",
            "label": "Headings",
            "icon": { "type": "glyph", "text": "H" },
            "presentation": "menu",
            "items": [
              {
                "type": "heading",
                "level": 1,
                "label": "Heading 1",
                "icon": { "type": "default", "id": "h1" }
              }
            ]
          },
          {
            "type": "group",
            "key": "insert",
            "label": "Insert",
            "icon": { "type": "glyph", "text": "+" },
            "presentation": "menu",
            "items": [
              {
                "type": "action",
                "key": "custom",
                "label": "Custom",
                "icon": { "type": "glyph", "text": "+" }
              }
            ]
          }
        ]
        """)
        toolbar.applyStateJSONForTesting("""
        {
          "activeState": {
            "marks": {},
            "nodes": { "h1": true },
            "commands": { "toggleHeading1": true },
            "allowedMarks": [],
            "insertableNodes": []
          },
          "historyState": {
            "canUndo": false,
            "canRedo": false
          }
        }
        """)
        toolbar.layoutIfNeeded()

        var descendants = toolbar.subviews
        var descendantIndex = 0
        while descendantIndex < descendants.count {
            descendants.append(contentsOf: descendants[descendantIndex].subviews)
            descendantIndex += 1
        }
        let visibleButton = descendants
            .compactMap { $0 as? UIButton }
            .first { $0.accessibilityLabel == "Headings" }

        XCTAssertNotNil(visibleButton)
        XCTAssertNil(visibleButton?.menu, "the visible parent button must not become UIKit's hidden menu source")
        XCTAssertEqual(visibleButton?.accessibilityHint, "Shows menu")

        guard let editMenuInteraction = toolbar.interactions.first(where: { $0 is UIEditMenuInteraction }) as? UIEditMenuInteraction else {
            return XCTFail("the toolbar should own the edit-menu presentation interaction")
        }
        defer {
            editMenuInteraction.dismissMenu()
            RunLoop.main.run(until: Date().addingTimeInterval(0.35))
        }
        toolbar.triggerButtonTapForTesting(0)
        RunLoop.main.run(until: Date().addingTimeInterval(0.35))
        let configuration = UIEditMenuConfiguration(identifier: nil, sourcePoint: .zero)
        let menu = editMenuInteraction.delegate?.editMenuInteraction?(
            editMenuInteraction,
            menuFor: configuration,
            suggestedActions: []
        )
        let headingAction = menu?.children.first as? UIAction

        XCTAssertEqual(toolbar.editMenuPresentationRequestCountForTesting, 1)
        XCTAssertEqual(menu?.title, "Headings")
        XCTAssertEqual(menu?.preferredElementSize, .large)
        XCTAssertEqual(headingAction?.title, "Heading 1")
        XCTAssertEqual(headingAction?.state, .on)
        XCTAssertFalse(headingAction?.attributes.contains(.disabled) ?? true)

        toolbar.triggerButtonTapForTesting(1)
        XCTAssertEqual(
            toolbar.editMenuPresentationRequestCountForTesting,
            2,
            "tapping a different source should immediately request its menu"
        )
        RunLoop.main.run(until: Date().addingTimeInterval(0.35))

        toolbar.triggerButtonTapForTesting(1)
        XCTAssertEqual(
            toolbar.editMenuPresentationRequestCountForTesting,
            2,
            "tapping the active source should dismiss without requesting another presentation"
        )
    }

    func testAccessoryToolbarGroupedChildrenCanOverrideParentPlacement() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "group",
            "key": "headings",
            "label": "Headings",
            "icon": { "type": "glyph", "text": "H" },
            "presentation": "expand",
            "placement": "start",
            "items": [
              {
                "type": "action",
                "key": "inherited",
                "label": "Inherited",
                "icon": { "type": "glyph", "text": "I" }
              },
              {
                "type": "action",
                "key": "pinned",
                "label": "Pinned",
                "icon": { "type": "glyph", "text": "P" },
                "placement": "end"
              }
            ]
          }
        ]
        """)

        XCTAssertEqual(toolbar.buttonLabelsForPlacementForTesting("start"), ["Headings"])
        XCTAssertEqual(toolbar.buttonLabelsForPlacementForTesting("end"), [])

        toolbar.triggerButtonTapForTesting(0)

        XCTAssertEqual(toolbar.buttonLabelsForPlacementForTesting("start"), ["Headings", "Inherited"])
        XCTAssertEqual(toolbar.buttonLabelsForPlacementForTesting("end"), ["Pinned"])
    }

    func testAccessoryToolbarEnablesListDepthCommandsForTaskLists() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "command",
            "command": "indentList",
            "label": "Indent",
            "icon": { "type": "default", "id": "indentList" }
          },
          {
            "type": "command",
            "command": "outdentList",
            "label": "Outdent",
            "icon": { "type": "default", "id": "outdentList" }
          }
        ]
        """)
        toolbar.applyStateJSONForTesting("""
        {
          "activeState": {
            "marks": {},
            "nodes": {
              "taskList": true,
              "taskItem": true
            },
            "commands": {
              "indentList": true,
              "outdentList": true
            },
            "allowedMarks": [],
            "insertableNodes": []
          },
          "historyState": {
            "canUndo": false,
            "canRedo": false
          }
        }
        """)

        XCTAssertEqual(toolbar.buttonIsEnabledForTesting(0), true)
        XCTAssertEqual(toolbar.buttonIsEnabledForTesting(1), true)
    }

    func testAccessoryToolbarGroupReflectsActiveChildState() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "group",
            "key": "headings",
            "label": "Headings",
            "icon": { "type": "glyph", "text": "H" },
            "items": [
              {
                "type": "heading",
                "level": 2,
                "label": "Heading 2",
                "icon": { "type": "default", "id": "h2" }
              }
            ]
          }
        ]
        """)
        toolbar.applyStateJSONForTesting("""
        {
          "activeState": {
            "marks": {},
            "nodes": {
              "h2": true
            },
            "commands": {
              "toggleHeading2": true
            },
            "allowedMarks": [],
            "insertableNodes": []
          },
          "historyState": {
            "canUndo": false,
            "canRedo": false
          }
        }
        """)

        XCTAssertEqual(toolbar.selectedButtonCountForTesting, 1)
    }

    func testAccessoryToolbarPreservesScrolledOffsetWhenExpandingGroupedButtons() {
        let toolbar = EditorAccessoryToolbarView(frame: CGRect(x: 0, y: 0, width: 180, height: 56))
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "action",
            "key": "bold",
            "label": "Bold",
            "icon": { "type": "default", "id": "bold" }
          },
          {
            "type": "action",
            "key": "italic",
            "label": "Italic",
            "icon": { "type": "default", "id": "italic" }
          },
          {
            "type": "action",
            "key": "underline",
            "label": "Underline",
            "icon": { "type": "default", "id": "underline" }
          },
          {
            "type": "group",
            "key": "headings",
            "label": "Headings",
            "icon": { "type": "glyph", "text": "H" },
            "presentation": "expand",
            "items": [
              {
                "type": "action",
                "key": "h1",
                "label": "Heading 1",
                "icon": { "type": "default", "id": "h1" }
              },
              {
                "type": "action",
                "key": "h2",
                "label": "Heading 2",
                "icon": { "type": "default", "id": "h2" }
              }
            ]
          },
          {
            "type": "action",
            "key": "undo",
            "label": "Undo",
            "icon": { "type": "default", "id": "undo" }
          },
          {
            "type": "action",
            "key": "redo",
            "label": "Redo",
            "icon": { "type": "default", "id": "redo" }
          }
        ]
        """)
        toolbar.layoutIfNeeded()

        let targetOffset = min(
            40,
            toolbar.nativeToolbarContentWidthForTesting - toolbar.nativeToolbarVisibleWidthForTesting
        )
        XCTAssertGreaterThan(targetOffset, 0)

        toolbar.setNativeToolbarContentOffsetXForTesting(targetOffset)
        toolbar.triggerButtonTapForTesting(3)
        toolbar.layoutIfNeeded()

        XCTAssertEqual(toolbar.nativeToolbarContentOffsetXForTesting, targetOffset, accuracy: 0.1)
    }

    func testAccessoryToolbarNativeDisabledButtonUsesAdaptiveTintInDarkHost() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.tintColor = .black

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.applyBoldStateForTesting(active: false, enabled: false)

        XCTAssertEqual(
            toolbar.firstButtonAlphaForTesting, 1.0, accuracy: 0.01,
            "Disabled native button must stay at full alpha because low alpha is invisible on dark blur backgrounds"
        )
        guard let tintColor = toolbar.firstButtonTintColorForTesting else {
            return XCTFail("Disabled native button should apply an explicit transparent tint")
        }
        XCTAssertEqual(tintColor.cgColor.alpha, 0.46, accuracy: 0.01)
        let darkTint = tintColor.resolvedColor(
            with: UITraitCollection(userInterfaceStyle: .dark)
        )
        var white: CGFloat = 0
        var alpha: CGFloat = 0
        XCTAssertTrue(darkTint.getWhite(&white, alpha: &alpha))
        XCTAssertGreaterThan(
            white, 0.9,
            "Disabled native button tint should adapt to a dark host instead of inheriting black"
        )
        XCTAssertEqual(alpha, 0.46, accuracy: 0.01)
        XCTAssertNotEqual(
            tintColor, .systemGray,
            "Disabled native button should use transparent foreground instead of fixed system gray"
        )
        XCTAssertEqual(toolbar.firstButtonTitleColorForTesting(.disabled), tintColor)
    }

    func testAccessoryToolbarNativeEnabledButtonInheritsSystemTintAtFullAlpha() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.applyBoldStateForTesting(active: false, enabled: true)

        XCTAssertEqual(
            toolbar.firstButtonAlphaForTesting, 1.0, accuracy: 0.01,
            "Enabled native button must be at full alpha"
        )
        XCTAssertNotEqual(
            toolbar.firstButtonTintColorForTesting, .systemGray,
            "Enabled native button must not use the disabled .systemGray tint"
        )
    }

    func testAccessoryToolbarAppliesNativeAppearanceToMentionSuggestions() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        _ = toolbar.setMentionSuggestions([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["label": "@alice"],
            ])!,
        ])

        XCTAssertTrue(toolbar.mentionButtonAtForTesting(0)?.usesNativeAppearanceForTesting() == true)
    }

    func testAccessoryToolbarNativeMentionSuggestionsUseNativeGlassTextRendering() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        _ = toolbar.setMentionSuggestions([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["label": "@alice"],
            ])!,
        ])

        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            XCTAssertTrue(
                toolbar.mentionButtonAtForTesting(0)?.usesNativeGlassTextRenderingForTesting() == true,
                "Native mention suggestions should let UIKit render adaptive glass text"
            )
            XCTAssertTrue(
                toolbar.mentionButtonAtForTesting(0)?.usesNativeGlassSemiboldTitleForTesting() == true,
                "Native mention suggestions should keep the mention label semibold in glass"
            )
        }
        #endif
    }

    func testAccessoryToolbarNativeMentionSuggestionsUseTransparentOuterChrome() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            XCTAssertFalse(toolbar.nativeChromeIsTransparentForTesting)

            _ = toolbar.setMentionSuggestions([
                NativeMentionSuggestion(dictionary: [
                    "key": "alice",
                    "title": "Alice Chen",
                    "subtitle": "Design",
                    "label": "@alice",
                    "attrs": ["label": "@alice"],
                ])!,
            ])

            XCTAssertTrue(
                toolbar.nativeChromeIsTransparentForTesting,
                "Native mention chips own the glass surface, so the surrounding toolbar chrome should be transparent"
            )

            _ = toolbar.setMentionSuggestions([])

            XCTAssertFalse(
                toolbar.nativeChromeIsTransparentForTesting,
                "The native toolbar chrome should return when mention suggestions are cleared"
            )
        }
        #endif
    }

    func testAccessoryToolbarNativeMentionChromeTransitionAnimatesWhenHosted() {
        #if compiler(>=6.2)
        guard #available(iOS 26.0, *) else {
            return
        }

        let animationsWereEnabled = UIView.areAnimationsEnabled
        UIView.setAnimationsEnabled(true)
        defer {
            UIView.setAnimationsEnabled(animationsWereEnabled)
        }

        let toolbar = EditorAccessoryToolbarView(frame: CGRect(x: 0, y: 0, width: 320, height: 56))
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(toolbar)
        toolbar.layoutIfNeeded()
        defer {
            toolbar.removeFromSuperview()
            window.isHidden = true
        }

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))

        _ = toolbar.setMentionSuggestions([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["label": "@alice"],
            ])!,
        ])

        XCTAssertTrue(toolbar.didAnimateChromeTransitionForTesting)
        XCTAssertFalse(
            toolbar.nativeChromeIsTransparentForTesting,
            "The outer chrome should fade out instead of disappearing immediately"
        )

        let expectation = expectation(description: "chrome transition completed")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
            expectation.fulfill()
        }
        wait(for: [expectation], timeout: 1.0)

        XCTAssertTrue(toolbar.nativeChromeIsTransparentForTesting)
        #endif
    }

    func testNativeMentionSuggestionFallbackTextTracksTintColor() {
        if #available(iOS 26.0, *) {
            return
        }

        let chip = MentionSuggestionChipButton(
            suggestion: NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["label": "@alice"],
            ])!,
            theme: nil,
            toolbarAppearance: .native
        )
        let tint = UIColor(red: 0.12, green: 0.34, blue: 0.56, alpha: 1)

        chip.tintColor = tint

        XCTAssertEqual(chip.titleTextColorForTesting(), tint)
        XCTAssertEqual(chip.subtitleTextColorForTesting(), tint.withAlphaComponent(0.72))
    }

    func testAccessoryToolbarNativeLayoutFittingPreservesVisibleHeight() {
        let toolbar = EditorAccessoryToolbarView(frame: CGRect(x: 0, y: 0, width: 320, height: 0))

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.layoutIfNeeded()

        let fittedSize = toolbar.systemLayoutSizeFitting(
            CGSize(width: 320, height: UIView.layoutFittingCompressedSize.height)
        )
        XCTAssertGreaterThanOrEqual(fittedSize.height, 50, "native accessory toolbar should not collapse")
    }

    func testAccessoryToolbarNativeLayoutAllowsHorizontalOverflowScrolling() {
        let toolbar = EditorAccessoryToolbarView(frame: CGRect(x: 0, y: 0, width: 180, height: 56))

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.layoutIfNeeded()

        XCTAssertGreaterThan(
            toolbar.nativeToolbarContentWidthForTesting,
            toolbar.nativeToolbarVisibleWidthForTesting,
            "native toolbar should overflow horizontally so all items remain reachable"
        )
        XCTAssertEqual(
            toolbar.nativeToolbarContentOffsetXForTesting,
            0,
            accuracy: 0.1,
            "native toolbar should start left-aligned"
        )
    }

    func testAccessoryToolbarNativeLayoutPreservesScrolledOffsetAcrossRelayout() {
        let toolbar = EditorAccessoryToolbarView(frame: CGRect(x: 0, y: 0, width: 180, height: 56))

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.layoutIfNeeded()

        let targetOffset = min(40, toolbar.nativeToolbarContentWidthForTesting - toolbar.nativeToolbarVisibleWidthForTesting)
        XCTAssertGreaterThan(targetOffset, 0)
        toolbar.setNativeToolbarContentOffsetXForTesting(targetOffset)
        toolbar.layoutIfNeeded()
        XCTAssertEqual(
            toolbar.nativeToolbarContentOffsetXForTesting,
            targetOffset,
            accuracy: 0.1,
            "native toolbar should not snap back after relayout"
        )
    }

    /// Attaches `toolbar` to a fixed-width host so Auto Layout has a real width to resolve
    /// `.fill`-distributed arranged subviews against. `EditorAccessoryToolbarView` sets
    /// `translatesAutoresizingMaskIntoConstraints = false` on itself with no width/height
    /// constraint of its own (it self-sizes as an input accessory view in production, where
    /// the keyboard/window supplies its width) — without a host, `layoutIfNeeded()` on the bare
    /// view collapses every flexible-width arranged subview to zero instead of distributing
    /// space, which would make a "does not overlap" assertion pass vacuously.
    private static func attachToFixedWidthHost(_ toolbar: EditorAccessoryToolbarView, width: CGFloat) -> UIView {
        let host = UIView(frame: CGRect(x: 0, y: 0, width: width, height: 100))
        host.addSubview(toolbar)
        NSLayoutConstraint.activate([
            toolbar.leadingAnchor.constraint(equalTo: host.leadingAnchor),
            toolbar.trailingAnchor.constraint(equalTo: host.trailingAnchor),
            toolbar.topAnchor.constraint(equalTo: host.topAnchor),
        ])
        return host
    }

    /// Every placement renders as a custom button, with scroll items in the
    /// scrolling middle and pinned items in the stacks on either side.
    func testNativeAppearanceRendersEveryPlacementAsCustomButtons() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.placementToolbarFixtureJSON)
        host.layoutIfNeeded()

        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("start"),
            ["Start"],
            "start-pinned items belong in the start pinned stack"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("end"),
            ["End"],
            "end-pinned items belong in the end pinned stack"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("scroll"),
            ["Scroll One", "Scroll Two"],
            "scroll-placement items belong in the scrolling middle"
        )
    }

    func testPinnedPlacementsPreserveOuterHorizontalInsets() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.placementToolbarFixtureJSON)
        host.layoutIfNeeded()

        func descendant(withLabel label: String, in view: UIView) -> UIView? {
            if view.accessibilityLabel == label {
                return view
            }
            return view.subviews.lazy.compactMap {
                descendant(withLabel: label, in: $0)
            }.first
        }

        guard let startButton = descendant(withLabel: "Start", in: toolbar),
              let endButton = descendant(withLabel: "End", in: toolbar),
              let startSection = startButton.superview,
              let endSection = endButton.superview
        else {
            XCTFail("expected both pinned toolbar buttons")
            return
        }

        XCTAssertEqual(
            startButton.frame.minX,
            startSection.bounds.minX + 12,
            accuracy: 0.1,
            "start-pinned items should include the standard 12-point leading inset"
        )
        XCTAssertEqual(
            endButton.frame.maxX,
            endSection.bounds.maxX - 12,
            accuracy: 0.1,
            "end-pinned items should include the standard 12-point trailing inset"
        )
    }

    /// `bodyStackView` lays the scrolling middle out between the two pinned
    /// stacks. Arranged subviews cannot overlap by construction, but the middle
    /// can still be starved to zero width if a pinned stack claims the row (see
    /// `updatePinnedStackParticipation`), so assert it actually claims width
    /// rather than merely avoiding overlap by being empty.
    func testContentStackClaimsMiddleSlotWithoutOverlappingPinnedStacks() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.placementToolbarFixtureJSON)
        host.setNeedsLayout()
        host.layoutIfNeeded()

        let contentFrame = toolbar.contentStackViewFrameForTesting
        let startFrame = toolbar.startPinnedStackViewFrameForTesting
        let endFrame = toolbar.endPinnedStackViewFrameForTesting

        XCTAssertGreaterThan(
            contentFrame.width,
            0,
            "the content column must claim the middle slot's width, not collapse to zero"
        )
        XCTAssertFalse(
            contentFrame.intersects(startFrame),
            "content stack frame \(contentFrame) must not overlap the start pinned stack frame \(startFrame)"
        )
        XCTAssertFalse(
            contentFrame.intersects(endFrame),
            "content stack frame \(contentFrame) must not overlap the end pinned stack frame \(endFrame)"
        )
    }

    /// The mention row and the button row share `contentStackView`, so showing
    /// suggestions swaps the middle slot's contents without disturbing either
    /// pinned stack or letting the middle overlap them.
    func testMentionSuggestionsSwapTheMiddleSlotWithoutDisturbingPinnedStacks() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.placementToolbarFixtureJSON)
        host.setNeedsLayout()
        host.layoutIfNeeded()

        let didChange = toolbar.setMentionSuggestions([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "alice",
                "attrs": ["label": "alice"],
            ])!,
        ], trigger: "@")
        host.setNeedsLayout()
        host.layoutIfNeeded()

        XCTAssertTrue(didChange, "setMentionSuggestions should report a mode change from empty to non-empty")
        XCTAssertEqual(
            toolbar.mentionButtonAtForTesting(0)?.titleTextForTesting(),
            "@alice",
            "the mention suggestion chip should render inside the content stack"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("start"),
            ["Start"],
            "start-pinned items should keep rendering while mention suggestions are shown"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("end"),
            ["End"],
            "end-pinned items should keep rendering while mention suggestions are shown"
        )

        let contentFrame = toolbar.contentStackViewFrameForTesting
        let startFrame = toolbar.startPinnedStackViewFrameForTesting
        let endFrame = toolbar.endPinnedStackViewFrameForTesting
        XCTAssertGreaterThan(
            contentFrame.width,
            0,
            "the content stack must claim the middle slot's width while showing mentions"
        )
        XCTAssertFalse(
            contentFrame.intersects(startFrame),
            "content stack frame \(contentFrame) must not overlap the start pinned stack frame \(startFrame) while mentions are shown"
        )
        XCTAssertFalse(
            contentFrame.intersects(endFrame),
            "content stack frame \(contentFrame) must not overlap the end pinned stack frame \(endFrame) while mentions are shown"
        )

        let didChangeBack = toolbar.setMentionSuggestions([], trigger: "@")
        host.setNeedsLayout()
        host.layoutIfNeeded()

        XCTAssertTrue(didChangeBack, "setMentionSuggestions should report a mode change back to empty")
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("scroll"),
            ["Scroll One", "Scroll Two"],
            "clearing mention suggestions should restore the scrolling button row"
        )
    }

    private static let placementToolbarFixtureJSON = """
    [
      {
        "type": "action",
        "key": "start-item",
        "label": "Start",
        "icon": { "type": "glyph", "text": "S" },
        "placement": "start"
      },
      {
        "type": "action",
        "key": "scroll-one",
        "label": "Scroll One",
        "icon": { "type": "glyph", "text": "1" }
      },
      {
        "type": "action",
        "key": "scroll-two",
        "label": "Scroll Two",
        "icon": { "type": "glyph", "text": "2" }
      },
      {
        "type": "action",
        "key": "end-item",
        "label": "End",
        "icon": { "type": "glyph", "text": "E" },
        "placement": "end"
      }
    ]
    """

    func testMentionSuggestionChipContentViewsAllowTouchPassthrough() {
        let chip = MentionSuggestionChipButton(
            suggestion: NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["label": "@alice"],
            ])!,
            theme: nil
        )
        chip.frame = CGRect(x: 0, y: 0, width: 160, height: 44)
        chip.layoutIfNeeded()

        XCTAssertTrue(
            chip.contentViewsAllowTouchPassthroughForTesting(),
            "mention chip content views should not intercept taps from the button"
        )
    }

    func testResolveMentionQueryStateTriggersAfterSentencePunctuation() {
        let state = resolveMentionQueryState(
            in: "Testing.@",
            cursorScalar: 9,
            trigger: "@",
            isCaretInsideMention: false
        )

        XCTAssertEqual(
            state,
            MentionQueryState(query: "", trigger: "@", anchor: 8, head: 9)
        )
    }

    func testResolveMentionQueryStateSupportsHyphenatedQueries() {
        let state = resolveMentionQueryState(
            in: "@apollo-team",
            cursorScalar: 12,
            trigger: "@",
            isCaretInsideMention: false
        )

        XCTAssertEqual(
            state,
            MentionQueryState(query: "apollo-team", trigger: "@", anchor: 0, head: 12)
        )
    }

    func testManualSelectionInMiddleOfWordSyncsInteriorCaretPositionToRust() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        guard
            let start = textView.position(from: textView.beginningOfDocument, offset: 2),
            let range = textView.textRange(from: start, to: start)
        else {
            XCTFail("expected interior caret position")
            return
        }

        textView.selectedTextRange = range
        flushMainQueue()

        let selection = currentSelection(in: editorId)
        let expectedDoc = EditorV2Shadow.scalarToDoc(id: editorId, scalar: 2)

        XCTAssertEqual(selection["type"] as? String, "text")
        XCTAssertEqual((selection["anchor"] as? NSNumber)?.uint32Value, expectedDoc)
        XCTAssertEqual((selection["head"] as? NSNumber)?.uint32Value, expectedDoc)
    }

    func testManualSelectionIntoListItemRefreshesSelectionDependentActiveState() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        textView.bindEditor(
            id: editorId,
            initialHTML: "<p>Alpha</p><ul><li><p>Beta</p></li></ul>"
        )

        let plainOffset = (textView.attributedText.string as NSString).range(of: "Alpha").location
        let listOffset = (textView.attributedText.string as NSString).range(of: "Beta").location
        XCTAssertNotEqual(plainOffset, NSNotFound)
        XCTAssertNotEqual(listOffset, NSNotFound)

        setCollapsedSelection(in: textView, utf16Offset: plainOffset + 2)
        flushMainQueue()
        XCTAssertTrue(
            activeState(in: editorId).insertableNodes.contains("horizontal_rule"),
            "horizontal rule should be insertable in a normal paragraph"
        )

        setCollapsedSelection(in: textView, utf16Offset: listOffset + 2)
        flushMainQueue()
        XCTAssertFalse(
            activeState(in: editorId).insertableNodes.contains("horizontal_rule"),
            "horizontal rule should be disabled in list items after a manual caret move"
        )
    }

    func testManualSelectionInMiddleOfWordPersistsAfterDeferredSelectionSync() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")

        setCollapsedSelection(in: textView, utf16Offset: 3)
        flushMainQueue()

        let actualOffset = textView.offset(
            from: textView.beginningOfDocument,
            to: textView.selectedTextRange?.start ?? textView.endOfDocument
        )
        XCTAssertEqual(
            actualOffset,
            3,
            "deferred selection sync should not snap the caret to a word boundary"
        )
    }

    func testManualSelectionAfterBlockquoteSyncsInteriorCaretPositionToRust() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(
            id: editorId,
            initialHTML: "<blockquote><p>Hello</p></blockquote><p>World</p>"
        )

        let secondParagraphOffset = (textView.attributedText.string as NSString).range(of: "World").location
        XCTAssertNotEqual(secondParagraphOffset, NSNotFound)

        setCollapsedSelection(in: textView, utf16Offset: secondParagraphOffset + 3)
        flushMainQueue()

        let selection = currentSelection(in: editorId)
        let expectedDoc = EditorV2Shadow.scalarToDoc(id: editorId, scalar: UInt32(secondParagraphOffset + 3))

        XCTAssertEqual(selection["type"] as? String, "text")
        XCTAssertEqual((selection["anchor"] as? NSNumber)?.uint32Value, expectedDoc)
        XCTAssertEqual((selection["head"] as? NSNumber)?.uint32Value, expectedDoc)
    }


    func expectedCaretRect(
        in textView: UITextView,
        offset: Int,
        referenceRect: CGRect,
        font: UIFont
    ) -> CGRect {
        let baselineY = resolvedBaselineY(
            in: textView,
            offset: offset,
            referenceRect: referenceRect
        )
        XCTAssertNotNil(baselineY)
        return EditorTextView.adjustedCaretRect(
            from: referenceRect,
            baselineY: baselineY ?? referenceRect.maxY,
            font: font,
            screenScale: 2
        )
    }

    private func resolvedBaselineY(
        in textView: UITextView,
        offset: Int,
        referenceRect: CGRect
    ) -> CGFloat? {
        guard textView.attributedText.length > 0 else { return nil }

        let clampedOffset = min(max(offset, 0), textView.attributedText.length)
        var candidateCharacters = Set<Int>()

        if clampedOffset < textView.attributedText.length {
            candidateCharacters.insert(clampedOffset)
        }
        if clampedOffset > 0 {
            candidateCharacters.insert(clampedOffset - 1)
        }
        if clampedOffset + 1 < textView.attributedText.length {
            candidateCharacters.insert(clampedOffset + 1)
        }

        let referenceMidY = referenceRect.midY
        let referenceMinY = referenceRect.minY
        var bestMatch: (score: CGFloat, baselineY: CGFloat)?

        for characterIndex in candidateCharacters.sorted() {
            let glyphIndex = textView.layoutManager.glyphIndexForCharacter(at: characterIndex)
            guard glyphIndex < textView.layoutManager.numberOfGlyphs else { continue }

            let lineFragmentRect = textView.layoutManager.lineFragmentRect(
                forGlyphAt: glyphIndex,
                effectiveRange: nil
            )
            let lineRectInView = lineFragmentRect.offsetBy(dx: 0, dy: textView.textContainerInset.top)
            let score = abs(lineRectInView.midY - referenceMidY) * 10
                + abs(lineRectInView.minY - referenceMinY)
            let glyphLocation = textView.layoutManager.location(forGlyphAt: glyphIndex)
            let baselineY = textView.textContainerInset.top + lineFragmentRect.minY + glyphLocation.y

            if let currentBest = bestMatch, currentBest.score <= score {
                continue
            }
            bestMatch = (score, baselineY)
        }

        return bestMatch?.baselineY
    }

    func setCollapsedSelection(in textView: UITextView, utf16Offset: Int) {
        guard
            let position = textView.position(from: textView.beginningOfDocument, offset: utf16Offset),
            let range = textView.textRange(from: position, to: position)
        else {
            XCTFail("expected caret position at offset \(utf16Offset)")
            return
        }

        textView.selectedTextRange = range
    }

    func setSelection(in textView: UITextView, utf16Range: NSRange) {
        guard
            let start = textView.position(from: textView.beginningOfDocument, offset: utf16Range.location),
            let end = textView.position(from: start, offset: utf16Range.length),
            let range = textView.textRange(from: start, to: end)
        else {
            XCTFail("expected selection range \(utf16Range)")
            return
        }

        textView.selectedTextRange = range
    }

    private func selectedUtf16Range(in textView: UITextView) -> NSRange? {
        guard let range = textView.selectedTextRange else { return nil }
        let location = textView.offset(from: textView.beginningOfDocument, to: range.start)
        let length = textView.offset(from: range.start, to: range.end)
        guard location >= 0, length >= 0 else { return nil }
        return NSRange(location: location, length: length)
    }

    func assertSelectedUtf16Range(
        in textView: UITextView,
        _ expectedRange: NSRange,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertEqual(selectedUtf16Range(in: textView), expectedRange, file: file, line: line)
    }

    func firstImageRange(in textView: UITextView) -> NSRange? {
        guard textView.textStorage.length > 0 else { return nil }

        for index in 0..<textView.textStorage.length {
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            if (attrs[RenderBridgeAttributes.voidNodeType] as? String) == "image" {
                return NSRange(location: index, length: 1)
            }
        }

        return nil
    }

    func renderedRect(in textView: UITextView, utf16Range: NSRange) -> CGRect {
        let glyphRange = textView.layoutManager.glyphRange(
            forCharacterRange: utf16Range,
            actualCharacterRange: nil
        )
        var rect = textView.layoutManager.boundingRect(forGlyphRange: glyphRange, in: textView.textContainer)
        rect.origin.x += textView.textContainerInset.left - textView.contentOffset.x
        rect.origin.y += textView.textContainerInset.top - textView.contentOffset.y
        return rect
    }

    func aliceMentionAddonsJson() -> String {
        """
        {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
        """
    }

    func hostEditorView(_ view: RichTextEditorView) -> UIWindow {
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        view.layoutIfNeeded()
        return window
    }

    func hostNativeEditorExpoView(_ view: NativeEditorExpoView) -> UIWindow {
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        view.layoutIfNeeded()
        return window
    }

    func flushMainQueue() {
        let expectation = expectation(description: "flush main queue")
        DispatchQueue.main.async {
            expectation.fulfill()
        }
        wait(for: [expectation], timeout: 1.0)
    }

    func currentSelection(in editorId: UInt64) -> [String: Any] {
        let data = EditorV2Shadow.getSelection(id: editorId).data(using: .utf8)
        XCTAssertNotNil(data)
        let json = try? JSONSerialization.jsonObject(with: data ?? Data()) as? [String: Any]
        XCTAssertNotNil(json)
        return json ?? [:]
    }

    func parseJSONObject(_ json: String?) -> [String: Any] {
        guard let json else {
            XCTFail("expected JSON string")
            return [:]
        }
        let data = json.data(using: .utf8)
        XCTAssertNotNil(data)
        let object = try? JSONSerialization.jsonObject(with: data ?? Data()) as? [String: Any]
        XCTAssertNotNil(object)
        return object ?? [:]
    }

    private func activeState(in editorId: UInt64) -> (insertableNodes: [String], allowedMarks: [String]) {
        let data = EditorV2Shadow.getCurrentState(id: editorId).data(using: .utf8)
        XCTAssertNotNil(data)
        let json = try? JSONSerialization.jsonObject(with: data ?? Data()) as? [String: Any]
        let activeState = json?["activeState"] as? [String: Any]
        let insertableNodes = (activeState?["insertableNodes"] as? [String]) ?? []
        let allowedMarks = (activeState?["allowedMarks"] as? [String]) ?? []
        return (insertableNodes: insertableNodes, allowedMarks: allowedMarks)
    }

    func mentionEditorConfigJson() -> String {
        let config: [String: Any] = [
            "initialization": ["type": "localEmpty"],
            "schema": [
                "nodes": [
                    [
                        "name": "doc",
                        "content": "block+",
                        "role": "doc",
                    ],
                    [
                        "name": "paragraph",
                        "content": "inline*",
                        "group": "block",
                        "role": "textBlock",
                        "htmlTag": "p",
                    ],
                    [
                        "name": "bulletList",
                        "content": "listItem+",
                        "group": "block",
                        "role": "list",
                        "htmlTag": "ul",
                    ],
                    [
                        "name": "orderedList",
                        "content": "listItem+",
                        "group": "block",
                        "role": "list",
                        "htmlTag": "ol",
                        "attrs": [
                            "start": ["default": 1],
                        ],
                    ],
                    [
                        "name": "listItem",
                        "content": "paragraph block*",
                        "role": "listItem",
                        "htmlTag": "li",
                    ],
                    [
                        "name": "hardBreak",
                        "content": "",
                        "group": "inline",
                        "role": "hardBreak",
                        "htmlTag": "br",
                        "isVoid": true,
                    ],
                    [
                        "name": "horizontalRule",
                        "content": "",
                        "group": "block",
                        "role": "block",
                        "htmlTag": "hr",
                        "isVoid": true,
                    ],
                    [
                        "name": "text",
                        "content": "",
                        "group": "inline",
                        "role": "text",
                    ],
                    [
                        "name": "mention",
                        "content": "",
                        "group": "inline",
                        "role": "inline",
                        "isVoid": true,
                        // Mirrors mentionNodeSpec() in src/addons.ts: mention nodes
                        // round-trip arbitrary app-defined attrs (e.g.
                        // mentionSuggestionChar) that this fixed attrs map cannot
                        // enumerate, so opt out of the schema-declared-attrs filter
                        // that Rust's set_json ingestion otherwise applies.
                        "allowUndeclaredAttrs": true,
                        "attrs": [
                            "label": ["default": NSNull()],
                        ],
                    ],
                ],
                "marks": [
                    ["name": "bold"],
                    ["name": "italic"],
                    ["name": "underline"],
                    ["name": "strike"],
                ],
            ],
        ]

        let data = try! JSONSerialization.data(withJSONObject: config)
        return String(data: data, encoding: .utf8)!
    }
}

/// Mirrors react-native-keyboard-controller's `KCTextInputCompositeDelegate`
/// call forwarding: the composite wraps the text view's current delegate and
/// forwards every selector it does not implement itself to that delegate via
/// `responds(to:)` / `forwardingTarget(for:)`.
private final class ForwardingCompositeTextViewDelegateSpy: NSObject, UITextViewDelegate {
    weak var wrappedDelegate: UITextViewDelegate?

    init(wrappedDelegate: UITextViewDelegate?) {
        self.wrappedDelegate = wrappedDelegate
    }

    override func responds(to aSelector: Selector!) -> Bool {
        if super.responds(to: aSelector) {
            return true
        }
        return wrappedDelegate?.responds(to: aSelector) ?? false
    }

    override func forwardingTarget(for aSelector: Selector!) -> Any? {
        if wrappedDelegate?.responds(to: aSelector) ?? false {
            return wrappedDelegate
        }
        return super.forwardingTarget(for: aSelector)
    }
}

private final class KeyboardProviderTextViewDelegateSpy: NSObject, UITextViewDelegate {
    weak var textViewDelegate: UITextViewDelegate?
    private(set) var selectionChangeCount = 0
    private(set) var textChangeCount = 0

    init(textViewDelegate: UITextViewDelegate?) {
        self.textViewDelegate = textViewDelegate
    }

    func textViewDidChangeSelection(_ textView: UITextView) {
        selectionChangeCount += 1
        textViewDelegate?.textViewDidChangeSelection?(textView)
        if let range = textView.selectedTextRange {
            _ = textView.firstRect(for: range)
            _ = textView.caretRect(for: range.start)
            _ = textView.caretRect(for: range.end)
            _ = textView.offset(from: textView.beginningOfDocument, to: range.start)
            _ = textView.offset(from: textView.beginningOfDocument, to: range.end)
        }
    }

    func textViewDidChange(_ textView: UITextView) {
        textChangeCount += 1
        _ = textView.text
        textViewDelegate?.textViewDidChange?(textView)
    }
}

final class EditorTextViewDelegateSpy: NSObject, EditorTextViewDelegate {
    var selectionChanges: [(anchor: UInt32, head: UInt32)] = []
    var receivedUpdates: [String] = []
    var externalCompositionEnds: [String] = []

    func editorTextView(_ textView: EditorTextView, selectionDidChange anchor: UInt32, head: UInt32) {
        selectionChanges.append((anchor: anchor, head: head))
    }

    func editorTextView(_ textView: EditorTextView, didReceiveUpdate updateJSON: String) {
        receivedUpdates.append(updateJSON)
    }

    func editorTextView(_ textView: EditorTextView, didEndExternalTextComposition resultJSON: String) {
        externalCompositionEnds.append(resultJSON)
    }
}
