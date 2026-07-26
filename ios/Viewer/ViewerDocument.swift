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
    /// Captured by Fabric with its native-font revision. UIKit obtains the
    /// same value from `ViewerFontEnvironment` when it creates a request.
    let nativeFontScale: CGFloat
    let fontEnvironmentRevision: UInt64
    let attachmentRevision: UInt64

    init(
        source: ProseViewerSource,
        configuration: ProseViewerConfiguration,
        nativeFontRevision: UInt64 = 0,
        nativeFontScale: CGFloat = 1,
        fontEnvironmentRevision: UInt64 = 0,
        attachmentRevision: UInt64 = 0
    ) {
        self.source = source
        self.configuration = configuration
        self.nativeFontRevision = nativeFontRevision
        self.nativeFontScale = nativeFontScale.isFinite && nativeFontScale > 0 ? nativeFontScale : 1
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

    /// Canonical publication identity. State-only layout/font revisions are
    /// intentionally excluded so cancellation/reinstall cannot reopen image
    /// metadata or resource-error publication for the same semantic source.
    var semanticGenerationIdentity: String {
        SHA256Digest.hex([
            source.kind == .json ? "json" : "html",
            source.value,
            configuration.configJSON,
            configuration.themeJSON ?? "",
            configuration.imagePolicyJSON ?? "",
            configuration.imagesEnabled ? "1" : "0",
            configuration.collapsesWhenEmpty ? "1" : "0",
            mentionPrefix ?? "",
        ].joined(separator: "\u{1F}"))
    }

    /// Includes the semantic generation plus the permitted state-only layout
    /// revisions. This remains the immutable layout/cache identity.
    var generationIdentity: String {
        SHA256Digest.hex([
            semanticGenerationIdentity,
            String(attachmentRevision),
            String(nativeFontRevision),
            String(Double(nativeFontScale).bitPattern),
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
    case resource

    var domain: String {
        switch self {
        case let .compiler(domain, _, _): domain
        case .hostContract: "viewer.host"
        case .layout: "viewer.layout"
        case .resource: "viewer.resource"
        }
    }

    var code: String {
        switch self {
        case let .compiler(_, code, _): code
        case .hostContract: "INVALID_WIDTH"
        case .layout: "LAYOUT_FAILED"
        case .resource: "RESOURCE_LOAD_FAILED"
        }
    }

    var message: String {
        switch self {
        case let .compiler(_, _, message), let .hostContract(message), let .layout(message): message
        case .resource: "An image resource could not be loaded."
        }
    }
}

struct ViewerParagraph: Hashable {
    let text: String
}

/// A declarative list marker inherited by a text block from its list-item.
struct ViewerListContext: Hashable {
    let ordered: Bool
    let index: Int
    let kind: String?
    let checked: Bool
}

/// Identifies the nearest list item that owns a renderable leaf. The identity
/// is assigned while consuming the deterministic compiler sequence, so leaves
/// from one item share geometry even when their paint differs.
struct ViewerListItemBoundary: Hashable {
    let identity: Int
    let nestingDepth: UInt16
    let isFirstRenderableLeaf: Bool
    let isFinalRenderableLeaf: Bool
}

enum ViewerInline: Hashable {
    case text(text: String, marks: [FfiViewerMark])
    case atom(nodeType: String, docPos: UInt32, attrsJSON: String, label: String)
}

/// A renderable leaf block. Container nodes are represented by the inherited
/// context instead of being re-laid out as synthetic paragraphs.
struct ViewerBlock: Hashable {
    let nodeType: String
    let depth: UInt16
    let inBlockquote: Bool
    let listContext: ViewerListContext?
    let listItemBoundary: ViewerListItemBoundary?
    let inlines: [ViewerInline]

    func withListItemBoundary(_ boundary: ViewerListItemBoundary) -> ViewerBlock {
        ViewerBlock(
            nodeType: nodeType,
            depth: depth,
            inBlockquote: inBlockquote,
            listContext: listContext,
            listItemBoundary: boundary,
            inlines: inlines
        )
    }
}

/// Width-independent native projection of the Rust compiled document. The
/// prepared theme is attached only to the measurement copy; cached compiler
/// output remains semantic/theme independent.
struct ViewerDocument {
    let semanticKey: String
    let blocks: [ViewerBlock]
    let isEmpty: Bool
    let retainedBytes: Int
    let preparedTheme: PreparedProseTheme?

    var paragraphs: [ViewerParagraph] {
        blocks.compactMap { block in
            let text = block.inlines.reduce(into: "") { partial, inline in
                switch inline {
                case let .text(text: value, marks: _): partial.append(value)
                case let .atom(nodeType: _, docPos: _, attrsJSON: _, label: label): partial.append(label)
                }
            }
            return ViewerParagraph(text: text)
        }
    }

    init(semanticKey: String, paragraphs: [ViewerParagraph], isEmpty: Bool, retainedBytes: Int) {
        self.semanticKey = semanticKey
        blocks = paragraphs.map {
            ViewerBlock(
                nodeType: "paragraph",
                depth: 0,
                inBlockquote: false,
                listContext: nil,
                listItemBoundary: nil,
                inlines: [.text(text: $0.text, marks: [])]
            )
        }
        self.isEmpty = isEmpty
        self.retainedBytes = retainedBytes
        preparedTheme = nil
    }

    init(
        semanticKey: String,
        blocks: [ViewerBlock],
        isEmpty: Bool,
        retainedBytes: Int,
        preparedTheme: PreparedProseTheme? = nil
    ) {
        self.semanticKey = semanticKey
        self.blocks = blocks
        self.isEmpty = isEmpty
        self.retainedBytes = retainedBytes
        self.preparedTheme = preparedTheme
    }

    init(compiled: ViewerCompiledDocument) throws {
        semanticKey = compiled.semanticKey()
        isEmpty = compiled.isEmpty()
        retainedBytes = Int(compiled.retainedBytesDecimal()) ?? 0
        preparedTheme = nil

        struct Builder {
            let nodeType: String
            let depth: UInt16
            let listContext: ViewerListContext?
            let listItemIdentity: Int?
            var inlines: [ViewerInline]
        }

        var stack: [Builder] = []
        var rendered: [ViewerBlock] = []
        var renderableLeavesByListItem: [Int: [Int]] = [:]
        var listItemDepthByIdentity: [Int: UInt16] = [:]
        var nextListItemIdentity = 0
        for element in compiled.elements() {
            switch element {
            case let .blockStart(nodeType: nodeType, depth: depth, listContextJson: listContextJSON):
                let listItemIdentity: Int?
                if nodeType == "listItem" {
                    listItemIdentity = nextListItemIdentity
                    listItemDepthByIdentity[nextListItemIdentity] = depth
                    nextListItemIdentity += 1
                } else {
                    listItemIdentity = nil
                }
                stack.append(
                    Builder(
                        nodeType: nodeType,
                        depth: depth,
                        listContext: Self.listContext(from: listContextJSON),
                        listItemIdentity: listItemIdentity,
                        inlines: []
                    )
                )
            case let .textRun(text: text, marks: marks):
                guard !stack.isEmpty else { continue }
                stack[stack.count - 1].inlines.append(.text(text: text, marks: marks))
            case let .inlineAtom(nodeType: nodeType, docPos: docPos, attrsJson: attrsJson, label: label):
                guard !stack.isEmpty else { continue }
                stack[stack.count - 1].inlines.append(
                    .atom(nodeType: nodeType, docPos: docPos, attrsJSON: attrsJson, label: label)
                )
            case let .blockAtom(nodeType: nodeType, docPos: docPos, attrsJson: attrsJson, label: label):
                let listContext = stack.reversed().compactMap(\.listContext).first
                let listItemIdentity = stack.reversed().compactMap(\.listItemIdentity).first
                rendered.append(
                    ViewerBlock(
                        nodeType: nodeType,
                        depth: stack.last?.depth ?? 0,
                        inBlockquote: stack.contains { $0.nodeType == "blockquote" },
                        listContext: listContext,
                        listItemBoundary: nil,
                        inlines: [.atom(nodeType: nodeType, docPos: docPos, attrsJSON: attrsJson, label: label)]
                    )
                )
                if let listItemIdentity {
                    renderableLeavesByListItem[listItemIdentity, default: []].append(rendered.count - 1)
                }
            case .blockEnd:
                guard let builder = stack.popLast(), !builder.inlines.isEmpty else { continue }
                let ancestors = stack + [builder]
                let listContext = ancestors.reversed().compactMap(\.listContext).first
                let listItemIdentity = ancestors.reversed().compactMap(\.listItemIdentity).first
                let inBlockquote = ancestors.contains { $0.nodeType == "blockquote" }
                rendered.append(
                    ViewerBlock(
                        nodeType: builder.nodeType,
                        depth: builder.depth,
                        inBlockquote: inBlockquote,
                        listContext: listContext,
                        listItemBoundary: nil,
                        inlines: builder.inlines
                    )
                )
                if let listItemIdentity {
                    renderableLeavesByListItem[listItemIdentity, default: []].append(rendered.count - 1)
                }
            }
        }
        for (identity, leaves) in renderableLeavesByListItem {
            guard let first = leaves.first, let final = leaves.last else { continue }
            for leaf in leaves {
                rendered[leaf] = rendered[leaf].withListItemBoundary(
                    ViewerListItemBoundary(
                        identity: identity,
                        nestingDepth: listItemDepthByIdentity[identity] ?? 0,
                        isFirstRenderableLeaf: leaf == first,
                        isFinalRenderableLeaf: leaf == final
                    )
                )
            }
        }
        let admittedAttachmentCount = rendered.reduce(into: 0) { count, block in
            if block.nodeType == "image", ViewerImageAttachment.sourceAndDeclaredSize(in: block) != nil {
                count += 1
            }
        }
        guard admittedAttachmentCount <= ViewerImageAttachment.maximumAdmittedAttachments else {
            throw ProseViewerError.compiler(
                domain: "viewer",
                code: "ATTACHMENT_LIMIT_EXCEEDED",
                message: "The document exceeds the maximum admitted image attachment count."
            )
        }
        blocks = rendered.isEmpty && !isEmpty
            ? [ViewerBlock(nodeType: "paragraph", depth: 0, inBlockquote: false, listContext: nil, listItemBoundary: nil, inlines: [.text(text: "", marks: [])])]
            : rendered
    }

    func withPreparedTheme(_ theme: PreparedProseTheme) -> ViewerDocument {
        ViewerDocument(
            semanticKey: semanticKey,
            blocks: blocks,
            isEmpty: isEmpty,
            retainedBytes: retainedBytes,
            preparedTheme: theme
        )
    }

    private static func listContext(from json: String?) -> ViewerListContext? {
        guard let json,
              let data = json.data(using: .utf8),
              let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return ViewerListContext(
            ordered: (value["ordered"] as? Bool) ?? false,
            index: (value["index"] as? NSNumber)?.intValue ?? 1,
            kind: value["kind"] as? String,
            checked: (value["checked"] as? Bool) ?? false
        )
    }
}

private enum SHA256Digest {
    static func hex(_ value: String) -> String {
        let data = Data(value.utf8)
        return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }
}
