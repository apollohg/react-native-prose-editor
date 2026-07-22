import ExpoModulesCore

/// Test-facing parser for the v2 create value. Handles are never bridged
/// through signed native numbers: the frozen wire shape is a canonical
/// decimal string.
func createdEditorId(_ resultJson: String) -> String? {
    guard let data = resultJson.data(using: .utf8),
          let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          result["error"] == nil,
          let editorId = result["editorId"] as? String,
          let value = v2UInt64Argument(editorId),
          value > 0,
          editorId == String(value)
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
    if let operationIndex = error.operationIndex { dictionary["operationIndex"] = operationIndex }
    if let limit = error.limit { dictionary["limit"] = limit }
    if let actual = error.actual { dictionary["actual"] = actual }
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

/// Canonical decimal u64 values are the only v2 wire representation.
func v2CanonicalUInt64String(_ raw: String) -> String? {
    guard !raw.isEmpty,
          raw.allSatisfy({ $0 >= "0" && $0 <= "9" }),
          raw == "0" || raw.first != "0",
          UInt64(raw) != nil
    else {
        return nil
    }
    return raw
}

/// Native-only conversion after canonical syntax and range verification.
private func v2UInt64Argument(_ raw: String) -> UInt64? {
    guard let canonical = v2CanonicalUInt64String(raw) else { return nil }
    return UInt64(canonical)
}

/// NSNumber is the Expo/Foundation numeric boundary. Do not use `uint32Value`:
/// it truncates fractions and wraps values outside the u32 domain.
func v2ExactUInt32(_ raw: NSNumber?) -> UInt32? {
    guard let raw,
          CFGetTypeID(raw) != CFBooleanGetTypeID()
    else {
        return nil
    }
    let value = raw.doubleValue
    guard value.isFinite,
          value >= 0,
          value.rounded(.towardZero) == value,
          value <= Double(UInt32.max)
    else {
        return nil
    }
    return UInt32(exactly: value)
}

/// A created v2 handle preserves its exact engine-issued string for adapter
/// binding while retaining the current numeric native-view registry id.
private struct CreatedV2SessionHandle {
    let handle: String
    let nativeViewId: UInt64
}

/// Parse the created v2 session's canonical decimal handle from the frozen
/// create value (`{"editorId":"<decimal>"}`). The adapter must receive the
/// exact engine-issued string rather than a numeric reconstruction.
private func createdV2SessionHandle(_ resultJson: String) -> CreatedV2SessionHandle? {
    guard let data = resultJson.data(using: .utf8),
          let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let editorIdString = result["editorId"] as? String,
          let editorId = v2UInt64Argument(editorIdString),
          editorId > 0,
          editorIdString == String(editorId)
    else {
        return nil
    }
    return CreatedV2SessionHandle(handle: editorIdString, nativeViewId: editorId)
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

/// Stateless viewer rendering still needs a transient engine session, but it
/// receives the complete v2 create envelope from TypeScript. Pairing only
/// attaches to that existing session; it never recreates or patches config.
private func renderDocumentProbe(
    configJson: String,
    apply: (EditorV2Adapter) -> String?
) -> String {
    let created = editorV2Create(configJson: configJson, snapshotState: nil)
    if let error = created.error { return v2ErrorJson(error) }
    guard let value = created.value,
          let createdHandle = createdV2SessionHandle(value)
    else {
        return v2ErrorJson(
            FfiError(
                domain: "boundary",
                code: "FFI_RESULT_INVALID",
                message: "v2 render probe could not bind its created editor",
                requestId: nil,
                operationIndex: nil,
                limit: nil,
                actual: nil,
                detailsJson: nil
            )
        )
    }
    guard let adapter = EditorV2Adapter.attach(editorId: createdHandle.handle, roomBound: false) else {
        _ = editorV2Destroy(editorId: createdHandle.handle)
        return v2ErrorJson(
            FfiError(
                domain: "boundary",
                code: "FFI_RESULT_INVALID",
                message: "v2 render probe could not bind its created editor",
                requestId: nil,
                operationIndex: nil,
                limit: nil,
                actual: nil,
                detailsJson: nil
            )
        )
    }
    defer { _ = adapter.destroy() }
    return apply(adapter) ?? v2ErrorJson(
        FfiError(
            domain: "boundary",
            code: "FFI_RESULT_INVALID",
            message: "v2 render probe could not apply content",
            requestId: nil,
            operationIndex: nil,
            limit: nil,
            actual: nil,
            detailsJson: nil
        )
    )
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
            if let value = result.value {
                guard let createdHandle = createdV2SessionHandle(value) else {
                    return v2InvalidResultDictionary("v2 create value carries no canonical editor id")
                }
                guard let adapter = EditorV2Adapter.attach(
                    editorId: createdHandle.handle,
                    roomBound: v2ConfigIndicatesRoomBinding(configJson)
                ) else {
                    _ = editorV2Destroy(editorId: createdHandle.handle)
                    return v2InvalidResultDictionary("v2 create could not bind its editor handle")
                }
                EditorV2Registry.register(adapter, forLegacyId: createdHandle.nativeViewId)
                NativeEditorViewRegistry.shared.markEditorCreated(editorId: createdHandle.nativeViewId)
            }
            return v2JsonResultDictionary(result)
        }
        Function("editorV2Destroy") { (editorId: String) -> [String: Any] in
            let result = editorV2Destroy(editorId: editorId)
            if let nativeViewId = v2UInt64Argument(editorId), nativeViewId > 0 {
                NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: nativeViewId)
                EditorV2Registry.destroyPair(forLegacyId: nativeViewId)
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
        Function("editorV2RenderUpdate") { (editorId: String, mirrorScalarAnchor: Double?, mirrorScalarHead: Double?) -> [String: Any] in
            // The render accessor for the interactive component: after a
            // JS-driven engine change the component fetches the current
            // render update here and pushes it to the bound view.
            let anchor = mirrorScalarAnchor.flatMap { v2ExactUInt32(NSNumber(value: $0)) }
            let head = mirrorScalarHead.flatMap { v2ExactUInt32(NSNumber(value: $0)) }
            if (mirrorScalarAnchor != nil && anchor == nil) || (mirrorScalarHead != nil && head == nil) {
                return v2InvalidResultDictionary("invalid render scalar position")
            }
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
            guard v2UInt64Argument(generation) != nil else {
                return v2InvalidResultDictionary("invalid generation")
            }
            return v2BytesResultDictionary(
                editorV2CollaborationSocketOpen(editorId: editorId, generation: generation)
            )
        }
        Function("editorV2CollaborationReceive") { (editorId: String, generation: String, message: Data) -> [String: Any] in
            guard v2UInt64Argument(generation) != nil else {
                return v2InvalidResultDictionary("invalid generation")
            }
            return v2JsonResultDictionary(
                editorV2CollaborationReceive(editorId: editorId, generation: generation, message: message)
            )
        }
        Function("editorV2CollaborationSocketClose") { (editorId: String, generation: String, code: Double?, reason: String?) -> [String: Any] in
            guard v2UInt64Argument(generation) != nil else {
                return v2InvalidResultDictionary("invalid generation")
            }
            let closeCode = code.flatMap { v2ExactUInt32(NSNumber(value: $0)) }
            if code != nil && closeCode == nil {
                return v2InvalidResultDictionary("invalid close code")
            }
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
            guard v2UInt64Argument(generation) != nil else {
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
            renderDocumentProbe(configJson: configJson) { adapter in
                adapter.setContentJson(json)
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
            renderDocumentProbe(configJson: configJson) { adapter in
                adapter.setContentHtml(html)
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

            Prop("editorId") { (view: NativeEditorExpoView, id: String) in
                view.setEditorId(v2UInt64Argument(id) ?? 0)
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
            Prop("editorUpdateRevision") { (view: NativeEditorExpoView, editorUpdateRevision: Double) in
                guard let exactRevision = v2ExactUInt32(NSNumber(value: editorUpdateRevision)) else {
                    return
                }
                view.setPendingEditorUpdateRevision(Int(exactRevision))
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
