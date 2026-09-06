import XCTest
import UIKit

/// Production v2 adapter tests.
///
/// The production v2-only bindings and XCFramework are linked, so the real
/// v2 engine backs every assertion. The adapter under test
/// (`EditorV2Adapter`) must route every native interaction through the typed
/// `editorV2*` transactions/results — the legacy sentinel-id/JSON editing
/// ABI no longer exists.
final class EditorV2AdapterTests: XCTestCase {


    enum TestHookError: Error {
        case failed
    }

    var adapters: [EditorV2Adapter] = []

    override func tearDown() {
        for adapter in adapters {
            adapter.destroy()
        }
        adapters = []
        super.tearDown()
    }

    func makeAdapter(
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

    func makeAttachedAdapter(
        configJson: String,
        roomBound: Bool,
        destroySession: @escaping (String) -> FfiUnitResult = {
            editorV2Destroy(editorId: $0)
        },
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
                destroySession: destroySession,
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

    func parseObject(_ json: String?, file: StaticString = #filePath, line: UInt = #line) -> [String: Any] {
        guard let json,
              let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            XCTFail("expected JSON object, got: \(json ?? "nil")", file: file, line: line)
            return [:]
        }
        return object
    }

    func mutatedObjectJSON(
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
    func renderedText(_ updateJSON: String?, file: StaticString = #filePath, line: UInt = #line) -> String {
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

    func documentJsonObject(_ adapter: EditorV2Adapter, file: StaticString = #filePath, line: UInt = #line) -> [String: Any] {
        let result = editorV2GetDocumentJson(editorId: adapter.editorId)
        guard let value = result.value, result.error == nil else {
            XCTFail("getDocumentJson failed: \(String(describing: result.error))", file: file, line: line)
            return [:]
        }
        return parseObject(value, file: file, line: line)
    }

    /// All text content of the v2 document, concatenated across text nodes.
    func documentText(_ adapter: EditorV2Adapter, file: StaticString = #filePath, line: UInt = #line) -> String {
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

    func historyState(_ updateJSON: String?, file: StaticString = #filePath, line: UInt = #line) -> [String: Any] {
        let update = parseObject(updateJSON, file: file, line: line)
        guard let history = update["historyState"] as? [String: Any] else {
            XCTFail("update carries no historyState: \(updateJSON ?? "nil")", file: file, line: line)
            return [:]
        }
        return history
    }

    final class ErrorSpy {
        private(set) var errors: [FfiError] = []
        func record(_ error: FfiError) {
            errors.append(error)
            print("V2ERROR domain=\(error.domain) code=\(error.code) message=\(error.message) requestId=\(error.requestId ?? "nil") details=\(error.detailsJson ?? "nil")")
        }
        var last: FfiError? { errors.last }
    }







    func assertMalformedDestroyResultRetainsPairUntilRetry(
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

    func commandPreparation(_ result: String) -> String? {
        guard let data = result.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }
        return object["blockedReason"] as? String
    }






    /// The fixture matrix the Task 15 probe derivation was pinned against:
    /// empty doc, nested lists, marks, emoji, void nodes, multi-block.
    static let accessorFixtures: [(String, String)] = [
        ("empty", ""),
        ("nested-lists", #"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}]}]}"#),
        ("marks", #"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"plain "},{"type":"text","text":"bold","marks":[{"type":"bold"}]},{"type":"text","text":"italiclinked","marks":[{"type":"italic"},{"type":"link","attrs":{"href":"https://example.com"}}]}]}]}"#),
        ("emoji", "{\"type\":\"doc\",\"content\":[{\"type\":\"paragraph\",\"content\":[{\"type\":\"text\",\"text\":\"a\u{1F600}e\u{0301}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}b\"}]}]}"),
        ("void-nodes", #"{"type":"doc","content":[{"type":"image","attrs":{"src":"https://example.com/a.png","alt":null,"title":null,"width":null,"height":null}},{"type":"paragraph","content":[{"type":"text","text":"after"}]}]}"#),
        ("multi-block", #"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]},{"type":"paragraph","content":[{"type":"text","text":"cd"}]}]}"#),
    ]

    func makeFixtureAdapter(_ contentJson: String) -> EditorV2Adapter {
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

}
