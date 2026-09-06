import XCTest
import UIKit
import CoreText

extension PreparedProseRevisionTests {
    func testExplicitFontAvailabilityAndDynamicTypeEachPublishOneReplacementRevision() {
        let environment = ViewerFontEnvironment(notificationCenter: .default)
        var revisions: [UInt64] = []
        environment.onInvalidated = { revisions.append($0) }
        environment.invalidateRegisteredFonts()
        NotificationCenter.default.post(name: UIContentSizeCategory.didChangeNotification, object: nil)
        XCTAssertEqual(revisions, [1, 2])
    }

    func testDocumentedCoreTextRegisteredFontNotificationIsWiredAndContentSizeDuplicatesAreIgnored() {
        let center = NotificationCenter()
        let environment = ViewerFontEnvironment(notificationCenter: center)
        center.post(name: ViewerFontEnvironment.registeredFontsDidChangeNotification, object: nil)
        center.post(name: UIContentSizeCategory.didChangeNotification, object: nil, userInfo: [
            UIContentSizeCategory.newValueUserInfoKey: UIContentSizeCategory.accessibilityLarge,
        ])
        center.post(name: UIContentSizeCategory.didChangeNotification, object: nil, userInfo: [
            UIContentSizeCategory.newValueUserInfoKey: UIContentSizeCategory.accessibilityLarge,
        ])
        XCTAssertEqual(environment.revision, 2)
    }

    func testFontScaleChangesResolvedGeometry() {
        let base = PreparedProseTheme.resolve(themeJSON: nil, fontScale: 1)
        let scaled = PreparedProseTheme.resolve(themeJSON: nil, fontScale: 1.4)
        XCTAssertGreaterThan(scaled.paragraph.font.pointSize, base.paragraph.font.pointSize)
    }

    func testPreparedLayoutCacheSeparatesCurrentLightAndDarkAppearance() throws {
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            paragraphs: [ViewerParagraph(text: "Appearance")],
            isEmpty: false,
            retainedBytes: 64
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })
        let lightTraits = UITraitCollection(userInterfaceStyle: .light)
        let darkTraits = UITraitCollection(userInterfaceStyle: .dark)
        var light: PreparedProseLayout!
        lightTraits.performAsCurrent {
            light = registry.measure(
                request: ProseViewerRequest(source: .json("{}"), configuration: .init()),
                widthPoints: 160,
                scale: 2
            )
        }
        var dark: PreparedProseLayout!
        darkTraits.performAsCurrent {
            dark = registry.measure(
                request: ProseViewerRequest(source: .json("{}"), configuration: .init()),
                widthPoints: 160,
                scale: 2
            )
        }

        XCTAssertNotEqual(light.key.generationIdentity, dark.key.generationIdentity)
        XCTAssertEqual(light.key.semanticGenerationIdentity, dark.key.semanticGenerationIdentity)
        XCTAssertEqual(try foregroundColor(in: light), UIColor.label.resolvedColor(with: lightTraits))
        XCTAssertEqual(try foregroundColor(in: dark), UIColor.label.resolvedColor(with: darkTraits))
    }

    func testPreparedLayoutCacheSeparatesNormalAndHighContrastAppearance() {
        let document = ViewerDocument(
            semanticKey: String(repeating: "c", count: 64),
            paragraphs: [ViewerParagraph(text: "Contrast")],
            isEmpty: false,
            retainedBytes: 64
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })
        let normalTraits = UITraitCollection(traitsFrom: [
            UITraitCollection(userInterfaceStyle: .light),
            UITraitCollection(accessibilityContrast: .normal)
        ])
        let highTraits = UITraitCollection(traitsFrom: [
            UITraitCollection(userInterfaceStyle: .light),
            UITraitCollection(accessibilityContrast: .high)
        ])
        var normal: PreparedProseLayout!
        normalTraits.performAsCurrent {
            normal = registry.measure(
                request: ProseViewerRequest(source: .json("{}"), configuration: .init()),
                widthPoints: 160,
                scale: 2
            )
        }
        var high: PreparedProseLayout!
        highTraits.performAsCurrent {
            high = registry.measure(
                request: ProseViewerRequest(source: .json("{}"), configuration: .init()),
                widthPoints: 160,
                scale: 2
            )
        }

        XCTAssertNotEqual(normal.key.generationIdentity, high.key.generationIdentity)
        XCTAssertEqual(normal.key.semanticGenerationIdentity, high.key.semanticGenerationIdentity)
        XCTAssertEqual(
            ProseViewerAppearance(
                userInterfaceStyle: .light,
                accessibilityContrast: .high
            ).traits.accessibilityContrast,
            .high
        )
    }

    func testFabricAppearanceSeparatesLayoutGenerationButNotImagePublication() {
        let registry = PreparedProseLayoutRegistry(compile: { _ in
            ViewerDocument(
                semanticKey: String(repeating: "a", count: 64),
                paragraphs: [],
                isEmpty: true,
                retainedBytes: 0
            )
        })
        func generation(style: UIUserInterfaceStyle) -> String {
            registry.fabricGenerationIdentity(
                sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil,
                imagePolicyJSON: nil, imagesEnabled: true, collapsesWhenEmpty: true,
                attachmentRevision: 0, nativeFontRevision: 0, nativeFontScale: 1,
                fontEnvironmentRevision: 0, userInterfaceStyle: style.rawValue
            ) as String
        }
        func semanticGeneration(style: UIUserInterfaceStyle) -> String {
            registry.fabricSemanticGenerationIdentity(
                sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil,
                imagePolicyJSON: nil, imagesEnabled: true, collapsesWhenEmpty: true,
                attachmentRevision: 0, nativeFontRevision: 0, nativeFontScale: 1,
                fontEnvironmentRevision: 0, userInterfaceStyle: style.rawValue
            ) as String
        }

        XCTAssertNotEqual(generation(style: .light), generation(style: .dark))
        XCTAssertEqual(semanticGeneration(style: .light), semanticGeneration(style: .dark))
    }

    func testFabricContrastSeparatesLayoutGenerationButNotImagePublication() {
        let registry = PreparedProseLayoutRegistry(compile: { _ in
            ViewerDocument(
                semanticKey: String(repeating: "c", count: 64),
                paragraphs: [],
                isEmpty: true,
                retainedBytes: 0
            )
        })
        func generation(contrast: UIAccessibilityContrast) -> String {
            registry.fabricGenerationIdentity(
                sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil,
                imagePolicyJSON: nil, imagesEnabled: true, collapsesWhenEmpty: true,
                attachmentRevision: 0, nativeFontRevision: 0, nativeFontScale: 1,
                fontEnvironmentRevision: 0, userInterfaceStyle: UIUserInterfaceStyle.light.rawValue,
                accessibilityContrast: contrast.rawValue
            ) as String
        }
        func semanticGeneration(contrast: UIAccessibilityContrast) -> String {
            registry.fabricSemanticGenerationIdentity(
                sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil,
                imagePolicyJSON: nil, imagesEnabled: true, collapsesWhenEmpty: true,
                attachmentRevision: 0, nativeFontRevision: 0, nativeFontScale: 1,
                fontEnvironmentRevision: 0, userInterfaceStyle: UIUserInterfaceStyle.light.rawValue,
                accessibilityContrast: contrast.rawValue
            ) as String
        }

        XCTAssertNotEqual(generation(contrast: .normal), generation(contrast: .high))
        XCTAssertEqual(
            semanticGeneration(contrast: .normal),
            semanticGeneration(contrast: .high)
        )
    }

    func testDirectViewerRepreparesWhenItsColorAppearanceChanges() throws {
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            paragraphs: [ViewerParagraph(text: "Appearance")],
            isEmpty: false,
            retainedBytes: 64
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })
        let viewer = ProseViewerView(
            frame: CGRect(x: 0, y: 0, width: 160, height: 80),
            layoutRegistry: registry
        )
        let window = UIWindow(frame: viewer.bounds)
        let host = UIViewController()
        window.rootViewController = host
        host.view.addSubview(viewer)
        window.isHidden = false
        defer { window.isHidden = true }
        viewer.overrideUserInterfaceStyle = .light
        flushMain(until: { viewer.traitCollection.userInterfaceStyle == .light })
        XCTAssertEqual(viewer.traitCollection.userInterfaceStyle, .light)
        XCTAssertTrue(viewer.apply(source: .json("{}"), configuration: .init()))
        _ = viewer.sizeThatFits(CGSize(width: 160, height: 80))
        let light = try XCTUnwrap(viewer.drawingViewForTesting.layout)

        let previousTraits = UITraitCollection(userInterfaceStyle: .light)
        viewer.overrideUserInterfaceStyle = .dark
        flushMain(until: { viewer.traitCollection.userInterfaceStyle == .dark })
        viewer.traitCollectionDidChange(previousTraits)
        XCTAssertEqual(viewer.traitCollection.userInterfaceStyle, .dark)
        _ = viewer.sizeThatFits(CGSize(width: 160, height: 80))
        let dark = try XCTUnwrap(viewer.drawingViewForTesting.layout)

        XCTAssertFalse(light === dark)
        XCTAssertEqual(
            try foregroundColor(in: dark),
            UIColor.label.resolvedColor(with: UITraitCollection(userInterfaceStyle: .dark))
        )
    }

    func testFabricInitialFontSnapshotUsesPublishedScaleBeforeFirstNativeRevision() {
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            paragraphs: [ViewerParagraph(text: "The initial Fabric font snapshot must alter geometry.")],
            isEmpty: false,
            retainedBytes: 128
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })

        func measure(nativeFontScale: CGFloat, leaseHandle: UInt64) -> CGSize {
            let generation = registry.fabricGenerationIdentity(
                sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
                imagesEnabled: true, collapsesWhenEmpty: true,
                attachmentRevision: 0, nativeFontRevision: 0, nativeFontScale: nativeFontScale,
                fontEnvironmentRevision: 0
            )
            registry.registerFabricLease(surfaceId: 41, componentTag: 7, leaseHandle: leaseHandle)
            registry.activateFabricGeneration(
                surfaceId: 41, componentTag: 7, generationIdentity: generation, leaseHandle: leaseHandle
            )
            return registry.measure(
                surfaceId: 41, componentTag: 7, leaseHandle: leaseHandle,
                sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
                imagesEnabled: true, collapsesWhenEmpty: true,
                attachmentRevision: 0, nativeFontRevision: 0, nativeFontScale: nativeFontScale,
                fontEnvironmentRevision: 0, widthPoints: 120, scale: 2
            )
        }

        let base = measure(nativeFontScale: 1, leaseHandle: 1)
        let scaled = measure(nativeFontScale: 1.6, leaseHandle: 2)

        XCTAssertGreaterThan(scaled.height, base.height)
    }

    func testFabricNativeRevisionUsesPublishedScaleForReplacementGeometry() {
        let document = ViewerDocument(
            semanticKey: String(repeating: "a", count: 64),
            paragraphs: [ViewerParagraph(text: "A prepared Fabric font scale must alter geometry.")],
            isEmpty: false,
            retainedBytes: 128
        )
        let registry = PreparedProseLayoutRegistry(compile: { _ in document })
        let baseGeneration = registry.fabricGenerationIdentity(
            sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
            imagesEnabled: true, collapsesWhenEmpty: true,
            attachmentRevision: 0, nativeFontRevision: 1, nativeFontScale: 1,
            fontEnvironmentRevision: 0
        )
        registry.registerFabricLease(surfaceId: 42, componentTag: 7, leaseHandle: 1)
        registry.activateFabricGeneration(
            surfaceId: 42, componentTag: 7, generationIdentity: baseGeneration, leaseHandle: 1
        )
        let base = registry.measure(
            surfaceId: 42, componentTag: 7, leaseHandle: 1,
            sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
            imagesEnabled: true, collapsesWhenEmpty: true,
            attachmentRevision: 0, nativeFontRevision: 1, nativeFontScale: 1,
            fontEnvironmentRevision: 0, widthPoints: 120, scale: 2
        )
        let replacementGeneration = registry.fabricGenerationIdentity(
            sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
            imagesEnabled: true, collapsesWhenEmpty: true,
            attachmentRevision: 0, nativeFontRevision: 2, nativeFontScale: 1.6,
            fontEnvironmentRevision: 0
        )
        registry.registerFabricLease(surfaceId: 42, componentTag: 7, leaseHandle: 2)
        registry.activateFabricGeneration(
            surfaceId: 42, componentTag: 7, generationIdentity: replacementGeneration, leaseHandle: 2
        )
        let replacement = registry.measure(
            surfaceId: 42, componentTag: 7, leaseHandle: 2,
            sourceKind: "json", source: "{}", configJSON: "{}", themeJSON: nil, imagePolicyJSON: nil,
            imagesEnabled: true, collapsesWhenEmpty: true,
            attachmentRevision: 0, nativeFontRevision: 2, nativeFontScale: 1.6,
            fontEnvironmentRevision: 0, widthPoints: 120, scale: 2
        )
        XCTAssertGreaterThan(replacement.height, base.height)
    }

    func testResourceFailureIsPublishedOncePerGenerationAndAttachmentWithoutSource() {
        let state = ViewerAttachmentRevisionState()
        XCTAssertTrue(state.beginSemanticGeneration("resource"))
        state.admit(attachmentCount: 1)
        XCTAssertTrue(state.recordResourceFailure(for: 0))
        XCTAssertFalse(state.recordResourceFailure(for: 0))
        // A Fabric attachment-revision reinstall cancels/reconfigures requests,
        // but remains in the same semantic generation.
        XCTAssertFalse(state.beginSemanticGeneration("resource"))
        XCTAssertFalse(state.recordResourceFailure(for: 0))
    }

}
