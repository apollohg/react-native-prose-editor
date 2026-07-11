import UIKit
import ImageIO

struct ImageLoadingPolicy: Equatable {
    static let `default` = ImageLoadingPolicy(
        maxSourceBytes: 10 * 1024 * 1024,
        connectTimeout: 10,
        readTimeout: 20,
        maxConcurrentRequests: 2,
        maxPendingRequests: 64,
        maxDecodeDimension: 2_048
    )

    let maxSourceBytes: Int
    let connectTimeout: TimeInterval
    let readTimeout: TimeInterval
    let maxConcurrentRequests: Int
    let maxPendingRequests: Int
    let maxDecodeDimension: Int

    static func from(json: String?) -> ImageLoadingPolicy {
        guard let json,
              let data = json.data(using: .utf8),
              let values = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return .default
        }
        let defaults = ImageLoadingPolicy.default
        func positiveInteger(_ key: String, fallback: Int) -> Int {
            guard let number = values[key] as? NSNumber,
                  CFGetTypeID(number) != CFBooleanGetTypeID(),
                  number.doubleValue.isFinite,
                  number.doubleValue.rounded(.towardZero) == number.doubleValue,
                  number.int64Value > 0,
                  number.int64Value <= Int64(Int.max)
            else {
                return fallback
            }
            return number.intValue
        }
        return ImageLoadingPolicy(
            maxSourceBytes: positiveInteger("maxSourceBytes", fallback: defaults.maxSourceBytes),
            connectTimeout: TimeInterval(
                positiveInteger("connectTimeoutMs", fallback: Int(defaults.connectTimeout * 1_000))
            ) / 1_000,
            readTimeout: TimeInterval(
                positiveInteger("readTimeoutMs", fallback: Int(defaults.readTimeout * 1_000))
            ) / 1_000,
            maxConcurrentRequests: positiveInteger(
                "maxConcurrentRequests",
                fallback: defaults.maxConcurrentRequests
            ),
            maxPendingRequests: positiveInteger(
                "maxPendingRequests",
                fallback: defaults.maxPendingRequests
            ),
            maxDecodeDimension: positiveInteger(
                "maxDecodeDimensionPx",
                fallback: defaults.maxDecodeDimension
            )
        )
    }
}

extension Notification.Name {
    static let editorImageAttachmentDidLoad = Notification.Name(
        "com.apollohg.editor.imageAttachmentDidLoad"
    )
}

final class RenderImageCostCache {
    private struct Entry {
        let image: UIImage
        let cost: Int
        var access: UInt64
    }

    private let lock = NSLock()
    private let costLimit: Int
    private var entries: [String: Entry] = [:]
    private var access: UInt64 = 0
    private(set) var totalCost = 0

    init(costLimit: Int) {
        self.costLimit = max(0, costLimit)
    }

    func image(forKey key: String) -> UIImage? {
        lock.lock()
        defer { lock.unlock() }
        guard var entry = entries[key] else { return nil }
        access &+= 1
        entry.access = access
        entries[key] = entry
        return entry.image
    }

    func insert(_ image: UIImage, forKey key: String, cost: Int) {
        lock.lock()
        defer { lock.unlock() }
        let boundedCost = max(0, cost)
        if let previous = entries.removeValue(forKey: key) {
            totalCost -= previous.cost
        }
        guard boundedCost <= costLimit else { return }
        access &+= 1
        entries[key] = Entry(image: image, cost: boundedCost, access: access)
        totalCost += boundedCost
        while totalCost > costLimit,
              let oldest = entries.min(by: { $0.value.access < $1.value.access })
        {
            entries.removeValue(forKey: oldest.key)
            totalCost -= oldest.value.cost
        }
    }
}

enum RenderImageCache {
    static let cache = RenderImageCostCache(costLimit: 64 * 1024 * 1024)

    static func key(source: String, policy: ImageLoadingPolicy) -> String {
        [
            source,
            String(policy.maxSourceBytes),
            String(policy.connectTimeout.bitPattern),
            String(policy.readTimeout.bitPattern),
            String(policy.maxConcurrentRequests),
            String(policy.maxPendingRequests),
            String(policy.maxDecodeDimension),
        ].joined(separator: "|")
    }

    static func decodedCost(_ image: UIImage) -> Int {
        let width = image.cgImage?.width ?? Int(image.size.width * image.scale)
        let height = image.cgImage?.height ?? Int(image.size.height * image.scale)
        let (pixels, pixelOverflow) = width.multipliedReportingOverflow(by: height)
        guard !pixelOverflow else { return Int.max }
        let (bytes, byteOverflow) = pixels.multipliedReportingOverflow(by: 4)
        return byteOverflow ? Int.max : bytes
    }
}

protocol ImageLoadingTask: AnyObject {
    func cancel()
}

protocol ImageLoadingTransport: AnyObject {
    func load(
        _ url: URL,
        policy: ImageLoadingPolicy,
        completion: @escaping (Result<Data, Error>) -> Void
    ) -> ImageLoadingTask
}

protocol ImageDataDecoding: AnyObject {
    func decode(_ data: Data, maxDimension: Int) -> UIImage?
}

final class ImageRequestTimeoutController {
    typealias Schedule = (TimeInterval, @escaping () -> Void) -> ImageLoadingTask

    private let connectTimeout: TimeInterval
    private let readTimeout: TimeInterval
    private let schedule: Schedule
    private let onTimeout: () -> Void
    private let lock = NSLock()
    private var timer: ImageLoadingTask?
    private var finished = false

    init(
        connectTimeout: TimeInterval,
        readTimeout: TimeInterval,
        schedule: @escaping Schedule,
        onTimeout: @escaping () -> Void
    ) {
        self.connectTimeout = connectTimeout
        self.readTimeout = readTimeout
        self.schedule = schedule
        self.onTimeout = onTimeout
    }

    func start() {
        arm(after: connectTimeout)
    }

    func receivedResponse() {
        arm(after: readTimeout)
    }

    func receivedData() {
        arm(after: readTimeout)
    }

    func cancel() {
        lock.lock()
        finished = true
        let timer = self.timer
        self.timer = nil
        lock.unlock()
        timer?.cancel()
    }

    private func arm(after delay: TimeInterval) {
        lock.lock()
        guard !finished else {
            lock.unlock()
            return
        }
        timer?.cancel()
        timer = schedule(delay) { [weak self] in
            guard let self else { return }
            self.lock.lock()
            guard !self.finished else {
                self.lock.unlock()
                return
            }
            self.finished = true
            self.timer = nil
            self.lock.unlock()
            self.onTimeout()
        }
        lock.unlock()
    }
}

private final class DispatchImageTimeoutTask: ImageLoadingTask {
    private let workItem: DispatchWorkItem

    init(delay: TimeInterval, action: @escaping () -> Void) {
        let item = DispatchWorkItem(block: action)
        workItem = item
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + delay, execute: item)
    }

    func cancel() {
        workItem.cancel()
    }
}

private final class DefaultImageDataDecoder: ImageDataDecoding {
    func decode(_ data: Data, maxDimension: Int) -> UIImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxDimension,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
        else {
            return nil
        }
        return UIImage(cgImage: image)
    }
}

final class URLSessionImageTask: NSObject, ImageLoadingTask, URLSessionDataDelegate {
    private let policy: ImageLoadingPolicy
    private let completion: (Result<Data, Error>) -> Void
    private let lock = NSLock()
    private var buffer = Data()
    private var session: URLSession?
    private var task: URLSessionDataTask?
    private var timeoutController: ImageRequestTimeoutController?
    private var finished = false

    init(url: URL, policy: ImageLoadingPolicy, completion: @escaping (Result<Data, Error>) -> Void) {
        self.policy = policy
        self.completion = completion
        super.init()
        let configuration = URLSessionConfiguration.ephemeral
        let fallbackTimeout = max(policy.connectTimeout, policy.readTimeout) * 1_000
        configuration.timeoutIntervalForRequest = fallbackTimeout
        configuration.timeoutIntervalForResource = fallbackTimeout
        let session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
        self.session = session
        let request = URLRequest(url: url)
        let task = session.dataTask(with: request)
        self.task = task
        let timeoutController = ImageRequestTimeoutController(
            connectTimeout: policy.connectTimeout,
            readTimeout: policy.readTimeout,
            schedule: { delay, action in
                DispatchImageTimeoutTask(delay: delay, action: action)
            },
            onTimeout: { [weak self] in
                self?.finish(.failure(URLError(.timedOut)))
            }
        )
        self.timeoutController = timeoutController
        timeoutController.start()
        task.resume()
    }

    func cancel() {
        finish(.failure(URLError(.cancelled)), deliver: false)
    }

    func urlSession(
        _ session: URLSession,
        dataTask: URLSessionDataTask,
        didReceive response: URLResponse,
        completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
    ) {
        if response.expectedContentLength > Int64(policy.maxSourceBytes) {
            completionHandler(.cancel)
            finish(.failure(URLError(.dataLengthExceedsMaximum)))
            return
        }
        timeoutController?.receivedResponse()
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock()
        let exceedsLimit = Self.wouldExceedLimit(
            currentCount: buffer.count,
            incomingCount: data.count,
            maxBytes: policy.maxSourceBytes
        )
        if !exceedsLimit {
            buffer.append(data)
        }
        lock.unlock()
        guard !exceedsLimit else {
            finish(.failure(URLError(.dataLengthExceedsMaximum)))
            return
        }
        timeoutController?.receivedData()
    }

    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        didCompleteWithError error: Error?
    ) {
        if let error {
            finish(.failure(error))
        } else {
            lock.lock()
            let data = buffer
            lock.unlock()
            finish(.success(data))
        }
    }

    static func wouldExceedLimit(
        currentCount: Int,
        incomingCount: Int,
        maxBytes: Int
    ) -> Bool {
        guard currentCount >= 0, incomingCount >= 0, maxBytes >= 0 else { return true }
        guard incomingCount <= maxBytes else { return true }
        return currentCount > maxBytes - incomingCount
    }

    private func finish(_ result: Result<Data, Error>, deliver: Bool = true) {
        lock.lock()
        guard !finished else {
            lock.unlock()
            return
        }
        finished = true
        let timeoutController = self.timeoutController
        let task = self.task
        let session = self.session
        self.task = nil
        self.session = nil
        self.timeoutController = nil
        lock.unlock()
        timeoutController?.cancel()
        task?.cancel()
        session?.invalidateAndCancel()
        if deliver {
            completion(result)
        }
    }
}

private final class URLSessionImageTransport: ImageLoadingTransport {
    func load(
        _ url: URL,
        policy: ImageLoadingPolicy,
        completion: @escaping (Result<Data, Error>) -> Void
    ) -> ImageLoadingTask {
        URLSessionImageTask(url: url, policy: policy, completion: completion)
    }
}

final class RenderImageLoadOwner {
    typealias Delivery = (@escaping () -> Void) -> Void
    final class ImageLoadReceipt {
        private let lock = NSLock()
        private var cancellation: (() -> Void)?

        fileprivate init(cancellation: @escaping () -> Void) {
            self.cancellation = cancellation
        }

        func cancel() {
            lock.lock()
            let cancellation = self.cancellation
            self.cancellation = nil
            lock.unlock()
            cancellation?()
        }
    }

    private struct Request {
        let id: UUID
        let source: String
        let generation: UInt64
        let completion: (UIImage?) -> Void
    }

    private final class ActiveRequest {
        let request: Request
        var task: ImageLoadingTask?

        init(request: Request) {
            self.request = request
        }
    }

    private static let contextKey = "com.apollohg.editor.image-load-owner"
    private static let suppressKey = "com.apollohg.editor.suppress-image-loads"

    private let stateQueue = DispatchQueue(label: "com.apollohg.editor.image-owner-state")
    private let decodeQueue = DispatchQueue(
        label: "com.apollohg.editor.image-decode",
        qos: .userInitiated,
        attributes: .concurrent
    )
    private let transport: ImageLoadingTransport
    private let decoder: ImageDataDecoding
    private let deliver: Delivery
    private var storedPolicy: ImageLoadingPolicy
    private var generation: UInt64 = 0
    private var pending: [Request] = []
    private var active: [UUID: ActiveRequest] = [:]
    private var decodeWorkIds = Set<UUID>()
    private var deliveryQueuedIds = Set<UUID>()

    init(
        policy: ImageLoadingPolicy,
        transport: ImageLoadingTransport = URLSessionImageTransport(),
        decoder: ImageDataDecoding = DefaultImageDataDecoder(),
        deliver: @escaping Delivery = { action in DispatchQueue.main.async(execute: action) }
    ) {
        storedPolicy = policy
        self.transport = transport
        self.decoder = decoder
        self.deliver = deliver
    }

    var policy: ImageLoadingPolicy {
        stateQueue.sync { storedPolicy }
    }

    func updatePolicy(_ policy: ImageLoadingPolicy) {
        stateQueue.sync {
            cancelAllLocked()
            storedPolicy = policy
        }
    }

    @discardableResult
    func loadImage(source: String, completion: @escaping (UIImage?) -> Void) -> Bool {
        startImageLoad(source: source, completion: completion) != nil
    }

    @discardableResult
    func startImageLoad(
        source: String,
        completion: @escaping (UIImage?) -> Void
    ) -> ImageLoadReceipt? {
        let request: Request? = stateQueue.sync {
            let request = Request(
                id: UUID(),
                source: source,
                generation: generation,
                completion: completion
            )
            if occupiedWorkCountLocked >= storedPolicy.maxConcurrentRequests {
                guard pending.count < storedPolicy.maxPendingRequests else { return nil }
                pending.append(request)
                return request
            }
            startLocked(request)
            return request
        }
        guard let request else { return nil }
        return ImageLoadReceipt { [weak self] in
            self?.cancel(request)
        }
    }

    func cancelAll() {
        stateQueue.sync { cancelAllLocked() }
    }

    func withCurrent<T>(_ body: () throws -> T) rethrows -> T {
        let dictionary = Thread.current.threadDictionary
        let previous = dictionary[Self.contextKey]
        dictionary[Self.contextKey] = self
        defer {
            if let previous {
                dictionary[Self.contextKey] = previous
            } else {
                dictionary.removeObject(forKey: Self.contextKey)
            }
        }
        return try body()
    }

    static func withoutLoading<T>(_ body: () throws -> T) rethrows -> T {
        let dictionary = Thread.current.threadDictionary
        let previous = dictionary[suppressKey]
        dictionary[suppressKey] = true
        defer {
            if let previous {
                dictionary[suppressKey] = previous
            } else {
                dictionary.removeObject(forKey: suppressKey)
            }
        }
        return try body()
    }

    static var current: RenderImageLoadOwner? {
        guard Thread.current.threadDictionary[suppressKey] == nil else { return nil }
        return Thread.current.threadDictionary[contextKey] as? RenderImageLoadOwner
    }

    private func startLocked(_ request: Request) {
        let activeRequest = ActiveRequest(request: request)
        active[request.id] = activeRequest
        let policy = storedPolicy
        let cacheKey = RenderImageCache.key(source: request.source, policy: policy)
        if let cached = RenderImageCache.cache.image(forKey: cacheKey) {
            finishLocked(request, image: cached)
            return
        }

        if Self.isDataURL(request.source) {
            guard Self.decodedDataURLByteCount(
                request.source,
                maxBytes: policy.maxSourceBytes
            ) != nil else {
                finishLocked(request, image: nil)
                return
            }
            decodeWorkIds.insert(request.id)
            decodeQueue.async { [weak self] in
                guard let self else { return }
                let data = Self.decodeDataURL(request.source, maxBytes: policy.maxSourceBytes)
                let image = data.flatMap {
                    self.decoder.decode($0, maxDimension: policy.maxDecodeDimension)
                }
                self.finishDecode(request, image: image, cacheKey: cacheKey)
            }
            return
        }

        guard let url = URL(string: request.source),
              let scheme = url.scheme?.lowercased(),
              scheme == "https" || scheme == "http"
        else {
            finishLocked(request, image: nil)
            return
        }
        activeRequest.task = transport.load(url, policy: policy) { [weak self] result in
            guard let self else { return }
            self.stateQueue.async {
                guard request.generation == self.generation,
                      self.active[request.id] != nil
                else {
                    return
                }
                self.decodeWorkIds.insert(request.id)
                self.decodeQueue.async {
                    let image: UIImage?
                    switch result {
                    case let .success(data) where data.count <= policy.maxSourceBytes:
                        image = self.decoder.decode(data, maxDimension: policy.maxDecodeDimension)
                    default:
                        image = nil
                    }
                    self.finishDecode(request, image: image, cacheKey: cacheKey)
                }
            }
        }
    }

    private func finishDecode(_ request: Request, image: UIImage?, cacheKey: String) {
        stateQueue.async { [weak self] in
            guard let self else { return }
            decodeWorkIds.remove(request.id)
            if request.generation == generation,
               active[request.id] != nil,
               let image
            {
                RenderImageCache.cache.insert(
                    image,
                    forKey: cacheKey,
                    cost: RenderImageCache.decodedCost(image)
                )
            }
            if request.generation == generation, active[request.id] != nil {
                finishLocked(request, image: image)
            } else {
                drainLocked()
            }
        }
    }

    private func finishLocked(_ request: Request, image: UIImage?) {
        guard request.generation == generation,
              active[request.id] != nil,
              deliveryQueuedIds.insert(request.id).inserted
        else {
            return
        }
        deliver { [weak self] in
            guard let self else { return }
            let shouldDeliver = stateQueue.sync {
                guard request.generation == self.generation,
                      self.active.removeValue(forKey: request.id) != nil,
                      self.deliveryQueuedIds.remove(request.id) != nil
                else {
                    return false
                }
                self.drainLocked()
                return true
            }
            guard shouldDeliver else { return }
            request.completion(image)
        }
    }

    private func drainLocked() {
        while occupiedWorkCountLocked < storedPolicy.maxConcurrentRequests, !pending.isEmpty {
            startLocked(pending.removeFirst())
        }
    }

    private var occupiedWorkCountLocked: Int {
        Set(active.keys).union(decodeWorkIds).count
    }

    private func cancelAllLocked() {
        generation &+= 1
        let tasks = active.values.compactMap(\.task)
        active.removeAll()
        pending.removeAll()
        deliveryQueuedIds.removeAll()
        tasks.forEach { $0.cancel() }
    }

    private func cancel(_ request: Request) {
        stateQueue.sync {
            guard request.generation == generation else { return }
            if let index = pending.firstIndex(where: { $0.id == request.id }) {
                pending.remove(at: index)
                return
            }
            guard let activeRequest = active.removeValue(forKey: request.id) else { return }
            deliveryQueuedIds.remove(request.id)
            activeRequest.task?.cancel()
            drainLocked()
        }
    }

    private static func isDataURL(_ source: String) -> Bool {
        let trimmed = source.drop(while: { $0.isWhitespace })
        return trimmed.prefix("data:image/".count).lowercased() == "data:image/"
    }

    static func decodedDataURLByteCount(_ source: String, maxBytes: Int) -> Int? {
        guard maxBytes >= 0, let comma = source.firstIndex(of: ",") else { return nil }
        let completeGroups = maxBytes / 3
        let maxSymbols: Int
        let (groupSymbols, overflow) = completeGroups.multipliedReportingOverflow(by: 4)
        if overflow {
            maxSymbols = Int.max
        } else if maxBytes % 3 == 0 {
            maxSymbols = groupSymbols
        } else {
            let (symbols, additionOverflow) = groupSymbols.addingReportingOverflow(4)
            maxSymbols = additionOverflow ? Int.max : symbols
        }

        var symbolCount = 0
        var trailingPadding = 0
        for character in source[source.index(after: comma)...] {
            guard !character.isWhitespace else { continue }
            let (nextCount, countOverflow) = symbolCount.addingReportingOverflow(1)
            guard !countOverflow, nextCount <= maxSymbols else { return nil }
            symbolCount = nextCount
            if character == "=" {
                guard trailingPadding < 2 else { return nil }
                trailingPadding += 1
            } else {
                trailingPadding = 0
            }
        }
        guard trailingPadding <= 2 else { return nil }
        let groups = symbolCount / 4
        let remainder = symbolCount % 4
        let (groupBytes, byteOverflow) = groups.multipliedReportingOverflow(by: 3)
        guard !byteOverflow else { return nil }
        let remainderBytes = remainder == 0 ? 0 : max(0, remainder - 1)
        let (bytesBeforePadding, additionOverflow) = groupBytes.addingReportingOverflow(remainderBytes)
        guard !additionOverflow, trailingPadding <= bytesBeforePadding else { return nil }
        let bytes = bytesBeforePadding - trailingPadding
        return bytes <= maxBytes ? bytes : nil
    }

    private static func decodeDataURL(_ source: String, maxBytes: Int) -> Data? {
        let trimmed = source.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let comma = trimmed.firstIndex(of: ","),
              trimmed[..<comma].lowercased().contains(";base64")
        else {
            return nil
        }
        let payload = trimmed[trimmed.index(after: comma)...]
        guard let data = Data(base64Encoded: String(payload), options: [.ignoreUnknownCharacters]),
              data.count <= maxBytes
        else {
            return nil
        }
        return data
    }
}

// MARK: - Constants

/// Custom NSAttributedString attribute keys for editor metadata.
enum RenderBridgeAttributes {
    /// Marks a character as a void element placeholder (hardBreak, horizontalRule).
    /// The value is the node type string (e.g. "hardBreak", "horizontalRule").
    static let voidNodeType = NSAttributedString.Key("com.apollohg.editor.voidNodeType")

    /// Stores the Rust document position (UInt32) for void elements.
    static let docPos = NSAttributedString.Key("com.apollohg.editor.docPos")

    /// Marks a character as a block boundary (for block start/end tracking).
    static let blockBoundary = NSAttributedString.Key("com.apollohg.editor.blockBoundary")

    /// Stores the block node type (e.g. "paragraph", "listItem").
    static let blockNodeType = NSAttributedString.Key("com.apollohg.editor.blockNodeType")

    /// Stores the block depth (UInt8).
    static let blockDepth = NSAttributedString.Key("com.apollohg.editor.blockDepth")

    /// Stores list context info as a dictionary for list items.
    static let listContext = NSAttributedString.Key("com.apollohg.editor.listContext")

    /// Marks blocks that should render a visible list marker.
    static let listMarkerContext = NSAttributedString.Key("com.apollohg.editor.listMarkerContext")

    /// Stores the rendered list marker color for the paragraph marker.
    static let listMarkerColor = NSAttributedString.Key("com.apollohg.editor.listMarkerColor")

    /// Stores the rendered list marker scale for unordered bullets.
    static let listMarkerScale = NSAttributedString.Key("com.apollohg.editor.listMarkerScale")

    /// Stores the paragraph base font used to render the list marker.
    static let listMarkerBaseFont = NSAttributedString.Key("com.apollohg.editor.listMarkerBaseFont")

    /// Stores the reserved list marker gutter width.
    static let listMarkerWidth = NSAttributedString.Key("com.apollohg.editor.listMarkerWidth")

    /// Stores the rendered blockquote border color.
    static let blockquoteBorderColor = NSAttributedString.Key("com.apollohg.editor.blockquoteBorderColor")

    /// Stores the rendered blockquote border width.
    static let blockquoteBorderWidth = NSAttributedString.Key("com.apollohg.editor.blockquoteBorderWidth")

    /// Stores the rendered blockquote gap between border and text.
    static let blockquoteMarkerGap = NSAttributedString.Key("com.apollohg.editor.blockquoteMarkerGap")

    /// Marks code-block paragraphs for custom background drawing.
    static let codeBlockBackgroundColor = NSAttributedString.Key("com.apollohg.editor.codeBlockBackgroundColor")
    static let codeBlockBorderRadius = NSAttributedString.Key("com.apollohg.editor.codeBlockBorderRadius")
    static let codeBlockPaddingHorizontal = NSAttributedString.Key("com.apollohg.editor.codeBlockPaddingHorizontal")
    static let codeBlockPaddingVertical = NSAttributedString.Key("com.apollohg.editor.codeBlockPaddingVertical")

    /// Marks synthetic zero-width placeholders used only for UIKit layout.
    static let syntheticPlaceholder = NSAttributedString.Key("com.apollohg.editor.syntheticPlaceholder")

    /// Stores the link href for visually styled link text without enabling UITextView's default link interaction.
    static let linkHref = NSAttributedString.Key("com.apollohg.editor.linkHref")

    /// Stores the owning top-level document child index for partial native patching.
    static let topLevelChildIndex = NSAttributedString.Key("com.apollohg.editor.topLevelChildIndex")
}

/// Layout constants for paragraph styles.
enum LayoutConstants {
    /// Spacing between paragraphs (points).
    static let paragraphSpacing: CGFloat = 8.0

    /// Base indentation per depth level (points).
    static let indentPerDepth: CGFloat = 24.0

    /// Width reserved for the list bullet/number (points).
    static let listMarkerWidth: CGFloat = 36.0

    /// Gap between the list marker and the text that follows (points).
    static let listMarkerTextGap: CGFloat = 8.0

    /// Height of the horizontal rule separator line (points).
    static let horizontalRuleHeight: CGFloat = 1.0

    /// Vertical padding above and below the horizontal rule (points).
    static let horizontalRuleVerticalPadding: CGFloat = 8.0

    /// Total leading inset reserved for each blockquote depth.
    static let blockquoteIndent: CGFloat = 18.0

    /// Width of the rendered blockquote border bar.
    static let blockquoteBorderWidth: CGFloat = 3.0

    /// Gap between the blockquote border bar and the text that follows.
    static let blockquoteMarkerGap: CGFloat = 8.0

    /// Bullet character for unordered list items.
    static let unorderedListBullet = "\u{2022} "

    /// Scale factor applied only to unordered list marker glyphs.
    static let unorderedListMarkerFontScale: CGFloat = 2.0

    /// Object replacement character used for void block elements.
    static let objectReplacementCharacter = "\u{FFFC}"
}

// MARK: - RenderBridge

/// Converts RenderElement JSON (emitted by Rust editor-core via UniFFI) into
/// NSAttributedString for display in a UITextView.
///
/// The JSON format matches the output of `serialize_render_elements` in lib.rs:
/// ```json
/// [
///   {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
///   {"type": "textRun", "text": "Hello ", "marks": []},
///   {"type": "textRun", "text": "world", "marks": ["bold"]},
///   {"type": "blockEnd"},
///   {"type": "voidInline", "nodeType": "hardBreak", "docPos": 12},
///   {"type": "voidBlock", "nodeType": "horizontalRule", "docPos": 15}
/// ]
/// ```
final class RenderBridge {

    // MARK: - Public API

    /// Convert a JSON array of RenderElements into an NSAttributedString.
    ///
    /// - Parameters:
    ///   - json: A JSON string representing an array of render elements.
    ///   - baseFont: The default font for unstyled text.
    ///   - textColor: The default text color.
    /// - Returns: The rendered attributed string. Returns an empty attributed
    ///   string if the JSON is invalid.
    static func renderElements(
        fromJSON json: String,
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme? = nil
    ) -> NSAttributedString {
        guard let data = json.data(using: .utf8),
              let parsed = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else {
            return NSAttributedString()
        }

        return renderElements(
            fromArray: parsed,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )
    }

    /// Convert a parsed array of RenderElement dictionaries into an NSAttributedString.
    ///
    /// This is the main rendering entry point. It processes elements in order,
    /// maintaining a block context stack for proper paragraph styling.
    ///
    /// - Parameters:
    ///   - elements: Parsed JSON array where each element is a dictionary.
    ///   - baseFont: The default font for unstyled text.
    ///   - textColor: The default text color.
    /// - Returns: The rendered attributed string.
    static func renderElements(
        fromArray elements: [[String: Any]],
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme? = nil
    ) -> NSAttributedString {
        let result = NSMutableAttributedString()
        var blockStack: [BlockContext] = []
        var isFirstBlock = true
        var pendingTrailingParagraphSpacing: CGFloat? = nil

        for element in elements {
            guard let type = element["type"] as? String else { continue }
            let topLevelChildIndex = jsonInt(element["topLevelChildIndex"])

            switch type {
            case "textRun":
                let text = element["text"] as? String ?? ""
                let marks = element["marks"] as? [Any] ?? []
                let isCodeBlock = blockStack.last?.nodeType == "codeBlock"
                let blockFont = resolvedFont(
                    for: blockStack,
                    baseFont: baseFont,
                    theme: theme
                )
                let blockColor = resolvedTextColor(
                    for: blockStack,
                    textColor: textColor,
                    theme: theme
                )
                var baseAttrs = attributesForMarks(
                    marks,
                    baseFont: blockFont,
                    textColor: blockColor,
                    theme: theme
                )
                if isCodeBlock {
                    // blockFont already carries theme.codeBlock.text (merged in
                    // effectiveTextStyle); only substitute the system monospace
                    // font when the theme did not pick an explicit family, and
                    // always re-apply the mark-resolved bold/italic traits.
                    //
                    // A single combined withSymbolicTraits(union(bold, italic))
                    // call can return nil when the mono family has no
                    // bold-italic face, which would silently drop BOTH traits.
                    // Bold must never be silently dropped, so bold is applied
                    // first via a path that cannot fail, then italic is
                    // layered on top where the face supports it (mirrors the
                    // inline-code path below).
                    let resolvedFont = baseAttrs[.font] as? UIFont ?? blockFont
                    let markTraits = resolvedFont.fontDescriptor.symbolicTraits
                        .intersection([.traitBold, .traitItalic])
                    let wantsBold = markTraits.contains(.traitBold)
                    let wantsItalic = markTraits.contains(.traitItalic)
                    let themedFamily = theme?.codeBlock?.text?.fontFamily != nil
                    var codeFont: UIFont
                    if themedFamily {
                        codeFont = blockFont
                        if !markTraits.isEmpty {
                            if let descriptor = codeFont.fontDescriptor.withSymbolicTraits(
                                codeFont.fontDescriptor.symbolicTraits.union(markTraits)
                            ) {
                                // Themed family supports the full combined trait set.
                                codeFont = UIFont(descriptor: descriptor, size: codeFont.pointSize)
                            } else if wantsBold,
                                      let boldOnlyDescriptor = codeFont.fontDescriptor.withSymbolicTraits(
                                          codeFont.fontDescriptor.symbolicTraits.union([.traitBold])
                                      ) {
                                // No combined bold-italic face in this themed
                                // family; fall back to bold-only rather than
                                // dropping both traits.
                                codeFont = UIFont(descriptor: boldOnlyDescriptor, size: codeFont.pointSize)
                            }
                        }
                    } else if wantsBold {
                        // System monospace has no combined bold-italic face.
                        // Apply bold via the weight parameter (cannot fail),
                        // then try to layer italic on top so bold always
                        // survives even when italic cannot be layered.
                        codeFont = UIFont.monospacedSystemFont(ofSize: blockFont.pointSize, weight: .bold)
                        if wantsItalic,
                           let descriptor = codeFont.fontDescriptor.withSymbolicTraits(
                               codeFont.fontDescriptor.symbolicTraits.union([.traitItalic])
                           ) {
                            codeFont = UIFont(descriptor: descriptor, size: codeFont.pointSize)
                        }
                    } else {
                        codeFont = UIFont.monospacedSystemFont(ofSize: blockFont.pointSize, weight: .regular)
                        if wantsItalic,
                           let descriptor = codeFont.fontDescriptor.withSymbolicTraits(.traitItalic) {
                            codeFont = UIFont(descriptor: descriptor, size: codeFont.pointSize)
                        }
                    }
                    baseAttrs[.font] = codeFont
                }
                let attrs = applyBlockStyle(
                    to: baseAttrs,
                    blockStack: blockStack,
                    theme: theme,
                    blockBaseFont: blockFont
                )
                let attributedText = NSAttributedString(string: text, attributes: attrs)
                result.append(
                    attributedStringApplyingLeadingTopLevelChildIndexIfNeeded(
                        attributedText,
                        topLevelChildIndex: topLevelChildIndex,
                        resultIsEmpty: result.length == 0
                    )
                )

            case "voidInline":
                let nodeType = element["nodeType"] as? String ?? ""
                let docPos = jsonUInt32(element["docPos"])
                let attrs = element["attrs"] as? [String: Any] ?? [:]
                if nodeType == "hardBreak" {
                    overrideTrailingParagraphSpacing(in: result, paragraphSpacing: 0)
                }
                let attrStr = attributedStringForVoidInline(
                    nodeType: nodeType,
                    docPos: docPos,
                    attrs: attrs,
                    baseFont: baseFont,
                    textColor: textColor,
                    blockStack: blockStack,
                    topLevelChildIndex: topLevelChildIndex,
                    theme: theme
                )
                result.append(
                    attributedStringApplyingLeadingTopLevelChildIndexIfNeeded(
                        attrStr,
                        topLevelChildIndex: topLevelChildIndex,
                        resultIsEmpty: result.length == 0
                    )
                )

            case "voidBlock":
                let nodeType = element["nodeType"] as? String ?? ""
                let docPos = jsonUInt32(element["docPos"])
                let attrs = element["attrs"] as? [String: Any] ?? [:]

                // Add inter-block newline if not the first block.
                if !isFirstBlock {
                    collapseTrailingSpacingBeforeHorizontalRuleIfNeeded(
                        in: result,
                        pendingParagraphSpacing: &pendingTrailingParagraphSpacing,
                        nodeType: nodeType,
                        theme: theme
                    )
                    applyPendingTrailingParagraphSpacing(
                        in: result,
                        pendingParagraphSpacing: &pendingTrailingParagraphSpacing
                    )
                    result.append(
                        interBlockNewline(
                            baseFont: baseFont,
                            textColor: textColor,
                            blockStack: [],
                            theme: theme,
                            topLevelChildIndex: topLevelChildIndex
                        )
                    )
                }
                isFirstBlock = false

                let attrStr = attributedStringForVoidBlock(
                    nodeType: nodeType,
                    docPos: docPos,
                    elementAttrs: attrs,
                    baseFont: baseFont,
                    textColor: textColor,
                    topLevelChildIndex: topLevelChildIndex,
                    theme: theme
                )
                result.append(attrStr)

            case "opaqueInlineAtom":
                let nodeType = element["nodeType"] as? String ?? ""
                let label = element["label"] as? String ?? "?"
                let docPos = jsonUInt32(element["docPos"])
                let mentionTheme = (element["mentionTheme"] as? [String: Any]).map(
                    EditorMentionTheme.init(dictionary:)
                )
                let attrStr = attributedStringForOpaqueInlineAtom(
                    nodeType: nodeType,
                    label: label,
                    docPos: docPos,
                    baseFont: baseFont,
                    textColor: textColor,
                    blockStack: blockStack,
                    topLevelChildIndex: topLevelChildIndex,
                    theme: theme,
                    mentionTheme: mentionTheme
                )
                result.append(
                    attributedStringApplyingLeadingTopLevelChildIndexIfNeeded(
                        attrStr,
                        topLevelChildIndex: topLevelChildIndex,
                        resultIsEmpty: result.length == 0
                    )
                )

            case "opaqueBlockAtom":
                let nodeType = element["nodeType"] as? String ?? ""
                let label = element["label"] as? String ?? "?"
                let docPos = jsonUInt32(element["docPos"])

                if !isFirstBlock {
                    applyPendingTrailingParagraphSpacing(
                        in: result,
                        pendingParagraphSpacing: &pendingTrailingParagraphSpacing
                    )
                    result.append(
                        interBlockNewline(
                            baseFont: baseFont,
                            textColor: textColor,
                            blockStack: [],
                            theme: theme,
                            topLevelChildIndex: topLevelChildIndex
                        )
                    )
                }
                isFirstBlock = false

                let attrStr = attributedStringForOpaqueBlockAtom(
                    nodeType: nodeType,
                    label: label,
                    docPos: docPos,
                    baseFont: baseFont,
                    textColor: textColor,
                    topLevelChildIndex: topLevelChildIndex,
                    theme: theme
                )
                result.append(attrStr)

            case "blockStart":
                let nodeType = element["nodeType"] as? String ?? ""
                let depth = jsonUInt8(element["depth"])
                let listContext = element["listContext"] as? [String: Any]
                let isListItemContainer = isListItemNodeType(nodeType) && listContext != nil
                let isTransparentLayoutContainer = isTransparentContainer(nodeType)
                let ctx = BlockContext(
                    nodeType: nodeType,
                    depth: depth,
                    listContext: listContext,
                    topLevelChildIndex: topLevelChildIndex,
                    markerPending: isListItemContainer
                )
                let nestedListItemContainer =
                    isListItemContainer && (theme?.list?.itemSpacing != nil)
                    && blockStack.contains(where: {
                        isListItemNodeType($0.nodeType) && $0.listContext != nil
                    })

                if !isListItemContainer && !isTransparentLayoutContainer {
                    // Add inter-block newline before non-first rendered blocks.
                    if !isFirstBlock {
                        applyPendingTrailingParagraphSpacing(
                            in: result,
                            pendingParagraphSpacing: &pendingTrailingParagraphSpacing
                        )
                        let newlineBlockStack: [BlockContext]
                        if ctx.nodeType == "codeBlock" {
                            newlineBlockStack = []
                        } else if blockquoteDepth(in: blockStack + [ctx]) > 0,
                           !trailingRenderedContentHasBlockquote(in: result)
                        {
                            newlineBlockStack = []
                        } else {
                            newlineBlockStack = blockStack + [ctx]
                        }
                        let collapsedSeparatorSpacing = collapsedParagraphSpacingAfterHorizontalRule(
                            in: result,
                            separatorBlockStack: newlineBlockStack,
                            theme: theme,
                            baseFont: baseFont
                        )
                        result.append(
                            interBlockNewline(
                                baseFont: baseFont,
                                textColor: textColor,
                                blockStack: newlineBlockStack,
                                theme: theme,
                                paragraphSpacingOverride: collapsedSeparatorSpacing,
                                topLevelChildIndex: topLevelChildIndex
                            )
                        )
                    }
                    isFirstBlock = false
                } else if applyPendingTrailingParagraphSpacing(
                    in: result,
                    pendingParagraphSpacing: &pendingTrailingParagraphSpacing
                ) {
                    // Applied list item spacing queued when the previous item ended.
                } else if nestedListItemContainer {
                    overrideTrailingParagraphSpacing(
                        in: result,
                        paragraphSpacing: CGFloat(theme?.list?.itemSpacing ?? 0)
                    )
                }

                // Push block context for inline children to reference.
                blockStack.append(ctx)

                var markerListContext: [String: Any]? = nil
                if !isListItemContainer {
                    if let directListContext = listContext {
                        markerListContext = directListContext
                    } else {
                        markerListContext = consumePendingListMarker(from: &blockStack)
                    }
                }

                if markerListContext != nil {
                    if var currentBlock = blockStack.popLast() {
                        currentBlock.listMarkerContext = markerListContext
                        if currentBlock.listContext != nil {
                            currentBlock.listContext = markerListContext
                        }
                        blockStack.append(currentBlock)
                    }
                    // On iOS we draw list markers outside the editable text stream so
                    // UIKit still sees paragraph-start for native capitalization.
                }

            case "blockEnd":
                if let endedBlock = blockStack.popLast() {
                    appendTrailingHardBreakPlaceholderIfNeeded(
                        in: result,
                        endedBlock: endedBlock,
                        remainingBlockStack: blockStack,
                        baseFont: baseFont,
                        textColor: textColor,
                        theme: theme
                    )
                    if endedBlock.listContext != nil {
                        pendingTrailingParagraphSpacing = theme?.list?.itemSpacing
                    }
                }

            default:
                break
            }
        }

        return result
    }

    static func renderBlocks(
        fromArray blocks: [[[String: Any]]],
        startIndex: Int = 0,
        includeLeadingInterBlockSeparator: Bool = false,
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme? = nil
    ) -> NSAttributedString {
        var flattened: [[String: Any]] = []
        flattened.reserveCapacity(blocks.reduce(0) { $0 + $1.count })

        for (offset, block) in blocks.enumerated() {
            let topLevelChildIndex = startIndex + offset
            for element in block {
                var tagged = element
                tagged["topLevelChildIndex"] = topLevelChildIndex
                flattened.append(tagged)
            }
        }

        let renderedBlocks = renderElements(
            fromArray: flattened,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )
        guard includeLeadingInterBlockSeparator, startIndex > 0, !blocks.isEmpty else {
            return renderedBlocks
        }

        let separatorReadyBlocks = removingLeadingTopLevelChildIndex(
            from: renderedBlocks,
            topLevelChildIndex: startIndex
        )

        let result = NSMutableAttributedString(
            attributedString: interBlockNewline(
                baseFont: baseFont,
                textColor: textColor,
                blockStack: [],
                theme: theme,
                topLevelChildIndex: startIndex
            )
        )
        result.append(separatorReadyBlocks)
        return result
    }

    // MARK: - Height Pre-Measurement

    static func measureHeight(
        forRenderJSON renderJSON: String,
        themeJSON: String?,
        width: CGFloat
    ) -> CGFloat {
        if !Thread.isMainThread {
            return DispatchQueue.main.sync {
                measureHeight(
                    forRenderJSON: renderJSON,
                    themeJSON: themeJSON,
                    width: width
                )
            }
        }
        guard width > 0 else { return 0 }

        let theme = EditorTheme.from(json: themeJSON)
        let baseFontSize = theme?.text?.fontSize ?? theme?.paragraph?.fontSize ?? 16
        let baseFont = UIFont.systemFont(ofSize: baseFontSize)
        let textColor = theme?.text?.color ?? UIColor.label

        let attributedString = RenderImageLoadOwner.withoutLoading {
            renderElements(
                fromJSON: renderJSON,
                baseFont: baseFont,
                textColor: textColor,
                theme: theme
            )
        }

        guard attributedString.length > 0 else { return 0 }

        let contentInsets = theme?.contentInsets
        let topInset = contentInsets?.top ?? 0
        let bottomInset = contentInsets?.bottom ?? 0
        let leftInset = contentInsets?.left ?? 0
        let rightInset = contentInsets?.right ?? 0

        // When contentInsets are set, lineFragmentPadding is 0 (matches
        // RichTextEditorView.theme didSet). Otherwise use the UITextView
        // default of 5.
        let lineFragmentPadding: CGFloat = contentInsets != nil ? 0 : 5

        let textStorage = NSTextStorage(attributedString: attributedString)
        let layoutManager = NSLayoutManager()
        let containerWidth = width - leftInset - rightInset - lineFragmentPadding * 2
        let textContainer = NSTextContainer(
            size: CGSize(width: max(containerWidth, 0), height: .greatestFiniteMagnitude)
        )
        textContainer.lineFragmentPadding = 0

        layoutManager.addTextContainer(textContainer)
        textStorage.addLayoutManager(layoutManager)

        layoutManager.ensureLayout(for: textContainer)

        var usedRect = layoutManager.usedRect(for: textContainer)
        let extraLineFragmentRect = layoutManager.extraLineFragmentRect
        if !extraLineFragmentRect.isEmpty {
            usedRect = usedRect.union(extraLineFragmentRect)
        }

        let height = ceil(usedRect.height + topInset + bottomInset)
        return height
    }

    // MARK: - Mark Handling

    /// Build NSAttributedString attributes for a set of render marks.
    ///
    /// Supported marks:
    /// - `bold` -> adds `.traitBold` to the font descriptor
    /// - `italic` -> adds `.traitItalic` to the font descriptor
    /// - `underline` -> sets `.underlineStyle = .single`
    /// - `strike` / `strikethrough` -> sets `.strikethroughStyle = .single`
    /// - `code` -> uses a monospaced font variant
    ///
    /// Multiple marks are combined: "bold italic" produces a bold-italic font.
    static func attributesForMarks(
        _ marks: [Any],
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme? = nil
    ) -> [NSAttributedString.Key: Any] {
        var attrs = defaultAttributes(baseFont: baseFont, textColor: textColor)

        if marks.isEmpty {
            return attrs
        }

        var traits: UIFontDescriptor.SymbolicTraits = []
        var useMonospace = false
        var linkTheme: EditorLinkTheme?
        var shouldUnderline = false
        for mark in marks {
            let markObject = mark as? [String: Any]
            let markType: String
            if let markName = mark as? String {
                markType = markName
            } else if let resolvedType = markObject?["type"] as? String {
                markType = resolvedType
            } else {
                continue
            }

            switch markType {
            case "bold", "strong":
                traits.insert(.traitBold)
            case "italic", "em":
                traits.insert(.traitItalic)
            case "underline":
                shouldUnderline = true
            case "strike", "strikethrough":
                attrs[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
            case "code":
                useMonospace = true
            case "link":
                linkTheme = theme?.links
                if theme?.links?.underline ?? true {
                    shouldUnderline = true
                }
                attrs[.foregroundColor] = theme?.links?.color ?? UIColor.systemBlue
                if let backgroundColor = theme?.links?.backgroundColor {
                    attrs[.backgroundColor] = backgroundColor
                }
                if let href = markObject?["href"] as? String, !href.isEmpty {
                    attrs[RenderBridgeAttributes.linkHref] = href
                }
            default:
                break
            }
        }

        var resolvedFont = linkTheme?.resolvedFont(fallback: baseFont) ?? baseFont

        if useMonospace {
            resolvedFont = UIFont.monospacedSystemFont(
                ofSize: resolvedFont.pointSize,
                weight: traits.contains(.traitBold) ? .bold : .regular
            )
            // Monospaced doesn't support italic via descriptor traits, but we
            // still apply bold via the weight parameter above. If italic is also
            // requested alongside code, we skip it (monospaced italic is rare).
            if traits.contains(.traitItalic) && !traits.contains(.traitBold) {
                // For code+italic only, try applying italic trait.
                if let descriptor = resolvedFont.fontDescriptor.withSymbolicTraits(.traitItalic) {
                    resolvedFont = UIFont(descriptor: descriptor, size: resolvedFont.pointSize)
                }
            }
        } else if !traits.isEmpty {
            let mergedTraits = resolvedFont.fontDescriptor.symbolicTraits.union(traits)
            if let descriptor = resolvedFont.fontDescriptor.withSymbolicTraits(mergedTraits) {
                resolvedFont = UIFont(descriptor: descriptor, size: resolvedFont.pointSize)
            }
        }

        if shouldUnderline {
            attrs[.underlineStyle] = NSUnderlineStyle.single.rawValue
        }
        attrs[.font] = resolvedFont
        return attrs
    }

    // MARK: - Void Inline Elements

    /// Create an attributed string for a void inline element (e.g. hardBreak).
    ///
    /// A hardBreak is rendered as a newline character with custom attributes
    /// so the position bridge knows it represents a single doc position.
    private static func attributedStringForVoidInline(
        nodeType: String,
        docPos: UInt32,
        attrs _: [String: Any],
        baseFont: UIFont,
        textColor: UIColor,
        blockStack: [BlockContext],
        topLevelChildIndex _: Int?,
        theme: EditorTheme?
    ) -> NSAttributedString {
        let blockFont = resolvedFont(for: blockStack, baseFont: baseFont, theme: theme)
        let blockColor = resolvedTextColor(for: blockStack, textColor: textColor, theme: theme)
        var attrs = defaultAttributes(baseFont: blockFont, textColor: blockColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos
        let styledAttrs = applyBlockStyle(
            to: attrs,
            blockStack: blockStack,
            theme: theme,
            blockBaseFont: blockFont
        )

        switch nodeType {
        case "hardBreak":
            var hardBreakAttrs = styledAttrs
            if let paragraphStyle = (hardBreakAttrs[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy()
                as? NSMutableParagraphStyle
            {
                paragraphStyle.paragraphSpacing = 0
                hardBreakAttrs[.paragraphStyle] = paragraphStyle
            }
            return NSAttributedString(string: "\n", attributes: hardBreakAttrs)
        default:
            // Unknown void inline: render as object replacement character.
            return NSAttributedString(
                string: LayoutConstants.objectReplacementCharacter,
                attributes: styledAttrs
            )
        }
    }

    // MARK: - Void Block Elements

    /// Create an attributed string for a void block element (e.g. horizontalRule).
    ///
    /// Horizontal rules are rendered as U+FFFC (object replacement character)
    /// with an NSTextAttachment that draws a separator line.
    private static func attributedStringForVoidBlock(
        nodeType: String,
        docPos: UInt32,
        elementAttrs: [String: Any],
        baseFont: UIFont,
        textColor: UIColor,
        topLevelChildIndex: Int?,
        theme: EditorTheme?
    ) -> NSAttributedString {
        var attrs = defaultAttributes(baseFont: baseFont, textColor: textColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos
        if let topLevelChildIndex {
            attrs[RenderBridgeAttributes.topLevelChildIndex] = NSNumber(value: topLevelChildIndex)
        }

        switch nodeType {
        case "horizontalRule":
            let attachment = HorizontalRuleAttachment()
            attachment.lineColor = theme?.horizontalRule?.color ?? textColor.withAlphaComponent(0.3)
            attachment.lineHeight = theme?.horizontalRule?.thickness ?? LayoutConstants.horizontalRuleHeight
            attachment.verticalPadding = resolvedHorizontalRuleVerticalMargin(theme: theme)
            let attrStr = NSMutableAttributedString(
                attachment: attachment
            )
            // Apply our custom attributes to the attachment character.
            let range = NSRange(location: 0, length: attrStr.length)
            attrStr.addAttributes(attrs, range: range)
            return attrStr
        case "image":
            guard let source = (elementAttrs["src"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines),
                  !source.isEmpty
            else {
                return NSAttributedString(
                    string: LayoutConstants.objectReplacementCharacter,
                    attributes: attrs
                )
            }
            let attachment = BlockImageAttachment(
                source: source,
                placeholderTint: textColor,
                preferredWidth: jsonCGFloat(elementAttrs["width"]),
                preferredHeight: jsonCGFloat(elementAttrs["height"])
            )
            let attrStr = NSMutableAttributedString(attachment: attachment)
            let range = NSRange(location: 0, length: attrStr.length)
            attrStr.addAttributes(attrs, range: range)
            return attrStr
        default:
            // Unknown void block: render as object replacement character.
            return NSAttributedString(
                string: LayoutConstants.objectReplacementCharacter,
                attributes: attrs
            )
        }
    }

    // MARK: - Opaque Atoms

    /// Create an attributed string for an opaque inline atom (unknown inline void).
    private static func attributedStringForOpaqueInlineAtom(
        nodeType: String,
        label: String,
        docPos: UInt32,
        baseFont: UIFont,
        textColor: UIColor,
        blockStack: [BlockContext],
        topLevelChildIndex _: Int?,
        theme: EditorTheme?,
        mentionTheme: EditorMentionTheme?
    ) -> NSAttributedString {
        let blockFont = resolvedFont(for: blockStack, baseFont: baseFont, theme: theme)
        let blockColor = resolvedTextColor(for: blockStack, textColor: textColor, theme: theme)
        var attrs = defaultAttributes(baseFont: blockFont, textColor: blockColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos
        if nodeType == "mention" {
            let resolvedMentionTheme = theme?.mentions?.merged(with: mentionTheme) ?? mentionTheme
            attrs[.foregroundColor] = resolvedMentionTheme?.textColor ?? blockColor
            attrs[.backgroundColor] =
                resolvedMentionTheme?.backgroundColor ?? UIColor.systemBlue.withAlphaComponent(0.12)
            if let mentionFont = mentionFont(from: blockFont, theme: resolvedMentionTheme) {
                attrs[.font] = mentionFont
            }
        } else {
            attrs[.backgroundColor] = UIColor.systemGray5
        }
        let styledAttrs = applyBlockStyle(
            to: attrs,
            blockStack: blockStack,
            theme: theme,
            blockBaseFont: blockFont
        )

        let visibleText = nodeType == "mention" ? label : "[\(label)]"
        return NSAttributedString(string: visibleText, attributes: styledAttrs)
    }

    /// Create an attributed string for an opaque block atom (unknown block void).
    private static func attributedStringForOpaqueBlockAtom(
        nodeType: String,
        label: String,
        docPos: UInt32,
        baseFont: UIFont,
        textColor: UIColor,
        topLevelChildIndex: Int?,
        theme: EditorTheme?
    ) -> NSAttributedString {
        var attrs = defaultAttributes(baseFont: baseFont, textColor: textColor)
        attrs[RenderBridgeAttributes.voidNodeType] = nodeType
        attrs[RenderBridgeAttributes.docPos] = docPos
        attrs[.backgroundColor] = UIColor.systemGray5
        if let topLevelChildIndex {
            attrs[RenderBridgeAttributes.topLevelChildIndex] = NSNumber(value: topLevelChildIndex)
        }

        return NSAttributedString(string: "[\(label)]", attributes: attrs)
    }

    private static func mentionFont(from baseFont: UIFont, theme: EditorMentionTheme?) -> UIFont? {
        guard let fontWeight = theme?.fontWeight else { return nil }
        let descriptorTraits = EditorTheme.shouldApplyBoldTrait(fontWeight)
            ? UIFontDescriptor.SymbolicTraits.traitBold
            : []
        if descriptorTraits.isEmpty {
            return UIFont.systemFont(
                ofSize: baseFont.pointSize,
                weight: EditorTheme.fontWeight(from: fontWeight)
            )
        }
        guard let descriptor = baseFont.fontDescriptor.withSymbolicTraits(descriptorTraits) else {
            return UIFont.systemFont(
                ofSize: baseFont.pointSize,
                weight: EditorTheme.fontWeight(from: fontWeight)
            )
        }
        return UIFont(descriptor: descriptor, size: baseFont.pointSize)
    }

    // MARK: - Block Styling

    /// Create a paragraph style for a block context.
    ///
    /// Applies indentation based on depth and list context. List items get
    /// a hanging indent so the bullet/number sits in the margin.
    static func paragraphStyleForBlock(
        _ context: BlockContext,
        blockStack: [BlockContext],
        theme: EditorTheme? = nil,
        baseFont: UIFont = .systemFont(ofSize: 16)
    ) -> NSMutableParagraphStyle {
        let style = NSMutableParagraphStyle()
        let blockStyle = theme?.effectiveTextStyle(
            for: context.nodeType,
            inBlockquote: blockquoteDepth(in: blockStack) > 0
        )
        let spacing = blockStyle?.spacingAfter
            ?? (context.listContext != nil ? theme?.list?.itemSpacing : nil)
            ?? LayoutConstants.paragraphSpacing
        style.paragraphSpacing = spacing

        let indentPerDepth = theme?.list?.indent ?? LayoutConstants.indentPerDepth
        let markerWidth = listMarkerWidth(for: context, theme: theme, baseFont: baseFont)
        let quoteDepth = CGFloat(blockquoteDepth(in: blockStack))
        let quoteIndent = max(
            theme?.blockquote?.indent ?? LayoutConstants.blockquoteIndent,
            (theme?.blockquote?.markerGap ?? LayoutConstants.blockquoteMarkerGap)
                + (theme?.blockquote?.borderWidth ?? LayoutConstants.blockquoteBorderWidth)
        )
        let listBaseIndentMultiplier = max(theme?.list?.baseIndentMultiplier ?? 1, 0)
        let listBaseIndentAdjustment = context.listContext != nil
            ? ((listBaseIndentMultiplier - 1) * indentPerDepth)
            : 0
        let columnsDepth = CGFloat(columnContainerDepth(in: blockStack))
        let baseIndent = (CGFloat(context.depth) * indentPerDepth)
            - (quoteDepth * indentPerDepth)
            - (columnsDepth * indentPerDepth)
            + listBaseIndentAdjustment
            + (quoteDepth * quoteIndent)

        if context.listContext != nil {
            // List item: reserve a fixed gutter and align all wrapped lines to
            // the text start since the marker is drawn separately.
            style.firstLineHeadIndent = baseIndent + markerWidth
            style.headIndent = baseIndent + markerWidth
        } else {
            style.firstLineHeadIndent = baseIndent
            style.headIndent = baseIndent
        }

        if context.nodeType == "codeBlock" {
            let horizontalPadding = theme?.codeBlock?.paddingHorizontal ?? 12
            style.firstLineHeadIndent += horizontalPadding
            style.headIndent += horizontalPadding
            style.tailIndent = -horizontalPadding
        }

        if let lineHeight = blockStyle?.lineHeight {
            style.minimumLineHeight = lineHeight
            style.maximumLineHeight = lineHeight
        }

        return style
    }

    // MARK: - List Markers

    /// Generate the list marker string (bullet or number) from a list context.
    static func listMarkerString(listContext: [String: Any]) -> String {
        if (listContext["kind"] as? String) == "task" {
            let checked = (listContext["checked"] as? NSNumber)?.boolValue ?? false
            return checked ? "\u{2611} " : "\u{2610} "
        }
        let ordered = (listContext["ordered"] as? NSNumber)?.boolValue ?? false

        if ordered {
            let index = (listContext["index"] as? NSNumber)?.intValue ?? 1
            return "\(index). "
        } else {
            return LayoutConstants.unorderedListBullet
        }
    }

    // MARK: - Private Helpers

    /// Extract a `UInt32` from a JSON value produced by `JSONSerialization`.
    static func jsonUInt32(_ value: Any?) -> UInt32 {
        if let number = value as? NSNumber {
            return number.uint32Value
        }
        return 0
    }

    /// Extract a `UInt8` from a JSON value produced by `JSONSerialization`.
    static func jsonUInt8(_ value: Any?) -> UInt8 {
        if let number = value as? NSNumber {
            return number.uint8Value
        }
        return 0
    }

    static func jsonInt(_ value: Any?) -> Int? {
        if let number = value as? NSNumber {
            return number.intValue
        }
        if let string = value as? String,
           let resolved = Int(string.trimmingCharacters(in: .whitespacesAndNewlines))
        {
            return resolved
        }
        return nil
    }

    /// Extract a positive `CGFloat` from a JSON value produced by `JSONSerialization`.
    static func jsonCGFloat(_ value: Any?) -> CGFloat? {
        if let number = value as? NSNumber {
            let resolved = CGFloat(truncating: number)
            return resolved > 0 ? resolved : nil
        }
        if let string = value as? String,
           let resolved = Double(string.trimmingCharacters(in: .whitespacesAndNewlines)),
           resolved > 0
        {
            return CGFloat(resolved)
        }
        return nil
    }

    private static func defaultAttributes(
        baseFont: UIFont,
        textColor: UIColor
    ) -> [NSAttributedString.Key: Any] {
        [
            .font: baseFont,
            .foregroundColor: textColor,
        ]
    }

    @discardableResult
    private static func applyBlockStyle(
        to attrs: [NSAttributedString.Key: Any],
        blockStack: [BlockContext],
        theme: EditorTheme?,
        blockBaseFont: UIFont? = nil
    ) -> [NSAttributedString.Key: Any] {
        guard let currentBlock = effectiveBlockContext(blockStack) else { return attrs }
        var mutableAttrs = attrs
        let renderedFont = mutableAttrs[.font] as? UIFont ?? .systemFont(ofSize: 16)
        let paragraphBaseFont = blockBaseFont ?? renderedFont
        mutableAttrs[.paragraphStyle] = paragraphStyleForBlock(
            currentBlock,
            blockStack: blockStack,
            theme: theme,
            baseFont: paragraphBaseFont
        )
        mutableAttrs[RenderBridgeAttributes.blockNodeType] = currentBlock.nodeType
        mutableAttrs[RenderBridgeAttributes.blockDepth] = currentBlock.depth
        if let listContext = currentBlock.listContext {
            mutableAttrs[RenderBridgeAttributes.listContext] = listContext
        }
        if let markerContext = currentBlock.listMarkerContext {
            mutableAttrs[RenderBridgeAttributes.listMarkerContext] = markerContext
            mutableAttrs[RenderBridgeAttributes.listMarkerColor] = theme?.list?.markerColor
            mutableAttrs[RenderBridgeAttributes.listMarkerScale] = theme?.list?.markerScale
            mutableAttrs[RenderBridgeAttributes.listMarkerBaseFont] = paragraphBaseFont
            mutableAttrs[RenderBridgeAttributes.listMarkerWidth] = listMarkerWidth(
                for: currentBlock,
                theme: theme,
                baseFont: paragraphBaseFont
            )
        }
        if currentBlock.nodeType == "codeBlock" {
            mutableAttrs[RenderBridgeAttributes.codeBlockBackgroundColor] =
                theme?.codeBlock?.backgroundColor ?? UIColor.secondarySystemBackground
            mutableAttrs[RenderBridgeAttributes.codeBlockBorderRadius] =
                theme?.codeBlock?.borderRadius ?? 8
            mutableAttrs[RenderBridgeAttributes.codeBlockPaddingHorizontal] =
                theme?.codeBlock?.paddingHorizontal ?? 12
            mutableAttrs[RenderBridgeAttributes.codeBlockPaddingVertical] =
                theme?.codeBlock?.paddingVertical ?? 8
        }
        if blockquoteDepth(in: blockStack) > 0 {
            let foreground = mutableAttrs[.foregroundColor] as? UIColor ?? .separator
            mutableAttrs[RenderBridgeAttributes.blockquoteBorderColor] =
                theme?.blockquote?.borderColor
                ?? foreground.withAlphaComponent(0.3)
            mutableAttrs[RenderBridgeAttributes.blockquoteBorderWidth] =
                theme?.blockquote?.borderWidth ?? LayoutConstants.blockquoteBorderWidth
            mutableAttrs[RenderBridgeAttributes.blockquoteMarkerGap] =
                theme?.blockquote?.markerGap ?? LayoutConstants.blockquoteMarkerGap
        }
        return mutableAttrs
    }

    /// Create a newline attributed string used between blocks.
    ///
    /// This newline separates consecutive blocks in the flat rendered text.
    /// It carries minimal styling (base font, no special attributes).
    private static func interBlockNewline(
        baseFont: UIFont,
        textColor: UIColor,
        blockStack: [BlockContext],
        theme: EditorTheme?,
        paragraphSpacingOverride: CGFloat? = nil,
        topLevelChildIndex: Int? = nil
    ) -> NSAttributedString {
        var attrs = applyBlockStyle(
            to: defaultAttributes(baseFont: baseFont, textColor: textColor),
            blockStack: blockStack,
            theme: theme,
            blockBaseFont: baseFont
        )
        if let topLevelChildIndex {
            attrs[RenderBridgeAttributes.topLevelChildIndex] = NSNumber(value: topLevelChildIndex)
        }
        if let paragraphSpacingOverride,
           let paragraphStyle = (attrs[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy()
               as? NSMutableParagraphStyle
        {
            paragraphStyle.paragraphSpacing = paragraphSpacingOverride
            attrs[.paragraphStyle] = paragraphStyle
        }
        return NSAttributedString(string: "\n", attributes: attrs)
    }

    private static func attributedStringApplyingLeadingTopLevelChildIndexIfNeeded(
        _ attributedString: NSAttributedString,
        topLevelChildIndex: Int?,
        resultIsEmpty: Bool
    ) -> NSAttributedString {
        guard resultIsEmpty,
              let topLevelChildIndex,
              attributedString.length > 0
        else {
            return attributedString
        }

        let tagged = NSMutableAttributedString(attributedString: attributedString)
        let firstComposedCharacterRange = (tagged.string as NSString)
            .rangeOfComposedCharacterSequence(at: 0)
        tagged.addAttribute(
            RenderBridgeAttributes.topLevelChildIndex,
            value: NSNumber(value: topLevelChildIndex),
            range: firstComposedCharacterRange
        )
        return tagged
    }

    private static func removingLeadingTopLevelChildIndex(
        from attributedString: NSAttributedString,
        topLevelChildIndex: Int
    ) -> NSAttributedString {
        guard attributedString.length > 0 else { return attributedString }

        let firstValue = attributedString.attribute(
            RenderBridgeAttributes.topLevelChildIndex,
            at: 0,
            effectiveRange: nil
        ) as? NSNumber
        guard firstValue?.intValue == topLevelChildIndex else {
            return attributedString
        }

        let adjusted = NSMutableAttributedString(attributedString: attributedString)
        var effectiveRange = NSRange(location: 0, length: 0)
        adjusted.attribute(
            RenderBridgeAttributes.topLevelChildIndex,
            at: 0,
            longestEffectiveRange: &effectiveRange,
            in: NSRange(location: 0, length: adjusted.length)
        )
        adjusted.removeAttribute(
            RenderBridgeAttributes.topLevelChildIndex,
            range: effectiveRange
        )
        return adjusted
    }

    private static func effectiveBlockContext(_ blockStack: [BlockContext]) -> BlockContext? {
        guard let currentBlock = blockStack.last else { return nil }
        if currentBlock.listContext != nil {
            return currentBlock
        }
        guard let inheritedListBlock = nearestListBlock(in: Array(blockStack.dropLast())) else {
            return currentBlock
        }
        return BlockContext(
            nodeType: currentBlock.nodeType,
            depth: currentBlock.depth,
            listContext: inheritedListBlock.listContext,
            listMarkerContext: currentBlock.listMarkerContext,
            markerPending: false
        )
    }

    private static func nearestListBlock(in contexts: [BlockContext]) -> BlockContext? {
        for context in contexts.reversed() where context.listContext != nil {
            return context
        }
        return nil
    }

    private static func trailingRenderedContentHasBlockquote(
        in result: NSAttributedString
    ) -> Bool {
        guard result.length > 0 else { return false }
        let nsString = result.string as NSString

        for index in stride(from: result.length - 1, through: 0, by: -1) {
            let scalar = nsString.character(at: index)
            if scalar == 0x000A || scalar == 0x000D {
                continue
            }
            return result.attribute(
                RenderBridgeAttributes.blockquoteBorderColor,
                at: index,
                effectiveRange: nil
            ) != nil
        }

        return false
    }

    private static func consumePendingListMarker(from blockStack: inout [BlockContext]) -> [String: Any]? {
        guard blockStack.count >= 2 else { return nil }
        for idx in stride(from: blockStack.count - 2, through: 0, by: -1) {
            guard blockStack[idx].markerPending else { continue }
            blockStack[idx].markerPending = false
            return blockStack[idx].listContext
        }
        return nil
    }

    private static func isListItemNodeType(_ nodeType: String) -> Bool {
        nodeType == "listItem" || nodeType == "taskItem"
    }

    private static func overrideTrailingParagraphSpacing(
        in result: NSMutableAttributedString,
        paragraphSpacing: CGFloat
    ) {
        guard result.length > 0 else { return }

        let nsString = result.string as NSString
        let paragraphRange = nsString.paragraphRange(for: NSRange(location: result.length - 1, length: 0))
        result.enumerateAttribute(
            .paragraphStyle,
            in: paragraphRange,
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, _ in
            let sourceStyle = (value as? NSParagraphStyle)?.mutableCopy() as? NSMutableParagraphStyle
                ?? NSMutableParagraphStyle()
            sourceStyle.paragraphSpacing = paragraphSpacing
            result.addAttribute(.paragraphStyle, value: sourceStyle, range: range)
        }
    }

    private static func collapseTrailingSpacingBeforeHorizontalRuleIfNeeded(
        in result: NSMutableAttributedString,
        pendingParagraphSpacing: inout CGFloat?,
        nodeType: String,
        theme: EditorTheme?
    ) {
        guard nodeType == "horizontalRule" else { return }
        let horizontalRuleMargin = resolvedHorizontalRuleVerticalMargin(theme: theme)

        if let pendingSpacing = pendingParagraphSpacing {
            pendingParagraphSpacing = collapsedSpacing(
                existingSpacing: pendingSpacing,
                adjacentHorizontalRuleMargin: horizontalRuleMargin
            )
            return
        }

        guard let trailingParagraphSpacing = trailingParagraphSpacing(in: result) else { return }
        let adjustedSpacing = collapsedSpacing(
            existingSpacing: trailingParagraphSpacing,
            adjacentHorizontalRuleMargin: horizontalRuleMargin
        )
        guard abs(adjustedSpacing - trailingParagraphSpacing) > 0.01 else { return }
        overrideTrailingParagraphSpacing(in: result, paragraphSpacing: adjustedSpacing)
    }

    private static func collapsedParagraphSpacingAfterHorizontalRule(
        in result: NSAttributedString,
        separatorBlockStack: [BlockContext],
        theme: EditorTheme?,
        baseFont: UIFont
    ) -> CGFloat? {
        guard let horizontalRuleMargin = trailingHorizontalRuleMargin(in: result),
              let separatorSpacing = separatorParagraphSpacing(
                  for: separatorBlockStack,
                  theme: theme,
                  baseFont: baseFont
              )
        else {
            return nil
        }

        return collapsedSpacing(
            existingSpacing: separatorSpacing,
            adjacentHorizontalRuleMargin: horizontalRuleMargin
        )
    }

    @discardableResult
    private static func applyPendingTrailingParagraphSpacing(
        in result: NSMutableAttributedString,
        pendingParagraphSpacing: inout CGFloat?
    ) -> Bool {
        guard let paragraphSpacing = pendingParagraphSpacing else { return false }
        overrideTrailingParagraphSpacing(in: result, paragraphSpacing: paragraphSpacing)
        pendingParagraphSpacing = nil
        return true
    }

    private static func trailingParagraphSpacing(in result: NSAttributedString) -> CGFloat? {
        guard result.length > 0 else { return nil }

        let nsString = result.string as NSString
        let paragraphRange = nsString.paragraphRange(for: NSRange(location: result.length - 1, length: 0))
        var spacing: CGFloat? = nil
        result.enumerateAttribute(
            .paragraphStyle,
            in: paragraphRange,
            options: [.reverse, .longestEffectiveRangeNotRequired]
        ) { value, _, stop in
            if let paragraphStyle = value as? NSParagraphStyle {
                spacing = paragraphStyle.paragraphSpacing
                stop.pointee = true
            }
        }
        return spacing
    }

    private static func separatorParagraphSpacing(
        for blockStack: [BlockContext],
        theme: EditorTheme?,
        baseFont: UIFont
    ) -> CGFloat? {
        guard let currentBlock = effectiveBlockContext(blockStack) else { return nil }
        return paragraphStyleForBlock(
            currentBlock,
            blockStack: blockStack,
            theme: theme,
            baseFont: baseFont
        ).paragraphSpacing
    }

    private static func trailingHorizontalRuleMargin(in result: NSAttributedString) -> CGFloat? {
        guard result.length > 0 else { return nil }
        let nsString = result.string as NSString

        for index in stride(from: result.length - 1, through: 0, by: -1) {
            let scalar = nsString.character(at: index)
            if scalar == 0x000A || scalar == 0x000D {
                continue
            }
            guard result.attribute(
                RenderBridgeAttributes.voidNodeType,
                at: index,
                effectiveRange: nil
            ) as? String == "horizontalRule" else {
                return nil
            }
            return (
                result.attribute(.attachment, at: index, effectiveRange: nil)
                    as? HorizontalRuleAttachment
            )?.verticalPadding
        }

        return nil
    }

    private static func resolvedHorizontalRuleVerticalMargin(theme: EditorTheme?) -> CGFloat {
        theme?.horizontalRule?.verticalMargin ?? LayoutConstants.horizontalRuleVerticalPadding
    }

    private static func collapsedSpacing(
        existingSpacing: CGFloat,
        adjacentHorizontalRuleMargin: CGFloat
    ) -> CGFloat {
        max(existingSpacing, adjacentHorizontalRuleMargin) - adjacentHorizontalRuleMargin
    }

    private static func appendTrailingHardBreakPlaceholderIfNeeded(
        in result: NSMutableAttributedString,
        endedBlock: BlockContext,
        remainingBlockStack: [BlockContext],
        baseFont: UIFont,
        textColor: UIColor,
        theme: EditorTheme?
    ) {
        guard result.length > 0 else { return }
        guard !isListItemNodeType(endedBlock.nodeType) else { return }
        guard result.attribute(
            RenderBridgeAttributes.voidNodeType,
            at: result.length - 1,
            effectiveRange: nil
        ) as? String == "hardBreak" else {
            return
        }

        let placeholderBlockStack = remainingBlockStack + [endedBlock]
        let blockFont = resolvedFont(
            for: placeholderBlockStack,
            baseFont: baseFont,
            theme: theme
        )
        let blockColor = resolvedTextColor(
            for: placeholderBlockStack,
            textColor: textColor,
            theme: theme
        )
        var attrs = defaultAttributes(baseFont: blockFont, textColor: blockColor)
        attrs[RenderBridgeAttributes.syntheticPlaceholder] = true
        var styledAttrs = applyBlockStyle(
            to: attrs,
            blockStack: placeholderBlockStack,
            theme: theme,
            blockBaseFont: blockFont
        )
        if let paragraphStyle = (styledAttrs[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy()
            as? NSMutableParagraphStyle
        {
            paragraphStyle.paragraphSpacing = 0
            styledAttrs[.paragraphStyle] = paragraphStyle
        }
        result.append(NSAttributedString(string: "\u{200B}", attributes: styledAttrs))
    }

    private static func listMarkerWidth(
        for context: BlockContext,
        theme: EditorTheme?,
        baseFont: UIFont
    ) -> CGFloat {
        guard let listContext = context.listContext else { return 0 }
        _ = listContext
        _ = theme
        _ = baseFont
        return LayoutConstants.listMarkerWidth
    }

    private static func resolvedTextStyle(
        for blockStack: [BlockContext],
        theme: EditorTheme?
    ) -> EditorTextStyle? {
        let inBlockquote = blockquoteDepth(in: blockStack) > 0
        guard let currentBlock = effectiveBlockContext(blockStack) else {
            return theme?.effectiveTextStyle(for: "paragraph", inBlockquote: inBlockquote)
        }
        return theme?.effectiveTextStyle(for: currentBlock.nodeType, inBlockquote: inBlockquote)
    }

    private static func blockquoteDepth(in blockStack: [BlockContext]) -> Int {
        blockStack.reduce(into: 0) { count, context in
            if context.nodeType == "blockquote" {
                count += 1
            }
        }
    }

    private static func columnContainerDepth(in blockStack: [BlockContext]) -> Int {
        blockStack.reduce(into: 0) { count, context in
            if context.nodeType == "columns" || context.nodeType == "column" {
                count += 1
            }
        }
    }

    private static func isTransparentContainer(_ nodeType: String) -> Bool {
        nodeType == "blockquote" || nodeType == "columns" || nodeType == "column"
    }

    private static func resolvedFont(
        for blockStack: [BlockContext],
        baseFont: UIFont,
        theme: EditorTheme?
    ) -> UIFont {
        resolvedTextStyle(for: blockStack, theme: theme)?.resolvedFont(fallback: baseFont)
            ?? baseFont
    }

    private static func resolvedTextColor(
        for blockStack: [BlockContext],
        textColor: UIColor,
        theme: EditorTheme?
    ) -> UIColor {
        resolvedTextStyle(for: blockStack, theme: theme)?.color ?? textColor
    }
}

// MARK: - BlockContext

/// Transient context while rendering block elements. Pushed onto a stack
/// when a `blockStart` element is encountered and popped on `blockEnd`.
struct BlockContext {
    let nodeType: String
    let depth: UInt8
    var listContext: [String: Any]?
    var topLevelChildIndex: Int? = nil
    var listMarkerContext: [String: Any]? = nil
    var markerPending: Bool = false
}

// MARK: - HorizontalRuleAttachment

/// NSTextAttachment subclass that draws a horizontal separator line.
///
/// The attachment renders as a thin line across the available width with
/// vertical padding. Used for `horizontalRule` void block elements.
final class HorizontalRuleAttachment: NSTextAttachment {

    var lineColor: UIColor = .separator
    var lineHeight: CGFloat = LayoutConstants.horizontalRuleHeight
    var verticalPadding: CGFloat = LayoutConstants.horizontalRuleVerticalPadding

    override func attachmentBounds(
        for textContainer: NSTextContainer?,
        proposedLineFragment lineFrag: CGRect,
        glyphPosition position: CGPoint,
        characterIndex charIndex: Int
    ) -> CGRect {
        let totalHeight = lineHeight + (verticalPadding * 2)
        return CGRect(
            x: 0,
            y: 0,
            width: lineFrag.width,
            height: totalHeight
        )
    }

    override func image(
        forBounds imageBounds: CGRect,
        textContainer: NSTextContainer?,
        characterIndex charIndex: Int
    ) -> UIImage? {
        let renderer = UIGraphicsImageRenderer(bounds: imageBounds)
        return renderer.image { context in
            lineColor.setFill()
            let lineY = imageBounds.midY - (lineHeight / 2)
            let lineRect = CGRect(
                x: imageBounds.origin.x,
                y: lineY,
                width: imageBounds.width,
                height: lineHeight
            )
            context.fill(lineRect)
        }
    }
}

final class BlockImageAttachment: NSTextAttachment {
    private let source: String
    private let placeholderTint: UIColor
    private weak var loadOwner: RenderImageLoadOwner?
    private var loadReceipt: RenderImageLoadOwner.ImageLoadReceipt?
    private var preferredWidth: CGFloat?
    private var preferredHeight: CGFloat?
    private var loadedImage: UIImage?

    init(
        source: String,
        placeholderTint: UIColor,
        preferredWidth: CGFloat?,
        preferredHeight: CGFloat?
    ) {
        self.source = source
        self.placeholderTint = placeholderTint
        self.preferredWidth = preferredWidth
        self.preferredHeight = preferredHeight
        self.loadOwner = RenderImageLoadOwner.current
        super.init(data: nil, ofType: nil)
        loadImageIfNeeded()
    }

    required init?(coder: NSCoder) {
        return nil
    }

    deinit {
        loadReceipt?.cancel()
    }

    func setPreferredSize(width: CGFloat, height: CGFloat) {
        preferredWidth = width
        preferredHeight = height
    }

    func previewImage() -> UIImage? {
        loadedImage ?? image
    }

    override func attachmentBounds(
        for textContainer: NSTextContainer?,
        proposedLineFragment lineFrag: CGRect,
        glyphPosition position: CGPoint,
        characterIndex charIndex: Int
    ) -> CGRect {
        let lineFragmentWidth = lineFrag.width.isFinite ? lineFrag.width : 0
        let containerWidth = textContainer.map {
            max(0, $0.size.width - ($0.lineFragmentPadding * 2))
        } ?? 0
        let widthCandidates = [lineFragmentWidth, containerWidth].filter { $0.isFinite && $0 > 0 }
        let maxWidth = max(160, widthCandidates.min() ?? 160)
        let fallbackAspectRatio = loadedImage.flatMap { image -> CGFloat? in
            let imageSize = image.size
            guard imageSize.width > 0, imageSize.height > 0 else { return nil }
            return imageSize.height / imageSize.width
        } ?? 0.56

        var resolvedWidth = preferredWidth
        var resolvedHeight = preferredHeight

        if resolvedWidth == nil, resolvedHeight == nil, let loadedImage {
            let imageSize = loadedImage.size
            if imageSize.width > 0, imageSize.height > 0 {
                resolvedWidth = imageSize.width
                resolvedHeight = imageSize.height
            }
        } else if resolvedWidth == nil, let resolvedHeight {
            resolvedWidth = resolvedHeight / fallbackAspectRatio
        } else if resolvedHeight == nil, let resolvedWidth {
            resolvedHeight = resolvedWidth * fallbackAspectRatio
        }

        let width = max(1, resolvedWidth ?? maxWidth)
        let height = max(1, resolvedHeight ?? min(180, maxWidth * fallbackAspectRatio))
        let scale = min(1, maxWidth / width)
        return CGRect(x: 0, y: 0, width: width * scale, height: height * scale)
    }

    override func image(
        forBounds imageBounds: CGRect,
        textContainer: NSTextContainer?,
        characterIndex charIndex: Int
    ) -> UIImage? {
        if let loadedImage {
            return loadedImage
        }

        let renderer = UIGraphicsImageRenderer(bounds: imageBounds)
        return renderer.image { _ in
            let path = UIBezierPath(roundedRect: imageBounds, cornerRadius: 12)
            UIColor.secondarySystemFill.setFill()
            path.fill()

            let iconSize = min(imageBounds.width, imageBounds.height) * 0.28
            let iconOrigin = CGPoint(
                x: imageBounds.midX - (iconSize / 2),
                y: imageBounds.midY - (iconSize / 2)
            )
            let iconRect = CGRect(origin: iconOrigin, size: CGSize(width: iconSize, height: iconSize))

            if #available(iOS 13.0, *) {
                let config = UIImage.SymbolConfiguration(pointSize: iconSize, weight: .medium)
                let icon = UIImage(systemName: "photo", withConfiguration: config)?
                    .withTintColor(placeholderTint.withAlphaComponent(0.7), renderingMode: .alwaysOriginal)
                icon?.draw(in: iconRect)
            }
        }
    }

    private func loadImageIfNeeded() {
        guard let loadOwner else { return }
        let cacheKey = RenderImageCache.key(source: source, policy: loadOwner.policy)
        if let cached = RenderImageCache.cache.image(forKey: cacheKey) {
            loadedImage = cached
            image = cached
            return
        }
        loadReceipt = loadOwner.startImageLoad(source: source) { [weak self] image in
            guard let self,
                  let image
            else {
                return
            }
            self.loadedImage = image
            self.image = image
            self.loadReceipt = nil
            NotificationCenter.default.post(name: .editorImageAttachmentDidLoad, object: self)
        }
    }
}
