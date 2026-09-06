import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {

    func assertExternalCompositionCommitFailureRestores(
        configJSON: String,
        initialText: String,
        finalText: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let editorId = makeV2Editor(configJson: configJSON, file: file, line: line)
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(
            EditorV2Registry.adapter(forLegacyId: editorId),
            file: file,
            line: line
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>\(initialText)</p>")
        setSelection(
            in: textView,
            utf16Range: NSRange(location: 0, length: (initialText as NSString).length)
        )
        let spy = EditorTextViewDelegateSpy()
        textView.editorDelegate = spy
        let revisionBefore = adapter.baseDocumentRevision
        let historyBefore = try XCTUnwrap(adapter.historyFlags(), file: file, line: line)

        _ = textView.beginExternalTextComposition(sessionId: "speech-1")
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: finalText)
        let resultJSON = textView.commitExternalTextComposition(
            sessionId: "speech-1",
            finalText: finalText
        )
        let result = parseJSONObject(resultJSON)
        let duplicate = textView.commitExternalTextComposition(
            sessionId: "speech-1",
            finalText: "ignored"
        )

        XCTAssertEqual(result["outcome"] as? String, "cancelled", file: file, line: line)
        assertExternalCompositionError(
            result,
            code: "EXTERNAL_COMPOSITION_COMMIT_FAILED",
            file: file,
            line: line
        )
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>\(initialText)</p>",
            file: file,
            line: line
        )
        XCTAssertEqual(textView.textStorage.string, initialText, file: file, line: line)
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore, file: file, line: line)
        XCTAssertEqual(adapter.historyFlags()?.canUndo, historyBefore.canUndo, file: file, line: line)
        XCTAssertEqual(adapter.historyFlags()?.canRedo, historyBefore.canRedo, file: file, line: line)
        XCTAssertEqual(duplicate, resultJSON, file: file, line: line)
        XCTAssertEqual(spy.externalCompositionEnds, [resultJSON], file: file, line: line)
        XCTAssertTrue(spy.receivedUpdates.isEmpty, file: file, line: line)
    }

    func assertExternalCompositionError(
        _ result: [String: Any],
        code: String,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        guard let error = result["error"] as? [String: Any] else {
            XCTFail("expected external composition error", file: file, line: line)
            return
        }
        XCTAssertEqual(
            Set(error.keys),
            Set([
                "domain",
                "code",
                "message",
                "requestId",
                "operationIndex",
                "limit",
                "actual",
                "details",
            ]),
            file: file,
            line: line
        )
        XCTAssertEqual(error["domain"] as? String, "lifecycle", file: file, line: line)
        XCTAssertEqual(error["code"] as? String, code, file: file, line: line)
        XCTAssertNotNil(error["message"] as? String, file: file, line: line)
        for key in ["requestId", "operationIndex", "limit", "actual", "details"] {
            XCTAssertTrue(error[key] is NSNull, "expected null \(key)", file: file, line: line)
        }
    }

}
