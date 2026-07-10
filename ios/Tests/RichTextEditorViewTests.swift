import XCTest

final class RichTextEditorViewTests: XCTestCase {
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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>😀</p>")
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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")
        editorSetSelectionScalar(id: editorId, scalarAnchor: 5, scalarHead: 5)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)

        let delegateSpy = KeyboardProviderTextViewDelegateSpy(textViewDelegate: textView.delegate)
        textView.delegate = delegateSpy
        XCTAssertFalse(textView.isUsingInternalTextViewDelegateForTesting())

        textView.insertText("!")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello!</p>")
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

    func testParagraphSplitAppliesTopLevelRenderPatch() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")

        let betaRange = (textView.text as NSString).range(of: "Beta")
        XCTAssertNotEqual(betaRange.location, NSNotFound)
        let splitOffset = UInt32(betaRange.location + betaRange.length)
        editorSetSelectionScalar(id: editorId, scalarAnchor: splitOffset, scalarHead: splitOffset)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)

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
            editorGetHtml(id: editorId),
            "<p>Alpha</p><p>Beta</p><p></p><p>Gamma</p>"
        )
    }

    func testSequentialParagraphSplitsKeepUsingTopLevelRenderPatch() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 180))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")

        let betaRange = (textView.text as NSString).range(of: "Beta")
        XCTAssertNotEqual(betaRange.location, NSNotFound)
        let firstSplitOffset = UInt32(betaRange.location + betaRange.length)
        editorSetSelectionScalar(id: editorId, scalarAnchor: firstSplitOffset, scalarHead: firstSplitOffset)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
        textView.insertText("\n")

        XCTAssertTrue(textView.lastRenderAppliedPatch())

        let gammaRange = (textView.text as NSString).range(of: "Gamma")
        XCTAssertNotEqual(gammaRange.location, NSNotFound)
        let secondSplitOffset = UInt32(gammaRange.location + gammaRange.length)
        editorSetSelectionScalar(id: editorId, scalarAnchor: secondSplitOffset, scalarHead: secondSplitOffset)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
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
            editorGetHtml(id: editorId),
            "<p>Alpha</p><p>Beta</p><p></p><p>Gamma</p><p></p>"
        )
    }

    func testTypingInsideListItemFallsBackToFullRenderAndPreservesTextOrder() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
            editorGetHtml(id: editorId),
            "<ul><li><p>Alpha!</p></li><li><p>Beta</p></li></ul>"
        )
    }

    func testReturnInsideListItemFallsBackToFullRenderAndKeepsTypingInNewItem() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
            editorGetHtml(id: editorId),
            "<ul><li><p>Alpha</p></li><li><p>B</p></li><li><p>Beta</p></li></ul>"
        )
    }

    func testFullCurrentStateLocalEditUsesSynthesizedTopLevelPatch() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
        _ = editorSetJson(id: editorId, json: updatedDocument)

        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)

        XCTAssertTrue(
            textView.lastRenderAppliedPatch(),
            "full current-state updates should synthesize a top-level patch when only a local block range changes"
        )
        XCTAssertEqual(textView.textStorage.string, "Alpha\nBetter\nGamma")
    }

    func testIdenticalFullCurrentStateSkipsNativeTextReapply() throws {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")
        textView.captureApplyUpdateTraceForTesting = true

        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)

        let trace = try XCTUnwrap(textView.lastApplyUpdateTrace())
        XCTAssertFalse(textView.lastRenderAppliedPatch())
        XCTAssertEqual(trace.buildRenderNanos, 0)
        XCTAssertEqual(trace.applyRenderNanos, 0)
        XCTAssertEqual(trace.applyRenderTextMutationNanos, 0)
        XCTAssertEqual(textView.textStorage.string, "Alpha\nBeta\nGamma")
    }

    func testRustDrivenSelectionApplyDoesNotNotifySelectionDelegate() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let delegate = EditorTextViewDelegateSpy()
        textView.editorDelegate = delegate
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p>")
        delegate.selectionChanges.removeAll()
        delegate.receivedUpdates.removeAll()

        editorSetSelectionScalar(id: editorId, scalarAnchor: 8, scalarHead: 8)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
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
        let editorId = editorCreate(configJson: #"{"allowBase64Images":true}"#)
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        view.layoutIfNeeded()

        let docPos = editorScalarToDoc(id: editorId, scalar: 6)
        view.setRemoteSelections([
            RemoteSelectionDecoration(
                clientId: 7,
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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        view.layoutIfNeeded()

        let endDocPos = editorScalarToDoc(id: editorId, scalar: 11)
        view.setRemoteSelections([
            RemoteSelectionDecoration(
                clientId: 9,
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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

        let docPos = editorScalarToDoc(id: editorId, scalar: targetScalar)
        view.setRemoteSelections([
            RemoteSelectionDecoration(
                clientId: 10,
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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        view.layoutIfNeeded()

        let docPos = editorScalarToDoc(id: editorId, scalar: 6)
        view.setRemoteSelections([
            RemoteSelectionDecoration(
                clientId: 8,
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
            "backgroundColor": "#d7e4ff",
            "optionTextColor": "#1a2c48",
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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>Hello</p>")
        editorSetSelectionScalar(id: editorId, scalarAnchor: 5, scalarHead: 5)

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        view.setToolbarPlacement("inline")
        view.richTextView.textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)

        view.richTextView.textView.insertText("!")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello!</p>")
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
        XCTAssertEqual(theme.toolbar?.resolvedHorizontalInset ?? 0, 10, accuracy: 0.1)
    }

    func testAccessoryToolbarAppliesNativeAppearanceChrome() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
            "height": 44,
        ]))
        // This test targets the always-on chrome (glass blur + hairline border) that the
        // custom stack toolbar itself renders for "native" appearance. On iOS 26 the bar
        // toolbar (UIToolbar) supplies its own translucent chrome instead and intentionally
        // suppresses these (see apply(theme:animateChrome:)'s usesBarToolbar branches), so
        // pin this test to the custom stack path to keep testing what it was written to test.
        toolbar.usesNativeBarToolbarOverrideForTesting = false

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

    func testAccessoryToolbarNativeDisabledButtonUsesTransparentTintAtFullAlpha() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

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
        // This test targets the mention-suggestions-specific chrome transparency (the outer
        // chrome fades out only while suggestions are showing). On iOS 26 the bar toolbar makes
        // the outer chrome transparent unconditionally (UIToolbar supplies its own translucent
        // material), which would make this test's baseline "not transparent yet" assertion
        // meaningless. Pin to the custom stack path to keep testing the mention-specific behavior.
        toolbar.usesNativeBarToolbarOverrideForTesting = false

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

        // This test targets the mention-suggestions-specific chrome fade-out animation on the
        // custom stack toolbar's own blur/glass chrome. On iOS 26 the bar toolbar makes the
        // outer chrome transparent unconditionally (UIToolbar supplies its own translucent
        // material) rather than animating a fade tied to mention state, so pin to the custom
        // stack path to keep testing the animated-transition behavior this test was written for.
        toolbar.usesNativeBarToolbarOverrideForTesting = false

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

        if #available(iOS 26.0, *) {
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
    }

    func testAccessoryToolbarNativeLayoutPreservesScrolledOffsetAcrossRelayout() {
        let toolbar = EditorAccessoryToolbarView(frame: CGRect(x: 0, y: 0, width: 180, height: 56))

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.layoutIfNeeded()

        if #available(iOS 26.0, *) {
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

    /// With appearance native on iOS 26, the bar toolbar activates, carries
    /// the scroll items, and the pinned stacks stay visible without overlap.
    func testNativeBarToolbarActivatesWithPinnedItems() throws {
        guard #available(iOS 26.0, *) else { throw XCTSkip("native bar requires iOS 26") }

        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.nativeBarToolbarFixtureJSON)
        host.layoutIfNeeded()

        XCTAssertFalse(
            toolbar.nativeToolbarScrollViewIsHiddenForTesting,
            "the UIToolbar-backed scroll view should be visible once native appearance is active on iOS 26"
        )
        XCTAssertTrue(
            toolbar.contentStackViewIsHiddenForTesting,
            "the custom stack toolbar's content column should be hidden while the bar toolbar owns the middle slot"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("start"),
            ["Start"],
            "start-pinned items should remain custom buttons beside the bar, not bar buttons"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("end"),
            ["End"],
            "end-pinned items should remain custom buttons beside the bar, not bar buttons"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("scroll"),
            ["Scroll One", "Scroll Two"],
            "only scroll-placement items should be carried by the UIToolbar bar buttons"
        )

        let barFrame = toolbar.nativeToolbarScrollViewFrameForTesting
        let startFrame = toolbar.startPinnedStackViewFrameForTesting
        let endFrame = toolbar.endPinnedStackViewFrameForTesting
        XCTAssertGreaterThan(
            barFrame.width,
            0,
            "the bar toolbar must actually claim the middle slot's width, not just avoid overlap by being empty"
        )
        XCTAssertFalse(
            barFrame.intersects(startFrame),
            "bar toolbar frame \(barFrame) must not overlap the start pinned stack frame \(startFrame)"
        )
        XCTAssertFalse(
            barFrame.intersects(endFrame),
            "bar toolbar frame \(barFrame) must not overlap the end pinned stack frame \(endFrame)"
        )
    }

    /// Structural counterpart to `testNativeBarToolbarActivatesWithPinnedItems` that does not
    /// require an iOS 26 runtime: it drives the show/hide and no-overlap layout behavior directly
    /// through `usesNativeBarToolbarOverrideForTesting`, so the `bodyStackView` restructure (the
    /// bar toolbar and `contentStackView` occupying the same arranged-subview slot) is verified
    /// even when the test simulator predates iOS 26.
    func testNativeBarToolbarOverrideHidesContentStackAndAvoidsPinnedOverlap() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.nativeBarToolbarFixtureJSON)

        // Each override toggle flips isHidden on two arranged subviews that swap the same
        // bodyStackView slot; explicitly re-dirtying layout before layoutIfNeeded() ensures
        // UIStackView fully redistributes the freed/claimed width within this single synchronous
        // test call (outside a test harness, the normal run-loop display cycle does this for free).
        toolbar.usesNativeBarToolbarOverrideForTesting = false
        host.setNeedsLayout()
        host.layoutIfNeeded()
        XCTAssertTrue(
            toolbar.nativeToolbarScrollViewIsHiddenForTesting,
            "with the bar toolbar forced off, the bar scroll view should stay hidden"
        )
        XCTAssertFalse(
            toolbar.contentStackViewIsHiddenForTesting,
            "with the bar toolbar forced off, the custom stack toolbar's content column should stay visible"
        )
        XCTAssertGreaterThan(
            toolbar.contentStackViewFrameForTesting.width,
            0,
            "with the bar toolbar forced off, the content column should claim the middle slot's width"
        )

        toolbar.usesNativeBarToolbarOverrideForTesting = true
        host.setNeedsLayout()
        host.layoutIfNeeded()

        XCTAssertFalse(
            toolbar.nativeToolbarScrollViewIsHiddenForTesting,
            "forcing usesNativeBarToolbar on should reveal the bar toolbar scroll view"
        )
        XCTAssertTrue(
            toolbar.contentStackViewIsHiddenForTesting,
            "forcing usesNativeBarToolbar on should hide the custom stack toolbar's content column"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("start"),
            ["Start"],
            "pinned start items remain custom buttons beside the bar even when the bar is forced on"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("end"),
            ["End"],
            "pinned end items remain custom buttons beside the bar even when the bar is forced on"
        )

        let barFrame = toolbar.nativeToolbarScrollViewFrameForTesting
        let startFrame = toolbar.startPinnedStackViewFrameForTesting
        let endFrame = toolbar.endPinnedStackViewFrameForTesting
        XCTAssertGreaterThan(
            barFrame.width,
            0,
            "the bar toolbar must actually claim the middle slot's width, not just avoid overlap by being empty"
        )
        XCTAssertFalse(
            barFrame.intersects(startFrame),
            "bar toolbar frame \(barFrame) must not overlap the start pinned stack frame \(startFrame) (forced-on)"
        )
        XCTAssertFalse(
            barFrame.intersects(endFrame),
            "bar toolbar frame \(barFrame) must not overlap the end pinned stack frame \(endFrame) (forced-on)"
        )

        toolbar.usesNativeBarToolbarOverrideForTesting = nil
    }

    /// Sibling to `testNativeBarToolbarOverrideHidesContentStackAndAvoidsPinnedOverlap`: covers the
    /// review-flagged combination of "native bar toolbar active" + "mention suggestions shown"
    /// together. When mentions appear while the bar path is active, `apply(theme:animateChrome:)`'s
    /// `nativeToolbarScrollView.isHidden = !(usesBarToolbar && mentionButtons.isEmpty)` /
    /// `contentStackView.isHidden = usesBarToolbar && mentionButtons.isEmpty` pair (lines ~1112-1117)
    /// swaps the bar back out for the custom content stack (which hosts the mention row), then swaps
    /// back once suggestions clear. This combination was previously untested: the bar tests never
    /// called `setMentionSuggestions`, and the mention-chrome tests pinned
    /// `usesNativeBarToolbarOverrideForTesting = false`.
    func testNativeBarToolbarSwapsToContentStackWhenMentionSuggestionsAppear() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.nativeBarToolbarFixtureJSON)
        toolbar.usesNativeBarToolbarOverrideForTesting = true
        host.setNeedsLayout()
        host.layoutIfNeeded()

        // Precondition: the bar path is active before mentions appear (same forced-on state as
        // testNativeBarToolbarOverrideHidesContentStackAndAvoidsPinnedOverlap).
        XCTAssertFalse(
            toolbar.nativeToolbarScrollViewIsHiddenForTesting,
            "precondition: the bar toolbar should be visible before mention suggestions appear"
        )
        XCTAssertTrue(
            toolbar.contentStackViewIsHiddenForTesting,
            "precondition: the custom content stack should be hidden before mention suggestions appear"
        )

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
        XCTAssertFalse(
            toolbar.contentStackViewIsHiddenForTesting,
            "the custom content stack should reappear to host the mention row once suggestions show, even while the bar toolbar is active"
        )
        XCTAssertTrue(
            toolbar.nativeToolbarScrollViewIsHiddenForTesting,
            "the bar toolbar scroll view should hide once mention suggestions swap the middle slot back to the content stack"
        )
        XCTAssertEqual(
            toolbar.mentionButtonAtForTesting(0)?.titleTextForTesting(),
            "@alice",
            "the mention suggestion chip should render inside the now-visible content stack"
        )

        // Pinned stacks must still render their bar-path buttons and must not overlap whichever
        // sibling now occupies the middle slot (contentStackView, not the bar).
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("start"),
            ["Start"],
            "start-pinned items should keep rendering while mention suggestions are shown on the bar path"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("end"),
            ["End"],
            "end-pinned items should keep rendering while mention suggestions are shown on the bar path"
        )

        let contentFrame = toolbar.contentStackViewFrameForTesting
        let startFrame = toolbar.startPinnedStackViewFrameForTesting
        let endFrame = toolbar.endPinnedStackViewFrameForTesting
        XCTAssertGreaterThan(
            contentFrame.width,
            0,
            "the content stack must actually claim the middle slot's width while showing mentions, not just avoid overlap by being empty"
        )
        XCTAssertFalse(
            contentFrame.intersects(startFrame),
            "content stack frame \(contentFrame) must not overlap the start pinned stack frame \(startFrame) while mentions are shown"
        )
        XCTAssertFalse(
            contentFrame.intersects(endFrame),
            "content stack frame \(contentFrame) must not overlap the end pinned stack frame \(endFrame) while mentions are shown"
        )

        // Clearing suggestions should restore the bar path.
        let didChangeBack = toolbar.setMentionSuggestions([], trigger: "@")
        host.setNeedsLayout()
        host.layoutIfNeeded()

        XCTAssertTrue(didChangeBack, "setMentionSuggestions should report a mode change back to empty")
        XCTAssertFalse(
            toolbar.nativeToolbarScrollViewIsHiddenForTesting,
            "clearing mention suggestions should restore the bar toolbar scroll view"
        )
        XCTAssertTrue(
            toolbar.contentStackViewIsHiddenForTesting,
            "clearing mention suggestions should hide the custom content stack again"
        )

        toolbar.usesNativeBarToolbarOverrideForTesting = nil
    }

    private static let nativeBarToolbarFixtureJSON = """
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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
        let expectedDoc = editorScalarToDoc(id: editorId, scalar: 2)

        XCTAssertEqual(selection["type"] as? String, "text")
        XCTAssertEqual((selection["anchor"] as? NSNumber)?.uint32Value, expectedDoc)
        XCTAssertEqual((selection["head"] as? NSNumber)?.uint32Value, expectedDoc)
    }

    func testManualSelectionIntoListItemRefreshesSelectionDependentActiveState() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
            activeState(in: editorId).insertableNodes.contains("horizontalRule"),
            "horizontal rule should be insertable in a normal paragraph"
        )

        setCollapsedSelection(in: textView, utf16Offset: listOffset + 2)
        flushMainQueue()
        XCTAssertFalse(
            activeState(in: editorId).insertableNodes.contains("horizontalRule"),
            "horizontal rule should be disabled in list items after a manual caret move"
        )
    }

    func testManualSelectionInMiddleOfWordPersistsAfterDeferredSelectionSync() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
        let expectedDoc = editorScalarToDoc(id: editorId, scalar: UInt32(secondParagraphOffset + 3))

        XCTAssertEqual(selection["type"] as? String, "text")
        XCTAssertEqual((selection["anchor"] as? NSNumber)?.uint32Value, expectedDoc)
        XCTAssertEqual((selection["head"] as? NSNumber)?.uint32Value, expectedDoc)
    }

    func testUnauthorizedTextMutationReconcilesOnNextRunLoop() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        let authorizedText = textView.textStorage.string

        textView.textStorage.replaceCharacters(in: NSRange(location: 0, length: 1), with: "X")

        XCTAssertEqual(textView.reconciliationCount, 1)
        XCTAssertEqual(
            textView.textStorage.string,
            "Xello",
            "reconciliation should not run synchronously inside the text storage edit callback"
        )

        flushMainQueue()

        XCTAssertEqual(textView.textStorage.string, authorizedText)
    }

    func testFocusedNativeTextMutationCommitsToRustInsteadOfReconciling() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 6, length: 5),
            with: "there"
        )

        XCTAssertEqual(view.textView.textStorage.string, "Hello there")
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        flushMainQueue()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello there</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello there")
    }

    func testFocusedNativeAutocompleteInsertionCommitsToRustOnNextRunLoop() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 6)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 6, length: 0),
            with: "there"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        flushMainQueue()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello there</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello there")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeAutocompleteInsertionMapsStaleCaretBeforeNextTypedCharacter() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 6)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 6, length: 0),
            with: "there"
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 6, length: 0))

        view.textView.insertText("!")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello there!</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello there!")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 12, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 12)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeAutocompleteInsertionMapsStaleCaretOnScheduledCommit() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 6)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 6, length: 0),
            with: "there"
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 6, length: 0))

        flushMainQueue()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello there</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello there")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 11, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 11)

        view.textView.insertText("!")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello there!</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello there!")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 12, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 12)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeReplacementKeepsCollapsedStaleCaretCollapsedInsideReplacementRange() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>abcd </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 2)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 4),
            with: "correct"
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 2, length: 0))

        view.textView.insertText("!")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>correct! </p>")
        XCTAssertEqual(view.textView.textStorage.string, "correct! ")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 8, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 8)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testInlinePredictionMutationIsNotCommittedToRust() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>autocom</p>")

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setCollapsedSelection(in: view.textView, utf16Offset: 7)
        flushMainQueue()

        // Simulate iOS inline prediction: iOS mutates textStorage directly
        // and sets markedTextRange without calling setMarkedText.
        view.textView.setMarkedText("plete", selectedRange: NSRange(location: 5, length: 0))

        flushMainQueue()

        // The prediction text must NOT be committed to Rust — Rust state
        // should still reflect "autocom", not "autocomplete".
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<p>autocom</p>",
            "inline prediction text must not be committed to Rust"
        )
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        // Now the user types 'p' while prediction is active.
        // This should commit only 'p' and discard the prediction.
        view.textView.insertText("p")

        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<p>autocomp</p>",
            "only the typed character should be committed, not the prediction"
        )
        XCTAssertEqual(view.textView.textStorage.string, "autocomp")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testInlinePredictionDoesNotCauseReconciliation() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>hello wor</p>")

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setCollapsedSelection(in: view.textView, utf16Offset: 9)
        flushMainQueue()

        // Simulate prediction appearing: textStorage gets "ld" appended as marked text.
        view.textView.setMarkedText("ld", selectedRange: NSRange(location: 2, length: 0))

        // Prediction must be treated as transient — no reconciliation.
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        flushMainQueue()

        // After a run loop cycle, still no reconciliation and Rust unchanged.
        XCTAssertEqual(view.textView.reconciliationCount, 0)
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<p>hello wor</p>",
            "prediction text must not leak into Rust state"
        )
    }

    func testFocusedNativeDeletionCorrectionCommitsToRustOnNextRunLoop() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello  world</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 7)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 5, length: 1),
            with: ""
        )
        setCollapsedSelection(in: view.textView, utf16Offset: 5)

        flushMainQueue()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello world</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello world")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationFlushesBeforeNextTypedCharacter() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        view.textView.insertText("n")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationInListUsesAdjustedScalarOffsetsBeforeNextTypedCharacter() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<ul><li><p>teh </p></li></ul>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(editorGetHtml(id: editorId), "<ul><li><p>the n</p></li></ul>")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationInListMapsStaleCaretBeforeNextTypedCharacter() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<ul><li><p>teh </p></li></ul>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 4, length: 0))

        view.textView.insertText("n")

        XCTAssertEqual(editorGetHtml(id: editorId), "<ul><li><p>the n</p></li></ul>")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 5, length: 0))
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationInSecondListItemUsesAdjustedScalarOffsets() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 140))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<ul><li><p>one</p></li><li><p>teh </p></li></ul>")
        let correctionRange = (view.textView.textStorage.string as NSString).range(of: "teh")
        XCTAssertNotEqual(correctionRange.location, NSNotFound)
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(in: correctionRange, with: "the")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<ul><li><p>one</p></li><li><p>the n</p></li></ul>"
        )
        XCTAssertEqual(view.textView.textStorage.string, "one\nthe n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationInNestedListUsesAdjustedScalarOffsets() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<ul><li><p>parent</p><ul><li><p>teh </p></li></ul></li></ul>")
        let correctionRange = (view.textView.textStorage.string as NSString).range(of: "teh")
        XCTAssertNotEqual(correctionRange.location, NSNotFound)
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(in: correctionRange, with: "the")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<ul><li><p>parent</p><ul><li><p>the n</p></li></ul></li></ul>"
        )
        XCTAssertEqual(view.textView.textStorage.string, "parent\nthe n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPendingNativeTextMutationInTwoDigitOrderedListUsesAdjustedScalarOffsets() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<ol start=\"10\"><li><p>one</p></li><li><p>teh </p></li></ol>")
        let correctionRange = (view.textView.textStorage.string as NSString).range(of: "teh")
        XCTAssertNotEqual(correctionRange.location, NSNotFound)
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(in: correctionRange, with: "the")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<ol start=\"10\"><li><p>one</p></li><li><p>the n</p></li></ol>"
        )
        XCTAssertEqual(view.textView.textStorage.string, "one\nthe n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPasteFlushesPendingNativeAutocorrectBeforePlainTextPaste() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            UIPasteboard.general.items = []
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
        setCollapsedSelection(in: view.textView, utf16Offset: 4)

        UIPasteboard.general.string = "now"
        view.textView.paste(nil)

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the now</p>")
        XCTAssertEqual(view.textView.textStorage.string, "the now")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeMutationUsesUIKitSelectionAlreadyMovedBeforeCapture() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>abcdef</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.beginEditing()
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "ABC"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: 3)
        view.textView.textStorage.endEditing()

        flushMainQueue()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>ABCdef</p>")
        XCTAssertEqual(view.textView.textStorage.string, "ABCdef")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 3, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 3)

        view.textView.insertText("!")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>ABC!def</p>")
        XCTAssertEqual(view.textView.textStorage.string, "ABC!def")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 4, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 4)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testPasteFlushesPendingNativeAutocorrectBeforeReplacingSelectedText() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            UIPasteboard.general.items = []
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>teh old</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 3)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        let oldRange = (view.textView.textStorage.string as NSString).range(of: "old")
        XCTAssertNotEqual(oldRange.location, NSNotFound)
        setSelection(in: view.textView, utf16Range: oldRange)

        UIPasteboard.general.string = "now"
        view.textView.paste(nil)

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the now</p>")
        XCTAssertEqual(view.textView.textStorage.string, "the now")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testHTMLPasteFlushesPendingNativeAutocorrectBeforePaste() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            UIPasteboard.general.items = []
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
        setCollapsedSelection(in: view.textView, utf16Offset: 4)

        UIPasteboard.general.setData(
            Data("<strong>now</strong>".utf8),
            forPasteboardType: "public.html"
        )
        view.textView.paste(nil)

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p><p><strong>now</strong></p>")
        XCTAssertEqual(view.textView.textStorage.string, "the \nnow")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testRTFPasteFlushesPendingNativeAutocorrectBeforePaste() throws {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            UIPasteboard.general.items = []
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
        setCollapsedSelection(in: view.textView, utf16Offset: 4)

        let attributedPaste = NSAttributedString(
            string: "now",
            attributes: [.font: UIFont.boldSystemFont(ofSize: 14)]
        )
        let rtfData = try attributedPaste.data(
            from: NSRange(location: 0, length: attributedPaste.length),
            documentAttributes: [.documentType: NSAttributedString.DocumentType.rtf]
        )
        UIPasteboard.general.setData(rtfData, forPasteboardType: "public.rtf")
        XCTAssertNotNil(UIPasteboard.general.data(forPasteboardType: "public.rtf"))

        view.textView.paste(nil)

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(html.contains("the"), "RTF paste should preserve native correction, got: \(html)")
        XCTAssertTrue(html.contains("now"), "RTF paste should insert converted rich text, got: \(html)")
        XCTAssertTrue(view.textView.textStorage.string.contains("the"))
        XCTAssertTrue(view.textView.textStorage.string.contains("now"))
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testInterceptWindowAutocorrectCommitsBeforeImmediateNextCharacter() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>teh</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 3)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.insertText(" ")
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: 4)

        view.textView.insertText("n")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeReplaceAutocorrectWithEmojiPrefixCommitsBeforeNextCharacter() throws {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>😀 teh </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        let start = try XCTUnwrap(view.textView.position(from: view.textView.beginningOfDocument, offset: 3))
        let end = try XCTUnwrap(view.textView.position(from: start, offset: 3))
        let correctionRange = try XCTUnwrap(view.textView.textRange(from: start, to: end))
        view.textView.replace(correctionRange, withText: "the")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>😀 the n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "😀 the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeEmojiReplacementAutocorrectDoesNotSplitSurrogatePairs() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>😀 test</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 2),
            with: "😁"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("!")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>😁 test!</p>")
        XCTAssertEqual(view.textView.textStorage.string, "😁 test!")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testNativeAutocorrectAfterComplexEmojiGraphemesPreservesScalarMapping() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>👨‍👩‍👧‍👦 🇦🇺 1️⃣ teh </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        let correctionRange = (view.textView.textStorage.string as NSString).range(of: "teh")
        XCTAssertNotEqual(correctionRange.location, NSNotFound)
        view.textView.textStorage.replaceCharacters(in: correctionRange, with: "the")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>👨‍👩‍👧‍👦 🇦🇺 1️⃣ the n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "👨‍👩‍👧‍👦 🇦🇺 1️⃣ the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testLengthChangingAutocorrectAfterComplexEmojiGraphemesMapsStaleCaret() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        let prefix = "👨‍👩‍👧‍👦 🇦🇺 1️⃣ "
        view.editorId = editorId
        view.setContent(html: "<p>\(prefix)dont </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())

        let correctionRange = (view.textView.textStorage.string as NSString).range(of: "dont")
        XCTAssertNotEqual(correctionRange.location, NSNotFound)
        view.textView.textStorage.replaceCharacters(in: correctionRange, with: "don't")
        assertSelectedUtf16Range(
            in: view.textView,
            NSRange(location: prefix.utf16.count + 5, length: 0)
        )

        view.textView.insertText("n")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>\(prefix)don't n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "\(prefix)don't n")
        assertSelectedUtf16Range(
            in: view.textView,
            NSRange(location: view.textView.textStorage.length, length: 0)
        )
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testLengthChangingAutocorrectInvalidatesCachedPositionMappingBeforeSelectionCapture() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>dont </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        _ = PositionBridge.cursorScalarOffset(in: view.textView)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 4),
            with: "don't"
        )
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)

        view.textView.insertText("n")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>don't n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "don't n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testLengthChangingAutocorrectMapsStaleCaretBeforeNextTypedCharacter() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>dont </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: view.textView.textStorage.length)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 3, length: 1),
            with: "'t"
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 5, length: 0))

        view.textView.insertText("n")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>don't n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "don't n")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 7, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 7)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testLengthShrinkingAutocorrectMapsStaleCaretBeforeNextTypedCharacter() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello  world</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 7)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 5, length: 1),
            with: ""
        )
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 7, length: 0))

        view.textView.insertText("!")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello !world</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello !world")
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 7, length: 0))
        assertCollapsedEditorSelection(in: editorId, scalarOffset: 7)
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testSetMarkedTextFlushesPendingStaleNativeAutocorrectBeforeComposition() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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
        assertSelectedUtf16Range(in: view.textView, NSRange(location: 4, length: 0))

        view.textView.setMarkedText("n", selectedRange: NSRange(location: 1, length: 0))

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        view.textView.unmarkText()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the n</p>")
        XCTAssertEqual(view.textView.textStorage.string, "the n")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlurTimeAutocorrectAfterResignStillCommitsToRust() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlurTimeAutocorrectAfterNextMainQueueTurnStillCommitsToRust() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlurTimeAutocorrectAfterGracePeriodReconcilesInsteadOfCommitting() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>teh </p>")
        XCTAssertEqual(view.textView.textStorage.string, "teh ")
        XCTAssertEqual(view.textView.reconciliationCount, 1)
    }

    func testBlurTimeAutocorrectAfterContentReplacementReconcilesInsteadOfCommitting() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Remote</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Remote")
        XCTAssertEqual(view.textView.reconciliationCount, 1)
    }

    func testBlurTimeAutocorrectGraceWindowIsConsumedAfterCommit() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.reconciliationCount, 0)

        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "xxx"
        )
        flushMainQueue()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 1)
    }

    func testThemeRefreshDrainsPendingNativeAutocorrectBeforeApplyingRustState() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.textView.textStorage.string, "the ")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testSetEditableFalseDrainsPendingNativeAutocorrectBeforeReadOnly() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        _ = editorSetHtml(id: editorId, html: "<p>teh </p>")

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
        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "the ")
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testInputTraitChangesDrainPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesInputTraitChange {
            $0.setAutoCorrect(true)
        }
        assertPendingNativeAutocorrectSurvivesInputTraitChange {
            $0.setAutoCapitalize("characters")
        }
        assertPendingNativeAutocorrectSurvivesInputTraitChange {
            $0.setKeyboardType("email-address")
        }
    }

    func testInputTraitChangeFlushesActiveMarkedCompositionBeforeReload() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 6)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))

        view.textView.setKeyboardType("email-address")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello brave world")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlockedAutoCorrectRetryDoesNotOverrideNewerValue() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCorrect(true)
        view.textView.setAutoCorrect(false)
        flushMainQueue()

        XCTAssertEqual(view.textView.autocorrectionType, .no)
        XCTAssertEqual(view.textView.spellCheckingType, .no)
    }

    func testBlockedAutoCapitalizeRetryDoesNotOverrideNewerValue() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCapitalize("characters")
        view.textView.setAutoCapitalize("none")
        flushMainQueue()

        XCTAssertEqual(view.textView.autocapitalizationType, .none)
    }

    func testBlockedKeyboardTypeRetryDoesNotOverrideNewerValue() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setKeyboardType("email-address")
        view.textView.setKeyboardType("url")
        flushMainQueue()

        XCTAssertEqual(view.textView.keyboardType, .URL)
    }

    func testPendingAutoCorrectRetryIsInvalidatedAndDesiredTraitReplayedOnEditorRebind() {
        let firstEditorId = editorCreate(configJson: "{}")
        let secondEditorId = editorCreate(configJson: "{}")
        defer {
            editorDestroy(id: firstEditorId)
            editorDestroy(id: secondEditorId)
        }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = firstEditorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCorrect(true)
        view.editorId = secondEditorId
        flushMainQueue()

        XCTAssertEqual(view.textView.autocorrectionType, .yes)
        XCTAssertEqual(view.textView.spellCheckingType, .default)
    }

    func testPendingAutoCapitalizeRetryIsInvalidatedAndDesiredTraitReplayedOnEditorRebind() {
        let firstEditorId = editorCreate(configJson: "{}")
        let secondEditorId = editorCreate(configJson: "{}")
        defer {
            editorDestroy(id: firstEditorId)
            editorDestroy(id: secondEditorId)
        }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = firstEditorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCapitalize("characters")
        view.editorId = secondEditorId
        flushMainQueue()

        XCTAssertEqual(view.textView.autocapitalizationType, .allCharacters)
    }

    func testPendingKeyboardTypeRetryIsInvalidatedAndDesiredTraitReplayedOnEditorRebind() {
        let firstEditorId = editorCreate(configJson: "{}")
        let secondEditorId = editorCreate(configJson: "{}")
        defer {
            editorDestroy(id: firstEditorId)
            editorDestroy(id: secondEditorId)
        }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = firstEditorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setKeyboardType("email-address")
        view.editorId = secondEditorId
        flushMainQueue()

        XCTAssertEqual(view.textView.keyboardType, .emailAddress)
    }

    func testAccessoryToolbarPlacementDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange { view in
            view.setToolbarPlacement("inline")
        } verify: { view, _, file, line in
            XCTAssertTrue(view.isUsingAccessoryPlaceholderForTesting(), file: file, line: line)
            XCTAssertFalse(view.isUsingAccessoryToolbarForTesting(), file: file, line: line)
        }
    }

    func testAccessoryToolbarVisibilityDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange { view in
            view.setShowToolbar(false)
        } verify: { view, _, file, line in
            XCTAssertTrue(view.isUsingAccessoryPlaceholderForTesting(), file: file, line: line)
            XCTAssertFalse(view.isUsingAccessoryToolbarForTesting(), file: file, line: line)
        }
    }

    func testThemeAccessoryReloadDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange { view in
            view.setThemeJson(#"{"toolbar":{"appearance":"native"}}"#)
        }
    }

    func testBlockedThemeRetryIsClearedWhenDesiredThemeRevertsBeforeRetry() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        _ = editorSetHtml(id: editorId, html: "<p>Hello</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        let themeA = "{\"backgroundColor\":\"#101820\"}"
        let themeB = "{\"backgroundColor\":\"#ffeedd\"}"
        view.setEditorId(editorId)
        view.setThemeJson(themeA)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.setThemeJson(themeB)
        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))

        view.setThemeJson(themeA)
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))
        XCTAssertNotEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#ffeedd"))
    }

    func testBlockedThemeRetryAppliesDesiredThemeAfterEditorRebind() {
        let firstEditorId = editorCreate(configJson: "{}")
        let secondEditorId = editorCreate(configJson: "{}")
        defer {
            editorDestroy(id: firstEditorId)
            editorDestroy(id: secondEditorId)
        }
        _ = editorSetHtml(id: firstEditorId, html: "<p>First</p>")
        _ = editorSetHtml(id: secondEditorId, html: "<p>Second</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        let themeA = "{\"backgroundColor\":\"#101820\"}"
        let themeB = "{\"backgroundColor\":\"#ffeedd\"}"
        view.setEditorId(firstEditorId)
        view.setThemeJson(themeA)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setThemeJson(themeB)
        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))

        view.setEditorId(secondEditorId)
        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#ffeedd"))

        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#ffeedd"))
        XCTAssertNotEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))
    }

    func testMentionAddonRefreshDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange(
            initialHTML: "<p>teh @al</p>",
            selectionOffset: 7
        ) { view in
            view.setAddonsJson(self.aliceMentionAddonsJson())
        } verify: { view, _, file, line in
            XCTAssertNotNil(
                view.currentMentionQueryStateForTesting(trigger: "@"),
                file: file,
                line: line
            )
        }
    }

    func testMentionAddonClearDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange(
            initialHTML: "<p>teh @al</p>",
            selectionOffset: 7,
            configure: { view, _ in
                view.setAddonsJson(self.aliceMentionAddonsJson())
            }
        ) { view in
            view.setAddonsJson(nil)
        }
    }

    func testStaleMentionClearRetryDoesNotHideFreshSuggestionsAfterRefreshSucceeds() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }
        _ = editorSetHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.setAddonsJson(aliceMentionAddonsJson())
        XCTAssertTrue(view.isShowingMentionSuggestionsForTesting())

        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(nil)
        view.setAddonsJson(aliceMentionAddonsJson())

        XCTAssertTrue(
            view.isShowingMentionSuggestionsForTesting(),
            "successful mention refresh should show suggestions before the stale clear retry runs"
        )

        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(
            view.isShowingMentionSuggestionsForTesting(),
            "stale clear retry should not hide suggestions from a later successful refresh"
        )
    }

    func testAccessoryRetryBatchKeepsNonConflictingToolbarVisibilityActionAfterMentionClear() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }
        _ = editorSetHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        view.setToolbarPlacement("keyboard")
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.setAddonsJson(aliceMentionAddonsJson())
        XCTAssertTrue(view.isUsingAccessoryToolbarForTesting())

        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(nil)
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setShowToolbar(false)

        XCTAssertTrue(
            view.isUsingAccessoryToolbarForTesting(),
            "toolbar visibility should remain unchanged while the accessory update is queued"
        )

        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(
            view.isUsingAccessoryPlaceholderForTesting(),
            "successful mention clear retry should not cancel a queued toolbar visibility retry"
        )
        XCTAssertFalse(view.isUsingAccessoryToolbarForTesting())
    }

    func testAccessoryRetryBatchKeepsRemainingActionsWhenFirstRetryRequeues() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }
        _ = editorSetHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        view.setToolbarPlacement("keyboard")
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.setAddonsJson(aliceMentionAddonsJson())
        XCTAssertTrue(view.isShowingMentionSuggestionsForTesting())
        XCTAssertTrue(view.isUsingAccessoryToolbarForTesting())

        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(nil)
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(aliceMentionAddonsJson())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setShowToolbar(false)

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(
            view.isShowingMentionSuggestionsForTesting(),
            "a refresh queued behind a requeued clear should still run"
        )
        XCTAssertTrue(
            view.isUsingAccessoryPlaceholderForTesting(),
            "toolbar visibility queued behind a requeued clear should still run"
        )
        XCTAssertFalse(view.isUsingAccessoryToolbarForTesting())
    }

    func testApplyEditorUpdateRetriesAfterBlockedCompositionOnSameEditor() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        _ = editorSetHtml(id: editorId, html: "<p>First</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 0)

        let updateJSON = editorReplaceHtml(id: editorId, html: "<p>Remote</p>")
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        XCTAssertFalse(view.applyEditorUpdate(updateJSON))
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "First")

        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Remote")
    }

    func testApplyEditorUpdateRetryIsDroppedAfterEditorRebind() {
        let firstEditorId = editorCreate(configJson: "{}")
        let secondEditorId = editorCreate(configJson: "{}")
        defer {
            editorDestroy(id: firstEditorId)
            editorDestroy(id: secondEditorId)
        }
        _ = editorSetHtml(id: firstEditorId, html: "<p>First</p>")
        _ = editorSetHtml(id: secondEditorId, html: "<p>Second</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(firstEditorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 0)

        let staleUpdateJSON = editorReplaceHtml(id: firstEditorId, html: "<p>Remote</p>")
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        XCTAssertFalse(view.applyEditorUpdate(staleUpdateJSON))
        view.setEditorId(secondEditorId)
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
    }

    func testSameEditorIdUpdateDoesNotDropPendingNativeAutocorrect() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        _ = editorSetHtml(id: editorId, html: "<p>teh </p>")

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

        view.setEditorId(editorId)
        flushMainQueue()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "the ")
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testPendingNativeAutocorrectIsDroppedAfterEditorRebind() {
        let firstEditorId = editorCreate(configJson: "{}")
        let secondEditorId = editorCreate(configJson: "{}")
        defer {
            editorDestroy(id: firstEditorId)
            editorDestroy(id: secondEditorId)
        }
        _ = editorSetHtml(id: firstEditorId, html: "<p>teh </p>")
        _ = editorSetHtml(id: secondEditorId, html: "<p>Second</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(firstEditorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.setEditorId(secondEditorId)
        flushMainQueue()

        XCTAssertEqual(editorGetHtml(id: firstEditorId), "<p>teh </p>")
        XCTAssertEqual(editorGetHtml(id: secondEditorId), "<p>Second</p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
    }

    func testPrepareForCommandAfterEditorRebindDoesNotDrainPreviousEditorMutation() {
        let firstEditorId = editorCreate(configJson: "{}")
        let secondEditorId = editorCreate(configJson: "{}")
        defer {
            editorDestroy(id: firstEditorId)
            editorDestroy(id: secondEditorId)
        }
        _ = editorSetHtml(id: firstEditorId, html: "<p>teh </p>")
        _ = editorSetHtml(id: secondEditorId, html: "<p>Second</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            NativeEditorViewRegistry.shared.unregister(editorId: secondEditorId, view: view)
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(firstEditorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        view.setEditorId(secondEditorId)

        let preparationJSON = NativeEditorViewRegistry.shared.prepareForCommandJSON(
            editorId: firstEditorId
        )
        XCTAssertTrue(preparationJSON.contains("\"ready\":true"))
        flushMainQueue()

        XCTAssertEqual(editorGetHtml(id: firstEditorId), "<p>teh </p>")
        XCTAssertEqual(editorGetHtml(id: secondEditorId), "<p>Second</p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
    }

    func testDestroyedEditorInvalidatesRegistryAndUnbindsView() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        XCTAssertEqual(view.richTextView.editorId, editorId)
        XCTAssertEqual(view.richTextView.textView.editorId, editorId)

        NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: editorId)
        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )

        XCTAssertEqual(preparation["ready"] as? Bool, false)
        XCTAssertEqual(preparation["blockedReason"] as? String, "destroyed")
        XCTAssertEqual(view.richTextView.editorId, 0)
        XCTAssertEqual(view.richTextView.textView.editorId, 0)
    }

    func testDestroyedEditorIdCannotRegisterNewView() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)
        NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: editorId)

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )

        XCTAssertEqual(view.richTextView.editorId, 0)
        XCTAssertEqual(view.richTextView.textView.editorId, 0)
        XCTAssertEqual(preparation["ready"] as? Bool, false)
        XCTAssertEqual(preparation["blockedReason"] as? String, "destroyed")
    }

    func testPrepareForCommandReportsCompositionBlockedReasonWhenMarkedTextPreflightDefers() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        _ = editorSetHtml(id: editorId, html: "<p>Hello</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: view)
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 0)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )

        XCTAssertEqual(preparation["ready"] as? Bool, false)
        XCTAssertEqual(preparation["blockedReason"] as? String, "composition")
        XCTAssertNil(preparation["updateJSON"])
    }

    func testPrepareForCommandIncludesUpdateJSONAfterNativeAutocorrectDrain() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        _ = editorSetHtml(id: editorId, html: "<p>teh </p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: view)
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )
        let updateJSON = preparation["updateJSON"] as? String

        XCTAssertEqual(preparation["ready"] as? Bool, true)
        XCTAssertNil(preparation["blockedReason"])
        XCTAssertNotNil(updateJSON)
        XCTAssertTrue(updateJSON?.contains("the ") == true, "preflight update should include the drained correction")
        XCTAssertFalse(updateJSON?.contains("teh ") == true, "preflight update should not contain stale text")
        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p>")
    }

    func testPrepareForCommandIncludesUpdateJSONAfterSameTextCompositionChangesSelectionState() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        editorSetSelectionScalar(id: editorId, scalarAnchor: 0, scalarHead: 5)
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 5))

        textView.setMarkedText("Hello", selectedRange: NSRange(location: 5, length: 0))
        let preparation = textView.prepareForExternalEditorCommand()

        XCTAssertTrue(preparation.ready)
        XCTAssertNil(preparation.blockedReason)
        XCTAssertNotNil(
            preparation.updateJSON,
            "same-text composition commits should still forward selection/state changes"
        )
        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello world")
    }

    func testMarkedTextDoesNotReconcileWhileCompositionIsTransient() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))

        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
        XCTAssertEqual(textView.reconciliationCount, 0)
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<p>Hello world</p>",
            "marked text should stay visible-only until the IME commits it"
        )
    }

    func testUnmarkTextCommitsAtOriginalAuthorizedOffset() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))
        textView.unmarkText()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
    }

    func testUnmarkTextReplacesOriginalAuthorizedSelection() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 6, length: 5))

        textView.setMarkedText("there", selectedRange: NSRange(location: 5, length: 0))
        textView.unmarkText()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello there</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello there")
    }

    func testSetMarkedTextNilCommitsVisibleComposition() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))
        textView.setMarkedText(nil, selectedRange: NSRange(location: 0, length: 0))

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
        XCTAssertEqual(textView.authorizedTextForTesting(), "Hello brave world")
    }

    func testSetMarkedTextNilCommitsEmptyReplacementOverOriginalSelection() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 6, length: 5))

        textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        textView.setMarkedText(nil, selectedRange: NSRange(location: 0, length: 0))

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello </p>")
        XCTAssertEqual(textView.textStorage.string, "Hello ")
        XCTAssertEqual(textView.authorizedTextForTesting(), "Hello ")
    }

    func testExternalUpdatePreflightCommitsActiveCompositionOnce() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))

        XCTAssertTrue(textView.applyTheme(EditorTheme(dictionary: ["textColor": "#123456"])))
        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")

        textView.unmarkText()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
    }

    func testToolbarCommandsCommitActiveMarkedCompositionBeforeMutatingEditor() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))
        textView.performToolbarToggleMark("bold")

        XCTAssertTrue(
            editorGetHtml(id: editorId).contains("Hello brave world"),
            "toolbar mark command should commit the active composition before mutating the editor"
        )
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
        XCTAssertEqual(textView.reconciliationCount, 0)

        setCollapsedSelection(in: textView, utf16Offset: textView.textStorage.length)
        textView.setMarkedText("!", selectedRange: NSRange(location: 1, length: 0))
        textView.performToolbarInsertNode("horizontalRule")

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(html.contains("Hello brave world"), "toolbar node insert should preserve the earlier composed text, got: \(html)")
        XCTAssertTrue(html.contains("!"), "toolbar node insert should preserve the newly composed text, got: \(html)")
        XCTAssertTrue(html.contains("<hr>"), "toolbar node insert should still apply after the composition drain, got: \(html)")
        XCTAssertEqual(textView.reconciliationCount, 0)
    }

    func testExternalUpdatePreflightCommitsEmptySelectedCompositionAsDeletion() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 6, length: 5))

        textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        XCTAssertTrue(textView.applyTheme(EditorTheme(dictionary: ["textColor": "#123456"])))
        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello </p>")
        XCTAssertEqual(textView.textStorage.string, "Hello ")

        textView.unmarkText()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello </p>")
        XCTAssertEqual(textView.textStorage.string, "Hello ")
    }

    func testInsertTextDuringMarkedCompositionUsesOriginalReplacementRange() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("brav", selectedRange: NSRange(location: 4, length: 0))
        textView.insertText("brave ")

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello brave world")
    }

    func testUpdatedMarkedTextStillUsesOriginalAuthorizedOffset() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("abc ", selectedRange: NSRange(location: 3, length: 0))
        textView.setMarkedText("ab ", selectedRange: NSRange(location: 3, length: 0))

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello world</p>")

        textView.unmarkText()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello ab world</p>")
    }

    func testDeleteBackwardDuringMarkedCompositionDoesNotMutateRust() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        setCollapsedSelection(in: textView, utf16Offset: 6)

        textView.setMarkedText("abc ", selectedRange: NSRange(location: 3, length: 0))
        textView.deleteBackward()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello world</p>")

        textView.unmarkText()

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>Hello world</p>")
    }

    func testAdjustedCaretRectUsesBaselineAndFontMetrics() {
        let font = UIFont.systemFont(ofSize: 16)
        let adjusted = EditorTextView.adjustedCaretRect(
            from: CGRect(x: 12, y: 20, width: 2, height: 32),
            baselineY: 36.140625,
            font: font,
            screenScale: 2
        )
        let expectedHeight = ceil(font.lineHeight * 2) / 2
        let typographicHeight = font.ascender - font.descender
        let leading = max(font.lineHeight - typographicHeight, 0)
        let expectedY = ((36.140625 - font.ascender - (leading / 2.0)) * 2).rounded() / 2

        XCTAssertEqual(adjusted.origin.x, 12, accuracy: 0.1)
        XCTAssertEqual(adjusted.origin.y, expectedY, accuracy: 0.1)
        XCTAssertEqual(adjusted.size.height, expectedHeight, accuracy: 0.1)
    }

    func testAdjustedCaretRectCentersWithinTallerLineFragment() {
        let adjusted = EditorTextView.adjustedCaretRect(
            from: CGRect(x: 12, y: 20, width: 2, height: 32),
            targetHeight: 19,
            screenScale: 2
        )

        XCTAssertEqual(adjusted.origin.x, 12, accuracy: 0.1)
        XCTAssertEqual(adjusted.origin.y, 26.5, accuracy: 0.1)
        XCTAssertEqual(adjusted.size.height, 19, accuracy: 0.1)
    }

    func testRichTextEditorViewAutoGrowDisablesInternalScrolling() {
        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 0))

        view.heightBehavior = .autoGrow

        XCTAssertFalse(
            view.textView.isScrollEnabled,
            "autoGrow mode should disable internal editor scrolling"
        )
    }

    func testRichTextEditorViewAutoGrowReportsIntrinsicHeightFromContent() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 0))
        view.heightBehavior = .autoGrow
        view.editorId = editorId
        view.setContent(html: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")
        view.layoutIfNeeded()

        let intrinsic = view.intrinsicContentSize

        XCTAssertEqual(intrinsic.width, UIView.noIntrinsicMetric, accuracy: 0.1)
        XCTAssertGreaterThan(intrinsic.height, 0)
    }

    func testApplyThemeRerendersExistingContentWhenTextIsUnchanged() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        let theme = EditorTheme(dictionary: [
            "text": [
                "fontFamily": "Courier",
                "fontSize": 21,
                "color": "#224466",
            ],
            "paragraph": [
                "lineHeight": 30,
            ],
        ])

        textView.applyTheme(theme)

        let attrs = textView.textStorage.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        let color = attrs[.foregroundColor] as? UIColor
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(font?.pointSize ?? 0, 21, accuracy: 0.1)
        XCTAssertEqual(color, EditorTheme.color(from: "#224466"))
        XCTAssertEqual(paragraphStyle?.minimumLineHeight ?? 0, 30, accuracy: 0.1)
    }

    func testEditorTextViewMeasuredAutoGrowHeightMatchesSizeThatFits() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 0))
        textView.heightBehavior = .autoGrow
        textView.bindEditor(
            id: editorId,
            initialHTML: "<p>Alpha</p><p>Beta<br></p><p>Gamma</p>"
        )
        textView.layoutIfNeeded()

        let measuredHeight = textView.measuredAutoGrowHeightForTesting(width: 320)
        let fittedHeight = ceil(
            textView.sizeThatFits(
                CGSize(width: 320, height: CGFloat.greatestFiniteMagnitude)
            ).height
        )

        XCTAssertEqual(measuredHeight, fittedHeight, accuracy: 1.0)
    }

    func testRichTextEditorViewAutoGrowHeightAfterParagraphSplitMatchesSizeThatFits() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 0))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.heightBehavior = .autoGrow
        view.editorId = editorId
        view.setContent(html: """
        <p>Alpha beta gamma delta epsilon zeta eta theta iota.</p>
        <p>Kappa lambda mu nu xi omicron pi rho sigma.</p>
        <p>Tau upsilon phi chi psi omega.</p>
        """)
        view.layoutIfNeeded()

        let splitOffset = ((view.textView.text as NSString).range(of: "sigma")).location + 5
        setSelection(in: view.textView, utf16Range: NSRange(location: splitOffset, length: 0))

        view.textView.insertText("\n")
        flushMainQueue()
        view.layoutIfNeeded()

        let intrinsicHeight = view.intrinsicContentSize.height
        let fittedHeight = ceil(
            view.textView.sizeThatFits(
                CGSize(width: 320, height: CGFloat.greatestFiniteMagnitude)
            ).height
        )

        XCTAssertEqual(intrinsicHeight, fittedHeight, accuracy: 1.0)
    }

    func testRichTextEditorViewAutoGrowIntrinsicHeightGrowsWhenHostAppliesMeasuredHeight() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 0))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.heightBehavior = .autoGrow
        view.editorId = editorId
        view.setContent(html: "<p>Alpha</p>")
        view.layoutIfNeeded()

        var measuredHeight = ceil(view.intrinsicContentSize.height)
        XCTAssertGreaterThan(measuredHeight, 0)

        view.frame.size.height = measuredHeight
        view.layoutIfNeeded()

        let endOffset = (view.textView.text as NSString).length
        setSelection(in: view.textView, utf16Range: NSRange(location: endOffset, length: 0))

        view.textView.insertText("\n")
        view.textView.insertText("Beta beta beta beta beta beta beta beta beta beta beta beta.")
        flushMainQueue()
        view.layoutIfNeeded()

        let grownHeight = ceil(view.intrinsicContentSize.height)

        XCTAssertGreaterThan(
            grownHeight,
            measuredHeight,
            "autoGrow should still expand when the host view applies the previously measured height"
        )
    }

    func testRichTextEditorViewAutoGrowIntrinsicHeightShrinksAfterDeletingContent() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 0))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.heightBehavior = .autoGrow
        view.editorId = editorId
        view.setContent(html: "<p>Alpha</p>")
        view.layoutIfNeeded()

        let baseHeight = ceil(view.intrinsicContentSize.height)
        XCTAssertGreaterThan(baseHeight, 0)

        view.frame.size.height = baseHeight
        view.layoutIfNeeded()

        let endOffset = (view.textView.text as NSString).length
        setSelection(in: view.textView, utf16Range: NSRange(location: endOffset, length: 0))

        let insertedSuffix = " beta beta beta beta beta beta beta beta beta beta beta beta."
        view.textView.insertText(insertedSuffix)
        flushMainQueue()
        view.layoutIfNeeded()

        let grownHeight = ceil(view.intrinsicContentSize.height)
        XCTAssertGreaterThan(grownHeight, baseHeight)

        view.frame.size.height = grownHeight
        view.layoutIfNeeded()

        let insertedTextRange = (view.textView.text as NSString).range(of: insertedSuffix)
        XCTAssertNotEqual(insertedTextRange.location, NSNotFound)
        setSelection(in: view.textView, utf16Range: insertedTextRange)
        view.textView.deleteBackward()
        flushMainQueue()
        view.layoutIfNeeded()

        let shrunkHeight = ceil(view.intrinsicContentSize.height)

        XCTAssertLessThan(
            shrunkHeight,
            grownHeight,
            "autoGrow should shrink again after deleting content from a host-sized editor"
        )
        XCTAssertEqual(shrunkHeight, baseHeight, accuracy: 1.0)
    }

    func testCaretRectInTallLineHeightListItemUsesResolvedGlyphBaseline() {
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
            "list": [
                "markerScale": 2,
            ],
        ])
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Bullet item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """

        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label,
            theme: theme
        )

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        let plainTextView = UITextView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        textView.attributedText = attributed
        plainTextView.attributedText = attributed
        textView.layoutIfNeeded()
        plainTextView.layoutIfNeeded()

        let position = textView.position(from: textView.beginningOfDocument, offset: 0)
        let plainPosition = plainTextView.position(from: plainTextView.beginningOfDocument, offset: 0)
        XCTAssertNotNil(position)
        XCTAssertNotNil(plainPosition)

        guard let caretPosition = position, let plainCaretPosition = plainPosition else { return }
        let caretRect = textView.caretRect(for: caretPosition)
        let plainCaretRect = plainTextView.caretRect(for: plainCaretPosition)
        let expected = expectedCaretRect(
            in: plainTextView,
            offset: 0,
            referenceRect: plainCaretRect,
            font: UIFont.systemFont(ofSize: 16)
        )

        XCTAssertEqual(caretRect.origin.y, expected.origin.y, accuracy: 1.0)
        XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0)
    }

    func testCaretRectUsesResolvedGlyphBaselineAcrossWrappedParagraphLines() {
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
        ])
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "This is a wrapped paragraph for caret alignment checks across multiple lines.", "marks": []},
            {"type": "blockEnd"}
        ]
        """

        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label,
            theme: theme
        )

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 120, height: 240))
        let plainTextView = UITextView(frame: CGRect(x: 0, y: 0, width: 120, height: 240))
        textView.attributedText = attributed
        plainTextView.attributedText = attributed
        textView.layoutIfNeeded()
        plainTextView.layoutIfNeeded()

        let offsets = [0, 20, attributed.length - 1]
        for offset in offsets {
            guard let position = textView.position(from: textView.beginningOfDocument, offset: offset) else {
                XCTFail("expected position for offset \(offset)")
                continue
            }
            guard let plainPosition = plainTextView.position(from: plainTextView.beginningOfDocument, offset: offset) else {
                XCTFail("expected plain position for offset \(offset)")
                continue
            }

            let caretRect = textView.caretRect(for: position)
            let plainCaretRect = plainTextView.caretRect(for: plainPosition)
            let expected = expectedCaretRect(
                in: plainTextView,
                offset: offset,
                referenceRect: plainCaretRect,
                font: UIFont.systemFont(ofSize: 16)
            )

            XCTAssertEqual(caretRect.origin.y, expected.origin.y, accuracy: 1.0, "offset \(offset)")
            XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0, "offset \(offset)")
        }
    }

    func testCaretRectUsesCorrectVisualLineAtWrappedParagraphBoundaries() {
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
        ])
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "This is a wrapped paragraph for caret alignment checks across multiple lines.", "marks": []},
            {"type": "blockEnd"}
        ]
        """

        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label,
            theme: theme
        )

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 120, height: 240))
        let plainTextView = UITextView(frame: CGRect(x: 0, y: 0, width: 120, height: 240))
        textView.attributedText = attributed
        plainTextView.attributedText = attributed
        textView.layoutIfNeeded()
        plainTextView.layoutIfNeeded()

        let offsets = [0, 20, attributed.length - 1]
        for offset in offsets {
            guard let position = textView.position(from: textView.beginningOfDocument, offset: offset) else {
                XCTFail("expected position for offset \(offset)")
                continue
            }
            guard let plainPosition = plainTextView.position(from: plainTextView.beginningOfDocument, offset: offset) else {
                XCTFail("expected plain position for offset \(offset)")
                continue
            }

            let caretRect = textView.caretRect(for: position)
            let plainCaretRect = plainTextView.caretRect(for: plainPosition)
            let expected = expectedCaretRect(
                in: plainTextView,
                offset: offset,
                referenceRect: plainCaretRect,
                font: UIFont.systemFont(ofSize: 16)
            )

            XCTAssertEqual(caretRect.origin.y, expected.origin.y, accuracy: 1.0, "offset \(offset)")
        }
    }

    func testCaretRectAfterBlockquoteMatchesPlainTextViewHorizontalPosition() {
        let attributed = RenderBridge.renderElements(
            fromJSON: """
            [
                {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
                {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
                {"type": "textRun", "text": "Hello", "marks": []},
                {"type": "blockEnd"},
                {"type": "blockEnd"},
                {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
                {"type": "textRun", "text": "World", "marks": []},
                {"type": "blockEnd"}
            ]
            """,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label,
            theme: EditorTheme(dictionary: [
                "blockquote": [
                    "indent": 20,
                    "borderWidth": 4,
                    "markerGap": 10,
                ],
            ])
        )

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        let plainTextView = UITextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.attributedText = attributed
        plainTextView.attributedText = attributed
        textView.layoutIfNeeded()
        plainTextView.layoutIfNeeded()

        let offset = (attributed.string as NSString).range(of: "World").location + 4
        guard let position = textView.position(from: textView.beginningOfDocument, offset: offset) else {
            XCTFail("expected editor caret position after blockquote")
            return
        }
        guard let plainPosition = plainTextView.position(from: plainTextView.beginningOfDocument, offset: offset) else {
            XCTFail("expected plain caret position after blockquote")
            return
        }

        let caretRect = textView.caretRect(for: position)
        let plainCaretRect = plainTextView.caretRect(for: plainPosition)
        let expected = expectedCaretRect(
            in: plainTextView,
            offset: offset,
            referenceRect: plainCaretRect,
            font: UIFont.systemFont(ofSize: 16)
        )

        XCTAssertEqual(caretRect.minX, expected.minX, accuracy: 1.0)
        XCTAssertEqual(caretRect.minY, expected.minY, accuracy: 1.0)
        XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0)
    }

    func testCaretRectAfterBlockquoteAlignsToNextCharacterEdge() {
        let attributed = RenderBridge.renderElements(
            fromJSON: """
            [
                {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
                {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
                {"type": "textRun", "text": "Hello", "marks": []},
                {"type": "blockEnd"},
                {"type": "blockEnd"},
                {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
                {"type": "textRun", "text": "World", "marks": []},
                {"type": "blockEnd"}
            ]
            """,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label,
            theme: EditorTheme(dictionary: [
                "blockquote": [
                    "indent": 20,
                    "borderWidth": 4,
                    "markerGap": 10,
                ],
            ])
        )

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let offset = (attributed.string as NSString).range(of: "World").location + 4
        guard let position = textView.position(from: textView.beginningOfDocument, offset: offset),
              let nextPosition = textView.position(from: position, offset: 1),
              let range = textView.textRange(from: position, to: nextPosition)
        else {
            XCTFail("expected caret and next character positions after blockquote")
            return
        }

        let expectedX = textView.selectionRects(for: range)
            .map(\.rect)
            .first(where: { !$0.isEmpty && $0.width > 0 })?.minX
        XCTAssertNotNil(expectedX)

        let caretRect = textView.caretRect(for: position)
        XCTAssertEqual(caretRect.minX, expectedX ?? caretRect.minX, accuracy: 1.0)
    }

    func testBoundEditorCaretRectAfterBlockquoteMatchesPlainTextViewHorizontalPosition() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.bindEditor(id: editorId, initialHTML: "<blockquote><p>Hello</p></blockquote><p>World</p>")

        editorSetSelectionScalar(id: editorId, scalarAnchor: 10, scalarHead: 10)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        let plainTextView = UITextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        plainTextView.attributedText = textView.attributedText
        plainTextView.layoutIfNeeded()

        let offset = textView.offset(
            from: textView.beginningOfDocument,
            to: textView.selectedTextRange?.start ?? textView.endOfDocument
        )

        guard let position = textView.position(from: textView.beginningOfDocument, offset: offset) else {
            XCTFail("expected editor caret position after blockquote in bound editor")
            return
        }
        guard let plainPosition = plainTextView.position(from: plainTextView.beginningOfDocument, offset: offset) else {
            XCTFail("expected plain caret position after blockquote in bound editor")
            return
        }

        let caretRect = textView.caretRect(for: position)
        let plainCaretRect = plainTextView.caretRect(for: plainPosition)
        let expected = expectedCaretRect(
            in: plainTextView,
            offset: offset,
            referenceRect: plainCaretRect,
            font: UIFont.systemFont(ofSize: 16)
        )

        XCTAssertEqual(caretRect.minX, expected.minX, accuracy: 1.0)
        XCTAssertEqual(caretRect.minY, expected.minY, accuracy: 1.0)
        XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0)
    }

    func testTypingAtParagraphEndAfterBlockquoteKeepsCaretAtRenderedEnd() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.bindEditor(id: editorId, initialHTML: "<blockquote><p>Hello</p></blockquote><p>World</p>")

        editorSetSelectionScalar(id: editorId, scalarAnchor: 11, scalarHead: 11)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.insertText("!")
        textView.layoutIfNeeded()

        let html = editorGetHtml(id: editorId)
        XCTAssertEqual(html, "<blockquote><p>Hello</p></blockquote><p>World!</p>")

        let caretOffset = textView.offset(
            from: textView.beginningOfDocument,
            to: textView.selectedTextRange?.start ?? textView.endOfDocument
        )
        XCTAssertEqual(caretOffset, textView.text.count, "logical selection should remain at rendered end")

        let plainTextView = UITextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        plainTextView.attributedText = textView.attributedText
        plainTextView.layoutIfNeeded()

        guard let position = textView.position(from: textView.beginningOfDocument, offset: caretOffset),
              let plainPosition = plainTextView.position(from: plainTextView.beginningOfDocument, offset: caretOffset)
        else {
            XCTFail("expected caret positions after typing at paragraph end")
            return
        }

        let caretRect = textView.caretRect(for: position)
        let plainCaretRect = plainTextView.caretRect(for: plainPosition)
        let expected = expectedCaretRect(
            in: plainTextView,
            offset: caretOffset,
            referenceRect: plainCaretRect,
            font: UIFont.systemFont(ofSize: 16)
        )

        XCTAssertEqual(caretRect.minX, expected.minX, accuracy: 1.0)
        XCTAssertEqual(caretRect.minY, expected.minY, accuracy: 1.0)
        XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0)
    }

    func testBlockquoteStripeRectStaysStableAcrossReturnDrivenLayoutPasses() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<blockquote><p>Hello</p></blockquote>")
        textView.layoutIfNeeded()

        editorSetSelectionScalar(id: editorId, scalarAnchor: 6, scalarHead: 6)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.insertText("\n")

        let firstPassStripeRects = textView.blockquoteStripeRectsForTesting()
        textView.layoutIfNeeded()
        let secondPassStripeRects = textView.blockquoteStripeRectsForTesting()
        RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        textView.layoutIfNeeded()
        let settledStripeRects = textView.blockquoteStripeRectsForTesting()

        XCTAssertFalse(firstPassStripeRects.isEmpty, "expected blockquote stripe after inserting quoted paragraph")
        XCTAssertEqual(firstPassStripeRects.count, secondPassStripeRects.count)
        XCTAssertEqual(secondPassStripeRects.count, settledStripeRects.count)

        for (first, second) in zip(firstPassStripeRects, secondPassStripeRects) {
            XCTAssertEqual(first.minX, second.minX, accuracy: 0.5)
            XCTAssertEqual(first.minY, second.minY, accuracy: 0.5)
            XCTAssertEqual(first.height, second.height, accuracy: 0.5)
        }

        for (first, settled) in zip(firstPassStripeRects, settledStripeRects) {
            XCTAssertEqual(first.minX, settled.minX, accuracy: 0.5)
            XCTAssertEqual(first.minY, settled.minY, accuracy: 0.5)
            XCTAssertEqual(first.height, settled.height, accuracy: 0.5)
        }
    }

    func testConsecutiveBlockquoteParagraphsShareOneStripeGroup() {
        let attributed = RenderBridge.renderElements(
            fromJSON: """
            [
                {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
                {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
                {"type": "textRun", "text": "Hello", "marks": []},
                {"type": "blockEnd"},
                {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
                {"type": "textRun", "text": "World", "marks": []},
                {"type": "blockEnd"},
                {"type": "blockEnd"}
            ]
            """,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 220))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let stripeRects = textView.blockquoteStripeRectsForTesting()
        XCTAssertEqual(stripeRects.count, 1, "consecutive quoted paragraphs should render one continuous stripe group")
    }

    func testConsecutiveBlockquoteParagraphsAfterPlainParagraphStillShareOneStripeGroup() {
        let attributed = RenderBridge.renderElements(
            fromJSON: """
            [
                {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
                {"type": "textRun", "text": "Intro", "marks": []},
                {"type": "blockEnd"},
                {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
                {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
                {"type": "textRun", "text": "Hello", "marks": []},
                {"type": "blockEnd"},
                {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
                {"type": "textRun", "text": "World", "marks": []},
                {"type": "blockEnd"},
                {"type": "blockEnd"}
            ]
            """,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 220))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let stripeRects = textView.blockquoteStripeRectsForTesting()
        XCTAssertEqual(
            stripeRects.count,
            1,
            "quoted paragraphs should still share one stripe group when the quote follows plain content"
        )
        XCTAssertGreaterThan(
            stripeRects[0].minY,
            0.5,
            "quote stripe should not extend into the preceding plain paragraph"
        )
        XCTAssertLessThan(
            stripeRects[0].height,
            60.0,
            "quote stripe should not extend through trailing paragraph spacing below the quote"
        )
    }

    func testBlockquoteStripeDrawPassStaysStableAfterReturn() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<blockquote><p>Hello</p></blockquote>")
        textView.layoutIfNeeded()

        editorSetSelectionScalar(id: editorId, scalarAnchor: 6, scalarHead: 6)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.resetBlockquoteStripeDrawPassesForTesting()
        textView.insertText("\n")
        forceDraw(textView)
        let firstRenderedPasses = textView.blockquoteStripeDrawPassesForTesting()

        RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        textView.layoutIfNeeded()
        forceDraw(textView)
        let allRenderedPasses = textView.blockquoteStripeDrawPassesForTesting()

        guard let firstPass = firstRenderedPasses.first,
              let settledPass = allRenderedPasses.last
        else {
            XCTFail("expected recorded blockquote stripe draw passes")
            return
        }

        XCTAssertEqual(firstPass.count, settledPass.count)
        for (first, settled) in zip(firstPass, settledPass) {
            XCTAssertEqual(first.minX, settled.minX, accuracy: 0.5)
            XCTAssertEqual(first.minY, settled.minY, accuracy: 0.5)
            XCTAssertEqual(first.height, settled.height, accuracy: 0.5)
        }
    }

    /// A single code block spanning multiple visual lines (intra-block hard
    /// breaks carrying codeBlockBackgroundColor) must be filled with exactly
    /// one rect per draw pass — one fill for the whole block, not one per
    /// paragraph. Regression coverage for the group-start dedupe key in
    /// EditorLayoutManager.drawCodeBlockBackgrounds.
    func testCodeBlockBackgroundDrawPassDedupesMultiParagraphBlock() {
        let attributed = RenderBridge.renderElements(
            fromJSON: """
            [
                {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
                {"type": "textRun", "text": "line1", "marks": []},
                {"type": "voidInline", "nodeType": "hardBreak", "docPos": 5},
                {"type": "textRun", "text": "line2", "marks": []},
                {"type": "voidInline", "nodeType": "hardBreak", "docPos": 11},
                {"type": "textRun", "text": "line3", "marks": []},
                {"type": "blockEnd"}
            ]
            """,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 220))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        textView.resetCodeBlockDrawPassesForTesting()
        forceDraw(textView)
        let passes = textView.codeBlockDrawPassesForTesting()

        guard let firstPass = passes.first else {
            XCTFail("expected at least one recorded code-block draw pass")
            return
        }

        XCTAssertEqual(
            firstPass.count,
            1,
            """
            a code block spanning 3 paragraphs must be filled with exactly one \
            rect per draw pass, not one per paragraph. Got \(firstPass.count) \
            rects: \(firstPass)
            """
        )
    }

    // MARK: - Task List Marker Hit Testing
    //
    // taskListMarkerParagraphStart (EditorLayoutManager) used to enumerate
    // listMarkerContext over the WHOLE document, with per-item TextKit
    // queries, on every touch (it backs
    // TaskListMarkerTapOverlayView.point(inside:)). These tests pin its
    // hit/miss/hard-break contract before inverting it to resolve the
    // touched line first via glyphIndex(for:in:).

    private func taskListJSON(items: [(text: String, checked: Bool)]) -> String {
        let total = items.count
        var elements: [String] = []
        for (index, item) in items.enumerated() {
            elements.append("""
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": \(index + 1), "total": \(total), \
            "start": 1, "isFirst": \(index == 0), "isLast": \(index == total - 1), \
            "kind": "task", "checked": \(item.checked)}}
            """)
            elements.append(#"{"type": "blockStart", "nodeType": "paragraph", "depth": 2}"#)
            elements.append(#"{"type": "textRun", "text": "\#(item.text)", "marks": []}"#)
            elements.append(#"{"type": "blockEnd"}"#)
            elements.append(#"{"type": "blockEnd"}"#)
        }
        return "[\n" + elements.joined(separator: ",\n") + "\n]"
    }

    private func taskListMarkerOrigin(for textView: EditorTextView) -> CGPoint {
        CGPoint(
            x: textView.textContainerInset.left - textView.contentOffset.x,
            y: textView.textContainerInset.top - textView.contentOffset.y
        )
    }

    /// Reproduces the exact marker-rect math taskListMarkerParagraphStart
    /// applies (before the `insetBy(dx: -10, dy: -8)` tap-slop expansion),
    /// so tests can derive precise probe points instead of guessing pixels.
    private func taskMarkerTightRect(forCharacterIndex characterIndex: Int, in textView: EditorTextView) -> CGRect {
        guard let layoutManager = textView.layoutManager as? EditorLayoutManager else {
            XCTFail("EditorTextView must be backed by EditorLayoutManager")
            return .zero
        }
        let textStorage = textView.textStorage
        let origin = taskListMarkerOrigin(for: textView)
        let glyphIndex = layoutManager.glyphIndexForCharacter(at: characterIndex)
        let attrs = textStorage.attributes(at: characterIndex, effectiveRange: nil)
        let baseFont = EditorLayoutManager.markerBaseFont(from: attrs)
        let markerWidth = (attrs[RenderBridgeAttributes.listMarkerWidth] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? LayoutConstants.listMarkerWidth

        var lineGlyphRange = NSRange()
        let usedRect = layoutManager.lineFragmentUsedRect(forGlyphAt: glyphIndex, effectiveRange: &lineGlyphRange)
        let lineFragmentRect = layoutManager.lineFragmentRect(forGlyphAt: glyphIndex, effectiveRange: nil)
        let glyphLocation = layoutManager.location(forGlyphAt: glyphIndex)
        let baselineY = lineFragmentRect.minY + glyphLocation.y

        return EditorLayoutManager.taskMarkerDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: markerWidth,
            baselineY: baselineY,
            baseFont: baseFont,
            origin: origin
        )
    }

    private func taskListLineFragmentRect(forCharacterIndex characterIndex: Int, in textView: EditorTextView) -> CGRect {
        let layoutManager = textView.layoutManager
        let origin = taskListMarkerOrigin(for: textView)
        let glyphIndex = layoutManager.glyphIndexForCharacter(at: characterIndex)
        var rect = layoutManager.lineFragmentRect(forGlyphAt: glyphIndex, effectiveRange: nil)
        rect.origin.x += origin.x
        rect.origin.y += origin.y
        return rect
    }

    /// Mirrors the point-first glyph resolution the new implementation
    /// performs, so a test can assert which paragraph a probe point
    /// naturally resolves to (independent of any marker-rect matching).
    private func taskListParagraphStart(forGlyphResolving point: CGPoint, in textView: EditorTextView) -> Int {
        let layoutManager = textView.layoutManager
        let origin = taskListMarkerOrigin(for: textView)
        let containerPoint = CGPoint(x: point.x - origin.x, y: point.y - origin.y)
        let glyphIndex = layoutManager.glyphIndex(for: containerPoint, in: textView.textContainer)
        let charIndex = layoutManager.characterIndexForGlyph(at: glyphIndex)
        let nsString = textView.textStorage.string as NSString
        return nsString.paragraphRange(for: NSRange(location: charIndex, length: 0)).location
    }

    func testTaskMarkerHitTest_hitsCheckboxCenterOfSingleTaskItem() {
        let attributed = RenderBridge.renderElements(
            fromJSON: taskListJSON(items: [(text: "Buy milk", checked: false)]),
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 120))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let markerRect = taskMarkerTightRect(forCharacterIndex: 0, in: textView)
        let point = CGPoint(x: markerRect.midX, y: markerRect.midY)

        XCTAssertTrue(
            textView.hasTaskListMarker(at: point),
            "tapping the checkbox center of the only task item must register a hit. markerRect=\(markerRect)"
        )
    }

    func testTaskMarkerHitTest_missesTapFarFromAnyMarker() {
        let attributed = RenderBridge.renderElements(
            fromJSON: taskListJSON(items: [(text: "Buy milk", checked: false)]),
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 120))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let farPoint = CGPoint(x: 220, y: 300)

        XCTAssertFalse(
            textView.hasTaskListMarker(at: farPoint),
            "tapping far outside every marker's tap-slop rect must not register a hit"
        )
    }

    func testTaskMarkerHitTest_hitsRealItemStartButMissesHardBreakContinuationLine() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true, "kind": "task", "checked": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Line one", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 8},
            {"type": "textRun", "text": "Line two", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 160))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let nsString = attributed.string as NSString
        XCTAssertEqual(nsString as String, "Line one\nLine two")

        let realStart = 0
        let hardBreakContinuationStart = nsString.range(of: "Line two").location
        XCTAssertGreaterThan(hardBreakContinuationStart, realStart)

        let realStartMarkerRect = taskMarkerTightRect(forCharacterIndex: realStart, in: textView)
        XCTAssertTrue(
            textView.hasTaskListMarker(at: CGPoint(x: realStartMarkerRect.midX, y: realStartMarkerRect.midY)),
            "the true task-item paragraph start must still register a hit. markerRect=\(realStartMarkerRect)"
        )

        // The checkbox's tap-slop (dy: -8) is intentionally taller than one
        // line's pitch (that generosity is what the straddle tests below
        // cover), so a point near the hard-break continuation line
        // legitimately still resolves to the REAL item's marker via slop.
        // A bare hasTaskListMarker(_:) Bool can't distinguish "correctly
        // matched the real marker via slop" from "incorrectly manufactured
        // a phantom marker for the hard-break line" — assert on the
        // resolved paragraph identity instead.
        let continuationLineRect = taskListLineFragmentRect(forCharacterIndex: hardBreakContinuationStart, in: textView)
        let continuationProbe = CGPoint(x: realStartMarkerRect.midX, y: continuationLineRect.midY)
        XCTAssertEqual(
            textView.taskListMarkerParagraphStartForTesting(at: continuationProbe),
            realStart,
            """
            a paragraph start created by a hard break must never be resolved \
            as its own distinct task-item start (paragraphStart=\(hardBreakContinuationStart)) \
            — any match at this position must be attributed to the real \
            item start (paragraphStart=\(realStart)). \
            continuationLineRect=\(continuationLineRect) probe=\(continuationProbe)
            """
        )
    }

    /// Behavior-pinning test: with the OLD whole-document scan this already
    /// passes. It exists to guard the point-first rewrite, which must keep
    /// resolving the touched line ONLY — never falling back to matching
    /// some other task item's marker rect elsewhere in the document.
    func testTaskMarkerHitTest_missOnPlainLineAmongManyTaskItems() {
        let taskItems = (0..<200).map { (text: "Task \($0)", checked: false) }
        var json = taskListJSON(items: taskItems)
        json.removeLast() // drop the closing "]"
        json += """
        ,
        {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
        {"type": "textRun", "text": "Just a plain paragraph", "marks": []},
        {"type": "blockEnd"}
        ]
        """

        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 6000))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let nsString = attributed.string as NSString
        let plainParagraphStart = nsString.range(of: "Just a plain paragraph").location
        XCTAssertGreaterThan(plainParagraphStart, 0)

        let plainLineRect = taskListLineFragmentRect(forCharacterIndex: plainParagraphStart, in: textView)
        // Tap over the plain paragraph's leading edge, exactly where a task
        // marker WOULD be drawn if this line were a task item.
        let probe = CGPoint(x: plainLineRect.minX - 20, y: plainLineRect.midY)

        XCTAssertFalse(
            textView.hasTaskListMarker(at: probe),
            """
            the tapped line is a plain paragraph, not a task item — it must \
            miss even though 200 other lines in the document ARE task \
            items. probe=\(probe) plainLineRect=\(plainLineRect)
            """
        )
    }

    /// Caveat coverage: the tap-slop inset (`insetBy(dx: -10, dy: -8)`) can
    /// be taller than the line pitch, so a point still inside a marker's
    /// slop zone can glyph-resolve, via point-first lookup, to the
    /// PREVIOUS task item's line. The implementation must probe
    /// point.y - 8 (in addition to the primary lookup) to still find that
    /// marker instead of missing outright.
    func testTaskMarkerHitTest_tapSlopAboveMarkerStillHitsWhenGlyphLookupLandsOnPreviousLine() {
        let attributed = RenderBridge.renderElements(
            fromJSON: taskListJSON(items: [
                (text: "Alpha", checked: false),
                (text: "Bravo", checked: false),
                (text: "Charlie", checked: false),
            ]),
            // A small font keeps line pitch well under the ~24pt checkbox
            // height, guaranteeing the slop zone bleeds into neighboring
            // lines.
            baseFont: .systemFont(ofSize: 8),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 200))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let nsString = attributed.string as NSString
        let bravoStart = nsString.range(of: "Bravo").location
        XCTAssertGreaterThan(bravoStart, 0)

        let bravoMarkerRect = taskMarkerTightRect(forCharacterIndex: bravoStart, in: textView)
        let slopRect = bravoMarkerRect.insetBy(dx: -10, dy: -8)
        let probe = CGPoint(x: slopRect.midX, y: slopRect.minY + 1)

        let resolvedParagraphStart = taskListParagraphStart(forGlyphResolving: probe, in: textView)
        XCTAssertNotEqual(
            resolvedParagraphStart,
            bravoStart,
            """
            test setup invalid: probe must glyph-resolve to a DIFFERENT \
            line than Bravo's to exercise the straddling-inset caveat. \
            resolvedParagraphStart=\(resolvedParagraphStart) bravoStart=\(bravoStart) \
            slopRect=\(slopRect) probe=\(probe)
            """
        )

        XCTAssertTrue(
            textView.hasTaskListMarker(at: probe),
            """
            probe is inside Bravo's tap-slop rect \(slopRect) even though it \
            glyph-resolves to a different line \
            (resolvedParagraphStart=\(resolvedParagraphStart)) — the \
            point-first hit test must still find Bravo's marker by probing \
            point.y +/- 8. probe=\(probe)
            """
        )
    }

    /// Symmetric to the above: a point inside a marker's slop zone that
    /// glyph-resolves to the NEXT task item's line must still hit, via a
    /// point.y + 8 probe.
    func testTaskMarkerHitTest_tapSlopBelowMarkerStillHitsWhenGlyphLookupLandsOnNextLine() {
        let attributed = RenderBridge.renderElements(
            fromJSON: taskListJSON(items: [
                (text: "Alpha", checked: false),
                (text: "Bravo", checked: false),
                (text: "Charlie", checked: false),
            ]),
            baseFont: .systemFont(ofSize: 8),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 200))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let nsString = attributed.string as NSString
        let bravoStart = nsString.range(of: "Bravo").location
        XCTAssertGreaterThan(bravoStart, 0)

        let bravoMarkerRect = taskMarkerTightRect(forCharacterIndex: bravoStart, in: textView)
        let slopRect = bravoMarkerRect.insetBy(dx: -10, dy: -8)
        let probe = CGPoint(x: slopRect.midX, y: slopRect.maxY - 1)

        let resolvedParagraphStart = taskListParagraphStart(forGlyphResolving: probe, in: textView)
        XCTAssertNotEqual(
            resolvedParagraphStart,
            bravoStart,
            """
            test setup invalid: probe must glyph-resolve to a DIFFERENT \
            line than Bravo's to exercise the straddling-inset caveat. \
            resolvedParagraphStart=\(resolvedParagraphStart) bravoStart=\(bravoStart) \
            slopRect=\(slopRect) probe=\(probe)
            """
        )

        XCTAssertTrue(
            textView.hasTaskListMarker(at: probe),
            """
            probe is inside Bravo's tap-slop rect \(slopRect) even though it \
            glyph-resolves to a different line \
            (resolvedParagraphStart=\(resolvedParagraphStart)) — the \
            point-first hit test must still find Bravo's marker by probing \
            point.y +/- 8. probe=\(probe)
            """
        )
    }

    func testReturnInsideBlockquoteAfterPlainParagraphKeepsOneStripeGroup() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 260))
        textView.bindEditor(id: editorId, initialHTML: "<p>Intro</p><blockquote><p>Hello</p></blockquote>")
        textView.layoutIfNeeded()

        editorSetSelectionScalar(id: editorId, scalarAnchor: 11, scalarHead: 11)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.insertText("\n")
        textView.layoutIfNeeded()

        let stripeRects = textView.blockquoteStripeRectsForTesting()
        XCTAssertEqual(
            stripeRects.count,
            1,
            "pressing Return inside a blockquote should not split the quote stripe when the quote follows plain content"
        )
        XCTAssertGreaterThan(
            stripeRects[0].minY,
            0.5,
            "quote stripe should start within the blockquote, not at the preceding paragraph"
        )
        XCTAssertLessThan(
            stripeRects[0].height,
            60.0,
            "quote stripe should stop at the quoted content, not the paragraph spacing below it"
        )
    }

    func testBlockquoteHardBreakAndFollowingParagraphShareOneStripeGroup() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 260))
        textView.bindEditor(
            id: editorId,
            initialHTML: "<blockquote><p>Hello<br>World</p><p>Tail</p></blockquote>"
        )
        textView.layoutIfNeeded()

        let stripeRects = textView.blockquoteStripeRectsForTesting()
        XCTAssertEqual(
            stripeRects.count,
            1,
            "hard breaks inside a blockquote should not split the quote stripe from later quoted content"
        )
        XCTAssertGreaterThan(
            stripeRects[0].height,
            60.0,
            "quote stripe should extend through the hard-break line and following quoted paragraph"
        )
    }

    func testTrailingHardBreakInBlockquoteKeepsStripeConnectedToFollowingParagraph() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 260))
        textView.bindEditor(
            id: editorId,
            initialHTML: "<blockquote><p>Hello<br></p><p>Tail</p></blockquote>"
        )
        textView.layoutIfNeeded()

        let stripeRects = textView.blockquoteStripeRectsForTesting()
        XCTAssertEqual(
            stripeRects.count,
            1,
            "a trailing hard break inside a blockquote should not split the quote stripe from the following quoted paragraph"
        )
        XCTAssertGreaterThan(
            stripeRects[0].height,
            40.0,
            "quote stripe should extend through the trailing hard-break line and following quoted paragraph"
        )
    }

    func testCaretRectAtParagraphStartDoesNotDropByOneLineHeight() {
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
        ])
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "First paragraph.", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Second paragraph starts here.", "marks": []},
            {"type": "blockEnd"}
        ]
        """

        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label,
            theme: theme
        )

        let secondParagraphOffset = (attributed.string as NSString).range(of: "Second").location
        XCTAssertNotEqual(secondParagraphOffset, NSNotFound)

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 240))
        let plainTextView = UITextView(frame: CGRect(x: 0, y: 0, width: 220, height: 240))
        textView.attributedText = attributed
        plainTextView.attributedText = attributed
        textView.layoutIfNeeded()
        plainTextView.layoutIfNeeded()

        guard
            let position = textView.position(from: textView.beginningOfDocument, offset: secondParagraphOffset),
            let plainPosition = plainTextView.position(from: plainTextView.beginningOfDocument, offset: secondParagraphOffset)
        else {
            XCTFail("expected caret positions at paragraph start")
            return
        }

        let caretRect = textView.caretRect(for: position)
        let plainCaretRect = plainTextView.caretRect(for: plainPosition)
        let expected = expectedCaretRect(
            in: plainTextView,
            offset: secondParagraphOffset,
            referenceRect: plainCaretRect,
            font: UIFont.systemFont(ofSize: 16)
        )

        XCTAssertEqual(caretRect.origin.y, expected.origin.y, accuracy: 1.0)
        XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0)
    }

    func testDirectScalarHardBreakTwiceInListItemPreservesExistingText() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<ul><li><p>A</p></li></ul>")

        let firstUpdate = editorInsertNodeAtSelectionScalar(
            id: editorId,
            scalarAnchor: 3,
            scalarHead: 3,
            nodeType: "hardBreak"
        )
        XCTAssertFalse(firstUpdate.isEmpty)
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<ul><li><p>A<br></p></li></ul>",
            "first hardBreak should preserve the existing list item text"
        )

        let secondUpdate = editorInsertNodeAtSelectionScalar(
            id: editorId,
            scalarAnchor: 4,
            scalarHead: 4,
            nodeType: "hardBreak"
        )
        XCTAssertFalse(secondUpdate.isEmpty)
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<ul><li><p>A<br><br></p></li></ul>",
            "second hardBreak at the next scalar position should preserve the original text"
        )
    }

    func testToolbarHardBreakTwiceInListItemPreservesExistingText() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: .zero)
        textView.bindEditor(id: editorId, initialHTML: "<ul><li><p>A</p></li></ul>")

        editorSetSelectionScalar(id: editorId, scalarAnchor: 3, scalarHead: 3)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)

        textView.performToolbarInsertNode("hardBreak")
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<ul><li><p>A<br></p></li></ul>",
            "first hardBreak should preserve the existing list item text"
        )

        textView.performToolbarInsertNode("hardBreak")
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<ul><li><p>A<br><br></p></li></ul>",
            "second hardBreak should append after the first one rather than replacing the text"
        )
    }

    func testToolbarHardBreakMovesCaretToNextVisualLine() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
        ])

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.applyTheme(theme)
        textView.bindEditor(id: editorId, initialHTML: "<p>A</p>")

        editorSetSelectionScalar(id: editorId, scalarAnchor: 1, scalarHead: 1)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        guard let beforePosition = textView.selectedTextRange?.start else {
            XCTFail("expected initial caret position")
            return
        }
        let beforeCaretRect = textView.caretRect(for: beforePosition)

        textView.performToolbarInsertNode("hardBreak")
        textView.layoutIfNeeded()

        let selectionOffset = textView.offset(
            from: textView.beginningOfDocument,
            to: textView.selectedTextRange?.start ?? textView.endOfDocument
        )
        XCTAssertEqual(selectionOffset, 2, "caret should land immediately after the inserted hard break")

        guard let afterPosition = textView.selectedTextRange?.start else {
            XCTFail("expected caret position after hard break")
            return
        }
        let caretRect = textView.caretRect(for: afterPosition)
        XCTAssertGreaterThan(caretRect.minY, beforeCaretRect.minY, "caret should move to the next visual line")
        XCTAssertEqual(caretRect.minY - beforeCaretRect.minY, 32, accuracy: 1.0)
    }

    func testToolbarHardBreakReservesTrailingVisualLineBeforeTyping() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
        ])

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.applyTheme(theme)
        textView.bindEditor(id: editorId, initialHTML: "<p>A</p>")

        editorSetSelectionScalar(id: editorId, scalarAnchor: 1, scalarHead: 1)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.performToolbarInsertNode("hardBreak")
        textView.layoutIfNeeded()
        let heightAfterBreak = ceil(
            textView.sizeThatFits(CGSize(width: 220, height: CGFloat.greatestFiniteMagnitude)).height
        )

        textView.insertText("B")
        textView.layoutIfNeeded()
        let heightAfterTyping = ceil(
            textView.sizeThatFits(CGSize(width: 220, height: CGFloat.greatestFiniteMagnitude)).height
        )

        XCTAssertEqual(heightAfterBreak, heightAfterTyping, accuracy: 1.0)
    }

    func testCaretBeforeHorizontalRuleUsesPreviousParagraphLine() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p><hr><p>World</p>")
        textView.layoutIfNeeded()

        guard let hrRange = firstHorizontalRuleRange(in: textView) else {
            XCTFail("expected a horizontal rule attachment in the rendered text")
            return
        }
        guard let previousCharacterIndex = previousVisibleCharacterIndex(before: hrRange.location, in: textView) else {
            XCTFail("expected visible content before the horizontal rule")
            return
        }

        setCollapsedSelection(in: textView, utf16Offset: hrRange.location)
        guard let position = textView.selectedTextRange?.start else {
            XCTFail("expected caret position before the horizontal rule")
            return
        }

        let caretRect = textView.caretRect(for: position)
        let expected = expectedCaretRectForCharacterEdge(
            in: textView,
            characterIndex: previousCharacterIndex,
            edge: .trailing,
            font: UIFont.systemFont(ofSize: 16)
        )

        XCTAssertEqual(caretRect.minX, expected.minX, accuracy: 1.0)
        XCTAssertEqual(caretRect.minY, expected.minY, accuracy: 1.0)
        XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0)
    }

    func testCaretAfterHorizontalRuleUsesFollowingParagraphLine() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p><hr><p>World</p>")
        textView.layoutIfNeeded()

        guard let hrRange = firstHorizontalRuleRange(in: textView) else {
            XCTFail("expected a horizontal rule attachment in the rendered text")
            return
        }
        guard let nextCharacterIndex = nextVisibleCharacterIndex(after: hrRange.location, in: textView) else {
            XCTFail("expected visible content after the horizontal rule")
            return
        }

        setCollapsedSelection(in: textView, utf16Offset: hrRange.location + hrRange.length)
        guard let position = textView.selectedTextRange?.start else {
            XCTFail("expected caret position after the horizontal rule")
            return
        }

        let caretRect = textView.caretRect(for: position)
        let expected = expectedCaretRectForCharacterEdge(
            in: textView,
            characterIndex: nextCharacterIndex,
            edge: .leading,
            font: UIFont.systemFont(ofSize: 16)
        )

        XCTAssertEqual(caretRect.minX, expected.minX, accuracy: 1.0)
        XCTAssertEqual(caretRect.minY, expected.minY, accuracy: 1.0)
        XCTAssertEqual(caretRect.height, expected.height, accuracy: 1.0)
    }

    func testToolbarHorizontalRulePlacesCaretInTrailingParagraphLine() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        editorSetSelectionScalar(id: editorId, scalarAnchor: 3, scalarHead: 3)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.performToolbarInsertNode("horizontalRule")
        textView.layoutIfNeeded()

        guard let hrRange = firstHorizontalRuleRange(in: textView) else {
            XCTFail("expected a horizontal rule attachment after toolbar insertion")
            return
        }
        guard let position = textView.selectedTextRange?.start else {
            XCTFail("expected a caret after inserting a horizontal rule")
            return
        }

        let selectionOffset = textView.offset(from: textView.beginningOfDocument, to: position)
        let caretRect = textView.caretRect(for: position)
        let hrRect = renderedRect(in: textView, utf16Range: hrRange)

        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<p>Hello</p><hr><p></p>",
            "toolbar hr insert should create a trailing empty paragraph"
        )
        XCTAssertEqual(
            selectionOffset,
            textView.text.count,
            "toolbar hr insert should place the caret at the end of the rendered trailing paragraph"
        )
        XCTAssertGreaterThan(
            caretRect.midY,
            hrRect.midY,
            "caret after inserting a horizontal rule should render below the rule line"
        )
    }

    func testMentionSuggestionTapInsertsMentionNode() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["id": "user_alice", "label": "@alice"],
            ])!,
        ])

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "tapping a mention suggestion should insert a mention node, got: \(html)"
        )
        XCTAssertTrue(
            html.contains("@alice"),
            "mention insertion should preserve the visible label, got: \(html)"
        )
        XCTAssertTrue(
            html.contains("mentionSuggestionChar"),
            "mention insertion should preserve the suggestion trigger in attrs, got: \(html)"
        )
    }

    func testMentionSuggestionTapDrainsPendingNativeAutocorrectBeforeInsert() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>teh @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 4, head: 7)
        )
        view.setMentionSuggestionsForTesting([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["id": "user_alice", "label": "@alice"],
            ])!,
        ])
        view.layoutIfNeeded()
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(html.contains("the"), "mention insert should preserve native correction, got: \(html)")
        XCTAssertFalse(html.contains("teh"), "mention insert should not restore stale text, got: \(html)")
        XCTAssertFalse(html.contains("@al</p>"), "mention insert should replace the query range, got: \(html)")
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention insert should still insert the mention node, got: \(html)"
        )
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testMentionSelectRequestIncludesPreflightUpdateAfterNativeAutocorrectDrain() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>teh @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","resolveSelectionAttrs":true,"suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 4, head: 7)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let event = parseJSONObject(view.lastAddonEventJSONForTesting())
        XCTAssertEqual(event["type"] as? String, "mentionsSelectRequest")
        XCTAssertEqual(event["suggestionKey"] as? String, "alice")
        let range = event["range"] as? [String: Any]
        XCTAssertEqual(jsonInt(range?["anchor"]), 4)
        XCTAssertEqual(jsonInt(range?["head"]), 7)

        let updateJSON = event["updateJson"] as? String
        XCTAssertNotNil(updateJSON)
        XCTAssertTrue(updateJSON?.contains("the @al") == true, "select request should carry the drained correction update")
        XCTAssertFalse(updateJSON?.contains("teh @al") == true, "select request should not carry stale pre-correction text")

        let update = parseJSONObject(updateJSON)
        XCTAssertEqual(jsonInt(event["documentVersion"]), jsonInt(update["documentVersion"]))
    }

    func testMentionSuggestionTapDrainsPendingNativeAutocorrectInsideListItem() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<ul><li><p>teh @al</p></li></ul>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 4, head: 7)
        )
        view.setMentionSuggestionsForTesting([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["id": "user_alice", "label": "@alice"],
            ])!,
        ])
        view.layoutIfNeeded()
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(html.contains("<ul><li><p>the "), "mention insert should preserve list correction, got: \(html)")
        XCTAssertFalse(html.contains("teh"), "mention insert should not restore stale list text, got: \(html)")
        XCTAssertFalse(html.contains("@al</p>"), "mention insert should replace the list query range, got: \(html)")
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention insert should still insert the mention node in the list item, got: \(html)"
        )
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testMentionSuggestionTapRecomputesRangeAfterLengthChangingAutocorrect() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>a @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 2, head: 5)
        )
        view.setMentionSuggestionsForTesting([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["id": "user_alice", "label": "@alice"],
            ])!,
        ])
        view.layoutIfNeeded()
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 1),
            with: "an"
        )
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(html.contains("an "), "mention insert should preserve length-changing correction, got: \(html)")
        XCTAssertFalse(html.contains("@al</p>"), "mention insert should replace the recomputed query range, got: \(html)")
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention insert should insert the mention node after recomputing the range, got: \(html)"
        )
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testMentionSuggestionTapRetriesAfterBlockedMarkedTextPreflight() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)

        XCTAssertFalse(editorGetHtml(id: editorId).contains("data-native-editor-mention=\"true\""))

        flushMainQueue()
        flushMainQueue()

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention tap should retry after composition preflight clears, got: \(html)"
        )
        XCTAssertFalse(html.contains("@al</p>"), "retried mention tap should replace query, got: \(html)")
    }

    func testMentionSuggestionTapRetrySurvivesPreflightDrainedAutocorrect() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>teh @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 4, head: 7)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)
        XCTAssertFalse(editorGetHtml(id: editorId).contains("data-native-editor-mention=\"true\""))

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(html.contains("the "), "retried mention tap should preserve preflight correction, got: \(html)")
        XCTAssertFalse(html.contains("teh"), "retried mention tap should not restore stale text, got: \(html)")
        XCTAssertFalse(html.contains("@al</p>"), "retried mention tap should replace the query, got: \(html)")
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention tap should retry after draining autocorrect during preflight, got: \(html)"
        )
    }

    func testMentionSuggestionTapRetrySurvivesLengthChangingPreflightDrainedAutocorrect() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>a @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 2, head: 5)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)
        XCTAssertFalse(editorGetHtml(id: editorId).contains("data-native-editor-mention=\"true\""))

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 1),
            with: "an"
        )
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(html.contains("an "), "retried mention tap should preserve length-changing correction, got: \(html)")
        XCTAssertFalse(html.contains("<p>a "), "retried mention tap should not restore stale text, got: \(html)")
        XCTAssertFalse(html.contains("@al</p>"), "retried mention tap should replace the shifted query, got: \(html)")
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention tap should retry after draining shifted autocorrect during preflight, got: \(html)"
        )
    }

    func testMentionSuggestionTapRetryIsDroppedWhenPreflightShiftTargetsDifferentSameQuery() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>a @al b @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 2, head: 5)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)
        XCTAssertFalse(editorGetHtml(id: editorId).contains("data-native-editor-mention=\"true\""))

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 1),
            with: "an"
        )
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        let html = editorGetHtml(id: editorId)
        XCTAssertEqual(html, "<p>an @al b @al</p>")
        XCTAssertFalse(
            html.contains("data-native-editor-mention=\"true\""),
            "retry should not jump to a different identical query after preflight drains a correction, got: \(html)"
        )
    }

    func testMentionSuggestionTapRetryUsesRefreshedSuggestionForSameKey() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let refreshedSuggestion = NativeMentionSuggestion(dictionary: [
            "key": "alice",
            "title": "Ally Chen",
            "subtitle": "Design",
            "label": "@ally",
            "attrs": ["id": "user_ally", "label": "@ally"],
        ])!
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Ally Chen","subtitle":"Design","label":"@ally","attrs":{"id":"user_ally","label":"@ally"}}]}}
            """
        )
        view.setMentionSuggestionsForTesting([refreshedSuggestion])

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(
            html.contains("@ally"),
            "retried mention tap should use the refreshed same-key label, got: \(html)"
        )
        XCTAssertFalse(
            html.contains("@alice"),
            "retried mention tap should not use the stale captured label, got: \(html)"
        )

        let event = parseJSONObject(view.lastAddonEventJSONForTesting())
        let attrs = event["attrs"] as? [String: Any]
        XCTAssertEqual(event["type"] as? String, "mentionsSelect")
        XCTAssertEqual(event["suggestionKey"] as? String, "alice")
        XCTAssertEqual(attrs?["id"] as? String, "user_ally")
        XCTAssertEqual(attrs?["label"] as? String, "@ally")
    }

    func testMentionSuggestionTapRetryIsDroppedAfterQueryChanges() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let changedUpdateJSON = editorReplaceHtml(id: editorId, html: "<p>Hello @bo</p>")
        view.richTextView.textView.applyUpdateJSON(changedUpdateJSON, notifyDelegate: false)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "bo", trigger: "@", anchor: 6, head: 9)
        )

        flushMainQueue()
        flushMainQueue()

        let html = editorGetHtml(id: editorId)
        XCTAssertEqual(html, "<p>Hello @bo</p>")
        XCTAssertFalse(
            html.contains("data-native-editor-mention=\"true\""),
            "stale mention retry should not insert into a changed query, got: \(html)"
        )
    }

    func testMentionSuggestionTapRetryIsDroppedAfterSameQueryRangeChanges() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        _ = editorSetHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let changedUpdateJSON = editorReplaceHtml(id: editorId, html: "<p>@al Hello @al</p>")
        view.richTextView.textView.applyUpdateJSON(changedUpdateJSON, notifyDelegate: false)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 10, head: 13)
        )

        flushMainQueue()
        flushMainQueue()

        let html = editorGetHtml(id: editorId)
        XCTAssertEqual(html, "<p>@al Hello @al</p>")
        XCTAssertFalse(
            html.contains("data-native-editor-mention=\"true\""),
            "same-query retry should still be dropped when its range moved, got: \(html)"
        )
    }

    func testMentionSuggestionTapRetryIsDroppedAfterEditorRebind() {
        let firstEditorId = editorCreate(configJson: mentionEditorConfigJson())
        let secondEditorId = editorCreate(configJson: mentionEditorConfigJson())
        defer {
            editorDestroy(id: firstEditorId)
            editorDestroy(id: secondEditorId)
        }
        _ = editorSetHtml(id: firstEditorId, html: "<p>Hello @al</p>")
        _ = editorSetHtml(id: secondEditorId, html: "<p>Second @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(firstEditorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)
        view.setEditorId(secondEditorId)
        flushMainQueue()
        flushMainQueue()

        XCTAssertFalse(editorGetHtml(id: firstEditorId).contains("data-native-editor-mention=\"true\""))
        XCTAssertFalse(editorGetHtml(id: secondEditorId).contains("data-native-editor-mention=\"true\""))
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second @al")
    }

    func testMentionSuggestionTapStillWorksAfterRebindingToMentionSchemaEditor() {
        let initialEditorId = editorCreate(configJson: "{}")
        let mentionEditorId = editorCreate(configJson: mentionEditorConfigJson())
        defer {
            editorDestroy(id: initialEditorId)
            editorDestroy(id: mentionEditorId)
        }

        _ = editorSetHtml(id: initialEditorId, html: "<p>Hello</p>")
        _ = editorSetHtml(id: mentionEditorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.setEditorId(initialEditorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setEditorId(mentionEditorId)
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["id": "user_alice", "label": "@alice"],
            ])!,
        ])

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let html = editorGetHtml(id: mentionEditorId)
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention insert should target the rebound mention-schema editor, got: \(html)"
        )
    }

    func testCurrentMentionQueryStateWorksInsideListItem() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        _ = editorSetHtml(id: editorId, html: "<ul><li><p>Hello @al</p></li></ul>")
        view.richTextView.textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)

        let text = view.richTextView.textView.text ?? ""
        let utf16Offset = (text as NSString).range(of: "@al").location + 3
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: utf16Offset)

        let queryState = view.currentMentionQueryStateForTesting(trigger: "@")
        XCTAssertEqual(queryState?.query, "al")
        XCTAssertNotNil(queryState, "mention query should resolve inside a list item")
    }

    func testCurrentMentionQueryStateWorksInLastParagraph() {
        let editorId = editorCreate(configJson: mentionEditorConfigJson())
        defer { editorDestroy(id: editorId) }

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        _ = editorSetHtml(id: editorId, html: "<p>First paragraph</p><p>@al</p>")
        view.richTextView.textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)

        let text = view.richTextView.textView.text ?? ""
        let utf16Offset = (text as NSString).range(of: "@al").location + 3
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: utf16Offset)

        let queryState = view.currentMentionQueryStateForTesting(trigger: "@")
        XCTAssertEqual(queryState?.query, "al")
        XCTAssertNotNil(queryState, "mention query should resolve in the final paragraph")
    }

    func testBackspaceBelowHorizontalRuleReplacesItWithParagraph() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        editorSetSelectionScalar(id: editorId, scalarAnchor: 3, scalarHead: 3)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)

        textView.performToolbarInsertNode("horizontalRule")
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<p>Hello</p><hr><p></p>",
            "toolbar hr insert should create a trailing empty paragraph"
        )

        textView.deleteBackward()
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<p>Hello</p><p></p>",
            "backspacing below an hr should replace it with an empty paragraph"
        )

        textView.insertText("B")
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<p>Hello</p><p>B</p>",
            "typing after hr removal should stay in the replacement paragraph"
        )
    }

    func testTypingAndBackspacingAroundImageUsesTrailingParagraphCaret() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        editorSetSelectionScalar(id: editorId, scalarAnchor: 3, scalarHead: 3)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)

        let imageFragmentJson = """
        {"type":"doc","content":[{"type":"image","attrs":{"src":"https://example.com/cat.png","alt":"Cat"}}]}
        """
        let updateJSON = editorInsertContentJsonAtSelectionScalar(
            id: editorId,
            scalarAnchor: 3,
            scalarHead: 3,
            json: imageFragmentJson
        )
        textView.applyUpdateJSON(updateJSON, notifyDelegate: false)

        let selectionOffset = textView.offset(
            from: textView.beginningOfDocument,
            to: textView.selectedTextRange?.start ?? textView.endOfDocument
        )
        XCTAssertEqual(
            selectionOffset,
            textView.text.count,
            "image insertion should place the caret in the trailing paragraph"
        )

        textView.insertText("B")
        let htmlAfterTyping = editorGetHtml(id: editorId)
        XCTAssertTrue(htmlAfterTyping.starts(with: "<p>Hello</p><img "))
        XCTAssertTrue(htmlAfterTyping.contains("src=\"https://example.com/cat.png\""))
        XCTAssertTrue(htmlAfterTyping.contains("alt=\"Cat\""))
        XCTAssertTrue(
            htmlAfterTyping.hasSuffix("<p>B</p>"),
            "typing after image insert should land in the trailing paragraph"
        )

        textView.deleteBackward()
        let htmlAfterFirstBackspace = editorGetHtml(id: editorId)
        XCTAssertTrue(htmlAfterFirstBackspace.starts(with: "<p>Hello</p><img "))
        XCTAssertTrue(htmlAfterFirstBackspace.contains("src=\"https://example.com/cat.png\""))
        XCTAssertTrue(htmlAfterFirstBackspace.contains("alt=\"Cat\""))
        XCTAssertTrue(
            htmlAfterFirstBackspace.hasSuffix("<p></p>"),
            "first backspace should delete the trailing paragraph text"
        )

        textView.deleteBackward()
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            "<p>Hello</p><p></p>",
            "second backspace from the empty trailing paragraph should replace the image with a paragraph"
        )
    }

    func testSelectingImageShowsResizeOverlayAndPersistsResizedDimensions() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: """
        <p>Hello</p><img src="https://example.com/cat.png" width="140" height="80"><p></p>
        """)
        view.layoutIfNeeded()

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setSelection(in: view.textView, utf16Range: imageRange)
        flushMainQueue()
        view.layoutIfNeeded()

        let initialRect = view.imageResizeOverlayRectForTesting()
        XCTAssertNotNil(initialRect, "selecting an image should show the resize overlay")
        XCTAssertEqual(initialRect?.width ?? 0, 140, accuracy: 1.0)
        XCTAssertEqual(initialRect?.height ?? 0, 80, accuracy: 1.0)

        view.resizeSelectedImageForTesting(width: 200, height: 100)
        flushMainQueue()
        view.layoutIfNeeded()

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(html.contains("width=\"200\""), "expected resized width in HTML, got: \(html)")
        XCTAssertTrue(html.contains("height=\"100\""), "expected resized height in HTML, got: \(html)")

        let resizedRect = view.imageResizeOverlayRectForTesting()
        XCTAssertNotNil(resizedRect)
        XCTAssertEqual(resizedRect?.width ?? 0, 200, accuracy: 1.0)
        XCTAssertEqual(resizedRect?.height ?? 0, 100, accuracy: 1.0)
        XCTAssertGreaterThan(resizedRect?.width ?? 0, initialRect?.width ?? 0)
        XCTAssertGreaterThan(resizedRect?.height ?? 0, initialRect?.height ?? 0)
    }

    func testSelectedImageOverlayAllowsTouchesOutsideResizeHandles() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: """
        <p>Hello</p><img src="https://example.com/cat.png" width="140" height="80"><p></p>
        """)
        view.layoutIfNeeded()

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setSelection(in: view.textView, utf16Range: imageRange)
        flushMainQueue()
        view.layoutIfNeeded()

        guard let overlayRect = view.imageResizeOverlayRectForTesting() else {
            XCTFail("expected a visible image resize overlay")
            return
        }

        XCTAssertTrue(
            view.imageResizeOverlayInterceptsPointForTesting(CGPoint(x: overlayRect.maxX, y: overlayRect.maxY)),
            "resize handles should remain interactive"
        )
        XCTAssertFalse(
            view.imageResizeOverlayInterceptsPointForTesting(CGPoint(x: overlayRect.midX, y: overlayRect.maxY + 24)),
            "touches below the selected image should pass through so the user can place the caret and deselect the image"
        )
    }

    func testSelectingImageHidesNativeSelectionChromeUntilCaretMovesAway() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: """
        <p>Hello</p><img src="https://example.com/cat.png" width="140" height="80"><p></p>
        """)
        view.layoutIfNeeded()

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setSelection(in: view.textView, utf16Range: imageRange)
        flushMainQueue()
        view.layoutIfNeeded()

        XCTAssertEqual(view.textView.tintColor.cgColor.alpha, 0, accuracy: 0.001)
        XCTAssertEqual(view.textView.caretRect(for: view.textView.selectedTextRange?.start ?? view.textView.beginningOfDocument), .zero)

        setSelection(in: view.textView, utf16Range: NSRange(location: imageRange.location + 1, length: 0))
        flushMainQueue()
        view.layoutIfNeeded()

        XCTAssertGreaterThan(view.textView.tintColor.cgColor.alpha, 0.1)
    }

    func testUnfocusedImageTapSelectsImageOnFirstTap() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        view.editorId = editorId
        view.setContent(html: """
        <p>Hello</p><img src="https://example.com/cat.png" width="140" height="80"><p></p>
        """)
        viewController.view.addSubview(view)
        view.layoutIfNeeded()

        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        let imageRect = renderedRect(in: view.textView, utf16Range: imageRange)
        XCTAssertNil(view.imageResizeOverlayRectForTesting())
        XCTAssertTrue(
            view.imageTapOverlayInterceptsPointForTesting(
                CGPoint(x: imageRect.midX, y: imageRect.midY)
            )
        )

        XCTAssertTrue(
            view.tapImageOverlayForTesting(
                at: CGPoint(x: imageRect.midX, y: imageRect.midY)
            ),
            "the first unfocused tap on an image should select it immediately"
        )
        flushMainQueue()
        view.layoutIfNeeded()

        let selectedRange = view.textView.selectedTextRange
        let startOffset = view.textView.offset(
            from: view.textView.beginningOfDocument,
            to: selectedRange?.start ?? view.textView.endOfDocument
        )
        let endOffset = view.textView.offset(
            from: view.textView.beginningOfDocument,
            to: selectedRange?.end ?? view.textView.endOfDocument
        )

        XCTAssertEqual(startOffset, imageRange.location)
        XCTAssertEqual(endOffset, imageRange.location + imageRange.length)
        XCTAssertNotNil(view.imageResizeOverlayRectForTesting())
    }

    func testFocusedImageTapSelectsImageOnFirstTap() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        view.editorId = editorId
        view.setContent(html: """
        <p>Hello</p><img src="https://example.com/cat.png" width="140" height="80"><p></p>
        """)
        viewController.view.addSubview(view)
        view.layoutIfNeeded()

        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setCollapsedSelection(in: view.textView, utf16Offset: 0)
        flushMainQueue()
        view.layoutIfNeeded()

        let imageRect = renderedRect(in: view.textView, utf16Range: imageRange)
        XCTAssertTrue(
            view.tapImageOverlayForTesting(
                at: CGPoint(x: imageRect.midX, y: imageRect.midY)
            ),
            "a focused image tap should select the image immediately"
        )
        flushMainQueue()
        view.layoutIfNeeded()

        let selectedRange = view.textView.selectedTextRange
        let startOffset = view.textView.offset(
            from: view.textView.beginningOfDocument,
            to: selectedRange?.start ?? view.textView.endOfDocument
        )
        let endOffset = view.textView.offset(
            from: view.textView.beginningOfDocument,
            to: selectedRange?.end ?? view.textView.endOfDocument
        )

        XCTAssertEqual(startOffset, imageRange.location)
        XCTAssertEqual(endOffset, imageRange.location + imageRange.length)
        XCTAssertNotNil(view.imageResizeOverlayRectForTesting())
    }

    func testDisablingImageResizingRemovesImageSelectionOverlayBehavior() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.allowImageResizing = false
        view.editorId = editorId
        view.setContent(html: """
        <p>Hello</p><img src="https://example.com/cat.png" width="140" height="80"><p></p>
        """)
        view.layoutIfNeeded()

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        let imageRect = renderedRect(in: view.textView, utf16Range: imageRange)
        XCTAssertFalse(
            view.imageTapOverlayInterceptsPointForTesting(
                CGPoint(x: imageRect.midX, y: imageRect.midY)
            )
        )

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setSelection(in: view.textView, utf16Range: imageRange)
        flushMainQueue()
        view.layoutIfNeeded()

        XCTAssertNil(view.imageResizeOverlayRectForTesting())
        XCTAssertGreaterThan(view.textView.tintColor.cgColor.alpha, 0.1)
    }

    func testSelectedImageOverlayHidesWhenEditorLosesFocus() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        view.editorId = editorId
        view.setContent(html: """
        <p>Hello</p><img src="https://example.com/cat.png" width="140" height="80"><p></p>
        """)
        viewController.view.addSubview(view)
        view.layoutIfNeeded()

        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setSelection(in: view.textView, utf16Range: imageRange)
        flushMainQueue()
        view.layoutIfNeeded()

        XCTAssertNotNil(view.imageResizeOverlayRectForTesting())

        XCTAssertTrue(view.textView.resignFirstResponder())
        view.refreshSelectionVisualStateForTesting()
        flushMainQueue()
        view.layoutIfNeeded()

        XCTAssertNil(view.imageResizeOverlayRectForTesting())
    }

    func testDeferredImageTapSelectionWinsAfterUIKitCaretPlacement() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        view.editorId = editorId
        view.setContent(html: """
        <p>Hello</p><img src="https://example.com/cat.png" width="140" height="80"><p></p>
        """)
        viewController.view.addSubview(view)
        view.layoutIfNeeded()

        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        let imageRect = renderedRect(in: view.textView, utf16Range: imageRange)
        XCTAssertTrue(view.textView.becomeFirstResponder())
        setCollapsedSelection(in: view.textView, utf16Offset: 0)
        flushMainQueue()
        view.layoutIfNeeded()

        XCTAssertTrue(
            view.tapImageOverlayForTesting(
                at: CGPoint(x: imageRect.midX, y: imageRect.midY)
            )
        )

        // Mirror UIKit collapsing the image selection back to a caret.
        setCollapsedSelection(in: view.textView, utf16Offset: imageRange.location + 1)
        view.textView.textViewDidChangeSelection(view.textView)
        flushMainQueue()
        view.layoutIfNeeded()

        let selectedRange = view.textView.selectedTextRange
        let startOffset = view.textView.offset(
            from: view.textView.beginningOfDocument,
            to: selectedRange?.start ?? view.textView.endOfDocument
        )
        let endOffset = view.textView.offset(
            from: view.textView.beginningOfDocument,
            to: selectedRange?.end ?? view.textView.endOfDocument
        )

        XCTAssertEqual(startOffset, imageRange.location)
        XCTAssertEqual(endOffset, imageRange.location + imageRange.length)
        XCTAssertNotNil(view.imageResizeOverlayRectForTesting())
    }

    func testImageTapOverlayInterceptsImagePointsOnly() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        view.editorId = editorId
        view.setContent(html: """
        <p>Hello</p><img src="https://example.com/cat.png" width="140" height="80"><p></p>
        """)
        view.layoutIfNeeded()

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        let imageRect = renderedRect(in: view.textView, utf16Range: imageRange)
        let imageTapPoint = CGPoint(x: imageRect.midX, y: imageRect.midY)

        XCTAssertTrue(view.imageTapOverlayInterceptsPointForTesting(imageTapPoint))
        XCTAssertFalse(
            view.imageTapOverlayInterceptsPointForTesting(
                CGPoint(x: imageRect.midX, y: imageRect.maxY + 24)
            )
        )
    }

    func testOversizedImageResizeClampsToContentWidthAndKeepsAutoGrowHeightBounded() {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 0))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.heightBehavior = .autoGrow
        view.editorId = editorId
        view.setContent(html: """
        <p>Hello</p><img src="https://example.com/cat.png" width="140" height="80"><p></p>
        """)
        view.layoutIfNeeded()

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setSelection(in: view.textView, utf16Range: imageRange)
        flushMainQueue()
        view.layoutIfNeeded()

        let maximumWidth = view.maximumImageWidthForTesting()
        let expectedHeight = max(48, maximumWidth / 2)

        view.resizeSelectedImageForTesting(width: 4_000, height: 2_000)
        flushMainQueue()
        view.layoutIfNeeded()

        let html = editorGetHtml(id: editorId)
        XCTAssertTrue(
            html.contains("width=\"\(Int(maximumWidth.rounded()))\""),
            "oversized image width should clamp to the editor content width, got: \(html)"
        )
        XCTAssertTrue(
            html.contains("height=\"\(Int(expectedHeight.rounded()))\""),
            "oversized image height should preserve aspect ratio after clamping, got: \(html)"
        )

        let overlayRect = view.imageResizeOverlayRectForTesting()
        XCTAssertEqual(overlayRect?.width ?? 0, maximumWidth, accuracy: 1.0)
        XCTAssertEqual(overlayRect?.height ?? 0, expectedHeight, accuracy: 1.0)
        XCTAssertLessThan(view.intrinsicContentSize.height, 400)
    }

    func testImageResizePreviewUsesOverlayImageAndDefersDocumentMutationUntilCommit() {
        let editorId = editorCreate(configJson: #"{"allowBase64Images":true}"#)
        defer { editorDestroy(id: editorId) }

        let dataUri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4////fwAJ+wP9KobjigAAAABJRU5ErkJggg=="

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 0))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.heightBehavior = .autoGrow
        view.editorId = editorId
        view.setContent(json: """
        {
          "type": "doc",
          "content": [
            {
              "type": "paragraph",
              "content": [
                {
                  "type": "text",
                  "text": "Hello"
                }
              ]
            },
            {
              "type": "image",
              "attrs": {
                "src": "\(dataUri)",
                "width": 140,
                "height": 80
              }
            },
            {
              "type": "paragraph"
            }
          ]
        }
        """)
        view.layoutIfNeeded()

        guard let imageRange = firstImageRange(in: view.textView) else {
            XCTFail("expected an image attachment in the rendered text")
            return
        }

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setSelection(in: view.textView, utf16Range: imageRange)
        flushMainQueue()
        view.layoutIfNeeded()

        let initialHtml = editorGetHtml(id: editorId)
        let initialHeight = view.intrinsicContentSize.height
        let maximumWidth = view.maximumImageWidthForTesting()

        view.previewResizeSelectedImageForTesting(width: 4_000, height: 2_000)
        flushMainQueue()
        view.layoutIfNeeded()

        XCTAssertTrue(
            view.imageResizePreviewHasImageForTesting(),
            "the live resize preview should render an image overlay instead of blanking while the drag is active"
        )
        XCTAssertEqual(
            editorGetHtml(id: editorId),
            initialHtml,
            "preview resizing should not mutate the document before the gesture commits"
        )
        XCTAssertEqual(
            view.intrinsicContentSize.height,
            initialHeight,
            accuracy: 1.0,
            "preview resizing should not change auto-grow measurement before commit"
        )
        XCTAssertEqual(view.imageResizeOverlayRectForTesting()?.width ?? 0, maximumWidth, accuracy: 1.0)

        view.commitPreviewResizeForTesting()
        flushMainQueue()
        view.layoutIfNeeded()

        let committedHtml = editorGetHtml(id: editorId)
        XCTAssertTrue(committedHtml.contains("width=\"\(Int(maximumWidth.rounded()))\""))
        XCTAssertNotEqual(committedHtml, initialHtml)
        XCTAssertFalse(view.imageResizePreviewHasImageForTesting())
    }

    private func expectedCaretRect(
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

    private enum CharacterEdge {
        case leading
        case trailing
    }

    private func expectedCaretRectForCharacterEdge(
        in textView: UITextView,
        characterIndex: Int,
        edge: CharacterEdge,
        font: UIFont
    ) -> CGRect {
        guard let rect = visibleCharacterRect(in: textView, characterIndex: characterIndex) else {
            XCTFail("expected visible rect for character index \(characterIndex)")
            return .zero
        }
        guard let baselineY = baselineYForCharacter(in: textView, characterIndex: characterIndex) else {
            XCTFail("expected baseline for character index \(characterIndex)")
            return .zero
        }

        let referenceRect = CGRect(
            x: edge == .leading ? rect.minX : rect.maxX,
            y: rect.minY,
            width: 2,
            height: rect.height
        )
        return EditorTextView.adjustedCaretRect(
            from: referenceRect,
            baselineY: baselineY,
            font: font,
            screenScale: 2
        )
    }

    private func baselineYForCharacter(
        in textView: UITextView,
        characterIndex: Int
    ) -> CGFloat? {
        guard characterIndex >= 0, characterIndex < textView.attributedText.length else { return nil }
        let glyphIndex = textView.layoutManager.glyphIndexForCharacter(at: characterIndex)
        guard glyphIndex < textView.layoutManager.numberOfGlyphs else { return nil }

        let lineFragmentRect = textView.layoutManager.lineFragmentRect(
            forGlyphAt: glyphIndex,
            effectiveRange: nil
        )
        let glyphLocation = textView.layoutManager.location(forGlyphAt: glyphIndex)
        return textView.textContainerInset.top + lineFragmentRect.minY + glyphLocation.y
    }

    private func setCollapsedSelection(in textView: UITextView, utf16Offset: Int) {
        guard
            let position = textView.position(from: textView.beginningOfDocument, offset: utf16Offset),
            let range = textView.textRange(from: position, to: position)
        else {
            XCTFail("expected caret position at offset \(utf16Offset)")
            return
        }

        textView.selectedTextRange = range
    }

    private func setSelection(in textView: UITextView, utf16Range: NSRange) {
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

    private func assertSelectedUtf16Range(
        in textView: UITextView,
        _ expectedRange: NSRange,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertEqual(selectedUtf16Range(in: textView), expectedRange, file: file, line: line)
    }

    private func assertCollapsedEditorSelection(
        in editorId: UInt64,
        scalarOffset: UInt32,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let selection = currentSelection(in: editorId)
        let expectedDocPos = editorScalarToDoc(id: editorId, scalar: scalarOffset)
        XCTAssertEqual(selection["type"] as? String, "text", file: file, line: line)
        XCTAssertEqual((selection["anchor"] as? NSNumber)?.uint32Value, expectedDocPos, file: file, line: line)
        XCTAssertEqual((selection["head"] as? NSNumber)?.uint32Value, expectedDocPos, file: file, line: line)
    }

    private func previousVisibleCharacterIndex(
        before utf16Offset: Int,
        in textView: UITextView
    ) -> Int? {
        let text = textView.textStorage.string as NSString
        guard text.length > 0 else { return nil }

        var index = min(utf16Offset - 1, text.length - 1)
        while index >= 0 {
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            let character = text.substring(with: NSRange(location: index, length: 1))
            if attrs[.attachment] == nil,
               character != "\n",
               character != "\r",
               visibleCharacterRect(in: textView, characterIndex: index) != nil
            {
                return index
            }
            index -= 1
        }

        return nil
    }

    private func nextVisibleCharacterIndex(
        after utf16Offset: Int,
        in textView: UITextView
    ) -> Int? {
        let text = textView.textStorage.string as NSString
        guard text.length > 0 else { return nil }

        var index = max(utf16Offset, 0)
        while index < text.length {
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            let character = text.substring(with: NSRange(location: index, length: 1))
            if attrs[.attachment] == nil,
               character != "\n",
               character != "\r",
               visibleCharacterRect(in: textView, characterIndex: index) != nil
            {
                return index
            }
            index += 1
        }

        return nil
    }

    private func visibleCharacterRect(
        in textView: UITextView,
        characterIndex: Int
    ) -> CGRect? {
        guard characterIndex >= 0, characterIndex < textView.textStorage.length else { return nil }
        guard let start = textView.position(from: textView.beginningOfDocument, offset: characterIndex),
              let end = textView.position(from: start, offset: 1),
              let range = textView.textRange(from: start, to: end)
        else {
            return nil
        }

        return textView.selectionRects(for: range)
            .map(\.rect)
            .first(where: { !$0.isEmpty && $0.width > 0 && $0.height > 0 })
    }

    private func firstImageRange(in textView: UITextView) -> NSRange? {
        guard textView.textStorage.length > 0 else { return nil }

        for index in 0..<textView.textStorage.length {
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            if (attrs[RenderBridgeAttributes.voidNodeType] as? String) == "image" {
                return NSRange(location: index, length: 1)
            }
        }

        return nil
    }

    private func firstHorizontalRuleRange(in textView: UITextView) -> NSRange? {
        guard textView.textStorage.length > 0 else { return nil }

        for index in 0..<textView.textStorage.length {
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            if attrs[.attachment] is NSTextAttachment,
               (attrs[RenderBridgeAttributes.voidNodeType] as? String) == "horizontalRule"
            {
                return NSRange(location: index, length: 1)
            }
        }

        return nil
    }

    private func renderedRect(in textView: UITextView, utf16Range: NSRange) -> CGRect {
        let glyphRange = textView.layoutManager.glyphRange(
            forCharacterRange: utf16Range,
            actualCharacterRange: nil
        )
        var rect = textView.layoutManager.boundingRect(forGlyphRange: glyphRange, in: textView.textContainer)
        rect.origin.x += textView.textContainerInset.left - textView.contentOffset.x
        rect.origin.y += textView.textContainerInset.top - textView.contentOffset.y
        return rect
    }

    private func assertPendingNativeAutocorrectSurvivesInputTraitChange(
        _ applyTraitChange: (EditorTextView) -> Void,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }

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

        XCTAssertTrue(view.textView.becomeFirstResponder(), file: file, line: line)
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        applyTraitChange(view.textView)

        XCTAssertEqual(editorGetHtml(id: editorId), "<p>the </p>", file: file, line: line)
        XCTAssertEqual(view.textView.textStorage.string, "the ", file: file, line: line)
        XCTAssertEqual(view.textView.reconciliationCount, 0, file: file, line: line)
    }

    private func beginEmptyMarkedComposition(
        in view: RichTextEditorView,
        utf16Offset: Int,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        setCollapsedSelection(in: view.textView, utf16Offset: utf16Offset)
        flushMainQueue()
        XCTAssertTrue(view.textView.becomeFirstResponder(), file: file, line: line)
        view.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
    }

    private func assertPendingNativeAutocorrectSurvivesAccessoryChange(
        initialHTML: String = "<p>teh </p>",
        selectionOffset: Int = 4,
        configure: ((NativeEditorExpoView, UInt64) -> Void)? = nil,
        _ applyAccessoryChange: (NativeEditorExpoView) -> Void,
        verify: ((NativeEditorExpoView, UInt64, StaticString, UInt) -> Void)? = nil,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        _ = editorSetHtml(id: editorId, html: initialHTML)

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        configure?(view, editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: selectionOffset)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder(), file: file, line: line)
        let expectedText = view.richTextView.textView.textStorage.string.replacingOccurrences(
            of: "teh",
            with: "the"
        )
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        applyAccessoryChange(view)
        flushMainQueue()

        XCTAssertEqual(
            editorGetHtml(id: editorId),
            initialHTML.replacingOccurrences(of: "teh", with: "the"),
            file: file,
            line: line
        )
        XCTAssertEqual(view.richTextView.textView.textStorage.string, expectedText, file: file, line: line)
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0, file: file, line: line)
        verify?(view, editorId, file, line)
    }

    private func aliceMentionAddonsJson() -> String {
        """
        {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
        """
    }

    private func aliceMentionSuggestion() -> NativeMentionSuggestion {
        NativeMentionSuggestion(dictionary: [
            "key": "alice",
            "title": "Alice Chen",
            "subtitle": "Design",
            "label": "@alice",
            "attrs": ["id": "user_alice", "label": "@alice"],
        ])!
    }

    private func hostEditorView(_ view: RichTextEditorView) -> UIWindow {
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        view.layoutIfNeeded()
        return window
    }

    private func hostNativeEditorExpoView(_ view: NativeEditorExpoView) -> UIWindow {
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        view.layoutIfNeeded()
        return window
    }

    private func flushMainQueue() {
        let expectation = expectation(description: "flush main queue")
        DispatchQueue.main.async {
            expectation.fulfill()
        }
        wait(for: [expectation], timeout: 1.0)
    }

    private func currentSelection(in editorId: UInt64) -> [String: Any] {
        let data = editorGetSelection(id: editorId).data(using: .utf8)
        XCTAssertNotNil(data)
        let json = try? JSONSerialization.jsonObject(with: data ?? Data()) as? [String: Any]
        XCTAssertNotNil(json)
        return json ?? [:]
    }

    private func parseJSONObject(_ json: String?) -> [String: Any] {
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

    private func jsonInt(_ value: Any?) -> Int? {
        if let value = value as? Int {
            return value
        }
        if let value = value as? NSNumber {
            return value.intValue
        }
        return nil
    }

    private func activeState(in editorId: UInt64) -> (insertableNodes: [String], allowedMarks: [String]) {
        let data = editorGetCurrentState(id: editorId).data(using: .utf8)
        XCTAssertNotNil(data)
        let json = try? JSONSerialization.jsonObject(with: data ?? Data()) as? [String: Any]
        let activeState = json?["activeState"] as? [String: Any]
        let insertableNodes = (activeState?["insertableNodes"] as? [String]) ?? []
        let allowedMarks = (activeState?["allowedMarks"] as? [String]) ?? []
        return (insertableNodes: insertableNodes, allowedMarks: allowedMarks)
    }

    private func forceDraw(_ textView: EditorTextView) {
        let renderer = UIGraphicsImageRenderer(bounds: textView.bounds)
        _ = renderer.image { context in
            textView.layer.render(in: context.cgContext)
        }
    }

    private func mentionEditorConfigJson() -> String {
        let config: [String: Any] = [
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

private final class EditorTextViewDelegateSpy: NSObject, EditorTextViewDelegate {
    var selectionChanges: [(anchor: UInt32, head: UInt32)] = []
    var receivedUpdates: [String] = []

    func editorTextView(_ textView: EditorTextView, selectionDidChange anchor: UInt32, head: UInt32) {
        selectionChanges.append((anchor: anchor, head: head))
    }

    func editorTextView(_ textView: EditorTextView, didReceiveUpdate updateJSON: String) {
        receivedUpdates.append(updateJSON)
    }
}
