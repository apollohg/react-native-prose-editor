import CryptoKit
import Foundation

/// The source accepted by both the UIKit and Fabric viewer surfaces.
public enum ProseViewerSource: Hashable {
    case json(String)
    case html(String)

    var kind: FfiViewerSourceKind {
        switch self {
        case .json: .json
        case .html: .html
        }
    }

    var value: String {
        switch self {
        case let .json(value), let .html(value): value
        }
    }
}

/// Immutable configuration for one viewer generation.
public struct ProseViewerConfiguration: Hashable {
    public var configJSON: String
    public var themeJSON: String?
    public var imagePolicyJSON: String?
    public var imagesEnabled: Bool
    public var collapsesWhenEmpty: Bool

    public init(
        configJSON: String = "{}",
        themeJSON: String? = nil,
        imagePolicyJSON: String? = nil,
        imagesEnabled: Bool = true,
        collapsesWhenEmpty: Bool = true
    ) {
        self.configJSON = configJSON
        self.themeJSON = themeJSON
        self.imagePolicyJSON = imagePolicyJSON
        self.imagesEnabled = imagesEnabled
        self.collapsesWhenEmpty = collapsesWhenEmpty
    }
}

struct ProseViewerRequest: Hashable {
    let source: ProseViewerSource
    let configuration: ProseViewerConfiguration
    let nativeFontRevision: UInt64
    let fontEnvironmentRevision: UInt64
    let attachmentRevision: UInt64

    init(
        source: ProseViewerSource,
        configuration: ProseViewerConfiguration,
        nativeFontRevision: UInt64 = 0,
        fontEnvironmentRevision: UInt64 = 0,
        attachmentRevision: UInt64 = 0
    ) {
        self.source = source
        self.configuration = configuration
        self.nativeFontRevision = nativeFontRevision
        self.fontEnvironmentRevision = fontEnvironmentRevision
        self.attachmentRevision = attachmentRevision
    }

    var compiledCacheKey: String {
        let mentionPrefix = Self.mentionPrefix(in: configuration.configJSON) ?? ""
        let input = [
            source.value,
            configuration.configJSON,
            configuration.imagePolicyJSON ?? "",
            configuration.imagesEnabled ? "1" : "0",
            mentionPrefix,
            source.kind == .json ? "json" : "html",
        ].joined(separator: "\u{1F}")
        return SHA256Digest.hex(input)
    }

    var themeDigest: String { SHA256Digest.hex(configuration.themeJSON ?? "") }

    /// Includes every input that makes a mounted generation genuinely different.
    var generationIdentity: String {
        SHA256Digest.hex([
            compiledCacheKey,
            themeDigest,
            configuration.collapsesWhenEmpty ? "1" : "0",
            String(attachmentRevision),
            String(nativeFontRevision),
            String(fontEnvironmentRevision),
        ].joined(separator: "\u{1F}"))
    }

    var mentionPrefix: String? { Self.mentionPrefix(in: configuration.configJSON) }

    private static func mentionPrefix(in json: String) -> String? {
        guard let data = json.data(using: .utf8),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return ((root["mentions"] as? [String: Any])?["prefix"] as? String)
            ?? (root["mentionPrefix"] as? String)
    }
}

public enum ProseViewerError: Error, Equatable {
    case compiler(domain: String, code: String, message: String)
    case hostContract(message: String)
    case layout(message: String)

    var domain: String {
        switch self {
        case let .compiler(domain, _, _): domain
        case .hostContract: "viewer.host"
        case .layout: "viewer.layout"
        }
    }

    var code: String {
        switch self {
        case let .compiler(_, code, _): code
        case .hostContract: "INVALID_WIDTH"
        case .layout: "LAYOUT_FAILED"
        }
    }

    var message: String {
        switch self {
        case let .compiler(_, _, message), let .hostContract(message), let .layout(message): message
        }
    }
}

struct ViewerParagraph: Hashable {
    let text: String
}

/// Width-independent native projection of the Rust compiled document.
struct ViewerDocument: Hashable {
    let semanticKey: String
    let paragraphs: [ViewerParagraph]
    let isEmpty: Bool
    let retainedBytes: Int

    init(semanticKey: String, paragraphs: [ViewerParagraph], isEmpty: Bool, retainedBytes: Int) {
        self.semanticKey = semanticKey
        self.paragraphs = paragraphs
        self.isEmpty = isEmpty
        self.retainedBytes = retainedBytes
    }

    init(compiled: ViewerCompiledDocument) throws {
        semanticKey = compiled.semanticKey()
        isEmpty = compiled.isEmpty()
        retainedBytes = Int(compiled.retainedBytesDecimal()) ?? 0

        var paragraphs: [ViewerParagraph] = []
        var text = ""
        var isInBlock = false
        for element in compiled.elements() {
            switch element {
            case .blockStart:
                if isInBlock { paragraphs.append(ViewerParagraph(text: text)); text = "" }
                isInBlock = true
            case let .textRun(text: value, marks: _):
                text.append(value)
            case let .inlineAtom(nodeType: _, docPos: _, attrsJson: _, label: label),
                 let .blockAtom(nodeType: _, docPos: _, attrsJson: _, label: label):
                text.append(label)
            case .blockEnd:
                paragraphs.append(ViewerParagraph(text: text))
                text = ""
                isInBlock = false
            }
        }
        if isInBlock || (!text.isEmpty && paragraphs.isEmpty) {
            paragraphs.append(ViewerParagraph(text: text))
        }
        self.paragraphs = paragraphs.isEmpty ? [ViewerParagraph(text: "")] : paragraphs
    }
}

private enum SHA256Digest {
    static func hex(_ value: String) -> String {
        let data = Data(value.utf8)
        return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }
}
