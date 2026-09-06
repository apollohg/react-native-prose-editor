import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testTaskMarkerHitTest_hitsCheckboxCenterOfSingleTaskItem() {
        let attributed = RenderBridge.renderElements(
            fromJSON: taskListJSON(items: [(text: "Buy milk", checked: false)]),
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 120))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let markerRect = taskMarkerTightRect(forCharacterIndex: 0, in: textView)
        let point = CGPoint(x: markerRect.midX, y: markerRect.midY)

        XCTAssertTrue(
            textView.hasTaskListMarker(at: point),
            "tapping the checkbox center of the only task item must register a hit. markerRect=\(markerRect)"
        )
    }

    func testTaskMarkerHitTest_missesTapFarFromAnyMarker() {
        let attributed = RenderBridge.renderElements(
            fromJSON: taskListJSON(items: [(text: "Buy milk", checked: false)]),
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 120))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let farPoint = CGPoint(x: 220, y: 300)

        XCTAssertFalse(
            textView.hasTaskListMarker(at: farPoint),
            "tapping far outside every marker's tap-slop rect must not register a hit"
        )
    }

    func testTaskMarkerHitTest_hitsRealItemStartButMissesHardBreakContinuationLine() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true, "kind": "task", "checked": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Line one", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 8},
            {"type": "textRun", "text": "Line two", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 160))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let nsString = attributed.string as NSString
        XCTAssertEqual(nsString as String, "Line one\nLine two")

        let realStart = 0
        let hardBreakContinuationStart = nsString.range(of: "Line two").location
        XCTAssertGreaterThan(hardBreakContinuationStart, realStart)

        let realStartMarkerRect = taskMarkerTightRect(forCharacterIndex: realStart, in: textView)
        XCTAssertTrue(
            textView.hasTaskListMarker(at: CGPoint(x: realStartMarkerRect.midX, y: realStartMarkerRect.midY)),
            "the true task-item paragraph start must still register a hit. markerRect=\(realStartMarkerRect)"
        )

        // The checkbox's tap-slop (dy: -8) is intentionally taller than one
        // line's pitch (that generosity is what the straddle tests below
        // cover), so a point near the hard-break continuation line
        // legitimately still resolves to the REAL item's marker via slop.
        // A bare hasTaskListMarker(_:) Bool can't distinguish "correctly
        // matched the real marker via slop" from "incorrectly manufactured
        // a phantom marker for the hard-break line" — assert on the
        // resolved paragraph identity instead.
        let continuationLineRect = taskListLineFragmentRect(forCharacterIndex: hardBreakContinuationStart, in: textView)
        let continuationProbe = CGPoint(x: realStartMarkerRect.midX, y: continuationLineRect.midY)
        XCTAssertEqual(
            textView.taskListMarkerParagraphStartForTesting(at: continuationProbe),
            realStart,
            """
            a paragraph start created by a hard break must never be resolved \
            as its own distinct task-item start (paragraphStart=\(hardBreakContinuationStart)) \
            — any match at this position must be attributed to the real \
            item start (paragraphStart=\(realStart)). \
            continuationLineRect=\(continuationLineRect) probe=\(continuationProbe)
            """
        )
    }

    /// Behavior-pinning test: with the OLD whole-document scan this already
    /// passes. It exists to guard the point-first rewrite, which must keep
    /// resolving the touched line ONLY — never falling back to matching
    /// some other task item's marker rect elsewhere in the document.
    func testTaskMarkerHitTest_missOnPlainLineAmongManyTaskItems() {
        let taskItems = (0..<200).map { (text: "Task \($0)", checked: false) }
        var json = taskListJSON(items: taskItems)
        json.removeLast() // drop the closing "]"
        json += """
        ,
        {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
        {"type": "textRun", "text": "Just a plain paragraph", "marks": []},
        {"type": "blockEnd"}
        ]
        """

        let attributed = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 6000))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let nsString = attributed.string as NSString
        let plainParagraphStart = nsString.range(of: "Just a plain paragraph").location
        XCTAssertGreaterThan(plainParagraphStart, 0)

        let plainLineRect = taskListLineFragmentRect(forCharacterIndex: plainParagraphStart, in: textView)
        // Tap over the plain paragraph's leading edge, exactly where a task
        // marker WOULD be drawn if this line were a task item.
        let probe = CGPoint(x: plainLineRect.minX - 20, y: plainLineRect.midY)

        XCTAssertFalse(
            textView.hasTaskListMarker(at: probe),
            """
            the tapped line is a plain paragraph, not a task item — it must \
            miss even though 200 other lines in the document ARE task \
            items. probe=\(probe) plainLineRect=\(plainLineRect)
            """
        )
    }

    /// Caveat coverage: the tap-slop inset (`insetBy(dx: -10, dy: -8)`) can
    /// be taller than the line pitch, so a point still inside a marker's
    /// slop zone can glyph-resolve, via point-first lookup, to the
    /// PREVIOUS task item's line. The implementation must probe
    /// point.y - 8 (in addition to the primary lookup) to still find that
    /// marker instead of missing outright.
    func testTaskMarkerHitTest_tapSlopAboveMarkerStillHitsWhenGlyphLookupLandsOnPreviousLine() {
        let attributed = RenderBridge.renderElements(
            fromJSON: taskListJSON(items: [
                (text: "Alpha", checked: false),
                (text: "Bravo", checked: false),
                (text: "Charlie", checked: false),
            ]),
            // A small font keeps line pitch well under the ~24pt checkbox
            // height, guaranteeing the slop zone bleeds into neighboring
            // lines.
            baseFont: .systemFont(ofSize: 8),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 200))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let nsString = attributed.string as NSString
        let bravoStart = nsString.range(of: "Bravo").location
        XCTAssertGreaterThan(bravoStart, 0)

        let bravoMarkerRect = taskMarkerTightRect(forCharacterIndex: bravoStart, in: textView)
        let slopRect = bravoMarkerRect.insetBy(dx: -10, dy: -8)
        let probe = CGPoint(x: slopRect.midX, y: slopRect.minY + 1)

        let resolvedParagraphStart = taskListParagraphStart(forGlyphResolving: probe, in: textView)
        XCTAssertNotEqual(
            resolvedParagraphStart,
            bravoStart,
            """
            test setup invalid: probe must glyph-resolve to a DIFFERENT \
            line than Bravo's to exercise the straddling-inset caveat. \
            resolvedParagraphStart=\(resolvedParagraphStart) bravoStart=\(bravoStart) \
            slopRect=\(slopRect) probe=\(probe)
            """
        )

        XCTAssertTrue(
            textView.hasTaskListMarker(at: probe),
            """
            probe is inside Bravo's tap-slop rect \(slopRect) even though it \
            glyph-resolves to a different line \
            (resolvedParagraphStart=\(resolvedParagraphStart)) — the \
            point-first hit test must still find Bravo's marker by probing \
            point.y +/- 8. probe=\(probe)
            """
        )
    }

    /// Symmetric to the above: a point inside a marker's slop zone that
    /// glyph-resolves to the NEXT task item's line must still hit, via a
    /// point.y + 8 probe.
    func testTaskMarkerHitTest_tapSlopBelowMarkerStillHitsWhenGlyphLookupLandsOnNextLine() {
        let attributed = RenderBridge.renderElements(
            fromJSON: taskListJSON(items: [
                (text: "Alpha", checked: false),
                (text: "Bravo", checked: false),
                (text: "Charlie", checked: false),
            ]),
            baseFont: .systemFont(ofSize: 8),
            textColor: .label
        )
        let textView = EditorTextView(frame: CGRect(x: 0, y: 0, width: 240, height: 200))
        textView.attributedText = attributed
        textView.layoutIfNeeded()

        let nsString = attributed.string as NSString
        let bravoStart = nsString.range(of: "Bravo").location
        XCTAssertGreaterThan(bravoStart, 0)

        let bravoMarkerRect = taskMarkerTightRect(forCharacterIndex: bravoStart, in: textView)
        let slopRect = bravoMarkerRect.insetBy(dx: -10, dy: -8)
        let probe = CGPoint(x: slopRect.midX, y: slopRect.maxY - 1)

        let resolvedParagraphStart = taskListParagraphStart(forGlyphResolving: probe, in: textView)
        XCTAssertNotEqual(
            resolvedParagraphStart,
            bravoStart,
            """
            test setup invalid: probe must glyph-resolve to a DIFFERENT \
            line than Bravo's to exercise the straddling-inset caveat. \
            resolvedParagraphStart=\(resolvedParagraphStart) bravoStart=\(bravoStart) \
            slopRect=\(slopRect) probe=\(probe)
            """
        )

        XCTAssertTrue(
            textView.hasTaskListMarker(at: probe),
            """
            probe is inside Bravo's tap-slop rect \(slopRect) even though it \
            glyph-resolves to a different line \
            (resolvedParagraphStart=\(resolvedParagraphStart)) — the \
            point-first hit test must still find Bravo's marker by probing \
            point.y +/- 8. probe=\(probe)
            """
        )
    }

}
