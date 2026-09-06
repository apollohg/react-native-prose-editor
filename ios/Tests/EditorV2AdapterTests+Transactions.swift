import XCTest
import UIKit

extension EditorV2AdapterTests {
    func testAttachesDecimalV2HandleAndDetachedLocalState() {
        let adapter = makeAdapter(
            configJson: #"{"initialization":{"type":"localEmpty"},"policy":{"readOnly":false}}"#
        )
        XCTAssertFalse(adapter.editorId.isEmpty)
        XCTAssertNotNil(UInt64(adapter.editorId), "editor id must be a decimal string, got \(adapter.editorId)")
        XCTAssertEqual(adapter.baseDocumentRevision, 0)

        let state = parseObject(editorV2GetState(editorId: adapter.editorId).value)
        XCTAssertEqual(state["documentState"] as? String, "LocalReady")
        XCTAssertEqual(state["transportState"] as? String, "Detached")
        XCTAssertEqual(state["renderState"] as? String, "Ready")
        XCTAssertEqual(state["canUndo"] as? Bool, false)
        XCTAssertEqual(state["canRedo"] as? Bool, false)
    }

    func testInitialContentSetUsesLocalApiAndRendersDerivedBlocks() {
        let adapter = makeAdapter()
        let update = adapter.setContentHtml("<p>Hello</p>")
        XCTAssertEqual(renderedText(update), "Hello")
        // resetAndClear history: not undoable.
        XCTAssertEqual(historyState(update)["canUndo"] as? Bool, false)
        let parsed = parseObject(update)
        XCTAssertEqual(parsed["documentVersion"] as? String, "1")
        XCTAssertEqual(adapter.baseDocumentRevision, 1)
        XCTAssertEqual(documentText(adapter), "Hello")
    }

    func testTypingCommitIsExactlyOneLocalInputTransaction() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>ab</p>")
        let revisionBefore = adapter.baseDocumentRevision

        let mapping = adapter.syncSelection(anchor: 2, head: 2)
        XCTAssertNotNil(mapping)
        XCTAssertEqual(mapping?.docAnchor, 3)
        XCTAssertEqual(mapping?.docHead, 3)

        let update = adapter.insertText("X", atScalar: 2)
        XCTAssertEqual(renderedText(update), "abX")
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore + 1, "one commit = one revision step")
        XCTAssertEqual(documentText(adapter), "abX")

        // Exactly one typed local-input transaction: a single undo removes the
        // whole commit and nothing else.
        let undone = adapter.undo()
        XCTAssertEqual(renderedText(undone), "ab")
    }

    func testNativeTextMutationRendersExplicitPostSelection() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>thueh</p>")
        let owner = UUID()
        adapter.claimNativeBindingIfUnowned(token: owner)
        defer { adapter.releaseNativeBindingOwner(token: owner) }

        let updateJSON = adapter.commitNativeTextMutation(
            from: 0,
            to: 4,
            with: "thrus",
            postSelection: (anchor: 6, head: 6)
        )
        let update = parseObject(updateJSON)

        XCTAssertEqual(documentText(adapter), "thrush")
        let renderPatch = update["renderPatch"] as? [String: Any]
        let renderBlocks = renderPatch?["renderBlocks"] as? [[[String: Any]]]
        let renderedRuns = renderBlocks?.flatMap { block in
            block.compactMap { element in
                element["type"] as? String == "textRun" ? element["text"] as? String : nil
            }
        }
        XCTAssertEqual(renderedRuns?.joined(), "thrush")
        let selection = update["selection"] as? [String: Any]
        XCTAssertEqual((selection?["anchorScalar"] as? NSNumber)?.uint32Value, 6)
        XCTAssertEqual((selection?["headScalar"] as? NSNumber)?.uint32Value, 6)
    }

    func testRoomTypingPublishesPostMutationAwarenessBeforeTransportWake() {
        var calls: [String] = []
        let adapter = makeAttachedAdapter(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            roomBound: true,
            setAwarenessSelection: { _, _ in
                calls.append("awarenessSelection")
                return FfiJsonResult(value: #"{"outboundChanged":true}"#, error: nil)
            },
            collaborationWake: { _, reason in
                calls.append("wake:\(reason.rawValue)")
            },
            file: #filePath,
            line: #line
        )
        _ = adapter.setContentHtml("<p>ab</p>")
        calls.removeAll()

        let update = adapter.insertText("X", atScalar: 2)

        XCTAssertEqual(renderedText(update), "abX")
        XCTAssertEqual(
            calls,
            ["awarenessSelection", "wake:awareness", "wake:localMutation"]
        )
    }

    func testRoomStandaloneSelectionPublishesAwarenessWithoutDocumentWake() {
        var calls: [String] = []
        let adapter = makeAttachedAdapter(
            configJson: #"{"initialization":{"type":"localHtml","html":"<p>ab</p>"}}"#,
            roomBound: true,
            setAwarenessSelection: { _, _ in
                calls.append("awarenessSelection")
                return FfiJsonResult(value: #"{"outboundChanged":true}"#, error: nil)
            },
            collaborationWake: { _, reason in
                calls.append("wake:\(reason.rawValue)")
            },
            file: #filePath,
            line: #line
        )
        _ = adapter.currentStateJSON()
        calls.removeAll()

        let mapping = adapter.syncSelection(anchor: 1, head: 1)

        XCTAssertEqual(mapping?.docAnchor, 2)
        XCTAssertEqual(mapping?.docHead, 2)
        XCTAssertEqual(calls, ["awarenessSelection", "wake:awareness"])
    }

    func testRoomImageSelectionPublishesAwarenessWithoutDocumentWake() {
        var selections: [[String: Any]] = []
        var wakes: [String] = []
        let adapter = makeAttachedAdapter(
            configJson: #"{"initialization":{"type":"localHtml","html":"<p>Hello</p><img src=\"https://example.com/cat.png\"><p>After</p>"}}"#,
            roomBound: true,
            setAwarenessSelection: { _, json in
                let data = Data(json.utf8)
                selections.append((try? JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:])
                return FfiJsonResult(value: #"{"outboundChanged":true}"#, error: nil)
            },
            collaborationWake: { _, reason in wakes.append(reason.rawValue) },
            file: #filePath,
            line: #line
        )
        _ = adapter.currentStateJSON()
        selections.removeAll()
        wakes.removeAll()

        let sync = adapter.syncNodeSelection(docPos: 7)

        XCTAssertEqual(sync?.docAnchor, 7)
        XCTAssertEqual(selections.count, 1)
        XCTAssertEqual(selections.first?["type"] as? String, "text")
        XCTAssertEqual((selections.first?["anchor"] as? NSNumber)?.uint32Value, 7)
        XCTAssertEqual((selections.first?["head"] as? NSNumber)?.uint32Value, 8)
        XCTAssertEqual(wakes, ["awareness"])
    }

    func testAwarenessPublicationFailureDoesNotRollbackCommittedTyping() {
        var rejectAwareness = false
        let adapter = makeAttachedAdapter(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            roomBound: true,
            setAwarenessSelection: { _, _ in
                if rejectAwareness {
                    return FfiJsonResult(
                        value: nil,
                        error: FfiError(
                            domain: "transport",
                            code: "TRANSPORT_RESOURCE_EXHAUSTED",
                            message: "awareness outbox is full",
                            requestId: nil,
                            operationIndex: nil,
                            limit: nil,
                            actual: nil,
                            detailsJson: nil
                        )
                    )
                }
                return FfiJsonResult(value: #"{"outboundChanged":false}"#, error: nil)
            },
            collaborationWake: { _, _ in },
            file: #filePath,
            line: #line
        )
        _ = adapter.setContentHtml("<p>ab</p>")
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        rejectAwareness = true

        let update = adapter.insertText("X", atScalar: 2)

        XCTAssertEqual(renderedText(update), "abX")
        XCTAssertEqual(documentText(adapter), "abX")
        XCTAssertEqual(spy.last?.code, "TRANSPORT_RESOURCE_EXHAUSTED")
    }

    func testReplacementCommitIsOneTransaction() {
        let adapter = makeAdapter()
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        _ = adapter.setContentHtml("<p>teh</p>")
        let revisionBefore = adapter.baseDocumentRevision

        let update = adapter.replaceTextRange(from: 0, to: 3, with: "the")
        XCTAssertEqual(renderedText(update), "the")
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore + 1)
        XCTAssertEqual(documentText(adapter), "the")

        let undone = adapter.undo()
        XCTAssertEqual(renderedText(undone), "teh")
    }

    func testBackwardSelectionPositionMapping() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>abcd</p>")

        // Backward selection (anchor ahead of head) maps each side exactly.
        let mapping = adapter.syncSelection(anchor: 3, head: 1)
        XCTAssertNotNil(mapping)
        XCTAssertEqual(mapping?.docAnchor, 4)
        XCTAssertEqual(mapping?.docHead, 2)

        let update = adapter.replaceTextRange(from: 1, to: 3, with: "X")
        XCTAssertEqual(renderedText(update), "aXd")
        XCTAssertEqual(documentText(adapter), "aXd")
    }

    func testEmojiAndCombiningMarkPositionMappingIsExact() {
        let adapter = makeAdapter()
        // a, emoji (1 scalar), e + combining acute (2 scalars), family emoji
        // (7 scalars incl. ZWJ), b.
        let base = "a\u{1F600}e\u{0301}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}b"
        let docJson = "{\"type\":\"doc\",\"content\":[{\"type\":\"paragraph\",\"content\":[{\"type\":\"text\",\"text\":\"\(base)\"}]}]}"
        _ = adapter.setContentJson(docJson)

        // Insert immediately after the emoji (scalars: a=0, emoji=1 -> pos 2).
        let inserted = adapter.insertText("X", atScalar: 2)
        var scalars = String.UnicodeScalarView(base.unicodeScalars)
        scalars.insert("X", at: scalars.index(scalars.startIndex, offsetBy: 2))
        let expected = String(scalars)
        XCTAssertEqual(renderedText(inserted), expected)
        XCTAssertEqual(documentText(adapter), expected)

        // Delete exactly the combining acute (scalar index 4 within the
        // mutated string: a, emoji, X, e, ́ -> range 4..5).
        let deleted = adapter.deleteScalarRange(from: 4, to: 5)
        var afterDelete = scalars
        afterDelete.remove(at: afterDelete.index(afterDelete.startIndex, offsetBy: 4))
        let expectedAfterDelete = String(afterDelete)
        XCTAssertEqual(renderedText(deleted), expectedAfterDelete)
        XCTAssertEqual(documentText(adapter), expectedAfterDelete)
    }

    func testDeleteBackwardAndReturnAndDeleteAndSplit() {
        let adapter = makeAdapter()
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        _ = adapter.setContentHtml("<p>ab</p>")

        let deleted = adapter.deleteBackward(anchor: 2, head: 2)
        XCTAssertEqual(renderedText(deleted), "a")

        _ = adapter.setContentHtml("<p>ab</p>")
        let split = adapter.splitBlock(atScalar: 1)
        XCTAssertNotNil(split)
        let docAfterSplit = documentJsonObject(adapter)
        XCTAssertEqual((docAfterSplit["content"] as? [[String: Any]])?.count, 2, "Return splits into two blocks")
        XCTAssertEqual(documentText(adapter), "ab")

        // E1 FIX (was Task 15 engine defect): the v2 lowering now accepts a
        // split at the very END of a block (empty suffix) and delivers the
        // compiler preview's empty right sibling — Return-at-EOL works.
        // (Scalar 3 = end of "b": block separators count as one scalar.)
        let endSplit = adapter.splitBlock(atScalar: 3)
        XCTAssertNotNil(endSplit)
        XCTAssertTrue(spy.errors.isEmpty)
        let docAfterEndSplit = documentJsonObject(adapter)
        XCTAssertEqual((docAfterEndSplit["content"] as? [[String: Any]])?.count, 3, "Return at the end of the last block appends an empty sibling")
        XCTAssertEqual(documentText(adapter), "ab")

        // Subsequent typed input lands in the new (empty) block: after the
        // split the caret is at the new block's placeholder (scalar 4).
        let typed = adapter.insertText("c", atScalar: 4)
        XCTAssertEqual(renderedText(typed), "abc")
        XCTAssertEqual(documentText(adapter), "abc")

        // Select across the boundary and delete-and-split atomically.
        _ = adapter.setContentHtml("<p>abcd</p>")
        let update = adapter.deleteAndSplit(from: 1, to: 3)
        XCTAssertEqual(renderedText(update), "ad")
    }

    func testReadOnlyRejectsEveryMutationPathAtomically() {
        let adapter = makeAdapter(
            configJson: #"{"initialization":{"type":"localEmpty"},"policy":{"readOnly":true}}"#
        )
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record

        // Controlled local content passes under read-only (legacy Source::Api
        // pass-through parity).
        let seed = adapter.setContentHtml("<p>seed</p>")
        XCTAssertEqual(renderedText(seed), "seed")
        XCTAssertEqual(documentText(adapter), "seed")
        XCTAssertTrue(spy.errors.isEmpty)
        let revisionAfterSeed = adapter.baseDocumentRevision

        let mutations: [(String, () -> String?)] = [
            ("insertText", { adapter.insertText("x", atScalar: 0) }),
            ("replaceTextRange", { adapter.replaceTextRange(from: 0, to: 1, with: "x") }),
            ("deleteBackward", { adapter.deleteBackward(anchor: 1, head: 1) }),
            ("deleteScalarRange", { adapter.deleteScalarRange(from: 0, to: 1) }),
            ("splitBlock", { adapter.splitBlock(atScalar: 1) }),
            ("deleteAndSplit", { adapter.deleteAndSplit(from: 0, to: 1) }),
            ("insertNode", { adapter.insertNode("hardBreak", anchor: 1, head: 1) }),
            ("insertContentHtml", { adapter.insertContentHtml("<p>x</p>", anchor: 1, head: 1) }),
            ("insertContentJson", { adapter.insertContentJson("{\"type\":\"paragraph\"}", anchor: 1, head: 1) }),
            ("toggleMark", { adapter.toggleMark("bold", anchor: 0, head: 1) }),
            ("setMark", { adapter.setMark("link", attrsJson: "{\"href\":\"https://example.com\"}", anchor: 0, head: 1) }),
            ("unsetMark", { adapter.unsetMark("bold", anchor: 0, head: 1) }),
            ("toggleHeading", { adapter.toggleHeading(level: 2, anchor: 1, head: 1) }),
            ("toggleCodeBlock", { adapter.toggleCodeBlock(anchor: 1, head: 1) }),
            ("toggleBlockquote", { adapter.toggleBlockquote(anchor: 1, head: 1) }),
            ("wrapInList", { adapter.wrapInList(listType: "bulletList", itemType: "listItem", anchor: 1, head: 1) }),
            ("unwrapFromList", { adapter.unwrapFromList(anchor: 1, head: 1) }),
            ("indentListItem", { adapter.indentListItem(anchor: 1, head: 1) }),
            ("outdentListItem", { adapter.outdentListItem(anchor: 1, head: 1) }),
            ("toggleTaskItemChecked", { adapter.toggleTaskItemChecked(anchor: 1, head: 1) }),
            ("resizeImage", { adapter.resizeImage(atDocPos: 0, width: 10, height: 10) }),
            ("undo", { adapter.undo() }),
            ("redo", { adapter.redo() }),
        ]
        for (name, mutate) in mutations {
            let update = mutate()
            XCTAssertNil(update, "read-only \(name) must be rejected")
            XCTAssertEqual(spy.last?.domain, "boundary", "\(name) domain")
            XCTAssertEqual(spy.last?.code, "MUTATION_REJECTED", "\(name) code")
            XCTAssertNotNil(spy.last?.requestId, "\(name) carries the request id")
        }

        // Atomicity: content and revision untouched by every rejection.
        XCTAssertEqual(documentText(adapter), "seed")
        XCTAssertEqual(adapter.baseDocumentRevision, revisionAfterSeed)

        // Selection/navigation remains allowed under read-only.
        let mapping = adapter.syncSelection(anchor: 1, head: 1)
        XCTAssertNotNil(mapping)
        XCTAssertEqual(mapping?.docAnchor, 2)

        // Controlled content still passes (API parity).
        let replaced = adapter.setContentJson("{\"type\":\"doc\",\"content\":[{\"type\":\"paragraph\",\"content\":[{\"type\":\"text\",\"text\":\"api\"}]}]}")
        XCTAssertEqual(renderedText(replaced), "api")
    }

}
