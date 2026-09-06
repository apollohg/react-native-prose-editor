import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testAccessoryToolbarPlacementDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange { view in
            view.setToolbarPlacement("inline")
        } verify: { view, _, file, line in
            XCTAssertTrue(view.isUsingAccessoryPlaceholderForTesting(), file: file, line: line)
            XCTAssertFalse(view.isUsingAccessoryToolbarForTesting(), file: file, line: line)
        }
    }

    func testAccessoryToolbarVisibilityDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange { view in
            view.setShowToolbar(false)
        } verify: { view, _, file, line in
            XCTAssertTrue(view.isUsingAccessoryPlaceholderForTesting(), file: file, line: line)
            XCTAssertFalse(view.isUsingAccessoryToolbarForTesting(), file: file, line: line)
        }
    }

    func testThemeAccessoryReloadDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange { view in
            view.setThemeJson(#"{"toolbar":{"appearance":"native"}}"#)
        }
    }

    func testBlockedThemeRetryIsClearedWhenDesiredThemeRevertsBeforeRetry() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        let themeA = "{\"backgroundColor\":\"#101820\"}"
        let themeB = "{\"backgroundColor\":\"#ffeedd\"}"
        view.setEditorId(editorId)
        view.setThemeJson(themeA)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.setThemeJson(themeB)
        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))

        view.setThemeJson(themeA)
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))
        XCTAssertNotEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#ffeedd"))
    }

    func testBlockedThemeRetryAppliesDesiredThemeAfterEditorRebind() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>First</p>")
        _ = EditorV2Shadow.setHtml(id: secondEditorId, html: "<p>Second</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        let themeA = "{\"backgroundColor\":\"#101820\"}"
        let themeB = "{\"backgroundColor\":\"#ffeedd\"}"
        view.setEditorId(firstEditorId)
        view.setThemeJson(themeA)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setThemeJson(themeB)
        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))

        view.setEditorId(secondEditorId)
        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#ffeedd"))

        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#ffeedd"))
        XCTAssertNotEqual(view.richTextView.textView.theme?.backgroundColor, EditorTheme.color(from: "#101820"))
    }

    func testBlockedAtomsPropRetriesAfterCompositionEndsWithoutPropRedelivery() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        flushMainQueue()
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        view.setAtomsJson(
            #"{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"#
        )
        XCTAssertNil(view.richTextView.textView.atomRenderConfiguration)

        view.richTextView.textView.unmarkText()
        let retried = expectation(description: "atoms configuration reapplied")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
            retried.fulfill()
        }
        wait(for: [retried], timeout: 1)

        XCTAssertEqual(
            view.richTextView.textView.atomRenderConfiguration?.registeredNodeTypes,
            ["counterCard"]
        )
    }

    func testBlockedAtomsPropRetryIsDelayedAndCapped() {
        let view = NativeEditorExpoView()
        view.blockAtomConfigurationApplyForTesting = true

        view.setAtomsJson(
            #"{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"#
        )
        let settled = expectation(description: "retry queue settles")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) {
            settled.fulfill()
        }
        wait(for: [settled], timeout: 1)

        XCTAssertEqual(view.atomsRetryAttemptsForTesting, 5)
        XCTAssertNil(view.richTextView.textView.atomRenderConfiguration)
    }

    func testBlockedAtomsPropWakesAfterRetryCapWhenCompositionEnds() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        view.blockAtomConfigurationApplyForTesting = true
        view.setAtomsJson(
            #"{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"#
        )

        let capped = expectation(description: "atom retries reach their cap")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) {
            capped.fulfill()
        }
        wait(for: [capped], timeout: 1)
        XCTAssertEqual(view.atomsRetryAttemptsForTesting, 5)
        XCTAssertNil(view.richTextView.textView.atomRenderConfiguration)

        view.blockAtomConfigurationApplyForTesting = false
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 5)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.richTextView.textView.unmarkText()
        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(
            view.richTextView.textView.atomRenderConfiguration?.registeredNodeTypes,
            ["counterCard"]
        )
    }

    func testMentionAddonRefreshDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange(
            initialHTML: "<p>teh @al</p>",
            selectionOffset: 7
        ) { view in
            view.setAddonsJson(self.aliceMentionAddonsJson())
        } verify: { view, _, file, line in
            XCTAssertNotNil(
                view.currentMentionQueryStateForTesting(trigger: "@"),
                file: file,
                line: line
            )
        }
    }

    func testMentionAddonClearDrainsPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesAccessoryChange(
            initialHTML: "<p>teh @al</p>",
            selectionOffset: 7,
            configure: { view, _ in
                view.setAddonsJson(self.aliceMentionAddonsJson())
            }
        ) { view in
            view.setAddonsJson(nil)
        }
    }

    func testStaleMentionClearRetryDoesNotHideFreshSuggestionsAfterRefreshSucceeds() {
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
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.setAddonsJson(aliceMentionAddonsJson())
        XCTAssertTrue(view.isShowingMentionSuggestionsForTesting())

        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(nil)
        view.setAddonsJson(aliceMentionAddonsJson())

        XCTAssertTrue(
            view.isShowingMentionSuggestionsForTesting(),
            "successful mention refresh should show suggestions before the stale clear retry runs"
        )

        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(
            view.isShowingMentionSuggestionsForTesting(),
            "stale clear retry should not hide suggestions from a later successful refresh"
        )
    }

    func testAccessoryRetryBatchKeepsNonConflictingToolbarVisibilityActionAfterMentionClear() {
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
        view.setToolbarPlacement("keyboard")
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.setAddonsJson(aliceMentionAddonsJson())
        XCTAssertTrue(view.isUsingAccessoryToolbarForTesting())

        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(nil)
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setShowToolbar(false)

        XCTAssertTrue(
            view.isUsingAccessoryToolbarForTesting(),
            "toolbar visibility should remain unchanged while the accessory update is queued"
        )

        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(
            view.isUsingAccessoryPlaceholderForTesting(),
            "successful mention clear retry should not cancel a queued toolbar visibility retry"
        )
        XCTAssertFalse(view.isUsingAccessoryToolbarForTesting())
    }

    func testAccessoryRetryBatchKeepsRemainingActionsWhenFirstRetryRequeues() {
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
        view.setToolbarPlacement("keyboard")
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: view.richTextView.textView.textStorage.length)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.setAddonsJson(aliceMentionAddonsJson())
        XCTAssertTrue(view.isShowingMentionSuggestionsForTesting())
        XCTAssertTrue(view.isUsingAccessoryToolbarForTesting())

        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(nil)
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setAddonsJson(aliceMentionAddonsJson())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))
        view.setShowToolbar(false)

        flushMainQueue()
        flushMainQueue()
        flushMainQueue()

        XCTAssertTrue(
            view.isShowingMentionSuggestionsForTesting(),
            "a refresh queued behind a requeued clear should still run"
        )
        XCTAssertTrue(
            view.isUsingAccessoryPlaceholderForTesting(),
            "toolbar visibility queued behind a requeued clear should still run"
        )
        XCTAssertFalse(view.isUsingAccessoryToolbarForTesting())
    }

}
