import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
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

}
