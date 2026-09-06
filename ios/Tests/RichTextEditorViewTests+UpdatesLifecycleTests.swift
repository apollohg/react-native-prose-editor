import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {

    func assertPendingNativeAutocorrectSurvivesInputTraitChange(
        _ applyTraitChange: (EditorTextView) -> Void,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>teh </p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder(), file: file, line: line)
        view.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        applyTraitChange(view.textView)

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>", file: file, line: line)
        XCTAssertEqual(view.textView.textStorage.string, "the ", file: file, line: line)
        XCTAssertEqual(view.textView.reconciliationCount, 0, file: file, line: line)
    }

    func beginEmptyMarkedComposition(
        in view: RichTextEditorView,
        utf16Offset: Int,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        setCollapsedSelection(in: view.textView, utf16Offset: utf16Offset)
        flushMainQueue()
        XCTAssertTrue(view.textView.becomeFirstResponder(), file: file, line: line)
        view.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
    }

    func assertPendingNativeAutocorrectSurvivesAccessoryChange(
        initialHTML: String = "<p>teh </p>",
        selectionOffset: Int = 4,
        configure: ((NativeEditorExpoView, UInt64) -> Void)? = nil,
        _ applyAccessoryChange: (NativeEditorExpoView) -> Void,
        verify: ((NativeEditorExpoView, UInt64, StaticString, UInt) -> Void)? = nil,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: initialHTML)

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        configure?(view, editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: selectionOffset)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder(), file: file, line: line)
        let expectedText = view.richTextView.textView.textStorage.string.replacingOccurrences(
            of: "teh",
            with: "the"
        )
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        applyAccessoryChange(view)
        flushMainQueue()

        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            initialHTML.replacingOccurrences(of: "teh", with: "the"),
            file: file,
            line: line
        )
        XCTAssertEqual(view.richTextView.textView.textStorage.string, expectedText, file: file, line: line)
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0, file: file, line: line)
        verify?(view, editorId, file, line)
    }

    func internalEditorUpdateRejections(in view: NativeEditorExpoView) -> [String] {
        Mirror(reflecting: view).children.first {
            $0.label == "editorUpdateInternalRejections"
        }?.value as? [String] ?? []
    }

    func retainedPendingEditorUpdateSourceId(in view: NativeEditorExpoView) -> String? {
        Mirror(reflecting: view).children.first {
            $0.label == "pendingEditorUpdateEditorId"
        }?.value as? String
    }

    func assertNoPendingEditorUpdate(
        in view: NativeEditorExpoView,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        let state = Dictionary(uniqueKeysWithValues: Mirror(reflecting: view).children.compactMap {
            child -> (String, Any)? in
            guard let label = child.label else { return nil }
            return (label, child.value)
        })
        XCTAssertNil(state["pendingEditorUpdateJSON"] as? String, file: file, line: line)
        XCTAssertEqual(state["pendingEditorUpdateRevision"] as? Int, 0, file: file, line: line)
        XCTAssertEqual(state["pendingEditorUpdateRetryScheduled"] as? Bool, false, file: file, line: line)
    }

    func encodedJSONObject(_ object: [String: Any]) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: object)
        return try XCTUnwrap(String(data: data, encoding: .utf8))
    }

}

final class AutonomousErrorEventSink {
    var errors: [FfiError] = []

    func record(_ payload: [String: Any]) {
        guard let error = payload["error"] as? [String: Any],
              let domain = error["domain"] as? String,
              let code = error["code"] as? String,
              let message = error["message"] as? String
        else { return }
        func optionalString(_ key: String) -> String? {
            error[key] as? String
        }
        errors.append(FfiError(
            domain: domain,
            code: code,
            message: message,
            requestId: optionalString("requestId"),
            operationIndex: optionalString("operationIndex"),
            limit: optionalString("limit"),
            actual: optionalString("actual"),
            detailsJson: optionalString("detailsJson")
        ))
    }
}
