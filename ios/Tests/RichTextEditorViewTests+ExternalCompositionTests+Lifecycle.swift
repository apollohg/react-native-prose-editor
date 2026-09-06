import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testExternalTextCompositionUpdatesViewWithoutMutatingEngine() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>arrival</p>")
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 7))

        let begin = parseJSONObject(
            textView.beginExternalTextComposition(sessionId: "speech-1")
        )
        let update = parseJSONObject(
            textView.updateExternalTextComposition(sessionId: "speech-1", text: "on arrival")
        )
        _ = textView.updateExternalTextComposition(sessionId: "speech-1", text: "O/A")

        XCTAssertEqual(Set(begin.keys), Set(["version", "type", "sessionId"]))
        XCTAssertEqual(begin["version"] as? Int, 1)
        XCTAssertEqual(begin["type"] as? String, "active")
        XCTAssertEqual(begin["sessionId"] as? String, "speech-1")
        XCTAssertEqual(Set(update.keys), Set(["version", "type", "sessionId"]))
        XCTAssertEqual(textView.textStorage.string, "O/A")
        XCTAssertTrue(EditorV2Shadow.getHtml(id: editorId).contains("arrival"))
        XCTAssertFalse(EditorV2Shadow.getHtml(id: editorId).contains("O/A"))
    }

    func testExternalTextCompositionUpdatesPlaceholderFromProvisionalText() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.placeholder = "Type here"
        textView.bindEditor(id: editorId, initialHTML: "<p></p>")

        XCTAssertTrue(textView.isPlaceholderVisibleForTesting())

        _ = textView.beginExternalTextComposition(sessionId: "placeholder")
        _ = textView.updateExternalTextComposition(sessionId: "placeholder", text: "draft")
        XCTAssertFalse(textView.isPlaceholderVisibleForTesting())

        _ = textView.updateExternalTextComposition(sessionId: "placeholder", text: "")
        XCTAssertTrue(textView.isPlaceholderVisibleForTesting())

        _ = textView.updateExternalTextComposition(sessionId: "placeholder", text: "draft")
        XCTAssertFalse(textView.isPlaceholderVisibleForTesting())

        _ = textView.cancelExternalTextComposition(sessionId: "placeholder", cause: "consumer")
        XCTAssertTrue(textView.isPlaceholderVisibleForTesting())
    }

    func testExternalTextCompositionExpoEventCarriesBoundEditorIdentity() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        setSelection(
            in: view.richTextView.textView,
            utf16Range: NSRange(location: 0, length: 7)
        )
        var events: [[String: Any]] = []
        view.onExternalTextCompositionEndForTesting = { events.append($0) }

        _ = view.beginExternalTextComposition(sessionId: "speech-1")
        _ = view.updateExternalTextComposition(sessionId: "speech-1", text: "on arrival")
        _ = view.commitExternalTextComposition(sessionId: "speech-1", finalText: "O/A")

        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events[0]["editorId"] as? String, String(view.richTextView.editorId))
        XCTAssertNotNil(events[0]["resultJson"] as? String)
    }

    func testExternalTextCompositionRebindCancelsInsteadOfCommitting() throws {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>arrival</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(firstEditorId)
        setSelection(
            in: view.richTextView.textView,
            utf16Range: NSRange(location: 0, length: 7)
        )
        var events: [[String: Any]] = []
        view.onExternalTextCompositionEndForTesting = { events.append($0) }
        _ = view.beginExternalTextComposition(sessionId: "speech-1")
        _ = view.updateExternalTextComposition(sessionId: "speech-1", text: "O/A")

        view.setEditorId(secondEditorId)

        XCTAssertFalse(EditorV2Shadow.getHtml(id: firstEditorId).contains("O/A"))
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events[0]["editorId"] as? String, String(firstEditorId))
    }

    func testExternalTextCompositionDestroyCancelsOnce() throws {
        let editorId = makeV2Editor()
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        setSelection(
            in: view.richTextView.textView,
            utf16Range: NSRange(location: 0, length: 7)
        )
        var events: [[String: Any]] = []
        view.onExternalTextCompositionEndForTesting = { events.append($0) }
        _ = view.beginExternalTextComposition(sessionId: "speech-1")
        _ = view.updateExternalTextComposition(sessionId: "speech-1", text: "O/A")

        NativeEditorViewRegistry.shared.destroy(editorId: editorId) {
            destroyV2Editor(id: editorId)
        }

        XCTAssertEqual(view.richTextView.editorId, 0)
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events[0]["editorId"] as? String, String(editorId))
    }

    func testExternalTextCompositionFinalUnbindCancelsOnce() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        var events: [[String: Any]] = []
        weak var releasedView: NativeEditorExpoView?

        autoreleasepool {
            let view = NativeEditorExpoView()
            releasedView = view
            let window = hostNativeEditorExpoView(view)
            view.setEditorId(editorId)
            setSelection(
                in: view.richTextView.textView,
                utf16Range: NSRange(location: 0, length: 7)
            )
            view.onExternalTextCompositionEndForTesting = { events.append($0) }
            _ = view.beginExternalTextComposition(sessionId: "speech-1")
            _ = view.updateExternalTextComposition(sessionId: "speech-1", text: "O/A")

            view.removeFromSuperview()
            XCTAssertTrue(events.isEmpty)
            view.setEditorId(0)
            XCTAssertEqual(events.count, 1)
            window.isHidden = true
        }

        XCTAssertNil(releasedView)
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(events.first?["editorId"] as? String, String(editorId))
        XCTAssertFalse(EditorV2Shadow.getHtml(id: editorId).contains("O/A"))
    }

    func testExternalTextCompositionReadOnlyCancelsWithoutCommitting() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let adapter = try XCTUnwrap(EditorV2Registry.adapter(forLegacyId: editorId))
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        setSelection(
            in: view.richTextView.textView,
            utf16Range: NSRange(location: 0, length: 7)
        )
        let revisionBefore = adapter.baseDocumentRevision
        var events: [[String: Any]] = []
        view.onExternalTextCompositionEndForTesting = { events.append($0) }
        _ = view.beginExternalTextComposition(sessionId: "speech-read-only")
        _ = view.updateExternalTextComposition(
            sessionId: "speech-read-only",
            text: "O/A"
        )

        view.setEditable(false)

        let event = try XCTUnwrap(events.first)
        let result = parseJSONObject(try XCTUnwrap(event["resultJson"] as? String))
        XCTAssertFalse(view.richTextView.textView.isEditable)
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(event["editorId"] as? String, String(editorId))
        XCTAssertEqual(result["outcome"] as? String, "cancelled")
        XCTAssertEqual(result["cause"] as? String, "lifecycle")
        XCTAssertEqual(adapter.baseDocumentRevision, revisionBefore)
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>arrival</p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "arrival")
    }

    func testExternalTextCompositionDeinitCancelsOnceWithBoundEditorIdentity() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>arrival</p>")
        var events: [[String: Any]] = []
        weak var releasedView: NativeEditorExpoView?

        autoreleasepool {
            let view = NativeEditorExpoView()
            releasedView = view
            view.setEditorId(editorId)
            setSelection(
                in: view.richTextView.textView,
                utf16Range: NSRange(location: 0, length: 7)
            )
            view.onExternalTextCompositionEndForTesting = { events.append($0) }
            _ = view.beginExternalTextComposition(sessionId: "speech-deinit")
            _ = view.updateExternalTextComposition(sessionId: "speech-deinit", text: "O/A")
        }

        let event = try XCTUnwrap(events.first)
        let result = parseJSONObject(try XCTUnwrap(event["resultJson"] as? String))
        XCTAssertNil(releasedView)
        XCTAssertEqual(events.count, 1)
        XCTAssertEqual(event["editorId"] as? String, String(editorId))
        XCTAssertEqual(result["outcome"] as? String, "cancelled")
        XCTAssertEqual(result["cause"] as? String, "lifecycle")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>arrival</p>")
    }

}
