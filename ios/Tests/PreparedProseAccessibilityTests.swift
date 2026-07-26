import XCTest

final class PreparedProseAccessibilityTests: XCTestCase {
    func testPreparedInteractionsFreezeWrappedLinkAndUInt32MentionInReadingOrder() throws {
        let document = ViewerDocument(
            semanticKey: "interaction-fixture",
            blocks: [
                ViewerBlock(
                    nodeType: "paragraph",
                    depth: 0,
                    inBlockquote: false,
                    listContext: nil,
                    listItemBoundary: nil,
                    inlines: [
                        .text(
                            text: String(repeating: "linked ", count: 12),
                            marks: [FfiViewerMark(markType: "link", attrsJson: #"{"href":"https://example.test/wrapped"}"#)]
                        ),
                        .atom(nodeType: "mention", docPos: UInt32.max, attrsJSON: "{}", label: "@Ada")
                    ]
                )
            ],
            isEmpty: false,
            retainedBytes: 64
        )
        let layout = try prepare(document, width: 90)

        XCTAssertEqual(layout.interactions.map(\.kind), [.link, .mention])
        XCTAssertEqual(layout.interactions.first?.href, "https://example.test/wrapped")
        XCTAssertGreaterThanOrEqual(layout.interactions.first?.rects.count ?? 0, 2)
        XCTAssertEqual(layout.interactions.last?.docPos, UInt32.max)
        XCTAssertEqual(layout.accessibilityNodes.map(\.role), [.link, .mention])
        XCTAssertEqual(layout.accessibilityNodes.last?.label, "@Ada")
        XCTAssertGreaterThan(layout.retainedBytes, document.retainedBytes)
    }

    func testDisabledLinksAreAbsentButMentionsRemainAccessible() throws {
        let document = ViewerDocument(
            semanticKey: "disabled-link-fixture",
            blocks: [ViewerBlock(nodeType: "paragraph", depth: 0, inBlockquote: false, listContext: nil, listItemBoundary: nil, inlines: [
                .text(text: "link", marks: [FfiViewerMark(markType: "link", attrsJson: #"{"href":"https://example.test"}"#)]),
                .atom(nodeType: "mention", docPos: 9, attrsJSON: "{}", label: "@Ada")
            ])],
            isEmpty: false,
            retainedBytes: 64
        )
        let layout = try prepare(document, width: 180)

        XCTAssertEqual(layout.interactions.filter { $0.kind == .link }.count, 1)
        XCTAssertEqual(layout.accessibilityNodes.filter { $0.kind == .link }.count, 1)
        XCTAssertEqual(layout.accessibilityNodes.filter { $0.kind == .mention }.count, 1)
    }

    private func prepare(_ document: ViewerDocument, width: CGFloat) throws -> PreparedProseLayout {
        let key = ProseLayoutKey(
            semanticKey: document.semanticKey,
            widthPixels: Int(width * 2),
            themeDigest: "fixture",
            nativeFontRevision: 0,
            fontEnvironmentRevision: 0,
            displayScale: 2,
            attachmentRevision: 0,
            generationIdentity: "fixture"
        )
        return try CoreTextProseLayoutEngine().prepare(
            document: document.withPreparedTheme(PreparedProseTheme.resolve(themeJSON: nil)),
            key: key,
            widthPoints: width,
            displayScale: 2
        )
    }
}
