import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testVersionedParagraphSplitKeepsCollapsedSiblingMargins() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        textView.captureApplyUpdateTraceForTesting = true
        textView.theme = try XCTUnwrap(EditorTheme.from(json: #"{"version":1,"styles":{"paragraph":{"marginTop":20,"marginBottom":12}}}"#))
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")
        let beta = (textView.text as NSString).range(of: "Beta")
        let splitOffset = UInt32(NSMaxRange(beta))
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: splitOffset, scalarHead: splitOffset)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.insertText("\n")
        XCTAssertTrue(textView.lastRenderAppliedPatch())
        let text = textView.textStorage.string as NSString
        var start = 0
        while start < text.length {
            let range = text.paragraphRange(for: NSRange(location: start, length: 0))
            let style = try XCTUnwrap(textView.textStorage.attribute(.paragraphStyle, at: start, effectiveRange: nil) as? NSParagraphStyle)
            XCTAssertEqual(style.paragraphSpacingBefore, start == 0 ? 20 : 8, accuracy: 0.01, "paragraph at \(start)")
            start = NSMaxRange(range)
        }
    }

    func testParagraphSplitAppliesTopLevelRenderPatch() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")

        let betaRange = (textView.text as NSString).range(of: "Beta")
        XCTAssertNotEqual(betaRange.location, NSNotFound)
        let splitOffset = UInt32(betaRange.location + betaRange.length)
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: splitOffset, scalarHead: splitOffset)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        XCTAssertEqual(textView.currentLogicalScalarSelection()?.head, splitOffset)

        textView.insertText("\n")

        XCTAssertTrue(
            textView.lastRenderAppliedPatch(),
            "splitting a middle paragraph should use the native top-level patch path"
        )
        XCTAssertEqual(
            textView.textStorage.string,
            "Alpha\nBeta\n\u{200B}\nGamma",
            "split patches must replace the full structural block region so the new paragraph separator renders correctly"
        )
        let selectedOffset = textView.offset(
            from: textView.beginningOfDocument,
            to: textView.selectedTextRange?.start ?? textView.endOfDocument
        )
        let gammaRange = (textView.text as NSString).range(of: "Gamma")
        XCTAssertGreaterThanOrEqual(
            selectedOffset,
            betaRange.location + betaRange.length + 1,
            "after splitting at the end of a paragraph, the caret should land inside the inserted empty paragraph"
        )
        XCTAssertLessThan(
            selectedOffset,
            gammaRange.location,
            "after splitting at the end of a paragraph, the caret must stay before the following paragraph"
        )
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>Alpha</p><p>Beta</p><p></p><p>Gamma</p>"
        )
    }

    func testSequentialParagraphSplitsKeepUsingTopLevelRenderPatch() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 180))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")

        let betaRange = (textView.text as NSString).range(of: "Beta")
        XCTAssertNotEqual(betaRange.location, NSNotFound)
        let firstSplitOffset = UInt32(betaRange.location + betaRange.length)
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: firstSplitOffset, scalarHead: firstSplitOffset)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.insertText("\n")

        XCTAssertTrue(textView.lastRenderAppliedPatch())

        let gammaRange = (textView.text as NSString).range(of: "Gamma")
        XCTAssertNotEqual(gammaRange.location, NSNotFound)
        let secondSplitOffset = UInt32(gammaRange.location + gammaRange.length)
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: secondSplitOffset, scalarHead: secondSplitOffset)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        textView.insertText("\n")

        XCTAssertTrue(
            textView.lastRenderAppliedPatch(),
            "top-level metadata cache should remain valid across consecutive structural edits"
        )
        XCTAssertEqual(
            textView.textStorage.string,
            "Alpha\nBeta\n\u{200B}\nGamma\n\u{200B}"
        )
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<p>Alpha</p><p>Beta</p><p></p><p>Gamma</p><p></p>"
        )
    }

    func testFullAtomicRenderRefreshesShiftedImageDocPosBeforeResizeAction() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let view = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let window = hostEditorView(view)
        defer {
            view.removeFromSuperview()
            window.isHidden = true
        }
        view.editorId = editorId
        view.setContent(html: """
        <p>A</p><img src="https://example.com/target.png" width="140" height="80"><img src="https://example.com/control.png" width="90" height="60"><p></p>
        """)
        view.layoutIfNeeded()

        guard let originalTargetRange = firstImageRange(in: view.textView),
              let originalDocPos = (view.textView.textStorage.attributes(
                  at: originalTargetRange.location,
                  effectiveRange: nil
              )[RenderBridgeAttributes.docPos] as? NSNumber)?.uint32Value
        else {
            XCTFail("expected the target image and its document position")
            return
        }

        let fullAtomicRender = EditorV2Shadow.replaceHtml(
            id: editorId,
            html: """
            <p>Preceding text is now longer</p><img src="https://example.com/target.png" width="140" height="80"><img src="https://example.com/control.png" width="90" height="60"><p></p>
            """
        )
        view.textView.applyUpdateJSON(fullAtomicRender, notifyDelegate: false)
        view.layoutIfNeeded()

        guard let targetRange = firstImageRange(in: view.textView),
              let refreshedDocPos = (view.textView.textStorage.attributes(
                  at: targetRange.location,
                  effectiveRange: nil
              )[RenderBridgeAttributes.docPos] as? NSNumber)?.uint32Value
        else {
            XCTFail("expected the target image after the atomic render")
            return
        }

        XCTAssertNotEqual(
            refreshedDocPos,
            originalDocPos,
            "a preceding extent change must refresh retained atom document positions"
        )

        XCTAssertTrue(view.textView.becomeFirstResponder())
        setSelection(in: view.textView, utf16Range: targetRange)
        flushMainQueue()
        view.layoutIfNeeded()
        view.resizeSelectedImageForTesting(width: 200, height: 100)
        flushMainQueue()

        let html = EditorV2Shadow.getHtml(id: editorId)
        XCTAssertTrue(
            html.contains("target.png\" width=\"200\""),
            "the selected target image must receive the resize action, got: \(html)"
        )
        XCTAssertTrue(
            html.contains("control.png\" width=\"90\""),
            "resizing the target must not affect the adjacent control image, got: \(html)"
        )
    }

    func testPrependingTopLevelChildRefreshesRetainedChildIndexes() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<hr><hr>")

        let fullAtomicRender = EditorV2Shadow.replaceHtml(
            id: editorId,
            html: "<p>Prelude</p><hr><hr>"
        )
        textView.applyUpdateJSON(fullAtomicRender, notifyDelegate: false)

        let horizontalRuleRanges = (0..<textView.textStorage.length).compactMap { index -> NSRange? in
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            return (attrs[RenderBridgeAttributes.voidNodeType] as? String)
                .map(EditorNodeTypes.isHorizontalRule) == true
                ? NSRange(location: index, length: 1)
                : nil
        }
        XCTAssertEqual(horizontalRuleRanges.count, 2)
        guard horizontalRuleRanges.count == 2 else { return }
        XCTAssertEqual(
            (textView.textStorage.attributes(
                at: horizontalRuleRanges[0].location,
                effectiveRange: nil
            )[RenderBridgeAttributes.topLevelChildIndex] as? NSNumber)?.intValue,
            1,
            "the first retained atom must receive its shifted top-level child index"
        )
        XCTAssertEqual(
            (textView.textStorage.attributes(
                at: horizontalRuleRanges[1].location,
                effectiveRange: nil
            )[RenderBridgeAttributes.topLevelChildIndex] as? NSNumber)?.intValue,
            2,
            "every retained sibling after a prepend must receive its shifted index"
        )
    }

    func testExplicitPrependRenderPatchRefreshesRetainedAtomMetadata() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(id: editorId, initialHTML: "<hr><hr>")

        let finalUpdateJSON = EditorV2Shadow.replaceHtml(
            id: editorId,
            html: "<p>Prelude</p><hr><hr>"
        )
        var explicitPatchUpdate = parseJSONObject(finalUpdateJSON)
        let finalRenderBlocks = try XCTUnwrap(
            (explicitPatchUpdate["renderPatch"] as? [String: Any])?["renderBlocks"]
                as? [[[String: Any]]]
        )
        explicitPatchUpdate["renderBlocks"] = finalRenderBlocks
        explicitPatchUpdate["renderPatch"] = [
            "startIndex": 0,
            "deleteCount": 0,
            "renderBlocks": [finalRenderBlocks[0]],
        ]
        let explicitPatchData = try JSONSerialization.data(withJSONObject: explicitPatchUpdate)
        let explicitPatchJSON = try XCTUnwrap(String(data: explicitPatchData, encoding: .utf8))

        textView.applyUpdateJSON(explicitPatchJSON, notifyDelegate: false)

        let expected = RenderBridge.renderBlocks(
            fromArray: finalRenderBlocks,
            baseFont: textView.baseFont,
            textColor: textView.baseTextColor
        )
        let trace = try XCTUnwrap(textView.lastApplyUpdateTrace())
        XCTAssertTrue(
            trace.attemptedPatch,
            "the update must exercise the explicit renderPatch path"
        )
        XCTAssertTrue(
            trace.usedPatch,
            "the explicit prepend must retain the compact renderPatch path"
        )
        XCTAssertEqual(textView.textStorage.string, expected.string, "the prepend must not duplicate content")

        let actualAtomOffsets = (0..<textView.textStorage.length).filter { index in
            textView.textStorage.attributes(at: index, effectiveRange: nil)[RenderBridgeAttributes.voidNodeType] != nil
        }
        let expectedAtomOffsets = (0..<expected.length).filter { index in
            expected.attributes(at: index, effectiveRange: nil)[RenderBridgeAttributes.voidNodeType] != nil
        }
        XCTAssertEqual(actualAtomOffsets.count, 2)
        XCTAssertEqual(actualAtomOffsets.count, expectedAtomOffsets.count)

        for (actualOffset, expectedOffset) in zip(actualAtomOffsets, expectedAtomOffsets) {
            let actualAttributes = textView.textStorage.attributes(at: actualOffset, effectiveRange: nil)
            let expectedAttributes = expected.attributes(at: expectedOffset, effectiveRange: nil)
            XCTAssertEqual(
                (actualAttributes[RenderBridgeAttributes.topLevelChildIndex] as? NSNumber)?.intValue,
                (expectedAttributes[RenderBridgeAttributes.topLevelChildIndex] as? NSNumber)?.intValue
            )
            XCTAssertEqual(
                (actualAttributes[RenderBridgeAttributes.docPos] as? NSNumber)?.uint32Value,
                (expectedAttributes[RenderBridgeAttributes.docPos] as? NSNumber)?.uint32Value
            )
        }
    }

    func testWrongRenderPatchBaseRecoversAFullNativeSnapshot() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else {
            XCTFail("expected adapter")
            return
        }
        _ = EditorV2Shadow.setHtml(id: editorId, html: "<p>Alpha</p>")
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p>")
        let revision = adapter.baseDocumentRevision
        let wrongBase = revision == 0 ? UInt64(1) : UInt64(0)
        let stalePatch: [String: Any] = [
            "documentVersion": String(revision),
            "renderPatch": [
                "baseDocumentVersion": String(wrongBase),
                "startIndex": 0,
                "deleteCount": 1,
                "renderBlocks": [[
                    ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
                    ["type": "textRun", "text": "Corrupt", "marks": []],
                    ["type": "blockEnd"],
                ]],
            ],
        ]
        let staleData = try JSONSerialization.data(withJSONObject: stalePatch)
        let staleJSON = try XCTUnwrap(String(data: staleData, encoding: .utf8))

        XCTAssertTrue(textView.applyUpdateJSON(staleJSON, notifyDelegate: false))
        XCTAssertEqual(textView.textStorage.string, "Alpha")
        XCTAssertFalse(textView.lastRenderAppliedPatch())

        let nextUpdate = EditorV2Shadow.insertTextScalar(
            id: editorId,
            scalarPos: 5,
            text: "!"
        )
        XCTAssertTrue(textView.applyUpdateJSON(nextUpdate, notifyDelegate: false))
        XCTAssertEqual(textView.textStorage.string, "Alpha!")
    }

    func testTypingInsideListItemFallsBackToFullRenderAndPreservesTextOrder() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(
            id: editorId,
            initialHTML: "<ul><li><p>Alpha</p></li><li><p>Beta</p></li></ul>"
        )

        let alphaRange = (textView.text as NSString).range(of: "Alpha")
        XCTAssertNotEqual(alphaRange.location, NSNotFound)
        setCollapsedSelection(in: textView, utf16Offset: alphaRange.location + alphaRange.length)
        flushMainQueue()

        textView.insertText("!")

        XCTAssertFalse(
            textView.lastRenderAppliedPatch(),
            "list items should bypass the top-level render patch path until list marker patching is made safe"
        )
        XCTAssertEqual(textView.textStorage.string, "Alpha!\nBeta")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>Alpha!</p></li><li><p>Beta</p></li></ul>"
        )
    }

    func testReturnInsideListItemFallsBackToFullRenderAndKeepsTypingInNewItem() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 180))
        textView.captureApplyUpdateTraceForTesting = true
        textView.bindEditor(
            id: editorId,
            initialHTML: "<ul><li><p>Alpha</p></li><li><p>Beta</p></li></ul>"
        )

        let alphaRange = (textView.text as NSString).range(of: "Alpha")
        XCTAssertNotEqual(alphaRange.location, NSNotFound)
        setCollapsedSelection(in: textView, utf16Offset: alphaRange.location + alphaRange.length)
        flushMainQueue()

        textView.insertText("\n")

        XCTAssertFalse(
            textView.lastRenderAppliedPatch(),
            "splitting list items should use the full render path to keep caret mapping stable"
        )
        textView.insertText("B")

        XCTAssertEqual(textView.textStorage.string, "Alpha\nB\nBeta")
        XCTAssertEqual(
            EditorV2Shadow.getHtml(id: editorId),
            "<ul><li><p>Alpha</p></li><li><p>B</p></li><li><p>Beta</p></li></ul>"
        )
    }

    func testFullCurrentStateLocalEditUsesSynthesizedTopLevelPatch() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")

        let updatedDocument = """
        {
          "type": "doc",
          "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "Alpha"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "Better"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "Gamma"}]}
          ]
        }
        """
        let update = EditorV2Shadow.setJson(id: editorId, json: updatedDocument)
        textView.applyUpdateJSON(update, notifyDelegate: false)

        XCTAssertTrue(
            textView.lastRenderAppliedPatch(),
            "full current-state updates should synthesize a top-level patch when only a local block range changes"
        )
        XCTAssertEqual(textView.textStorage.string, "Alpha\nBetter\nGamma")
    }

    func testIdenticalFullCurrentStateSkipsNativeTextReapply() throws {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p><p>Gamma</p>")
        textView.captureApplyUpdateTraceForTesting = true

        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)

        let trace = try XCTUnwrap(textView.lastApplyUpdateTrace())
        XCTAssertFalse(textView.lastRenderAppliedPatch())
        XCTAssertEqual(trace.buildRenderNanos, 0)
        XCTAssertEqual(trace.applyRenderNanos, 0)
        XCTAssertEqual(trace.applyRenderTextMutationNanos, 0)
        XCTAssertEqual(textView.textStorage.string, "Alpha\nBeta\nGamma")
    }

    func testRustDrivenSelectionApplyDoesNotNotifySelectionDelegate() {
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }

        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let delegate = EditorTextViewDelegateSpy()
        textView.editorDelegate = delegate
        textView.bindEditor(id: editorId, initialHTML: "<p>Alpha</p><p>Beta</p>")
        delegate.selectionChanges.removeAll()
        delegate.receivedUpdates.removeAll()

        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: 8, scalarHead: 8)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        flushMainQueue()

        XCTAssertEqual(delegate.selectionChanges.count, 0)
        XCTAssertEqual(delegate.receivedUpdates.count, 0)
    }

}
