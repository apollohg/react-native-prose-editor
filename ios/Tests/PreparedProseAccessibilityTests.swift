import UIKit
import XCTest

final class PreparedProseAccessibilityTests: XCTestCase {
    func testSameLineInteractionGeometryMergesOnlyOverlappingOrEdgeTouchingPieces() {
        var edgeTouching = [CGRect(x: 2, y: 10, width: 8, height: 10)]
        PreparedProseInteractionGeometry.appendSameLinePiece(
            CGRect(x: 10, y: 10, width: 8, height: 10),
            to: &edgeTouching,
            mayMergeWithPrior: true
        )
        XCTAssertEqual(edgeTouching, [CGRect(x: 2, y: 10, width: 16, height: 10)])

        var positiveGap = [CGRect(x: 2, y: 10, width: 8, height: 10)]
        PreparedProseInteractionGeometry.appendSameLinePiece(
            CGRect(x: 10.25, y: 10, width: 8, height: 10),
            to: &positiveGap,
            mayMergeWithPrior: true
        )
        XCTAssertEqual(
            positiveGap,
            [
                CGRect(x: 2, y: 10, width: 8, height: 10),
                CGRect(x: 10.25, y: 10, width: 8, height: 10)
            ]
        )

        var oppositeDirection = [CGRect(x: 2, y: 10, width: 8, height: 10)]
        PreparedProseInteractionGeometry.appendSameLinePiece(
            CGRect(x: 10, y: 10, width: 8, height: 10),
            to: &oppositeDirection,
            mayMergeWithPrior: false
        )
        XCTAssertEqual(oppositeDirection.count, 2)
    }

    func testBidiLinkUsesDiscontiguousShapedRunRectsInVisualOrder() throws {
        let document = ViewerDocument(
            semanticKey: "bidi-link-fixture",
            blocks: [ViewerBlock(
                nodeType: "paragraph",
                depth: 0,
                inBlockquote: false,
                listContext: nil,
                listItemBoundary: nil,
                inlines: [
                    .text(
                        text: "Latin \u{05E2}\u{05D1}\u{05E8}\u{05D9}\u{05EA} Latin",
                        marks: [FfiViewerMark(markType: "link", attrsJson: #"{"href":"https://example.test/bidi"}"#)]
                    )
                ]
            )],
            isEmpty: false,
            retainedBytes: 64
        )

        let link = try prepare(document, width: 300).interactions.single(where: { $0.kind == .link })

        XCTAssertGreaterThanOrEqual(link.rects.count, 2)
        XCTAssertEqual(link.rects, link.rects.sorted { left, right in
            left.minY == right.minY ? left.minX < right.minX : left.minY < right.minY
        })
    }

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
                        .atom(
                            nodeType: "mention",
                            docPos: UInt32.max,
                            attrsJSON: #"{"id":"user-9","profile":{"kind":"clinician"}}"#,
                            label: "@Ada"
                        )
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
        XCTAssertEqual(
            layout.interactions.last?.attrsJSON,
            #"{"id":"user-9","profile":{"kind":"clinician"}}"#
        )
        XCTAssertEqual(layout.accessibilityNodes.map(\.role), [.link, .mention])
        XCTAssertEqual(layout.accessibilityNodes.last?.label, "@Ada")
        XCTAssertGreaterThan(layout.retainedBytes, document.retainedBytes)
    }

    func testLayoutRetainsLinkAndMentionAccessibilityNodes() throws {
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
        XCTAssertEqual(layout.accessibilityNodes.filter { $0.role == .link }.count, 1)
        XCTAssertEqual(layout.accessibilityNodes.filter { $0.role == .mention }.count, 1)
    }

    func testInlineAccessibilityNodesFollowDocumentOrder() throws {
        let document = ViewerDocument(
            semanticKey: "inline-accessibility-order",
            blocks: [ViewerBlock(
                nodeType: "paragraph",
                depth: 0,
                inBlockquote: false,
                listContext: nil,
                listItemBoundary: nil,
                inlines: [
                    .text(text: "Before ", marks: []),
                    .text(text: "link", marks: [
                        FfiViewerMark(markType: "link", attrsJson: #"{"href":"https://example.test"}"#)
                    ]),
                    .text(text: " between ", marks: []),
                    .atom(nodeType: "mention", docPos: 9, attrsJSON: "{}", label: "@Ada"),
                    .text(text: " after", marks: [])
                ]
            )],
            isEmpty: false,
            retainedBytes: 64
        )

        let layout = try prepare(document, width: 400)

        XCTAssertEqual(layout.accessibilityNodes.map(\.role), [.text, .link, .text, .mention, .text])
        XCTAssertEqual(
            layout.accessibilityNodes.map(\.label),
            ["Before", "link", "between", "@Ada", "after"]
        )
        let nodes = layout.accessibilityNodes
        XCTAssertTrue(nodes.allSatisfy { $0.bounds != layout.blocks[0].bounds })
        XCTAssertLessThanOrEqual(nodes[0].bounds.maxX, nodes[1].bounds.minX)
        XCTAssertLessThanOrEqual(nodes[1].bounds.maxX, nodes[2].bounds.minX)
        XCTAssertLessThanOrEqual(nodes[2].bounds.maxX, nodes[3].bounds.minX)
        XCTAssertLessThanOrEqual(nodes[3].bounds.maxX, nodes[4].bounds.minX)
        XCTAssertEqual(nodes[1].bounds, layout.interactions[0].rects.reduce(.null) { $0.union($1) })
        XCTAssertEqual(nodes[3].bounds, layout.interactions[1].rects.reduce(.null) { $0.union($1) })
    }

    func testWrappedBidiAccessibilityNodeRetainsShapedFragments() throws {
        let document = ViewerDocument(
            semanticKey: "wrapped-bidi-accessibility",
            blocks: [ViewerBlock(
                nodeType: "paragraph",
                depth: 0,
                inBlockquote: false,
                listContext: nil,
                listItemBoundary: nil,
                inlines: [
                    .text(text: "Before ", marks: []),
                    .text(
                        text: String(repeating: "Latin \u{05E2}\u{05D1}\u{05E8}\u{05D9}\u{05EA} ", count: 4),
                        marks: [FfiViewerMark(markType: "link", attrsJson: #"{"href":"https://example.test/bidi"}"#)]
                    ),
                    .text(text: "after", marks: [])
                ]
            )],
            isEmpty: false,
            retainedBytes: 64
        )

        let layout = try prepare(document, width: 110)
        let link = try XCTUnwrap(layout.accessibilityNodes.first { $0.role == .link })
        let interaction = try layout.interactions.single(where: { $0.kind == .link })

        XCTAssertEqual(
            layout.accessibilityNodes.map(\.label),
            ["Before", interaction.label.trimmingCharacters(in: .whitespacesAndNewlines), "after"]
        )
        XCTAssertEqual(link.rects, interaction.rects)
        XCTAssertGreaterThanOrEqual(link.rects.count, 2)
        XCTAssertGreaterThan(Set(link.rects.map(\.minY)).count, 1)
        XCTAssertTrue(layout.accessibilityNodes.allSatisfy { !$0.rects.isEmpty })
    }

    func testAccessibleDrawingViewLazilyMaterializesPermittedNodesAndRecycles() throws {
        let drawing = PreparedProseDrawingView(frame: CGRect(x: 0, y: 0, width: 180, height: 80))
        let layout = try prepare(
            ViewerDocument(
                semanticKey: "lazy-accessibility-fixture",
                blocks: [ViewerBlock(nodeType: "paragraph", depth: 0, inBlockquote: false, listContext: nil, listItemBoundary: nil, inlines: [
                    .text(text: "link", marks: [FfiViewerMark(markType: "link", attrsJson: #"{"href":"https://example.test"}"#)]),
                    .atom(nodeType: "mention", docPos: UInt32.max, attrsJSON: "{}", label: "@Ada")
                ])],
                isEmpty: false,
                retainedBytes: 64
            ),
            width: 180
        )
        drawing.install(layout: layout)

        XCTAssertEqual(drawing.accessibilityElementCount(), 2)
        XCTAssertEqual(drawing.materializedAccessibilityElementCountForTesting, 0)
        let enabledLink = try XCTUnwrap(
            drawing.accessibilityElement(at: 0) as? UIAccessibilityElement
        )
        let enabledLinkFrame = enabledLink.accessibilityFrame
        let enabledLinkPathBounds = try XCTUnwrap(enabledLink.accessibilityPath?.bounds)
        XCTAssertNotNil(drawing.accessibilityElement(at: 1))
        XCTAssertEqual(drawing.materializedAccessibilityElementCountForTesting, 2)
        var activatedMention: UInt32?
        drawing.onActivateInteraction = { interaction in
            activatedMention = interaction.docPos
            return interaction.kind == .mention
        }
        XCTAssertTrue((drawing.accessibilityElement(at: 1) as? UIAccessibilityElement)?.accessibilityActivate() ?? false)
        XCTAssertEqual(activatedMention, UInt32.max)

        drawing.linkInteractionsEnabled = false
        XCTAssertTrue(drawing.layout === layout)
        XCTAssertEqual(drawing.accessibilityElementCount(), 2)
        XCTAssertEqual(drawing.materializedAccessibilityElementCountForTesting, 0)
        let disabledLink = try XCTUnwrap(
            drawing.accessibilityElement(at: 0) as? UIAccessibilityElement
        )
        XCTAssertEqual(disabledLink.accessibilityLabel, "link")
        XCTAssertTrue(disabledLink.accessibilityTraits.contains(.staticText))
        XCTAssertEqual(disabledLink.accessibilityFrame, enabledLinkFrame)
        XCTAssertEqual(disabledLink.accessibilityPath?.bounds, enabledLinkPathBounds)
        XCTAssertEqual(layout.accessibilityNodes[0].rects, layout.interactions[0].rects)
        XCTAssertFalse(disabledLink.accessibilityActivate())

        drawing.install(layout: nil)
        XCTAssertEqual(drawing.accessibilityElementCount(), 0)
        XCTAssertEqual(drawing.materializedAccessibilityElementCountForTesting, 0)
    }

    func testPlainParagraphIsExposedAsStaticText() throws {
        let layout = try prepare(
            ViewerDocument(
                semanticKey: "plain-accessibility-fixture",
                paragraphs: [ViewerParagraph(text: "Readable plain prose")],
                isEmpty: false,
                retainedBytes: 64
            ),
            width: 180
        )
        let drawing = PreparedProseDrawingView(frame: CGRect(x: 0, y: 0, width: 180, height: 80))
        drawing.install(layout: layout)

        XCTAssertEqual(layout.accessibilityNodes.map(\.role), [.text])
        XCTAssertEqual(drawing.accessibilityElementCount(), 1)
        let element = try XCTUnwrap(
            drawing.accessibilityElement(at: 0) as? UIAccessibilityElement
        )
        XCTAssertEqual(element.accessibilityLabel, "Readable plain prose")
        XCTAssertTrue(element.accessibilityTraits.contains(.staticText))
    }

    func testHeadingBlocksAreExposedWithHeaderTrait() throws {
        let headings = (1...6).map { level in
            ViewerBlock(
                nodeType: "h\(level)",
                depth: 0,
                inBlockquote: false,
                listContext: nil,
                listItemBoundary: nil,
                inlines: [.text(text: "Heading \(level)", marks: [])]
            )
        }
        let layout = try prepare(
            ViewerDocument(
                semanticKey: "heading-accessibility-fixture",
                blocks: headings,
                isEmpty: false,
                retainedBytes: 64
            ),
            width: 180
        )
        let drawing = PreparedProseDrawingView(frame: CGRect(x: 0, y: 0, width: 180, height: 80))
        drawing.install(layout: layout)

        XCTAssertEqual(layout.accessibilityNodes.map(\.role), Array(repeating: .heading, count: 6))
        for index in 0..<6 {
            let element = try XCTUnwrap(
                drawing.accessibilityElement(at: index) as? UIAccessibilityElement
            )
            XCTAssertTrue(element.accessibilityTraits.contains(.staticText))
            XCTAssertTrue(element.accessibilityTraits.contains(.header))
        }
    }

    func testImageAccessibilityUsesAuthoredAltText() throws {
        let layout = try prepare(
            ViewerDocument(
                semanticKey: "image-alt-accessibility-fixture",
                blocks: [ViewerBlock(
                    nodeType: "image",
                    depth: 0,
                    inBlockquote: false,
                    listContext: nil,
                    listItemBoundary: nil,
                    inlines: [.atom(
                        nodeType: "image",
                        docPos: 4,
                        attrsJSON: #"{"src":"https://example.test/cat.png","alt":"A sleeping cat"}"#,
                        label: "image"
                    )]
                )],
                isEmpty: false,
                retainedBytes: 64
            ),
            width: 180
        )

        XCTAssertEqual(layout.accessibilityNodes.map(\.role), [.image])
        XCTAssertEqual(layout.accessibilityNodes.map(\.label), ["A sleeping cat"])

        let fallbackLayout = try prepare(
            ViewerDocument(
                semanticKey: "image-fallback-accessibility-fixture",
                blocks: [ViewerBlock(
                    nodeType: "image",
                    depth: 0,
                    inBlockquote: false,
                    listContext: nil,
                    listItemBoundary: nil,
                    inlines: [.atom(
                        nodeType: "image",
                        docPos: 4,
                        attrsJSON: #"{"src":"https://example.test/cat.png","alt":""}"#,
                        label: "image"
                    )]
                )],
                isEmpty: false,
                retainedBytes: 64
            ),
            width: 180
        )
        XCTAssertEqual(fallbackLayout.accessibilityNodes.map(\.label), ["Image"])
    }

    func testAccessibilityElementFromReplacedLayoutCannotActivateNewNodeAtSameIndex() throws {
        func linkedDocument(key: String, href: String, text: String) -> ViewerDocument {
            ViewerDocument(
                semanticKey: key,
                blocks: [ViewerBlock(
                    nodeType: "paragraph",
                    depth: 0,
                    inBlockquote: false,
                    listContext: nil,
                    listItemBoundary: nil,
                    inlines: [
                        .text(text: text, marks: [
                            FfiViewerMark(markType: "link", attrsJson: #"{"href":"\#(href)"}"#)
                        ])
                    ]
                )],
                isEmpty: false,
                retainedBytes: 64
            )
        }

        let drawing = PreparedProseDrawingView(frame: CGRect(x: 0, y: 0, width: 180, height: 80))
        drawing.install(layout: try prepare(
            linkedDocument(key: "old-link", href: "https://old.example", text: "Old"),
            width: 180
        ))
        let staleElement = try XCTUnwrap(
            drawing.accessibilityElement(at: 0) as? UIAccessibilityElement
        )
        drawing.install(layout: try prepare(
            linkedDocument(key: "new-link", href: "https://new.example", text: "New"),
            width: 180
        ))
        var activatedHref: String?
        drawing.onActivateInteraction = { interaction in
            activatedHref = interaction.href
            return true
        }

        XCTAssertFalse(staleElement.accessibilityActivate())
        XCTAssertNil(activatedHref)
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
            generationIdentity: "fixture",
            semanticGenerationIdentity: "fixture"
        )
        return try CoreTextProseLayoutEngine().prepare(
            document: document.withPreparedTheme(PreparedProseTheme.resolve(themeJSON: nil)),
            key: key,
            widthPoints: width,
            displayScale: 2
        )
    }
}

private extension Collection {
    func single(where predicate: (Element) -> Bool) throws -> Element {
        let matches = filter(predicate)
        return try XCTUnwrap(matches.single)
    }
}

private extension Array {
    var single: Element? { count == 1 ? first : nil }
}
