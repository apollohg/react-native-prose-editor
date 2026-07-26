import CoreText
import XCTest

final class PreparedProseRenderingTests: XCTestCase {
    func testPreparedFixtureCorpusCoversEveryTypedElementAndMark() throws {
        let fixtures: [Fixture] = [
            Fixture("paragraph", blocks: [.text("paragraph", "Paragraph")], kinds: [.text]),
            Fixture("heading-1", blocks: [.text("h1", "Heading 1")], kinds: [.text]),
            Fixture("heading-2", blocks: [.text("h2", "Heading 2")], kinds: [.text]),
            Fixture("heading-3", blocks: [.text("h3", "Heading 3")], kinds: [.text]),
            Fixture("heading-4", blocks: [.text("h4", "Heading 4")], kinds: [.text]),
            Fixture("heading-5", blocks: [.text("h5", "Heading 5")], kinds: [.text]),
            Fixture("heading-6", blocks: [.text("h6", "Heading 6")], kinds: [.text]),
            Fixture(
                "nested ordered bullet and task lists",
                blocks: [
                    .list("ordered", depth: 1, index: 3, text: "third"),
                    .list("bullet", depth: 2, index: 1, text: "nested bullet"),
                    .task(depth: 3, checked: true, text: "nested task"),
                ],
                kinds: [.text, .marker]
            ),
            Fixture("blockquote", blocks: [.quote("Quoted prose")], kinds: [.text, .border]),
            Fixture("code block", blocks: [.text("codeBlock", "let x = 1")], kinds: [.text, .background]),
            Fixture("horizontal rule", blocks: [.rule], kinds: [.rule]),
            Fixture(
                "hard break",
                blocks: [.inline("paragraph", [.text("first", []), .atom("hardBreak", ""), .text("second", [])])],
                kinds: [.text]
            ),
            Fixture("opaque inline atom", blocks: [.inline("paragraph", [.atom("__opaque", "opaque")])], kinds: [.atom]),
            Fixture("opaque block atom", blocks: [.blockAtom("__opaque", "opaque block")], kinds: [.atom]),
            Fixture("mention", blocks: [.inline("paragraph", [.atom("mention", "@Ada")])], kinds: [.atom]),
            Fixture(
                "link",
                blocks: [.inline("paragraph", [.text("link", [.mark("link", #"{"href":"https://example.test"}"#)])])],
                kinds: [.text]
            ),
            Fixture(
                "all marks",
                blocks: [.inline("paragraph", [
                    .text("bold ", [.mark("bold")]),
                    .text("italic ", [.mark("italic")]),
                    .text("underline ", [.mark("underline")]),
                    .text("strike ", [.mark("strike")]),
                    .text("code ", [.mark("code")]),
                    .text("colour ", [.mark("textColor", #"{"color":"#C00020"}"#)]),
                    .text("highlight ", [.mark("highlight", #"{"color":"#FFF176"}"#)]),
                    .text("family size", [.mark("textStyle", #"{"fontFamily":"Courier","fontSize":19}"#)]),
                ])],
                kinds: [.text]
            ),
            Fixture("unicode emoji bidi", blocks: [.text("paragraph", "עברית 😀 العربية — café")], kinds: [.text]),
        ]

        let engine = CoreTextProseLayoutEngine()
        for fixture in fixtures {
            let document = ViewerDocument(
                semanticKey: String(repeating: "a", count: 64),
                blocks: fixture.blocks,
                isEmpty: false,
                retainedBytes: 256
            ).withPreparedTheme(PreparedProseTheme.resolve(themeJSON: fixture.themeJSON))
            let key = ProseLayoutKey(
                semanticKey: document.semanticKey,
                widthPixels: 640,
                themeDigest: "fixture",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: fixture.name
            )

            let first = try engine.prepare(document: document, key: key, widthPoints: 320, displayScale: 2)
            let second = try engine.prepare(document: document, key: key, widthPoints: 320, displayScale: 2)
            let kinds = Set(first.blocks.flatMap(\.fragments).map(\.kind))

            XCTAssertTrue(fixture.kinds.isSubset(of: kinds), fixture.name)
            XCTAssertEqual(first.size.height, second.size.height, accuracy: 0.000_1, fixture.name)
            for block in first.blocks {
                XCTAssertGreaterThanOrEqual(block.bounds.minY, 0, fixture.name)
                XCTAssertLessThanOrEqual(block.bounds.maxY, first.size.height, fixture.name)
                XCTAssertGreaterThanOrEqual(block.bounds.minX, 0, fixture.name)
                XCTAssertLessThanOrEqual(block.bounds.maxX, first.size.width, fixture.name)
                for fragment in block.fragments {
                    XCTAssertTrue(block.bounds.contains(fragment.bounds), "\(fixture.name): \(fragment.kind)")
                }
            }
        }
    }
}

private struct Fixture {
    let name: String
    let blocks: [ViewerBlock]
    let kinds: Set<PreparedProseFragmentKind>
    let themeJSON: String?

    init(
        _ name: String,
        blocks: [ViewerBlock],
        kinds: Set<PreparedProseFragmentKind>,
        themeJSON: String? = #"{"mentions":{"backgroundColor":"#DDEEFF"},"codeBlock":{"backgroundColor":"#F2F2F7","paddingHorizontal":12,"paddingVertical":8}}"#
    ) {
        self.name = name
        self.blocks = blocks
        self.kinds = kinds
        self.themeJSON = themeJSON
    }
}

private extension ViewerBlock {
    static func text(_ nodeType: String, _ text: String) -> ViewerBlock {
        ViewerBlock(nodeType: nodeType, depth: 0, inBlockquote: false, listContext: nil, inlines: [.text(text: text, marks: [])])
    }

    static func inline(_ nodeType: String, _ inlines: [ViewerInline]) -> ViewerBlock {
        ViewerBlock(nodeType: nodeType, depth: 0, inBlockquote: false, listContext: nil, inlines: inlines)
    }

    static func quote(_ text: String) -> ViewerBlock {
        ViewerBlock(nodeType: "paragraph", depth: 1, inBlockquote: true, listContext: nil, inlines: [.text(text: text, marks: [])])
    }

    static func list(_ kind: String, depth: Int, index: Int, text: String) -> ViewerBlock {
        ViewerBlock(
            nodeType: "paragraph",
            depth: UInt16(depth),
            inBlockquote: false,
            listContext: ViewerListContext(ordered: kind == "ordered", index: index, kind: kind, checked: false),
            inlines: [.text(text: text, marks: [])]
        )
    }

    static func task(depth: Int, checked: Bool, text: String) -> ViewerBlock {
        ViewerBlock(
            nodeType: "paragraph",
            depth: UInt16(depth),
            inBlockquote: false,
            listContext: ViewerListContext(ordered: false, index: 1, kind: "task", checked: checked),
            inlines: [.text(text: text, marks: [])]
        )
    }

    static func blockAtom(_ nodeType: String, _ label: String) -> ViewerBlock {
        ViewerBlock(nodeType: nodeType, depth: 0, inBlockquote: false, listContext: nil, inlines: [.atom(nodeType: nodeType, docPos: 0, attrsJSON: "{}", label: label)])
    }

    static var rule: ViewerBlock { blockAtom("horizontalRule", "") }
}

private extension ViewerInline {
    static func text(_ text: String, _ marks: [FfiViewerMark]) -> ViewerInline { .text(text: text, marks: marks) }
    static func atom(_ nodeType: String, _ label: String) -> ViewerInline { .atom(nodeType: nodeType, docPos: 0, attrsJSON: "{}", label: label) }
}

private extension FfiViewerMark {
    static func mark(_ type: String, _ attrsJSON: String = "{}") -> FfiViewerMark {
        FfiViewerMark(markType: type, attrsJson: attrsJSON)
    }
}
