import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testInputTraitChangesDrainPendingNativeAutocorrectBeforeReload() {
        assertPendingNativeAutocorrectSurvivesInputTraitChange {
            $0.setAutoCorrect(true)
        }
        assertPendingNativeAutocorrectSurvivesInputTraitChange {
            $0.setAutoCapitalize("characters")
        }
        assertPendingNativeAutocorrectSurvivesInputTraitChange {
            $0.setKeyboardType("email-address")
        }
    }

    func testInputTraitChangeFlushesActiveMarkedCompositionBeforeReload() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        setCollapsedSelection(in: view.textView, utf16Offset: 6)
        flushMainQueue()

        XCTAssertTrue(view.textView.becomeFirstResponder())
        view.textView.setMarkedText("brave ", selectedRange: NSRange(location: 6, length: 0))

        view.textView.setKeyboardType("email-address")

        XCTAssertEqual(EditorV2Shadow.getHtml(id: editorId), "<p>Hello brave world</p>")
        XCTAssertEqual(view.textView.textStorage.string, "Hello brave world")
        XCTAssertEqual(view.textView.reconciliationCount, 0)
    }

    func testBlockedAutoCorrectRetryDoesNotOverrideNewerValue() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCorrect(true)
        view.textView.setAutoCorrect(false)
        flushMainQueue()

        XCTAssertEqual(view.textView.autocorrectionType, .no)
        XCTAssertEqual(view.textView.spellCheckingType, .no)
    }

    func testBlockedAutoCapitalizeRetryDoesNotOverrideNewerValue() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCapitalize("characters")
        view.textView.setAutoCapitalize("none")
        flushMainQueue()

        XCTAssertEqual(view.textView.autocapitalizationType, .none)
    }

    func testBlockedKeyboardTypeRetryDoesNotOverrideNewerValue() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setKeyboardType("email-address")
        view.textView.setKeyboardType("url")
        flushMainQueue()

        XCTAssertEqual(view.textView.keyboardType, .URL)
    }

    func testPendingAutoCorrectRetryIsInvalidatedAndDesiredTraitReplayedOnEditorRebind() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = firstEditorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCorrect(true)
        view.editorId = secondEditorId
        flushMainQueue()

        XCTAssertEqual(view.textView.autocorrectionType, .yes)
        XCTAssertEqual(view.textView.spellCheckingType, .default)
    }

    func testPendingAutoCapitalizeRetryIsInvalidatedAndDesiredTraitReplayedOnEditorRebind() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = firstEditorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setAutoCapitalize("characters")
        view.editorId = secondEditorId
        flushMainQueue()

        XCTAssertEqual(view.textView.autocapitalizationType, .allCharacters)
    }

    func testPendingKeyboardTypeRetryIsInvalidatedAndDesiredTraitReplayedOnEditorRebind() {
        let firstEditorId = makeV2Editor()
        let secondEditorId = makeV2Editor()
        defer {
            destroyV2Editor(id: firstEditorId)
            destroyV2Editor(id: secondEditorId)
        }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 120))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = firstEditorId
        view.setContent(html: "<p>Hello world</p>")
        beginEmptyMarkedComposition(in: view, utf16Offset: 6)

        view.textView.setKeyboardType("email-address")
        view.editorId = secondEditorId
        flushMainQueue()

        XCTAssertEqual(view.textView.keyboardType, .emailAddress)
    }

}
