import CoreText
import Foundation
import UIKit
import XCTest

final class PreparedProseLayoutTests: XCTestCase {

    let document = ViewerDocument(
        semanticKey: String(repeating: "a", count: 64),
        paragraphs: [ViewerParagraph(text: "One prepared paragraph")],
        isEmpty: false,
        retainedBytes: 64
    )


    func configuration() -> ProseViewerConfiguration {
        ProseViewerConfiguration(configJSON: "{}", collapsesWhenEmpty: true)
    }

    func request(
        source: String = "{\"type\":\"doc\"}",
        attachmentRevision: UInt64 = 0,
        nativeFontRevision: UInt64 = 0,
        fontEnvironmentRevision: UInt64 = 0
    ) -> ProseViewerRequest {
        ProseViewerRequest(
            source: .json(source),
            configuration: configuration(),
            nativeFontRevision: nativeFontRevision,
            fontEnvironmentRevision: fontEnvironmentRevision,
            attachmentRevision: attachmentRevision
        )
    }

    func makeRegistry(
        prepare: @escaping PreparedProseLayoutRegistry.LayoutPreparation
    ) -> PreparedProseLayoutRegistry {
        PreparedProseLayoutRegistry(
            compile: { [document = self.document] _ in document },
            prepare: prepare
        )
    }

    func install(
        _ request: ProseViewerRequest,
        in drawingView: PreparedProseDrawingView,
        surface: FabricSurfaceToken,
        registry: PreparedProseLayoutRegistry,
        width: CGFloat = 160,
        leaseHandle: UInt64 = 1
    ) -> Bool {
        registry.installCachedLayout(
            in: drawingView,
            surfaceId: surface.surfaceId,
            componentTag: surface.componentTag,
            leaseHandle: leaseHandle,
            sourceKind: "json",
            source: request.source.value as NSString,
            configJSON: request.configuration.configJSON as NSString,
            themeJSON: nil,
            imagePolicyJSON: nil,
            imagesEnabled: request.configuration.imagesEnabled,
            collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
            attachmentRevision: request.attachmentRevision,
            nativeFontRevision: request.nativeFontRevision,
            fontEnvironmentRevision: request.fontEnvironmentRevision,
            widthPoints: width,
            scale: 2
        )
    }

    func canonicalFabricGenerationIdentity(
        _ request: ProseViewerRequest,
        registry: PreparedProseLayoutRegistry
    ) -> String {
        registry.fabricGenerationIdentity(
            sourceKind: "json",
            source: request.source.value as NSString,
            configJSON: request.configuration.configJSON as NSString,
            themeJSON: request.configuration.themeJSON as NSString?,
            imagePolicyJSON: request.configuration.imagePolicyJSON as NSString?,
            imagesEnabled: request.configuration.imagesEnabled,
            collapsesWhenEmpty: request.configuration.collapsesWhenEmpty,
            attachmentRevision: request.attachmentRevision,
            nativeFontRevision: request.nativeFontRevision,
            fontEnvironmentRevision: request.fontEnvironmentRevision
        ) as String
    }

    func registerAndActivateFabricGeneration(
        _ request: ProseViewerRequest,
        surface: FabricSurfaceToken,
        registry: PreparedProseLayoutRegistry,
        leaseHandle: UInt64
    ) -> FabricGenerationToken {
        let generation = FabricGenerationToken(
            surface: surface,
            generationIdentity: canonicalFabricGenerationIdentity(request, registry: registry),
            leaseHandle: leaseHandle
        )
        registry.registerFabricLease(
            surfaceId: surface.surfaceId,
            componentTag: surface.componentTag,
            leaseHandle: leaseHandle
        )
        registry.activateFabricGeneration(generation)
        return generation
    }

    final class FailureRecordingDelegate: ProseViewerInteractionDelegate {
        var errors: [ProseViewerError] = []
        var mentions: [ProseViewerMention] = []

        func proseViewer(_ view: ProseViewerView, didTapLink href: String, text: String) {}
        func proseViewer(_ view: ProseViewerView, didTapMention mention: ProseViewerMention) {
            mentions.append(mention)
        }
        func proseViewer(_ view: ProseViewerView, didFail error: ProseViewerError) {
            errors.append(error)
        }
    }
}

// Direct registry tests model the C++ state-family bridge explicitly. Calls
// that omit an opaque handle use the test's canonical H1 family.
extension PreparedProseLayoutRegistry {
    func measure(
        request: ProseViewerRequest,
        widthPoints: CGFloat,
        scale: CGFloat,
        fabricSurface: FabricSurfaceToken
    ) -> PreparedProseLayout {
        registerFabricLease(
            surfaceId: fabricSurface.surfaceId,
            componentTag: fabricSurface.componentTag,
            leaseHandle: 1
        )
        return measure(
            request: request,
            widthPoints: widthPoints,
            scale: scale,
            fabricSurface: fabricSurface,
            fabricLeaseHandle: 1
        )
    }
}
