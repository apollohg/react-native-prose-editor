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

        XCTAssertTrue(viewer.apply(request: request()))
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

        XCTAssertTrue(viewer.apply(request: request()))
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

        XCTAssertFalse(viewer.apply(request: request(source: "not valid")))
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

    private func request(source: String = "{\"type\":\"doc\"}") -> ProseViewerRequest {
        ProseViewerRequest(
            source: .json(source),
            configuration: ProseViewerConfiguration(
                configJSON: "{}",
                collapsesWhenEmpty: true
            )
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
