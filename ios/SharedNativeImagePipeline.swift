import UIKit
import ImageIO
import CryptoKit

struct ImageLoadingPolicy: Equatable {
    static let `default` = ImageLoadingPolicy(
        maxSourceBytes: 10 * 1024 * 1024,
        connectTimeout: 10,
        readTimeout: 20,
        requestTimeout: 60,
        maxConcurrentRequests: 2,
        maxPendingRequests: 64,
        maxDecodeDimension: 2_048
    )

    let maxSourceBytes: Int
    let connectTimeout: TimeInterval
    let readTimeout: TimeInterval
    let requestTimeout: TimeInterval
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
        func positiveInteger(_ key: String, fallback: Int, ceiling: Int) -> Int {
            guard let number = values[key] as? NSNumber,
                  CFGetTypeID(number) != CFBooleanGetTypeID(),
                  number.doubleValue.isFinite,
                  number.doubleValue.rounded(.towardZero) == number.doubleValue,
                  number.int64Value > 0,
                  number.int64Value <= Int64(ceiling)
            else {
                return fallback
            }
            return number.intValue
        }
        return ImageLoadingPolicy(
            maxSourceBytes: positiveInteger(
                "maxSourceBytes",
                fallback: defaults.maxSourceBytes,
                ceiling: 64 * 1024 * 1024
            ),
            connectTimeout: TimeInterval(
                positiveInteger(
                    "connectTimeoutMs",
                    fallback: Int(defaults.connectTimeout * 1_000),
                    ceiling: 600_000
                )
            ) / 1_000,
            readTimeout: TimeInterval(
                positiveInteger(
                    "readTimeoutMs",
                    fallback: Int(defaults.readTimeout * 1_000),
                    ceiling: 600_000
                )
            ) / 1_000,
            requestTimeout: TimeInterval(
                positiveInteger(
                    "requestTimeoutMs",
                    fallback: Int(defaults.requestTimeout * 1_000),
                    ceiling: 600_000
                )
            ) / 1_000,
            maxConcurrentRequests: positiveInteger(
                "maxConcurrentRequests",
                fallback: defaults.maxConcurrentRequests,
                ceiling: 16
            ),
            maxPendingRequests: positiveInteger(
                "maxPendingRequests",
                fallback: defaults.maxPendingRequests,
                ceiling: 512
            ),
            maxDecodeDimension: positiveInteger(
                "maxDecodeDimensionPx",
                fallback: defaults.maxDecodeDimension,
                ceiling: 8_192
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
    static let cacheEntryOverhead = 128

    static func key(source: String, policy: ImageLoadingPolicy) -> String {
        let canonicalPolicy = [
            String(policy.maxSourceBytes),
            String(policy.connectTimeout.bitPattern),
            String(policy.readTimeout.bitPattern),
            String(policy.requestTimeout.bitPattern),
            String(policy.maxConcurrentRequests),
            String(policy.maxPendingRequests),
            String(policy.maxDecodeDimension),
        ].joined(separator: "|")
        var input = Data(source.utf8)
        input.append(0)
        input.append(contentsOf: canonicalPolicy.utf8)
        return SHA256.hash(data: input).map { String(format: "%02x", $0) }.joined()
    }

    static func decodedCost(_ image: UIImage) -> Int {
        if let cgImage = image.cgImage {
            return backingCost(
                bytesPerRow: cgImage.bytesPerRow,
                height: cgImage.height,
                fallback: Int.max
            )
        }
        let width = Int(image.size.width * image.scale)
        let height = Int(image.size.height * image.scale)
        let (pixels, pixelOverflow) = width.multipliedReportingOverflow(by: height)
        guard !pixelOverflow else { return Int.max }
        let (bytes, byteOverflow) = pixels.multipliedReportingOverflow(by: 4)
        return byteOverflow ? Int.max : bytes
    }

    static func backingCost(bytesPerRow: Int, height: Int, fallback: Int) -> Int {
        guard bytesPerRow >= 0, height >= 0 else { return fallback }
        let (cost, overflow) = bytesPerRow.multipliedReportingOverflow(by: height)
        return overflow ? Int.max : cost
    }

    static func retainedCost(_ image: UIImage) -> Int {
        let metadataCost = 64 + cacheEntryOverhead
        let (cost, overflow) = decodedCost(image).addingReportingOverflow(metadataCost)
        return overflow ? Int.max : cost
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
    private let requestTimeout: TimeInterval
    private let schedule: Schedule
    private let onTimeout: () -> Void
    private let lock = NSLock()
    private var phaseTimer: ImageLoadingTask?
    private var totalTimer: ImageLoadingTask?
    private var phaseGeneration: UInt64 = 0
    private var totalGeneration: UInt64 = 0
    private var finished = false

    init(
        connectTimeout: TimeInterval,
        readTimeout: TimeInterval,
        requestTimeout: TimeInterval,
        schedule: @escaping Schedule,
        onTimeout: @escaping () -> Void
    ) {
        self.connectTimeout = connectTimeout
        self.readTimeout = readTimeout
        self.requestTimeout = requestTimeout
        self.schedule = schedule
        self.onTimeout = onTimeout
    }

    func start() {
        lock.lock()
        guard !finished, phaseTimer == nil, totalTimer == nil else {
            lock.unlock()
            return
        }
        phaseGeneration &+= 1
        totalGeneration &+= 1
        phaseTimer = makeTimer(after: connectTimeout, phase: true, generation: phaseGeneration)
        totalTimer = makeTimer(after: requestTimeout, phase: false, generation: totalGeneration)
        lock.unlock()
    }

    func receivedResponse() {
        armPhase(after: readTimeout)
    }

    func receivedData() {
        armPhase(after: readTimeout)
    }

    func cancel() {
        lock.lock()
        finished = true
        let timers = [phaseTimer, totalTimer]
        phaseTimer = nil
        totalTimer = nil
        lock.unlock()
        timers.forEach { $0?.cancel() }
    }

    private func armPhase(after delay: TimeInterval) {
        lock.lock()
        guard !finished else {
            lock.unlock()
            return
        }
        phaseTimer?.cancel()
        phaseGeneration &+= 1
        phaseTimer = makeTimer(after: delay, phase: true, generation: phaseGeneration)
        lock.unlock()
    }

    private func makeTimer(
        after delay: TimeInterval,
        phase: Bool,
        generation: UInt64
    ) -> ImageLoadingTask {
        schedule(delay) { [weak self] in
            guard let self else { return }
            self.lock.lock()
            let isCurrent = phase
                ? generation == self.phaseGeneration && self.phaseTimer != nil
                : generation == self.totalGeneration && self.totalTimer != nil
            guard !self.finished, isCurrent else {
                self.lock.unlock()
                return
            }
            self.finished = true
            let timers = [self.phaseTimer, self.totalTimer]
            self.phaseTimer = nil
            self.totalTimer = nil
            self.lock.unlock()
            timers.forEach { $0?.cancel() }
            self.onTimeout()
        }
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
        let configuration = Self.configuration(policy: policy)
        let session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
        self.session = session
        let request = URLRequest(url: url)
        let task = session.dataTask(with: request)
        self.task = task
        startTimeoutController { delay, action in
            DispatchImageTimeoutTask(delay: delay, action: action)
        }
        task.resume()
    }

    init(
        policy: ImageLoadingPolicy,
        timeoutSchedule: @escaping ImageRequestTimeoutController.Schedule,
        completion: @escaping (Result<Data, Error>) -> Void
    ) {
        self.policy = policy
        self.completion = completion
        super.init()
        startTimeoutController(schedule: timeoutSchedule)
    }

    static func configuration(policy: ImageLoadingPolicy) -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = max(
            policy.connectTimeout,
            policy.readTimeout
        )
        configuration.timeoutIntervalForResource = policy.requestTimeout
        return configuration
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
        lock.lock()
        guard !finished else {
            lock.unlock()
            completionHandler(.cancel)
            return
        }
        let timeoutController = self.timeoutController
        lock.unlock()
        timeoutController?.receivedResponse()
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock()
        guard !finished else {
            lock.unlock()
            return
        }
        let exceedsLimit = Self.wouldExceedLimit(
            currentCount: buffer.count,
            incomingCount: data.count,
            maxBytes: policy.maxSourceBytes
        )
        if !exceedsLimit {
            buffer.append(data)
        }
        let timeoutController = exceedsLimit ? nil : self.timeoutController
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

    private func startTimeoutController(
        schedule: @escaping ImageRequestTimeoutController.Schedule
    ) {
        let timeoutController = ImageRequestTimeoutController(
            connectTimeout: policy.connectTimeout,
            readTimeout: policy.readTimeout,
            requestTimeout: policy.requestTimeout,
            schedule: schedule,
            onTimeout: { [weak self] in
                self?.finish(.failure(URLError(.timedOut)))
            }
        )
        lock.lock()
        guard !finished else {
            lock.unlock()
            return
        }
        self.timeoutController = timeoutController
        lock.unlock()
        timeoutController.start()
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
    typealias Now = () -> TimeInterval
    typealias TimeoutSchedule = (TimeInterval, @escaping () -> Void) -> ImageLoadingTask
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
        let deadline: TimeInterval
        let completion: (UIImage?) -> Void
        let onAcceptedStart: () -> Void
    }

    private final class ActiveRequest {
        let request: Request
        var task: ImageLoadingTask?

        init(request: Request) {
            self.request = request
        }
    }

    private final class DeliveryTicket {
        // All transitions happen on stateQueue. Once a ticket is running, the
        // callback owns delivery; cancellation suppresses pending tickets but
        // never waits on arbitrary user code.
        private enum State { case pending, running, cancelled, finished }
        private var state: State = .pending

        func begin() -> Bool {
            guard state == .pending else { return false }
            state = .running
            return true
        }

        func cancel() {
            if state == .pending { state = .cancelled }
        }

        func finish() {
            if state == .running { state = .finished }
        }
    }

    struct DataURLHeaderScan {
        let commaIndex: String.Index?
        let scannedByteCount: Int
        let exceededLimit: Bool
    }

    private static let contextKey = "com.apollohg.editor.image-load-owner"
    private static let suppressKey = "com.apollohg.editor.suppress-image-loads"
    private static let dataURLHeaderByteLimit = 256
    private static let dataURLEncodedOverheadByteLimit = 4_096

    private let stateQueue = DispatchQueue(label: "com.apollohg.editor.image-owner-state")
    private let decodeQueue = DispatchQueue(
        label: "com.apollohg.editor.image-decode",
        qos: .userInitiated,
        attributes: .concurrent
    )
    private let transport: ImageLoadingTransport
    private let decoder: ImageDataDecoding
    private let deliver: Delivery
    private let now: Now
    private let scheduleTimeout: TimeoutSchedule
    private var storedPolicy: ImageLoadingPolicy
    private var generation: UInt64 = 0
    private var pending: [Request] = []
    private var active: [UUID: ActiveRequest] = [:]
    private var decodeWorkIds = Set<UUID>()
    private var deliveryTickets: [UUID: DeliveryTicket] = [:]
    private var deadlineTasks: [UUID: ImageLoadingTask] = [:]

    init(
        policy: ImageLoadingPolicy,
        transport: ImageLoadingTransport = URLSessionImageTransport(),
        decoder: ImageDataDecoding = DefaultImageDataDecoder(),
        deliver: @escaping Delivery = { action in DispatchQueue.main.async(execute: action) },
        now: @escaping Now = { ProcessInfo.processInfo.systemUptime },
        scheduleTimeout: @escaping TimeoutSchedule = { delay, action in
            DispatchImageTimeoutTask(delay: delay, action: action)
        }
    ) {
        storedPolicy = policy
        self.transport = transport
        self.decoder = decoder
        self.deliver = deliver
        self.now = now
        self.scheduleTimeout = scheduleTimeout
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
        completion: @escaping (UIImage?) -> Void,
        onAcceptedStart: @escaping () -> Void = {}
    ) -> ImageLoadReceipt? {
        let request: Request? = stateQueue.sync {
            let request = Request(
                id: UUID(),
                source: source,
                generation: generation,
                deadline: now() + storedPolicy.requestTimeout,
                completion: completion,
                onAcceptedStart: onAcceptedStart
            )
            if occupiedWorkCountLocked >= storedPolicy.maxConcurrentRequests {
                guard pending.count < storedPolicy.maxPendingRequests else { return nil }
                scheduleDeadlineLocked(for: request)
                pending.append(request)
                return request
            }
            scheduleDeadlineLocked(for: request)
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
        guard isWithinDeadline(request) else {
            expireLocked(request)
            return
        }
        let activeRequest = ActiveRequest(request: request)
        active[request.id] = activeRequest
        let policy = storedPolicy
        let cacheKey = RenderImageCache.key(source: request.source, policy: policy)
        guard isWithinDeadline(request) else {
            expireLocked(request)
            return
        }
        if let cached = RenderImageCache.cache.image(forKey: cacheKey) {
            request.onAcceptedStart()
            finishLocked(request, image: cached)
            return
        }

        if Self.isDataURL(request.source) {
            guard Self.decodedDataURLByteCount(
                request.source,
                maxBytes: policy.maxSourceBytes
            ) != nil, isWithinDeadline(request) else {
                finishLocked(request, image: nil)
                return
            }
            request.onAcceptedStart()
            decodeWorkIds.insert(request.id)
            decodeQueue.async { [weak self] in
                guard let self else { return }
                guard self.isLiveAndWithinDeadline(request) else {
                    self.finishDecode(request, image: nil, cacheKey: cacheKey)
                    return
                }
                let data = Self.decodeDataURL(request.source, maxBytes: policy.maxSourceBytes)
                let image: UIImage? = data.flatMap { data -> UIImage? in
                    guard self.isLiveAndWithinDeadline(request) else { return nil }
                    return self.decoder.decode(data, maxDimension: policy.maxDecodeDimension)
                }
                self.finishDecode(request, image: image, cacheKey: cacheKey)
            }
            return
        }

        guard let url = URL(string: request.source),
              let scheme = url.scheme?.lowercased(),
              scheme == "https" || scheme == "http",
              isWithinDeadline(request)
        else {
            finishLocked(request, image: nil)
            return
        }
        request.onAcceptedStart()
        activeRequest.task = transport.load(url, policy: policy) { [weak self] result in
            guard let self else { return }
            self.stateQueue.async {
                guard request.generation == self.generation,
                      self.active[request.id] != nil,
                      self.isWithinDeadline(request)
                else {
                    return
                }
                self.decodeWorkIds.insert(request.id)
                self.decodeQueue.async {
                    guard self.isLiveAndWithinDeadline(request) else {
                        self.finishDecode(request, image: nil, cacheKey: cacheKey)
                        return
                    }
                    let image: UIImage?
                    switch result {
                    case let .success(data) where data.count <= policy.maxSourceBytes:
                        image = self.isLiveAndWithinDeadline(request)
                            ? self.decoder.decode(data, maxDimension: policy.maxDecodeDimension)
                            : nil
                    default:
                        image = nil
                    }
                    self.finishDecode(request, image: image, cacheKey: cacheKey)
                }
            }
        }
    }

    private func scheduleDeadlineLocked(for request: Request) {
        let remaining = max(0, request.deadline - now())
        deadlineTasks[request.id] = scheduleTimeout(remaining) {
            [weak self] in
            self?.expire(request)
        }
    }

    private func finishDecode(_ request: Request, image: UIImage?, cacheKey: String) {
        stateQueue.async { [weak self] in
            guard let self else { return }
            decodeWorkIds.remove(request.id)
            if request.generation == generation,
               active[request.id] != nil,
               isWithinDeadline(request),
               let image
            {
                let cost = RenderImageCache.retainedCost(image)
                if isWithinDeadline(request) {
                    RenderImageCache.cache.insert(image, forKey: cacheKey, cost: cost)
                }
            }
            if request.generation == generation,
               active[request.id] != nil,
               isWithinDeadline(request)
            {
                finishLocked(request, image: image)
            } else {
                expireLocked(request)
                drainLocked()
            }
        }
    }

    private func finishLocked(_ request: Request, image: UIImage?) {
        guard request.generation == generation,
              active[request.id] != nil,
              isWithinDeadline(request),
              deliveryTickets[request.id] == nil
        else {
            return
        }
        let ticket = DeliveryTicket()
        deliveryTickets[request.id] = ticket
        deliver { [weak self] in
            guard let self else { return }
            let shouldDeliver = stateQueue.sync {
                guard request.generation == self.generation,
                      self.isWithinDeadline(request),
                      self.active.removeValue(forKey: request.id) != nil,
                      self.deliveryTickets.removeValue(forKey: request.id) === ticket,
                      ticket.begin()
                else {
                    return false
                }
                self.drainLocked()
                self.deadlineTasks.removeValue(forKey: request.id)?.cancel()
                return true
            }
            guard shouldDeliver else { return }
            request.completion(image)
            ticket.finish()
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
        let deadlineTasks = Array(self.deadlineTasks.values)
        deliveryTickets.values.forEach { $0.cancel() }
        active.removeAll()
        pending.removeAll()
        deliveryTickets.removeAll()
        self.deadlineTasks.removeAll()
        tasks.forEach { $0.cancel() }
        deadlineTasks.forEach { $0.cancel() }
    }

    private func cancel(_ request: Request) {
        stateQueue.sync {
            guard request.generation == generation else { return }
            if let index = pending.firstIndex(where: { $0.id == request.id }) {
                pending.remove(at: index)
                deadlineTasks.removeValue(forKey: request.id)?.cancel()
                return
            }
            guard let activeRequest = active.removeValue(forKey: request.id) else { return }
            deliveryTickets.removeValue(forKey: request.id)?.cancel()
            activeRequest.task?.cancel()
            deadlineTasks.removeValue(forKey: request.id)?.cancel()
            drainLocked()
        }
    }

    private func isWithinDeadline(_ request: Request) -> Bool {
        now() < request.deadline
    }

    private func isLiveAndWithinDeadline(_ request: Request) -> Bool {
        stateQueue.sync {
            request.generation == generation
                && active[request.id] != nil
                && isWithinDeadline(request)
        }
    }

    private func expire(_ request: Request) {
        stateQueue.sync { expireLocked(request) }
    }

    private func expireLocked(_ request: Request) {
        if let index = pending.firstIndex(where: { $0.id == request.id }) {
            pending.remove(at: index)
        }
        let activeRequest = active.removeValue(forKey: request.id)
        activeRequest?.task?.cancel()
        deliveryTickets.removeValue(forKey: request.id)?.cancel()
        deadlineTasks.removeValue(forKey: request.id)?.cancel()
        drainLocked()
    }

    private static func isDataURL(_ source: String) -> Bool {
        let scan = scanDataURLHeader(source)
        guard let comma = scan.commaIndex, !scan.exceededLimit else { return false }
        let trimmed = source[..<comma].drop(while: { $0.isWhitespace })
        return trimmed.prefix("data:image/".count).lowercased() == "data:image/"
    }

    static func scanDataURLHeader(_ source: String) -> DataURLHeaderScan {
        var scannedBytes = 0
        let bytes = source.utf8
        var index = bytes.startIndex
        while index < bytes.endIndex {
            let byte = bytes[index]
            if byte == 44 {
                return DataURLHeaderScan(
                    commaIndex: index,
                    scannedByteCount: scannedBytes + 1,
                    exceededLimit: false
                )
            }
            guard scannedBytes < dataURLHeaderByteLimit else {
                return DataURLHeaderScan(
                    commaIndex: nil,
                    scannedByteCount: scannedBytes,
                    exceededLimit: true
                )
            }
            scannedBytes += 1
            index = bytes.index(after: index)
        }
        return DataURLHeaderScan(
            commaIndex: nil,
            scannedByteCount: scannedBytes,
            exceededLimit: false
        )
    }

    static func decodedDataURLByteCount(_ source: String, maxBytes: Int) -> Int? {
        let scan = scanDataURLHeader(source)
        guard maxBytes >= 0,
              let comma = scan.commaIndex,
              !scan.exceededLimit
        else {
            return nil
        }
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

        let (encodedLimit, encodedLimitOverflow) = maxSymbols.addingReportingOverflow(
            dataURLEncodedOverheadByteLimit
        )
        let maxEncodedBytes = encodedLimitOverflow ? Int.max : encodedLimit
        var encodedByteCount = 0
        var symbolCount = 0
        var trailingPadding = 0
        var sawPadding = false
        for byte in source[source.index(after: comma)...].utf8 {
            let (nextEncodedCount, encodedCountOverflow) = encodedByteCount.addingReportingOverflow(1)
            guard !encodedCountOverflow, nextEncodedCount <= maxEncodedBytes else { return nil }
            encodedByteCount = nextEncodedCount
            if byte == 9 || byte == 10 || byte == 13 || byte == 32 { continue }
            let isBase64Symbol = (65...90).contains(byte)
                || (97...122).contains(byte)
                || (48...57).contains(byte)
                || byte == 43
                || byte == 47
            guard isBase64Symbol || byte == 61 else { return nil }
            let (nextCount, countOverflow) = symbolCount.addingReportingOverflow(1)
            guard !countOverflow, nextCount <= maxSymbols else { return nil }
            symbolCount = nextCount
            if byte == 61 {
                guard trailingPadding < 2 else { return nil }
                trailingPadding += 1
                sawPadding = true
            } else {
                guard !sawPadding else { return nil }
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
        let scan = scanDataURLHeader(source)
        guard let comma = scan.commaIndex,
              !scan.exceededLimit else { return nil }
        let trimmed = source[...].drop(while: { $0.isWhitespace })
        guard trimmed.indices.contains(comma),
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
