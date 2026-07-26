import UIKit

/// The bounded editor owner is the shared native transport/cache implementation.
/// This alias makes the cross-surface boundary explicit without forking policy.
typealias NativeImagePipeline = RenderImageLoadOwner

/// Immutable geometry produced by preparation. Pixels are deliberately kept
/// outside `PreparedProseLayout`, so image completion cannot mutate layout.
struct ViewerImageAttachment: Hashable {
    /// Compiler/admission ceiling. Publication storage is a compact bitset,
    /// but preparation still caps adversarial attachment artifacts.
    static let maximumAdmittedAttachments = 8_192
    /// Ordinal within the immutable prepared artifact. This is the compact
    /// publication-state address; `id` remains source-qualified cache identity.
    let ordinal: Int = -1
    let id: String
    let source: String
    let bounds: CGRect
    let declaredSize: CGSize?

    var hasDeclaredSize: Bool { (declaredSize?.width ?? 0) > 0 && (declaredSize?.height ?? 0) > 0 }

    static func sourceAndDeclaredSize(in block: ViewerBlock) -> (id: String, source: String, declaredSize: CGSize?)? {
        guard let atom = block.inlines.compactMap({ inline -> (UInt32, String, String)? in
            guard case let .atom(nodeType, docPos, attrsJSON, _) = inline,
                  nodeType == "image" else { return nil }
            return (docPos, attrsJSON, nodeType)
        }).first,
        let values = try? JSONSerialization.jsonObject(with: Data(atom.1.utf8)) as? [String: Any],
        let source = values["src"] as? String, !source.isEmpty else { return nil }
        func dimension(_ name: String) -> CGFloat? {
            guard let value = values[name] as? NSNumber else { return nil }
            let dimension = CGFloat(truncating: value)
            return dimension.isFinite && dimension > 0 ? dimension : nil
        }
        let width = dimension("width")
        let height = dimension("height")
        let declared = width.flatMap { w in height.map { CGSize(width: w, height: $0) } }
        return ("\(atom.0):\(source)", source, declared)
    }
}

/// The only mutable image-metadata state. It records the first valid intrinsic
/// size atomically and advances the attachment revision exactly once per id.
final class ViewerImageIntrinsicStore {
    static let shared = ViewerImageIntrinsicStore()

    private struct Entry {
        let size: CGSize
        var access: UInt64
    }

    private let lock = NSLock()
    private var entryLimit: Int
    private var access: UInt64 = 0
    private var values: [String: Entry] = [:]

    init(entryLimit: Int = 256) { self.entryLimit = max(1, entryLimit) }

    func size(for id: String) -> CGSize? {
        lock.lock()
        if var entry = values[id] {
            access &+= 1
            entry.access = access
            values[id] = entry
            lock.unlock()
            return entry.size
        }
        lock.unlock()
        return ViewerAttachmentRevisionState.authoritativeSize(for: id)
    }

    /// Test-only global-LRU inspection. `size(for:)` intentionally falls back
    /// to mounted sidecars, so it cannot prove an LRU eviction on its own.
    func globalSize(for id: String) -> CGSize? {
        lock.withLock { values[id]?.size }
    }

    /// Test-only control of the actual process-global LRU. `size(for:)` keeps
    /// its active-sidecar fallback, while this inspection proves eviction.
    func clearAndSetEntryLimitForTesting(_ limit: Int = 256) {
        lock.withLock {
            entryLimit = max(1, limit)
            access = 0
            values.removeAll(keepingCapacity: false)
        }
    }

    func store(_ size: CGSize, for id: String) {
        guard size.width.isFinite, size.height.isFinite, size.width > 0, size.height > 0 else { return }
        lock.lock()
        defer { lock.unlock() }
        access &+= 1
        values[id] = Entry(size: size, access: access)
        while values.count > entryLimit,
              let oldest = values.min(by: { lhs, rhs in
                  lhs.value.access == rhs.value.access ? lhs.key < rhs.key : lhs.value.access < rhs.value.access
              })
        {
            values.removeValue(forKey: oldest.key)
        }
    }
}

/// Per-surface publication state. It retains compact ordinal metadata plus a
/// bitset, so the global LRU is only an optimization. It resets only for a
/// semantic replacement or recycle/teardown, never request cancellation.
final class ViewerAttachmentRevisionState {
    /// Project accounting convention: fixed owner plus one header per retained
    /// proportional collection. Payload is charged below at native stride.
    static let fixedRetainedBytes = 160
    static let collectionRetainedBytes = 32
    static let activeRegistrationRetainedBytes = 48
    private final class WeakState {
        weak var value: ViewerAttachmentRevisionState?
        init(_ value: ViewerAttachmentRevisionState) { self.value = value }
    }
    private static let activeStateLock = NSLock()
    private static var activeStates: [WeakState] = []
    private let lock = NSLock()
    private var publishedBits: [UInt8] = []
    private var reportedErrorBits: [UInt8] = []
    private var intrinsicSizes: [CGSize] = []
    private var sourceQualifiedIDs: [String?] = []
    private var attachmentOrdinals: [Int] = []
    private var admittedAttachmentCount = 0
    private var semanticGenerationIdentity: String?
    private(set) var revision: UInt64 = 0

    /// Exact per-surface retained state. This is not an immutable layout cost:
    /// it belongs to the mounted host, including both bitsets, dimensions,
    /// source identity references, ordinal addresses, collection headers, and
    /// active-sidecar registration.
    var retainedPublicationBytesForTesting: Int {
        lock.withLock {
            Self.fixedRetainedBytes
                + Self.collectionRetainedBytes * 5
                + Self.activeRegistrationRetainedBytes
                + publishedBits.count
                + reportedErrorBits.count
                + intrinsicSizes.count * MemoryLayout<CGSize>.stride
                + sourceQualifiedIDs.count * MemoryLayout<String?>.stride
                + attachmentOrdinals.count * MemoryLayout<Int>.stride
                + sourceQualifiedIDs.compactMap { $0 }.reduce(0) { $0 + $1.utf8.count * 2 }
                + (semanticGenerationIdentity?.utf8.count ?? 0) * 2
        }
    }

    /// Returns true exactly when a true semantic replacement has cleared all
    /// generation-scoped correctness state. State-revision reinstalls return
    /// false and preserve metadata/error publication.
    @discardableResult
    func beginSemanticGeneration(_ identity: String) -> Bool {
        let changed = lock.withLock {
            guard semanticGenerationIdentity != identity else { return false }
            clearLocked()
            semanticGenerationIdentity = identity
            return true
        }
        if changed { Self.unregister(self) }
        return changed
    }

    func admit(attachmentCount: Int) {
        let count = max(0, attachmentCount)
        lock.withLock {
            guard admittedAttachmentCount != count else { return }
            admittedAttachmentCount = count
            publishedBits = Array(repeating: 0, count: (count + 7) / 8)
            reportedErrorBits = Array(repeating: 0, count: (count + 7) / 8)
            intrinsicSizes = Array(repeating: .zero, count: count)
            sourceQualifiedIDs = Array(repeating: nil, count: count)
            attachmentOrdinals = Array(0..<count)
        }
        if count > 0 { Self.register(self) }
    }

    func reset() {
        lock.withLock {
            clearLocked()
            semanticGenerationIdentity = nil
        }
        Self.unregister(self)
    }

    @discardableResult
    func recordIntrinsicSize(_ size: CGSize, for id: String, ordinal: Int, declaredSize: CGSize?) -> Bool {
        guard declaredSize == nil, size.width.isFinite, size.height.isFinite, size.width > 0, size.height > 0 else { return false }
        lock.lock(); defer { lock.unlock() }
        guard ordinal >= 0, ordinal < admittedAttachmentCount else { return false }
        let byteIndex = ordinal / 8
        let mask = UInt8(1 << (ordinal % 8))
        guard publishedBits[byteIndex] & mask == 0 else { return false }
        publishedBits[byteIndex] |= mask
        intrinsicSizes[ordinal] = size
        sourceQualifiedIDs[ordinal] = id
        ViewerImageIntrinsicStore.shared.store(size, for: id)
        revision &+= 1
        return true
    }

    func intrinsicSize(for ordinal: Int) -> CGSize? {
        lock.withLock {
            guard ordinal >= 0, ordinal < admittedAttachmentCount else { return nil }
            let mask = UInt8(1 << (ordinal % 8))
            return publishedBits[ordinal / 8] & mask == 0 ? nil : intrinsicSizes[ordinal]
        }
    }

    @discardableResult
    func recordResourceFailure(for ordinal: Int) -> Bool {
        lock.withLock {
            guard ordinal >= 0, ordinal < admittedAttachmentCount else { return false }
            let byteIndex = ordinal / 8
            let mask = UInt8(1 << (ordinal % 8))
            guard reportedErrorBits[byteIndex] & mask == 0 else { return false }
            reportedErrorBits[byteIndex] |= mask
            return true
        }
    }

    private func intrinsicSize(forSourceQualifiedID id: String) -> CGSize? {
        lock.withLock {
            guard let index = sourceQualifiedIDs.firstIndex(where: { $0 == id }) else { return nil }
            let ordinal = attachmentOrdinals[index]
            let mask = UInt8(1 << (ordinal % 8))
            return publishedBits[ordinal / 8] & mask == 0 ? nil : intrinsicSizes[ordinal]
        }
    }

    private func clearLocked() {
        publishedBits.removeAll(keepingCapacity: false)
        reportedErrorBits.removeAll(keepingCapacity: false)
        intrinsicSizes.removeAll(keepingCapacity: false)
        sourceQualifiedIDs.removeAll(keepingCapacity: false)
        attachmentOrdinals.removeAll(keepingCapacity: false)
        admittedAttachmentCount = 0
        revision = 0
    }

    static func authoritativeSize(for id: String) -> CGSize? {
        activeStateLock.withLock {
            activeStates.removeAll { $0.value == nil }
            return activeStates.compactMap { $0.value?.intrinsicSize(forSourceQualifiedID: id) }.first
        }
    }

    private static func register(_ state: ViewerAttachmentRevisionState) {
        activeStateLock.withLock {
            activeStates.removeAll { $0.value == nil }
            if !activeStates.contains(where: { $0.value === state }) { activeStates.append(WeakState(state)) }
        }
    }

    private static func unregister(_ state: ViewerAttachmentRevisionState) {
        activeStateLock.withLock { activeStates.removeAll { $0.value == nil || $0.value === state } }
    }

    deinit { Self.unregister(self) }
}

/// Bounded/cancellable viewer facade over the editor's existing native image
/// owner. The owner retains validation, fetch/decode limits, cache ownership,
/// receipts, deadlines and error isolation; this facade adds viewport and
/// generation ownership without changing editor behaviour.
final class ViewerImagePipeline {
    typealias PixelCompletion = (_ attachment: ViewerImageAttachment, _ image: UIImage) -> Void
    typealias MetadataCompletion = (_ attachment: ViewerImageAttachment, _ size: CGSize) -> Void

    static let prefetchMargin: CGFloat = 480

    private let owner: NativeImagePipeline
    private let lock = NSLock()
    private var generation = ""
    private var enabled = false
    private var receipts: [String: NativeImagePipeline.ImageLoadReceipt] = [:]
    private var requested = Set<String>()
    private(set) var requestCountForTesting = 0
    var onPixels: PixelCompletion?
    var onIntrinsicMetadata: MetadataCompletion?
    /// Deliberately carries no source URL; hosts map it to their public error contract.
    var onResourceFailure: ((ViewerImageAttachment) -> Void)?

    init(policy: ImageLoadingPolicy) {
        owner = NativeImagePipeline(policy: policy)
    }

    func begin(generation: String, imagesEnabled: Bool, policy: ImageLoadingPolicy? = nil) {
        lock.lock()
        let nextPolicy = policy ?? owner.policy
        if self.generation == generation, enabled == imagesEnabled, owner.policy == nextPolicy {
            lock.unlock()
            return
        }
        owner.cancelAll()
        if owner.policy != nextPolicy { owner.updatePolicy(nextPolicy) }
        self.generation = generation
        enabled = imagesEnabled
        receipts.removeAll()
        requested.removeAll()
        requestCountForTesting = 0
        lock.unlock()
    }

    func cancel() {
        lock.lock()
        generation = ""
        enabled = false
        receipts.values.forEach { $0.cancel() }
        receipts.removeAll()
        requested.removeAll()
        lock.unlock()
        owner.cancelAll()
    }

    func acceptsCompletion(generation: String) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return enabled && !self.generation.isEmpty && self.generation == generation
    }

    func updateVisibleRect(_ visibleRect: CGRect, attachments: [ViewerImageAttachment]) {
        guard visibleRect.origin.x.isFinite, visibleRect.origin.y.isFinite,
              visibleRect.size.width.isFinite, visibleRect.size.height.isFinite,
              !visibleRect.isNull, !visibleRect.isEmpty else { return }
        let expanded = visibleRect.insetBy(dx: -Self.prefetchMargin, dy: -Self.prefetchMargin)
        let eligible = attachments.filter { $0.ordinal >= 0 && !$0.source.isEmpty && $0.bounds.intersects(expanded) }
        let start: (String, [ViewerImageAttachment])? = lock.withLock {
            guard enabled, !generation.isEmpty else { return nil }
            let next = eligible.filter { requested.insert($0.id).inserted }
            requestCountForTesting += next.count
            return (generation, next)
        }
        guard let (currentGeneration, toStart) = start else { return }
        for attachment in toStart {
            let receipt = owner.startImageLoad(source: attachment.source) { [weak self] image in
                guard let self, self.acceptsCompletion(generation: currentGeneration) else { return }
                guard let image else {
                    self.reportFailure(attachment, generation: currentGeneration)
                    return
                }
                let size = image.size.applying(CGAffineTransform(scaleX: image.scale, y: image.scale))
                guard size.width.isFinite, size.height.isFinite, size.width > 0, size.height > 0 else {
                    self.reportFailure(attachment, generation: currentGeneration)
                    return
                }
                self.onIntrinsicMetadata?(attachment, size)
                guard self.acceptsCompletion(generation: currentGeneration) else { return }
                self.onPixels?(attachment, image)
            }
            if let receipt {
                let shouldRetain = lock.withLock { () -> Bool in
                    guard enabled, generation == currentGeneration else { return false }
                    receipts[attachment.id] = receipt
                    return true
                }
                if !shouldRetain { receipt.cancel() }
            } else {
                reportFailure(attachment, generation: currentGeneration)
            }
        }
    }

    private func reportFailure(_ attachment: ViewerImageAttachment, generation: String) {
        let callback: ((ViewerImageAttachment) -> Void)? = lock.withLock {
            guard enabled, self.generation == generation else { return nil }
            return onResourceFailure
        }
        callback?(attachment)
    }

    internal func reportFailureForTesting(_ attachment: ViewerImageAttachment) {
        lock.lock()
        let currentGeneration = generation
        lock.unlock()
        reportFailure(attachment, generation: currentGeneration)
    }
}

private extension NSLock {
    func withLock<T>(_ body: () -> T) -> T {
        lock()
        defer { unlock() }
        return body()
    }
}
