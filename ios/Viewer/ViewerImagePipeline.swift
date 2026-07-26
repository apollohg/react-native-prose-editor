import UIKit

/// The bounded editor owner is the shared native transport/cache implementation.
/// This alias makes the cross-surface boundary explicit without forking policy.
typealias NativeImagePipeline = RenderImageLoadOwner

/// Immutable geometry produced by preparation. Pixels are deliberately kept
/// outside `PreparedProseLayout`, so image completion cannot mutate layout.
struct ViewerImageAttachment: Hashable {
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
enum ViewerImageIntrinsicStore {
    private static let lock = NSLock()
    private static var values: [String: CGSize] = [:]

    static func size(for id: String) -> CGSize? { lock.lock(); defer { lock.unlock() }; return values[id] }
    static func store(_ size: CGSize, for id: String) { lock.lock(); values[id] = size; lock.unlock() }
}

final class ViewerAttachmentRevisionState {
    private let lock = NSLock()
    private var intrinsicSizes: [String: CGSize] = [:]
    private(set) var revision: UInt64 = 0

    func intrinsicSize(for id: String) -> CGSize? {
        lock.lock(); defer { lock.unlock() }
        return intrinsicSizes[id]
    }

    @discardableResult
    func recordIntrinsicSize(_ size: CGSize, for id: String, declaredSize: CGSize?) -> Bool {
        guard declaredSize == nil, size.width.isFinite, size.height.isFinite, size.width > 0, size.height > 0 else { return false }
        lock.lock(); defer { lock.unlock() }
        guard intrinsicSizes[id] == nil else { return false }
        intrinsicSizes[id] = size
        ViewerImageIntrinsicStore.store(size, for: id)
        revision &+= 1
        return true
    }
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
        let expanded = visibleRect.insetBy(dx: -Self.prefetchMargin, dy: -Self.prefetchMargin)
        let eligible = attachments.filter { !$0.source.isEmpty && $0.bounds.intersects(expanded) }
        lock.lock()
        guard enabled, !generation.isEmpty else { lock.unlock(); return }
        let currentGeneration = generation
        for attachment in eligible where !requested.contains(attachment.id) {
            requested.insert(attachment.id)
            requestCountForTesting += 1
            let receipt = owner.startImageLoad(source: attachment.source) { [weak self] image in
                guard let self, let image, self.acceptsCompletion(generation: currentGeneration) else { return }
                let size = image.size.applying(CGAffineTransform(scaleX: image.scale, y: image.scale))
                self.onIntrinsicMetadata?(attachment, size)
                guard self.acceptsCompletion(generation: currentGeneration) else { return }
                self.onPixels?(attachment, image)
            }
            if let receipt { receipts[attachment.id] = receipt }
        }
        lock.unlock()
    }
}
