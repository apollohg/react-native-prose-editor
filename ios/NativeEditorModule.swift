import ExpoModulesCore

private func nativeUInt64(_ value: Int) -> UInt64? {
    guard value >= 0 else { return nil }
    return UInt64(value)
}

private func nativeUInt32(_ value: Int) -> UInt32? {
    guard value >= 0, value <= Int(UInt32.max) else { return nil }
    return UInt32(value)
}

private func nativeArgumentError(_ field: String) -> String {
    "{\"error\":\"invalid \(field)\"}"
}

private func createdEditorId(_ resultJson: String) -> UInt64? {
    guard let data = resultJson.data(using: .utf8),
          let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          result["error"] == nil,
          let number = result["editorId"] as? NSNumber,
          CFGetTypeID(number) != CFBooleanGetTypeID(),
          number.doubleValue.isFinite,
          number.doubleValue.rounded(.towardZero) == number.doubleValue,
          number.doubleValue > 0,
          number.doubleValue <= Double(UInt64.max)
    else {
        return nil
    }
    return number.uint64Value
}

@discardableResult
private func registerCreatedEditorResult(
    _ resultJson: String,
    markCreated: (UInt64) -> Void = { editorId in
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)
    }
) -> String {
    guard let editorId = createdEditorId(resultJson) else {
        return resultJson
    }
    markCreated(editorId)
    return resultJson
}

public class NativeEditorModule: Module {
    public func definition() -> ModuleDefinition {
        Name("NativeEditor")

        Function("editorCreateResult") { (configJson: String) -> String in
            registerCreatedEditorResult(editorCreateResult(configJson: configJson))
        }
        Function("editorDestroy") { (id: Int) in
            guard let editorId = nativeUInt64(id) else { return }
            NativeEditorViewRegistry.shared.destroy(editorId: editorId) {
                editorDestroy(id: editorId)
            }
        }
        Function("editorPrepareForCommand") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else {
                return nativeArgumentError("editor id")
            }
            return NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        }
        Function("collaborationSessionCreate") { (configJson: String) -> Int in
            Int(collaborationSessionCreate(configJson: configJson))
        }
        Function("collaborationSessionDestroy") { (id: Int) in
            guard let sessionId = nativeUInt64(id) else { return }
            collaborationSessionDestroy(id: sessionId)
        }
        Function("collaborationSessionGetDocumentJson") { (id: Int) -> String in
            guard let sessionId = nativeUInt64(id) else { return "{}" }
            return collaborationSessionGetDocumentJson(id: sessionId)
        }
        Function("collaborationSessionGetEncodedState") { (id: Int) -> String in
            guard let sessionId = nativeUInt64(id) else { return "[]" }
            return collaborationSessionGetEncodedState(id: sessionId)
        }
        Function("collaborationSessionGetPeersJson") { (id: Int) -> String in
            guard let sessionId = nativeUInt64(id) else { return "[]" }
            return collaborationSessionGetPeersJson(id: sessionId)
        }
        Function("collaborationSessionStart") { (id: Int) -> String in
            guard let sessionId = nativeUInt64(id) else { return nativeArgumentError("session id") }
            return collaborationSessionStart(id: sessionId)
        }
        Function("collaborationSessionApplyLocalDocumentJson") { (id: Int, json: String) -> String in
            guard let sessionId = nativeUInt64(id) else { return nativeArgumentError("session id") }
            return collaborationSessionApplyLocalDocumentJson(id: sessionId, json: json)
        }
        Function("collaborationSessionApplyEncodedState") { (id: Int, encodedStateJson: String) -> String in
            guard let sessionId = nativeUInt64(id) else { return nativeArgumentError("session id") }
            return collaborationSessionApplyEncodedState(id: sessionId, encodedStateJson: encodedStateJson)
        }
        Function("collaborationSessionReplaceEncodedState") { (id: Int, encodedStateJson: String) -> String in
            guard let sessionId = nativeUInt64(id) else { return nativeArgumentError("session id") }
            return collaborationSessionReplaceEncodedState(id: sessionId, encodedStateJson: encodedStateJson)
        }
        Function("collaborationSessionHandleMessage") { (id: Int, messageJson: String) -> String in
            guard let sessionId = nativeUInt64(id) else { return nativeArgumentError("session id") }
            return collaborationSessionHandleMessage(id: sessionId, messageJson: messageJson)
        }
        Function("collaborationSessionSetLocalAwareness") { (id: Int, awarenessJson: String) -> String in
            guard let sessionId = nativeUInt64(id) else { return nativeArgumentError("session id") }
            return collaborationSessionSetLocalAwareness(id: sessionId, awarenessJson: awarenessJson)
        }
        Function("collaborationSessionClearLocalAwareness") { (id: Int) -> String in
            guard let sessionId = nativeUInt64(id) else { return nativeArgumentError("session id") }
            return collaborationSessionClearLocalAwareness(id: sessionId)
        }
        Function("editorSetHtml") { (id: Int, html: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorSetHtml(id: editorId, html: html)
        }
        Function("editorGetHtml") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return "" }
            return editorGetHtml(id: editorId)
        }
        Function("editorSetJson") { (id: Int, json: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorSetJson(id: editorId, json: json)
        }
        Function("editorGetJson") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return "{}" }
            return editorGetJson(id: editorId)
        }
        Function("editorGetContentSnapshot") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return "{\"html\":\"\",\"json\":{}}" }
            return editorGetContentSnapshot(id: editorId)
        }
        Function("editorInsertText") { (id: Int, pos: Int, text: String) -> String in
            guard let editorId = nativeUInt64(id),
                  let pos = nativeUInt32(pos)
            else {
                return nativeArgumentError("position")
            }
            return editorInsertText(id: editorId, pos: pos, text: text)
        }
        Function("editorInsertTextScalar") { (id: Int, scalarPos: Int, text: String) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarPos = nativeUInt32(scalarPos)
            else {
                return nativeArgumentError("position")
            }
            return editorInsertTextScalar(id: editorId, scalarPos: scalarPos, text: text)
        }
        Function("editorReplaceSelectionText") { (id: Int, text: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorReplaceSelectionText(id: editorId, text: text)
        }
        Function("editorDeleteRange") { (id: Int, from: Int, to: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let from = nativeUInt32(from),
                  let to = nativeUInt32(to)
            else {
                return nativeArgumentError("position")
            }
            return editorDeleteRange(id: editorId, from: from, to: to)
        }
        Function("editorDeleteScalarRange") { (id: Int, scalarFrom: Int, scalarTo: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarFrom = nativeUInt32(scalarFrom),
                  let scalarTo = nativeUInt32(scalarTo)
            else {
                return nativeArgumentError("position")
            }
            return editorDeleteScalarRange(
                id: editorId,
                scalarFrom: scalarFrom,
                scalarTo: scalarTo
            )
        }
        Function(
            "editorReplaceTextScalar"
        ) { (id: Int, scalarFrom: Int, scalarTo: Int, text: String) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarFrom = nativeUInt32(scalarFrom),
                  let scalarTo = nativeUInt32(scalarTo)
            else {
                return nativeArgumentError("position")
            }
            return editorReplaceTextScalar(
                id: editorId,
                scalarFrom: scalarFrom,
                scalarTo: scalarTo,
                text: text
            )
        }
        Function("editorSplitBlock") { (id: Int, pos: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let pos = nativeUInt32(pos)
            else {
                return nativeArgumentError("position")
            }
            return editorSplitBlock(id: editorId, pos: pos)
        }
        Function("editorSplitBlockScalar") { (id: Int, scalarPos: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarPos = nativeUInt32(scalarPos)
            else {
                return nativeArgumentError("position")
            }
            return editorSplitBlockScalar(id: editorId, scalarPos: scalarPos)
        }
        Function("editorDeleteAndSplitScalar") { (id: Int, scalarFrom: Int, scalarTo: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarFrom = nativeUInt32(scalarFrom),
                  let scalarTo = nativeUInt32(scalarTo)
            else {
                return nativeArgumentError("position")
            }
            return editorDeleteAndSplitScalar(
                id: editorId,
                scalarFrom: scalarFrom,
                scalarTo: scalarTo
            )
        }
        Function("editorInsertContentHtml") { (id: Int, html: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorInsertContentHtml(id: editorId, html: html)
        }
        Function("editorToggleMark") { (id: Int, markName: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorToggleMark(id: editorId, markName: markName)
        }
        Function("editorSetMark") { (id: Int, markName: String, attrsJson: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorSetMark(id: editorId, markName: markName, attrsJson: attrsJson)
        }
        Function("editorUnsetMark") { (id: Int, markName: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorUnsetMark(id: editorId, markName: markName)
        }
        Function("editorToggleBlockquote") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorToggleBlockquote(id: editorId)
        }
        Function("editorToggleCodeBlock") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorToggleCodeBlock(id: editorId)
        }
        Function("editorToggleHeading") { (id: Int, level: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            guard (1...6).contains(level) else {
                return "{\"error\":\"invalid heading level\"}"
            }
            return editorToggleHeading(id: editorId, level: UInt8(level))
        }
        Function("editorSetSelection") { (id: Int, anchor: Int, head: Int) in
            guard let editorId = nativeUInt64(id),
                  let anchor = nativeUInt32(anchor),
                  let head = nativeUInt32(head)
            else {
                return
            }
            editorSetSelection(id: editorId, anchor: anchor, head: head)
        }
        Function("editorSetSelectionScalar") { (id: Int, scalarAnchor: Int, scalarHead: Int) in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return
            }
            editorSetSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead
            )
        }
        Function(
            "editorToggleMarkAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, markName: String) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorToggleMarkAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead,
                markName: markName
            )
        }
        Function(
            "editorSetMarkAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, markName: String, attrsJson: String) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorSetMarkAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead,
                markName: markName,
                attrsJson: attrsJson
            )
        }
        Function(
            "editorUnsetMarkAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, markName: String) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorUnsetMarkAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead,
                markName: markName
            )
        }
        Function(
            "editorToggleBlockquoteAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorToggleBlockquoteAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead
            )
        }
        Function(
            "editorToggleCodeBlockAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorToggleCodeBlockAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead
            )
        }
        Function(
            "editorToggleHeadingAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, level: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            guard (1...6).contains(level) else {
                return "{\"error\":\"invalid heading level\"}"
            }
            return editorToggleHeadingAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead,
                level: UInt8(level)
            )
        }
        Function(
            "editorWrapInListAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, listType: String) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorWrapInListAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead,
                listType: listType
            )
        }
        Function(
            "editorUnwrapFromListAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorUnwrapFromListAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead
            )
        }
        Function(
            "editorIndentListItemAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorIndentListItemAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead
            )
        }
        Function(
            "editorOutdentListItemAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorOutdentListItemAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead
            )
        }
        Function(
            "editorInsertNodeAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, nodeType: String) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorInsertNodeAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead,
                nodeType: nodeType
            )
        }
        Function("editorGetSelection") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else {
                return "{\"type\":\"text\",\"anchor\":0,\"head\":0}"
            }
            return editorGetSelection(id: editorId)
        }
        Function("editorGetSelectionState") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorGetSelectionState(id: editorId)
        }
        Function("editorDocToScalar") { (id: Int, docPos: Int) -> Int in
            guard let editorId = nativeUInt64(id),
                  let docPos = nativeUInt32(docPos)
            else {
                return 0
            }
            return Int(editorDocToScalar(id: editorId, docPos: docPos))
        }
        Function("editorScalarToDoc") { (id: Int, scalar: Int) -> Int in
            guard let editorId = nativeUInt64(id),
                  let scalar = nativeUInt32(scalar)
            else {
                return 0
            }
            return Int(editorScalarToDoc(id: editorId, scalar: scalar))
        }
        Function("editorGetCurrentState") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorGetCurrentState(id: editorId)
        }
        Function("editorUndo") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorUndo(id: editorId)
        }
        Function("editorRedo") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorRedo(id: editorId)
        }
        Function("editorCanUndo") { (id: Int) -> Bool in
            guard let editorId = nativeUInt64(id) else { return false }
            return editorCanUndo(id: editorId)
        }
        Function("editorCanRedo") { (id: Int) -> Bool in
            guard let editorId = nativeUInt64(id) else { return false }
            return editorCanRedo(id: editorId)
        }
        Function("renderDocumentJson") { (configJson: String, json: String) -> String in
            let creation = editorCreateResult(configJson: configJson)
            guard let editorId = createdEditorId(creation) else { return creation }
            defer {
                editorDestroy(id: editorId)
            }
            return editorSetJson(id: editorId, json: json)
        }
        Function("measureContentHeight") { (renderJson: String, themeJson: String?, width: Double) -> Double in
            let height = RenderBridge.measureHeight(
                forRenderJSON: renderJson,
                themeJSON: themeJson,
                width: CGFloat(width)
            )
            return Double(height)
        }
        Function("renderDocumentHtml") { (configJson: String, html: String) -> String in
            let creation = editorCreateResult(configJson: configJson)
            guard let editorId = createdEditorId(creation) else { return creation }
            defer {
                editorDestroy(id: editorId)
            }
            return editorSetHtml(id: editorId, html: html)
        }
        Function("editorReplaceHtml") { (id: Int, html: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorReplaceHtml(id: editorId, html: html)
        }
        Function("editorReplaceJson") { (id: Int, json: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorReplaceJson(id: editorId, json: json)
        }
        Function("editorInsertContentJson") { (id: Int, json: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorInsertContentJson(id: editorId, json: json)
        }
        Function(
            "editorInsertContentJsonAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, json: String) -> String in
            guard let editorId = nativeUInt64(id),
                  let scalarAnchor = nativeUInt32(scalarAnchor),
                  let scalarHead = nativeUInt32(scalarHead)
            else {
                return nativeArgumentError("position")
            }
            return editorInsertContentJsonAtSelectionScalar(
                id: editorId,
                scalarAnchor: scalarAnchor,
                scalarHead: scalarHead,
                json: json
            )
        }
        Function("editorWrapInList") { (id: Int, listType: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorWrapInList(id: editorId, listType: listType)
        }
        Function("editorUnwrapFromList") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorUnwrapFromList(id: editorId)
        }
        Function("editorIndentListItem") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorIndentListItem(id: editorId)
        }
        Function("editorOutdentListItem") { (id: Int) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorOutdentListItem(id: editorId)
        }
        Function("editorInsertNode") { (id: Int, nodeType: String) -> String in
            guard let editorId = nativeUInt64(id) else { return nativeArgumentError("editor id") }
            return editorInsertNode(id: editorId, nodeType: nodeType)
        }

        View(NativeEditorExpoView.self) {
            Events(
                "onEditorUpdate",
                "onSelectionChange",
                "onFocusChange",
                "onContentHeightChange",
                "onToolbarAction",
                "onAddonEvent"
            )

            Prop("editorId") { (view: NativeEditorExpoView, id: Int) in
                view.setEditorId(nativeUInt64(id) ?? 0)
            }
            Prop("editable") { (view: NativeEditorExpoView, editable: Bool) in
                view.setEditable(editable)
            }
            Prop("placeholder") { (view: NativeEditorExpoView, placeholder: String) in
                view.richTextView.textView.placeholder = placeholder
            }
            Prop("autoFocus") { (view: NativeEditorExpoView, autoFocus: Bool) in
                view.setAutoFocus(autoFocus)
            }
            Prop("autoCapitalize") { (view: NativeEditorExpoView, autoCapitalize: String?) in
                view.setAutoCapitalize(autoCapitalize)
            }
            Prop("autoCorrect") { (view: NativeEditorExpoView, autoCorrect: Bool?) in
                view.setAutoCorrect(autoCorrect)
            }
            Prop("keyboardType") { (view: NativeEditorExpoView, keyboardType: String?) in
                view.setKeyboardType(keyboardType)
            }
            Prop("showToolbar") { (view: NativeEditorExpoView, showToolbar: Bool) in
                view.setShowToolbar(showToolbar)
            }
            Prop("toolbarPlacement") { (view: NativeEditorExpoView, toolbarPlacement: String?) in
                view.setToolbarPlacement(toolbarPlacement)
            }
            Prop("heightBehavior") { (view: NativeEditorExpoView, heightBehavior: String) in
                view.setHeightBehavior(heightBehavior)
            }
            Prop("allowImageResizing") { (view: NativeEditorExpoView, allowImageResizing: Bool) in
                view.setAllowImageResizing(allowImageResizing)
            }
            Prop("imageLoadingPolicyJson") { (view: NativeEditorExpoView, json: String?) in
                view.setImageLoadingPolicyJson(json)
            }
            Prop("themeJson") { (view: NativeEditorExpoView, themeJson: String?) in
                view.setThemeJson(themeJson)
            }
            Prop("addonsJson") { (view: NativeEditorExpoView, addonsJson: String?) in
                view.setAddonsJson(addonsJson)
            }
            Prop("remoteSelectionsJson") { (view: NativeEditorExpoView, remoteSelectionsJson: String?) in
                view.setRemoteSelectionsJson(remoteSelectionsJson)
            }
            Prop("toolbarItemsJson") { (view: NativeEditorExpoView, toolbarItemsJson: String?) in
                view.setToolbarButtonsJson(toolbarItemsJson)
            }
            Prop("toolbarFrameJson") { (view: NativeEditorExpoView, toolbarFrameJson: String?) in
                view.setToolbarFrameJson(toolbarFrameJson)
            }
            Prop("editorUpdateJson") { (view: NativeEditorExpoView, editorUpdateJson: String?) in
                view.setPendingEditorUpdateJson(editorUpdateJson)
            }
            Prop("editorUpdateRevision") { (view: NativeEditorExpoView, editorUpdateRevision: Int) in
                view.setPendingEditorUpdateRevision(editorUpdateRevision)
            }
            OnViewDidUpdateProps { (view: NativeEditorExpoView) in
                view.applyPendingEditorUpdateIfNeeded()
            }

            AsyncFunction("applyEditorUpdate") { (view: NativeEditorExpoView, updateJson: String) -> Bool in
                view.applyEditorUpdate(updateJson)
            }
            AsyncFunction("focus") { (view: NativeEditorExpoView) in
                view.focus()
            }
            AsyncFunction("blur") { (view: NativeEditorExpoView) in
                view.blur()
            }
            AsyncFunction("getCaretRect") { (view: NativeEditorExpoView) -> String? in
                view.getCaretRectJson()
            }
        }

        View(NativeProseViewerExpoView.self) {
            ViewName("NativeProseViewer")
            Events("onContentHeightChange", "onPressLink", "onPressMention")

            Prop("renderJson") { (view: NativeProseViewerExpoView, renderJson: String?) in
                view.setRenderJson(renderJson)
            }
            Prop("themeJson") { (view: NativeProseViewerExpoView, themeJson: String?) in
                view.setThemeJson(themeJson)
            }
            Prop("imageLoadingPolicyJson") { (view: NativeProseViewerExpoView, json: String?) in
                view.setImageLoadingPolicyJson(json)
            }
            Prop("collapsesWhenEmpty") {
                (view: NativeProseViewerExpoView, collapsesWhenEmpty: Bool?) in
                view.setCollapsesWhenEmpty(collapsesWhenEmpty)
            }
            Prop("enableLinkTaps") { (view: NativeProseViewerExpoView, enableLinkTaps: Bool?) in
                view.setEnableLinkTaps(enableLinkTaps)
            }
            Prop("interceptLinkTaps") { (view: NativeProseViewerExpoView, interceptLinkTaps: Bool?) in
                view.setInterceptLinkTaps(interceptLinkTaps)
            }
        }
    }
}
