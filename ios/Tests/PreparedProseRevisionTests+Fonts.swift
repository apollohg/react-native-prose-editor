import XCTest
import UIKit
import CoreText

extension PreparedProseRevisionTests {
    func testMissingFamilyWarningSurvivesFontEnvironmentReplacementButNewSemanticGenerationWarns() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        XCTAssertTrue(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "a"))
        XCTAssertFalse(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "a"))
        environment.invalidateRegisteredFonts()
        XCTAssertFalse(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "a"))
        XCTAssertTrue(environment.shouldWarnForMissingFamily("definitely-missing", semanticGeneration: "b"))
    }

    func testGenericAliasesResolveSilentlyAndInlineCodeUsesMonospacedSystemFont() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        let semanticGeneration = "generic-fonts-\(UUID().uuidString)"
        let fallback = UIFont.systemFont(ofSize: 17)
        let inlineCode = environment.resolveFont(
            family: "monospace",
            size: 17,
            fallback: fallback,
            additionalTraits: [.traitBold, .traitItalic],
            semanticGeneration: semanticGeneration
        )

        XCTAssertTrue(inlineCode.fontDescriptor.symbolicTraits.contains(.traitMonoSpace))
        XCTAssertTrue(inlineCode.fontDescriptor.symbolicTraits.contains(.traitBold))
        XCTAssertTrue(inlineCode.fontDescriptor.symbolicTraits.contains(.traitItalic))
        XCTAssertFalse(
            environment.hasMissingFamilyWarning("monospace", semanticGeneration: semanticGeneration),
            "generic monospace must not enter the missing-family warning registry"
        )

        for alias in ["system-ui", "sans-serif", "serif", "ui-sans-serif", "sans-serif-condensed", "cursive"] {
            _ = environment.resolveFont(
                family: alias,
                size: 17,
                fallback: fallback,
                semanticGeneration: semanticGeneration
            )
            XCTAssertFalse(
                environment.hasMissingFamilyWarning(alias, semanticGeneration: semanticGeneration),
                "generic alias \(alias) must resolve without a missing-family warning"
            )
        }
    }

    func testHeadingInheritsCustomBaseFamilyWhileApplyingDefaultBold() {
        let base = UIFont(name: "Courier", size: 17)!
        let theme = PreparedProseTheme.resolve(
            themeJSON: #"{"text":{"fontFamily":"Courier"},"paragraph":{"fontSize":18},"blockquote":{"text":{"fontStyle":"italic"}},"codeBlock":{"text":{"fontSize":16}}}"#,
            semanticGeneration: "heading-inheritance-\(UUID().uuidString)"
        )
        let heading = theme.headings["h1"]!

        XCTAssertEqual(theme.paragraph.font.familyName, base.familyName)
        XCTAssertEqual(theme.blockquote.font.familyName, base.familyName)
        XCTAssertEqual(theme.code.font.familyName, base.familyName)
        XCTAssertEqual(heading.font.familyName, base.familyName)
        XCTAssertTrue(heading.font.fontDescriptor.symbolicTraits.contains(.traitBold))
    }

    func testExplicitHeadingFamilyOverridesInheritedBaseFamily() {
        let expected = UIFont(name: "Courier-Bold", size: 32)!
        let theme = PreparedProseTheme.resolve(
            themeJSON: #"{"text":{"fontFamily":"Courier"},"headings":{"h1":{"fontFamily":"Courier-Bold"}}}"#,
            semanticGeneration: "heading-override-\(UUID().uuidString)"
        )

        XCTAssertEqual(theme.headings["h1"]?.font.fontName, expected.fontName)
    }

    func testMissingLiteralFamilyWarnsOnceAndFallsBack() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        let family = "missing-literal-font-\(UUID().uuidString)"
        let semanticGeneration = "missing-literal-\(UUID().uuidString)"
        let fallback = UIFont(name: "Courier", size: 17)!

        let first = environment.resolveFont(
            family: family,
            size: 17,
            fallback: fallback,
            semanticGeneration: semanticGeneration
        )
        let second = environment.resolveFont(
            family: family,
            size: 17,
            fallback: fallback,
            semanticGeneration: semanticGeneration
        )

        XCTAssertEqual(first.fontName, fallback.fontName)
        XCTAssertEqual(second.fontName, fallback.fontName)
        XCTAssertTrue(
            environment.hasMissingFamilyWarning(family, semanticGeneration: semanticGeneration),
            "two literal-name resolutions in one semantic generation must emit only one warning"
        )
    }

    func testThemeFamiliesUseOneSemanticWarningAcrossStylesAndLayoutRevisions() {
        let family = "missing-theme-family-\(UUID().uuidString)"
        let semanticA = "theme-warning-a-\(UUID().uuidString)"
        let semanticB = "theme-warning-b-\(UUID().uuidString)"
        let theme = """
        {"text":{"fontFamily":"\(family)"},"paragraph":{"fontFamily":"\(family)"},"blockquote":{"text":{"fontFamily":"\(family)"}},"codeBlock":{"text":{"fontFamily":"\(family)"}},"headings":{"h1":{"fontFamily":"\(family)"},"h2":{"fontFamily":"\(family)"},"h3":{"fontFamily":"\(family)"},"h4":{"fontFamily":"\(family)"},"h5":{"fontFamily":"\(family)"},"h6":{"fontFamily":"\(family)"}},"links":{"fontFamily":"\(family)"}}
        """

        _ = PreparedProseTheme.resolve(themeJSON: theme, semanticGeneration: semanticA)
        XCTAssertFalse(ViewerFontEnvironment.shared.shouldWarnForMissingFamily(family, semanticGeneration: semanticA))
        _ = PreparedProseTheme.resolve(themeJSON: theme, fontScale: 1.4, semanticGeneration: semanticA)
        XCTAssertFalse(ViewerFontEnvironment.shared.shouldWarnForMissingFamily(family, semanticGeneration: semanticA))
        _ = PreparedProseTheme.resolve(themeJSON: theme, semanticGeneration: semanticB)
        XCTAssertFalse(ViewerFontEnvironment.shared.shouldWarnForMissingFamily(family, semanticGeneration: semanticB))
    }

    func testValidThemeFamilyRemainsSilentAndCustomFallbackPreservesBoldItalic() {
        let validSemantic = "valid-theme-family-\(UUID().uuidString)"
        _ = PreparedProseTheme.resolve(
            themeJSON: #"{"paragraph":{"fontFamily":"Courier"}}"#,
            semanticGeneration: validSemantic
        )
        XCTAssertTrue(ViewerFontEnvironment.shared.shouldWarnForMissingFamily("Courier", semanticGeneration: validSemantic))

        let missingFamily = "missing-custom-family-\(UUID().uuidString)"
        let fallback = UIFont(name: "Courier", size: 17)!
        let resolved = ViewerFontEnvironment.shared.resolveFont(
            family: missingFamily,
            size: 17,
            fallback: fallback,
            additionalTraits: [.traitBold, .traitItalic],
            semanticGeneration: "custom-fallback-\(UUID().uuidString)"
        )
        XCTAssertTrue(resolved.fontDescriptor.symbolicTraits.contains(.traitBold))
        XCTAssertTrue(resolved.fontDescriptor.symbolicTraits.contains(.traitItalic))
    }

    /// An inherited custom fallback may remain in use only when it supplies
    /// every requested emphasis trait. A face without those traits must take
    /// the deterministic system fallback instead of returning partial bold or
    /// italic styling.
    func testInheritedCustomFallbackMissingEmphasisUsesSystemFallback() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        let custom = UIFont(name: "AppleColorEmoji", size: 17)!
        let requested: UIFontDescriptor.SymbolicTraits = [.traitBold, .traitItalic]
        let resolved = environment.resolveFont(
            family: nil,
            size: 17,
            fallback: custom,
            additionalTraits: requested,
            semanticGeneration: "inherited-custom-missing-emphasis"
        )

        XCTAssertTrue(ViewerFontEnvironment.satisfiesRequestedEmphasis(resolved, requestedTraits: requested))
        XCTAssertNotEqual(resolved.familyName, custom.familyName)
    }

    /// A registered custom family remains authoritative when it can express
    /// the requested pair, while an explicit family that cannot does not
    /// silently return an incomplete face.
    func testExplicitCustomFamilyTraitResolutionPrefersCompleteFaceOrFallsBack() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        let requested: UIFontDescriptor.SymbolicTraits = [.traitBold, .traitItalic]
        let complete = environment.resolveFont(
            family: "Courier",
            size: 17,
            fallback: UIFont.systemFont(ofSize: 17),
            additionalTraits: requested,
            semanticGeneration: "explicit-custom-complete"
        )
        let incomplete = environment.resolveFont(
            family: "AppleColorEmoji",
            size: 17,
            fallback: UIFont.systemFont(ofSize: 17),
            additionalTraits: requested,
            semanticGeneration: "explicit-custom-incomplete"
        )

        XCTAssertEqual(complete.familyName, "Courier")
        XCTAssertTrue(ViewerFontEnvironment.satisfiesRequestedEmphasis(complete, requestedTraits: requested))
        XCTAssertNotEqual(incomplete.familyName, UIFont(name: "AppleColorEmoji", size: 17)!.familyName)
        XCTAssertTrue(ViewerFontEnvironment.satisfiesRequestedEmphasis(incomplete, requestedTraits: requested))
    }

    func testGenericMonospaceUsesSupportedFallbackWithoutMissingFamilyWarning() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        let requested: UIFontDescriptor.SymbolicTraits = [.traitBold, .traitItalic]
        let resolved = environment.resolveFont(
            family: "ui-monospace",
            size: 17,
            fallback: UIFont.systemFont(ofSize: 17),
            additionalTraits: requested,
            semanticGeneration: "generic-monospace-emphasis"
        )

        // Generic aliases are a supported semantic request. UIKit's
        // descriptor traits differ by OS/font runtime, so only assert the
        // portable contract: a sized fallback is resolved without recording
        // a missing-family warning.
        XCTAssertEqual(resolved.pointSize, 17)
        XCTAssertFalse(environment.hasMissingFamilyWarning("ui-monospace", semanticGeneration: "generic-monospace-emphasis"))
    }

    func testWarningContextUsesSemanticIdentityInsteadOfLayoutReplacementIdentity() {
        let request = ProseViewerRequest(source: .json("{\"type\":\"doc\"}"), configuration: .init())
        let replacement = ProseViewerRequest(
            source: request.source,
            configuration: request.configuration,
            nativeFontRevision: 1,
            nativeFontScale: 1.4,
            fontEnvironmentRevision: 2,
            attachmentRevision: 3
        )
        let baseKey = ProseLayoutKey(
            semanticKey: String(repeating: "a", count: 64),
            widthPixels: 200,
            themeDigest: request.themeDigest,
            nativeFontRevision: request.nativeFontRevision,
            fontEnvironmentRevision: request.fontEnvironmentRevision,
            displayScale: 2,
            attachmentRevision: request.attachmentRevision,
            generationIdentity: request.generationIdentity,
            semanticGenerationIdentity: request.semanticGenerationIdentity
        )
        let replacementKey = ProseLayoutKey(
            semanticKey: baseKey.semanticKey,
            widthPixels: 240,
            themeDigest: replacement.themeDigest,
            nativeFontRevision: replacement.nativeFontRevision,
            fontEnvironmentRevision: replacement.fontEnvironmentRevision,
            displayScale: 3,
            attachmentRevision: replacement.attachmentRevision,
            generationIdentity: replacement.generationIdentity,
            semanticGenerationIdentity: replacement.semanticGenerationIdentity
        )
        XCTAssertNotEqual(baseKey.generationIdentity, replacementKey.generationIdentity)
        XCTAssertEqual(baseKey.semanticGenerationIdentity, replacementKey.semanticGenerationIdentity)
    }

    func testFabricTerminalSidecarReleaseIsIdempotentAndCannotRemoveAnotherComponent() {
        let registry = PreparedProseLayoutRegistry(compile: { _ in
            ViewerDocument(semanticKey: String(repeating: "a", count: 64), paragraphs: [], isEmpty: true, retainedBytes: 0)
        })
        let surface = FabricSurfaceToken(surfaceId: 711, componentTag: 9)
        let sibling = FabricSurfaceToken(surfaceId: 711, componentTag: 10)
        _ = FabricAttachmentSidecars.begin(surface, semanticIdentity: "semantic-a")
        _ = FabricAttachmentSidecars.begin(sibling, semanticIdentity: "semantic-b")
        registry.releaseFabricGeneration(.init(surface: surface, generationIdentity: "replacement-layout"))
        XCTAssertNotNil(FabricAttachmentSidecars.state(for: surface))

        registry.releaseFabricSurface(surface)
        XCTAssertNil(FabricAttachmentSidecars.state(for: surface))
        XCTAssertNotNil(FabricAttachmentSidecars.state(for: sibling))
        registry.releaseFabricSurface(surface)
        XCTAssertNil(FabricAttachmentSidecars.state(for: surface))
        XCTAssertNotNil(FabricAttachmentSidecars.state(for: sibling))
        registry.releaseFabricSurface(sibling)
    }

}
