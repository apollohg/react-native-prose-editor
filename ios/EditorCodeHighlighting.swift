import CoreText
import UIKit

struct NativeCodeHighlightConfiguration: Equatable {
    let provider: String
    let theme: String

    static func from(_ value: Any?) -> NativeCodeHighlightConfiguration? {
        guard let values = value as? [String: Any], let provider = values["provider"] as? String,
              let theme = values["theme"] as? String, !provider.isEmpty, !theme.isEmpty else { return nil }
        return NativeCodeHighlightConfiguration(provider: provider, theme: theme)
    }
}

let editorCodeBlockAttribute = NSAttributedString.Key("com.apollohg.editor.codeBlock")
private let editorSyntaxOwnedAttribute = NSAttributedString.Key("com.apollohg.editor.syntaxOwned")

final class EditorCodeBlockPresentation: NSObject {
    let language: String?
    init(language: String?) { self.language = language }
}

private final class EditorSyntaxOwnedAttributes: NSObject {
    let values: [NSAttributedString.Key: Any]
    init(_ values: [NSAttributedString.Key: Any]) { self.values = values }
}

enum NativeCodeHighlightPresentation {
    static func apply(_ ranges: [NativeCodeHighlightRange], to string: NSMutableAttributedString, offset: Int = 0, owned: Bool = false) {
        for token in ranges {
            let range = NSRange(location: offset + token.start, length: token.length)
            guard range.location >= 0, NSMaxRange(range) <= string.length else { continue }
            string.enumerateAttributes(in: range) { original, piece, _ in
                var attributes: [NSAttributedString.Key: Any] = [:]
                let color = UIColor(red: CGFloat(token.color >> 24) / 255,
                                    green: CGFloat((token.color >> 16) & 255) / 255,
                                    blue: CGFloat((token.color >> 8) & 255) / 255,
                                    alpha: CGFloat(token.color & 255) / 255)
                attributes[.foregroundColor] = color
                attributes[kCTForegroundColorAttributeName as NSAttributedString.Key] = color.cgColor
                let font = original[.font] as? UIFont ?? .systemFont(ofSize: 16)
                var traits = font.fontDescriptor.symbolicTraits
                if token.fontStyle & 1 != 0 { traits.insert(.traitBold) }
                if token.fontStyle & 4 != 0 { traits.insert(.traitItalic) }
                let styled = ViewerFontEnvironment.shared.resolveFont(family: nil, size: font.pointSize, fallback: font, additionalTraits: traits, semanticGeneration: "native-code-highlighting")
                attributes[.font] = styled
                attributes[kCTFontAttributeName as NSAttributedString.Key] = CoreTextProseLayoutEngine.coreTextFont(from: styled)
                if token.fontStyle & 2 != 0 {
                    attributes[.underlineStyle] = NSUnderlineStyle.single.rawValue
                    attributes[kCTUnderlineStyleAttributeName as NSAttributedString.Key] = NSUnderlineStyle.single.rawValue
                }
                if owned {
                    var base: [NSAttributedString.Key: Any] = [:]
                    for key in attributes.keys { base[key] = original[key] ?? NSNull() }
                    attributes[editorSyntaxOwnedAttribute] = EditorSyntaxOwnedAttributes(base)
                }
                string.addAttributes(attributes, range: piece)
            }
        }
    }
}

extension EditorTextView {
    func restoreCodeHighlighting() {
        guard textStorage.length > 0 else { return }
        let wasApplying = isApplyingRustState
        isApplyingRustState = true
        textStorage.beginEditing()
        textStorage.enumerateAttribute(editorSyntaxOwnedAttribute, in: NSRange(location: 0, length: textStorage.length)) { value, range, _ in
            guard let original = value as? EditorSyntaxOwnedAttributes else { return }
            for (key, value) in original.values {
                if value is NSNull { textStorage.removeAttribute(key, range: range) }
                else { textStorage.addAttribute(key, value: value, range: range) }
            }
            textStorage.removeAttribute(editorSyntaxOwnedAttribute, range: range)
        }
        textStorage.endEditing()
        isApplyingRustState = wasApplying
        invalidateAutoGrowHeightMeasurement()
        setNeedsLayout()
        setNeedsDisplay()
    }

    func scheduleCodeHighlighting() {
        codeHighlightingSession.cancel()
        guard let configuration = codeHighlighting else { return }
        do { _ = try NativeCodeHighlightingRegistry.provider(id: configuration.provider) }
        catch { onCodeHighlightingError?(error); return }
        guard markedTextRange == nil, textStorage.length > 0 else { return }
        restoreCodeHighlighting()
        let text = textStorage.string
        let binding = editorId
        var blocks: [NativeCodeHighlightBlock] = []
        textStorage.enumerateAttribute(editorCodeBlockAttribute, in: NSRange(location: 0, length: textStorage.length)) { value, range, _ in
            guard let presentation = value as? EditorCodeBlockPresentation else { return }
            var range = range
            while range.length > 0, textStorage.attribute(RenderBridgeAttributes.syntheticPlaceholder, at: NSMaxRange(range) - 1, effectiveRange: nil) != nil { range.length -= 1 }
            blocks.append(NativeCodeHighlightBlock(start: range.location, text: (text as NSString).substring(with: range), language: presentation.language))
        }
        guard !blocks.isEmpty else { return }
        do {
            try codeHighlightingSession.update(provider: configuration.provider, theme: configuration.theme, blocks: blocks) { [weak self] result in
                guard let self, self.codeHighlighting == configuration, self.editorId == binding,
                      self.markedTextRange == nil, self.textStorage.string == text else { return }
                switch result {
                case let .failure(error): self.onCodeHighlightingError?(error)
                case let .success(output):
                    let wasApplying = self.isApplyingRustState
                    self.isApplyingRustState = true
                    self.textStorage.beginEditing()
                    for block in output { NativeCodeHighlightPresentation.apply(block.ranges, to: self.textStorage, offset: block.block.start, owned: true) }
                    self.textStorage.endEditing()
                    self.isApplyingRustState = wasApplying
                    self.invalidateAutoGrowHeightMeasurement()
                    self.setNeedsLayout()
                    self.setNeedsDisplay()
                }
            }
        } catch { onCodeHighlightingError?(error) }
    }
}

struct PreparedViewerHighlightingRequest {
    let configuration: NativeCodeHighlightConfiguration
    let generation: String
    let blocks: [NativeCodeHighlightBlock]
    var retainedBytes: Int { blocks.reduce(128) { $0 + $1.text.utf16.count * 2 + 64 } }
}

final class PreparedViewerHighlightingResult: NSObject {
    let ranges: [Int: [NativeCodeHighlightRange]]
    init(_ output: [NativeHighlightedCodeBlock]) {
        let count = output.reduce(0) { $0 + $1.ranges.count }
        ranges = count <= 100_000 ? Dictionary(uniqueKeysWithValues: output.map { ($0.block.start, $0.ranges) }) : [:]
    }
    var retainedBytes: Int { ranges.values.reduce(128) { $0 + $1.count * 40 } }
}

enum PreparedViewerHighlightingStore {
    private static let cache: NSCache<NSString, PreparedViewerHighlightingResult> = {
        let cache = NSCache<NSString, PreparedViewerHighlightingResult>()
        cache.countLimit = 32
        cache.totalCostLimit = 4 * 1024 * 1024
        return cache
    }()
    static func result(for generation: String) -> PreparedViewerHighlightingResult? { cache.object(forKey: generation as NSString) }
    static func publish(_ result: PreparedViewerHighlightingResult, generation: String) {
        cache.setObject(result, forKey: generation as NSString, cost: result.retainedBytes)
    }
}

extension PreparedProseDrawingView {
    func scheduleCodeHighlighting() {
        codeHighlightingSession.cancel()
        guard let layout, let request = layout.highlightingRequest, !layout.highlightingResolved else { return }
        if PreparedViewerHighlightingStore.result(for: request.generation) != nil {
            DispatchQueue.main.async { [weak self, weak layout] in
                guard let self, let layout, self.layout === layout else { return }
                self.publishCodeHighlightingResolution(request.generation)
            }
            return
        }
        do {
            try codeHighlightingSession.update(provider: request.configuration.provider, theme: request.configuration.theme, blocks: request.blocks) { [weak self, weak layout] result in
                guard let self, let layout, self.layout === layout else { return }
                switch result {
                case let .success(output):
                    PreparedViewerHighlightingStore.publish(PreparedViewerHighlightingResult(output), generation: request.generation)
                    self.publishCodeHighlightingResolution(request.generation)
                case let .failure(error): self.reportCodeHighlightingError(error, generation: request.generation)
                }
            }
        } catch { reportCodeHighlightingError(error, generation: request.generation) }
    }

    private func publishCodeHighlightingResolution(_ generation: String) {
        onCodeHighlightingResolved?(generation)
        NotificationCenter.default.post(name: Self.codeHighlightingDidResolve, object: self, userInfo: ["generation": generation])
    }

    private func reportCodeHighlightingError(_ error: Error, generation: String) {
        PreparedViewerHighlightingStore.publish(PreparedViewerHighlightingResult([]), generation: generation)
        onCodeHighlightingFailure?(error)
        NotificationCenter.default.post(name: Self.codeHighlightingDidFail, object: self, userInfo: ["generation": generation, "message": error.localizedDescription])
    }
}
