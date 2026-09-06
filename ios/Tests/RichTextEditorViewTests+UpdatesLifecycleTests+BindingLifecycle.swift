import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testApplyEditorUpdateRetriesAfterBlockedCompositionOnSameEditor() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>First</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 0)

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Remote</p>")
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId),
              let updateJSON = editorV2RenderUpdate(
                editorId: adapter.editorId,
                mirrorScalarAnchor: nil,
                mirrorScalarHead: nil
              ).value
        else {
            XCTFail("expected atomic render snapshot")
            return
        }
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        XCTAssertFalse(view.applyEditorUpdate(updateJSON))
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "First")

        flushMainQueue()
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Remote")
    }

    func testApplyEditorUpdateRetryIsDroppedAfterEditorRebind() {
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
        view.setEditorId(firstEditorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 0)

        let staleUpdateJSON = EditorV2Shadow.replaceHtml(id: firstEditorId, html: "<p>Remote</p>")
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        XCTAssertFalse(view.applyEditorUpdate(staleUpdateJSON))
        view.setEditorId(secondEditorId)
        flushMainQueue()

        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
    }

    func testSameEditorIdUpdateDoesNotDropPendingNativeAutocorrect() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>teh </p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.setEditorId(editorId)
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "the ")
        XCTAssertEqual(view.richTextView.textView.reconciliationCount, 0)
    }

    func testPendingNativeAutocorrectIsDroppedAfterEditorRebind() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>teh </p>")
        _ = EditorV2Shadow.setHtml(id: secondEditorId, html: "<p>Second</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(firstEditorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        view.setEditorId(secondEditorId)
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: firstEditorId), "<p>teh </p>")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: secondEditorId), "<p>Second</p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
    }

    func testPrepareForCommandAfterEditorRebindDoesNotDrainPreviousEditorMutation() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }
        _ = EditorV2Shadow.setHtml(id: firstEditorId, html: "<p>teh </p>")
        _ = EditorV2Shadow.setHtml(id: secondEditorId, html: "<p>Second</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            NativeEditorViewRegistry.shared.unregister(editorId: secondEditorId, view: view)
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(firstEditorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        flushMainQueue()

        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )
        view.setEditorId(secondEditorId)

        let preparationJSON = NativeEditorViewRegistry.shared.prepareForCommandJSON(
            editorId: firstEditorId
        )
        XCTAssertTrue(preparationJSON.contains("\"ready\":true"))
        flushMainQueue()

        XCTAssertEqual(EditorV2Shadow.getHtml(id: firstEditorId), "<p>teh </p>")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: secondEditorId), "<p>Second</p>")
        XCTAssertEqual(view.richTextView.textView.textStorage.string, "Second")
    }

    func testDestroyedEditorInvalidatesRegistryAndUnbindsView() {
        let editorId = makeV2Editor()
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }

        view.setEditorId(editorId)
        XCTAssertEqual(view.richTextView.editorId, editorId)
        XCTAssertEqual(view.richTextView.textView.editorId, editorId)
        view.setPendingEditorUpdateEditorId(String(editorId))
        XCTAssertEqual(retainedPendingEditorUpdateSourceId(in: view), String(editorId))

        NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: editorId)
        destroyV2Editor(id: editorId)
        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )

        XCTAssertEqual(preparation["ready"] as? Bool, false)
        XCTAssertEqual(preparation["blockedReason"] as? String, "destroyed")
        XCTAssertEqual(view.richTextView.editorId, 0)
        XCTAssertEqual(view.richTextView.textView.editorId, 0)
        XCTAssertNil(retainedPendingEditorUpdateSourceId(in: view))
    }

    func testMalformedEditorIdPropRetainsExistingBinding() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)

        applyNativeEditorIdProp("01", to: view)

        XCTAssertEqual(view.richTextView.editorId, editorId)
        XCTAssertEqual(view.richTextView.textView.editorId, editorId)
    }

    func testDestroyedEditorInvalidatesEveryBoundView() {
        let editorId = makeV2Editor()
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)
        let first = NativeEditorExpoView()
        let second = NativeEditorExpoView()
        first.setEditorId(editorId)
        second.setEditorId(editorId)

        NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: editorId)
        destroyV2Editor(id: editorId)

        XCTAssertEqual(first.richTextView.editorId, 0)
        XCTAssertEqual(second.richTextView.editorId, 0)
    }

    func testUnregisterRemovesOnlyCallingViewFromEditorRegistry() {
        let editorId = makeV2Editor()
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)
        let first = NativeEditorExpoView()
        let second = NativeEditorExpoView()
        first.setEditorId(editorId)
        second.setEditorId(editorId)

        NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: first)
        NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: editorId)
        destroyV2Editor(id: editorId)

        XCTAssertEqual(first.richTextView.editorId, editorId)
        XCTAssertEqual(second.richTextView.editorId, 0)
        first.setEditorId(0)
    }

    func testDetachedOwnerReelectsNewestAttachedSurvivorAndCatchesItUp() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Initial</p>")

        let first = NativeEditorExpoView()
        let second = NativeEditorExpoView()
        let third = NativeEditorExpoView()
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        [first, second, third].forEach {
            $0.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
            viewController.view.addSubview($0)
            $0.setEditorId(editorId)
        }
        defer {
            [first, second, third].forEach {
                $0.setEditorId(0)
                $0.removeFromSuperview()
            }
            window.isHidden = true
        }

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Updated</p>")
        third.removeFromSuperview()

        XCTAssertEqual(second.richTextView.textView.textStorage.string, "Updated")
        XCTAssertEqual(first.richTextView.textView.textStorage.string, "Initial")

        _ = EditorV2Shadow.replaceHtml(id: editorId, html: "<p>Latest</p>")
        second.removeFromSuperview()

        XCTAssertEqual(first.richTextView.textView.textStorage.string, "Latest")
    }

    func testDestroyBoundaryBlocksReentrantRegistrationAndCommandsUntilInvalidation() {
        let editorId = makeV2Editor()
        let registry = NativeEditorViewRegistry.shared
        registry.markEditorCreated(editorId: editorId)
        let first = NativeEditorExpoView()
        let second = NativeEditorExpoView()
        first.setEditorId(editorId)
        var nestedDestroyRan = false

        registry.destroy(editorId: editorId) {
            XCTAssertFalse(registry.register(editorId: editorId, view: second))
            XCTAssertTrue(
                registry.prepareForCommandJSON(editorId: editorId).contains("\"blockedReason\":\"destroying\"")
            )
            registry.destroy(editorId: editorId) { nestedDestroyRan = true }
            destroyV2Editor(id: editorId)
            XCTAssertEqual(first.richTextView.editorId, editorId)
        }

        XCTAssertFalse(nestedDestroyRan)
        XCTAssertEqual(first.richTextView.editorId, 0)
        XCTAssertEqual(second.richTextView.editorId, 0)
    }

    func testDestroyBoundaryInvalidatesViewsWhenDestroyOperationDoesNotRemoveEditor() {
        let editorId = makeV2Editor()
        let registry = NativeEditorViewRegistry.shared
        registry.markEditorCreated(editorId: editorId)
        let view = NativeEditorExpoView()
        view.setEditorId(editorId)

        registry.destroy(editorId: editorId) {
            // Deterministically simulate a native destroy operation that returned
            // without removing the Rust editor.
        }

        XCTAssertEqual(view.richTextView.editorId, 0)
        XCTAssertFalse(EditorV2Shadow.getCurrentState(id: editorId).contains("editor not found"))
        destroyV2Editor(id: editorId)
    }

    func testDestroyedEditorIdCannotRegisterNewView() {
        let editorId = makeV2Editor()
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: editorId)
        NativeEditorViewRegistry.shared.invalidateDestroyedEditor(editorId: editorId)
        destroyV2Editor(id: editorId)

        let view = NativeEditorExpoView()
        view.setEditorId(editorId)
        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )

        XCTAssertEqual(view.richTextView.editorId, 0)
        XCTAssertEqual(view.richTextView.textView.editorId, 0)
        XCTAssertEqual(preparation["ready"] as? Bool, false)
        XCTAssertEqual(preparation["blockedReason"] as? String, "destroyed")
    }

    func testPrepareForCommandReportsCompositionBlockedReasonWhenMarkedTextPreflightDefers() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Hello</p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: view)
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 0)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0))

        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )

        XCTAssertEqual(preparation["ready"] as? Bool, false)
        XCTAssertEqual(preparation["blockedReason"] as? String, "composition")
        XCTAssertNil(preparation["updateJSON"])
    }

    func testPrepareForCommandIncludesUpdateJSONAfterNativeAutocorrectDrain() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>teh </p>")

        let view = NativeEditorExpoView()
        view.frame = CGRect(x: 0, y: 0, width: 320, height: 160)
        let window = hostNativeEditorExpoView(view)
        defer {
            NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: view)
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.setEditorId(editorId)
        setCollapsedSelection(in: view.richTextView.textView, utf16Offset: 4)
        XCTAssertTrue(view.richTextView.textView.becomeFirstResponder())
        view.richTextView.textView.textStorage.replaceCharacters(
            in: NSRange(location: 0, length: 3),
            with: "the"
        )

        let preparation = parseJSONObject(
            NativeEditorViewRegistry.shared.prepareForCommandJSON(editorId: editorId)
        )
        let updateJSON = preparation["updateJSON"] as? String

        XCTAssertEqual(preparation["ready"] as? Bool, true)
        XCTAssertNil(preparation["blockedReason"])
        XCTAssertNotNil(updateJSON)
        XCTAssertTrue(updateJSON?.contains("the ") == true, "preflight update should include the drained correction")
        XCTAssertFalse(updateJSON?.contains("teh ") == true, "preflight update should not contain stale text")
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>the </p>")
    }

    func testPrepareForCommandIncludesUpdateJSONAfterSameTextCompositionChangesSelectionState() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        textView.bindEditor(id: editorId, initialHTML: "<p>Hello world</p>")
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 0, scalarHead: 5)
        setSelection(in: textView, utf16Range: NSRange(location: 0, length: 5))

        textView.setMarkedText("Hello", selectedRange: NSRange(location: 5, length: 0))
        let preparation = textView.prepareForExternalEditorCommand()

        XCTAssertTrue(preparation.ready)
        XCTAssertNil(preparation.blockedReason)
        XCTAssertNotNil(
            preparation.updateJSON,
            "same-text composition commits should still forward selection/state changes"
        )
        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello world</p>")
        XCTAssertEqual(textView.textStorage.string, "Hello world")
    }

}
