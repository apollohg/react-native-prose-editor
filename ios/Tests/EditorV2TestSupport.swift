import XCTest

// MARK: - v2 test construction shim (Task 16 production cutover)
//
// Legacy-era tests constructed editors through the deleted legacy UniFFI
// ABI. These helpers keep the same test shape — one public id per editor,
// destroyed after use — backed by the production v2 create path: the public
// id IS the v2 session handle, registered in the session pairing registry,
// so every view/shadow interaction routes through the typed v2 adapter.

/// Create one v2 editor session for tests (production create-path parity:
/// one v2 session per editor, the public id registered in the session
/// pairing registry). Fails the test on a structured v2 creation error.
func makeV2Editor(
    configJson: String = "{}",
    file: StaticString = #filePath,
    line: UInt = #line
) -> UInt64 {
    switch EditorV2Registry.createPair(legacyConfigJson: configJson) {
    case .success(let publicId):
        return publicId
    case .failure(let error):
        XCTFail(
            "v2 create failed: \(error.domain)/\(error.code): \(error.message)",
            file: file,
            line: line
        )
        return 0
    }
}

/// Destroy a v2 editor session created by `makeV2Editor`.
func destroyV2Editor(id: UInt64) {
    EditorV2Registry.destroyPair(forLegacyId: id)
}
