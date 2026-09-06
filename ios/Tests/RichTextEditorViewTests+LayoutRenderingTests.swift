import XCTest
import ExpoModulesCore

final class AutoGrowStyleTrackingNativeEditorView: NativeEditorExpoView {
    var publishedStyleHeights: [CGFloat?] = []

    required init(appContext: AppContext? = nil) {
        super.init(appContext: appContext)
    }

    override func setStyleSize(_ width: NSNumber?, height: NSNumber?) {
        publishedStyleHeights.append(height.map { CGFloat($0.doubleValue) })
        super.setStyleSize(width, height: height)
    }
}

extension RichTextEditorViewTests {

    //
    // taskListMarkerParagraphStart (EditorLayoutManager) used to enumerate
    // listMarkerContext over the WHOLE document, with per-item TextKit
    // queries, on every touch (it backs
    // TaskListMarkerTapOverlayView.point(inside:)). These tests pin its
    // hit/miss/hard-break contract before inverting it to resolve the
    // touched line first via glyphIndex(for:in:).

    func taskListJSON(items: [(text: String, checked: Bool)]) -> String {
        let total = items.count
        var elements: [String] = []
        for (index, item) in items.enumerated() {
            elements.append("""
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": \(index + 1), "total": \(total), \
            "start": 1, "isFirst": \(index == 0), "isLast": \(index == total - 1), \
            "kind": "task", "checked": \(item.checked)}}
            """)
            elements.append(#"{"type": "blockStart", "nodeType": "paragraph", "depth": 2}"#)
            elements.append(#"{"type": "textRun", "text": "\#(item.text)", "marks": []}"#)
            elements.append(#"{"type": "blockEnd"}"#)
            elements.append(#"{"type": "blockEnd"}"#)
        }
        return "[\n" + elements.joined(separator: ",\n") + "\n]"
    }

    private func taskListMarkerOrigin(for textView: EditorTextView) -> CGPoint {
        CGPoint(
            x: textView.textContainerInset.left - textView.contentOffset.x,
            y: textView.textContainerInset.top - textView.contentOffset.y
        )
    }

    /// Reproduces the exact marker-rect math taskListMarkerParagraphStart
    /// applies (before the `insetBy(dx: -10, dy: -8)` tap-slop expansion),
    /// so tests can derive precise probe points instead of guessing pixels.
    func taskMarkerTightRect(forCharacterIndex characterIndex: Int, in textView: EditorTextView) -> CGRect {
        guard let layoutManager = textView.layoutManager as? EditorLayoutManager else {
            XCTFail("EditorTextView must be backed by EditorLayoutManager")
            return .zero
        }
        let textStorage = textView.textStorage
        let origin = taskListMarkerOrigin(for: textView)
        let glyphIndex = layoutManager.glyphIndexForCharacter(at: characterIndex)
        let attrs = textStorage.attributes(at: characterIndex, effectiveRange: nil)
        let baseFont = EditorLayoutManager.markerBaseFont(from: attrs)
        let markerWidth = (attrs[RenderBridgeAttributes.listMarkerWidth] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? LayoutConstants.listMarkerWidth

        var lineGlyphRange = NSRange()
        let usedRect = layoutManager.lineFragmentUsedRect(forGlyphAt: glyphIndex, effectiveRange: &lineGlyphRange)
        let lineFragmentRect = layoutManager.lineFragmentRect(forGlyphAt: glyphIndex, effectiveRange: nil)
        let glyphLocation = layoutManager.location(forGlyphAt: glyphIndex)
        let baselineY = lineFragmentRect.minY + glyphLocation.y

        return EditorLayoutManager.taskMarkerDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: markerWidth,
            baselineY: baselineY,
            baseFont: baseFont,
            origin: origin
        )
    }

    func taskListLineFragmentRect(forCharacterIndex characterIndex: Int, in textView: EditorTextView) -> CGRect {
        let layoutManager = textView.layoutManager
        let origin = taskListMarkerOrigin(for: textView)
        let glyphIndex = layoutManager.glyphIndexForCharacter(at: characterIndex)
        var rect = layoutManager.lineFragmentRect(forGlyphAt: glyphIndex, effectiveRange: nil)
        rect.origin.x += origin.x
        rect.origin.y += origin.y
        return rect
    }

    /// Mirrors the point-first glyph resolution the new implementation
    /// performs, so a test can assert which paragraph a probe point
    /// naturally resolves to (independent of any marker-rect matching).
    func taskListParagraphStart(forGlyphResolving point: CGPoint, in textView: EditorTextView) -> Int {
        let layoutManager = textView.layoutManager
        let origin = taskListMarkerOrigin(for: textView)
        let containerPoint = CGPoint(x: point.x - origin.x, y: point.y - origin.y)
        let glyphIndex = layoutManager.glyphIndex(for: containerPoint, in: textView.textContainer)
        let charIndex = layoutManager.characterIndexForGlyph(at: glyphIndex)
        let nsString = textView.textStorage.string as NSString
        return nsString.paragraphRange(for: NSRange(location: charIndex, length: 0)).location
    }

    enum CharacterEdge {
        case leading
        case trailing
    }

    func expectedCaretRectForCharacterEdge(
        in textView: UITextView,
        characterIndex: Int,
        edge: CharacterEdge,
        font: UIFont
    ) -> CGRect {
        guard let rect = visibleCharacterRect(in: textView, characterIndex: characterIndex) else {
            XCTFail("expected visible rect for character index \(characterIndex)")
            return .zero
        }
        guard let baselineY = baselineYForCharacter(in: textView, characterIndex: characterIndex) else {
            XCTFail("expected baseline for character index \(characterIndex)")
            return .zero
        }

        let referenceRect = CGRect(
            x: edge == .leading ? rect.minX : rect.maxX,
            y: rect.minY,
            width: 2,
            height: rect.height
        )
        return EditorTextView.adjustedCaretRect(
            from: referenceRect,
            baselineY: baselineY,
            font: font,
            screenScale: 2
        )
    }

    private func baselineYForCharacter(
        in textView: UITextView,
        characterIndex: Int
    ) -> CGFloat? {
        guard characterIndex >= 0, characterIndex < textView.attributedText.length else { return nil }
        let glyphIndex = textView.layoutManager.glyphIndexForCharacter(at: characterIndex)
        guard glyphIndex < textView.layoutManager.numberOfGlyphs else { return nil }

        let lineFragmentRect = textView.layoutManager.lineFragmentRect(
            forGlyphAt: glyphIndex,
            effectiveRange: nil
        )
        let glyphLocation = textView.layoutManager.location(forGlyphAt: glyphIndex)
        return textView.textContainerInset.top + lineFragmentRect.minY + glyphLocation.y
    }

    func previousVisibleCharacterIndex(
        before utf16Offset: Int,
        in textView: UITextView
    ) -> Int? {
        let text = textView.textStorage.string as NSString
        guard text.length > 0 else { return nil }

        var index = min(utf16Offset - 1, text.length - 1)
        while index >= 0 {
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            let character = text.substring(with: NSRange(location: index, length: 1))
            if attrs[.attachment] == nil,
               character != "\n",
               character != "\r",
               visibleCharacterRect(in: textView, characterIndex: index) != nil
            {
                return index
            }
            index -= 1
        }

        return nil
    }

    func nextVisibleCharacterIndex(
        after utf16Offset: Int,
        in textView: UITextView
    ) -> Int? {
        let text = textView.textStorage.string as NSString
        guard text.length > 0 else { return nil }

        var index = max(utf16Offset, 0)
        while index < text.length {
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            let character = text.substring(with: NSRange(location: index, length: 1))
            if attrs[.attachment] == nil,
               character != "\n",
               character != "\r",
               visibleCharacterRect(in: textView, characterIndex: index) != nil
            {
                return index
            }
            index += 1
        }

        return nil
    }

    private func visibleCharacterRect(
        in textView: UITextView,
        characterIndex: Int
    ) -> CGRect? {
        guard characterIndex >= 0, characterIndex < textView.textStorage.length else { return nil }
        guard let start = textView.position(from: textView.beginningOfDocument, offset: characterIndex),
              let end = textView.position(from: start, offset: 1),
              let range = textView.textRange(from: start, to: end)
        else {
            return nil
        }

        return textView.selectionRects(for: range)
            .map(\.rect)
            .first(where: { !$0.isEmpty && $0.width > 0 && $0.height > 0 })
    }

    func firstHorizontalRuleRange(in textView: UITextView) -> NSRange? {
        guard textView.textStorage.length > 0 else { return nil }

        for index in 0..<textView.textStorage.length {
            let attrs = textView.textStorage.attributes(at: index, effectiveRange: nil)
            if attrs[.attachment] is NSTextAttachment,
               (attrs[RenderBridgeAttributes.voidNodeType] as? String)
                .map(EditorNodeTypes.isHorizontalRule) == true
            {
                return NSRange(location: index, length: 1)
            }
        }

        return nil
    }

    func forceDraw(_ textView: EditorTextView) {
        let renderer = UIGraphicsImageRenderer(bounds: textView.bounds)
        _ = renderer.image { context in
            textView.layer.render(in: context.cgContext)
        }
    }

}
