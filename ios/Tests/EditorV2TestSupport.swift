import XCTest

// MARK: - v2 test construction shim (Task 16 production cutover)
//
// Test fixtures use the production v2 factory directly, then attach the
// view adapter to its returned handle. No adapter or registry helper creates
// a session or reconstructs a partial config envelope.

func createdV2TestEditorHandle(_ resultJson: String) -> (handle: String, nativeViewId: UInt64)? {
    guard let data = resultJson.data(using: .utf8),
          let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let editorIdString = result["editorId"] as? String,
          let editorId = UInt64(editorIdString),
          editorId > 0,
          editorId <= UInt64(Int64.max),
          editorIdString == String(editorId)
    else {
        return nil
    }
    return (editorIdString, editorId)
}

/// Create one v2 editor session for tests from a complete v2 create envelope.
/// The public id is the session's own decimal handle.
func makeV2Editor(
    configJson: String = #"{"initialization":{"type":"localEmpty"}}"#,
    file: StaticString = #filePath,
    line: UInt = #line
) -> UInt64 {
    let result = editorV2Create(configJson: configJson, snapshotState: nil)
    guard let value = result.value,
          result.error == nil,
          let createdHandle = createdV2TestEditorHandle(value),
          let adapter = EditorV2Adapter.attach(editorId: createdHandle.handle, roomBound: false)
    else {
        let error = result.error
        XCTFail(
            "v2 create/attach failed: \(error?.domain ?? "boundary")/\(error?.code ?? "FFI_RESULT_INVALID"): \(error?.message ?? "missing canonical editor id")",
            file: file,
            line: line
        )
        return 0
    }
    EditorV2Registry.register(adapter, forLegacyId: createdHandle.nativeViewId)
    return createdHandle.nativeViewId
}

/// Destroy a v2 editor session created by `makeV2Editor`.
func destroyV2Editor(id: UInt64) {
    EditorV2Registry.destroyPair(forLegacyId: id)
}
