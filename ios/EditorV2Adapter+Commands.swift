import Foundation

extension EditorV2Adapter {
    func toggleMark(_ markType: String, anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(["type": "toggleMark", "markType": markType], anchor: anchor, head: head)
    }

    func setMark(_ markType: String, attrsJson: String, anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard let data = attrsJson.data(using: .utf8),
              let attrs = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            emit(contractError("setMark attrs are not valid JSON"))
            return nil
        }
        return commandAtSelection(
            ["type": "setMark", "markType": markType, "attrs": attrs],
            anchor: anchor,
            head: head
        )
    }

    func unsetMark(_ markType: String, anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(["type": "unsetMark", "markType": markType], anchor: anchor, head: head)
    }

    func toggleHeading(level: UInt8, anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(["type": "toggleHeading", "level": Int(level)], anchor: anchor, head: head)
    }

    func toggleCodeBlock(anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(["type": "toggleCodeBlock"], anchor: anchor, head: head)
    }

    func toggleBlockquote(anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(["type": "toggleBlockquote"], anchor: anchor, head: head)
    }

    func wrapInList(listType: String, itemType: String, anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(
            ["type": "wrapInList", "listType": listType, "itemType": itemType],
            anchor: anchor,
            head: head
        )
    }

    func unwrapFromList(anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(["type": "unwrapFromList"], anchor: anchor, head: head)
    }

    func indentListItem(anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(["type": "indentListItem"], anchor: anchor, head: head)
    }

    func outdentListItem(anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(["type": "outdentListItem"], anchor: anchor, head: head)
    }

    func toggleTaskItemChecked(anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(["type": "toggleTaskItemChecked"], anchor: anchor, head: head)
    }

    func moveSelection(anchor: UInt32, head: UInt32, to destination: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(
            [
                "type": "moveSelection",
                "range": [
                    "from": EditorV2PositionBridge.positionEnvelope(scalar: min(anchor, head)),
                    "to": EditorV2PositionBridge.positionEnvelope(scalar: max(anchor, head)),
                ],
                "at": EditorV2PositionBridge.positionEnvelope(scalar: destination),
            ],
            anchor: anchor,
            head: head
        )
    }

    func commandAtSelection(_ command: [String: Any], anchor: UInt32, head: UInt32) -> String? {
        if nativeOwnerId != nil {
            var intent = nativeIntent("command", anchor: anchor, head: head)
            intent["command"] = command
            return performNativeIntent(intent)?.updateJSON
        }
        return performMutation(preSelection: (anchor, head), adoptEngineSelection: true) {
            self.callWithEnvelope(["command": command]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func resizeImage(atDocPos docPos: UInt32, width: UInt32, height: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard let scalar = scalarPosition(forDoc: docPos) else { return nil }
        return performMutation {
            self.callWithEnvelope([
                    "command": [
                        "type": "resizeImage",
                        "at": EditorV2PositionBridge.positionEnvelope(scalar: scalar),
                        "width": Int(width),
                        "height": Int(height),
                    ] as [String: Any],
            ]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func undo() -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return performHistoryMutation { requestJson in
            editorV2Undo(editorId: self.editorId, requestJson: requestJson)
        }
    }

    func redo() -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return performHistoryMutation { requestJson in
            editorV2Redo(editorId: self.editorId, requestJson: requestJson)
        }
    }

    func setContentHtml(_ html: String) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return performMutation(postSelectionMirror: (0, 0), includeSelectionInUpdate: true) {
            self.callWithEnvelope(["setHtml": html, "history": "resetAndClear"]) { requestJson in
                editorV2ApplyLocalApi(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func setContentJson(_ json: String) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard let data = json.data(using: .utf8),
              let document = try? JSONSerialization.jsonObject(with: data)
        else {
            emit(contractError("setContentJson document is not valid JSON"))
            return nil
        }
        return performMutation(postSelectionMirror: (0, 0), includeSelectionInUpdate: true) {
            self.callWithEnvelope(["setJson": document, "history": "resetAndClear"]) { requestJson in
                editorV2ApplyLocalApi(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    /// Undoable whole-document replace (legacy `editorReplaceHtml` parity:
    /// one undoable local-API boundary, selection preserved where possible).
    func replaceContentHtml(_ html: String) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return performMutation(postSelectionMirror: (0, 0), includeSelectionInUpdate: true) {
            self.callWithEnvelope(["setHtml": html, "history": "undoableBoundary"]) { requestJson in
                editorV2ApplyLocalApi(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    /// Undoable whole-document replace from JSON (legacy `editorReplaceJson`
    /// parity).
    func replaceContentJson(_ json: String) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard let data = json.data(using: .utf8),
              let document = try? JSONSerialization.jsonObject(with: data)
        else {
            emit(contractError("replaceContentJson document is not valid JSON"))
            return nil
        }
        return performMutation(postSelectionMirror: (0, 0), includeSelectionInUpdate: true) {
            self.callWithEnvelope(["setJson": document, "history": "undoableBoundary"]) { requestJson in
                editorV2ApplyLocalApi(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

}
