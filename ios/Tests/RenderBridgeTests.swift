import XCTest
import CoreText

// MARK: - RenderBridge Tests

final class RenderBridgeTests: XCTestCase {

    func securityFixtures() throws -> [String: Any] {
        // Resolution order: explicit env override, the copy bundled with the
        // test target (required on physical devices, where the repository
        // path does not exist), then the repository-relative host path used
        // by simulator runs.
        let configured = ProcessInfo.processInfo.environment["SECURITY_FIXTURE_PATH"]
        let bundledURL = Bundle(for: RenderBridgeTests.self)
            .url(forResource: "security-contract-fixtures", withExtension: "json")
        let defaultURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("scripts/tests/security-contract-fixtures.json")
        let url = configured.map(URL.init(fileURLWithPath:)) ?? bundledURL ?? defaultURL
        let data = try Data(contentsOf: url)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }


    func ffiResultError() -> FfiError {
        FfiError(
            domain: "engine",
            code: "FAILED",
            message: "engine failure",
            requestId: nil,
            operationIndex: nil,
            limit: nil,
            actual: nil,
            detailsJson: nil
        )
    }

    func assertFfiResultContractFailure(
        _ result: [String: Any],
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertNil(result["value"], file: file, line: line)
        let error = result["error"] as? [String: Any]
        XCTAssertEqual(error?["domain"] as? String, "boundary", file: file, line: line)
        XCTAssertEqual(error?["code"] as? String, "FFI_RESULT_INVALID", file: file, line: line)
        XCTAssertEqual(
            error?["message"] as? String,
            "v2 result must carry exactly one of value/error",
            file: file,
            line: line
        )
    }

    func commandPreparation(result: String) -> String? {
        guard let data = result.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }
        return object["blockedReason"] as? String
    }

    let baseFont = UIFont.systemFont(ofSize: 16)
    let textColor = UIColor.black























}

func imagePolicy(
    maxSourceBytes: Int = 10 * 1024 * 1024,
    connectTimeout: TimeInterval = 10,
    readTimeout: TimeInterval = 20,
    requestTimeout: TimeInterval = 60,
    maxConcurrentRequests: Int = 2,
    maxPendingRequests: Int = 64,
    maxDecodeDimension: Int = 2_048
) -> ImageLoadingPolicy {
    ImageLoadingPolicy(
        maxSourceBytes: maxSourceBytes,
        connectTimeout: connectTimeout,
        readTimeout: readTimeout,
        requestTimeout: requestTimeout,
        maxConcurrentRequests: maxConcurrentRequests,
        maxPendingRequests: maxPendingRequests,
        maxDecodeDimension: maxDecodeDimension
    )
}

func onePixelImage() -> UIImage {
    let renderer = UIGraphicsImageRenderer(size: CGSize(width: 1, height: 1))
    return renderer.image { context in
        UIColor.red.setFill()
        context.fill(CGRect(x: 0, y: 0, width: 1, height: 1))
    }
}

func paddedBackingImage(bytesPerRow: Int, height: Int) -> UIImage {
    let data = Data(repeating: 0, count: bytesPerRow * height)
    let provider = CGDataProvider(data: data as CFData)!
    let image = CGImage(
        width: 1,
        height: height,
        bitsPerComponent: 8,
        bitsPerPixel: 32,
        bytesPerRow: bytesPerRow,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue),
        provider: provider,
        decode: nil,
        shouldInterpolate: false,
        intent: .defaultIntent
    )!
    return UIImage(cgImage: image)
}

func imageRenderJSON(source: String) -> String {
    """
    [{"type":"voidBlock","nodeType":"image","docPos":1,"attrs":{"src":"\(source)"}}]
    """
}

final class ManualImageTimeoutScheduler {
    private final class ScheduledTask: ImageLoadingTask {
        let delay: TimeInterval
        let action: () -> Void
        var cancelCount = 0

        var cancelled: Bool { cancelCount > 0 }

        init(delay: TimeInterval, action: @escaping () -> Void) {
            self.delay = delay
            self.action = action
        }

        func cancel() {
            cancelCount += 1
        }
    }

    private var tasks: [ScheduledTask] = []

    var pendingDelays: [TimeInterval] {
        tasks.filter { !$0.cancelled }.map(\.delay)
    }

    var allDelays: [TimeInterval] { tasks.map(\.delay) }
    var allCancelCounts: [Int] { tasks.map(\.cancelCount) }

    lazy var schedule: (TimeInterval, @escaping () -> Void) -> ImageLoadingTask = {
        [weak self] delay, action in
        let task = ScheduledTask(delay: delay, action: action)
        self?.tasks.append(task)
        return task
    }

    func fireCancelledTasks() {
        let cancelled = tasks.filter(\.cancelled)
        tasks.removeAll(where: \.cancelled)
        cancelled.forEach { task in
            if !task.cancelled { task.action() }
        }
    }

    func fireNext() {
        guard let index = tasks.firstIndex(where: { !$0.cancelled }) else { return }
        let task = tasks.remove(at: index)
        task.action()
    }

    func fire(delay: TimeInterval) {
        guard let index = tasks.firstIndex(where: { !$0.cancelled && $0.delay == delay }) else {
            return
        }
        let task = tasks[index]
        task.action()
    }

    func fireAllIncludingCancelled() {
        tasks.forEach { $0.action() }
    }

    func fireFirstCancelledIgnoringCancellation() {
        tasks.first(where: \.cancelled)?.action()
    }

    func fireAllActive() {
        tasks.filter { !$0.cancelled }.forEach { $0.action() }
    }
}

final class ConcurrentImageTimeoutScheduler {
    private final class ScheduledTask: ImageLoadingTask {
        let delay: TimeInterval
        let action: () -> Void
        private let lock = NSLock()
        private var cancelled = false

        init(delay: TimeInterval, action: @escaping () -> Void) {
            self.delay = delay
            self.action = action
        }

        func cancel() {
            lock.lock()
            cancelled = true
            lock.unlock()
        }

        var isCancelled: Bool {
            lock.lock()
            defer { lock.unlock() }
            return cancelled
        }
    }

    let lock = NSLock()
    private var tasks: [ScheduledTask] = []

    lazy var schedule: (TimeInterval, @escaping () -> Void) -> ImageLoadingTask = {
        [weak self] delay, action in
        let task = ScheduledTask(delay: delay, action: action)
        guard let self else { return task }
        self.lock.lock()
        self.tasks.append(task)
        self.lock.unlock()
        return task
    }

    var totalCount: Int {
        lock.lock()
        defer { lock.unlock() }
        return tasks.count
    }

    var pendingCount: Int {
        lock.lock()
        let snapshot = tasks
        lock.unlock()
        return snapshot.filter { !$0.isCancelled }.count
    }

    func fire(delay: TimeInterval) {
        lock.lock()
        let task = tasks.first { $0.delay == delay && !$0.isCancelled }
        lock.unlock()
        task?.action()
    }
}

final class ManualImageClock {
    let lock = NSLock()
    var value: TimeInterval = 0
    lazy var now: () -> TimeInterval = { [weak self] in
        guard let self else { return .infinity }
        self.lock.lock()
        defer { self.lock.unlock() }
        return self.value
    }
    func advance(to value: TimeInterval) {
        lock.lock()
        self.value = value
        lock.unlock()
    }
}

final class DeadlineAdvancingImageDecoder: ImageDataDecoding {
    let clock: ManualImageClock
    let image: UIImage

    init(clock: ManualImageClock, image: UIImage) {
        self.clock = clock
        self.image = image
    }

    func decode(_ data: Data, maxDimension: Int) -> UIImage? {
        clock.advance(to: 61)
        return image
    }
}

final class ManualImageDeliveryScheduler {
    let condition = NSCondition()
    private var actions: [() -> Void] = []

    lazy var schedule: (@escaping () -> Void) -> Void = { [weak self] action in
        guard let self else { return }
        condition.lock()
        actions.append(action)
        condition.broadcast()
        condition.unlock()
    }

    func waitUntilScheduled(timeout: TimeInterval) -> Bool {
        condition.lock()
        defer { condition.unlock() }
        let deadline = Date().addingTimeInterval(timeout)
        while actions.isEmpty {
            guard condition.wait(until: deadline) else { return false }
        }
        return true
    }

    func runAll() {
        condition.lock()
        let pending = actions
        actions.removeAll()
        condition.unlock()
        pending.forEach { $0() }
    }
}

final class BlockingImageDecoder: ImageDataDecoding {
    let condition = NSCondition()
    let result: UIImage?
    private var permits = 0
    private var concurrentDecodes = 0
    var decodeCount = 0
    var maximumConcurrentDecodes = 0

    init(image: UIImage?) {
        result = image
    }

    func decode(_ data: Data, maxDimension: Int) -> UIImage? {
        condition.lock()
        decodeCount += 1
        concurrentDecodes += 1
        maximumConcurrentDecodes = max(maximumConcurrentDecodes, concurrentDecodes)
        condition.broadcast()
        while permits == 0 {
            condition.unlock()
            Thread.sleep(forTimeInterval: 0.001)
            condition.lock()
        }
        permits -= 1
        concurrentDecodes -= 1
        condition.unlock()
        return result
    }

    func waitForDecodeCount(_ expected: Int, timeout: TimeInterval) -> Bool {
        condition.lock()
        defer { condition.unlock() }
        let deadline = Date().addingTimeInterval(timeout)
        while decodeCount < expected {
            guard condition.wait(until: deadline) else { return false }
        }
        return true
    }

    func releaseNext() {
        condition.lock()
        permits += 1
        condition.broadcast()
        condition.unlock()
    }
}

final class RecordingImageDecoder: ImageDataDecoding {
    let lock = NSLock()
    let result: UIImage?
    var decodeCount = 0
    var calledOnMainThread: Bool?

    init(image: UIImage? = nil) {
        result = image
    }

    func decode(_ data: Data, maxDimension: Int) -> UIImage? {
        lock.lock()
        decodeCount += 1
        calledOnMainThread = Thread.isMainThread
        lock.unlock()
        return result
    }
}

private final class TestImageLoadingTask: ImageLoadingTask {
    private let onCancel: () -> Void

    init(onCancel: @escaping () -> Void = {}) {
        self.onCancel = onCancel
    }

    func cancel() {
        onCancel()
    }
}

final class ImmediateImageTransport: ImageLoadingTransport {
    let result: Result<Data, Error>
    var receivedPolicy: ImageLoadingPolicy?

    init(result: Result<Data, Error>) {
        self.result = result
    }

    func load(
        _ url: URL,
        policy: ImageLoadingPolicy,
        completion: @escaping (Result<Data, Error>) -> Void
    ) -> ImageLoadingTask {
        receivedPolicy = policy
        completion(result)
        return TestImageLoadingTask()
    }
}

final class HoldingImageTransport: ImageLoadingTransport {
    let lock = NSLock()
    private var completions: [(Result<Data, Error>) -> Void] = []
    var requestCount = 0
    var cancelCount = 0
    var receivedPolicies: [ImageLoadingPolicy] = []

    func load(
        _ url: URL,
        policy: ImageLoadingPolicy,
        completion: @escaping (Result<Data, Error>) -> Void
    ) -> ImageLoadingTask {
        lock.lock()
        requestCount += 1
        receivedPolicies.append(policy)
        completions.append(completion)
        lock.unlock()
        return TestImageLoadingTask { [weak self] in
            self?.lock.lock()
            self?.cancelCount += 1
            self?.lock.unlock()
        }
    }

    func completeAll(with result: Result<Data, Error>) {
        lock.lock()
        let callbacks = completions
        completions.removeAll()
        lock.unlock()
        callbacks.forEach { $0(result) }
    }
}
