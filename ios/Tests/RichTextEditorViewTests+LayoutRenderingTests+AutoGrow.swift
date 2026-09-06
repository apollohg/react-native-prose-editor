import XCTest
import ExpoModulesCore

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

}
