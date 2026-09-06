import UIKit
import ImageIO
import CryptoKit

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
    let deliver: Delivery
    let now: Now
    private let scheduleTimeout: TimeoutSchedule
    private var storedPolicy: ImageLoadingPolicy
    var generation: UInt64 = 0
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
