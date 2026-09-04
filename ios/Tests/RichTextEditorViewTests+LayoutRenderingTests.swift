import XCTest
import ExpoModulesCore

private final class AutoGrowStyleTrackingNativeEditorView: NativeEditorExpoView {
    private(set) var publishedStyleHeights: [CGFloat?] = []

    required init(appContext: AppContext? = nil) {
        super.init(appContext: appContext)
    }

    override func setStyleSize(_ width: NSNumber?, height: NSNumber?) {
        publishedStyleHeights.append(height.map { CGFloat($0.doubleValue) })
        super.setStyleSize(width, height: height)
    }
}


extension RichTextEditorViewTests {
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 0))
        view.heightBehavior = .autoGrow
        view.editorId = editorId
        view.setContent(html: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")
        view.layoutIfNeeded()

        let intrinsic = view.intrinsicContentSize

        XCTAssertEqual(intrinsic.width, UIView.noIntrinsicMetric, accuracy: 0.1)
        XCTAssertGreaterThan(intrinsic.height, 0)
    }

    func testNativeEditorRemeasuresAfterSwitchingFromFixedToAutoGrow() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 200)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.richTextView.setContent(html: (1 ... 12).map { "<p>Line \($0)</p>" }.joined())
        view.setHeightBehavior("fixed")
        view.layoutIfNeeded()

        let expectedHeight = ceil(
            view.richTextView.textView.sizeThatFits(
                CGSize(width: 320, height: CGFloat.greatestFiniteMagnitude)
            ).height
        )
        XCTAssertGreaterThan(expectedHeight, view.bounds.height)

        view.setHeightBehavior("autoGrow")
        flushMainQueue()
        view.layoutIfNeeded()

        XCTAssertEqual(view.intrinsicContentSize.height, expectedHeight, accuracy: 1.0)
    }

    func testNativeEditorAutoGrowPublishesFabricStyleHeightDuringNativeLayout() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = AutoGrowStyleTrackingNativeEditorView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 0)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        view.richTextView.setContent(html: "<p>Alpha</p>")
        view.setHeightBehavior("autoGrow")
        view.layoutIfNeeded()

        let initialHeight = try XCTUnwrap(view.publishedStyleHeights.compactMap { $0 }.last)
        let initialPublicationCount = view.publishedStyleHeights.count
        view.richTextView.setContent(html: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")
        view.layoutIfNeeded()

        let publishedHeight = try XCTUnwrap(view.publishedStyleHeights.compactMap { $0 }.last)
        XCTAssertGreaterThan(publishedHeight, initialHeight)
        XCTAssertGreaterThan(view.publishedStyleHeights.count, initialPublicationCount)

        let publicationCount = view.publishedStyleHeights.count
        view.richTextView.onHeightMayChange?(publishedHeight)
        view.richTextView.onHeightMayChange?(publishedHeight)
        XCTAssertEqual(view.publishedStyleHeights.count, publicationCount)

        view.frame.size.height = max(1, publishedHeight - 20)
        view.setNeedsLayout()
        view.layoutIfNeeded()
        view.setNeedsLayout()
        view.layoutIfNeeded()
        XCTAssertEqual(view.publishedStyleHeights.count, publicationCount)

        view.setHeightBehavior("fixed")
        XCTAssertNil(view.publishedStyleHeights.last!)
    }

    func testApplyThemeRerendersExistingContentWhenTextIsUnchanged() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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

    func testAppearanceChangeForcesFullRenderForEmptyRenderPatch() {
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.applyTheme(EditorTheme(dictionary: [
            "text": ["color": "#112233"],
        ]))
        textView.applyUpdateJSON("""
        {
          "renderBlocks": [[
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"Alpha","marks":[]},
            {"type":"blockEnd"}
          ]]
        }
        """, notifyDelegate: false)

        textView.applyTheme(EditorTheme(dictionary: [
            "text": ["color": "#DDEEFF"],
        ]))
        textView.applyUpdateJSON("""
        {
          "renderPatch": {
            "startIndex": 0,
            "deleteCount": 0,
            "renderBlocks": []
          }
        }
        """, notifyDelegate: false)

        let color = textView.textStorage.attribute(
            .foregroundColor,
            at: 0,
            effectiveRange: nil
        ) as? UIColor
        XCTAssertEqual(color, EditorTheme.color(from: "#DDEEFF"))
        XCTAssertFalse(textView.lastRenderAppliedPatch())
    }

    func testEditorTextViewMeasuredAutoGrowHeightMatchesSizeThatFits() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.bindEditor(id: editorId, initialHTML: "<blockquote><p>Hello</p></blockquote><p>World</p>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 10, scalarHead: 10)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.bindEditor(id: editorId, initialHTML: "<blockquote><p>Hello</p></blockquote><p>World</p>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 11, scalarHead: 11)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.insertText("!")
        textView.layoutIfNeeded()

        let html = EditorV2Shadow.getHtml(id: editorId)
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<blockquote><p>Hello</p></blockquote>")
        textView.layoutIfNeeded()

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 6, scalarHead: 6)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<blockquote><p>Hello</p></blockquote>")
        textView.layoutIfNeeded()

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 6, scalarHead: 6)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 260))
        textView.bindEditor(id: editorId, initialHTML: "<p>Intro</p><blockquote><p>Hello</p></blockquote>")
        textView.layoutIfNeeded()

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 11, scalarHead: 11)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<ul><li><p>A</p></li></ul>")

        let firstUpdate = EditorV2Shadow.insertNodeAtSelectionScalar(
            id: editorId,
            scalarAnchor: 3,
            scalarHead: 3,
            nodeType: "hard_break"
        )
        XCTAssertFalse(firstUpdate.isEmpty)
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>A<br></p></li></ul>",
            "first hardBreak should preserve the existing list item text"
        )

        let secondUpdate = EditorV2Shadow.insertNodeAtSelectionScalar(
            id: editorId,
            scalarAnchor: 4,
            scalarHead: 4,
            nodeType: "hard_break"
        )
        XCTAssertFalse(secondUpdate.isEmpty)
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>A<br><br></p></li></ul>",
            "second hardBreak at the next scalar position should preserve the original text"
        )
    }

    func testToolbarHardBreakTwiceInListItemPreservesExistingText() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: .zero)
        textView.bindEditor(id: editorId, initialHTML: "<ul><li><p>A</p></li></ul>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 3, scalarHead: 3)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        textView.performToolbarInsertNode("hard_break")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>A<br></p></li></ul>",
            "first hardBreak should preserve the existing list item text"
        )

        textView.performToolbarInsertNode("hard_break")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>A<br><br></p></li></ul>",
            "second hardBreak should append after the first one rather than replacing the text"
        )
    }

    func testToolbarHardBreakMovesCaretToNextVisualLine() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
        ])

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.applyTheme(theme)
        textView.bindEditor(id: editorId, initialHTML: "<p>A</p>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 1, scalarHead: 1)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        guard let beforePosition = textView.selectedTextRange?.start else {
            XCTFail("expected initial caret position")
            return
        }
        let beforeCaretRect = textView.caretRect(for: beforePosition)

        textView.performToolbarInsertNode("hard_break")
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
        ])

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 220, height: 200))
        textView.applyTheme(theme)
        textView.bindEditor(id: editorId, initialHTML: "<p>A</p>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 1, scalarHead: 1)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.performToolbarInsertNode("hard_break")
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 220))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 3, scalarHead: 3)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.layoutIfNeeded()

        textView.performToolbarInsertNode("horizontal_rule")
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
            EditorV2Shadow.getHtml(id: editorId),
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

    private func firstHorizontalRuleRange(in textView: UITextView) -> NSRange? {
        guard textView.textStorage.length > 0 else { return nil }

        for index in 0..<textView.textStorage.length {
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            if attrs[.attachment] is NSTextAttachment,
               (attrs[RenderBridgeAttributes.voidNodeType] as? String)
                .map(EditorNodeTypes.isHorizontalRule) == true
            {
                return NSRange(location: index, length: 1)
            }
        }

        return nil
    }

    private func forceDraw(_ textView: EditorTextView) {
        let renderer = UIGraphicsImageRenderer(bounds: textView.bounds)
        _ = renderer.image { context in
            textView.layer.render(in: context.cgContext)
        }
    }

}
