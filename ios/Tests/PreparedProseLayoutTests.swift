import CoreText
import XCTest

final class PreparedProseLayoutTests: XCTestCase {
    private let document = ViewerDocument(
        semanticKey: String(repeating: "a", count: 64),
        paragraphs: [ViewerParagraph(text: "One prepared paragraph")],
        isEmpty: false,
        retainedBytes: 64
    )

    func testRepeatedFittingAndDrawingAtSamePhysicalWidthPrepareExactlyOnce() {
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
        let viewer = ProseViewerView(layoutRegistry: registry)

        XCTAssertTrue(viewer.apply(source: .json("{\"type\":\"doc\"}"), configuration: configuration()))
        let first = viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude))
        let second = viewer.sizeThatFits(CGSize(width: 160.1, height: .greatestFiniteMagnitude))
        XCTAssertGreaterThan(first.height, 0)
        XCTAssertEqual(first.height, second.height)

        viewer.frame = CGRect(x: 0, y: 0, width: 160, height: 200)
        viewer.layoutIfNeeded()
        viewer.drawingViewForTesting.draw(viewer.drawingViewForTesting.bounds)

        XCTAssertEqual(preparations, 1)
        XCTAssertEqual(registry.layoutPreparationCount, 1)
    }

    func testChangedPhysicalWidthPreparesOneAdditionalArtifact() {
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
        let viewer = ProseViewerView(layoutRegistry: registry)

        XCTAssertTrue(viewer.apply(source: .json("{\"type\":\"doc\"}"), configuration: configuration()))
        _ = viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude))
        _ = viewer.sizeThatFits(CGSize(width: 120, height: .greatestFiniteMagnitude))
        _ = viewer.sizeThatFits(CGSize(width: 120.1, height: .greatestFiniteMagnitude))

        XCTAssertEqual(preparations, 2)
        XCTAssertEqual(registry.layoutPreparationCount, 2)
    }

    func testMalformedInputProducesOneZeroHeightErrorArtifactAndOneDelegateEvent() {
        let registry = PreparedProseLayoutRegistry(
            compile: { _ in
                throw ProseViewerError.compiler(
                    domain: "viewer",
                    code: "MALFORMED_INPUT",
                    message: "Malformed content"
                )
            }
        )
        let viewer = ProseViewerView(layoutRegistry: registry)
        let delegate = FailureRecordingDelegate()
        viewer.interactionDelegate = delegate

        XCTAssertFalse(viewer.apply(source: .json("not valid"), configuration: configuration()))
        XCTAssertEqual(
            viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude)).height,
            0
        )
        XCTAssertEqual(
            viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude)).height,
            0
        )

        XCTAssertEqual(delegate.errors.count, 1)
        XCTAssertEqual(delegate.errors.first?.code, "MALFORMED_INPUT")
        XCTAssertEqual(registry.layoutPreparationCount, 0)
    }

    func testInvalidWidthProducesAnUncachedErrorAndReportsOnceForTheGeneration() {
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
        let viewer = ProseViewerView(layoutRegistry: registry)
        let delegate = FailureRecordingDelegate()
        viewer.interactionDelegate = delegate

        XCTAssertTrue(viewer.apply(source: .json("{\"type\":\"doc\"}"), configuration: configuration()))
        XCTAssertEqual(viewer.sizeThatFits(CGSize(width: .infinity, height: 100)).height, 0)
        XCTAssertEqual(viewer.sizeThatFits(CGSize(width: .infinity, height: 100)).height, 0)
        XCTAssertEqual(delegate.errors.map(\.code), ["INVALID_WIDTH"])
        XCTAssertEqual(preparations, 0)
        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
    }

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
        _ = viewer.sizeThatFits(CGSize(width: 160, height: .greatestFiniteMagnitude))
        guard let mountedArtifact = viewer.drawingViewForTesting.layout else {
            return XCTFail("Measurement should install the prepared artifact in the drawing view.")
        }

        registry.didReceiveMemoryWarning()

        XCTAssertEqual(registry.preparedLayoutCacheCountForTesting, 0)
        XCTAssertTrue(viewer.drawingViewForTesting.layout === mountedArtifact)
    }

    private func configuration() -> ProseViewerConfiguration {
        ProseViewerConfiguration(configJSON: "{}", collapsesWhenEmpty: true)
    }

    private func request(
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

    private func makeRegistry(
        prepare: @escaping PreparedProseLayoutRegistry.LayoutPreparation
    ) -> PreparedProseLayoutRegistry {
        PreparedProseLayoutRegistry(
            compile: { [document = self.document] _ in document },
            prepare: prepare
        )
    }

    private final class FailureRecordingDelegate: ProseViewerInteractionDelegate {
        var errors: [ProseViewerError] = []

        func proseViewer(_ view: ProseViewerView, didTapLink href: String, text: String) {}
        func proseViewer(_ view: ProseViewerView, didTapMention docPos: Int, label: String) {}
        func proseViewer(_ view: ProseViewerView, didFail error: ProseViewerError) {
            errors.append(error)
        }
    }
}
