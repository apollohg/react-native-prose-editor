import CryptoKit
import Foundation
import UIKit

extension PreparedProseLayoutRegistry {
    func layoutKey(
        for document: ViewerDocument,
        request: ProseViewerRequest,
        widthPixels: Int,
        scale: CGFloat
    ) -> ProseLayoutKey {
        return ProseLayoutKey(
            semanticKey: document.semanticKey,
            widthPixels: widthPixels,
            themeDigest: request.themeDigest,
            nativeFontRevision: request.nativeFontRevision,
            fontEnvironmentRevision: request.fontEnvironmentRevision,
            displayScale: scale,
            attachmentRevision: request.attachmentRevision,
            generationIdentity: request.generationIdentity,
            semanticGenerationIdentity: request.semanticGenerationIdentity
        )
    }

    func errorArtifact(
        key: ProseLayoutKey,
        width: CGFloat,
        error: ProseViewerError
    ) -> PreparedProseLayout {
        .error(key: key, width: width, error: error)
    }

    func cachedErrorArtifact(
        request: ProseViewerRequest,
        widthPixels: Int,
        scale: CGFloat,
        error: ProseViewerError,
        fabricSurface: FabricSurfaceToken?,
        fabricGeneration: FabricGenerationToken?,
        fabricLeaseHandle: UInt64?
    ) -> PreparedProseLayout {
        let key = errorLayoutKey(request: request, widthPixels: widthPixels, scale: scale)
        let width = ProseLayoutMetrics.canonicalWidth(widthPixels: widthPixels, scale: scale)
        return (try? layoutCache.value(
            for: key,
            fabricSurface: fabricSurface,
            fabricLeaseHandle: fabricLeaseHandle,
            shouldCreateFabricLease: {
                guard let fabricGeneration else { return true }
                return self.isFabricLeaseActive(fabricGeneration)
            }
        ) {
            self.errorArtifact(key: key, width: width, error: error)
        }) ?? errorArtifact(key: key, width: width, error: error)
    }

    private func errorLayoutKey(
        request: ProseViewerRequest,
        widthPixels: Int,
        scale: CGFloat
    ) -> ProseLayoutKey {
        ProseLayoutKey(
            semanticKey: "error:" + request.compiledCacheKey,
            widthPixels: widthPixels,
            themeDigest: request.themeDigest,
            nativeFontRevision: request.nativeFontRevision,
            fontEnvironmentRevision: request.fontEnvironmentRevision,
            displayScale: scale,
            attachmentRevision: request.attachmentRevision,
            generationIdentity: request.generationIdentity,
            semanticGenerationIdentity: request.semanticGenerationIdentity
        )
    }

    func invalidWidthArtifact(
        request: ProseViewerRequest,
        scale: CGFloat,
        error: ProseViewerError
    ) -> PreparedProseLayout {
        let safeScale = scale.isFinite && scale > 0 ? scale : 1
        return errorArtifact(
            key: errorLayoutKey(request: request, widthPixels: 0, scale: safeScale),
            width: 0,
            error: error
        )
    }

    func preparedDocument(
        request: ProseViewerRequest,
        compiledDocument: ViewerDocument?,
        fabricGeneration: FabricGenerationToken?
    ) throws -> ViewerDocument {
        if let data = request.configuration.configJSON.data(using: .utf8),
           let values = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let highlighting = NativeCodeHighlightConfiguration.from(values["codeHighlighting"]) {
            _ = try NativeCodeHighlightingRegistry.provider(id: highlighting.provider)
        }
        compiledCondition.lock()
        if let fabricGeneration,
           !isFabricLeaseActiveLocked(fabricGeneration) {
            compiledCondition.unlock()
            throw FabricMeasurementCancelled()
        }
        if let fabricGeneration, let document = documentsByFabricGeneration[fabricGeneration] {
            compiledCondition.unlock()
            return documentForEmptyContentPolicy(document, request: request)
                .withPreparedTheme(try preparedTheme(for: request, generation: fabricGeneration))
        }
        if let fabricGeneration, let failure = failuresByFabricGeneration[fabricGeneration] {
            compiledCondition.unlock()
            throw failure
        }
        compiledCondition.unlock()

        do {
            let document: ViewerDocument
            if let compiledDocument {
                document = compiledDocument
            } else {
                document = try compileDocument(request: request)
            }
            if let fabricGeneration {
                compiledCondition.lock()
                guard isFabricLeaseActiveLocked(fabricGeneration) else {
                    compiledCondition.unlock()
                    throw FabricMeasurementCancelled()
                }
                documentsByFabricGeneration[fabricGeneration] = document
                compiledCondition.unlock()
            }
            return documentForEmptyContentPolicy(document, request: request)
                .withPreparedTheme(try preparedTheme(for: request, generation: fabricGeneration))
        } catch {
            if error is FabricMeasurementCancelled {
                throw error
            }
            if let fabricGeneration {
                compiledCondition.lock()
                if isFabricLeaseActiveLocked(fabricGeneration) {
                    failuresByFabricGeneration[fabricGeneration] = error
                }
                compiledCondition.unlock()
            }
            throw error
        }
    }

    private func documentForEmptyContentPolicy(
        _ compiledDocument: ViewerDocument,
        request: ProseViewerRequest
    ) -> ViewerDocument {
        let collapse = request.configuration.collapsesWhenEmpty
        let trailingCount = collapse
            ? min(compiledDocument.trailingEmptyTextBlockCount, compiledDocument.blocks.count)
            : 0
        let blocks = trailingCount == 0
            ? compiledDocument.blocks
            : Array(compiledDocument.blocks.dropLast(trailingCount))
        return ViewerDocument(
            semanticKey: compiledDocument.semanticKey,
            blocks: blocks,
            isEmpty: collapse ? blocks.isEmpty : false,
            retainedBytes: compiledDocument.retainedBytes,
            trailingEmptyTextBlockCount: compiledDocument.trailingEmptyTextBlockCount
        )
    }

    /// Parsed paint values are immutable and shared across all width-specific
    /// layouts for the same semantic generation.
    private func preparedTheme(
        for request: ProseViewerRequest,
        generation: FabricGenerationToken?
    ) throws -> PreparedProseTheme {
        compiledCondition.lock()
        defer { compiledCondition.unlock() }
        if let generation,
           !isFabricLeaseActiveLocked(generation) {
            throw FabricMeasurementCancelled()
        }
        return preparedThemeLocked(for: request, generation: generation)
    }

    /// Caller must hold `compiledCondition`.
    func preparedThemeLocked(
        for request: ProseViewerRequest,
        generation: FabricGenerationToken?
    ) -> PreparedProseTheme {
        if let generation, themeOwners[generation] == nil {
            themeOwners[generation] = request.generationIdentity
            themeOwnerCounts[request.generationIdentity, default: 0] += 1
        }
        if let theme = themesByGeneration[request.generationIdentity] {
            touchTheme(request.generationIdentity)
            return theme
        }
        var theme: PreparedProseTheme!
        request.appearance.traits.performAsCurrent {
            theme = PreparedProseTheme.resolve(
                themeJSON: request.configuration.themeJSON,
                fontScale: request.nativeFontScale,
                semanticGeneration: request.semanticGenerationIdentity
            )
        }
        if let data = request.configuration.configJSON.data(using: .utf8),
           let values = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            theme.codeHighlighting = NativeCodeHighlightConfiguration.from(values["codeHighlighting"])
        }
        themesByGeneration[request.generationIdentity] = theme
        themesRetainedBytes += theme.estimatedRetainedBytes
        touchTheme(request.generationIdentity)
        trimThemesToBudget()
        return theme
    }

    private func touchTheme(_ generationIdentity: String) {
        themeAccessOrder.removeAll { $0 == generationIdentity }
        themeAccessOrder.append(generationIdentity)
    }

    /// Pinned Fabric generations own their exact resolved value until the
    /// matching release callback. Unowned values form a byte/count-bounded
    /// LRU, so background source churn cannot grow this cache without limit.
    private func trimThemesToBudget() {
        while (themesByGeneration.count > themeEntryBudget || themesRetainedBytes > themeByteBudget),
              let oldest = themeAccessOrder.first(where: { themeOwnerCounts[$0, default: 0] == 0 }) {
            themeAccessOrder.removeAll { $0 == oldest }
            if let theme = themesByGeneration.removeValue(forKey: oldest) {
                themesRetainedBytes -= theme.estimatedRetainedBytes
            }
        }
    }

    func releaseThemeOwnership(for generation: FabricGenerationToken) {
        guard let generationIdentity = themeOwners.removeValue(forKey: generation) else { return }
        let remaining = max(0, (themeOwnerCounts[generationIdentity] ?? 1) - 1)
        if remaining == 0 {
            themeOwnerCounts.removeValue(forKey: generationIdentity)
        } else {
            themeOwnerCounts[generationIdentity] = remaining
        }
        trimThemesToBudget()
    }

    func makeRequest(
        sourceKind: NSString,
        source: NSString,
        configJSON: NSString,
        themeJSON: NSString?,
        imagePolicyJSON: NSString?,
        imagesEnabled: Bool,
        collapsesWhenEmpty: Bool,
        attachmentRevision: UInt64,
        nativeFontRevision: UInt64,
        nativeFontScale: CGFloat,
        fontEnvironmentRevision: UInt64,
        userInterfaceStyle: Int,
        accessibilityContrast: Int
    ) -> ProseViewerRequest {
        ProseViewerRequest(
            source: sourceKind == "html" ? .html(source as String) : .json(source as String),
            configuration: ProseViewerConfiguration(
                configJSON: configJSON as String,
                themeJSON: themeJSON as String?,
                imagePolicyJSON: imagePolicyJSON as String?,
                imagesEnabled: imagesEnabled,
                collapsesWhenEmpty: collapsesWhenEmpty
            ),
            nativeFontRevision: nativeFontRevision,
            nativeFontScale: nativeFontScale,
            fontEnvironmentRevision: fontEnvironmentRevision,
            attachmentRevision: attachmentRevision,
            appearance: ProseViewerAppearance(
                rawUserInterfaceStyle: userInterfaceStyle,
                rawAccessibilityContrast: accessibilityContrast
            )
        )
    }

    func touchCompiled(_ cacheKey: String) {
        guard compiledDocuments[cacheKey] != nil else { return }
        precondition(nextCompiledAccessGeneration < UInt64.max, "Prepared prose compiled LRU generation overflowed.")
        nextCompiledAccessGeneration += 1
        compiledAccessGenerations[cacheKey] = nextCompiledAccessGeneration
        compiledAccessOrder.append((cacheKey, nextCompiledAccessGeneration))
        compactCompiledAccessOrderIfNeeded()
    }

    func trimCompiledToBudget() {
        while compiledRetainedBytes > compiledByteBudget, let oldest = oldestCompiledKey() {
            if let removed = compiledDocuments.removeValue(forKey: oldest) {
                compiledRetainedBytes -= removed.retainedBytes
                compiledAccessGenerations.removeValue(forKey: oldest)
            }
        }
        PreparedProseInstrumentation.retained(.compiled, scope: "registry", bytes: compiledRetainedBytes)
        PreparedProseInstrumentation.cacheUpdated(
            compiledBytes: compiledRetainedBytes,
            compiledResidentCount: compiledDocuments.count
        )
    }

    private func oldestCompiledKey() -> String? {
        while compiledAccessOrderHead < compiledAccessOrder.count {
            let token = compiledAccessOrder[compiledAccessOrderHead]
            compiledAccessOrderHead += 1
            if compiledAccessGenerations[token.key] == token.generation, compiledDocuments[token.key] != nil {
                compactCompiledAccessOrderIfNeeded()
                return token.key
            }
        }
        compiledAccessOrder.removeAll(keepingCapacity: true)
        compiledAccessOrderHead = 0
        return nil
    }

    private func compactCompiledAccessOrderIfNeeded() {
        let liveTokenCount = compiledAccessOrder.count - compiledAccessOrderHead
        guard liveTokenCount > max(64, compiledDocuments.count * 3) else { return }
        compiledAccessOrder = compiledAccessOrder[compiledAccessOrderHead...].filter { token in
            compiledAccessGenerations[token.key] == token.generation && compiledDocuments[token.key] != nil
        }
        compiledAccessOrderHead = 0
    }

    func touchCompilationFailure(_ cacheKey: String) {
        compilationFailureAccessOrder.removeAll { $0 == cacheKey }
        compilationFailureAccessOrder.append(cacheKey)
    }

    func trimCompilationFailuresToBudget() {
        while compilationFailures.count > compilationFailureBudget,
              let oldest = compilationFailureAccessOrder.first {
            compilationFailureAccessOrder.removeFirst()
            compilationFailures.removeValue(forKey: oldest)
        }
    }

    static func prepareWithCoreText(
        document: ViewerDocument,
        key: ProseLayoutKey,
        widthPoints: CGFloat,
        scale: CGFloat
    ) throws -> PreparedProseLayout {
        try CoreTextProseLayoutEngine().prepare(
            document: document,
            key: key,
            widthPoints: widthPoints,
            displayScale: scale,
            semanticGenerationIdentity: key.semanticGenerationIdentity
        )
    }

    static func compileWithRust(request: ProseViewerRequest) throws -> ViewerDocument {
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
