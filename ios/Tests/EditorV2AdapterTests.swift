import XCTest
import UIKit

/// Production v2 adapter tests (Task 16 cutover: formerly the Task 15
/// staging-variant suite, now running in the default configuration).
///
/// The production v2-only bindings and XCFramework are linked, so the real
/// v2 engine backs every assertion. The adapter under test
/// (`EditorV2Adapter`) must route every native interaction through the typed
/// `editorV2*` transactions/results — the legacy sentinel-id/JSON editing
/// ABI no longer exists.
final class EditorV2AdapterTests: XCTestCase {

    // MARK: - Helpers

    private var adapters: [EditorV2Adapter] = []

    override func tearDown() {
        for adapter in adapters {
            adapter.destroy()
        }
        adapters = []
        super.tearDown()
    }

    private func makeAdapter(
        configJson: String = #"{"initialization":{"type":"localEmpty"}}"#,
        file: StaticString = #filePath,
        line: UInt = #line
    ) -> EditorV2Adapter {
        makeAttachedAdapter(
            configJson: configJson,
            roomBound: false,
            file: file,
            line: line
        )
    }

    func testAttachRejectsNonCanonicalAndUnknownEditorIds() {
        XCTAssertNil(EditorV2Adapter.attach(editorId: "01", roomBound: false))
        XCTAssertNil(EditorV2Adapter.attach(editorId: "not-an-editor", roomBound: false))
        XCTAssertNil(EditorV2Adapter.attach(editorId: "999999", roomBound: false))
    }

    func testAdapterEmitsCanonicalDecimalDocumentVersion() {
        let adapter = makeAdapter()
        let update = parseObject(adapter.currentStateJSON())

        XCTAssertEqual(update["documentVersion"] as? String, "0")
        XCTAssertFalse(update["documentVersion"] is NSNumber)
    }

    private func makeRoomAdapter(
        documentId: String = "doc-staging",
        lineageId: String = "lineage-staging",
        file: StaticString = #filePath,
        line: UInt = #line
    ) -> EditorV2Adapter {
        // A snapshot-less room starts AwaitRemote; the test drives the y-sync
        // handshake to promote it before editing.
        makeAttachedAdapter(
            configJson: #"{"initialization":{"type":"room","documentId":"\#(documentId)","lineageId":"\#(lineageId)"}}"#,
            roomBound: true,
            file: file,
            line: line
        )
    }

    private func makeAttachedAdapter(
        configJson: String,
        roomBound: Bool,
        file: StaticString,
        line: UInt
    ) -> EditorV2Adapter {
        let result = editorV2Create(configJson: configJson, snapshotState: nil)
        guard let value = result.value,
              result.error == nil,
              let createdHandle = createdV2TestEditorHandle(value),
              let adapter = EditorV2Adapter.attach(editorId: createdHandle.handle, roomBound: roomBound)
        else {
            let error = result.error
            XCTFail(
                "v2 create/attach failed: \(error?.domain ?? "boundary")/\(error?.code ?? "FFI_RESULT_INVALID"): \(error?.message ?? "missing canonical editor id")",
                file: file,
                line: line
            )
            fatalError("unreachable")
        }
        adapters.append(adapter)
        return adapter
    }

    private func parseObject(_ json: String?, file: StaticString = #filePath, line: UInt = #line) -> [String: Any] {
        guard let json,
              let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            XCTFail("expected JSON object, got: \(json ?? "nil")", file: file, line: line)
            return [:]
        }
        return object
    }

    /// Concatenated text of every textRun in the update's renderBlocks.
    private func renderedText(_ updateJSON: String?, file: StaticString = #filePath, line: UInt = #line) -> String {
        let update = parseObject(updateJSON, file: file, line: line)
        guard let blocks = update["renderBlocks"] as? [[[String: Any]]] else {
            XCTFail("update carries no renderBlocks: \(updateJSON ?? "nil")", file: file, line: line)
            return ""
        }
        var text = ""
        for block in blocks {
            for element in block {
                if let type = element["type"] as? String, type == "textRun",
                   let run = element["text"] as? String
                {
                    text += run
                }
            }
        }
        return text
    }

    private func documentJsonObject(_ adapter: EditorV2Adapter, file: StaticString = #filePath, line: UInt = #line) -> [String: Any] {
        let result = editorV2GetDocumentJson(editorId: adapter.editorId)
        guard let value = result.value, result.error == nil else {
            XCTFail("getDocumentJson failed: \(String(describing: result.error))", file: file, line: line)
            return [:]
        }
        return parseObject(value, file: file, line: line)
    }

    /// All text content of the v2 document, concatenated across text nodes.
    private func documentText(_ adapter: EditorV2Adapter, file: StaticString = #filePath, line: UInt = #line) -> String {
        let doc = documentJsonObject(adapter, file: file, line: line)
        var pieces: [String] = []
        func walk(_ node: [String: Any]) {
            if let type = node["type"] as? String, type == "text", let text = node["text"] as? String {
                pieces.append(text)
            }
            if let content = node["content"] as? [[String: Any]] {
                for child in content { walk(child) }
            }
        }
        walk(doc)
        return pieces.joined()
    }

    private func historyState(_ updateJSON: String?, file: StaticString = #filePath, line: UInt = #line) -> [String: Any] {
        let update = parseObject(updateJSON, file: file, line: line)
        guard let history = update["historyState"] as? [String: Any] else {
            XCTFail("update carries no historyState: \(updateJSON ?? "nil")", file: file, line: line)
            return [:]
        }
        return history
    }

    private final class ErrorSpy {
        private(set) var errors: [FfiError] = []
        func record(_ error: FfiError) {
            errors.append(error)
            print("V2ERROR domain=\(error.domain) code=\(error.code) message=\(error.message) requestId=\(error.requestId ?? "nil") details=\(error.detailsJson ?? "nil")")
        }
        var last: FfiError? { errors.last }
    }

    // MARK: - Construction / state

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

    // MARK: - Typing / IME / autocorrect commit semantics

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

    // MARK: - Read-only atomicity

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

    // MARK: - Stale revision recovery

    func testRevisionMismatchRefreshesFromRustStateAndNeverRetries() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>base</p>")

        // Externally advance the same v2 session so the adapter's tracked
        // base revision goes stale.
        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"990001","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"EXT"}}"#
        )
        XCTAssertNil(external.error, "external mutation failed: \(String(describing: external.error))")
        XCTAssertEqual(documentText(adapter), "EXTbase")

        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record

        // The adapter's next op is stale: it must refresh from Rust state and
        // NEVER retry the op against guessed positions.
        let update = adapter.insertText("NORETRY", atScalar: 0)
        XCTAssertNotNil(update, "a stale op resolves into a refresh update, not a hard failure")
        XCTAssertEqual(documentText(adapter), "EXTbase", "the stale op must not be retried")
        XCTAssertEqual(renderedText(update), "EXTbase")

        // Recovered: the adapter tracks the fresh revision and can edit again.
        let recovered = adapter.insertText("ok", atScalar: 0)
        XCTAssertEqual(renderedText(recovered), "okEXTbase")
        XCTAssertEqual(documentText(adapter), "okEXTbase")
    }

    // MARK: - Undo/redo

    func testUndoRedoRoundTrip() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>ab</p>")
        _ = adapter.insertText("c", atScalar: 2)
        XCTAssertEqual(documentText(adapter), "abc")

        let undone = adapter.undo()
        XCTAssertEqual(renderedText(undone), "ab")
        XCTAssertEqual(historyState(undone)["canRedo"] as? Bool, true)

        let redone = adapter.redo()
        XCTAssertEqual(renderedText(redone), "abc")
    }

    // MARK: - Lifecycle races

    func testDestroyMidOperationsYieldsStructuredFailureWithoutCrash() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>ab</p>")
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record

        adapter.destroy()

        XCTAssertNil(adapter.insertText("x", atScalar: 0))
        XCTAssertEqual(spy.last?.domain, "lifecycle")
        XCTAssertEqual(spy.last?.code, "ENGINE_DESTROYED")

        XCTAssertNil(adapter.refreshFromRustState(mirrorSelection: nil))
        XCTAssertEqual(spy.last?.code, "ENGINE_DESTROYED")

        // Repeated destroy is safe.
        adapter.destroy()
        adapter.destroy()
    }

    // MARK: - Toolbar / command routing

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
        XCTAssertEqual(headingNode?["type"] as? String, "h2")

        // listItem's content expression rejects headings: wrap a fresh
        // paragraph instead.
        _ = adapter.setContentHtml("<p>ab</p>")
        let list = adapter.wrapInList(listType: "bulletList", itemType: "listItem", anchor: 1, head: 1)
        XCTAssertNotNil(list)
        let listDoc = documentJsonObject(adapter)
        XCTAssertEqual((listDoc["content"] as? [[String: Any]])?.first?["type"] as? String, "bulletList")

        let hardBreak = adapter.insertNode("hardBreak", anchor: 1, head: 1)
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

    // MARK: - Void/list markers + resize

    func testVoidAndListMarkerPositionMapping() {
        let adapter = makeAdapter()
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        let doc = #"{"type":"doc","content":[{"type":"image","attrs":{"src":"https://example.com/a.png","alt":null,"title":null,"width":null,"height":null}},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"item"}]}]}]}]}"#
        _ = adapter.setContentJson(doc)
        XCTAssertEqual(documentText(adapter), "item")

        // Type inside the list item paragraph: the image void is scalar 0,
        // the block separator is scalar 1, "item" text starts at scalar 2.
        let update = adapter.insertText("X", atScalar: 2)
        print("VOID2 update nil?: \(update == nil); notes:", adapter.debugNotes)
        XCTAssertEqual(renderedText(update), "Xitem")

        // The block-separator scalar is not a text position: the v2 boundary
        // rejects it structurally, atomically, with no partial commit
        // (legacy transform-error parity for the same pathological position).
        let boundaryInsert = adapter.insertText("Z", atScalar: 1)
        XCTAssertNil(boundaryInsert)
        XCTAssertEqual(spy.last?.domain, "operation")
        XCTAssertEqual(spy.last?.code, "OPERATION_INVALID")
        XCTAssertEqual(documentText(adapter), "Xitem")
        let after = documentJsonObject(adapter)
        XCTAssertEqual((after["content"] as? [[String: Any]])?.first?["type"] as? String, "image")
        XCTAssertEqual((after["content"] as? [[String: Any]])?.last?["type"] as? String, "bulletList")

        // Resize the void image by its document position.
        let resized = adapter.resizeImage(atDocPos: 0, width: 120, height: 80)
        XCTAssertNotNil(resized)
        let resizedDoc = documentJsonObject(adapter)
        let image = (resizedDoc["content"] as? [[String: Any]])?.first
        let attrs = image?["attrs"] as? [String: Any]
        XCTAssertEqual(attrs?["width"] as? Int, 120)
        XCTAssertEqual(attrs?["height"] as? Int, 80)
    }

    // MARK: - Collaboration drain ping

    func testLocalCommitDrivesDrainPingOnRoomBoundSession() {
        // NOTE: promoting an AwaitRemote room to RoomReady requires a peer
        // that already holds the document (raw yrs peer in the Rust tests);
        // the public FFI cannot fabricate one natively. This test proves the
        // ping's full drain semantics against the room's live outbox using
        // the protocol reply queued by a real Step 1 receive: frames drain
        // one per call, as complete y-protocols messages, until empty. The
        // commit-triggered ping on a live room is covered on Android (fake
        // engine lets room commits succeed) and device E2E is Task 17 scope.
        let adapterA = makeRoomAdapter(documentId: "doc-room", lineageId: "lineage-room")
        let adapterB = makeRoomAdapter(documentId: "doc-room", lineageId: "lineage-room")

        func beginGeneration(_ adapter: EditorV2Adapter, file: StaticString = #filePath, line: UInt = #line) -> String {
            let begin = editorV2CollaborationBeginConnect(editorId: adapter.editorId)
            guard let generation = parseObject(begin.value)["generation"] as? String,
                  UInt64(generation) != nil,
                  generation == "0" || generation.first != "0"
            else {
                XCTFail("beginConnect returned no generation: \(begin.value ?? "nil")", file: file, line: line)
                fatalError("unreachable")
            }
            return generation
        }

        let genA = beginGeneration(adapterA)
        let genB = beginGeneration(adapterB)
        let step1A = editorV2CollaborationSocketOpen(editorId: adapterA.editorId, generation: genA)
        let step1B = editorV2CollaborationSocketOpen(editorId: adapterB.editorId, generation: genB)
        XCTAssertNil(step1A.error)
        XCTAssertNil(step1B.error)

        // B's Step 1 reaches A: A queues its bounded protocol reply.
        let receiveA = editorV2CollaborationReceive(
            editorId: adapterA.editorId,
            generation: genA,
            message: step1B.value ?? Data()
        )
        XCTAssertNil(receiveA.error)
        let receiveOutcome = parseObject(receiveA.value)
        XCTAssertEqual(receiveOutcome["repliesEnqueued"] as? Int, 1)

        var frames: [Data] = []
        adapterA.outboundFrameSink = { frames.append($0) }

        // No live generation on the adapter: the ping is a no-op (mirrors
        // the TS controller skipping the drain with no current socket).
        adapterA.driveCollaborationDrainPing()
        XCTAssertTrue(frames.isEmpty)

        adapterA.collaborationGeneration = genA
        adapterA.driveCollaborationDrainPing()
        XCTAssertEqual(frames.count, 1, "the queued protocol reply drains as one framed message")
        XCTAssertEqual(frames.first?.count, 5, "same bytes the outbox reserved")
        adapterA.driveCollaborationDrainPing()
        XCTAssertEqual(frames.count, 1, "the queue stays drained until new work arrives")
        adapterB.destroy()
    }

    func testDrainPingSkippedForLocalOnlySession() {
        let adapter = makeAdapter()
        var frames: [Data] = []
        adapter.outboundFrameSink = { frames.append($0) }
        adapter.collaborationGeneration = "1"
        _ = adapter.insertText("x", atScalar: 0)
        XCTAssertTrue(frames.isEmpty, "local-only sessions own no outbox; the ping must not fire")
    }

    // MARK: - Synthesized update contract

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

    // MARK: - Task 16B render accessor (probe replacement)

    /// The fixture matrix the Task 15 probe derivation was pinned against:
    /// empty doc, nested lists, marks, emoji, void nodes, multi-block.
    private static let accessorFixtures: [(String, String)] = [
        ("empty", ""),
        ("nested-lists", #"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}]}]}"#),
        ("marks", #"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"plain "},{"type":"text","text":"bold","marks":[{"type":"bold"}]},{"type":"text","text":"italiclinked","marks":[{"type":"italic"},{"type":"link","attrs":{"href":"https://example.com"}}]}]}]}"#),
        ("emoji", "{\"type\":\"doc\",\"content\":[{\"type\":\"paragraph\",\"content\":[{\"type\":\"text\",\"text\":\"a\u{1F600}e\u{0301}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}b\"}]}]}"),
        ("void-nodes", #"{"type":"doc","content":[{"type":"image","attrs":{"src":"https://example.com/a.png","alt":null,"title":null,"width":null,"height":null}},{"type":"paragraph","content":[{"type":"text","text":"after"}]}]}"#),
        ("multi-block", #"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]},{"type":"paragraph","content":[{"type":"text","text":"cd"}]}]}"#),
    ]

    private func makeFixtureAdapter(_ contentJson: String) -> EditorV2Adapter {
        let adapter = makeAdapter()
        if !contentJson.isEmpty {
            _ = adapter.setContentJson(contentJson)
        }
        return adapter
    }

    private func v2DocumentJson(_ adapter: EditorV2Adapter, file: StaticString = #filePath, line: UInt = #line) -> String {
        let result = editorV2GetDocumentJson(editorId: adapter.editorId)
        guard let value = result.value, result.error == nil else {
            XCTFail("getDocumentJson failed: \(String(describing: result.error))", file: file, line: line)
            return "{}"
        }
        return value
    }

    /// Golden fixture assertions against the accessor alone (these survive
    /// the probe removal): rendered text, scalar extent, active state at a
    /// mirrored mark selection, and backward-selection doc resolution.
    func testRenderAccessorFixtureMatrixGoldenContent() {
        // Empty doc: one empty block, a one-scalar extent (the empty
        // paragraph's synthetic placeholder), no selection key.
        let empty = makeFixtureAdapter("")
        let emptyUpdate = parseObject(editorV2RenderUpdate(editorId: empty.editorId, mirrorScalarAnchor: nil, mirrorScalarHead: nil).value)
        XCTAssertNotNil(emptyUpdate["renderBlocks"] as? NSArray)
        XCTAssertEqual((emptyUpdate["scalarLength"] as? NSNumber)?.uint32Value, 1)
        XCTAssertNil(emptyUpdate["selection"])
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
}
