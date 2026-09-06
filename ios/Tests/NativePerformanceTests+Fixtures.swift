import XCTest
import UIKit

enum NativePerformanceFixtureFactory {
    private static let blockCount = 96
    private static let paragraphCharacterCount = 180

    struct ParagraphSplitSession {
        let editorId: UInt64
        let textView: EditorTextView
        let splitOffset: Int
        let initialTextLength: Int
    }

    struct HostedParagraphSplitSession {
        let editorId: UInt64
        let window: UIWindow
        let view: RichTextEditorView
        let splitOffset: Int
        let initialTextLength: Int
    }

    static func largeRenderJSON() -> String {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        return EditorV2Shadow.setJson(id: editorId, json: largeDocumentJSONString())
    }

    static func loadLargeDocument(into editorId: UInt64) -> String {
        EditorV2Shadow.setJson(id: editorId, json: largeDocumentJSONString())
    }

    static func remoteSelections(
        editorId: UInt64,
        peerCount: Int = 6,
        selectionWidth: Int = 0
    ) -> [RemoteSelectionDecoration] {
        let totalScalar = EditorV2Shadow.docToScalar(id: editorId, docPos: editorDocumentContentSize(id: editorId))
        let upperBound = max(1, Int(totalScalar > 0 ? totalScalar - 1 : 0))
        let samplePoints = evenlySpacedValues(from: 1, through: upperBound, count: peerCount)

        return samplePoints.enumerated().map { index, scalar in
            let headScalar = (selectionWidth > 0 && !index.isMultiple(of: 2))
                ? min(upperBound, scalar + selectionWidth)
                : scalar
            let anchorDoc = EditorV2Shadow.scalarToDoc(id: editorId, scalar: UInt32(scalar))
            let headDoc = EditorV2Shadow.scalarToDoc(id: editorId, scalar: UInt32(headScalar))
            return RemoteSelectionDecoration(
                clientId: String(index + 1),
                anchor: anchorDoc,
                head: headDoc,
                color: indexedColor(index),
                name: "Peer \(index + 1)",
                isFocused: true
            )
        }
    }

    static func typingCursorOffset(in textView: UITextView) -> Int {
        selectionScrubOffsets(in: textView, points: 1).first ?? 0
    }

    static func paragraphSplitSessions(count: Int, autoGrow: Bool = false) -> [ParagraphSplitSession] {
        (0..<count).map { _ in
            let editorId = makeV2Editor()
            _ = loadLargeDocument(into: editorId)

            let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
            textView.heightBehavior = autoGrow ? .autoGrow : .fixed
            textView.captureApplyUpdateTraceForTesting = true
            textView.bindEditor(id: editorId)
            textView.layoutIfNeeded()

            return ParagraphSplitSession(
                editorId: editorId,
                textView: textView,
                splitOffset: paragraphSplitCursorOffset(in: textView),
                initialTextLength: textView.attributedText.length
            )
        }
    }

    static func hostedParagraphSplitSessions(count: Int) -> [HostedParagraphSplitSession] {
        (0..<count).map { _ in
            let editorId = makeV2Editor()

            let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 390, height: 0))
            let window = hostEditorView(view, size: CGSize(width: 390, height: 844))
            view.heightBehavior = .autoGrow
            view.textView.captureApplyUpdateTraceForTesting = true
            view.editorId = editorId
            view.setContent(json: largeDocumentJSONString())
            flushMainQueue()

            let measuredHeight = ceil(view.intrinsicContentSize.height)
            view.frame.size.height = measuredHeight
            view.layoutIfNeeded()

            return HostedParagraphSplitSession(
                editorId: editorId,
                window: window,
                view: view,
                splitOffset: paragraphSplitCursorOffset(in: view.textView),
                initialTextLength: view.textView.attributedText.length
            )
        }
    }

    static func selectionScrubOffsets(in textView: UITextView, points: Int) -> [Int] {
        let candidates = visibleCharacterOffsets(in: textView.textStorage.string as NSString)
        guard !candidates.isEmpty else { return [0] }
        return evenlySpacedValues(from: 0, through: candidates.count - 1, count: points).map { candidates[$0] }
    }

    static func paragraphSplitCursorOffset(in textView: UITextView) -> Int {
        let text = textView.textStorage.string as NSString
        let firstBlockBreak = (0..<text.length).first { index in
            let character = text.character(at: index)
            return character == 0x000A || character == 0x000D
        }

        guard let firstBlockBreak else {
            return typingCursorOffset(in: textView)
        }

        let paragraphOffsets = visibleCharacterOffsets(in: text).filter { $0 > firstBlockBreak }
        guard !paragraphOffsets.isEmpty else {
            return typingCursorOffset(in: textView)
        }

        return paragraphOffsets[min(32, paragraphOffsets.count - 1)]
    }

    private static func largeDocumentJSONString() -> String {
        let jsonObject: [String: Any] = [
            "type": "doc",
            "content": largeDocumentContent(),
        ]
        let data = try! JSONSerialization.data(withJSONObject: jsonObject, options: [])
        return String(data: data, encoding: .utf8)!
    }

    private static func largeDocumentContent() -> [[String: Any]] {
        var content: [[String: Any]] = [
            [
                "type": "heading",
                "attrs": ["level": 1],
                "content": [textNode(textFragment(seed: 10_000, minCharacterCount: 40))],
            ],
        ]

        for index in 0..<blockCount {
            if index % 12 == 5 {
                content.append([
                    "type": "blockquote",
                    "content": [[
                        "type": "paragraph",
                        "content": richInlineContent(seed: index, totalCharacters: paragraphCharacterCount),
                    ]],
                ])
                continue
            }

            if index % 9 == 3 {
                content.append([
                    "type": "heading",
                    "attrs": ["level": 2],
                    "content": [textNode(textFragment(seed: index + 2_000, minCharacterCount: 72))],
                ])
                continue
            }

            content.append([
                "type": "paragraph",
                "content": richInlineContent(seed: index, totalCharacters: paragraphCharacterCount),
            ])
        }

        return content
    }

    private static func richInlineContent(seed: Int, totalCharacters: Int) -> [[String: Any]] {
        let text = textFragment(seed: seed, minCharacterCount: totalCharacters)
        let characters = Array(text)
        let count = characters.count
        let cutA = count / 4
        let cutB = count / 2
        let cutC = (count * 3) / 4

        let segments: [(String, [[String: Any]]?)] = [
            (String(characters[0..<cutA]), nil),
            (String(characters[cutA..<cutB]), [["type": "bold"]]),
            (String(characters[cutB..<cutC]), [["type": "italic"]]),
            (
                String(characters[cutC..<count]),
                [[
                    "type": "link",
                    "attrs": [
                        "href": "https://example.com/item/\(seed)",
                        "target": "_blank",
                        "rel": "noopener noreferrer nofollow",
                        "class": NSNull(),
                        "title": NSNull(),
                    ],
                ]]
            ),
        ]

        return segments.compactMap { text, marks in
            guard !text.isEmpty else { return nil }
            return textNode(text, marks: marks)
        }
    }

    static func textNode(_ text: String, marks: [[String: Any]]? = nil) -> [String: Any] {
        var node: [String: Any] = [
            "type": "text",
            "text": text,
        ]
        if let marks, !marks.isEmpty {
            node["marks"] = marks
        }
        return node
    }

    private static func textFragment(seed: Int, minCharacterCount: Int) -> String {
        let words = [
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
            "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo",
            "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
        ]

        var result = ""
        var cursor = 0
        while result.count < minCharacterCount {
            if !result.isEmpty {
                result.append(" ")
            }
            result.append(words[(seed + cursor) % words.count])
            cursor += 1
        }
        return String(result.prefix(minCharacterCount))
    }

    private static func indexedColor(_ index: Int) -> UIColor {
        let colors: [UIColor] = [
            .systemBlue,
            .systemGreen,
            .systemOrange,
            .systemPink,
            .systemPurple,
            .systemTeal,
        ]
        return colors[index % colors.count]
    }

    private static func visibleCharacterOffsets(in text: NSString) -> [Int] {
        (0..<text.length).compactMap { index in
            switch text.character(at: index) {
            case 0xFFFC, 0x200B, 0x000A, 0x000D:
                return nil
            default:
                return index
            }
        }
    }

    private static func evenlySpacedValues(from start: Int, through end: Int, count: Int) -> [Int] {
        guard count > 1, end > start else {
            return [min(start, end)]
        }

        return (0..<count).map { index in
            start + Int((Double(end - start) * Double(index) / Double(count - 1)).rounded(.toNearestOrAwayFromZero))
        }
    }

    private static func editorDocumentContentSize(id: UInt64) -> UInt32 {
        guard let data = EditorV2Shadow.getJson(id: id).data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return 0
        }
        let children = json["content"] as? [[String: Any]] ?? []
        return children.reduce(UInt32(0)) { partial, child in
            partial + nodeSize(child)
        }
    }

    private static func nodeSize(_ node: [String: Any]) -> UInt32 {
        let type = node["type"] as? String ?? ""
        if type == "text" {
            let text = node["text"] as? String ?? ""
            return UInt32(text.count)
        }

        if isVoidNode(type) {
            return 1
        }

        let children = node["content"] as? [[String: Any]] ?? []
        let childrenSize = children.reduce(UInt32(0)) { partial, child in
            partial + nodeSize(child)
        }

        return 1 + childrenSize + 1
    }

    private static func isVoidNode(_ type: String) -> Bool {
        switch type {
        case "horizontal_rule", "hard_break", "image", "mention":
            return true
        default:
            return false
        }
    }
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

func hostEditorView(_ view: RichTextEditorView, size: CGSize) -> UIWindow {
    let window = UIWindow(frame: CGRect(origin: .zero, size: size))
    let viewController = UIViewController()
    window.rootViewController = viewController
    window.makeKeyAndVisible()
    view.frame = CGRect(origin: .zero, size: size)
    viewController.view.addSubview(view)
    view.layoutIfNeeded()
    return window
}

func flushMainQueue() {
    let expectation = XCTestExpectation(description: "flush main queue")
    DispatchQueue.main.async {
        expectation.fulfill()
    }
    XCTWaiter().wait(for: [expectation], timeout: 1.0)
}
