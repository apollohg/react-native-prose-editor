import CoreText
import UIKit
import XCTest

final class PreparedProseRenderingTests: XCTestCase {
    func testVersionedMentionHonorsLineHeightAndStrikeDecoration() throws {
        let source = #"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"mention","attrs":{"label":"Jay"}}]}]}"#
        try withCompiledDocument(source: .json(source), configJSON: Fixture.customConfig) { document in
            let layout = try prepare(document, themeJSON: ##"{"version":1,"styles":{"mention":{"lineHeight":60,"borderTopWidth":3,"borderBottomWidth":5,"textDecorationLine":"line-through","textDecorationColor":"#ff0000ff"}}}"##)
            let atom = try XCTUnwrap(layout.blocks.flatMap(\.fragments).first { $0.kind == .atom })
            XCTAssertGreaterThanOrEqual(atom.bounds.height, 76)
            XCTAssertTrue(layout.blocks.flatMap(\.fragments).contains { $0.kind == .strike && $0.color == UIColor.red.cgColor })
        }
    }

    func testVersionedBulletScaleMatchesEditorMarkerDiameter() throws {
        for scale: CGFloat in [1, 2] {
            let theme = PreparedProseTheme.resolve(themeJSON: "{\"version\":1,\"styles\":{\"listMarker\":{\"scale\":\(scale)}}}")
            let context = ViewerListContext(ordered: false, index: 1, kind: nil, checked: false, isLast: true)
            let marker = CoreTextProseLayoutEngine().makeListMarker(context, nestingDepth: 0, paint: theme.text, theme: theme)
            let expected = EditorLayoutManager.unorderedBulletDrawingRect(usedRect: .zero, lineFragmentRect: .zero, markerWidth: 0, baselineY: 0, baseFont: theme.text.font, markerScale: scale, origin: .zero)
            XCTAssertEqual(marker.width, expected.width, accuracy: 0.01)
            XCTAssertEqual(marker.ascent + marker.descent, expected.height, accuracy: 0.01)
        }
    }

    func testVersionedStrikeKeepsDecorationColorAndPattern() throws {
        try withCompiledDocument(source: .json(#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Decorated text","marks":[{"type":"strike"}]}]}]}"#), configJSON: Fixture.customConfig) { document in
            for pattern in ["dashed", "dotted", "double"] {
                let theme = "{\"version\":1,\"styles\":{\"strike\":{\"textDecorationColor\":\"#ff0000ff\",\"textDecorationStyle\":\"\(pattern)\"}}}"
                let layout = try prepare(document, themeJSON: theme)
                let strikes = layout.blocks.flatMap(\.fragments).filter { $0.kind == .strike }
                XCTAssertGreaterThan(strikes.count, 1)
                XCTAssertTrue(strikes.allSatisfy { $0.color == UIColor.red.cgColor })
                if pattern == "double" { XCTAssertEqual(strikes.count, 2) }
                if pattern == "dotted" { XCTAssertTrue(strikes.allSatisfy { $0.cornerRadius > 0 }) }
            }
        }
    }

    func testVersionedStyleFixtureProducesEditorAndViewerImages() throws {
        let source = #"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Styled native text","marks":[{"type":"bold"}]}]},{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"A quote with its own inset, border, and rounded background."}]},{"type":"paragraph","content":[{"type":"text","text":"Continuous container spacing."}]}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"First list item"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"Second list item"}]}]}]},{"type":"paragraph","content":[{"type":"text","text":"Mention "},{"type":"mention","attrs":{"label":"Jay Den"}},{"type":"text","text":" after chip."}]},{"type":"codeBlock","content":[{"type":"text","text":"let answer = 42;\nprint(answer)"}]}]}"#
        let themeJSON = ##"{"version":1,"styles":{"content":{"backgroundColor":"#ffffffff","paddingTop":12,"paddingBottom":12,"paddingLeft":12,"paddingRight":12},"text":{"fontSize":16,"color":"#243047ff"},"bold":{"fontSize":24,"fontWeight":"700","color":"#303b8bff"},"paragraph":{"marginBottom":10},"blockquote":{"backgroundColor":"#edf0ffff","borderLeftColor":"#6267c9ff","borderLeftWidth":4,"borderTopLeftRadius":10,"borderBottomRightRadius":16,"paddingTop":10,"paddingRight":10,"paddingBottom":8,"marginBottom":12},"bulletList":{"backgroundColor":"#f3f7faff","paddingTop":8,"paddingBottom":8,"borderRadius":10,"marginBottom":12},"listMarker":{"color":"#6267c9ff"},"mention":{"fontWeight":"600","color":"#333b91ff","backgroundColor":"#e4e8ffff","borderLeftWidth":2,"borderRightWidth":2,"borderTopWidth":1,"borderBottomWidth":1,"borderColor":"#9098dfff","borderRadius":8},"codeBlock":{"color":"#dbeaffff","backgroundColor":"#243047ff","borderRadius":10,"paddingTop":12,"paddingBottom":12}}}"##
        let editorId = makeV2Editor(configJson: Fixture.customConfig)
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setJson(id: editorId, json: source)
        let editor = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 650))
        editor.editorId = editorId
        XCTAssertTrue(editor.applyTheme(try XCTUnwrap(EditorTheme.from(json: themeJSON))))
        editor.textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        editor.layoutIfNeeded()
        XCTAssertTrue(editor.textView.text.contains("Styled native text"))
        let format = UIGraphicsImageRendererFormat()
        format.scale = 2
        let editorImage = UIGraphicsImageRenderer(size: editor.bounds.size, format: format).image { context in
            UIColor.white.setFill()
            context.fill(editor.bounds)
            editor.layer.render(in: context.cgContext)
        }
        let directory = FileManager.default.temporaryDirectory
        try XCTUnwrap(editorImage.pngData()).write(to: directory.appendingPathComponent("ios-styles-editor.png"))
        try withCompiledDocument(source: .json(source), configJSON: Fixture.customConfig) { document in
            let layout = try prepare(document, themeJSON: themeJSON)
            let drawing = PreparedProseDrawingView(frame: CGRect(origin: .zero, size: layout.size))
            drawing.install(layout: layout)
            let viewerImage = UIGraphicsImageRenderer(size: CGSize(width: 320, height: 650), format: format).image { context in
                UIColor.white.setFill()
                context.fill(CGRect(x: 0, y: 0, width: 320, height: 650))
                drawing.draw(drawing.bounds)
            }
            try XCTUnwrap(viewerImage.pngData()).write(to: directory.appendingPathComponent("ios-styles-viewer.png"))
            XCTAssertGreaterThan(layout.blocks.count, 5)
        }
        print("STYLE_FIXTURES: \(directory.path)")
    }

    func testVersionedParagraphBoxReservesAllSidesAndRetainsTrailingMargin() throws {
        let document = ViewerDocument(semanticKey: "style-box", paragraphs: [.init(text: "Hello")], isEmpty: false, retainedBytes: 0)
        let plain = try prepare(document, themeJSON: nil)
        let styled = try prepare(document, themeJSON: """
        {"version":1,"styles":{"paragraph":{"paddingTop":11,"paddingBottom":13,"paddingLeft":7,"paddingRight":9,"borderTopWidth":2,"borderBottomWidth":3,"borderLeftWidth":4,"borderRightWidth":5,"marginTop":17,"marginBottom":19,"backgroundColor":"#ff0000ff","borderTopLeftRadius":8}}}
        """)
        let text = try XCTUnwrap(styled.blocks.flatMap(\.fragments).first { $0.kind == .text })
        XCTAssertEqual(text.bounds.minX, 11)
        XCTAssertEqual(text.bounds.minY, 30)
        XCTAssertGreaterThanOrEqual(styled.size.height - plain.size.height, 65)
        XCTAssertTrue(styled.blocks.flatMap(\.fragments).contains { $0.kind == .background })
    }

    func testVersionedNestedQuoteAndListKeepDistinctContinuousBoxes() throws {
        try withCompiledDocument(source: .json(#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"First"}]},{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"Nested"}]}]},{"type":"paragraph","content":[{"type":"text","text":"Last"}]}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"One"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"Two"}]}]}]}]}"#), configJSON: Fixture.customConfig) { document in
            let layout = try prepare(document, themeJSON: """
            {"version":1,"styles":{"blockquote":{"paddingTop":5,"paddingBottom":7,"backgroundColor":"#ffff00ff"},"bulletList":{"paddingTop":9,"paddingBottom":11,"backgroundColor":"#ff0000ff"},"listItem":{"marginBottom":3}}}
            """)
            let quoteBoxes = layout.decorations.filter { $0.styleBox?.color("backgroundColor") == UIColor.yellow }
            XCTAssertEqual(quoteBoxes.count, 2)
            XCTAssertGreaterThan(quoteBoxes[0].bounds.height, quoteBoxes[1].bounds.height)
            XCTAssertTrue(quoteBoxes[0].bounds.contains(quoteBoxes[1].bounds))
            let listBoxes = layout.decorations.filter { $0.styleBox?.color("backgroundColor") == UIColor.red }
            XCTAssertEqual(listBoxes.count, 1)
            XCTAssertGreaterThan(try XCTUnwrap(listBoxes.first, String(describing: document.blocks.map { ($0.nodeType, $0.styleAncestors) })).bounds.height, 50)
        }
    }

    func testVersionedQuoteOmitsOnlyFinalParagraphBottomMargin() throws {
        try withCompiledDocument(source: .html("<blockquote><p>First</p><p>Last</p></blockquote><p>Outside</p>"), configJSON: Fixture.localConfig) { document in
            let layout = try prepare(document, themeJSON: """
            {"version":1,"styles":{"paragraph":{"marginBottom":12,"paddingBottom":3,"borderBottomWidth":2},"blockquote":{"paddingBottom":7,"marginBottom":11,"backgroundColor":"#ffff00ff"}}}
            """)
            XCTAssertEqual(layout.blocks.count, 3)
            let first = layout.blocks[0].bounds
            let last = layout.blocks[1].bounds
            let outside = layout.blocks[2].bounds
            let quote = try XCTUnwrap(layout.decorations.first { $0.styleBox?.color("backgroundColor") == UIColor.yellow })
            XCTAssertEqual(last.minY - first.maxY, 12, accuracy: 0.01)
            XCTAssertEqual(quote.bounds.maxY - last.maxY, 7, accuracy: 0.01)
            XCTAssertEqual(outside.minY - quote.bounds.maxY, 11, accuracy: 0.01)
            XCTAssertEqual(layout.size.height - outside.maxY, 12, accuracy: 0.5)
        }
    }

    func testVersionedQuoteKeepsNestedContainerSpacing() throws {
        for (content, expected): (String, CGFloat) in [("<blockquote><p>Nested</p></blockquote>", 14), ("<ul><li><p>Nested</p></li></ul>", 23)] {
            try withCompiledDocument(source: .html("<blockquote>\(content)</blockquote>"), configJSON: Fixture.localConfig) { document in
                let layout = try prepare(document, themeJSON: """
                {"version":1,"styles":{"paragraph":{"marginBottom":12},"blockquote":{"paddingBottom":7,"backgroundColor":"#ffff00ff"}}}
                """)
                let outer = try XCTUnwrap(layout.decorations.first { $0.styleBox?.color("backgroundColor") == UIColor.yellow })
                let block = try XCTUnwrap(layout.blocks.last)
                XCTAssertEqual(outer.bounds.maxY - block.bounds.maxY, expected, accuracy: 0.01, content)
            }
        }
    }

    func testVersionedSiblingMarginsCollapseWithoutCollapsingPadding() throws {
        try withCompiledDocument(source: .html("<p>First</p><pre><code>Second</code></pre>"), configJSON: Fixture.localConfig) { document in
            for (bottom, top, collapsed) in [(12, 20, 20), (-12, -20, -20), (12, -5, 7)] {
                let layout = try prepare(document, themeJSON: """
                {"version":1,"styles":{"paragraph":{"marginBottom":\(bottom),"paddingBottom":3,"borderBottomWidth":2},"codeBlock":{"marginTop":\(top),"paddingTop":7,"borderTopWidth":4}}}
                """)
                XCTAssertEqual(layout.blocks.count, 2)
                XCTAssertEqual(layout.blocks[1].bounds.minY - layout.blocks[0].bounds.maxY, CGFloat(collapsed), accuracy: 0.01)
            }
        }
    }

    func testVersionedSiblingQuoteMarginsCollapseOutsideTheirPadding() throws {
        try withCompiledDocument(source: .html("<blockquote><p>First</p></blockquote><blockquote><p>Second</p></blockquote>"), configJSON: Fixture.localConfig) { document in
            let layout = try prepare(document, themeJSON: """
            {"version":1,"styles":{"paragraph":{"marginBottom":12},"blockquote":{"marginTop":20,"marginBottom":14,"paddingTop":3,"paddingBottom":5,"backgroundColor":"#ffff00ff"}}}
            """)
            let quotes = layout.decorations.filter { $0.styleBox?.color("backgroundColor") == UIColor.yellow }.sorted { $0.bounds.minY < $1.bounds.minY }
            XCTAssertEqual(quotes.count, 2)
            XCTAssertEqual(quotes[1].bounds.minY - quotes[0].bounds.maxY, 20, accuracy: 0.01)
        }
    }

    func testVersionedVoidBlockMarginsCollapseWithAdjacentParagraphs() throws {
        for (nodeType, html) in [("image", "<img src=\"https://example.test/image.png\" width=\"64\" height=\"32\">"), ("horizontalRule", "<hr>")] {
            try withCompiledDocument(source: .html("<p>First</p>\(html)<p>Last</p>"), configJSON: Fixture.localConfig) { document in
                let layout = try prepare(document, themeJSON: """
                {"version":1,"styles":{"paragraph":{"marginTop":17,"marginBottom":12},"\(nodeType)":{"marginTop":20,"marginBottom":8}}}
                """)
                XCTAssertEqual(layout.blocks.count, 3)
                XCTAssertEqual(layout.blocks[1].bounds.minY - layout.blocks[0].bounds.maxY, 20, accuracy: 0.01, nodeType)
                XCTAssertEqual(layout.blocks[2].bounds.minY - layout.blocks[1].bounds.maxY, 17, accuracy: 0.01, nodeType)
            }
        }
    }

    func testVersionedNegativeMarginsRetainAndDrawEarlierVisibleBlocks() throws {
        try withCompiledDocument(source: .html("<p>First</p><pre><code>Second</code></pre>"), configJSON: Fixture.localConfig) { document in
            let layout = try prepare(document, themeJSON: """
            {"version":1,"styles":{"content":{"paddingTop":80},"paragraph":{"marginBottom":-60,"backgroundColor":"#ff0000ff"},"codeBlock":{"marginTop":-60,"marginBottom":0,"paddingTop":0,"paddingBottom":0}}}
            """)
            let first = layout.blocks[0].bounds
            XCTAssertGreaterThanOrEqual(layout.size.height, first.maxY)
            let drawing = PreparedProseDrawingView(frame: CGRect(x: 0, y: 0, width: layout.size.width, height: max(layout.size.height, first.maxY)))
            drawing.install(layout: layout)
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            format.preferredRange = .standard
            let clip = CGRect(x: 0, y: first.minY + 2, width: layout.size.width, height: 2)
            let pixels = try XCTUnwrap(UIGraphicsImageRenderer(size: drawing.bounds.size, format: format).image { context in
                context.cgContext.clip(to: clip)
                drawing.draw(clip)
            }.cgImage)
            let data = try XCTUnwrap(pixels.dataProvider?.data)
            let bytes = try XCTUnwrap(CFDataGetBytePtr(data))
            let offset = Int(clip.minY) * pixels.bytesPerRow + 200 * 4
            XCTAssertGreaterThan(bytes[offset + 3], 0)
        }
    }

    func testVersionedInlineBackgroundHasPreparedPaintFragment() throws {
        try withCompiledDocument(source: .json(#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Painted","marks":[{"type":"bold"}]}]}]}"#), configJSON: #"{"initialization":{"type":"localEmpty"}}"#) { document in
            let layout = try prepare(document, themeJSON: """
            {"version":1,"styles":{"bold":{"backgroundColor":"#ff0000ff"}}}
            """)
            XCTAssertTrue(layout.blocks.flatMap(\.fragments).contains { $0.kind == .background && $0.color == UIColor.red.cgColor })
        }
    }

    func testVersionedImageStyleKeepsDeclaredSizeAndClipsDecodedPixels() throws {
        try withCompiledDocument(source: .json(#"{"type":"doc","content":[{"type":"image","attrs":{"src":"https://example.test/image.png","width":64,"height":32}}]}"#), configJSON: Fixture.localConfig) { document in
            let layout = try prepare(document, themeJSON: """
            {"version":1,"styles":{"image":{"paddingLeft":4,"paddingRight":6,"paddingTop":3,"paddingBottom":5,"borderTopWidth":2,"borderBottomWidth":2,"borderLeftWidth":2,"borderRightWidth":2,"borderTopLeftRadius":16,"backgroundColor":"#00ff00ff","resizeMode":"contain"}}}
            """)
            let attachment = try XCTUnwrap(layout.imageAttachments.first)
            XCTAssertEqual(attachment.bounds.width, 78)
            XCTAssertEqual(attachment.bounds.height, 44)
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            let pixels = UIGraphicsImageRenderer(size: CGSize(width: 64, height: 32), format: format).image { context in
                UIColor.red.setFill(); context.fill(CGRect(x: 0, y: 0, width: 64, height: 32))
            }
            let drawing = PreparedProseDrawingView(frame: CGRect(origin: .zero, size: layout.size))
            drawing.install(layout: layout)
            drawing.imagePixels = [attachment.id: pixels]
            let result = try XCTUnwrap(UIGraphicsImageRenderer(size: layout.size, format: format).image { _ in drawing.draw(drawing.bounds) }.cgImage)
            let data = try XCTUnwrap(result.dataProvider?.data)
            let bytes = try XCTUnwrap(CFDataGetBytePtr(data))
            let corner = Int(attachment.bounds.minY) * result.bytesPerRow + Int(attachment.bounds.minX) * 4
            XCTAssertEqual(bytes[corner + 3], 0)
        }
    }

    func testImagePixelsPreserveAllFourCornersInCoreTextDrawingContext() throws {
        try withCompiledDocument(
            source: .json(#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Before"}]},{"type":"image","attrs":{"src":"https://example.test/corners.png","width":64,"height":64}}]}"#),
            configJSON: #"{"initialization":{"type":"localEmpty"}}"#
        ) { document in
            let layout = try prepare(document, themeJSON: nil)
            let attachment = try XCTUnwrap(layout.imageAttachments.first)
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            format.preferredRange = .standard
            let source = UIGraphicsImageRenderer(size: CGSize(width: 64, height: 64), format: format).image { context in
                for (index, color) in [UIColor.red, .green, .blue, .yellow].enumerated() {
                    color.setFill()
                    context.fill(CGRect(x: (index % 2) * 32, y: (index / 2) * 32, width: 32, height: 32))
                }
            }
            let drawing = PreparedProseDrawingView(frame: CGRect(origin: .zero, size: layout.size))
            drawing.install(layout: layout)
            drawing.imagePixels = [attachment.id: source]
            let renderer = UIGraphicsImageRenderer(size: layout.size, format: format)
            let actual = try XCTUnwrap(renderer.image { _ in drawing.draw(drawing.bounds) }.cgImage)
            let expected = try XCTUnwrap(renderer.image { _ in source.draw(in: attachment.bounds) }.cgImage)
            let actualData = try XCTUnwrap(actual.dataProvider?.data)
            let expectedData = try XCTUnwrap(expected.dataProvider?.data)
            let actualBytes = try XCTUnwrap(CFDataGetBytePtr(actualData))
            let expectedBytes = try XCTUnwrap(CFDataGetBytePtr(expectedData))
            XCTAssertEqual(actual.bitsPerPixel, 32)
            XCTAssertEqual(expected.bitsPerPixel, 32)
            for yFraction in [0.25, 0.75] {
                for xFraction in [0.25, 0.75] {
                    let x = Int(attachment.bounds.minX + attachment.bounds.width * xFraction)
                    let y = Int(attachment.bounds.minY + attachment.bounds.height * yFraction)
                    for channel in 0..<4 {
                        XCTAssertEqual(
                            actualBytes[y * actual.bytesPerRow + x * 4 + channel],
                            expectedBytes[y * expected.bytesPerRow + x * 4 + channel],
                            "image corner at \(xFraction), \(yFraction)"
                        )
                    }
                }
            }
        }
    }

    func testCompilerBackedJSONAndHTMLFixturesPreserveInheritedContexts() throws {
        for fixture in Fixture.structuralFixtures {
            try withCompiledDocument(source: fixture.source, configJSON: fixture.configJSON) { document in
                XCTAssertTrue(fixture.expectedKinds.isSubset(of: preparedKinds(for: document)), fixture.name)
                XCTAssertTrue(fixture.assertDocument(document), fixture.name)
            }
        }
    }

    func testEveryCompilerFixtureHasDeterministicContainedGeometry() throws {
        for fixture in Fixture.compilerFixtures {
            try withCompiledDocument(source: fixture.source, configJSON: fixture.configJSON) { document in
                let first = try prepare(document, themeJSON: Fixture.themeJSON)
                let second = try prepare(document, themeJSON: Fixture.themeJSON)

                assertPreparedLayoutsEqual(first, second, fixture: fixture.name)
                XCTAssertTrue(fixture.expectedKinds.isSubset(of: Set(first.blocks.flatMap(\.fragments).map(\.kind))), fixture.name)
                XCTAssertTrue(fixture.assertDocument(document), fixture.name)
                assertGeometryContained(first, fixture: fixture.name)
                assertGeometryContained(second, fixture: fixture.name)
            }
        }
    }

    func testCompilerBackedMultiBlockAndNestedListItemsHaveIndependentBoundaries() throws {
        try withCompiledDocument(source: Fixture.multiBlockList.source, configJSON: Fixture.multiBlockList.configJSON) { document in
            let itemEntries: [(Int, ViewerBlock)] = document.blocks.compactMap { block in
                block.listItemBoundary.map { ($0.identity, block) }
            }
            let itemLeaves = Dictionary(grouping: itemEntries, by: { $0.0 })
            XCTAssertEqual(itemLeaves.count, 3, "outer two items and nested item must remain distinct")
            for (_, leaves) in itemLeaves {
                XCTAssertEqual(leaves.filter { $0.1.listItemBoundary!.isFirstRenderableLeaf }.count, 1)
                XCTAssertEqual(leaves.filter { $0.1.listItemBoundary!.isFinalRenderableLeaf }.count, 1)
            }

            let theme = PreparedProseTheme.resolve(themeJSON: Fixture.themeJSON)
            let layout = try prepare(document, themeJSON: Fixture.themeJSON)
            let layoutEntries: [(Int, (ViewerBlock, PreparedProseBlock))] = zip(document.blocks, layout.blocks).compactMap { pair in
                let (block, prepared) = pair
                return block.listItemBoundary.map { ($0.identity, (block, prepared)) }
            }
            let layoutByItem = Dictionary(grouping: layoutEntries, by: { $0.0 })
            for (_, leaves) in layoutByItem {
                let contentAnchors = leaves.compactMap { _, entry -> CGFloat? in
                    let (block, prepared) = entry
                    let content = prepared.fragments.first { fragment in
                        switch fragment.kind {
                        case .text, .atom:
                            return true
                        default:
                            return false
                        }
                    }
                    guard let content else {
                        return nil
                    }
                    let internalPadding = block.nodeType == "codeBlock" ? theme.codePaddingHorizontal : 0
                    return content.bounds.minX - internalPadding
                }
                XCTAssertEqual(contentAnchors.count, leaves.count, "every list leaf exposes a content anchor")
                if let sharedContentAnchor = contentAnchors.first {
                    for contentAnchor in contentAnchors.dropFirst() {
                        XCTAssertEqual(contentAnchor, sharedContentAnchor, accuracy: 0.001, "all leaves in one item reserve the same list content/gutter anchor")
                    }
                }
                XCTAssertEqual(leaves.flatMap { $0.1.1.fragments }.filter { $0.kind == .marker }.count, 1)
            }

            let outerOrdered = layoutByItem.values.first { leaves in
                leaves.contains { $0.1.0.listContext?.ordered == true && $0.1.0.listContext?.index == 7 }
            }
            let nestedOrdered = layoutByItem.values.first { leaves in
                leaves.contains { $0.1.0.listContext?.ordered == true && $0.1.0.listContext?.index == 12 }
            }
            XCTAssertEqual(outerOrdered?.flatMap { $0.1.1.fragments }.first(where: { $0.kind == .marker })?.label, "7.")
            XCTAssertEqual(nestedOrdered?.flatMap { $0.1.1.fragments }.first(where: { $0.kind == .marker })?.label, "l.")
            let emptyOrdered = layoutByItem.values.first { leaves in
                leaves.contains { $0.1.0.listContext?.ordered == true && $0.1.0.listContext?.index == 8 }
            }
            XCTAssertEqual(emptyOrdered?.flatMap { $0.1.1.fragments }.first(where: { $0.kind == .marker })?.label, "8.")
            XCTAssertTrue(document.blocks.filter { $0.listContext != nil }.allSatisfy(\.inBlockquote))

            let outerLeaves = outerOrdered!.sorted { $0.1.1.bounds.minY < $1.1.1.bounds.minY }
            XCTAssertEqual(outerLeaves.count, 3, "paragraph, code, and opaque block must share one outer item")
            let paragraph = try XCTUnwrap(outerLeaves[0].1.1.fragments.first(where: { $0.kind == .text }))
            let code = try XCTUnwrap(outerLeaves[1].1.1.fragments.first(where: { $0.kind == .text }))
            let opaque = try XCTUnwrap(outerLeaves[2].1.1.fragments.first(where: { $0.kind == .atom }))
            let sharedContentAnchor = paragraph.bounds.minX
            XCTAssertEqual(opaque.bounds.minX, sharedContentAnchor, accuracy: 0.001)
            XCTAssertEqual(code.bounds.minX, sharedContentAnchor + theme.codePaddingHorizontal, accuracy: 0.001)
            XCTAssertEqual(outerLeaves[0].1.1.bounds.maxY, outerLeaves[1].1.1.bounds.minY, accuracy: 0.001)
            XCTAssertEqual(outerLeaves[1].1.1.bounds.maxY, outerLeaves[2].1.1.bounds.minY, accuracy: 0.001)
            let nestedFirstY = nestedOrdered!.map { $0.1.1.bounds.minY }.min()!
            XCTAssertEqual(nestedFirstY - outerLeaves[2].1.1.bounds.maxY, 4, accuracy: 0.001)
        }
    }

    func testCompilerBackedFixturesExposeCoreTextMarkAttributesAndPreparedStrikeGeometry() throws {
        try withCompiledDocument(source: Fixture.markSource, configJSON: Fixture.customConfig) { document in
            let layout = try prepare(document, themeJSON: Fixture.themeJSON)
            let runs = layout.blocks.flatMap(\.fragments).compactMap(\.line).flatMap(coreTextRuns)
            let strikes = layout.blocks.flatMap(\.fragments).filter { $0.kind == .strike }

            XCTAssertTrue(runs.contains { fontTraits($0).contains(.traitBold) })
            XCTAssertTrue(runs.contains { fontTraits($0).contains(.traitItalic) })
            XCTAssertTrue(runs.contains {
                fontTraits($0).contains(.traitBold)
                    && fontTraits($0).contains(.traitItalic)
                    && fontTraits($0).contains(.traitMonoSpace)
                    && CTFontGetSize(font($0)) == 19
            })
            XCTAssertTrue(runs.contains { underlineStyle($0) == CTUnderlineStyle.single.rawValue })
            XCTAssertTrue(runs.contains { CTFontGetSymbolicTraits(font($0)).contains(.traitMonoSpace) })
            XCTAssertTrue(runs.contains { UIColor(cgColor: foreground($0)).isEqual(EditorTheme.color(from: "#007AFF")!) })
            XCTAssertTrue(runs.contains { UIColor(cgColor: foreground($0)).isEqual(EditorTheme.color(from: "#FF0000")!) })
            XCTAssertTrue(runs.contains { UIColor(cgColor: foreground($0)).isEqual(EditorTheme.color(from: "#00AA00")!) })
            XCTAssertTrue(runs.contains { background($0) != nil })
            XCTAssertTrue(runs.contains { CTFontCopyFamilyName(font($0)) as String == "Courier" })
            XCTAssertTrue(runs.contains { CTFontGetSize(font($0)) == 19 })
            XCTAssertFalse(strikes.isEmpty)
            XCTAssertTrue(strikes.allSatisfy { $0.bounds.width > 0 && $0.bounds.height > 0 })
            XCTAssertTrue(strikes.allSatisfy { strike in
                layout.blocks.contains { block in
                    block.bounds.contains(strike.bounds)
                        && block.fragments.contains { $0.kind == .text && $0.bounds.intersects(strike.bounds) }
                }
            })
        }
    }

    func testCoreTextFontBridgePreservesSystemFontIdentityAndWeightedTraits() {
        let fonts = [
            UIFont.systemFont(ofSize: 17),
            UIFont.systemFont(ofSize: 17, weight: .semibold),
        ]

        for font in fonts {
            let bridged = CoreTextProseLayoutEngine.coreTextFont(from: font)
            let source = font as CTFont

            XCTAssertTrue(CFEqual(bridged, source))
            XCTAssertEqual(CTFontGetSize(bridged), CTFontGetSize(source))
            XCTAssertEqual(CTFontCopyFamilyName(bridged) as String, CTFontCopyFamilyName(source) as String)
            XCTAssertEqual(CTFontGetSymbolicTraits(bridged), CTFontGetSymbolicTraits(source))
            XCTAssertEqual(CTFontCopyPostScriptName(bridged) as String, CTFontCopyPostScriptName(source) as String)
        }
    }

    func testExtremeListMarkerAndNestedCodeBlockquoteEdgesAreCompilerBacked() throws {
        try withCompiledDocument(source: Fixture.edgeSource, configJSON: Fixture.customConfig) { document in
            let layout = try prepare(
                document,
                themeJSON: ##"{"list":{"indent":28,"baseIndentMultiplier":0,"markerScale":4,"itemSpacing":3},"codeBlock":{"backgroundColor":"#F2F2F7","paddingHorizontal":12,"paddingVertical":8}}"##
            )
            let fragments = layout.blocks.flatMap(\.fragments)
            guard let listBlock = layout.blocks.first(where: { $0.fragments.contains { $0.kind == .marker } }),
                  let marker = listBlock.fragments.first(where: { $0.kind == .marker }),
                  let firstText = listBlock.fragments.first(where: { $0.kind == .text }),
                  let quoteBorder = fragments.first(where: { $0.kind == .border }),
                  let codeBackground = fragments.first(where: { $0.kind == .background })
            else { return XCTFail("edge fixture must prepare marker, text, quote border, and code background") }

            XCTAssertLessThanOrEqual(marker.bounds.maxX, firstText.bounds.minX)
            XCTAssertGreaterThanOrEqual(marker.bounds.minY, 0)
            XCTAssertTrue(quoteBorder.bounds.intersects(codeBackground.bounds))
        }
    }

    func testScaledListMarkersStayCenteredWithoutChangingItemSpacing() throws {
        let source = FixtureSource.json(#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"second"}]}]}]}]}"#)
        try withCompiledDocument(source: source, configJSON: Fixture.customConfig) { document in
            let regular = try prepare(document, themeJSON: #"{"list":{"markerScale":1,"itemSpacing":7}}"#)
            let scaled = try prepare(document, themeJSON: #"{"list":{"markerScale":3,"itemSpacing":7}}"#)

            for layout in [regular, scaled] {
                let itemBlocks = layout.blocks.filter { $0.fragments.contains { $0.kind == .marker } }
                XCTAssertEqual(itemBlocks.count, 2)
                for block in itemBlocks {
                    let marker = try XCTUnwrap(block.fragments.first { $0.kind == .marker })
                    let firstLine = try XCTUnwrap(block.fragments.first { $0.kind == .text })
                    XCTAssertEqual(marker.bounds.midY, firstLine.bounds.midY, accuracy: 0.001)
                    let markerLine = try XCTUnwrap(marker.line)
                    let imageBounds = CTLineGetImageBounds(markerLine, nil)
                    XCTAssertEqual(marker.origin.y - imageBounds.midY, firstLine.bounds.midY, accuracy: 0.001)
                }
            }

            let regularLines = regular.blocks.compactMap { $0.fragments.first { $0.kind == .text } }
            let scaledLines = scaled.blocks.compactMap { $0.fragments.first { $0.kind == .text } }
            XCTAssertEqual(regularLines.count, 2)
            XCTAssertEqual(scaledLines.count, 2)
            XCTAssertEqual(
                scaledLines[1].bounds.minY - scaledLines[0].bounds.maxY,
                regularLines[1].bounds.minY - regularLines[0].bounds.maxY,
                accuracy: 0.001
            )
        }
    }

    func testTerminalBlockSpacingDoesNotIncreaseViewerHeight() throws {
        let fixtures: [(source: FixtureSource, themeJSON: String, expectedGap: CGFloat)] = [
            (
                .json(#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]},{"type":"paragraph","content":[{"type":"text","text":"second"}]}]}"#),
                #"{"paragraph":{"spacingAfter":13},"contentInsets":{"bottom":7}}"#,
                13
            ),
            (
                .json(#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"second"}]}]}]}]}"#),
                #"{"list":{"itemSpacing":11,"spacingAfter":20},"contentInsets":{"bottom":7}}"#,
                11
            ),
        ]

        for fixture in fixtures {
            try withCompiledDocument(source: fixture.source, configJSON: Fixture.customConfig) { document in
                let layout = try prepare(document, themeJSON: fixture.themeJSON)
                XCTAssertEqual(layout.blocks.count, 2)
                XCTAssertEqual(
                    layout.blocks[1].bounds.minY - layout.blocks[0].bounds.maxY,
                    fixture.expectedGap,
                    accuracy: 0.001
                )
                let expectedHeight = ceil((layout.blocks[1].bounds.maxY + 7) * 2) / 2
                XCTAssertEqual(layout.size.height, expectedHeight, accuracy: 0.001)
            }
        }
    }

    func testListSpacingAfterReplacesTerminalItemSpacingBeforeFollowingContent() throws {
        let source = FixtureSource.json(#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"second"}]}]}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"third"}]}]}]},{"type":"paragraph","content":[{"type":"text","text":"after"}]}]}"#)
        try withCompiledDocument(source: source, configJSON: Fixture.customConfig) { document in
            let layout = try prepare(
                document,
                themeJSON: #"{"list":{"itemSpacing":6,"spacingAfter":20}}"#
            )

            XCTAssertEqual(layout.blocks.count, 4)
            XCTAssertEqual(layout.blocks[1].bounds.minY - layout.blocks[0].bounds.maxY, 6, accuracy: 0.001)
            XCTAssertEqual(layout.blocks[2].bounds.minY - layout.blocks[1].bounds.maxY, 20, accuracy: 0.001)
            XCTAssertEqual(layout.blocks[3].bounds.minY - layout.blocks[2].bounds.maxY, 20, accuracy: 0.001)
        }
    }

    func testListSpacingAfterSupportsTaskItemNodeNames() throws {
        let source = FixtureSource.json(#"{"type":"doc","content":[{"type":"taskList","content":[{"type":"taskItem","attrs":{"checked":false},"content":[{"type":"paragraph","content":[{"type":"text","text":"task"}]}]}]},{"type":"paragraph","content":[{"type":"text","text":"after"}]}]}"#)
        let config = #"{"schema":{"nodes":[{"name":"doc","content":"block+","role":"doc"},{"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},{"name":"taskList","content":"taskItem+","group":"block","role":"list"},{"name":"taskItem","content":"paragraph block*","role":"listItem","attrs":{"checked":{"default":false}}},{"name":"text","group":"inline","role":"text"}],"marks":[]},"initialization":{"type":"localEmpty"}}"#

        try withCompiledDocument(source: source, configJSON: config) { document in
            let layout = try prepare(
                document,
                themeJSON: #"{"list":{"itemSpacing":6,"spacingAfter":20}}"#
            )

            XCTAssertEqual(layout.blocks.count, 2)
            XCTAssertEqual(layout.blocks[1].bounds.minY - layout.blocks[0].bounds.maxY, 20, accuracy: 0.001)
        }
    }

    func testNestedListSpacingAfter() throws {
        let source = FixtureSource.json(#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"parent"}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]},{"type":"paragraph","content":[{"type":"text","text":"after nested"}]}]}]}]}"#)
        try withCompiledDocument(source: source, configJSON: Fixture.customConfig) { document in
            let layout = try prepare(
                document,
                themeJSON: #"{"list":{"itemSpacing":6,"spacingAfter":20}}"#
            )

            XCTAssertEqual(layout.blocks.count, 3)
            XCTAssertEqual(layout.blocks[2].bounds.minY - layout.blocks[1].bounds.maxY, 20, accuracy: 0.001)
        }
    }

    func testStackedNestedListSpacingAfter() throws {
        let source = FixtureSource.json(#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"parent"}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]}]}]},{"type":"paragraph","content":[{"type":"text","text":"after"}]}]}"#)
        try withCompiledDocument(source: source, configJSON: Fixture.customConfig) { document in
            XCTAssertEqual(document.blocks[1].listItemAncestors.count, 2)
            XCTAssertTrue(document.blocks[1].listItemAncestors.allSatisfy(\.context.isLast))
            let layout = try prepare(
                document,
                themeJSON: #"{"list":{"itemSpacing":6,"spacingAfter":20}}"#
            )

            XCTAssertEqual(layout.blocks.count, 3)
            XCTAssertEqual(layout.blocks[2].bounds.minY - layout.blocks[1].bounds.maxY, 40, accuracy: 0.001)
        }
    }

    func testMixedNestedListSpacingAfter() throws {
        let source = FixtureSource.json(#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"parent"}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"second"}]}]}]}]}"#)
        try withCompiledDocument(source: source, configJSON: Fixture.customConfig) { document in
            let layout = try prepare(
                document,
                themeJSON: #"{"list":{"itemSpacing":6,"spacingAfter":20}}"#
            )

            XCTAssertEqual(layout.blocks.count, 3)
            XCTAssertEqual(layout.blocks[2].bounds.minY - layout.blocks[1].bounds.maxY, 26, accuracy: 0.001)
        }
    }

    func testListMarkerScaleDoesNotResizeOrderedNumbers() throws {
        let source = FixtureSource.json(#"{"type":"doc","content":[{"type":"orderedList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]}]}]}]}"#)
        try withCompiledDocument(source: source, configJSON: Fixture.customConfig) { document in
            let regular = try prepare(document, themeJSON: #"{"list":{"markerScale":1}}"#)
            let scaled = try prepare(document, themeJSON: #"{"list":{"markerScale":3}}"#)
            let regularMarker = try XCTUnwrap(regular.blocks.flatMap(\.fragments).first { $0.kind == .marker })
            let scaledMarker = try XCTUnwrap(scaled.blocks.flatMap(\.fragments).first { $0.kind == .marker })
            let regularRun = try XCTUnwrap(coreTextRuns(try XCTUnwrap(regularMarker.line)).first)
            let scaledRun = try XCTUnwrap(coreTextRuns(try XCTUnwrap(scaledMarker.line)).first)

            XCTAssertEqual(CTFontGetSize(font(scaledRun)), CTFontGetSize(font(regularRun)), accuracy: 0.001)
        }
    }

    func testCompilerBackedMentionMergesLocalPaintIntoImmutableAtomFragment() throws {
        let fixture = Fixture.structuralFixtures[2]
        try withCompiledDocument(source: fixture.source, configJSON: fixture.configJSON) { document in
            let layout = try prepare(document, themeJSON: Fixture.themeJSON)
            guard let atom = layout.blocks.flatMap(\.fragments).first(where: { $0.kind == .atom }) else {
                return XCTFail("compiler-backed mention must produce an atom fragment")
            }
            XCTAssertEqual(UIColor(cgColor: atom.color!), EditorTheme.color(from: "#00FF00"))
            XCTAssertEqual(UIColor(cgColor: atom.borderColor!), EditorTheme.color(from: "#0000FF"))
            XCTAssertEqual(atom.strokeWidth, 2)
            XCTAssertEqual(atom.cornerRadius, 9)
            XCTAssertEqual(atom.padding, UIEdgeInsets(top: 4, left: 6, bottom: 4, right: 6))
            XCTAssertNotNil(atom.line)
        }
    }

    private func prepare(_ document: ViewerDocument, themeJSON: String?) throws -> PreparedProseLayout {
        let themed = document.withPreparedTheme(PreparedProseTheme.resolve(themeJSON: themeJSON))
        let key = ProseLayoutKey(
            semanticKey: themed.semanticKey,
            widthPixels: 640,
            themeDigest: "fixture",
            nativeFontRevision: 0,
            fontEnvironmentRevision: 0,
            displayScale: 2,
            attachmentRevision: 0,
            generationIdentity: "fixture",
            semanticGenerationIdentity: "fixture"
        )
        return try CoreTextProseLayoutEngine().prepare(document: themed, key: key, widthPoints: 320, displayScale: 2)
    }

    private func preparedKinds(for document: ViewerDocument) -> Set<PreparedProseFragmentKind> {
        let layout = try! prepare(document, themeJSON: Fixture.themeJSON)
        return Set(layout.blocks.flatMap(\.fragments).map(\.kind))
    }

    private func assertGeometryContained(_ layout: PreparedProseLayout, fixture: String) {
        let artifact = CGRect(origin: .zero, size: layout.size)
        for block in layout.blocks {
            XCTAssertTrue(artifact.contains(block.bounds), "block escapes artifact: \(fixture)")
            for fragment in block.fragments {
                let tolerance = max(0.5, fragment.strokeWidth / 2 + 0.5)
                XCTAssertTrue(block.bounds.insetBy(dx: -tolerance, dy: -tolerance).contains(fragment.bounds), "fragment escapes block: \(fixture)")
                XCTAssertTrue(artifact.insetBy(dx: -tolerance, dy: -tolerance).contains(fragment.bounds), "fragment escapes artifact: \(fixture)")
            }
        }
    }

    private func assertPreparedLayoutsEqual(
        _ first: PreparedProseLayout,
        _ second: PreparedProseLayout,
        fixture: String,
        accuracy: CGFloat = 0.001
    ) {
        assertEqual(first.size, second.size, accuracy: accuracy, "artifact size", fixture: fixture)
        XCTAssertEqual(first.blocks.count, second.blocks.count, "block count: \(fixture)")
        XCTAssertEqual(first.interactions, second.interactions, "interaction payload: \(fixture)")
        XCTAssertEqual(first.accessibilityNodes, second.accessibilityNodes, "accessibility payload: \(fixture)")

        for (blockIndex, pair) in zip(first.blocks, second.blocks).enumerated() {
            let (firstBlock, secondBlock) = pair
            assertEqual(firstBlock.bounds, secondBlock.bounds, accuracy: accuracy, "block \(blockIndex) bounds", fixture: fixture)
            XCTAssertEqual(firstBlock.fragments.count, secondBlock.fragments.count, "block \(blockIndex) fragment count: \(fixture)")

            for (fragmentIndex, fragmentPair) in zip(firstBlock.fragments, secondBlock.fragments).enumerated() {
                let (firstFragment, secondFragment) = fragmentPair
                XCTAssertEqual(firstFragment.kind, secondFragment.kind, "block \(blockIndex) fragment \(fragmentIndex) kind: \(fixture)")
                assertEqual(firstFragment.origin, secondFragment.origin, accuracy: accuracy, "block \(blockIndex) fragment \(fragmentIndex) origin", fixture: fixture)
                assertEqual(firstFragment.bounds, secondFragment.bounds, accuracy: accuracy, "block \(blockIndex) fragment \(fragmentIndex) bounds", fixture: fixture)
                XCTAssertEqual(firstFragment.line != nil, secondFragment.line != nil, "block \(blockIndex) fragment \(fragmentIndex) line presence: \(fixture)")
                XCTAssertEqual(firstFragment.label, secondFragment.label, "block \(blockIndex) fragment \(fragmentIndex) label: \(fixture)")
                XCTAssertEqual(firstFragment.checked, secondFragment.checked, "block \(blockIndex) fragment \(fragmentIndex) checked state: \(fixture)")
                XCTAssertEqual(firstFragment.cornerRadius, secondFragment.cornerRadius, accuracy: accuracy, "block \(blockIndex) fragment \(fragmentIndex) corner radius: \(fixture)")
                XCTAssertEqual(firstFragment.strokeWidth, secondFragment.strokeWidth, accuracy: accuracy, "block \(blockIndex) fragment \(fragmentIndex) stroke width: \(fixture)")
                assertEqual(firstFragment.padding, secondFragment.padding, accuracy: accuracy, "block \(blockIndex) fragment \(fragmentIndex) padding", fixture: fixture)

                if let firstLine = firstFragment.line, let secondLine = secondFragment.line {
                    let firstRange = CTLineGetStringRange(firstLine)
                    let secondRange = CTLineGetStringRange(secondLine)
                    XCTAssertEqual(firstRange.location, secondRange.location, "block \(blockIndex) fragment \(fragmentIndex) line range location: \(fixture)")
                    XCTAssertEqual(firstRange.length, secondRange.length, "block \(blockIndex) fragment \(fragmentIndex) line range length: \(fixture)")
                }
            }
        }
    }

    private func assertEqual(_ first: CGSize, _ second: CGSize, accuracy: CGFloat, _ component: String, fixture: String) {
        XCTAssertEqual(first.width, second.width, accuracy: accuracy, "\(component) width: \(fixture)")
        XCTAssertEqual(first.height, second.height, accuracy: accuracy, "\(component) height: \(fixture)")
    }

    private func assertEqual(_ first: CGPoint, _ second: CGPoint, accuracy: CGFloat, _ component: String, fixture: String) {
        XCTAssertEqual(first.x, second.x, accuracy: accuracy, "\(component) x: \(fixture)")
        XCTAssertEqual(first.y, second.y, accuracy: accuracy, "\(component) y: \(fixture)")
    }

    private func assertEqual(_ first: CGRect, _ second: CGRect, accuracy: CGFloat, _ component: String, fixture: String) {
        assertEqual(first.origin, second.origin, accuracy: accuracy, "\(component) origin", fixture: fixture)
        assertEqual(first.size, second.size, accuracy: accuracy, "\(component) size", fixture: fixture)
    }

    private func assertEqual(_ first: UIEdgeInsets, _ second: UIEdgeInsets, accuracy: CGFloat, _ component: String, fixture: String) {
        XCTAssertEqual(first.top, second.top, accuracy: accuracy, "\(component) top: \(fixture)")
        XCTAssertEqual(first.left, second.left, accuracy: accuracy, "\(component) left: \(fixture)")
        XCTAssertEqual(first.bottom, second.bottom, accuracy: accuracy, "\(component) bottom: \(fixture)")
        XCTAssertEqual(first.right, second.right, accuracy: accuracy, "\(component) right: \(fixture)")
    }

    /// Dropping both the result field and the local optional before returning
    /// deterministically releases UniFFI's generated compiled-document owner.
    private func withCompiledDocument<T>(
        source: FixtureSource,
        configJSON: String,
        body: (ViewerDocument) throws -> T
    ) throws -> T {
        var result = viewerCompile(
            request: FfiViewerCompileRequest(
                sourceKind: source.kind,
                source: source.value,
                configJson: configJSON,
                imagesEnabled: true,
                mentionPrefix: "@"
            )
        )
        if let error = result.error {
            throw ProseViewerError.compiler(domain: error.domain, code: error.code, message: error.message)
        }
        var compiled: ViewerCompiledDocument? = try XCTUnwrap(result.value)
        result.value = nil
        defer { compiled = nil }
        return try body(try ViewerDocument(compiled: try XCTUnwrap(compiled)))
    }

    private func coreTextRuns(_ line: CTLine) -> [CTRun] {
        CTLineGetGlyphRuns(line) as? [CTRun] ?? []
    }

    private func attributes(_ run: CTRun) -> [NSAttributedString.Key: Any] {
        CTRunGetAttributes(run) as? [NSAttributedString.Key: Any] ?? [:]
    }

    private func font(_ run: CTRun) -> CTFont {
        attributes(run)[kCTFontAttributeName as NSAttributedString.Key] as! CTFont
    }

    private func foreground(_ run: CTRun) -> CGColor {
        attributes(run)[kCTForegroundColorAttributeName as NSAttributedString.Key] as! CGColor
    }

    private func background(_ run: CTRun) -> CGColor? {
        guard let value = attributes(run)[kCTBackgroundColorAttributeName as NSAttributedString.Key] else { return nil }
        return (value as! CGColor)
    }

    private func underlineStyle(_ run: CTRun) -> Int32? {
        (attributes(run)[kCTUnderlineStyleAttributeName as NSAttributedString.Key] as? NSNumber)?.int32Value
    }

    private func fontTraits(_ run: CTRun) -> CTFontSymbolicTraits {
        CTFontGetSymbolicTraits(font(run))
    }
}

private enum FixtureSource {
    case json(String)
    case html(String)

    var kind: FfiViewerSourceKind {
        switch self {
        case .json: .json
        case .html: .html
        }
    }
    var value: String {
        switch self {
        case let .json(value), let .html(value): value
        }
    }
}

private struct Fixture {
    let name: String
    let source: FixtureSource
    let configJSON: String
    let expectedKinds: Set<PreparedProseFragmentKind>
    let assertDocument: (ViewerDocument) -> Bool

    static let themeJSON = ###"{"mentions":{"node":{"textColor":"#102030","backgroundColor":"#DDEEFF","borderColor":"#445566","borderWidth":2,"borderRadius":7}},"links":{"color":"#007AFF"}}"###
    static let localConfig = #"{"initialization":{"type":"localEmpty"}}"#
    static let customConfig = #"{"schema":{"nodes":[{"name":"doc","content":"block+","role":"doc"},{"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},{"name":"codeBlock","content":"text*","group":"block","role":"textBlock"},{"name":"blockquote","content":"block+","group":"block","role":"block"},{"name":"bulletList","content":"listItem+","group":"block","role":"list"},{"name":"orderedList","content":"listItem+","group":"block","role":"list","attrs":{"start":{"default":1}}},{"name":"taskList","content":"listItem+","group":"block","role":"list"},{"name":"listItem","content":"paragraph block*","role":"listItem","attrs":{"checked":{"default":false}}},{"name":"horizontal_rule","content":"","group":"block","role":"block","isVoid":true},{"name":"opaqueBlock","content":"","group":"block","role":"block","isVoid":true,"allowUndeclaredAttrs":true},{"name":"hardBreak","content":"","group":"inline","role":"hardBreak","isVoid":true},{"name":"mention","content":"","group":"inline","role":"inline","isVoid":true,"allowUndeclaredAttrs":true,"attrs":{"label":{"default":null}}},{"name":"opaque","content":"","group":"inline","role":"inline","isVoid":true,"allowUndeclaredAttrs":true},{"name":"text","group":"inline","role":"text"}],"marks":[{"name":"bold"},{"name":"italic"},{"name":"underline"},{"name":"strike"},{"name":"code"},{"name":"link","attrs":{"href":{}}},{"name":"textColor","attrs":{"color":{}}},{"name":"highlight","attrs":{"color":{}}},{"name":"textStyle","attrs":{"fontFamily":{},"fontSize":{}}}]},"initialization":{"type":"localEmpty"}}"#

    static let structuralFixtures: [Fixture] = [
        Fixture(
            name: "nested JSON list and blockquote inheritance",
            source: .json(#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"bullet_list","content":[{"type":"list_item","content":[{"type":"paragraph","content":[{"type":"text","text":"outer"}]},{"type":"ordered_list","attrs":{"start":12},"content":[{"type":"list_item","content":[{"type":"paragraph","content":[{"type":"text","text":"inner"}]}]}]}]}]}]}]}"#),
            configJSON: localConfig,
            expectedKinds: [.text, .marker, .border],
            assertDocument: { document in
                document.blocks.contains { $0.inBlockquote && $0.listContext != nil }
                    && document.blocks.contains { $0.listContext?.ordered == true && $0.listContext?.index == 12 }
            }
        ),
        Fixture(
            name: "HTML headings marks rule and hard break",
            source: .html("<h1>Heading 1</h1><h2>Heading 2</h2><h3>Heading 3</h3><h4>Heading 4</h4><h5>Heading 5</h5><h6>Heading 6</h6><blockquote><p><strong>bold</strong><br>quote</p></blockquote><ol start=\"3\"><li>third</li></ol><hr>"),
            configJSON: localConfig,
            expectedKinds: [.text, .marker, .border, .rule],
            assertDocument: { document in
                ["h1", "h2", "h3", "h4", "h5", "h6"].allSatisfy { heading in document.blocks.contains { $0.nodeType == heading } }
                    && document.blocks.contains { $0.inBlockquote }
            }
        ),
        Fixture(
            name: "custom atoms and snake rule",
            source: .json(##"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"mention","attrs":{"label":"Ada","mentionTheme":{"node":{"textColor":"#FF0000","backgroundColor":"#00FF00","borderColor":"#0000FF","borderWidth":2,"borderRadius":9}}}},{"type":"opaque","attrs":{"label":"opaque"}}]},{"type":"taskList","content":[{"type":"listItem","attrs":{"checked":true},"content":[{"type":"paragraph","content":[{"type":"text","text":"task"}]}]}]},{"type":"horizontal_rule"}]}"##),
            configJSON: customConfig,
            expectedKinds: [.atom, .rule],
            assertDocument: { document in
                document.blocks.contains { $0.nodeType == "horizontal_rule" }
                    && document.blocks.contains { $0.listContext?.kind == "task" && $0.listContext?.checked == true }
            }
        ),
    ]

    static let markSource = FixtureSource.json(##"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"bold","marks":[{"type":"bold"}]},{"type":"text","text":"italic","marks":[{"type":"italic"}]},{"type":"text","text":"under","marks":[{"type":"underline"}]},{"type":"text","text":"strike","marks":[{"type":"strike"}]},{"type":"text","text":"code","marks":[{"type":"code"}]},{"type":"text","text":"link","marks":[{"type":"link","attrs":{"href":"https://example.test"}}]},{"type":"text","text":"red","marks":[{"type":"textColor","attrs":{"color":"#FF0000"}}]},{"type":"text","text":"link-color","marks":[{"type":"link","attrs":{"href":"https://example.test"}},{"type":"textColor","attrs":{"color":"#00AA00"}}]},{"type":"text","text":"highlight","marks":[{"type":"highlight","attrs":{"color":"#FFF176"}}]},{"type":"text","text":"sized","marks":[{"type":"textStyle","attrs":{"fontFamily":"Courier","fontSize":19}}]},{"type":"text","text":"combo","marks":[{"type":"code"},{"type":"bold"},{"type":"italic"},{"type":"textStyle","attrs":{"fontFamily":"monospace","fontSize":19}}]}]}]}"##)
    static let edgeSource = FixtureSource.json(#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"codeBlock","content":[{"type":"text","text":"nested code"}]},{"type":"orderedList","attrs":{"start":9999},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"marker"}]}]}]}]}]}"#)
    static let markFixture = Fixture(name: "every mark", source: markSource, configJSON: customConfig, expectedKinds: [.text], assertDocument: { _ in true })
    static let edgeFixture = Fixture(name: "nested code blockquote edge", source: edgeSource, configJSON: customConfig, expectedKinds: [.text, .marker, .border, .background], assertDocument: { _ in true })
    static let multiBlockList = Fixture(
        name: "multi-block and nested ordered list boundaries",
        source: .json(#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"orderedList","attrs":{"start":7},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]},{"type":"codeBlock","content":[{"type":"text","text":"second"}]},{"type":"opaqueBlock","attrs":{"label":"third"}},{"type":"orderedList","attrs":{"start":12},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]}]},{"type":"listItem","content":[{"type":"paragraph"}]}]}]}]}"#),
        configJSON: customConfig,
        expectedKinds: [.text, .marker, .border, .background, .atom],
        assertDocument: { document in
            document.blocks.contains { $0.inBlockquote && $0.listContext?.index == 7 }
                && document.blocks.contains { $0.listContext?.index == 12 }
        }
    )
    static let unicodeFixture = Fixture(
        name: "unicode emoji bidi hard break and opaque atoms",
        source: .json(#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"\u05e9\u05dc\u05d5\u05dd \ud83d\ude80"},{"type":"hardBreak"},{"type":"opaque","attrs":{"label":"inline"}},{"type":"text","text":" cafe\u0301"}]},{"type":"opaqueBlock","attrs":{"label":"block"}}]}"#),
        configJSON: customConfig,
        expectedKinds: [.text, .atom],
        assertDocument: { document in document.blocks.contains { $0.nodeType == "opaqueBlock" } }
    )
    static let compilerFixtures = structuralFixtures + [markFixture, edgeFixture, multiBlockList, unicodeFixture]
}
