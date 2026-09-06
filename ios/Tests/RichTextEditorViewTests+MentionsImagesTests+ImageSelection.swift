import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testBackspaceBelowHorizontalRuleReplacesItWithParagraph() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 3, scalarHead: 3)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        textView.performToolbarInsertNode("horizontal_rule")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>Hello</p><hr><p></p>",
            "toolbar hr insert should create a trailing empty paragraph"
        )

        textView.deleteBackward()
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>Hello</p><p></p>",
            "backspacing below an hr should replace it with an empty paragraph"
        )

        textView.insertText("B")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>Hello</p><p>B</p>",
            "typing after hr removal should stay in the replacement paragraph"
        )
    }

    func testTypingAndBackspacingAroundImageUsesTrailingParagraphCaret() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello</p>")

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 3, scalarHead: 3)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        let imageFragmentJson = """
        {"type":"doc","content":[{"type":"image","attrs":{"src":"https://example.com/cat.png","alt":"Cat"}}]}
        """
        let updateJSON = EditorV2Shadow.insertContentJsonAtSelectionScalar(
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
        let htmlAfterTyping = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(htmlAfterTyping.starts(with: "<p>Hello</p><img "))
        XCTAssertTrue(htmlAfterTyping.contains("src=\"https://example.com/cat.png\""))
        XCTAssertTrue(htmlAfterTyping.contains("alt=\"Cat\""))
        XCTAssertTrue(
            htmlAfterTyping.hasSuffix("<p>B</p>"),
            "typing after image insert should land in the trailing paragraph"
        )

        textView.deleteBackward()
        let htmlAfterFirstBackspace = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(htmlAfterFirstBackspace.starts(with: "<p>Hello</p><img "))
        XCTAssertTrue(htmlAfterFirstBackspace.contains("src=\"https://example.com/cat.png\""))
        XCTAssertTrue(htmlAfterFirstBackspace.contains("alt=\"Cat\""))
        XCTAssertTrue(
            htmlAfterFirstBackspace.hasSuffix("<p></p>"),
            "first backspace should delete the trailing paragraph text"
        )

        textView.deleteBackward()
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>Hello</p><p></p>",
            "second backspace from the empty trailing paragraph should replace the image with a paragraph"
        )
    }

    func testSelectingImageShowsResizeOverlayAndPersistsResizedDimensions() {
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

        let html = EditorV2Shadow.getHtml(id: editorId)
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

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

}
