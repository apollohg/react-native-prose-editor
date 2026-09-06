import CoreText
import XCTest
import UIKit

final class EditorHighlightingTests: XCTestCase {
    func testMissingProviderReportsErrorBeforeCodeContentExists() {
        let view = EditorTextView(frame: .zero)
        var failure: Error?
        view.onCodeHighlightingError = { failure = $0 }
        view.codeHighlighting = NativeCodeHighlightConfiguration(provider: "missing-native-fixture", theme: "fixture")
        XCTAssertEqual(view.codeHighlighting?.provider, "missing-native-fixture")
        XCTAssertThrowsError(try NativeCodeHighlightingRegistry.provider(id: "missing-native-fixture"))
        XCTAssertNotNil(failure)
    }

    func testEditorAppliesAndRemovesPresentationWithoutChangingText() throws {
        let provider = EditorHighlightingFixtureProvider()
        try NativeCodeHighlightingRegistry.register(provider: provider)
        let view = EditorTextView(frame: CGRect(x: 0, y: 0, width: 320, height: 180))
        view.codeHighlighting = NativeCodeHighlightConfiguration(provider: provider.id, theme: "fixture")
        view.applyRenderJSON("""
        [{"type":"blockStart","nodeType":"codeBlock","language":"rust","depth":0},{"type":"textRun","text":"let answer = 42;","marks":[]},{"type":"blockEnd"}]
        """)
        let original = view.textStorage.string
        let painted = expectation(for: NSPredicate { _, _ in
            (view.textStorage.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? UIColor) == UIColor.red
        }, evaluatedWith: view)
        wait(for: [painted], timeout: 3)
        XCTAssertEqual(view.textStorage.string, original)
        XCTAssertEqual(provider.language, "rust")
        view.codeHighlighting = nil
        XCTAssertNotEqual(view.textStorage.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? UIColor, UIColor.red)
        XCTAssertEqual(view.textStorage.string, original)
    }
}

private final class EditorHighlightingFixtureProvider: NativeCodeHighlightingProvider {
    let id = "editor-styles-fixture"
    let version = 1
    var language: String?
    func highlight(text: String, language: String?, theme: String) throws -> [NativeCodeHighlightRange] {
        self.language = language
        XCTAssertFalse(Thread.isMainThread)
        return text.isEmpty ? [] : [NativeCodeHighlightRange(start: 0, length: min(3, text.utf16.count), color: 0xff0000ff, fontStyle: 1)]
    }
}

extension EditorHighlightingTests {
    func testViewerRepreparesImmutableLayoutAfterWorkerPublication() throws {
        let provider = ViewerHighlightingFixtureProvider()
        try NativeCodeHighlightingRegistry.register(provider: provider)
        var theme = PreparedProseTheme.resolve(themeJSON: nil)
        theme.codeHighlighting = NativeCodeHighlightConfiguration(provider: provider.id, theme: "fixture")
        let document = ViewerDocument(semanticKey: "highlight-viewer", blocks: [
            ViewerBlock(nodeType: "codeBlock", depth: 0, inBlockquote: false, listContext: nil, listItemBoundary: nil, inlines: [.text(text: "let answer = 42;", marks: [])], language: "rust")
        ], isEmpty: false, retainedBytes: 0, preparedTheme: theme)
        let key = ProseLayoutKey(semanticKey: "highlight-viewer", widthPixels: 320, themeDigest: "fixture", nativeFontRevision: 0, fontEnvironmentRevision: 0, displayScale: 1, attachmentRevision: 0, generationIdentity: "highlight-viewer", semanticGenerationIdentity: UUID().uuidString)
        let engine = CoreTextProseLayoutEngine()
        let plain = try engine.prepare(document: document, key: key, widthPoints: 320, displayScale: 1)
        let view = PreparedProseDrawingView(frame: CGRect(origin: .zero, size: plain.size))
        let ready = expectation(forNotification: PreparedProseDrawingView.codeHighlightingDidResolve, object: view)
        view.install(layout: plain)
        wait(for: [ready], timeout: 3)
        let remounted = PreparedProseDrawingView(frame: view.frame)
        let refreshed = expectation(forNotification: PreparedProseDrawingView.codeHighlightingDidResolve, object: remounted)
        remounted.install(layout: plain)
        wait(for: [refreshed], timeout: 3)
        let highlighted = try engine.prepare(document: document, key: key, widthPoints: 320, displayScale: 1)
        let line = try XCTUnwrap(highlighted.blocks.flatMap(\.fragments).first { $0.kind == .text }?.line)
        let run = try XCTUnwrap((CTLineGetGlyphRuns(line) as? [CTRun])?.first)
        let attributes = CTRunGetAttributes(run) as NSDictionary
        XCTAssertEqual(attributes[kCTForegroundColorAttributeName] as! CGColor, UIColor.red.cgColor)
        XCTAssertFalse(plain === highlighted)
        let alreadyPainted = expectation(forNotification: PreparedProseDrawingView.codeHighlightingDidResolve, object: remounted)
        alreadyPainted.isInverted = true
        remounted.install(layout: highlighted)
        wait(for: [alreadyPainted], timeout: 0.1)
        remounted.install(layout: nil)
        view.install(layout: nil)
    }
}

private final class ViewerHighlightingFixtureProvider: NativeCodeHighlightingProvider {
    let id = "viewer-styles-fixture"
    let version = 1
    func highlight(text: String, language: String?, theme: String) throws -> [NativeCodeHighlightRange] {
        XCTAssertFalse(Thread.isMainThread)
        return [NativeCodeHighlightRange(start: 0, length: 3, color: 0xff0000ff, fontStyle: 1)]
    }
}

extension EditorHighlightingTests {
    func testProviderFontStyleBitsHaveIndependentMeanings() throws {
        for bits: UInt8 in [1, 2, 4] {
            let text = NSMutableAttributedString(string: "x", attributes: [.font: UIFont.monospacedSystemFont(ofSize: 16, weight: .regular)])
            NativeCodeHighlightPresentation.apply([.init(start: 0, length: 1, color: 0xff0000ff, fontStyle: bits)], to: text)
            let font = try XCTUnwrap(text.attribute(.font, at: 0, effectiveRange: nil) as? UIFont)
            XCTAssertEqual(font.fontDescriptor.symbolicTraits.contains(.traitBold), bits == 1)
            XCTAssertEqual(font.fontDescriptor.symbolicTraits.contains(.traitItalic), bits == 4)
            XCTAssertEqual(text.attribute(.underlineStyle, at: 0, effectiveRange: nil) != nil, bits == 2)
        }
    }
}
