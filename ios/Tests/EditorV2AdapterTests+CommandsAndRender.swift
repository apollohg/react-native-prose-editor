import XCTest
import UIKit

extension EditorV2AdapterTests {
    func testCommandsRouteThroughTypedV2Transactions() {
        let adapter = makeAdapter()
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        _ = adapter.setContentHtml("<p>ab</p>")

        let bold = adapter.toggleMark("bold", anchor: 0, head: 2)
        XCTAssertNotNil(bold)
        let doc = documentJsonObject(adapter)
        let paragraph = (doc["content"] as? [[String: Any]])?.first
        let textNode = (paragraph?["content"] as? [[String: Any]])?.first
        XCTAssertEqual((textNode?["marks"] as? [[String: Any]])?.first?["type"] as? String, "bold")

        let heading = adapter.toggleHeading(level: 2, anchor: 1, head: 1)
        XCTAssertNotNil(heading)
        let headedDoc = documentJsonObject(adapter)
        let headingNode = (headedDoc["content"] as? [[String: Any]])?.first
        XCTAssertEqual(headingNode?["type"] as? String, "heading")
        XCTAssertEqual((headingNode?["attrs"] as? [String: Any])?["level"] as? Int, 2)

        // list_item's content expression rejects headings: wrap a fresh
        // paragraph instead.
        _ = adapter.setContentHtml("<p>ab</p>")
        let list = adapter.wrapInList(listType: "bullet_list", itemType: "list_item", anchor: 1, head: 1)
        XCTAssertNotNil(list)
        let listDoc = documentJsonObject(adapter)
        XCTAssertEqual((listDoc["content"] as? [[String: Any]])?.first?["type"] as? String, "bullet_list")

        let hardBreak = adapter.insertNode("hard_break", anchor: 1, head: 1)
        XCTAssertNotNil(hardBreak)

        _ = adapter.setContentHtml("<p>ab</p>")
        let quote = adapter.toggleBlockquote(anchor: 1, head: 1)
        XCTAssertNotNil(quote)
        let code = adapter.toggleCodeBlock(anchor: 1, head: 1)
        XCTAssertNotNil(code)
    }

    func testPasteRoutesThroughV2() {
        let adapter = makeAdapter()
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        _ = adapter.setContentHtml("<p>ab</p>")

        let html = adapter.insertContentHtml("<strong>CD</strong>", anchor: 2, head: 2)
        XCTAssertEqual(renderedText(html), "abCD")

        // Plain-text paste with newlines goes through insertContentJson on the
        // composed fragment (the view builds the fragment; the adapter just
        // routes it typed).
        let fragment = #"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"X"}]},{"type":"paragraph","content":[{"type":"text","text":"Y"}]}]}"#
        let json = adapter.insertContentJson(fragment, anchor: 4, head: 4)
        XCTAssertNotNil(json)
        XCTAssertEqual(documentText(adapter), "abCDXY")
    }

    func testVoidAndListMarkerPositionMapping() {
        let adapter = makeAdapter()
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        let doc = #"{"type":"doc","content":[{"type":"image","attrs":{"src":"https://example.com/a.png","alt":null,"title":null,"width":null,"height":null}},{"type":"bullet_list","content":[{"type":"list_item","content":[{"type":"paragraph","content":[{"type":"text","text":"item"}]}]}]}]}"#
        _ = adapter.setContentJson(doc)
        XCTAssertEqual(documentText(adapter), "item")

        // Type inside the list item paragraph: the image void is scalar 0,
        // the block separator is scalar 1, "item" text starts at scalar 2.
        let update = adapter.insertText("X", atScalar: 2)
        XCTAssertEqual(renderedText(update), "Xitem")

        // Atom boundaries cannot host a caret, so typing there is a no-op.
        let boundaryInsert = adapter.insertText("Z", atScalar: 1)
        XCTAssertEqual(renderedText(boundaryInsert), "Xitem")
        XCTAssertNil(spy.last)
        XCTAssertEqual(documentText(adapter), "Xitem")
        let after = documentJsonObject(adapter)
        XCTAssertEqual((after["content"] as? [[String: Any]])?.first?["type"] as? String, "image")
        XCTAssertEqual((after["content"] as? [[String: Any]])?.last?["type"] as? String, "bullet_list")

        // Resize the void image by its document position.
        let resized = adapter.resizeImage(atDocPos: 0, width: 120, height: 80)
        XCTAssertNotNil(resized)
        let resizedDoc = documentJsonObject(adapter)
        let image = (resizedDoc["content"] as? [[String: Any]])?.first
        let attrs = image?["attrs"] as? [String: Any]
        XCTAssertEqual(attrs?["width"] as? Int, 120)
        XCTAssertEqual(attrs?["height"] as? Int, 80)
    }

    func testLocalOnlyMutationRequiresNoTransportOwner() {
        let adapter = makeAdapter()
        XCTAssertNotNil(adapter.insertText("x", atScalar: 0))
        XCTAssertEqual(documentText(adapter), "x")
    }

    func testSynthesizedUpdateCarriesRustStateNotFabricatedState() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>ab</p>")
        let update = parseObject(adapter.insertText("c", atScalar: 2))

        XCTAssertEqual(update["documentVersion"] as? String, "2")
        let history = historyState(adapter.insertText("d", atScalar: 3))
        XCTAssertEqual(history["canUndo"] as? Bool, true)
        XCTAssertEqual(history["canRedo"] as? Bool, false)
        // Toolbar state derives from the authoritative document+selection.
        let state = parseObject(adapter.currentStateJSON())
        XCTAssertNotNil(state["activeState"])
        XCTAssertEqual(state["documentVersion"] as? String, "3")
    }

    func testStructuredErrorEnvelopeFields() {
        let adapter = makeAdapter(
            configJson: #"{"initialization":{"type":"localEmpty"},"policy":{"readOnly":true}}"#
        )
        _ = adapter.setContentHtml("<p>seed</p>")
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        XCTAssertNil(adapter.insertText("x", atScalar: 1))
        let error = spy.last
        XCTAssertEqual(error?.domain, "boundary")
        XCTAssertEqual(error?.code, "MUTATION_REJECTED")
        XCTAssertFalse(error?.message.isEmpty ?? true)
        XCTAssertNotNil(error?.requestId)
        XCTAssertNotNil(UInt64(error?.requestId ?? ""), "requestId is a decimal string")
        XCTAssertNil(error?.operationIndex)
        XCTAssertNil(error?.limit)
        XCTAssertNil(error?.actual)
    }

    /// Golden fixture assertions against the accessor alone (these survive
    /// the probe removal): rendered text, scalar extent, active state at a
    /// mirrored mark selection, and backward-selection doc resolution.
    func testRenderAccessorFixtureMatrixGoldenContent() {
        // Empty doc: one empty block, a one-scalar extent (the empty
        // paragraph's synthetic placeholder), and the authoritative initial
        // text selection carried by the locked render snapshot.
        let empty = makeFixtureAdapter("")
        let emptyUpdate = parseObject(editorV2RenderUpdate(editorId: empty.editorId, mirrorScalarAnchor: nil, mirrorScalarHead: nil).value)
        XCTAssertNotNil(emptyUpdate["renderBlocks"] as? NSArray)
        XCTAssertEqual((emptyUpdate["scalarLength"] as? NSNumber)?.uint32Value, 1)
        let emptySelection = emptyUpdate["selection"] as? [String: Any]
        XCTAssertEqual(emptySelection?["type"] as? String, "text")
        XCTAssertEqual((emptySelection?["anchor"] as? NSNumber)?.uint32Value, 1)
        XCTAssertEqual((emptySelection?["head"] as? NSNumber)?.uint32Value, 1)
        XCTAssertNotNil(emptyUpdate["activeState"] as? NSDictionary)

        // Emoji: the scalar extent counts every Unicode scalar (a, emoji,
        // e + combining acute, 7-scalar ZWJ family, b = 12).
        let emoji = makeFixtureAdapter(Self.accessorFixtures.first { $0.0 == "emoji" }!.1)
        let emojiUpdate = parseObject(editorV2RenderUpdate(editorId: emoji.editorId, mirrorScalarAnchor: nil, mirrorScalarHead: nil).value)
        XCTAssertEqual((emojiUpdate["scalarLength"] as? NSNumber)?.uint32Value, 12)

        // Marks: mirroring the bold run activates the bold toolbar mark.
        let marks = makeFixtureAdapter(Self.accessorFixtures.first { $0.0 == "marks" }!.1)
        let marksUpdate = parseObject(editorV2RenderUpdate(editorId: marks.editorId, mirrorScalarAnchor: 6, mirrorScalarHead: 10).value)
        let activeState = marksUpdate["activeState"] as? [String: Any]
        let activeMarks = activeState?["marks"] as? [String: Any]
        XCTAssertEqual(activeMarks?["bold"] as? Bool, true, "mirror over the bold run activates bold")

        // Multi-block: extent counts the block separator as one scalar; a
        // backward mirror resolves each side to its doc position.
        let multi = makeFixtureAdapter(Self.accessorFixtures.first { $0.0 == "multi-block" }!.1)
        let multiUpdate = parseObject(editorV2RenderUpdate(editorId: multi.editorId, mirrorScalarAnchor: 4, mirrorScalarHead: 1).value)
        XCTAssertEqual((multiUpdate["scalarLength"] as? NSNumber)?.uint32Value, 5)
        let selection = multiUpdate["selection"] as? [String: Any]
        XCTAssertEqual((selection?["anchorScalar"] as? NSNumber)?.uint32Value, 4)
        XCTAssertEqual((selection?["headScalar"] as? NSNumber)?.uint32Value, 1)
        XCTAssertEqual((selection?["anchor"] as? NSNumber)?.uint32Value, 6)
        XCTAssertEqual((selection?["head"] as? NSNumber)?.uint32Value, 2)

        // Void nodes: the image void occupies one scalar; text starts after
        // the block separator.
        let void = makeFixtureAdapter(Self.accessorFixtures.first { $0.0 == "void-nodes" }!.1)
        let voidUpdate = parseObject(editorV2RenderUpdate(editorId: void.editorId, mirrorScalarAnchor: nil, mirrorScalarHead: nil).value)
        XCTAssertEqual((voidUpdate["scalarLength"] as? NSNumber)?.uint32Value, 7)
        let voidBlocks = voidUpdate["renderBlocks"] as? [[[String: Any]]]
        let voidText = (voidBlocks ?? []).flatMap { $0 }
            .compactMap { ($0["type"] as? String) == "textRun" ? ($0["text"] as? String) : nil }
            .joined()
        XCTAssertEqual(voidText, "after")
    }

    /// Wrapping a line in a list must not drag the caret backwards.
    ///
    /// `commandAtSelection` mirrors the post-command render update at the
    /// caret's *pre-command* scalar offsets. That is right for a mark toggle,
    /// which moves no text, but a structural wrap inserts the list, item, and
    /// paragraph opening tokens in front of the line — every offset in it
    /// shifts by two. Mirroring the old offsets pins the caret two characters
    /// short of where the user left it.
    func testWrappingALineInAListKeepsTheCaretAtTheEndOfTheText() {
        let adapter = makeAdapter()
        _ = adapter.insertText("one", atScalar: 0)

        let beforeUpdate = parseObject(
            editorV2RenderUpdate(
                editorId: adapter.editorId,
                mirrorScalarAnchor: nil,
                mirrorScalarHead: nil
            ).value
        )
        let beforeSelection = beforeUpdate["selection"] as? [String: Any]
        XCTAssertEqual(
            (beforeSelection?["anchorScalar"] as? NSNumber)?.uint32Value,
            3,
            "precondition: the caret sits at the end of the bare line"
        )

        let updateJSON = adapter.wrapInList(
            listType: "bullet_list",
            itemType: "list_item",
            anchor: 3,
            head: 3
        )
        let update = parseObject(updateJSON)
        let selection = update["selection"] as? [String: Any]

        XCTAssertEqual(
            (selection?["anchorScalar"] as? NSNumber)?.uint32Value,
            5,
            "the caret must land at the end of the wrapped line, not at the "
                + "offset it held before the list tokens were inserted"
        )
        XCTAssertEqual(
            (selection?["headScalar"] as? NSNumber)?.uint32Value,
            5,
            "the caret must stay collapsed at the end of the wrapped line"
        )
    }

    /// The same shift applies to a caret parked mid-word, so a fix cannot get
    /// away with pinning the caret to the end of the line.
    func testWrappingALineInAListKeepsAMidWordCaretOnTheSameCharacter() {
        let adapter = makeAdapter()
        _ = adapter.insertText("one", atScalar: 0)

        let updateJSON = adapter.wrapInList(
            listType: "bullet_list",
            itemType: "list_item",
            anchor: 1,
            head: 1
        )
        let update = parseObject(updateJSON)
        let selection = update["selection"] as? [String: Any]

        XCTAssertEqual(
            (selection?["anchorScalar"] as? NSNumber)?.uint32Value,
            3,
            "a caret one character into the line must still be one character "
                + "in after the wrap"
        )
    }

    /// A render update that fails at the engine used to return nil in silence,
    /// so no caller could name the cause. The adapter reports it like any
    /// other boundary failure.
    func testRefreshReportsAFailedEngineRenderUpdate() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>base</p>")
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record

        // Destroy the engine session behind the adapter's back: the next
        // render update reaches a handle the registry no longer knows.
        _ = editorV2Destroy(editorId: adapter.editorId)

        XCTAssertNil(adapter.refreshFromRustState(mirrorSelection: nil))
        XCTAssertEqual(spy.errors.count, 1, "the failure is reported exactly once")
        XCTAssertEqual(spy.last?.domain, "lifecycle")
    }

}
