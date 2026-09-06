import XCTest
import UIKit

extension EditorV2AdapterTests {
    func testRevisionMismatchRefusesSelectionRelativeInputWithoutReplay() {
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

        let callsBefore = adapter.backendEnvelopeCallCountForTesting
        let update = adapter.insertText("REBASED", atScalar: 0)
        XCTAssertNotNil(update)
        XCTAssertEqual(documentText(adapter), "EXTbase")
        XCTAssertEqual(renderedText(update), "EXTbase")
        XCTAssertEqual(adapter.backendEnvelopeCallCountForTesting, callsBefore + 1)

        let recovered = adapter.insertText("ok", atScalar: 0)
        XCTAssertEqual(renderedText(recovered), "okEXTbase")
        XCTAssertEqual(documentText(adapter), "okEXTbase")
    }

    func testPreSyncMismatchRefusesSelectionRelativeInputWithoutReplay() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>base</p>")
        _ = adapter.syncSelection(anchor: 0, head: 0)

        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"990002","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"EXT"}}"#
        )
        XCTAssertNil(external.error)
        let callsBefore = adapter.backendEnvelopeCallCountForTesting

        let update = adapter.insertText("X", atScalar: 2)

        XCTAssertEqual(documentText(adapter), "EXTbase")
        XCTAssertEqual(renderedText(update), "EXTbase")
        XCTAssertEqual(adapter.backendEnvelopeCallCountForTesting, callsBefore + 1)
    }

    func testPreSyncMismatchRefusesDeleteBackwardWithoutReplay() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>base</p>")
        _ = adapter.syncSelection(anchor: 4, head: 4)
        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"990003","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"R"}}"#
        )
        XCTAssertNil(external.error)

        let update = adapter.deleteBackward(anchor: 2, head: 2)

        XCTAssertNotNil(update)
        XCTAssertEqual(documentText(adapter), "baseR")
    }

    func testMismatchRefreshDoesNotInvokeCompatibilityRecovery() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>base</p>")
        _ = adapter.syncSelection(anchor: 0, head: 0)
        let first = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"990004","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"EXT"}}"#
        )
        XCTAssertNil(first.error)
        var recovered = false
        adapter.onRemoteRecoveryForTesting = { recovered = true }
        let callsBefore = adapter.backendEnvelopeCallCountForTesting

        let update = adapter.insertText("X", atScalar: 2)

        XCTAssertNotNil(update)
        XCTAssertEqual(documentText(adapter), "EXTbase")
        XCTAssertEqual(adapter.backendEnvelopeCallCountForTesting, callsBefore + 1)
        XCTAssertFalse(recovered)
    }

    func testRevisionMismatchNeverReplaysExplicitlyPositionedMutation() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>base</p>")
        let external = editorV2ApplyCommand(
            editorId: adapter.editorId,
            requestJson: #"{"version":1,"requestId":"990006","baseDocumentRevision":"\#(adapter.baseDocumentRevision)","command":{"type":"insertText","text":"EXT"}}"#
        )
        XCTAssertNil(external.error)
        let callsBefore = adapter.backendEnvelopeCallCountForTesting

        let update = adapter.deleteScalarRange(from: 0, to: 4)

        XCTAssertNotNil(update)
        XCTAssertEqual(documentText(adapter), "EXTbase")
        XCTAssertEqual(adapter.backendEnvelopeCallCountForTesting, callsBefore + 1)
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
            let renderBlocks = object["renderBlocks"]!
            object["renderBlocks"] = NSNull()
            object["renderPatch"] = [
                "baseDocumentVersion": object["documentVersion"]!,
                "startIndex": 0,
                "deleteCount": 0,
                "renderBlocks": renderBlocks,
            ]
        }

        XCTAssertNotNil(adapter.adoptExternalRender(withPatch))
    }

    /// Rust emits `attrs` on every void/opaque element, so an inserted mention
    /// must survive external-render validation on its way back to the view.
    func testAtomicRenderValidationAcceptsAnInsertedMentionCarryingNodeAttrs() {
        let adapter = makeAdapter()
        _ = adapter.setContentHtml("<p>base</p>")
        let raw = editorV2RenderUpdate(
            editorId: adapter.editorId,
            mirrorScalarAnchor: nil,
            mirrorScalarHead: nil
        ).value!
        let withMention = mutatedObjectJSON(raw) { object in
            object["renderBlocks"] = [[
                ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
                [
                    "type": "opaqueInlineAtom",
                    "nodeType": "mention",
                    "label": "@Alice Chen",
                    "docPos": 1,
                    "attrs": [
                        "id": "user-alice",
                        "label": "Alice Chen",
                        "mentionSuggestionChar": "@",
                        "type": "user",
                    ],
                    "mentionTheme": ["node": ["textColor": "#336EC1"]],
                ],
                ["type": "blockEnd"],
            ]]
        }

        XCTAssertNotNil(adapter.adoptExternalRender(withMention))
    }

    func testAtomicRenderValidationAcceptsAtomIdOnlyOnVoidBlock() {
        func adopt(_ element: [String: Any]) -> String? {
            let adapter = makeAdapter()
            _ = adapter.setContentHtml("<p>base</p>")
            let raw = editorV2RenderUpdate(
                editorId: adapter.editorId,
                mirrorScalarAnchor: nil,
                mirrorScalarHead: nil
            ).value!
            let snapshot = mutatedObjectJSON(raw) { object in
                object["renderBlocks"] = [[element]]
            }
            return adapter.adoptExternalRender(snapshot)
        }

        XCTAssertNotNil(adopt([
            "type": "voidBlock",
            "nodeType": "counterCard",
            "docPos": 1,
            "atomId": "y1-2",
        ]))
        XCTAssertNil(adopt([
            "type": "voidBlock",
            "nodeType": "counterCard",
            "docPos": 1,
            "atomId": 7,
        ]))
        XCTAssertNil(adopt([
            "type": "voidInline",
            "nodeType": "hardBreak",
            "docPos": 1,
            "atomId": "y1-2",
        ]))
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
                    "mentionTheme": ["node": ["borderWidth": true]],
                ]]
            }),
            ("opaque block extra field", {
                $0["renderBlocks"] = [[
                    "type": "opaqueBlockAtom",
                    "nodeType": "unknown",
                    "label": "Unknown",
                    "docPos": 0,
                    "legacy": true,
                ]]
            }),
            ("opaque inline array attrs", {
                $0["renderBlocks"] = [[
                    "type": "opaqueInlineAtom",
                    "nodeType": "mention",
                    "label": "Ada",
                    "docPos": 0,
                    "attrs": [],
                ]]
            }),
            ("render patch invalid nested element", { object in
                object["renderPatch"] = [
                    "baseDocumentVersion": object["documentVersion"]!,
                    "startIndex": 0,
                    "deleteCount": 0,
                    "renderBlocks": [["type": "unknownElement"]],
                ]
            }),
            ("render patch fractional start", { object in
                object["renderPatch"] = [
                    "baseDocumentVersion": object["documentVersion"]!,
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

}
