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

    private enum TestHookError: Error {
        case failed
    }

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

    func testRequestIdExhaustionEmitsMaxOnceThenRejectsLocally() {
        let adapter = makeAdapter()
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        adapter.setNextRequestIdForTesting(UInt64.max - 1)

        let backendCallsBefore = adapter.backendEnvelopeCallCountForTesting
        XCTAssertNotNil(adapter.setContentHtml("<p>max</p>"))
        XCTAssertEqual(adapter.lastRequestIdForTesting, UInt64.max)
        XCTAssertEqual(adapter.backendEnvelopeCallCountForTesting, backendCallsBefore + 1)

        XCTAssertNil(adapter.setContentHtml("<p>must not reach backend</p>"))
        XCTAssertEqual(adapter.lastRequestIdForTesting, UInt64.max)
        XCTAssertEqual(adapter.backendEnvelopeCallCountForTesting, backendCallsBefore + 1)
        XCTAssertEqual(spy.last?.domain, "boundary")
        XCTAssertEqual(spy.last?.code, "CONFIG_INVALID")
        XCTAssertEqual(spy.last?.requestId, String(UInt64.max))
        XCTAssertEqual(documentText(adapter), "max")
    }

    func testTask15AutonomousErrorOwnerTokensProtectNewerOwnersFromStaleClears() {
        let adapter = makeAdapter()
        let firstOwner = UUID()
        let secondOwner = UUID()
        var firstErrors: [FfiError] = []
        var secondErrors: [FfiError] = []

        adapter.bindAutonomousErrorOwner(token: firstOwner) { firstErrors.append($0) }
        adapter.bindAutonomousErrorOwner(token: secondOwner) { secondErrors.append($0) }
        adapter.clearAutonomousErrorOwner(token: firstOwner)
        adapter.rejectExternalRenderEnvelope("first real adapter failure")

        XCTAssertTrue(firstErrors.isEmpty)
        XCTAssertEqual(secondErrors.count, 1)
        XCTAssertTrue(adapter.isAutonomousErrorOwner(token: secondOwner))

        adapter.clearAutonomousErrorOwner(token: secondOwner)
        adapter.rejectExternalRenderEnvelope("cleared owner must not receive failures")
        XCTAssertEqual(secondErrors.count, 1)
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
        setAwarenessSelection: @escaping (String, String) -> FfiJsonResult = {
            editorV2CollaborationSetAwarenessSelection(editorId: $0, selectionJson: $1)
        },
        collaborationWake: @escaping (UInt64, CollaborationWakeReason) -> Void = {
            NativeCollaborationTransportRegistry.notifyOutboundAvailable(
                editorId: $0,
                reason: $1
            )
        },
        file: StaticString,
        line: UInt
    ) -> EditorV2Adapter {
        let result = editorV2Create(configJson: configJson, snapshotState: nil)
        guard let value = result.value,
              result.error == nil,
              let createdHandle = createdV2TestEditorHandle(value),
              let adapter = EditorV2Adapter.attach(
                editorId: createdHandle.handle,
                roomBound: roomBound,
                setAwarenessSelection: setAwarenessSelection,
                collaborationWake: collaborationWake
              )
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

    private func mutatedObjectJSON(
        _ json: String,
        file: StaticString = #filePath,
        line: UInt = #line,
        _ mutate: (inout [String: Any]) -> Void
    ) -> String {
        var object = parseObject(json, file: file, line: line)
        mutate(&object)
        guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]),
              let result = String(data: data, encoding: .utf8)
        else {
            XCTFail("failed to serialize mutated snapshot", file: file, line: line)
            return "{}"
        }
        return result
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

        let mapping = adapter.syncSelection(anchor: 1, head: 1)

        XCTAssertEqual(mapping?.docAnchor, 2)
        XCTAssertEqual(mapping?.docHead, 2)
        XCTAssertEqual(calls, ["awarenessSelection", "wake:awareness"])
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

    func testForcedRevisionRaceRefreshesOnceWithoutRetryingTheMutation() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>base</p>")

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"990002","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"EXT"}}"#
        )
        XCTAssertNil(external.error, "external mutation failed: \(String(describing: external.error))")

        let backendCallsBefore = adapter.backendEnvelopeCallCountForTesting
        let renderCallsBefore = adapter.renderUpdateCallCountForTesting
        let update = adapter.insertText("NORETRY", atScalar: 0)

        XCTAssertNotNil(update, "the race resolves into one authoritative refresh")
        XCTAssertEqual(
            adapter.backendEnvelopeCallCountForTesting,
            backendCallsBefore + 1,
            "the stale mutation must be attempted once and never retried"
        )
        XCTAssertEqual(
            adapter.renderUpdateCallCountForTesting,
            renderCallsBefore + 1,
            "a genuine revision race must perform exactly one render refresh"
        )
        XCTAssertEqual(documentText(adapter), "EXTbase", "the failed mutation must not be replayed")
        XCTAssertEqual(renderedText(update), "EXTbase")
    }

    func testAtomicRenderValidationAcceptsAValidRenderPatch() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>base</p>")
        let raw = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value!
        let withPatch = mutatedObjectJSON(raw) { object in
            object["renderPatch"] = [
                "startIndex": 0,
                "deleteCount": 0,
                "renderBlocks": object["renderBlocks"]!,
            ]
        }

        XCTAssertNotNil(adapter.adoptExternalRender(withPatch))
    }

    func testMalformedAtomicRenderNestedVariantsLeaveEveryCacheUnchanged() {
        let adapter = makeAdapter()
        let spy = ErrorSpy()
        adapter.onAutonomousError = spy.record
        _ = adapter.setContentHtml("<p>base</p>")
        let raw = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value!
        XCTAssertNotNil(adapter.adoptExternalRender(raw))
        let baseline = adapter.cacheStateForTesting
        let baselineDebugNotes = adapter.debugNotes

        let variants: [(String, (inout [String: Any]) -> Void)] = [
            ("extra top-level field", { $0["legacyRevision"] = 1 }),
            ("null selection", { $0["selection"] = NSNull() }),
            ("selection extra field", { object in
                var selection = object["selection"] as! [String: Any]
                selection["legacyAnchor"] = 0
                object["selection"] = selection
            }),
            ("selection scalar above u32", { object in
                var selection = object["selection"] as! [String: Any]
                selection["anchorScalar"] = NSNumber(value: UInt64(UInt32.max) + 1)
                object["selection"] = selection
            }),
            ("node selection fractional scalar", {
                $0["selection"] = ["type": "node", "pos": 1, "posScalar": 0.5]
            }),
            ("all selection extra field", {
                $0["selection"] = ["type": "all", "anchor": 0]
            }),
            ("invalid text mark", { object in
                var blocks = object["renderBlocks"] as! [[[String: Any]]]
                let index = blocks[0].firstIndex { $0["type"] as? String == "textRun" }!
                var textRun = blocks[0][index]
                textRun["marks"] = [["type": 7]]
                blocks[0][index] = textRun
                object["renderBlocks"] = blocks
            }),
            ("text run extra field", { object in
                var blocks = object["renderBlocks"] as! [[[String: Any]]]
                let index = blocks[0].firstIndex { $0["type"] as? String == "textRun" }!
                blocks[0][index]["legacyText"] = "base"
                object["renderBlocks"] = blocks
            }),
            ("block start list u32 above range", { object in
                var blocks = object["renderBlocks"] as! [[[String: Any]]]
                let index = blocks[0].firstIndex { $0["type"] as? String == "blockStart" }!
                var blockStart = blocks[0][index]
                blockStart["listContext"] = [
                    "ordered": false,
                    "index": NSNumber(value: UInt64(UInt32.max) + 1),
                    "total": 1,
                    "start": 1,
                    "isFirst": true,
                    "isLast": true,
                ]
                blocks[0][index] = blockStart
                object["renderBlocks"] = blocks
            }),
            ("block start invalid list boolean", { object in
                var blocks = object["renderBlocks"] as! [[[String: Any]]]
                let index = blocks[0].firstIndex { $0["type"] as? String == "blockStart" }!
                var blockStart = blocks[0][index]
                blockStart["listContext"] = [
                    "ordered": 1,
                    "index": 1,
                    "total": 1,
                    "start": 1,
                    "isFirst": true,
                    "isLast": true,
                ]
                blocks[0][index] = blockStart
                object["renderBlocks"] = blocks
            }),
            ("block end extra field", {
                $0["renderBlocks"] = [["type": "blockEnd", "legacy": true]]
            }),
            ("void inline array attrs", {
                $0["renderBlocks"] = [[
                    "type": "voidInline",
                    "nodeType": "image",
                    "docPos": 0,
                    "attrs": [],
                ]]
            }),
            ("void block fractional doc position", {
                $0["renderBlocks"] = [[
                    "type": "voidBlock",
                    "nodeType": "image",
                    "docPos": 0.5,
                ]]
            }),
            ("opaque inline invalid mention theme", {
                $0["renderBlocks"] = [[
                    "type": "opaqueInlineAtom",
                    "nodeType": "mention",
                    "label": "Ada",
                    "docPos": 0,
                    "mentionTheme": ["borderWidth": true],
                ]]
            }),
            ("opaque block extra field", {
                $0["renderBlocks"] = [[
                    "type": "opaqueBlockAtom",
                    "nodeType": "unknown",
                    "label": "Unknown",
                    "docPos": 0,
                    "attrs": [:],
                ]]
            }),
            ("render patch invalid nested element", { object in
                object["renderPatch"] = [
                    "startIndex": 0,
                    "deleteCount": 0,
                    "renderBlocks": [["type": "unknownElement"]],
                ]
            }),
            ("render patch fractional start", { object in
                object["renderPatch"] = [
                    "startIndex": 0.5,
                    "deleteCount": 0,
                    "renderBlocks": object["renderBlocks"]!,
                ]
            }),
            ("active-state extra field", { object in
                var active = object["activeState"] as! [String: Any]
                active["legacy"] = false
                object["activeState"] = active
            }),
            ("active-state non-boolean map value", { object in
                var active = object["activeState"] as! [String: Any]
                active["marks"] = ["bold": "yes"]
                object["activeState"] = active
            }),
            ("active-state non-record mark attrs", { object in
                var active = object["activeState"] as! [String: Any]
                active["markAttrs"] = ["link": "https://example.com"]
                object["activeState"] = active
            }),
            ("active-state non-string insertion", { object in
                var active = object["activeState"] as! [String: Any]
                active["insertableNodes"] = ["image", 1]
                object["activeState"] = active
            }),
            ("history numeric boolean", { object in
                object["historyState"] = ["canUndo": 1, "canRedo": false]
            }),
            ("non-canonical revision", { $0["documentVersion"] = "01" }),
            ("state revision numeric", { $0["stateRevision"] = 1 }),
            ("scalar length above u32", {
                $0["scalarLength"] = NSNumber(value: UInt64(UInt32.max) + 1)
            }),
            ("scalar length fractional", { $0["scalarLength"] = 0.5 }),
        ]

        for (name, mutate) in variants {
            let errorsBefore = spy.errors.count
            let malformed = mutatedObjectJSON(raw, mutate)
            XCTAssertNil(adapter.adoptExternalRender(malformed), name)
            XCTAssertEqual(adapter.cacheStateForTesting, baseline, name)
            XCTAssertEqual(adapter.debugNotes, baselineDebugNotes, name)
            XCTAssertEqual(spy.errors.count, errorsBefore + 1, name)
            XCTAssertEqual(spy.last?.domain, "boundary", name)
            XCTAssertEqual(spy.last?.code, "FFI_RESULT_INVALID", name)
        }
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

    func testDestroyFailureRetainsThePairAndErrorOwnerUntilRetrySucceeds() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { editorId in
                destroyAttempts += 1
                if destroyAttempts == 1 {
                    return FfiUnitResult(
                        value: nil,
                        error: FfiError(
                            domain: "operation",
                            code: "OPERATION_INVALID",
                            message: "temporary destroy failure",
                            requestId: nil,
                            operationIndex: nil,
                            limit: nil,
                            actual: nil,
                            detailsJson: nil
                        )
                    )
                }
                return editorV2Destroy(editorId: editorId)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        defer { EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId) }

        let owner = UUID()
        var deliveredErrors: [FfiError] = []
        adapter.bindAutonomousErrorOwner(token: owner) { deliveredErrors.append($0) }

        let firstError = EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId)
        XCTAssertEqual(firstError?.code, "OPERATION_INVALID")
        XCTAssertFalse(adapter.isDestroyed)
        XCTAssertTrue(adapter.isAutonomousErrorOwner(token: owner))
        XCTAssertTrue(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId) === adapter)

        adapter.rejectExternalRenderEnvelope("pair must remain live after destroy failure")
        XCTAssertEqual(deliveredErrors.count, 1)

        XCTAssertNil(EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId))
        XCTAssertTrue(adapter.isDestroyed)
        XCTAssertFalse(adapter.isAutonomousErrorOwner(token: owner))
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))
        XCTAssertEqual(destroyAttempts, 2)

        adapter.rejectExternalRenderEnvelope("destroyed adapter must not deliver again")
        XCTAssertEqual(deliveredErrors.count, 1)
    }

    func testDestroyWithNeitherValueNorErrorRetainsThePairUntilRetrySucceeds() {
        assertMalformedDestroyResultRetainsPairUntilRetry(
            FfiUnitResult(value: nil, error: nil)
        )
    }

    func testDestroyWithBothValueAndErrorRetainsThePairUntilRetrySucceeds() {
        assertMalformedDestroyResultRetainsPairUntilRetry(
            FfiUnitResult(
                value: true,
                error: FfiError(
                    domain: "lifecycle",
                    code: "ENGINE_DESTROYED",
                    message: "malformed destroy result",
                    requestId: nil,
                    operationIndex: nil,
                    limit: nil,
                    actual: nil,
                    detailsJson: nil
                )
            )
        )
    }

    func testDestroyReservesTheViewBeforeFfiAndRollsBackAfterRetryableFailure() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        let registry = NativeEditorViewRegistry.shared
        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                destroyAttempts += 1
                XCTAssertTrue(registry.isDestroyed(editorId: handle.nativeViewId))
                XCTAssertTrue(
                    registry.prepareForCommandJSON(editorId: handle.nativeViewId)
                        .contains("\"ready\":false")
                )
                return FfiUnitResult(
                    value: nil,
                    error: FfiError(
                        domain: "operation",
                        code: "OPERATION_INVALID",
                        message: "retryable destroy failure",
                        requestId: nil,
                        operationIndex: nil,
                        limit: nil,
                        actual: nil,
                        detailsJson: nil
                    )
                )
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        registry.markEditorCreated(editorId: handle.nativeViewId)
        defer {
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            registry.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        let error = EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId)

        XCTAssertEqual(error?.code, "OPERATION_INVALID")
        XCTAssertEqual(destroyAttempts, 1)
        XCTAssertFalse(registry.isDestroyed(editorId: handle.nativeViewId))
        XCTAssertTrue(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId) === adapter)
        XCTAssertEqual(
            commandPreparation(registry.prepareForCommandJSON(editorId: handle.nativeViewId)),
            nil
        )
    }

    func testDestroyReservationContentionReturnsRetryableErrorThenOwnerSuccessFinalizesOnce() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        let firstFfiEntered = expectation(description: "first destroy ffi entered")
        let firstDestroyFinished = expectation(description: "first destroy finished")
        let releaseFirstFfi = DispatchSemaphore(value: 0)
        let attemptsLock = NSLock()
        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                attemptsLock.lock()
                destroyAttempts += 1
                let attempt = destroyAttempts
                attemptsLock.unlock()
                if attempt == 1 {
                    firstFfiEntered.fulfill()
                    _ = releaseFirstFfi.wait(timeout: .now() + 1)
                }
                return FfiUnitResult(value: true, error: nil)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        let viewRegistry = NativeEditorViewRegistry.shared
        viewRegistry.markEditorCreated(editorId: handle.nativeViewId)
        var finalizationChecks = 0
        viewRegistry.onFinalizeDestroyForTesting = { editorId in
            guard editorId == handle.nativeViewId else { return }
            finalizationChecks += 1
            XCTAssertNil(EditorV2Registry.adapter(forLegacyId: editorId))
            XCTAssertTrue(viewRegistry.isDestroyReserved(editorId: editorId))
            XCTAssertTrue(
                viewRegistry.prepareForCommandJSON(editorId: editorId)
                    .contains("\"ready\":false")
            )
        }
        defer {
            viewRegistry.onFinalizeDestroyForTesting = nil
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            viewRegistry.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        DispatchQueue.global().async {
            let ownerResult = destroyEditorV2FromModule(editorId: handle.handle)
            XCTAssertEqual(ownerResult.value, true)
            XCTAssertNil(ownerResult.error)
            firstDestroyFinished.fulfill()
        }
        wait(for: [firstFfiEntered], timeout: 1)

        let contentionResult = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertNil(contentionResult.value)
        XCTAssertEqual(contentionResult.error?.domain, "operation")
        XCTAssertEqual(contentionResult.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(contentionResult.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 1)
        releaseFirstFfi.signal()
        wait(for: [firstDestroyFinished], timeout: 1)

        XCTAssertEqual(destroyAttempts, 1)
        XCTAssertEqual(finalizationChecks, 1)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))
    }

    func testDestroyReservationContentionAllowsRetryAfterOwnerRollback() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        let firstFfiEntered = expectation(description: "first destroy ffi entered")
        let firstDestroyFinished = expectation(description: "first destroy rolled back")
        let releaseFirstFfi = DispatchSemaphore(value: 0)
        let attemptsLock = NSLock()
        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                attemptsLock.lock()
                destroyAttempts += 1
                let attempt = destroyAttempts
                attemptsLock.unlock()
                if attempt == 1 {
                    firstFfiEntered.fulfill()
                    _ = releaseFirstFfi.wait(timeout: .now() + 1)
                    return FfiUnitResult(
                        value: nil,
                        error: FfiError(
                            domain: "operation",
                            code: "OPERATION_INVALID",
                            message: "owner retryable destroy failure",
                            requestId: nil,
                            operationIndex: nil,
                            limit: nil,
                            actual: nil,
                            detailsJson: nil
                        )
                    )
                }
                return FfiUnitResult(value: true, error: nil)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: handle.nativeViewId)
        defer {
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        DispatchQueue.global().async {
            let ownerResult = destroyEditorV2FromModule(editorId: handle.handle)
            XCTAssertNil(ownerResult.value)
            XCTAssertEqual(ownerResult.error?.message, "owner retryable destroy failure")
            firstDestroyFinished.fulfill()
        }
        wait(for: [firstFfiEntered], timeout: 1)

        let contentionResult = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertNil(contentionResult.value)
        XCTAssertEqual(contentionResult.error?.domain, "operation")
        XCTAssertEqual(contentionResult.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(contentionResult.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 1)

        releaseFirstFfi.signal()
        wait(for: [firstDestroyFinished], timeout: 1)
        XCTAssertTrue(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId) === adapter)
        XCTAssertFalse(NativeEditorViewRegistry.shared.isDestroyed(editorId: handle.nativeViewId))
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))

        let retryResult = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(retryResult.value, true)
        XCTAssertNil(retryResult.error)
        XCTAssertEqual(destroyAttempts, 2)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))
    }

    func testThrowingHandleReservationHookDoesNotStrandDestroyRetry() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                destroyAttempts += 1
                return destroyAttempts == 1
                    ? FfiUnitResult(
                        value: nil,
                        error: FfiError(
                            domain: "operation",
                            code: "OPERATION_INVALID",
                            message: "retryable",
                            requestId: nil,
                            operationIndex: nil,
                            limit: nil,
                            actual: nil,
                            detailsJson: nil
                        )
                    )
                    : FfiUnitResult(value: true, error: nil)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: handle.nativeViewId)
        let throwingHook: (UInt64) throws -> Void = { editorId in
            if editorId == handle.nativeViewId { throw TestHookError.failed }
        }
        EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = throwingHook
        defer {
            EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = nil
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        let first = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(first.error?.message, "retryable")
        XCTAssertTrue(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId) === adapter)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
        XCTAssertFalse(NativeEditorViewRegistry.shared.isDestroyReserved(editorId: handle.nativeViewId))

        EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = nil
        let retry = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(retry.value, true)
        XCTAssertNil(retry.error)
        XCTAssertEqual(destroyAttempts, 2)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
    }

    func testThrowingPairRemovalHookPreservesTerminalResultAndFinalizesDestroy() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                destroyAttempts += 1
                return FfiUnitResult(value: true, error: nil)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: handle.nativeViewId)
        let throwingHook: (UInt64) throws -> Void = { editorId in
            if editorId == handle.nativeViewId { throw TestHookError.failed }
        }
        EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = throwingHook
        defer {
            EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = nil
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        let result = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(result.value, true)
        XCTAssertNil(result.error)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
        XCTAssertTrue(NativeEditorViewRegistry.shared.isDestroyed(editorId: handle.nativeViewId))

        EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = nil
        let subsequent = destroyEditorV2FromModule(
            editorId: handle.handle,
            destroy: { _ in
                destroyAttempts += 1
                return FfiUnitResult(value: true, error: nil)
            }
        )
        XCTAssertEqual(subsequent.value, true)
        XCTAssertNil(subsequent.error)
        XCTAssertEqual(destroyAttempts, 2)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
    }

    private func assertMalformedDestroyResultRetainsPairUntilRetry(
        _ malformedResult: FfiUnitResult,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed", file: file, line: line)
            return
        }

        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { editorId in
                destroyAttempts += 1
                return destroyAttempts == 1
                    ? malformedResult
                    : editorV2Destroy(editorId: editorId)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed", file: file, line: line)
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        defer { EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId) }

        let owner = UUID()
        adapter.bindAutonomousErrorOwner(token: owner) { _ in }

        let firstError = EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId)
        XCTAssertEqual(firstError?.domain, "boundary", file: file, line: line)
        XCTAssertEqual(firstError?.code, "FFI_RESULT_INVALID", file: file, line: line)
        XCTAssertFalse(adapter.isDestroyed, file: file, line: line)
        XCTAssertTrue(adapter.isAutonomousErrorOwner(token: owner), file: file, line: line)
        XCTAssertTrue(
            EditorV2Registry.adapter(forLegacyId: handle.nativeViewId) === adapter,
            file: file,
            line: line
        )
        XCTAssertFalse(
            EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId),
            file: file,
            line: line
        )

        XCTAssertNil(
            EditorV2Registry.destroyPair(forLegacyId: handle.nativeViewId),
            file: file,
            line: line
        )
        XCTAssertTrue(adapter.isDestroyed, file: file, line: line)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId), file: file, line: line)
        XCTAssertEqual(destroyAttempts, 2, file: file, line: line)
    }

    func testHandleTransactionBlocksContenderBeforePairLookup() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        let reservationAcquired = expectation(description: "handle reservation acquired before pair lookup")
        let ownerFinished = expectation(description: "owner finished")
        let releaseOwner = DispatchSemaphore(value: 0)
        let attemptsLock = NSLock()
        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                attemptsLock.lock()
                destroyAttempts += 1
                attemptsLock.unlock()
                return FfiUnitResult(value: true, error: nil)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: handle.nativeViewId)
        EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = { editorId in
            guard editorId == handle.nativeViewId else { return }
            reservationAcquired.fulfill()
            _ = releaseOwner.wait(timeout: .now() + 1)
        }
        defer {
            EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = nil
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        DispatchQueue.global().async {
            let result = destroyEditorV2FromModule(editorId: handle.handle)
            XCTAssertEqual(result.value, true)
            XCTAssertNil(result.error)
            ownerFinished.fulfill()
        }
        wait(for: [reservationAcquired], timeout: 1)

        let contender = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertNil(contender.value)
        XCTAssertEqual(contender.error?.domain, "operation")
        XCTAssertEqual(contender.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(contender.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 0)

        releaseOwner.signal()
        wait(for: [ownerFinished], timeout: 1)
        XCTAssertEqual(destroyAttempts, 1)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
    }

    func testHandleTransactionBlocksContenderAfterFfiAndAfterPairRemoval() {
        let created = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let value = created.value,
              created.error == nil,
              let handle = createdV2TestEditorHandle(value)
        else {
            XCTFail("expected v2 editor creation to succeed")
            return
        }

        let ffiReturned = expectation(description: "destroy ffi returned")
        let pairRemoved = expectation(description: "pair removed before finalization")
        let ownerFinished = expectation(description: "owner finalized")
        let releaseAfterFfi = DispatchSemaphore(value: 0)
        let releaseAfterPairRemoval = DispatchSemaphore(value: 0)
        let attemptsLock = NSLock()
        var destroyAttempts = 0
        guard let adapter = EditorV2Adapter.attach(
            editorId: handle.handle,
            roomBound: false,
            destroySession: { _ in
                attemptsLock.lock()
                destroyAttempts += 1
                attemptsLock.unlock()
                return FfiUnitResult(value: true, error: nil)
            }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: handle.nativeViewId)
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: handle.nativeViewId)
        EditorV2Registry.onDestroyFfiResultReceivedForTesting = { editorId in
            guard editorId == handle.nativeViewId else { return }
            ffiReturned.fulfill()
            _ = releaseAfterFfi.wait(timeout: .now() + 1)
        }
        EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = { editorId in
            guard editorId == handle.nativeViewId else { return }
            pairRemoved.fulfill()
            _ = releaseAfterPairRemoval.wait(timeout: .now() + 1)
        }
        defer {
            EditorV2Registry.onDestroyFfiResultReceivedForTesting = nil
            EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = nil
            EditorV2Registry.removePairing(forLegacyId: handle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: handle.nativeViewId)
            _ = editorV2Destroy(editorId: handle.handle)
        }

        DispatchQueue.global().async {
            let result = destroyEditorV2FromModule(editorId: handle.handle)
            XCTAssertEqual(result.value, true)
            XCTAssertNil(result.error)
            ownerFinished.fulfill()
        }
        wait(for: [ffiReturned], timeout: 1)

        let afterFfi = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(afterFfi.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(afterFfi.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 1)

        releaseAfterFfi.signal()
        wait(for: [pairRemoved], timeout: 1)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: handle.nativeViewId))

        let afterPairRemoval = destroyEditorV2FromModule(editorId: handle.handle)
        XCTAssertEqual(afterPairRemoval.error?.code, "OPERATION_INVALID")
        XCTAssertEqual(afterPairRemoval.error?.message, "destroy already in progress")
        XCTAssertEqual(destroyAttempts, 1)

        releaseAfterPairRemoval.signal()
        wait(for: [ownerFinished], timeout: 1)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle.nativeViewId))
    }

    func testHandleTransactionReturnsOriginalLifecycleTerminalResultForPairedAndUnpairedEditors() {
        let paired = editorV2Create(
            configJson: #"{"initialization":{"type":"localEmpty"}}"#,
            snapshotState: nil
        )
        guard let pairedValue = paired.value,
              paired.error == nil,
              let pairedHandle = createdV2TestEditorHandle(pairedValue)
        else {
            XCTFail("expected paired v2 editor creation to succeed")
            return
        }
        let lifecycle = FfiError(
            domain: "lifecycle",
            code: "ENGINE_DESTROYED",
            message: "already destroyed by the engine",
            requestId: "request-7",
            operationIndex: "3",
            limit: nil,
            actual: nil,
            detailsJson: #"{"source":"test"}"#
        )
        guard let adapter = EditorV2Adapter.attach(
            editorId: pairedHandle.handle,
            roomBound: false,
            destroySession: { _ in FfiUnitResult(value: nil, error: lifecycle) }
        )
        else {
            XCTFail("expected v2 adapter attachment to succeed")
            return
        }
        adapters.append(adapter)
        EditorV2Registry.register(adapter, forLegacyId: pairedHandle.nativeViewId)
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: pairedHandle.nativeViewId)
        defer {
            EditorV2Registry.removePairing(forLegacyId: pairedHandle.nativeViewId)
            NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: pairedHandle.nativeViewId)
            _ = editorV2Destroy(editorId: pairedHandle.handle)
        }

        let pairedResult = destroyEditorV2FromModule(editorId: pairedHandle.handle)
        XCTAssertNil(pairedResult.value)
        XCTAssertEqual(pairedResult.error, lifecycle)
        XCTAssertTrue(adapter.isDestroyed)
        XCTAssertNil(EditorV2Registry.adapter(forLegacyId: pairedHandle.nativeViewId))

        let unpairedId: UInt64 = 9_000_111
        let unpairedResult = destroyEditorV2FromModule(
            editorId: String(unpairedId),
            destroy: { _ in FfiUnitResult(value: nil, error: lifecycle) }
        )
        XCTAssertNil(unpairedResult.value)
        XCTAssertEqual(unpairedResult.error, lifecycle)
        XCTAssertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(unpairedId))
    }

    private func commandPreparation(_ result: String) -> String? {
        guard let data = result.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }
        return object["blockedReason"] as? String
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

    // MARK: - Collaboration ownership

    func testLocalOnlyMutationRequiresNoTransportOwner() {
        let adapter = makeAdapter()
        XCTAssertNotNil(adapter.insertText("x", atScalar: 0))
        XCTAssertEqual(documentText(adapter), "x")
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
}
