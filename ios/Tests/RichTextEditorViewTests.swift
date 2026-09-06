import XCTest
import ExpoModulesCore

final class RichTextEditorViewTests: XCTestCase {

    /// Attaches `toolbar` to a fixed-width host so Auto Layout has a real width to resolve
    /// `.fill`-distributed arranged subviews against. `EditorAccessoryToolbarView` sets
    /// `translatesAutoresizingMaskIntoConstraints = false` on itself with no width/height
    /// constraint of its own (it self-sizes as an input accessory view in production, where
    /// the keyboard/window supplies its width) — without a host, `layoutIfNeeded()` on the bare
    /// view collapses every flexible-width arranged subview to zero instead of distributing
    /// space, which would make a "does not overlap" assertion pass vacuously.
    static func attachToFixedWidthHost(_ toolbar: EditorAccessoryToolbarView, width: CGFloat) -> UIView {
        let host = UIView(frame: CGRect(x: 0, y: 0, width: width, height: 100))
        host.addSubview(toolbar)
        NSLayoutConstraint.activate([
            toolbar.leadingAnchor.constraint(equalTo: host.leadingAnchor),
            toolbar.trailingAnchor.constraint(equalTo: host.trailingAnchor),
            toolbar.topAnchor.constraint(equalTo: host.topAnchor),
        ])
        return host
    }

    static let placementToolbarFixtureJSON = """
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

    func activeState(in editorId: UInt64) -> (insertableNodes: [String], allowedMarks: [String]) {
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
final class ForwardingCompositeTextViewDelegateSpy: NSObject, UITextViewDelegate {
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

final class KeyboardProviderTextViewDelegateSpy: NSObject, UITextViewDelegate {
    weak var textViewDelegate: UITextViewDelegate?
    var selectionChangeCount = 0
    var textChangeCount = 0

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
