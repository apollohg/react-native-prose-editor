import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testMentionSuggestionTapRetryIsDroppedAfterQueryChanges() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let changedUpdateJSON = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Hello @bo</p>")
        view.richTextView.textView.applyUpdateJSON(changedUpdateJSON, notifyDelegate: false)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "bo", trigger: "@", anchor: 6, head: 9)
        )

        flushMainQueue()
        flushMainQueue()

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertEqual(html, "<p>Hello @bo</p>")
        XCTAssertFalse(
            html.contains("data-native-editor-mention=\"true\""),
            "stale mention retry should not insert into a changed query, got: \(html)"
        )
    }

    func testMentionSuggestionTapRetryIsDroppedAfterSameQueryRangeChanges() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let changedUpdateJSON = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>@al Hello @al</p>")
        view.richTextView.textView.applyUpdateJSON(changedUpdateJSON, notifyDelegate: false)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 10, head: 13)
        )

        flushMainQueue()
        flushMainQueue()

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertEqual(html, "<p>@al Hello @al</p>")
        XCTAssertFalse(
            html.contains("data-native-editor-mention=\"true\""),
            "same-query retry should still be dropped when its range moved, got: \(html)"
        )
    }

    func testMentionSuggestionTapRetryIsDroppedAfterEditorRebind() {
        let firstEditorId = makeV2Editor(configJson: mentionEditorConfigJson())
        let secondEditorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>Hello @al</p>")
        _ = EditorV2Shadow.setHtml(id: secondEditorId, html: "<p>Second @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(firstEditorId)
        view.setAddonsJson(aliceMentionAddonsJson())
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)
        view.setEditorId(secondEditorId)
        flushMainQueue()
        flushMainQueue()

        XCTAssertFalse(EditorV2Shadow.getHtml(id: firstEditorId).contains("data-native-editor-mention=\"true\""))
        XCTAssertFalse(EditorV2Shadow.getHtml(id: secondEditorId).contains("data-native-editor-mention=\"true\""))
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second @al")
    }

    func testMentionSuggestionTapStillWorksAfterRebindingToMentionSchemaEditor() {
        let initialEditorId = makeV2Editor()
        let mentionEditorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer {
            destroyV2Editor(id: initialEditorId)
            destroyV2Editor(id: mentionEditorId)
        }

        _ = EditorV2Shadow.setHtml(id: initialEditorId, html: "<p>Hello</p>")
        _ = EditorV2Shadow.setHtml(id: mentionEditorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.setEditorId(initialEditorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setEditorId(mentionEditorId)
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 6, head: 9)
        )
        view.setMentionSuggestionsForTesting([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["id": "user_alice", "label": "@alice"],
            ])!,
        ])

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let html = EditorV2Shadow.getHtml(id: mentionEditorId)
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention insert should target the rebound mention-schema editor, got: \(html)"
        )
    }

    func testCurrentMentionQueryStateWorksInsideListItem() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        let setHtmlUpdate = EditorV2Shadow.setHtml(id: editorId, html: "<ul><li><p>Hello @al</p></li></ul>")
        XCTAssertTrue(setHtmlUpdate.contains("@al"), "setHtml must return the updated list snapshot, got: \(setHtmlUpdate)")
        view.richTextView.textView.applyUpdateJSON(setHtmlUpdate, notifyDelegate: false)

        let text = view.richTextView.textView.text ?? ""
        let mentionRange = (text as NSString).range(of: "@al")
        XCTAssertNotEqual(mentionRange.location, NSNotFound, "rendered list text should contain the mention query, got: \(text), state: \(setHtmlUpdate)")
        guard mentionRange.location != NSNotFound else { return }
        let utf16Offset = mentionRange.location + 3
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: utf16Offset)

        let queryState = view.currentMentionQueryStateForTesting(trigger: "@")
        XCTAssertEqual(queryState?.query, "al")
        XCTAssertNotNil(queryState, "mention query should resolve inside a list item")
    }

    func testCurrentMentionQueryStateWorksInLastParagraph() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        let setHtmlUpdate = EditorV2Shadow.setHtml(id: editorId, html: "<p>First paragraph</p><p>@al</p>")
        XCTAssertTrue(setHtmlUpdate.contains("@al"), "setHtml must return the updated paragraph snapshot, got: \(setHtmlUpdate)")
        view.richTextView.textView.applyUpdateJSON(setHtmlUpdate, notifyDelegate: false)

        let text = view.richTextView.textView.text ?? ""
        let mentionRange = (text as NSString).range(of: "@al")
        XCTAssertNotEqual(mentionRange.location, NSNotFound, "rendered final paragraph should contain the mention query, got: \(text), state: \(setHtmlUpdate)")
        guard mentionRange.location != NSNotFound else { return }
        let utf16Offset = mentionRange.location + 3
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: utf16Offset)

        let queryState = view.currentMentionQueryStateForTesting(trigger: "@")
        XCTAssertEqual(queryState?.query, "al")
        XCTAssertNotNil(queryState, "mention query should resolve in the final paragraph")
    }

}
