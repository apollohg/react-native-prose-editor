import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testMentionSuggestionTapInsertsMentionNode() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello @al</p>")

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
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

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "tapping a mention suggestion should insert a mention node, got: \(html)"
        )
        XCTAssertTrue(
            html.contains("@alice"),
            "mention insertion should preserve the visible label, got: \(html)"
        )
        XCTAssertTrue(
            html.contains("mentionSuggestionChar"),
            "mention insertion should preserve the suggestion trigger in attrs, got: \(html)"
        )
    }

    func testMentionSuggestionTapDrainsPendingNativeAutocorrectBeforeInsert() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>teh @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 4, head: 7)
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
        view.layoutIfNeeded()
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(html.contains("the"), "mention insert should preserve native correction, got: \(html)")
        XCTAssertFalse(html.contains("teh"), "mention insert should not restore stale text, got: \(html)")
        XCTAssertFalse(html.contains("@al</p>"), "mention insert should replace the query range, got: \(html)")
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention insert should still insert the mention node, got: \(html)"
        )
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testMentionSelectRequestIncludesPreflightUpdateAfterNativeAutocorrectDrain() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>teh @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","resolveSelectionAttrs":true,"suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 4, head: 7)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let event = parseJSONObject(view.lastAddonEventJSONForTesting())
        XCTAssertEqual(event["type"] as? String, "mentionsSelectRequest")
        XCTAssertEqual(event["suggestionKey"] as? String, "alice")
        let range = event["range"] as? [String: Any]
        XCTAssertEqual(jsonInt(range?["anchor"]), 4)
        XCTAssertEqual(jsonInt(range?["head"]), 7)

        let updateJSON = event["updateJson"] as? String
        XCTAssertNotNil(updateJSON)
        XCTAssertTrue(updateJSON?.contains("the @al") == true, "select request should carry the drained correction update")
        XCTAssertFalse(updateJSON?.contains("teh @al") == true, "select request should not carry stale pre-correction text")

        let update = parseJSONObject(updateJSON)
        XCTAssertEqual(event["documentVersion"] as? String, update["documentVersion"] as? String)
    }

    func testMentionSuggestionTapDrainsPendingNativeAutocorrectInsideListItem() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<ul><li><p>teh @al</p></li></ul>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 4, head: 7)
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
        view.layoutIfNeeded()
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(html.contains("<ul><li><p>the "), "mention insert should preserve list correction, got: \(html)")
        XCTAssertFalse(html.contains("teh"), "mention insert should not restore stale list text, got: \(html)")
        XCTAssertFalse(html.contains("@al</p>"), "mention insert should replace the list query range, got: \(html)")
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention insert should still insert the mention node in the list item, got: \(html)"
        )
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testMentionSuggestionTapRecomputesRangeAfterLengthChangingAutocorrect() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>a @al</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Alice Chen","subtitle":"Design","label":"@alice","attrs":{"id":"user_alice","label":"@alice"}}]}}
            """
        )
        view.setMentionQueryStateForTesting(
            MentionQueryState(query: "al", trigger: "@", anchor: 2, head: 5)
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
        view.layoutIfNeeded()
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 1),
            with: "an"
        )
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)

        view.triggerMentionSuggestionTapForTesting(at: 0)

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(html.contains("an "), "mention insert should preserve length-changing correction, got: \(html)")
        XCTAssertFalse(html.contains("@al</p>"), "mention insert should replace the recomputed query range, got: \(html)")
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention insert should insert the mention node after recomputing the range, got: \(html)"
        )
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testMentionSuggestionTapRetriesAfterBlockedMarkedTextPreflight() {
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

        XCTAssertFalse(EditorV2Shadow.getHtml(id: editorId).contains("data-native-editor-mention=\"true\""))

        flushMainQueue()
        flushMainQueue()

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention tap should retry after composition preflight clears, got: \(html)"
        )
        XCTAssertFalse(html.contains("@al</p>"), "retried mention tap should replace query, got: \(html)")
    }

    func testMentionSuggestionTapRetrySurvivesPreflightDrainedAutocorrect() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>teh @al</p>")

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
            MentionQueryState(query: "al", trigger: "@", anchor: 4, head: 7)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)
        XCTAssertFalse(EditorV2Shadow.getHtml(id: editorId).contains("data-native-editor-mention=\"true\""))

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(html.contains("the "), "retried mention tap should preserve preflight correction, got: \(html)")
        XCTAssertFalse(html.contains("teh"), "retried mention tap should not restore stale text, got: \(html)")
        XCTAssertFalse(html.contains("@al</p>"), "retried mention tap should replace the query, got: \(html)")
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention tap should retry after draining autocorrect during preflight, got: \(html)"
        )
    }

    func testMentionSuggestionTapRetrySurvivesLengthChangingPreflightDrainedAutocorrect() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>a @al</p>")

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
            MentionQueryState(query: "al", trigger: "@", anchor: 2, head: 5)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)
        XCTAssertFalse(EditorV2Shadow.getHtml(id: editorId).contains("data-native-editor-mention=\"true\""))

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 1),
            with: "an"
        )
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(html.contains("an "), "retried mention tap should preserve length-changing correction, got: \(html)")
        XCTAssertFalse(html.contains("<p>a "), "retried mention tap should not restore stale text, got: \(html)")
        XCTAssertFalse(html.contains("@al</p>"), "retried mention tap should replace the shifted query, got: \(html)")
        XCTAssertTrue(
            html.contains("data-native-editor-mention=\"true\""),
            "mention tap should retry after draining shifted autocorrect during preflight, got: \(html)"
        )
    }

    func testMentionSuggestionTapRetryIsDroppedWhenPreflightShiftTargetsDifferentSameQuery() {
        let editorId = makeV2Editor(configJson: mentionEditorConfigJson())
        defer { destroyV2Editor(id: editorId) }

        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>a @al b @al</p>")

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
            MentionQueryState(query: "al", trigger: "@", anchor: 2, head: 5)
        )
        view.setMentionSuggestionsForTesting([aliceMentionSuggestion()])
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.triggerMentionSuggestionTapForTesting(at: 0)
        XCTAssertFalse(EditorV2Shadow.getHtml(id: editorId).contains("data-native-editor-mention=\"true\""))

        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 1),
            with: "an"
        )
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertEqual(html, "<p>an @al b @al</p>")
        XCTAssertFalse(
            html.contains("data-native-editor-mention=\"true\""),
            "retry should not jump to a different identical query after preflight drains a correction, got: \(html)"
        )
    }

    func testMentionSuggestionTapRetryUsesRefreshedSuggestionForSameKey() {
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

        let refreshedSuggestion = NativeMentionSuggestion(dictionary: [
            "key": "alice",
            "title": "Ally Chen",
            "subtitle": "Design",
            "label": "@ally",
            "attrs": ["id": "user_ally", "label": "@ally"],
        ])!
        view.setAddonsJson(
            """
            {"mentions":{"trigger":"@","suggestions":[{"key":"alice","title":"Ally Chen","subtitle":"Design","label":"@ally","attrs":{"id":"user_ally","label":"@ally"}}]}}
            """
        )
        view.setMentionSuggestionsForTesting([refreshedSuggestion])

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(
            html.contains("@ally"),
            "retried mention tap should use the refreshed same-key label, got: \(html)"
        )
        XCTAssertFalse(
            html.contains("@alice"),
            "retried mention tap should not use the stale captured label, got: \(html)"
        )

        let event = parseJSONObject(view.lastAddonEventJSONForTesting())
        let attrs = event["attrs"] as? [String: Any]
        XCTAssertEqual(event["type"] as? String, "mentionsSelect")
        XCTAssertEqual(event["suggestionKey"] as? String, "alice")
        XCTAssertEqual(attrs?["id"] as? String, "user_ally")
        XCTAssertEqual(attrs?["label"] as? String, "@ally")
    }

}
