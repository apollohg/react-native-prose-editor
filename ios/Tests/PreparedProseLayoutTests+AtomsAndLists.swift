import CoreText
import Foundation
import UIKit
import XCTest

extension PreparedProseLayoutTests {
    func testRegisteredBlockAtomReservesMeasuredWidthAndZeroHeight() {
        let block = ViewerBlock(nodeType: "card", depth: 0, inBlockquote: false,
                                listContext: nil, listItemBoundary: nil,
                                inlines: [.atom(nodeType: "card", docPos: 3, attrsJSON: "{}", label: "Card")], isBlockAtom: true)
        let document = ViewerDocument(semanticKey: String(repeating: "a", count: 64),
                                      blocks: [block], isEmpty: false, retainedBytes: 128,
                                      trailingEmptyTextBlockCount: 0)
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })
        func measure(_ measuredWidth: Int) -> PreparedProseLayout {
            let theme = """
            {"viewerAtoms":{"generation":"g","revision":"r","nodeTypes":["card"],"estimatedHeights":{"card":70},"measurements":{"3":{"width":\(measuredWidth),"height":0}}}}
            """
            return registry.measure(request: ProseViewerRequest(source: .json("{}"),
                configuration: ProseViewerConfiguration(configJSON: "{}", themeJSON: theme)),
                widthPoints: 160, scale: 2)
        }
        XCTAssertEqual(measure(160).size.height, 0)
        XCTAssertEqual(measure(120).size.height, 70)
        XCTAssertTrue(measure(160).accessibilityNodes.isEmpty)
        let layout = measure(160)
        XCTAssertEqual(layout.blocks.first?.atomSlot?.bounds.width, 160)
        XCTAssertEqual(layout.blocks.first?.atomSlot?.docPos, 3)
        XCTAssertTrue(layout.blocks.first?.fragments.isEmpty == true)
        let drawing = PreparedProseDrawingView(frame: .zero)
        drawing.install(layout: layout)
        let data = drawing.atomLayoutsJSON(origin: CGPoint(x: 5, y: 9)).data(using: .utf8)!
        let atoms = try! JSONSerialization.jsonObject(with: data) as! [[String: Any]]
        XCTAssertEqual(atoms.first?["x"] as? Double, 5)
        XCTAssertEqual(atoms.first?["y"] as? Double, 9)
    }

    func testShortListedViewerAtomsIncludeMarkerBoundsAndFollowingSpacing() throws {
        let atom = ViewerBlock(nodeType: "card", depth: 0, inBlockquote: false,
            listContext: ViewerListContext(ordered: false, index: 0, kind: nil, checked: false, isLast: true),
            listItemBoundary: nil,
            inlines: [.atom(nodeType: "card", docPos: 3, attrsJSON: "{}", label: "Card")], isBlockAtom: true)
        let paragraph = ViewerBlock(nodeType: "paragraph", depth: 0, inBlockquote: false,
            listContext: nil, listItemBoundary: nil, inlines: [.text(text: "After", marks: [])])
        for height in [0, 1] {
            for followingParagraph in [false, true] {
                let document = ViewerDocument(semanticKey: String(repeating: "a", count: 64),
                    blocks: followingParagraph ? [atom, paragraph] : [atom], isEmpty: false,
                    retainedBytes: 128, trailingEmptyTextBlockCount: 0)
                let registry = PreparedProseLayoutRegistry(compile: { _ in document })
                let theme = """
                {"viewerAtoms":{"nodeTypes":["card"],"estimatedHeights":{"card":\(height)}}}
                """
                let layout = registry.measure(request: ProseViewerRequest(source: .json("{}"),
                    configuration: ProseViewerConfiguration(configJSON: "{}", themeJSON: theme)),
                    widthPoints: 200, scale: 2)
                let block = try XCTUnwrap(layout.blocks.first)
                let marker = try XCTUnwrap(block.fragments.first { $0.kind == .marker })
                XCTAssertEqual(block.atomSlot?.bounds.height, CGFloat(height))
                XCTAssertGreaterThan(marker.bounds.height, CGFloat(height))
                XCTAssertTrue(block.bounds.contains(marker.bounds))
                XCTAssertGreaterThanOrEqual(layout.size.height, marker.bounds.maxY)
                if followingParagraph {
                    XCTAssertGreaterThanOrEqual(layout.blocks[1].bounds.minY, marker.bounds.maxY)
                }
            }
        }
    }

    func testViewerAtomDefaultsAndExactMeasurementWidth() {
        let atoms = PreparedViewerAtoms.resolve("""
        {"viewerAtoms":{"nodeTypes":["card"],"measurements":{"3":{"width":160.1,"height":90}}}}
        """)!
        XCTAssertEqual(atoms.height(nodeType: "card", docPos: 3, width: 160), 32)
        XCTAssertEqual(atoms.height(nodeType: "card", docPos: 3, width: 160.1), 90)
    }

    func testViewerAtomDecorationsFallbackAndDownstreamGeometry() {
        func block(_ nodeType: String, atom: Bool, quote: Bool = false,
                   list: ViewerListContext? = nil) -> ViewerBlock {
            ViewerBlock(nodeType: nodeType, depth: 0, inBlockquote: quote,
                        listContext: list, listItemBoundary: nil,
                        inlines: [.atom(nodeType: "card", docPos: 3, attrsJSON: "{}", label: "Card")],
                        isBlockAtom: atom)
        }
        func measure(_ first: ViewerBlock, registered: Bool = true) -> PreparedProseLayout {
            let paragraph = ViewerBlock(nodeType: "paragraph", depth: 0, inBlockquote: false,
                listContext: nil, listItemBoundary: nil, inlines: [.text(text: "After", marks: [])])
            let document = ViewerDocument(semanticKey: String(repeating: "a", count: 64),
                blocks: [first, paragraph], isEmpty: false, retainedBytes: 128, trailingEmptyTextBlockCount: 0)
            let registry = PreparedProseLayoutRegistry(compile: { _ in document })
            let theme = """
            {"viewerAtoms":{"nodeTypes":["\(registered ? "card" : "other")"],"estimatedHeights":{"card":80}}}
            """
            return registry.measure(request: ProseViewerRequest(source: .json("{}"),
                configuration: ProseViewerConfiguration(configJSON: "{}", themeJSON: theme)),
                widthPoints: 200, scale: 2)
        }
        let plain = measure(block("card", atom: true))
        XCTAssertEqual(plain.blocks[0].atomSlot?.bounds.height, 80)
        XCTAssertGreaterThanOrEqual(plain.blocks[1].bounds.minY, 80)
        let decorated = measure(block("card", atom: true, quote: true,
            list: ViewerListContext(ordered: false, index: 0, kind: nil, checked: false, isLast: true)))
        XCTAssertTrue(decorated.blocks[0].fragments.contains { $0.kind == .marker })
        XCTAssertTrue(decorated.blocks[0].fragments.contains { $0.kind == .border })
        XCTAssertLessThan(decorated.blocks[0].atomSlot!.bounds.width, 200)
        XCTAssertGreaterThan(decorated.blocks[0].atomSlot!.bounds.minX, 0)
        let inline = measure(block("paragraph", atom: false))
        XCTAssertNil(inline.blocks[0].atomSlot)
        XCTAssertTrue(inline.blocks[0].fragments.contains { $0.kind == .atom })
        let fallback = measure(block("card", atom: true), registered: false)
        XCTAssertNil(fallback.blocks[0].atomSlot)
        XCTAssertTrue(fallback.blocks[0].fragments.contains { $0.kind == .atom })
    }

    func testCollapseTrailingEmptyParagraphs() {
        let blocks = ["first", "", "second", "", ""].map { text in
            ViewerBlock(
                nodeType: "paragraph",
                depth: 0,
                inBlockquote: false,
                listContext: nil,
                listItemBoundary: nil,
                inlines: [.text(text: text.isEmpty ? "\u{200B}" : text, marks: [])]
            )
        }
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            blocks: blocks,
            isEmpty: false,
            retainedBytes: 128,
            trailingEmptyTextBlockCount: 2
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })
        func request(collapse: Bool) -> ProseViewerRequest {
            ProseViewerRequest(
                source: .json("{}"),
                configuration: ProseViewerConfiguration(
                    configJSON: "{}",
                    collapsesWhenEmpty: collapse
                )
            )
        }

        let collapsed = registry.measure(
            request: request(collapse: true),
            widthPoints: 160,
            scale: 2
        )
        let expanded = registry.measure(
            request: request(collapse: false),
            widthPoints: 160,
            scale: 2
        )

        XCTAssertEqual(collapsed.blocks.count, 3)
        XCTAssertEqual(expanded.blocks.count, 5)
    }

    func testCollapseTrailingHiddenInlineImagePreservesPrecedingParagraph() {
        let source = """
        {"type":"doc","content":[
          {"type":"paragraph","content":[{"type":"text","text":"keep"}]},
          {"type":"paragraph","content":[{"type":"image","attrs":{"src":"https://example.test/image.png"}}]}
        ]}
        """
        let configJSON = """
        {"schema":{"nodes":[
          {"name":"doc","content":"block+","role":"doc"},
          {"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},
          {"name":"image","content":"","group":"inline","role":"inline","isVoid":true,"attrs":{"src":{}}},
          {"name":"text","group":"inline","role":"text"}
        ],"marks":[]},"initialization":{"type":"localEmpty"}}
        """
        let request = ProseViewerRequest(
            source: .json(source),
            configuration: ProseViewerConfiguration(
                configJSON: configJSON,
                imagesEnabled: false,
                collapsesWhenEmpty: true
            )
        )

        let layout = PreparedProseLayoutRegistry().measure(
            request: request,
            widthPoints: 160,
            scale: 2
        )

        XCTAssertNil(layout.error)
        XCTAssertEqual(layout.blocks.count, 1)
    }

    func testCustomBlockContainerDoesNotBecomeAnEmptyLeaf() {
        let source = """
        {"type":"doc","content":[
          {"type":"callout","content":[
            {"type":"paragraph","content":[{"type":"text","text":"keep"}]}
          ]}
        ]}
        """
        let configJSON = """
        {"schema":{"nodes":[
          {"name":"doc","content":"block+","role":"doc"},
          {"name":"callout","content":"block+","group":"block","role":"block"},
          {"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},
          {"name":"text","group":"inline","role":"text"}
        ],"marks":[]},"initialization":{"type":"localEmpty"}}
        """
        let request = ProseViewerRequest(
            source: .json(source),
            configuration: ProseViewerConfiguration(
                configJSON: configJSON,
                imagesEnabled: false,
                collapsesWhenEmpty: false
            )
        )

        let layout = PreparedProseLayoutRegistry().measure(
            request: request,
            widthPoints: 160,
            scale: 2
        )

        XCTAssertNil(layout.error)
        XCTAssertEqual(layout.blocks.count, 1)
    }

    func testOrderedListMarkersUseDefaultSchemesBySemanticNestingDepth() throws {
        let orderedContext = ViewerListContext(
            ordered: true,
            index: 1,
            kind: nil,
            checked: false,
            isLast: true
        )
        let bulletAncestor = ViewerListItemAncestor(
            identity: 100,
            context: ViewerListContext(
                ordered: false,
                index: 1,
                kind: nil,
                checked: false,
                isLast: true
            )
        )
        let nestedOrderedAncestor = ViewerListItemAncestor(
            identity: 101,
            context: orderedContext
        )
        let nestedBulletAncestor = ViewerListItemAncestor(
            identity: 102,
            context: bulletAncestor.context
        )
        let ancestorChains = [
            [ViewerListItemAncestor(identity: 0, context: orderedContext)],
            [
                bulletAncestor,
                ViewerListItemAncestor(identity: 1, context: orderedContext),
            ],
            [
                nestedOrderedAncestor,
                nestedBulletAncestor,
                ViewerListItemAncestor(identity: 2, context: orderedContext),
            ],
            [
                bulletAncestor,
                nestedOrderedAncestor,
                nestedBulletAncestor,
                ViewerListItemAncestor(identity: 3, context: orderedContext),
            ],
        ]
        let mismatchedBoundaryDepths: [UInt16] = [2, 0, 0, 1]
        let blocks = ancestorChains.enumerated().map { index, ancestors in
            ViewerBlock(
                nodeType: "paragraph",
                depth: UInt16(40 + index),
                inBlockquote: index == 1,
                listContext: orderedContext,
                listItemBoundary: ViewerListItemBoundary(
                    identity: ancestors.last!.identity,
                    nestingDepth: mismatchedBoundaryDepths[index],
                    isFirstRenderableLeaf: true,
                    isFinalRenderableLeaf: true
                ),
                listItemAncestors: ancestors,
                inlines: [.text(text: "item", marks: [])]
            )
        }
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            blocks: blocks,
            isEmpty: false,
            retainedBytes: 128,
            preparedTheme: PreparedProseTheme.resolve(themeJSON: nil)
        )
        let key = ProseLayoutKey(
            semanticKey: document.semanticKey,
            widthPixels: 640,
            themeDigest: "ordered-marker-theme",
            nativeFontRevision: 0,
            fontEnvironmentRevision: 0,
            displayScale: 2,
            attachmentRevision: 0,
            generationIdentity: "ordered-marker-theme",
            semanticGenerationIdentity: "ordered-marker-theme"
        )

        let layout = try CoreTextProseLayoutEngine().prepare(
            document: document,
            key: key,
            widthPoints: 320,
            displayScale: 2
        )
        let markerLabels = layout.blocks
            .flatMap(\.fragments)
            .filter { $0.kind == .marker }
            .compactMap(\.label)

        XCTAssertEqual(markerLabels, ["1.", "a.", "i.", "1."])
    }

    func testOrderedListFallbackUsesSemanticAncestorDepth() throws {
        let orderedContext = ViewerListContext(
            ordered: true,
            index: 1,
            kind: nil,
            checked: false,
            isLast: true
        )
        let bulletContext = ViewerListContext(
            ordered: false,
            index: 1,
            kind: nil,
            checked: false,
            isLast: true
        )
        let block = ViewerBlock(
            nodeType: "paragraph",
            depth: 64,
            inBlockquote: true,
            listContext: orderedContext,
            listItemBoundary: nil,
            listItemAncestors: [
                ViewerListItemAncestor(identity: 100, context: bulletContext),
                ViewerListItemAncestor(identity: 101, context: orderedContext),
                ViewerListItemAncestor(identity: 102, context: orderedContext),
            ],
            inlines: [.text(text: "item", marks: [])]
        )
        let themeJSON = """
        {"list":{"orderedMarker":{"schemes":["decimal","lowerAlpha","lowerRoman"],"suffix":")"}}}
        """
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            blocks: [block],
            isEmpty: false,
            retainedBytes: 128,
            preparedTheme: PreparedProseTheme.resolve(themeJSON: themeJSON)
        )
        let key = ProseLayoutKey(
            semanticKey: document.semanticKey,
            widthPixels: 640,
            themeDigest: "ordered-marker-theme",
            nativeFontRevision: 0,
            fontEnvironmentRevision: 0,
            displayScale: 2,
            attachmentRevision: 0,
            generationIdentity: "ordered-marker-theme",
            semanticGenerationIdentity: "ordered-marker-theme"
        )

        let layout = try CoreTextProseLayoutEngine().prepare(
            document: document,
            key: key,
            widthPoints: 320,
            displayScale: 2
        )
        let markerLabels = layout.blocks
            .flatMap(\.fragments)
            .filter { $0.kind == .marker }
            .compactMap(\.label)

        XCTAssertEqual(markerLabels, ["i)"])
    }

    func testOrderedMarkerEditorAndViewerRenderingConformForSharedTuples() throws {
        let fixtures: [(index: Int, semanticDepth: Int, expected: String)] = [
            (27, 0, "AA)"),
            (3_999, 1, "MMMCMXCIX)"),
            (42, 2, "42)"),
        ]
        let themeDictionary: [String: Any] = [
            "list": [
                "orderedMarker": [
                    "schemes": ["upperAlpha", "upperRoman", "decimal"],
                    "suffix": ")",
                ],
            ],
        ]
        let themeJSONData = try JSONSerialization.data(withJSONObject: themeDictionary)
        let themeJSON = try XCTUnwrap(String(data: themeJSONData, encoding: .utf8))
        let editorTheme = EditorTheme(dictionary: themeDictionary)
        let viewerTheme = PreparedProseTheme.resolve(themeJSON: themeJSON)

        for fixture in fixtures {
            var elements: [[String: Any]] = []
            for depth in 0...fixture.semanticDepth {
                let deepest = depth == fixture.semanticDepth
                elements.append([
                    "type": "blockStart",
                    "nodeType": "listItem",
                    "depth": depth,
                    "listContext": [
                        "ordered": deepest,
                        "index": deepest ? fixture.index : 1,
                        "isFirst": true,
                        "isLast": true,
                    ],
                ])
            }
            elements.append([
                "type": "blockStart",
                "nodeType": "paragraph",
                "depth": fixture.semanticDepth + 1,
            ])
            elements.append(["type": "textRun", "text": "item", "marks": []])
            elements.append(["type": "blockEnd"])
            for _ in 0...fixture.semanticDepth {
                elements.append(["type": "blockEnd"])
            }
            let renderData = try JSONSerialization.data(withJSONObject: elements)
            let renderJSON = try XCTUnwrap(String(data: renderData, encoding: .utf8))
            let editor = RenderBridge.renderElements(
                fromJSON: renderJSON,
                baseFont: .systemFont(ofSize: 16),
                textColor: .label,
                theme: editorTheme
            )
            let editorLabel = editor.attribute(
                RenderBridgeAttributes.orderedListMarkerLabel,
                at: 0,
                effectiveRange: nil
            ) as? String

            let orderedContext = ViewerListContext(
                ordered: true,
                index: fixture.index,
                kind: nil,
                checked: false,
                isLast: true
            )
            let ancestors = (0...fixture.semanticDepth).map { depth in
                ViewerListItemAncestor(
                    identity: depth,
                    context: depth == fixture.semanticDepth
                        ? orderedContext
                        : ViewerListContext(
                            ordered: false,
                            index: 1,
                            kind: nil,
                            checked: false,
                            isLast: true
                        )
                )
            }
            let block = ViewerBlock(
                nodeType: "paragraph",
                depth: UInt16(80 + fixture.semanticDepth),
                inBlockquote: false,
                listContext: orderedContext,
                listItemBoundary: ViewerListItemBoundary(
                    identity: ancestors.last!.identity,
                    nestingDepth: UInt16(40 - fixture.semanticDepth),
                    isFirstRenderableLeaf: true,
                    isFinalRenderableLeaf: true
                ),
                listItemAncestors: ancestors,
                inlines: [.text(text: "item", marks: [])]
            )
            let semanticKey = "conformance-\(fixture.semanticDepth)"
            let viewer = try CoreTextProseLayoutEngine().prepare(
                document: ViewerDocument(
                    semanticKey: semanticKey,
                    blocks: [block],
                    isEmpty: false,
                    retainedBytes: 64,
                    preparedTheme: viewerTheme
                ),
                key: ProseLayoutKey(
                    semanticKey: semanticKey,
                    widthPixels: 640,
                    themeDigest: "ordered-marker-conformance",
                    nativeFontRevision: 0,
                    fontEnvironmentRevision: 0,
                    displayScale: 2,
                    attachmentRevision: 0,
                    generationIdentity: semanticKey,
                    semanticGenerationIdentity: semanticKey
                ),
                widthPoints: 320,
                displayScale: 2
            )
            let viewerLabel = viewer.blocks
                .flatMap(\.fragments)
                .first { $0.kind == .marker }?
                .label

            XCTAssertEqual(editorLabel, fixture.expected)
            XCTAssertEqual(viewerLabel, fixture.expected)
            XCTAssertEqual(editorLabel, viewerLabel)
        }
    }

}
