import ExpoModulesCore

private func nativeUInt64(_ value: Int) -> UInt64? {
    guard value >= 0 else { return nil }
    return UInt64(value)
}

func createdEditorId(_ resultJson: String) -> UInt64? {
    guard let data = resultJson.data(using: .utf8),
          let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          result["error"] == nil,
          result["editorId"] != nil,
          let expression = try? NSRegularExpression(
              pattern: #"^\s*\{\s*"editorId"\s*:\s*(0|[1-9][0-9]*)\s*\}\s*$"#
          )
    else {
        return nil
    }
    let range = NSRange(resultJson.startIndex..., in: resultJson)
    let matches = expression.matches(in: resultJson, range: range)
    guard matches.count == 1,
          let valueRange = Range(matches[0].range(at: 1), in: resultJson),
          let editorId = UInt64(resultJson[valueRange]),
          editorId > 0,
          editorId <= UInt64(Int64.max)
    else {
        return nil
    }
    return editorId
}


/// Serialize a structured v2 failure into the legacy `{"error":...}` envelope
/// shape the JS boundary already understands.
private func v2ErrorJson(_ error: FfiError) -> String {
    let object: [String: Any] = [
        "error": [
            "domain": error.domain,
            "code": error.code,
            "message": error.message,
        ]
    ]
    guard let data = try? JSONSerialization.data(withJSONObject: object),
          let json = String(data: data, encoding: .utf8)
    else {
        return "{\"error\":{\"code\":\"\(error.code)\"}}"
    }
    return json
}

// MARK: - v2 result-record bridging (frozen {value, error} contract)

/// One FfiError as the plain dictionary the TS boundary normalizes
/// (`normalizeNativeEditorV2Error`): required domain/code/message plus the
/// optional fields only when present.
private func v2FfiErrorDictionary(_ error: FfiError) -> [String: Any] {
    var dictionary: [String: Any] = [
        "domain": error.domain,
        "code": error.code,
        "message": error.message,
    ]
    if let requestId = error.requestId { dictionary["requestId"] = requestId }
    if let operationIndex = error.operationIndex {
        dictionary["operationIndex"] = NSNumber(value: operationIndex)
    }
    if let limit = error.limit { dictionary["limit"] = NSNumber(value: limit) }
    if let actual = error.actual { dictionary["actual"] = NSNumber(value: actual) }
    if let detailsJson = error.detailsJson { dictionary["detailsJson"] = detailsJson }
    return dictionary
}

/// A boundary-fabricated error record for arguments that cannot reach Rust
/// (non-canonical generation strings and the like).
private func v2ContractErrorDictionary(_ message: String) -> [String: Any] {
    [
        "domain": "boundary",
        "code": "FFI_RESULT_INVALID",
        "message": message,
    ]
}

private func v2InvalidResultDictionary(_ message: String) -> [String: Any] {
    ["error": v2ContractErrorDictionary(message)]
}

private func v2JsonResultDictionary(_ result: FfiJsonResult) -> [String: Any] {
    if let value = result.value { return ["value": value] }
    if let error = result.error { return ["error": v2FfiErrorDictionary(error)] }
    return v2InvalidResultDictionary("v2 result carries neither value nor error")
}

private func v2BytesResultDictionary(_ result: FfiBytesResult) -> [String: Any] {
    if let value = result.value { return ["value": value] }
    if let error = result.error { return ["error": v2FfiErrorDictionary(error)] }
    return v2InvalidResultDictionary("v2 result carries neither value nor error")
}

private func v2UnitResultDictionary(_ result: FfiUnitResult) -> [String: Any] {
    if let value = result.value { return ["value": value] }
    if let error = result.error { return ["error": v2FfiErrorDictionary(error)] }
    return v2InvalidResultDictionary("v2 result carries neither value nor error")
}

private func v2SnapshotExportResultDictionary(_ result: FfiSnapshotExportResult) -> [String: Any] {
    if let value = result.value {
        return [
            "value": [
                "metadataJson": value.metadataJson,
                "encodedState": value.encodedState,
            ]
        ]
    }
    if let error = result.error { return ["error": v2FfiErrorDictionary(error)] }
    return v2InvalidResultDictionary("v2 result carries neither value nor error")
}

/// Decimal-string u64 (generations cross the JS boundary as strings).
private func v2UInt64Argument(_ raw: String) -> UInt64? {
    UInt64(raw)
}

/// Parse the created v2 session's decimal-string editor id from the frozen
/// create value (`{"editorId":"<decimal>"}` — the v2 handle is a string,
/// unlike the legacy numeric create shape `createdEditorId` matches).
private func createdV2SessionEditorId(_ resultJson: String) -> UInt64? {
    guard let data = resultJson.data(using: .utf8),
          let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let editorIdString = result["editorId"] as? String,
          let editorId = UInt64(editorIdString),
          editorId > 0,
          editorId <= UInt64(Int64.max)
    else {
        return nil
    }
    return editorId
}

/// Whether the JS-facing create config initializes a room-bound session
/// (the paired adapter mirrors the binding for its collaboration gating).
private func v2ConfigIndicatesRoomBinding(_ configJson: String) -> Bool {
    guard let data = configJson.data(using: .utf8),
          let config = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let initialization = config["initialization"] as? [String: Any]
    else {
        return false
    }
    return initialization["type"] as? String == "room"
}

public class NativeEditorModule: Module {
    public func definition() -> ModuleDefinition {
        Name("NativeEditor")

        // MARK: v2 UniFFI surface (production ABI)
        //
        // The JS document handle drives the engine directly through these
        // passthroughs; every call returns the frozen {value, error} result
        // record. Decimal-string handles/generations keep full u64 fidelity
        // across the JS boundary; binaries travel as Data.

        Function("editorV2Create") { (configJson: String, snapshotState: Data?) -> [String: Any] in
            let result = editorV2Create(configJson: configJson, snapshotState: snapshotState)
            // Pair the view-facing adapter with the JS-created session and
            // mark the public id live so views may bind to it: the Expo view
            // receives the handle's editorId and routes every interaction
            // through the shared session.
            if let value = result.value, let editorId = createdV2SessionEditorId(value) {
                EditorV2Registry.register(
                    EditorV2Adapter.attach(
                        editorId: String(editorId),
                        roomBound: v2ConfigIndicatesRoomBinding(configJson)
                    ),
                    forLegacyId: editorId
                )
                NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)
            }
            return v2JsonResultDictionary(result)
        }
        Function("editorV2Destroy") { (editorId: String) -> [String: Any] in
            let result = editorV2Destroy(editorId: editorId)
            if let signedId = UInt64(editorId), signedId > 0, signedId <= UInt64(Int64.max) {
                NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: signedId)
                EditorV2Registry.destroyPair(forLegacyId: signedId)
            }
            return v2UnitResultDictionary(result)
        }
        Function("editorV2GetState") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2GetState(editorId: editorId))
        }
        Function("editorV2GetDocumentJson") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2GetDocumentJson(editorId: editorId))
        }
        Function("editorV2GetDocumentHtml") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2GetDocumentHtml(editorId: editorId))
        }
        Function("editorV2GetContentSnapshot") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2GetContentSnapshot(editorId: editorId))
        }
        Function("editorV2ReplaceDocument") { (editorId: String, requestJson: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2ReplaceDocument(editorId: editorId, requestJson: requestJson))
        }
        Function("editorV2ApplyInput") { (editorId: String, requestJson: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2ApplyInput(editorId: editorId, requestJson: requestJson))
        }
        Function("editorV2ApplyCommand") { (editorId: String, requestJson: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2ApplyCommand(editorId: editorId, requestJson: requestJson))
        }
        Function("editorV2ApplyLocalApi") { (editorId: String, requestJson: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2ApplyLocalApi(editorId: editorId, requestJson: requestJson))
        }
        Function("editorV2SetSelection") { (editorId: String, requestJson: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2SetSelection(editorId: editorId, requestJson: requestJson))
        }
        Function("editorV2Undo") { (editorId: String, requestJson: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2Undo(editorId: editorId, requestJson: requestJson))
        }
        Function("editorV2Redo") { (editorId: String, requestJson: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2Redo(editorId: editorId, requestJson: requestJson))
        }
        Function("editorV2RenderUpdate") { (editorId: String, mirrorScalarAnchor: Int?, mirrorScalarHead: Int?) -> [String: Any] in
            // The render accessor for the interactive component: after a
            // JS-driven engine change the component fetches the current
            // render update here and pushes it to the bound view.
            let anchor = mirrorScalarAnchor.flatMap { $0 >= 0 ? UInt32($0) : nil }
            let head = mirrorScalarHead.flatMap { $0 >= 0 ? UInt32($0) : nil }
            return v2JsonResultDictionary(
                editorV2RenderUpdate(
                    editorId: editorId,
                    mirrorScalarAnchor: anchor,
                    mirrorScalarHead: head
                )
            )
        }
        Function("editorV2CollaborationBeginConnect") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2CollaborationBeginConnect(editorId: editorId))
        }
        Function("editorV2CollaborationSocketOpen") { (editorId: String, generation: String) -> [String: Any] in
            guard let generation = v2UInt64Argument(generation) else {
                return v2InvalidResultDictionary("invalid generation")
            }
            return v2BytesResultDictionary(
                editorV2CollaborationSocketOpen(editorId: editorId, generation: generation)
            )
        }
        Function("editorV2CollaborationReceive") { (editorId: String, generation: String, message: Data) -> [String: Any] in
            guard let generation = v2UInt64Argument(generation) else {
                return v2InvalidResultDictionary("invalid generation")
            }
            return v2JsonResultDictionary(
                editorV2CollaborationReceive(editorId: editorId, generation: generation, message: message)
            )
        }
        Function("editorV2CollaborationSocketClose") { (editorId: String, generation: String, code: Int?, reason: String?) -> [String: Any] in
            guard let generation = v2UInt64Argument(generation) else {
                return v2InvalidResultDictionary("invalid generation")
            }
            let closeCode = code.flatMap { $0 >= 0 ? UInt32($0) : nil }
            return v2JsonResultDictionary(
                editorV2CollaborationSocketClose(
                    editorId: editorId,
                    generation: generation,
                    code: closeCode,
                    reason: reason
                )
            )
        }
        Function("editorV2CollaborationTakeOutbound") { (editorId: String, generation: String) -> [String: Any] in
            guard let generation = v2UInt64Argument(generation) else {
                return v2InvalidResultDictionary("invalid generation")
            }
            return v2BytesResultDictionary(
                editorV2CollaborationTakeOutbound(editorId: editorId, generation: generation)
            )
        }
        Function("editorV2CollaborationSetAwareness") { (editorId: String, awarenessJson: String) -> [String: Any] in
            v2UnitResultDictionary(
                editorV2CollaborationSetAwareness(editorId: editorId, awarenessJson: awarenessJson)
            )
        }
        Function("editorV2CollaborationPeers") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(editorV2CollaborationPeers(editorId: editorId))
        }
        Function("editorV2SnapshotExport") { (editorId: String) -> [String: Any] in
            v2SnapshotExportResultDictionary(editorV2SnapshotExport(editorId: editorId))
        }
        Function("editorV2SnapshotRestore") { (editorId: String, metadataJson: String, encodedState: Data) -> [String: Any] in
            v2JsonResultDictionary(
                editorV2SnapshotRestore(editorId: editorId, metadataJson: metadataJson, encodedState: encodedState)
            )
        }

        Function("renderDocumentJson") { (configJson: String, json: String) -> String in
            // Stateless render probe: a transient v2 session renders the
            // document and is destroyed immediately (NativeProseViewer).
            switch EditorV2Adapter.create(legacyConfigJson: configJson) {
            case .failure(let error):
                return v2ErrorJson(error)
            case .success(let adapter):
                defer { adapter.destroy() }
                return adapter.setContentJson(json) ?? "{}"
            }
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
            switch EditorV2Adapter.create(legacyConfigJson: configJson) {
            case .failure(let error):
                return v2ErrorJson(error)
            case .success(let adapter):
                defer { adapter.destroy() }
                return adapter.setContentHtml(html) ?? "{}"
            }
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
            Prop("accessibilityLabel") { (view: NativeEditorExpoView, label: String?) in
                view.setAccessibilityLabel(label)
            }
            Prop("accessibilityHint") { (view: NativeEditorExpoView, hint: String?) in
                view.setAccessibilityHint(hint)
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
