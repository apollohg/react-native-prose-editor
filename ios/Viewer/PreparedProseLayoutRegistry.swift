import CryptoKit
import Foundation
import UIKit

/// Shared compiler and prepared-layout owner for UIKit and Fabric.
@objc(PREPPreparedProseLayoutRegistry)
public final class PreparedProseLayoutRegistry: NSObject {
    typealias DocumentCompiler = (ProseViewerRequest) throws -> ViewerDocument
    typealias LayoutPreparation = (ViewerDocument, ProseLayoutKey, CGFloat, CGFloat) throws -> PreparedProseLayout

    @objc public class var sharedRegistry: PreparedProseLayoutRegistry { shared }
    static let shared = PreparedProseLayoutRegistry()

    private final class Compilation {
        var result: Result<ViewerDocument, Error>?
    }

    private let lock = NSLock()
    private let compiledCondition = NSCondition()
    private var compiledDocuments: [String: ViewerDocument] = [:]
    private var compiledInFlight: [String: Compilation] = [:]
    private let layoutCache: PreparedProseLayoutCache
    private let compile: DocumentCompiler
    private let prepare: LayoutPreparation
    private(set) var layoutPreparationCount = 0

    override convenience init() {
        self.init(compile: Self.compileWithRust, prepare: Self.prepareWithCoreText)
    }

    init(
        byteBudget: Int = 32 * 1024 * 1024,
        compile: @escaping DocumentCompiler,
        prepare: @escaping LayoutPreparation = Self.prepareWithCoreText
    ) {
        self.compile = compile
        self.prepare = prepare
        layoutCache = PreparedProseLayoutCache(byteBudget: byteBudget)
        super.init()
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(didReceiveMemoryWarning),
            name: UIApplication.didReceiveMemoryWarningNotification,
            object: nil
        )
    }

    deinit { NotificationCenter.default.removeObserver(self) }

    func compileDocument(request: ProseViewerRequest) throws -> ViewerDocument {
        let cacheKey = request.compiledCacheKey
        compiledCondition.lock()
        if let document = compiledDocuments[cacheKey] {
            compiledCondition.unlock()
            return document
        }
        if let compilation = compiledInFlight[cacheKey] {
            while compilation.result == nil { compiledCondition.wait() }
            let result = compilation.result!
            compiledCondition.unlock()
            return try result.get()
        }
        let compilation = Compilation()
        compiledInFlight[cacheKey] = compilation
        compiledCondition.unlock()

        let result = Result<ViewerDocument, Error> {
            let compiled = try compile(request)
            guard compiled.semanticKey.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil else {
                throw ProseViewerError.compiler(
                    domain: "viewer",
                    code: "INVALID_SEMANTIC_KEY",
                    message: "The compiler returned an invalid semantic key."
                )
            }
            return compiled
        }

        compiledCondition.lock()
        if case let .success(document) = result {
            compiledDocuments[cacheKey] = document
        }
        compilation.result = result
        compiledInFlight.removeValue(forKey: cacheKey)
        compiledCondition.broadcast()
        compiledCondition.unlock()
        return try result.get()
    }

    func measure(request: ProseViewerRequest, widthPoints: CGFloat, scale: CGFloat) -> PreparedProseLayout {
        guard widthPoints.isFinite, widthPoints > 0, scale.isFinite, scale > 0 else {
            return errorArtifact(
                request: request,
                widthPoints: widthPoints,
                scale: scale,
                error: .hostContract(message: "A finite positive width is required for prose measurement.")
            )
        }
        do {
            var document = try compileDocument(request: request)
            if document.isEmpty && !request.configuration.collapsesWhenEmpty {
                document = ViewerDocument(
                    semanticKey: document.semanticKey,
                    paragraphs: document.paragraphs,
                    isEmpty: false,
                    retainedBytes: document.retainedBytes
                )
            }
            let key = layoutKey(for: document, request: request, widthPoints: widthPoints, scale: scale)
            return try layoutCache.value(for: key) { [weak self] in
                guard let self else {
                    throw ProseViewerError.layout(message: "The layout registry was released during preparation.")
                }
                self.lock.lock()
                self.layoutPreparationCount += 1
                self.lock.unlock()
                return try self.prepare(document, key, CGFloat(key.widthPixels) / scale, scale)
            }
        } catch let error as ProseViewerError {
            return publishErrorArtifact(request: request, widthPoints: widthPoints, scale: scale, error: error)
        } catch {
            return publishErrorArtifact(
                request: request,
                widthPoints: widthPoints,
                scale: scale,
                error: .layout(message: String(describing: error))
            )
        }
    }

    @objc(measureSourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:widthPoints:scale:)
    public func measure(
        sourceKind: NSString,
        source: NSString,
        configJSON: NSString,
        themeJSON: NSString?,
        imagePolicyJSON: NSString?,
        imagesEnabled: Bool,
        collapsesWhenEmpty: Bool,
        widthPoints: CGFloat,
        scale: CGFloat
    ) -> CGSize {
        let request = ProseViewerRequest(
            source: sourceKind == "html" ? .html(source as String) : .json(source as String),
            configuration: ProseViewerConfiguration(
                configJSON: configJSON as String,
                themeJSON: themeJSON as String?,
                imagePolicyJSON: imagePolicyJSON as String?,
                imagesEnabled: imagesEnabled,
                collapsesWhenEmpty: collapsesWhenEmpty
            )
        )
        return measure(request: request, widthPoints: widthPoints, scale: scale).size
    }

    @objc(installCachedLayoutInDrawingView:sourceKind:source:configJSON:themeJSON:imagePolicyJSON:imagesEnabled:collapsesWhenEmpty:widthPoints:scale:)
    public func installCachedLayout(
        in drawingView: PreparedProseDrawingView,
        sourceKind: NSString,
        source: NSString,
        configJSON: NSString,
        themeJSON: NSString?,
        imagePolicyJSON: NSString?,
        imagesEnabled: Bool,
        collapsesWhenEmpty: Bool,
        widthPoints: CGFloat,
        scale: CGFloat
    ) -> Bool {
        let request = ProseViewerRequest(
            source: sourceKind == "html" ? .html(source as String) : .json(source as String),
            configuration: ProseViewerConfiguration(
                configJSON: configJSON as String,
                themeJSON: themeJSON as String?,
                imagePolicyJSON: imagePolicyJSON as String?,
                imagesEnabled: imagesEnabled,
                collapsesWhenEmpty: collapsesWhenEmpty
            )
        )
        guard var document = try? compileDocument(request: request) else {
            let artifact = errorArtifact(
                request: request,
                widthPoints: widthPoints,
                scale: scale,
                error: .compiler(domain: "viewer", code: "MALFORMED_INPUT", message: "The source could not be compiled.")
            )
            guard let cached = layoutCache.cachedValue(for: artifact.key) else { return false }
            drawingView.install(layout: cached)
            return true
        }
        if document.isEmpty && !request.configuration.collapsesWhenEmpty {
            document = ViewerDocument(
                semanticKey: document.semanticKey,
                paragraphs: document.paragraphs,
                isEmpty: false,
                retainedBytes: document.retainedBytes
            )
        }
        let key = layoutKey(for: document, request: request, widthPoints: widthPoints, scale: scale)
        guard let artifact = layoutCache.cachedValue(for: key) else { return false }
        drawingView.install(layout: artifact)
        return true
    }

    @objc func didReceiveMemoryWarning() {
        layoutCache.removeAllUnmounted()
        compiledCondition.lock()
        compiledDocuments.removeAll()
        compiledCondition.unlock()
    }

    private func layoutKey(
        for document: ViewerDocument,
        request: ProseViewerRequest,
        widthPoints: CGFloat,
        scale: CGFloat
    ) -> ProseLayoutKey {
        let pixels = Int((widthPoints * scale).rounded())
        return ProseLayoutKey(
            semanticKey: document.semanticKey,
            widthPixels: pixels,
            themeDigest: request.themeDigest,
            fontRevision: request.fontRevision,
            displayScale: scale,
            attachmentRevision: request.attachmentRevision
        )
    }

    private func errorArtifact(
        request: ProseViewerRequest,
        widthPoints: CGFloat,
        scale: CGFloat,
        error: ProseViewerError
    ) -> PreparedProseLayout {
        let safeScale = scale.isFinite && scale > 0 ? scale : 1
        let safeWidth = widthPoints.isFinite && widthPoints > 0 ? widthPoints : 0
        let key = ProseLayoutKey(
            semanticKey: "error:" + request.compiledCacheKey,
            widthPixels: Int((safeWidth * safeScale).rounded()),
            themeDigest: request.themeDigest,
            fontRevision: request.fontRevision,
            displayScale: safeScale,
            attachmentRevision: request.attachmentRevision
        )
        return .error(key: key, width: safeWidth, error: error)
    }

    private func publishErrorArtifact(
        request: ProseViewerRequest,
        widthPoints: CGFloat,
        scale: CGFloat,
        error: ProseViewerError
    ) -> PreparedProseLayout {
        let artifact = errorArtifact(request: request, widthPoints: widthPoints, scale: scale, error: error)
        return (try? layoutCache.value(for: artifact.key) { artifact }) ?? artifact
    }

    private static func prepareWithCoreText(
        document: ViewerDocument,
        key: ProseLayoutKey,
        widthPoints: CGFloat,
        scale: CGFloat
    ) throws -> PreparedProseLayout {
        try CoreTextProseLayoutEngine().prepare(
            document: document,
            key: key,
            widthPoints: widthPoints,
            displayScale: scale
        )
    }

    private static func compileWithRust(request: ProseViewerRequest) throws -> ViewerDocument {
        let result = viewerCompile(
            request: FfiViewerCompileRequest(
                sourceKind: request.source.kind,
                source: request.source.value,
                configJson: request.configuration.configJSON,
                imagesEnabled: request.configuration.imagesEnabled,
                mentionPrefix: request.mentionPrefix
            )
        )
        if let error = result.error {
            throw ProseViewerError.compiler(domain: error.domain, code: error.code, message: error.message)
        }
        guard let compiled = result.value else {
            throw ProseViewerError.compiler(
                domain: "viewer",
                code: "MISSING_COMPILED_DOCUMENT",
                message: "The compiler returned neither a document nor an error."
            )
        }
        return try ViewerDocument(compiled: compiled)
    }
}
