import UIKit
import ImageIO
import CryptoKit

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

    let connectTimeout: TimeInterval
    let readTimeout: TimeInterval
    let requestTimeout: TimeInterval
    private let schedule: Schedule
    private let onTimeout: () -> Void
    let lock = NSLock()
    private var phaseTimer: ImageLoadingTask?
    private var totalTimer: ImageLoadingTask?
    private var phaseGeneration: UInt64 = 0
    private var totalGeneration: UInt64 = 0
    var finished = false

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

final class DispatchImageTimeoutTask: ImageLoadingTask {
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

final class DefaultImageDataDecoder: ImageDataDecoding {
    func decode(_ data: Data, maxDimension: Int) -> UIImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              CGImageSourceGetType(source) != nil else {
            return NativeSVGImageDecoder.decode(data, maxDimension: maxDimension)
        }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxDimension,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
        else {
            return NativeSVGImageDecoder.decode(data, maxDimension: maxDimension)
        }
        return UIImage(cgImage: image)
    }
}

final class URLSessionImageTask: NSObject, ImageLoadingTask, URLSessionDataDelegate {
    let policy: ImageLoadingPolicy
    let completion: (Result<Data, Error>) -> Void
    let lock = NSLock()
    private var buffer = Data()
    private var session: URLSession?
    var task: URLSessionDataTask?
    private var timeoutController: ImageRequestTimeoutController?
    var finished = false

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

    func finish(_ result: Result<Data, Error>, deliver: Bool = true) {
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

final class URLSessionImageTransport: ImageLoadingTransport {
    func load(
        _ url: URL,
        policy: ImageLoadingPolicy,
        completion: @escaping (Result<Data, Error>) -> Void
    ) -> ImageLoadingTask {
        URLSessionImageTask(url: url, policy: policy, completion: completion)
    }
}
