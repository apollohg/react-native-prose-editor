import XCTest
import UIKit

struct PreparedProseBenchmarkCorpus: Decodable {
    struct Entry: Decodable { let id: String; let category: String; let contentJSON: [String: JSONValue] }
    struct WarmWindow: Decodable { let id: String; let primeIds: [String]; let warmIds: [String] }
    let documents: [Entry]
    let coldTraversal: [String]
    let warmTraversal: [String]
    let warmWindows: [WarmWindow]

    static func load() throws -> Self {
        guard let url = Bundle(for: NativePerformanceTests.self).url(
            forResource: "viewer-performance-corpus", withExtension: "json"
        ) else { throw NSError(domain: "PreparedProseBenchmarkCorpus", code: 1, userInfo: [NSLocalizedDescriptionKey: "Bundled viewer performance corpus is missing."]) }
        let data = try Data(contentsOf: url)
        let corpus = try JSONDecoder().decode(Self.self, from: data)
        XCTAssertEqual(corpus.documents.count, 1_000)
        XCTAssertEqual(Set(corpus.documents.map(\.id)).count, 1_000)
        XCTAssertEqual(corpus.coldTraversal.count, 1_000)
        XCTAssertEqual(corpus.warmTraversal.count, 1_000)
        XCTAssertEqual(corpus.warmWindows.count, 27)
        return corpus
    }
}

/// This fixture is the one complete configuration shared by the iOS,
/// Android, and FlatList harnesses. The corpus intentionally contains node
/// kinds beyond the default schema, so an empty configuration is invalid.
struct PreparedProseBenchmarkConfiguration: Decodable {
    let configuration: JSONValue
    let imageLoadingPolicy: JSONValue

    static func load() throws -> Self {
        guard let url = Bundle(for: NativePerformanceTests.self).url(
            forResource: "prepared-prose-benchmark-config", withExtension: "json"
        ) else { throw NSError(domain: "PreparedProseBenchmarkConfiguration", code: 1, userInfo: [NSLocalizedDescriptionKey: "Bundled prepared prose benchmark configuration is missing."]) }
        return try JSONDecoder().decode(Self.self, from: Data(contentsOf: url))
    }

    func viewerConfiguration(imagesEnabled: Bool) throws -> ProseViewerConfiguration {
        ProseViewerConfiguration(
            configJSON: String(data: try JSONEncoder().encode(configuration), encoding: .utf8) ?? "{}",
            imagePolicyJSON: String(data: try JSONEncoder().encode(imageLoadingPolicy), encoding: .utf8),
            imagesEnabled: imagesEnabled,
            collapsesWhenEmpty: true
        )
    }
}

struct PreparedProseHarnessStaticFixtures: Decodable {
    struct Preparation: Decodable { let entryId: String; let widthPoints: CGFloat }
    struct DifferingHeights: Decodable { let shortEntryId: String; let longEntryId: String; let widthPoints: CGFloat }
    struct DrawEvidence: Decodable { let phase: String }
    struct FrameClassification: Decodable {
        let nominalFramePeriodNanos: UInt64
        let singleTickToleranceNanos: UInt64
        let oneTickDeltasNanos: [UInt64]
        let delayedDeltaNanos: UInt64
    }
    let preparation: Preparation
    let differingHeights: DifferingHeights
    let drawEvidence: DrawEvidence
    let frameClassification: FrameClassification

    static func load() throws -> Self {
        guard let url = Bundle(for: NativePerformanceTests.self).url(
            forResource: "prepared-prose-harness-static-fixtures", withExtension: "json"
        ) else { throw NSError(domain: "PreparedProseHarnessStaticFixtures", code: 1, userInfo: [NSLocalizedDescriptionKey: "Bundled prepared prose harness fixtures are missing."]) }
        return try JSONDecoder().decode(Self.self, from: Data(contentsOf: url))
    }
}

struct PreparedProseBenchmarkExportContract: Decodable {
    struct CacheSnapshot: Decodable {
        let unmountedCurrentBytes: Int
        let unmountedHighWaterBytes: Int
        let unmountedCurrentResidentCount: Int
        let unmountedHighWaterResidentCount: Int
        let compiledCurrentBytes: Int
        let compiledCurrentResidentCount: Int
    }
    struct Phase: Decodable {
        let imageRequestCount: Int
        let imageMetadataCount: Int
        let imageDecodeCount: Int
        let rawFrameDeltasNanos: [UInt64]
        let nominalFrameCount: Int
        let onTimeNominalFrameCount: Int
        let frameCallbackSamples: [FrameCallbackSample]
        struct FrameCallbackSample: Decodable {
            let callbackDurationNanos: UInt64
            let targetLeadNanos: Int64
        }
    }
    struct PhaseSamples: Decodable {
        let cold: Phase
        let warm: Phase
        let imagesDisabled: Phase
    }
    struct WindowEvidence: Decodable {
        let windowId: String
        let phase: String
    }
    let schemaVersion: Int
    let nominalFramePeriodNanos: UInt64
    let singleTickToleranceNanos: UInt64
    let phaseSamples: PhaseSamples
    let windowEvidence: [WindowEvidence]
    let preResetSnapshot: CacheSnapshot
    let postResetSnapshot: CacheSnapshot
}

enum JSONValue: Codable {
    case string(String), number(Double), bool(Bool), object([String: JSONValue]), array([JSONValue]), null
    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() { self = .null } else if let value = try? container.decode(Bool.self) { self = .bool(value) } else if let value = try? container.decode(Double.self) { self = .number(value) } else if let value = try? container.decode(String.self) { self = .string(value) } else if let value = try? container.decode([String: JSONValue].self) { self = .object(value) } else { self = .array(try container.decode([JSONValue].self)) }
    }
    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case let .string(value): try container.encode(value)
        case let .number(value): try container.encode(value)
        case let .bool(value): try container.encode(value)
        case let .object(value): try container.encode(value)
        case let .array(value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
}

final class PreparedProseCollectionHarness: NSObject, UICollectionViewDataSource, UICollectionViewDelegate {
    struct WindowPhaseResult {
        let residentKeys: [ProseLayoutKey]
        let residentKeyCount: Int
        let residentKeyDigest: String
        let compileCount: Int
        let layoutCount: Int
        let cacheMisses: Int
    }

    struct WindowTraversalResult {
        let windowId: String
        let prime: WindowPhaseResult
        let warm: WindowPhaseResult
        let renderedHeight: CGFloat
        let preparedArtifactHeight: CGFloat
    }

    private enum Direction { case forward, reverse }
    private struct CounterBaseline {
        let compileCount: Int
        let layoutCount: Int
        let cacheMisses: Int
    }
    private struct ActiveTraversal {
        let windows: [PreparedProseBenchmarkCorpus.WarmWindow]
        let phase: PreparedProseInstrumentation.TraversalPhase
        let imagesEnabled: Bool
        let completion: (Result<[WindowTraversalResult], Error>) -> Void
        var index = 0
        var direction: Direction = .forward
        var prime: WindowPhaseResult?
        var results: [WindowTraversalResult] = []
        var counterBaseline = CounterBaseline(compileCount: 0, layoutCount: 0, cacheMisses: 0)
    }

    let corpus: PreparedProseBenchmarkCorpus
    let configuration: PreparedProseBenchmarkConfiguration
    private let defaultImagesEnabled: Bool
    private let byID: [String: PreparedProseBenchmarkCorpus.Entry]
    private let sourceByID: [String: String]
    private let collectionView: UICollectionView
    let window: UIWindow
    private var orderedEntries: [PreparedProseBenchmarkCorpus.Entry] = []
    private var activeImagesEnabled = true
    private var activeViewerConfiguration: ProseViewerConfiguration?
    var displayLink: CADisplayLink?
    private var traversal: ActiveTraversal?

    init(corpus: PreparedProseBenchmarkCorpus, configuration: PreparedProseBenchmarkConfiguration, imagesEnabled: Bool) {
        self.corpus = corpus; self.configuration = configuration; self.defaultImagesEnabled = imagesEnabled
        byID = Dictionary(uniqueKeysWithValues: corpus.documents.map { ($0.id, $0) })
        sourceByID = Dictionary(uniqueKeysWithValues: corpus.documents.map { entry in
            guard let data = try? JSONEncoder().encode(entry.contentJSON),
                  let source = String(data: data, encoding: .utf8)
            else { preconditionFailure("invalid corpus entry \(entry.id)") }
            return (entry.id, source)
        })
        let layout = UICollectionViewFlowLayout()
        layout.estimatedItemSize = UICollectionViewFlowLayout.automaticSize
        layout.minimumLineSpacing = 8
        layout.sectionInset = .zero
        collectionView = UICollectionView(frame: CGRect(x: 0, y: 0, width: 390, height: 844), collectionViewLayout: layout)
        window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
        super.init()
        collectionView.dataSource = self; collectionView.delegate = self
        collectionView.register(PreparedProseCollectionCell.self, forCellWithReuseIdentifier: "prepared")
        let host = UIViewController(); host.view = collectionView; window.rootViewController = host; window.isHidden = false
    }
    deinit { displayLink?.invalidate(); window.isHidden = true }
    func resetCache() { PreparedProseLayoutRegistry.shared.didReceiveMemoryWarning() }
    var hasMountedPreparedViewer: Bool {
        collectionView.visibleCells.contains { ($0 as? PreparedProseCollectionCell)?.hasPreparedArtifact == true }
    }

    func traverseWindows(
        _ windows: [PreparedProseBenchmarkCorpus.WarmWindow],
        phase: PreparedProseInstrumentation.TraversalPhase,
        imagesEnabled: Bool? = nil,
        completion: @escaping (Result<[WindowTraversalResult], Error>) -> Void
    ) {
        guard traversal == nil else {
            completion(.failure(NSError(domain: "PreparedProseCollectionHarness", code: 1, userInfo: [NSLocalizedDescriptionKey: "a traversal is already active"])))
            return
        }
        guard phase == .cold || phase == .imagesDisabled else {
            completion(.failure(NSError(domain: "PreparedProseCollectionHarness", code: 2, userInfo: [NSLocalizedDescriptionKey: "window traversal begins with cold or imagesDisabled"])))
            return
        }
        traversal = .init(
            windows: windows,
            phase: phase,
            imagesEnabled: imagesEnabled ?? defaultImagesEnabled,
            completion: completion
        )
        startCurrentWindow()
    }

    private func startCurrentWindow() {
        guard var traversal, traversal.index < traversal.windows.count else {
            finishTraversal()
            return
        }
        let window = traversal.windows[traversal.index]
        traversal.direction = .forward
        traversal.prime = nil
        self.traversal = traversal
        beginWindowPass(
            phase: traversal.phase,
            preservingDisplayLinkBaseline: displayLink != nil
        )
        orderedEntries = window.primeIds.compactMap { byID[$0] }
        guard orderedEntries.count == window.primeIds.count else {
            finishTraversal(error: NSError(domain: "PreparedProseCollectionHarness", code: 3, userInfo: [NSLocalizedDescriptionKey: "window \(window.id) references an unknown entry"]))
            return
        }
        activeImagesEnabled = traversal.imagesEnabled
        do {
            activeViewerConfiguration = try configuration.viewerConfiguration(imagesEnabled: activeImagesEnabled)
        } catch {
            finishTraversal(error: error)
            return
        }
        recordLifecycleWork {
            collectionView.contentOffset = .zero
            collectionView.reloadData()
        }
    }

    private func beginWindowPass(
        phase: PreparedProseInstrumentation.TraversalPhase,
        preservingDisplayLinkBaseline: Bool = false
    ) {
        let seededKeys: [ProseLayoutKey]
        if let traversal, traversal.direction == .reverse {
            seededKeys = traversal.prime?.residentKeys ?? []
        } else {
            seededKeys = []
        }
        PreparedProseLayoutRegistry.shared.beginBenchmarkResidentCensus(seeding: seededKeys)
        if preservingDisplayLinkBaseline {
            PreparedProseInstrumentation.transitionPhase(phase)
        } else {
            PreparedProseInstrumentation.beginPhase(phase)
        }
        if var traversal {
            let counters = PreparedProseInstrumentation.phaseCounters()
            traversal.counterBaseline = .init(
                compileCount: counters.compileCount,
                layoutCount: counters.layoutCount,
                cacheMisses: counters.cacheMisses
            )
            self.traversal = traversal
        }
        if displayLink == nil {
            let link = CADisplayLink(target: self, selector: #selector(displayLinkTick(_:)))
            PreparedProseInstrumentation.configureBenchmarkCadence(link)
            displayLink = link
            link.add(to: .main, forMode: .common)
        }
    }

    @objc private func displayLinkTick(_ link: CADisplayLink) {
        PreparedProseInstrumentation.displayLinkDidTick(link)
        driveCurrentWindow(with: link)
    }

    private func driveCurrentWindow(with link: CADisplayLink) {
        guard let traversal, !orderedEntries.isEmpty else { return }
        let maximumOffset = max(0, collectionView.contentSize.height - collectionView.bounds.height)
        if maximumOffset == 0 {
            guard collectionView.indexPathsForVisibleItems.contains(IndexPath(item: 0, section: 0)) else { return }
            finishCurrentWindowPass()
            return
        }
        let distance = CGFloat(max(1, 2_000 * link.duration))
        let currentOffset = collectionView.contentOffset.y
        let target: CGFloat
        switch traversal.direction {
        case .forward:
            target = min(maximumOffset, currentOffset + distance)
        case .reverse:
            target = max(0, currentOffset - distance)
        }
        recordLifecycleWork {
            collectionView.contentOffset = CGPoint(x: 0, y: target)
        }

        let destination = traversal.direction == .forward ? orderedEntries.count - 1 : 0
        guard target == (traversal.direction == .forward ? maximumOffset : 0),
              collectionView.indexPathsForVisibleItems.contains(IndexPath(item: destination, section: 0))
        else { return }
        finishCurrentWindowPass()
    }

    private func finishCurrentWindowPass() {
        guard var traversal else { return }
        let window = traversal.windows[traversal.index]
        let census = PreparedProseLayoutRegistry.shared.endBenchmarkResidentCensus()
        let counters = PreparedProseInstrumentation.phaseCounters()
        let result = WindowPhaseResult(
            residentKeys: census.keys,
            residentKeyCount: census.count,
            residentKeyDigest: census.digest,
            compileCount: counters.compileCount - traversal.counterBaseline.compileCount,
            layoutCount: counters.layoutCount - traversal.counterBaseline.layoutCount,
            cacheMisses: counters.cacheMisses - traversal.counterBaseline.cacheMisses
        )
        let phase: PreparedProseInstrumentation.TraversalPhase =
            traversal.direction == .reverse && traversal.phase == .cold ? .warm : traversal.phase
        PreparedProseInstrumentation.recordWindow(
            windowId: window.id,
            entryIds: traversal.direction == .forward ? window.primeIds : window.warmIds,
            phase: phase,
            residentKeyCount: result.residentKeyCount,
            residentKeyDigest: result.residentKeyDigest,
            cache: PreparedProseInstrumentation.snapshotCache(),
            counters: (result.compileCount, result.layoutCount, result.cacheMisses)
        )
        if traversal.direction == .forward {
            traversal.prime = result
            traversal.direction = .reverse
            self.traversal = traversal
            beginWindowPass(
                phase: traversal.phase == .cold ? .warm : traversal.phase,
                preservingDisplayLinkBaseline: true
            )
            return
        }

        let warm = result
        let visible = collectionView.cellForItem(at: IndexPath(item: 0, section: 0)) as? PreparedProseCollectionCell
        guard let visible else {
            finishTraversal(error: NSError(domain: "PreparedProseCollectionHarness", code: 4, userInfo: [NSLocalizedDescriptionKey: "leading cell was not attached at window completion"]))
            return
        }
        traversal.results.append(
            .init(
                windowId: window.id,
                prime: traversal.prime ?? result,
                warm: warm,
                renderedHeight: visible.bounds.height,
                preparedArtifactHeight: visible.preparedArtifactHeight
            )
        )
        traversal.index += 1
        self.traversal = traversal
        startCurrentWindow()
    }

    private func finishTraversal(error: Error? = nil) {
        displayLink?.invalidate()
        displayLink = nil
        guard let traversal else { return }
        PreparedProseInstrumentation.endPhase()
        self.traversal = nil
        if let error {
            traversal.completion(.failure(error))
        } else {
            traversal.completion(.success(traversal.results))
        }
    }

    private func recordLifecycleWork(_ work: () -> Void) {
        let startNanos = PreparedProseInstrumentation.now()
        work()
        PreparedProseInstrumentation.recordViewerWork(
            startNanos: startNanos,
            endNanos: PreparedProseInstrumentation.now(),
            kind: .lifecycle
        )
    }

    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int { orderedEntries.count }
    func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
        let cell = collectionView.dequeueReusableCell(withReuseIdentifier: "prepared", for: indexPath) as! PreparedProseCollectionCell
        let entry = orderedEntries[indexPath.item]
        guard let source = sourceByID[entry.id], let activeViewerConfiguration else {
            XCTFail("missing stable benchmark input for \(entry.id)")
            return cell
        }
        do {
            try cell.configure(
                source: source,
                configuration: activeViewerConfiguration
            )
        } catch {
            XCTFail("invalid benchmark configuration: \(error)")
        }
        return cell
    }
}

private final class PreparedProseCollectionCell: UICollectionViewCell {
    let viewer = ProseViewerView()
    var preparedArtifactHeight: CGFloat = 0
    var hasPreparedArtifact: Bool { preparedArtifactHeight > 0 }
    override init(frame: CGRect) {
        super.init(frame: frame)
        viewer.translatesAutoresizingMaskIntoConstraints = false
        contentView.addSubview(viewer)
        NSLayoutConstraint.activate([
            viewer.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            viewer.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            viewer.topAnchor.constraint(equalTo: contentView.topAnchor),
            viewer.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
        ])
    }
    required init?(coder: NSCoder) { fatalError("PreparedProseCollectionCell is programmatic") }
    override func prepareForReuse() {
        let startNanos = PreparedProseInstrumentation.now()
        defer {
            PreparedProseInstrumentation.recordViewerWork(
                startNanos: startNanos,
                endNanos: PreparedProseInstrumentation.now(),
                kind: .lifecycle
            )
        }
        super.prepareForReuse()
        preparedArtifactHeight = 0
        viewer.prepareForReuse()
    }
    func configure(source: String, configuration: ProseViewerConfiguration) throws {
        let startNanos = PreparedProseInstrumentation.now()
        defer {
            PreparedProseInstrumentation.recordViewerWork(
                startNanos: startNanos,
                endNanos: PreparedProseInstrumentation.now(),
                kind: .lifecycle
            )
        }
        guard viewer.apply(source: .json(source), configuration: configuration) else {
            throw NSError(domain: "PreparedProseCollectionCell", code: 1, userInfo: [NSLocalizedDescriptionKey: "benchmark source was rejected"])
        }
        setNeedsLayout()
    }
    override func preferredLayoutAttributesFitting(_ attributes: UICollectionViewLayoutAttributes) -> UICollectionViewLayoutAttributes {
        let startNanos = PreparedProseInstrumentation.now()
        defer {
            PreparedProseInstrumentation.recordViewerWork(
                startNanos: startNanos,
                endNanos: PreparedProseInstrumentation.now(),
                kind: .lifecycle
            )
        }
        let fitted = attributes.copy() as! UICollectionViewLayoutAttributes
        let width = max(1, attributes.size.width)
        preparedArtifactHeight = max(1, ceil(viewer.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height))
        fitted.size = CGSize(width: width, height: preparedArtifactHeight)
        return fitted
    }
}

enum PreparedProsePerformanceGates {
    private struct DelayedInterval: Decodable { let rawDeltaNanos: UInt64 }
    private struct CacheSnapshot: Decodable { let unmountedCurrentBytes: Int; let unmountedHighWaterBytes: Int; let unmountedCurrentResidentCount: Int; let unmountedHighWaterResidentCount: Int; let compiledCurrentBytes: Int; let compiledCurrentResidentCount: Int }
    private struct WindowEvidence: Decodable { let windowId: String; let entryIds: [String]; let phase: String; let residentKeyCount: Int; let compileCount: Int; let layoutCount: Int; let cacheMisses: Int }
    private struct Phase: Decodable { let combinedCompileLayoutNanos: [UInt64]; let cacheLookupNanos: [UInt64]; let drawNanos: [UInt64]; let rawFrameDeltasNanos: [UInt64]; let frameCallbackSamples: [FrameCallbackSample]; let nominalFrameCount: Int; let onTimeNominalFrameCount: Int; let viewerCausedDelayedIntervals: [DelayedInterval]; let imageRequestCount: Int; let imageMetadataCount: Int; let imageDecodeCount: Int; let drawCount: Int }
    private struct FrameCallbackSample: Decodable { let callbackDurationNanos: UInt64; let targetLeadNanos: Int64 }
    private struct Export: Decodable { let percentileDefinition: String; let phaseSamples: [String: Phase]; let windowEvidence: [WindowEvidence]; let preResetSnapshot: CacheSnapshot; let postResetSnapshot: CacheSnapshot; let duplicatePublications: Int }
    static func assertPasses(exportJSON: String, expectedDocuments: Int) throws {
        let export = try JSONDecoder().decode(Export.self, from: Data(exportJSON.utf8))
        guard let cold = export.phaseSamples["cold"], let warm = export.phaseSamples["warm"], let imagesDisabled = export.phaseSamples["imagesDisabled"] else { XCTFail("every traversal phase must export samples"); return }
        requireNonEmpty(cold.combinedCompileLayoutNanos, "cold compile+layout")
        requireNonEmpty(cold.cacheLookupNanos, "cold cache lookup")
        requireNonEmpty(cold.drawNanos, "cold draw")
        XCTAssertGreaterThanOrEqual(cold.combinedCompileLayoutNanos.count, expectedDocuments)
        XCTAssertLessThan(percentile(cold.combinedCompileLayoutNanos, 0.95), 4_000_000)
        XCTAssertLessThan(percentile(cold.cacheLookupNanos, 0.99), 100_000)
        XCTAssertLessThan(percentile(cold.drawNanos, 0.95), 1_000_000)
        XCTAssertEqual(export.percentileDefinition, "nearest-rank: sorted[ceil(p*n)-1]")
        for phase in [cold, warm, imagesDisabled] {
            XCTAssertGreaterThan(phase.drawCount, 0, "phase must include actual viewer draw evidence")
            requireNonEmpty(phase.rawFrameDeltasNanos, "phase raw frame")
            XCTAssertFalse(phase.frameCallbackSamples.isEmpty, "phase scheduler diagnostics must not be empty")
            XCTAssertGreaterThan(phase.nominalFrameCount, 0)
            XCTAssertGreaterThanOrEqual(
                Double(phase.onTimeNominalFrameCount) / Double(phase.nominalFrameCount),
                0.99
            )
        }
        XCTAssertLessThanOrEqual(warm.viewerCausedDelayedIntervals.map(\.rawDeltaNanos).max() ?? 0, 33_300_000)
        XCTAssertEqual(imagesDisabled.imageRequestCount, 0)
        XCTAssertEqual(imagesDisabled.imageMetadataCount, 0)
        XCTAssertEqual(imagesDisabled.imageDecodeCount, 0)
        XCTAssertLessThanOrEqual(export.preResetSnapshot.unmountedHighWaterBytes, 32 * 1024 * 1024)
        XCTAssertEqual(export.postResetSnapshot.unmountedCurrentBytes, 0)
        XCTAssertEqual(export.postResetSnapshot.unmountedCurrentResidentCount, 0)
        XCTAssertEqual(export.postResetSnapshot.compiledCurrentBytes, 0)
        XCTAssertEqual(export.postResetSnapshot.compiledCurrentResidentCount, 0)
        let coldWindows = export.windowEvidence.filter { $0.phase == "cold" }
        let warmWindows = export.windowEvidence.filter { $0.phase == "warm" }
        XCTAssertEqual(coldWindows.count, 27)
        XCTAssertEqual(warmWindows.count, 27)
        for evidence in coldWindows + warmWindows {
            XCTAssertFalse(evidence.windowId.isEmpty)
            XCTAssertEqual(evidence.residentKeyCount, evidence.entryIds.count)
        }
        for evidence in warmWindows {
            XCTAssertEqual(evidence.compileCount, 0)
            XCTAssertEqual(evidence.layoutCount, 0)
            XCTAssertEqual(evidence.cacheMisses, 0)
        }
        XCTAssertEqual(export.duplicatePublications, 0)
    }
    private static func requireNonEmpty(_ values: [UInt64], _ name: String) { XCTAssertFalse(values.isEmpty, "\(name) evidence must be nonempty") }
    private static func percentile(_ values: [UInt64], _ percentile: Double) -> UInt64 { guard !values.isEmpty else { return .max }; return values.sorted()[max(0, Int((Double(values.count) * percentile).rounded(.up)) - 1)] }
}
