import XCTest

final class RichTextEditorViewTests: XCTestCase {
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
        let editorId = editorCreate(configJson: "{}")
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
                "label": "@alice",
                "attrs": ["label": "@alice"],
            ])!,
            NativeMentionSuggestion(dictionary: [
                "key": "ben",
                "title": "Ben Ortiz",
                "subtitle": "Engineering",
                "label": "@ben",
                "attrs": ["label": "@ben"],
            ])!,
        ])

        XCTAssertTrue(didChange)
        XCTAssertEqual(toolbar.intrinsicContentSize.height, baseHeight + 2)
        XCTAssertTrue(toolbar.isShowingMentionSuggestions)
    }

    func testToolbarThemeParsesNativeAppearance() {
        let theme = EditorTheme(dictionary: [
            "toolbar": [
                "appearance": "native",
            ],
        ])

        XCTAssertEqual(theme.toolbar?.appearance, .native)
        XCTAssertEqual(theme.toolbar?.resolvedKeyboardOffset ?? 0, 6, accuracy: 0.1)
        XCTAssertEqual(theme.toolbar?.resolvedHorizontalInset ?? 0, 10, accuracy: 0.1)
    }

    func testAccessoryToolbarAppliesNativeAppearanceChrome() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))

        XCTAssertTrue(toolbar.usesNativeAppearanceForTesting)
        if #available(iOS 26.0, *) {
            XCTAssertTrue(toolbar.usesUIGlassEffectForTesting)
            XCTAssertEqual(toolbar.chromeBorderWidthForTesting, 1 / UIScreen.main.scale, accuracy: 0.1)
        } else {
            XCTAssertEqual(toolbar.chromeBorderWidthForTesting, 1 / UIScreen.main.scale, accuracy: 0.1)
        }
        XCTAssertEqual(toolbar.intrinsicContentSize.height, 56, accuracy: 0.1)
    }

    func testAccessoryToolbarAppliesSelectedStateForActiveNativeButton() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.applyBoldStateForTesting(active: true, enabled: true)

        XCTAssertEqual(toolbar.selectedButtonCountForTesting, 1)
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
