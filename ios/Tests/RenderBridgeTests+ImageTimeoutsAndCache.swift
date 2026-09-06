import XCTest
import CoreText

extension RenderBridgeTests {
    func testImageTimeoutControllerSeparatesConnectAndReadIdleTimeouts() {
        let scheduler = ManualImageTimeoutScheduler()
        var timeoutCount = 0
        let controller = ImageRequestTimeoutController(
            connectTimeout: 10,
            readTimeout: 20,
            requestTimeout: 60,
            schedule: scheduler.schedule,
            onTimeout: { timeoutCount += 1 }
        )

        controller.start()
        XCTAssertEqual(scheduler.pendingDelays, [10, 60])

        controller.receivedResponse()
        XCTAssertEqual(scheduler.pendingDelays, [60, 20])
        scheduler.fireCancelledTasks()
        XCTAssertEqual(timeoutCount, 0)

        controller.receivedData()
        XCTAssertEqual(scheduler.pendingDelays, [60, 20])
        scheduler.fireNext()
        XCTAssertEqual(timeoutCount, 1)
        XCTAssertEqual(scheduler.pendingDelays, [])
    }

    func testImageTimeoutControllerTotalDeadlineDoesNotResetAndCancelsAllTimersOnce() {
        let scheduler = ManualImageTimeoutScheduler()
        var timeoutCount = 0
        let controller = ImageRequestTimeoutController(
            connectTimeout: 10,
            readTimeout: 20,
            requestTimeout: 60,
            schedule: scheduler.schedule,
            onTimeout: { timeoutCount += 1 }
        )

        controller.start()
        controller.receivedResponse()
        controller.receivedData()
        XCTAssertEqual(scheduler.allDelays, [10, 60, 20, 20])
        XCTAssertEqual(scheduler.pendingDelays, [60, 20])

        scheduler.fire(delay: 60)
        XCTAssertEqual(timeoutCount, 1)
        XCTAssertEqual(scheduler.pendingDelays, [])
        XCTAssertTrue(scheduler.allCancelCounts.allSatisfy { $0 == 1 })

        controller.cancel()
        scheduler.fireAllIncludingCancelled()
        XCTAssertEqual(timeoutCount, 1)
        XCTAssertTrue(scheduler.allCancelCounts.allSatisfy { $0 == 1 })
    }

    func testCancelledPhaseTimerCallbackCannotFinishReplacementPhase() {
        let scheduler = ManualImageTimeoutScheduler()
        var timeoutCount = 0
        let controller = ImageRequestTimeoutController(
            connectTimeout: 10,
            readTimeout: 20,
            requestTimeout: 60,
            schedule: scheduler.schedule,
            onTimeout: { timeoutCount += 1 }
        )

        controller.start()
        controller.receivedResponse()
        scheduler.fireFirstCancelledIgnoringCancellation()

        XCTAssertEqual(timeoutCount, 0)
        XCTAssertEqual(scheduler.pendingDelays, [60, 20])
        scheduler.fire(delay: 20)
        XCTAssertEqual(timeoutCount, 1)
    }

    func testURLSessionUsesPolicySecondsAndTotalResourceDeadline() {
        let configuration = URLSessionImageTask.configuration(
            policy: imagePolicy(connectTimeout: 10, readTimeout: 20, requestTimeout: 60)
        )

        XCTAssertEqual(configuration.timeoutIntervalForRequest, 20)
        XCTAssertEqual(configuration.timeoutIntervalForResource, 60)
    }

    func testURLSessionTimeoutRaceDeliversOnceAndCannotRearmAfterTerminalState() {
        for _ in 0..<100 {
            let scheduler = ConcurrentImageTimeoutScheduler()
            let completionLock = NSLock()
            var completionCount = 0
            let task = URLSessionImageTask(
                policy: imagePolicy(connectTimeout: 10, readTimeout: 20, requestTimeout: 60),
                timeoutSchedule: scheduler.schedule
            ) { _ in
                completionLock.lock()
                completionCount += 1
                completionLock.unlock()
            }
            let dataTask = URLSession.shared.dataTask(with: URL(string: "https://example.com")!)
            let group = DispatchGroup()
            let queue = DispatchQueue(label: "URLSessionImageTask.timeout-race", attributes: .concurrent)

            group.enter()
            queue.async {
                scheduler.fire(delay: 60)
                group.leave()
            }
            group.enter()
            queue.async {
                let response = URLResponse(
                    url: dataTask.originalRequest!.url!,
                    mimeType: "image/png",
                    expectedContentLength: 1,
                    textEncodingName: nil
                )
                task.urlSession(
                    URLSession.shared,
                    dataTask: dataTask,
                    didReceive: response,
                    completionHandler: { _ in }
                )
                task.urlSession(URLSession.shared, dataTask: dataTask, didReceive: Data([1]))
                group.leave()
            }
            XCTAssertEqual(group.wait(timeout: .now() + 1), .success)

            completionLock.lock()
            let terminalCount = completionCount
            completionLock.unlock()
            XCTAssertEqual(terminalCount, 1)
            XCTAssertEqual(scheduler.pendingCount, 0)

            let scheduledAtTerminal = scheduler.totalCount
            let response = URLResponse(
                url: dataTask.originalRequest!.url!,
                mimeType: "image/png",
                expectedContentLength: 1,
                textEncodingName: nil
            )
            task.urlSession(
                URLSession.shared,
                dataTask: dataTask,
                didReceive: response,
                completionHandler: { _ in }
            )
            task.urlSession(URLSession.shared, dataTask: dataTask, didReceive: Data([2]))
            XCTAssertEqual(scheduler.totalCount, scheduledAtTerminal)
            XCTAssertEqual(scheduler.pendingCount, 0)
        }
    }

    func testImageCacheEvictsLeastRecentlyUsedEntriesByDecodedCost() {
        let cache = RenderImageCostCache(costLimit: 10)
        let image = onePixelImage()

        cache.insert(image, forKey: "one", cost: 6)
        cache.insert(image, forKey: "two", cost: 3)
        XCTAssertNotNil(cache.image(forKey: "one"))
        cache.insert(image, forKey: "three", cost: 6)

        XCTAssertNil(cache.image(forKey: "one"))
        XCTAssertNil(cache.image(forKey: "two"))
        XCTAssertNotNil(cache.image(forKey: "three"))
        XCTAssertLessThanOrEqual(cache.totalCost, 10)
    }

    func testImageCacheKeyIncludesEveryPolicyLimit() {
        let source = "https://example.com/image.png"
        let baseline = imagePolicy()
        let baselineKey = RenderImageCache.key(source: source, policy: baseline)
        let variants = [
            imagePolicy(maxSourceBytes: baseline.maxSourceBytes + 1),
            imagePolicy(connectTimeout: baseline.connectTimeout + 1),
            imagePolicy(readTimeout: baseline.readTimeout + 1),
            imagePolicy(requestTimeout: baseline.requestTimeout + 1),
            imagePolicy(maxConcurrentRequests: baseline.maxConcurrentRequests + 1),
            imagePolicy(maxPendingRequests: baseline.maxPendingRequests + 1),
            imagePolicy(maxDecodeDimension: baseline.maxDecodeDimension + 1),
        ]

        for variant in variants {
            XCTAssertNotEqual(RenderImageCache.key(source: source, policy: variant), baselineKey)
        }
    }

    func testImageCacheUsesFixedDigestWithoutRetainingSource() {
        let source = "data:image/png;base64," + String(repeating: "A", count: 1_000_000)
        let key = RenderImageCache.key(source: source, policy: imagePolicy())

        XCTAssertEqual(key.utf8.count, 64)
        XCTAssertFalse(key.contains("data:image"))
        XCTAssertTrue(key.allSatisfy { $0.isHexDigit && !$0.isUppercase })
    }

    func testImageCacheRetainedCostIncludesDigestMetadataAndEntryOverhead() {
        let image = onePixelImage()
        let decoded = RenderImageCache.decodedCost(image)

        XCTAssertEqual(
            RenderImageCache.retainedCost(image),
            decoded + 64 + RenderImageCache.cacheEntryOverhead
        )
    }

    func testImageCacheRetainedCostUsesBackingRowBytesAndSaturatesOverflow() {
        let image = paddedBackingImage(bytesPerRow: 64, height: 2)

        XCTAssertEqual(
            RenderImageCache.retainedCost(image),
            128 + 64 + RenderImageCache.cacheEntryOverhead
        )
        XCTAssertEqual(
            RenderImageCache.backingCost(
                bytesPerRow: Int.max,
                height: 2,
                fallback: 4
            ),
            Int.max
        )
    }

}
