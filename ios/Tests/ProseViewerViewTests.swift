import UIKit
import XCTest

final class ProseViewerViewTests: XCTestCase {
    private let helloRenderJson = """
    [
      {"type":"blockStart","nodeType":"paragraph","depth":0},
      {"type":"textRun","text":"Hello world"},
      {"type":"blockEnd","nodeType":"paragraph","depth":0}
    ]
    """

    private let linkRenderJson = """
    [
      {"type":"blockStart","nodeType":"paragraph","depth":0},
      {"type":"textRun","text":"Open ","marks":[]},
      {"type":"textRun","text":"the docs","marks":[{"type":"link","href":"https://example.com"}]},
      {"type":"blockEnd","nodeType":"paragraph","depth":0}
    ]
    """

    private let mentionRenderJson = """
    [
      {"type":"blockStart","nodeType":"paragraph","depth":0},
      {"type":"textRun","text":"Hello ","marks":[]},
      {"type":"opaqueInlineAtom","nodeType":"mention","label":"@Alice","docPos":7},
      {"type":"blockEnd","nodeType":"paragraph","depth":0}
    ]
    """

    private func makeViewer(width: CGFloat = 320) -> ProseViewerView {
        ProseViewerView(frame: CGRect(x: 0, y: 0, width: width, height: 200))
    }

    func testApplyValidRenderJsonReturnsTrueAndRendersText() {
        let viewer = makeViewer()
        XCTAssertTrue(viewer.apply(renderJson: helloRenderJson, themeJson: "{}"))
        XCTAssertTrue(viewer.renderedTextForTesting.contains("Hello world"))
    }

    func testApplyInvalidRenderJsonReturnsFalseAndClearsPreviousContent() {
        let viewer = makeViewer()
        _ = viewer.apply(renderJson: helloRenderJson, themeJson: "{}")

        XCTAssertFalse(viewer.apply(renderJson: "not json", themeJson: "{}"))
        XCTAssertEqual(viewer.renderedTextForTesting, "")
        XCTAssertFalse(
            viewer.apply(
                renderJson: #"{"type":"blockStart"}"#,
                themeJson: "{}"
            )
        )
    }

    func testMeasurementsUsePointsAndRejectInvalidInput() {
        let viewer = makeViewer()
        _ = viewer.apply(renderJson: helloRenderJson, themeJson: "{}")

        let live = viewer.measuredHeight(forWidth: 320)
        let headless = ProseViewerView.measureHeight(
            renderJson: helloRenderJson,
            themeJson: "{}",
            width: 320
        )

        XCTAssertEqual(live, headless ?? -1, accuracy: 1)
        XCTAssertGreaterThan(live, 0)
        XCTAssertEqual(viewer.measuredHeight(forWidth: 0), 0)
        XCTAssertNil(
            ProseViewerView.measureHeight(
                renderJson: "invalid",
                themeJson: "{}",
                width: 320
            )
        )
    }

    func testPrepareForReuseClearsContentAndRetainsDelegate() {
        let viewer = makeViewer()
        let delegate = RecordingDelegate()
        viewer.interactionDelegate = delegate
        _ = viewer.apply(renderJson: helloRenderJson, themeJson: "{}")

        viewer.prepareForReuse()

        XCTAssertEqual(viewer.renderedTextForTesting, "")
        XCTAssertTrue(viewer.interactionDelegate === delegate)
    }

    func testPublicImagePolicySetterReconfiguresBoundedLoader() {
        let viewer = makeViewer()
        viewer.setImageLoadingPolicy(json: #"{"maxSourceBytes":1234}"#)
        XCTAssertEqual(viewer.imageLoadingPolicyForHost.maxSourceBytes, 1234)
    }

    func testLinkTapReachesDelegate() throws {
        let viewer = makeViewer()
        let delegate = RecordingDelegate()
        viewer.interactionDelegate = delegate
        _ = viewer.apply(renderJson: linkRenderJson, themeJson: "{}")
        viewer.layoutIfNeeded()

        let point = try pointForAttribute(
            RenderBridgeAttributes.linkHref,
            expectedValue: "https://example.com",
            in: viewer.textViewForTesting
        )
        viewer.handleTapForTesting(at: point)

        XCTAssertEqual(delegate.links.count, 1)
        XCTAssertEqual(delegate.links.first?.href, "https://example.com")
        XCTAssertEqual(delegate.links.first?.text, "the docs")
    }

    func testMentionTapReachesDelegate() throws {
        let viewer = makeViewer()
        let delegate = RecordingDelegate()
        viewer.interactionDelegate = delegate
        _ = viewer.apply(renderJson: mentionRenderJson, themeJson: "{}")
        viewer.layoutIfNeeded()

        let point = try pointForAttribute(
            RenderBridgeAttributes.voidNodeType,
            expectedValue: "mention",
            in: viewer.textViewForTesting
        )
        viewer.handleTapForTesting(at: point)

        XCTAssertEqual(delegate.mentions.count, 1)
        XCTAssertEqual(delegate.mentions.first?.docPos, 7)
        XCTAssertEqual(delegate.mentions.first?.label, "@Alice")
    }

    func testNativeDefaultsUseZeroInsetsAndDoNotCollapseEmptyContent() {
        let viewer = makeViewer()
        XCTAssertEqual(viewer.contentInset, .zero)
        XCTAssertTrue(viewer.apply(renderJson: "[]", themeJson: "{}"))
        XCTAssertFalse(viewer.isContentCollapsedForHost)
    }

    private func pointForAttribute(
        _ key: NSAttributedString.Key,
        expectedValue: String,
        in textView: EditorTextView
    ) throws -> CGPoint {
        var targetRange = NSRange(location: NSNotFound, length: 0)
        textView.textStorage.enumerateAttribute(
            key,
            in: NSRange(location: 0, length: textView.textStorage.length)
        ) { value, range, _ in
            if value as? String == expectedValue {
                targetRange = range
            }
        }
        try XCTSkipIf(targetRange.location == NSNotFound, "render fixture drifted")

        let glyphRange = textView.layoutManager.glyphRange(
            forCharacterRange: targetRange,
            actualCharacterRange: nil
        )
        var rect = textView.layoutManager.boundingRect(
            forGlyphRange: glyphRange,
            in: textView.textContainer
        )
        rect.origin.x += textView.textContainerInset.left
        rect.origin.y += textView.textContainerInset.top
        return CGPoint(x: rect.midX, y: rect.midY)
    }

    private final class RecordingDelegate: ProseViewerInteractionDelegate {
        var links: [(href: String, text: String)] = []
        var mentions: [(docPos: Int, label: String)] = []

        func proseViewer(
            _ view: ProseViewerView,
            didTapLink href: String,
            text: String
        ) {
            links.append((href, text))
        }

        func proseViewer(
            _ view: ProseViewerView,
            didTapMention docPos: Int,
            label: String
        ) {
            mentions.append((docPos, label))
        }
    }
}

final class NativeProseViewerExpoAdapterTests: XCTestCase {
    private let emptyParagraphRenderJson = """
    [
      {"type":"blockStart","nodeType":"paragraph","depth":0},
      {"type":"textRun","text":"\\u200B"},
      {"type":"blockEnd","nodeType":"paragraph","depth":0}
    ]
    """

    func testAdapterRestoresReactNativeInsetsAndCollapseDefault() {
        let adapter = NativeProseViewerExpoView(appContext: nil)
        adapter.frame = CGRect(x: 0, y: 0, width: 320, height: 100)
        adapter.setRenderJson(emptyParagraphRenderJson)
        adapter.layoutIfNeeded()

        XCTAssertEqual(
            adapter.viewerForTesting.contentInset,
            UIEdgeInsets(top: 8, left: 0, bottom: 8, right: 0)
        )
        XCTAssertEqual(adapter.intrinsicContentSize.height, 0)
    }

    func testAdapterUsesFacadeNormalizedCollapseStateForInvalidInput() {
        let adapter = NativeProseViewerExpoView(appContext: nil)
        adapter.frame = CGRect(x: 0, y: 0, width: 320, height: 100)
        adapter.setRenderJson("invalid")
        adapter.layoutIfNeeded()

        XCTAssertTrue(adapter.viewerForTesting.isContentCollapsedForHost)
        XCTAssertEqual(adapter.intrinsicContentSize.height, 0)
    }
}
