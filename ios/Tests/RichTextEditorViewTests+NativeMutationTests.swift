import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {

    func assertCollapsedEditorSelection(
        in editorId: UInt64,
        scalarOffset: UInt32,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let selection = currentSelection(in: editorId)
        let expectedDocPos = EditorV2Shadow.scalarToDoc(id: editorId, scalar: scalarOffset)
        XCTAssertEqual(selection["type"] as? String, "text", file: file, line: line)
        XCTAssertEqual((selection["anchor"] as? NSNumber)?.uint32Value, expectedDocPos, file: file, line: line)
        XCTAssertEqual((selection["head"] as? NSNumber)?.uint32Value, expectedDocPos, file: file, line: line)
    }

}
