import CoreText
import Foundation
import UIKit
import XCTest

extension PreparedProseLayoutTests {
    func testRevisionVariantsUseDistinctPreparedArtifacts() {
        var preparations = 0
        let registry = makeRegistry { document, key, width, scale in
            preparations += 1
            return try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let base = request()
        let attachmentChange = request(attachmentRevision: 1)
        let nativeFontChange = request(nativeFontRevision: 1)
        let environmentChange = request(fontEnvironmentRevision: 1)

        _ = registry.measure(request: base, widthPoints: 160, scale: 2)
        _ = registry.measure(request: attachmentChange, widthPoints: 160, scale: 2)
        _ = registry.measure(request: nativeFontChange, widthPoints: 160, scale: 2)
        _ = registry.measure(request: environmentChange, widthPoints: 160, scale: 2)

        XCTAssertEqual(preparations, 4)
        XCTAssertEqual(registry.layoutPreparationCount, 4)
    }

    func testClampedDrawingUsesViewBoundsForTheCoreTextCoordinateSystem() {
        let artifactHeight: CGFloat = 120
        let baselineFromArtifactTop: CGFloat = 18
        let clampedBounds = CGRect(x: 0, y: 0, width: 160, height: 40)

        XCTAssertEqual(
            PreparedProseDrawingView.textPosition(
                baselineFromArtifactTop: baselineFromArtifactTop,
                in: clampedBounds,
                artifactHeight: artifactHeight
            ).y,
            22
        )
    }

    func testCompiledDocumentsAreBudgetedAndEvictedInAccessOrder() throws {
        var compilations = 0
        let registry = PreparedProseLayoutRegistry(
            compiledByteBudget: 100,
            compile: { request in
                compilations += 1
                return ViewerDocument(
                    semanticKey: String(repeating: request.source.value == "first" ? "a" : "b", count: 64),
                    paragraphs: [ViewerParagraph(text: request.source.value)],
                    isEmpty: false,
                    retainedBytes: 40
                )
            }
        )
        let first = ProseViewerRequest(source: .json("first"), configuration: configuration())
        let second = ProseViewerRequest(source: .json("second"), configuration: configuration())
        let third = ProseViewerRequest(source: .json("third"), configuration: configuration())

        _ = try registry.compileDocument(request: first)
        _ = try registry.compileDocument(request: second)
        _ = try registry.compileDocument(request: first)
        _ = try registry.compileDocument(request: third)
        _ = try registry.compileDocument(request: first)
        _ = try registry.compileDocument(request: second)

        XCTAssertEqual(compilations, 4)
        XCTAssertEqual(registry.compiledDocumentBytesForTesting, 80)
    }

    func testMemoryWarningReleasesCacheOwnershipWithoutReleasingMountedArtifact() {
        let registry = makeRegistry { document, key, width, scale in
            try CoreTextProseLayoutEngine().prepare(
                document: document,
                key: key,
                widthPoints: width,
                displayScale: scale
            )
        }
        let viewer = ProseViewerView(layoutRegistry: registry)

        XCTAssertTrue(viewer.apply(source: .json("{\"type\":\"doc\"}"), configuration: configuration()))
        _ = viewer.sizeThatFits(CGSize(width: 160, height: CGFloat.greatestFiniteMagnitude))
        guard let mountedArtifact = viewer.drawingViewForTesting.layout else {
            return XCTFail("Measurement should install the prepared artifact in the drawing view.")
        }

        registry.didReceiveMemoryWarning()

        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
        XCTAssertTrue(viewer.drawingViewForTesting.layout === mountedArtifact)
    }

}
