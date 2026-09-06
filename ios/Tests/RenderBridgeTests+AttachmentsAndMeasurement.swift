import XCTest
import CoreText

extension RenderBridgeTests {
    func testHorizontalRuleAttachment_bounds() {
        let attachment = HorizontalRuleAttachment()
        let proposedRect = CGRect(x: 0, y: 0, width: 320, height: 20)
        let bounds = attachment.attachmentBounds(
            for: nil,
            proposedLineFragment: proposedRect,
            glyphPosition: .zero,
            characterIndex: 0
        )

        XCTAssertEqual(
            bounds.width, 320,
            "Attachment width should match proposed line fragment width"
        )
        let expectedHeight = 1.0 + (8.0 * 2)  // line + padding
        XCTAssertEqual(
            bounds.height, expectedHeight,
            "Attachment height should be line height + 2 * vertical padding"
        )
    }

    func testHorizontalRuleAttachment_rendersImage() {
        let attachment = HorizontalRuleAttachment()
        attachment.lineColor = .red
        let bounds = CGRect(x: 0, y: 0, width: 200, height: 17)
        let image = attachment.image(
            forBounds: bounds,
            textContainer: nil,
            characterIndex: 0
        )
        XCTAssertNotNil(image, "HorizontalRuleAttachment should produce a non-nil image")
    }

    func testMeasureHeightForSingleParagraph() {
        let renderJSON = """
        [
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"Hello world"},
            {"type":"blockEnd"}
        ]
        """
        let height = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: nil,
            width: 375
        )
        XCTAssertGreaterThan(height, 0, "Single paragraph should have positive height")
    }

    func testMeasureHeightFromBackgroundWaitsForMainThreadMeasurement() {
        let finished = expectation(description: "background measurement finished")
        let started = DispatchSemaphore(value: 0)
        let completion = DispatchSemaphore(value: 0)
        let renderJSON = """
        [
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"Measured on main"},
            {"type":"blockEnd"}
        ]
        """

        DispatchQueue.global(qos: .userInitiated).async {
            started.signal()
            _ = RenderBridge.measureHeight(
                forRenderJSON: renderJSON,
                themeJSON: nil,
                width: 320
            )
            completion.signal()
            finished.fulfill()
        }

        XCTAssertEqual(started.wait(timeout: .now() + 1), .success)
        XCTAssertEqual(
            completion.wait(timeout: .now() + 0.1),
            .timedOut,
            "background callers must synchronously marshal UIKit measurement to the main thread"
        )
        wait(for: [finished], timeout: 1)
    }

    func testMeasureHeightForEmptyContent() {
        let renderJSON = "[]"
        let height = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: nil,
            width: 375
        )
        XCTAssertEqual(height, 0, "Empty content should have zero height")
    }

    func testMeasureHeightRespectsWidth() {
        let longText = String(repeating: "word ", count: 100)
        let renderJSON = """
        [
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"\(longText)"},
            {"type":"blockEnd"}
        ]
        """
        let narrowHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: nil,
            width: 100
        )
        let wideHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: nil,
            width: 1000
        )
        XCTAssertGreaterThan(narrowHeight, wideHeight, "Narrower width should produce taller height")
    }

    func testMeasureHeightRespectsThemeFontSize() {
        let renderJSON = """
        [
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"Hello world"},
            {"type":"blockEnd"}
        ]
        """
        let smallTheme = """
        {"text":{"fontSize":12}}
        """
        let largeTheme = """
        {"text":{"fontSize":32}}
        """
        let smallHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: smallTheme,
            width: 375
        )
        let largeHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: largeTheme,
            width: 375
        )
        XCTAssertGreaterThan(largeHeight, smallHeight, "Larger font should produce taller height")
    }

    func testMeasureHeightRespectsContentInsets() {
        let renderJSON = """
        [
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"Hello world"},
            {"type":"blockEnd"}
        ]
        """
        let noInsetHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: nil,
            width: 375
        )
        let insetTheme = """
        {"contentInsets":{"top":20,"bottom":20}}
        """
        let insetHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: insetTheme,
            width: 375
        )
        XCTAssertEqual(insetHeight, noInsetHeight + 40, accuracy: 1.0, "Content insets should add to height")
    }

    func testRender_imageAttachmentHonorsPreferredDimensions() {
        let json = """
        [
            {"type": "voidBlock", "nodeType": "image", "docPos": 1, "attrs": {
                "src": "https://example.com/cat.png",
                "width": 140,
                "height": 80
            }}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, LayoutConstants.objectReplacementCharacter)

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let attachment = attrs[.attachment] as? NSTextAttachment
        XCTAssertNotNil(attachment, "Image render should produce an attachment")

        let bounds = attachment?.attachmentBounds(
            for: nil,
            proposedLineFragment: CGRect(x: 0, y: 0, width: 320, height: 24),
            glyphPosition: .zero,
            characterIndex: 0
        )

        XCTAssertEqual(bounds?.width ?? 0, 140, accuracy: 0.1)
        XCTAssertEqual(bounds?.height ?? 0, 80, accuracy: 0.1)
    }

}
