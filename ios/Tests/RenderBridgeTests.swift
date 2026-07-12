import XCTest
import CoreText

// MARK: - RenderBridge Tests

final class RenderBridgeTests: XCTestCase {
    private func securityFixtures() throws -> [String: Any] {
        let configured = ProcessInfo.processInfo.environment["SECURITY_FIXTURE_PATH"]
        let defaultURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("scripts/tests/security-contract-fixtures.json")
        let data = try Data(contentsOf: configured.map(URL.init(fileURLWithPath:)) ?? defaultURL)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    func testStructuredEditorCreationIdUsesExactIntegerSemantics() {
        XCTAssertEqual(createdEditorId(#"{"editorId":42}"#), 42)
        XCTAssertNil(createdEditorId(#"{"editorId":true}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":-1}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":1.5}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":1e3}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":9223372036854775808}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":18446744073709551616}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":18446744073709551615.0}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":7,"editorId":8}"#))
        XCTAssertNil(createdEditorId(#"{"editorId":1.5,"nested":{"editorId":7}}"#))
        XCTAssertNil(createdEditorId(#"{"error":{"code":"CONFIG_INVALID"},"editorId":7}"#))
    }

    // MARK: - Test Fixtures

    private let baseFont = UIFont.systemFont(ofSize: 16)
    private let textColor = UIColor.black

    func testImageLoadingPolicyDefaultsMatchPublicContract() {
        let policy = ImageLoadingPolicy.from(json: nil)

        XCTAssertEqual(policy.maxSourceBytes, 10 * 1024 * 1024)
        XCTAssertEqual(policy.connectTimeout, 10)
        XCTAssertEqual(policy.readTimeout, 20)
        XCTAssertEqual(policy.requestTimeout, 60)
        XCTAssertEqual(policy.maxConcurrentRequests, 2)
        XCTAssertEqual(policy.maxPendingRequests, 64)
        XCTAssertEqual(policy.maxDecodeDimension, 2_048)
    }

    func testImageLoadingPolicyParsesPositiveIntegersAndDefaultsInvalidValues() {
        let policy = ImageLoadingPolicy.from(json: """
        {
          "maxSourceBytes": 4096,
          "connectTimeoutMs": 1500,
          "readTimeoutMs": 2750,
          "requestTimeoutMs": 4500,
          "maxConcurrentRequests": 3,
          "maxPendingRequests": 7,
          "maxDecodeDimensionPx": 512
        }
        """)

        XCTAssertEqual(policy.maxSourceBytes, 4096)
        XCTAssertEqual(policy.connectTimeout, 1.5)
        XCTAssertEqual(policy.readTimeout, 2.75)
        XCTAssertEqual(policy.requestTimeout, 4.5)
        XCTAssertEqual(policy.maxConcurrentRequests, 3)
        XCTAssertEqual(policy.maxPendingRequests, 7)
        XCTAssertEqual(policy.maxDecodeDimension, 512)

        let invalid = ImageLoadingPolicy.from(json: """
        {"maxSourceBytes":0,"connectTimeoutMs":-1,"readTimeoutMs":"20","requestTimeoutMs":600001,"maxConcurrentRequests":17,"maxPendingRequests":513,"maxDecodeDimensionPx":8193}
        """)
        XCTAssertEqual(invalid, .default)
    }

    func testImageLoadingPolicyAcceptsExactHardCeilings() {
        let policy = ImageLoadingPolicy.from(json: """
        {
          "maxSourceBytes": 67108864,
          "connectTimeoutMs": 600000,
          "readTimeoutMs": 600000,
          "requestTimeoutMs": 600000,
          "maxConcurrentRequests": 16,
          "maxPendingRequests": 512,
          "maxDecodeDimensionPx": 8192
        }
        """)

        XCTAssertEqual(policy.maxSourceBytes, 64 * 1024 * 1024)
        XCTAssertEqual(policy.connectTimeout, 600)
        XCTAssertEqual(policy.readTimeout, 600)
        XCTAssertEqual(policy.requestTimeout, 600)
        XCTAssertEqual(policy.maxConcurrentRequests, 16)
        XCTAssertEqual(policy.maxPendingRequests, 512)
        XCTAssertEqual(policy.maxDecodeDimension, 8_192)
    }

    func testImageLoaderRejectsOversizedDataURLWithoutDecoding() {
        let decoder = RecordingImageDecoder()
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(maxSourceBytes: 3),
            transport: HoldingImageTransport(),
            decoder: decoder
        )
        let completion = expectation(description: "oversized source rejected")

        XCTAssertTrue(owner.loadImage(source: "data:image/png;base64,AQIDBA==") { image in
            XCTAssertNil(image)
            completion.fulfill()
        })

        wait(for: [completion], timeout: 1)
        XCTAssertEqual(decoder.decodeCount, 0)
    }

    func testImageLoaderRejectsTimeoutAndOversizedRemoteResponses() {
        let timeoutTransport = ImmediateImageTransport(result: .failure(URLError(.timedOut)))
        let timeoutOwner = RenderImageLoadOwner(
            policy: imagePolicy(maxSourceBytes: 4, connectTimeout: 1.5, readTimeout: 2.75),
            transport: timeoutTransport
        )
        let timeout = expectation(description: "timeout")
        XCTAssertTrue(timeoutOwner.loadImage(source: "https://example.com/timeout.png") { image in
            XCTAssertNil(image)
            timeout.fulfill()
        })

        let oversizedTransport = ImmediateImageTransport(result: .success(Data(repeating: 1, count: 5)))
        let oversizedOwner = RenderImageLoadOwner(
            policy: imagePolicy(maxSourceBytes: 4),
            transport: oversizedTransport
        )
        let oversized = expectation(description: "oversized response")
        XCTAssertTrue(oversizedOwner.loadImage(source: "https://example.com/large.png") { image in
            XCTAssertNil(image)
            oversized.fulfill()
        })

        wait(for: [timeout, oversized], timeout: 1)
        XCTAssertEqual(timeoutTransport.receivedPolicy?.connectTimeout, 1.5)
        XCTAssertEqual(timeoutTransport.receivedPolicy?.readTimeout, 2.75)
    }

    func testImageLoaderBoundsPendingWorkAndCancelsOwnerGeneration() {
        let transport = HoldingImageTransport()
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(maxConcurrentRequests: 1, maxPendingRequests: 1),
            transport: transport
        )
        let cancelled = expectation(description: "cancelled work does not complete")
        cancelled.isInverted = true

        XCTAssertTrue(owner.loadImage(source: "https://example.com/one.png") { _ in cancelled.fulfill() })
        XCTAssertTrue(owner.loadImage(source: "https://example.com/two.png") { _ in cancelled.fulfill() })
        XCTAssertFalse(owner.loadImage(source: "https://example.com/three.png") { _ in cancelled.fulfill() })
        XCTAssertEqual(transport.requestCount, 1)

        owner.cancelAll()

        XCTAssertEqual(transport.cancelCount, 1)
        transport.completeAll(with: .success(Data()))
        wait(for: [cancelled], timeout: 0.1)
    }

    func testImageLoadReceiptCancelsOnlyItsRequest() {
        let transport = HoldingImageTransport()
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(maxConcurrentRequests: 2),
            transport: transport
        )

        let first = owner.startImageLoad(source: "https://example.com/one.png") { _ in }
        _ = owner.startImageLoad(source: "https://example.com/two.png") { _ in }
        first?.cancel()

        XCTAssertEqual(transport.requestCount, 2)
        XCTAssertEqual(transport.cancelCount, 1)
    }

    func testStreamingLimitRejectsSingleChunkLargerThanMaximumWithoutUnderflow() {
        XCTAssertTrue(
            URLSessionImageTask.wouldExceedLimit(
                currentCount: 0,
                incomingCount: Int.max,
                maxBytes: 10 * 1024 * 1024
            )
        )
        XCTAssertFalse(
            URLSessionImageTask.wouldExceedLimit(
                currentCount: 4,
                incomingCount: 6,
                maxBytes: 10
            )
        )
    }

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

    func testAbsoluteRequestDeadlineStartsAtAdmissionAndSuppressesQueuedWork() {
        let clock = ManualImageClock()
        let deadlines = ManualImageTimeoutScheduler()
        let transport = HoldingImageTransport()
        let delivery = ManualImageDeliveryScheduler()
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(requestTimeout: 60, maxConcurrentRequests: 1),
            transport: transport,
            deliver: delivery.schedule,
            now: clock.now,
            scheduleTimeout: deadlines.schedule
        )
        let completion = expectation(description: "expired queued request is not delivered")
        completion.isInverted = true

        XCTAssertNotNil(owner.startImageLoad(source: "https://example.com/active.png") { _ in })
        XCTAssertNotNil(owner.startImageLoad(source: "https://example.com/queued.png") { _ in
            completion.fulfill()
        })
        XCTAssertEqual(transport.requestCount, 1)

        clock.advance(to: 61)
        deadlines.fireAllActive()
        transport.completeAll(with: .success(Data()))
        delivery.runAll()

        XCTAssertEqual(transport.requestCount, 1)
        wait(for: [completion], timeout: 0.05)
    }

    func testSharedWhitespaceBase64AndTrickleFixturesExecuteAgainstIOSBoundary() throws {
        let fixtures = try securityFixtures()
        let whitespace = try XCTUnwrap(fixtures["whitespaceBase64"] as? [String: Any])
        XCTAssertEqual(whitespace["whitespaceCountsTowardAdmission"] as? Bool, true)
        XCTAssertEqual(
            RenderImageLoadOwner.decodedDataURLByteCount(
                try XCTUnwrap(whitespace["source"] as? String),
                maxBytes: 1
            ),
            1
        )

        let trickle = try XCTUnwrap(fixtures["trickleDeadline"] as? [String: Any])
        let requestTimeout = try XCTUnwrap(trickle["requestTimeoutMs"] as? Double) / 1_000
        let expectedTerminal = try XCTUnwrap(trickle["expectedTerminalMs"] as? Double) / 1_000
        let arrivals = try XCTUnwrap(trickle["byteArrivalMs"] as? [Double])
        XCTAssertTrue(arrivals.contains { $0 > expectedTerminal * 1_000 })
        XCTAssertEqual(trickle["expectedOutcome"] as? String, "timeout")

        let clock = ManualImageClock()
        let deadlines = ManualImageTimeoutScheduler()
        let completion = expectation(description: "fixture deadline suppresses delivery")
        completion.isInverted = true
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(requestTimeout: requestTimeout),
            transport: HoldingImageTransport(),
            now: clock.now,
            scheduleTimeout: deadlines.schedule
        )
        XCTAssertNotNil(owner.startImageLoad(source: "https://example.com/trickle.png") { _ in
            completion.fulfill()
        })
        XCTAssertEqual(deadlines.allDelays, [expectedTerminal])
        clock.advance(to: expectedTerminal)
        deadlines.fireAllActive()
        wait(for: [completion], timeout: 0.05)
    }

    func testRejectedImageAdmissionDoesNotRetainDeadlineHandle() {
        let deadlines = ManualImageTimeoutScheduler()
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(maxConcurrentRequests: 1, maxPendingRequests: 1),
            transport: HoldingImageTransport(),
            scheduleTimeout: deadlines.schedule
        )

        XCTAssertNotNil(owner.startImageLoad(source: "https://example.com/active.png") { _ in })
        XCTAssertNotNil(owner.startImageLoad(source: "https://example.com/pending.png") { _ in })
        XCTAssertNil(owner.startImageLoad(source: "https://example.com/rejected.png") { _ in })
        XCTAssertEqual(deadlines.allDelays.count, 2)
    }

    func testAbsoluteRequestDeadlineSuppressesStaleMainDelivery() {
        let clock = ManualImageClock()
        let deadlines = ManualImageTimeoutScheduler()
        let delivery = ManualImageDeliveryScheduler()
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(requestTimeout: 60),
            transport: ImmediateImageTransport(result: .failure(URLError(.cannotDecodeContentData))),
            deliver: delivery.schedule,
            now: clock.now,
            scheduleTimeout: deadlines.schedule
        )
        let completion = expectation(description: "post-deadline delivery suppressed")
        completion.isInverted = true

        XCTAssertNotNil(owner.startImageLoad(source: "https://example.com/image.png") { _ in
            completion.fulfill()
        })
        XCTAssertTrue(delivery.waitUntilScheduled(timeout: 1))
        clock.advance(to: 61)
        deadlines.fireAllActive()
        delivery.runAll()

        wait(for: [completion], timeout: 0.05)
    }

    func testAbsoluteRequestDeadlineRejectsDecodeResultAndCacheCommit() {
        let clock = ManualImageClock()
        let deadlines = ManualImageTimeoutScheduler()
        let source = "https://example.com/deadline-\(UUID().uuidString).png"
        let policy = imagePolicy(requestTimeout: 60)
        let key = RenderImageCache.key(source: source, policy: policy)
        let owner = RenderImageLoadOwner(
            policy: policy,
            transport: ImmediateImageTransport(result: .success(Data([1]))),
            decoder: DeadlineAdvancingImageDecoder(clock: clock, image: onePixelImage()),
            now: clock.now,
            scheduleTimeout: deadlines.schedule
        )
        let completion = expectation(description: "expired decode is not delivered")
        completion.isInverted = true

        XCTAssertNil(RenderImageCache.cache.image(forKey: key))
        XCTAssertNotNil(owner.startImageLoad(source: source) { _ in completion.fulfill() })

        wait(for: [completion], timeout: 0.1)
        XCTAssertNil(RenderImageCache.cache.image(forKey: key))
    }

    func testEditorTextViewsKeepConfiguredImageOwnersAcrossInternalRenders() {
        let firstTransport = HoldingImageTransport()
        let secondTransport = HoldingImageTransport()
        let firstOwner = RenderImageLoadOwner(
            policy: imagePolicy(maxSourceBytes: 111),
            transport: firstTransport
        )
        let secondOwner = RenderImageLoadOwner(
            policy: imagePolicy(maxSourceBytes: 222),
            transport: secondTransport
        )
        let first = EditorTextView(frame: .zero, textContainer: nil)
        let second = EditorTextView(frame: .zero, textContainer: nil)
        first.imageLoadOwner = firstOwner
        second.imageLoadOwner = secondOwner

        first.applyRenderJSON(imageRenderJSON(source: "https://example.com/first.png"))
        second.applyRenderJSON(imageRenderJSON(source: "https://example.com/second.png"))

        XCTAssertEqual(firstTransport.receivedPolicies.map(\.maxSourceBytes), [111])
        XCTAssertEqual(secondTransport.receivedPolicies.map(\.maxSourceBytes), [222])
    }

    func testNativeEditKeepsUsingConfiguredImagePolicyForLaterInternalRenders() {
        let transport = HoldingImageTransport()
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(maxSourceBytes: 333),
            transport: transport
        )
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        let textView = EditorTextView(frame: .zero, textContainer: nil)
        textView.imageLoadOwner = owner
        textView.bindEditor(id: editorId)

        textView.insertText("A")
        textView.applyRenderJSON(imageRenderJSON(source: "https://example.com/after-edit.png"))

        XCTAssertTrue(textView.imageLoadOwner === owner)
        XCTAssertEqual(transport.receivedPolicies.map(\.maxSourceBytes), [333])
    }

    func testPolicyChangeForcesUnchangedEditorStateToRebuildImageAttachments() {
        let transport = HoldingImageTransport()
        let owner = RenderImageLoadOwner(policy: imagePolicy(maxDecodeDimension: 2_048), transport: transport)
        let editorId = editorCreate(configJson: "{}")
        defer { editorDestroy(id: editorId) }
        _ = editorSetJson(
            id: editorId,
            json: #"{"type":"doc","content":[{"type":"image","attrs":{"src":"https://example.com/policy.png"}}]}"#
        )
        let state = editorGetCurrentState(id: editorId)
        let textView = EditorTextView(frame: .zero, textContainer: nil)
        textView.imageLoadOwner = owner
        textView.applyUpdateJSON(state, notifyDelegate: false)
        XCTAssertEqual(transport.requestCount, 1)

        owner.updatePolicy(imagePolicy(maxDecodeDimension: 512))
        textView.imageLoadingPolicyDidChange()
        textView.applyUpdateJSON(state, notifyDelegate: false)

        XCTAssertEqual(transport.requestCount, 2)
        XCTAssertEqual(transport.receivedPolicies.map(\.maxDecodeDimension), [2_048, 512])
    }

    func testQueuedDeliveryRevalidatesGenerationAfterCancellation() {
        let delivery = ManualImageDeliveryScheduler()
        let transport = ImmediateImageTransport(result: .success(Data([1])))
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(),
            transport: transport,
            decoder: RecordingImageDecoder(image: onePixelImage()),
            deliver: delivery.schedule
        )
        let completion = expectation(description: "stale delivery")
        completion.isInverted = true

        XCTAssertTrue(owner.loadImage(source: "https://example.com/stale.png") { _ in
            completion.fulfill()
        })
        XCTAssertTrue(delivery.waitUntilScheduled(timeout: 1))
        owner.cancelAll()
        delivery.runAll()

        wait(for: [completion], timeout: 0.05)
    }

    func testCompletionCanSynchronouslyWaitForCrossThreadCancellationOperations() {
        enum Operation: CaseIterable { case cancelAll, updatePolicy, cancelReceipt }

        for operation in Operation.allCases {
            let delivery = ManualImageDeliveryScheduler()
            var owner: RenderImageLoadOwner!
            var receipt: RenderImageLoadOwner.ImageLoadReceipt?
            owner = RenderImageLoadOwner(
                policy: imagePolicy(),
                transport: ImmediateImageTransport(result: .success(Data([1]))),
                decoder: RecordingImageDecoder(image: onePixelImage()),
                deliver: delivery.schedule
            )
            receipt = owner.startImageLoad(source: "https://example.com/interleaving-\(operation).png") { _ in
                let finished = DispatchSemaphore(value: 0)
                DispatchQueue.global(qos: .userInitiated).async {
                    switch operation {
                    case .cancelAll: owner.cancelAll()
                    case .updatePolicy: owner.updatePolicy(imagePolicy(maxDecodeDimension: 512))
                    case .cancelReceipt: receipt?.cancel()
                    }
                    finished.signal()
                }
                XCTAssertEqual(finished.wait(timeout: .now() + 0.5), .success)
            }
            XCTAssertTrue(delivery.waitUntilScheduled(timeout: 1))
            delivery.runAll()
        }
    }

    func testCancelledDecodeStillOccupiesConcurrencyUntilClosureExits() {
        let decoder = BlockingImageDecoder(image: onePixelImage())
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(maxConcurrentRequests: 1, maxPendingRequests: 4),
            transport: HoldingImageTransport(),
            decoder: decoder
        )

        XCTAssertTrue(owner.loadImage(source: "data:image/png;base64,AQ==") { _ in })
        XCTAssertTrue(decoder.waitForDecodeCount(1, timeout: 1))
        owner.cancelAll()
        XCTAssertTrue(owner.loadImage(source: "data:image/png;base64,Ag==") { _ in })

        XCTAssertEqual(decoder.decodeCount, 1)
        XCTAssertEqual(decoder.maximumConcurrentDecodes, 1)
        decoder.releaseNext()
        XCTAssertTrue(decoder.waitForDecodeCount(2, timeout: 1))
        XCTAssertEqual(decoder.maximumConcurrentDecodes, 1)
        decoder.releaseNext()
    }

    func testDataURLSizeScanAvoidsAllocationAndOverflowArithmetic() {
        XCTAssertEqual(
            RenderImageLoadOwner.decodedDataURLByteCount(
                "data:image/png;base64, A Q I D \n",
                maxBytes: 3
            ),
            3
        )
        XCTAssertNil(
            RenderImageLoadOwner.decodedDataURLByteCount(
                "data:image/png;base64,AQIDBA==",
                maxBytes: 3
            )
        )
        XCTAssertNil(
            RenderImageLoadOwner.decodedDataURLByteCount(
                "data:image/png;base64,AQ==",
                maxBytes: -1
            )
        )
        XCTAssertNil(
            RenderImageLoadOwner.decodedDataURLByteCount(
                "data:image/png;base64," + String(repeating: " ", count: 5_000) + "AQ==",
                maxBytes: 1
            )
        )
    }

    func testDataURLHeaderScanStopsAtFixedLimitBeforeCommaOrLeadingTrim() {
        let leadingWhitespace = String(repeating: " ", count: 5_000)
            + "data:image/png;base64,AQ=="
        let missingComma = "data:image/png;base64" + String(repeating: "A", count: 5_000)

        let whitespaceScan = RenderImageLoadOwner.scanDataURLHeader(leadingWhitespace)
        let missingCommaScan = RenderImageLoadOwner.scanDataURLHeader(missingComma)

        XCTAssertTrue(whitespaceScan.exceededLimit)
        XCTAssertLessThanOrEqual(whitespaceScan.scannedByteCount, 257)
        XCTAssertTrue(missingCommaScan.exceededLimit)
        XCTAssertLessThanOrEqual(missingCommaScan.scannedByteCount, 257)
    }

    func testDataURLHeaderScanDetectsCommaBeforeUnboundedCombiningGrapheme() {
        let adversarial = "data:image/png;base64,"
            + String(repeating: "\u{0301}\u{200D}", count: 2_000)
            + "AQ=="
        let prefix = "data:image/png;base64"
        let boundaryHeader = prefix
            + String(repeating: "A", count: 256 - prefix.utf8.count)
            + ",AQ=="

        let adversarialScan = RenderImageLoadOwner.scanDataURLHeader(adversarial)
        let boundaryScan = RenderImageLoadOwner.scanDataURLHeader(boundaryHeader)

        XCTAssertNotNil(adversarialScan.commaIndex)
        XCTAssertFalse(adversarialScan.exceededLimit)
        XCTAssertEqual(adversarialScan.scannedByteCount, "data:image/png;base64,".utf8.count)
        XCTAssertNotNil(boundaryScan.commaIndex)
        XCTAssertFalse(boundaryScan.exceededLimit)
        XCTAssertEqual(boundaryScan.scannedByteCount, 257)
    }

    func testDataURLDecodeRunsOffMainAndDeliversOnMain() {
        let decoder = RecordingImageDecoder(image: onePixelImage())
        let owner = RenderImageLoadOwner(
            policy: imagePolicy(),
            transport: HoldingImageTransport(),
            decoder: decoder
        )
        let completion = expectation(description: "decoded")

        XCTAssertTrue(owner.loadImage(source: "data:image/png;base64,AQID") { image in
            XCTAssertTrue(Thread.isMainThread)
            XCTAssertNotNil(image)
            completion.fulfill()
        })

        wait(for: [completion], timeout: 1)
        XCTAssertEqual(decoder.calledOnMainThread, false)
    }

    func testMeasureHeightDoesNotStartImageLoads() {
        let transport = HoldingImageTransport()
        let owner = RenderImageLoadOwner(policy: imagePolicy(), transport: transport)
        let json = """
        [{"type":"voidBlock","nodeType":"image","docPos":1,"attrs":{"src":"https://example.com/image.png"}}]
        """

        _ = owner.withCurrent {
            RenderBridge.measureHeight(forRenderJSON: json, themeJSON: nil, width: 320)
        }

        XCTAssertEqual(transport.requestCount, 0)
    }

    func testEditorAndViewerApplyImageLoadingPolicyJson() {
        let json = """
        {"maxSourceBytes":1234,"connectTimeoutMs":2500,"readTimeoutMs":3500,"maxConcurrentRequests":4,"maxPendingRequests":8,"maxDecodeDimensionPx":640}
        """
        let editor = NativeEditorExpoView()
        let viewer = NativeProseViewerExpoView()

        editor.setImageLoadingPolicyJson(json)
        viewer.setImageLoadingPolicyJson(json)

        XCTAssertEqual(editor.imageLoadingPolicy.maxSourceBytes, 1234)
        XCTAssertEqual(viewer.imageLoadingPolicy.maxDecodeDimension, 640)
    }

    func testViewerEmptyCollapseDetectsDocumentsWithOnlyEmptyTopLevelParagraphs() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "\\u200B", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "", "marks": []},
            {"type": "blockEnd"}
        ]
        """

        XCTAssertTrue(NativeProseViewerExpoView.renderJsonContainsOnlyEmptyParagraphs(json))
    }

    func testViewerEmptyCollapseKeepsVisibleRenderedContentMeasurable() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Hello", "marks": []},
            {"type": "blockEnd"}
        ]
        """

        XCTAssertFalse(NativeProseViewerExpoView.renderJsonContainsOnlyEmptyParagraphs(json))
    }

    func testViewerEmptyCollapseKeepsNonParagraphRenderedBlocksMeasurable() {
        let json = """
        [
            {"type": "voidBlock", "nodeType": "image", "docPos": 1, "attrs": {}}
        ]
        """

        XCTAssertFalse(NativeProseViewerExpoView.renderJsonContainsOnlyEmptyParagraphs(json))
    }

    // MARK: - Plain Text Rendering

    /// A single paragraph with unstyled text should produce the text with base font.
    func testRender_plainParagraph() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Hello, world!", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "Hello, world!",
            "Plain paragraph should render as the text content"
        )

        // Verify the font is the base font.
        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertNotNil(font, "Should have a font attribute")
        XCTAssertEqual(
            font?.pointSize, baseFont.pointSize,
            "Font size should match base font"
        )
    }

    func testRender_leadingTopLevelChildMetadataCoversWholeEmoji() {
        let blocks: [[[String: Any]]] = [[
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
            ["type": "textRun", "text": "😀", "marks": []],
            ["type": "blockEnd"],
        ]]

        let result = RenderBridge.renderBlocks(
            fromArray: blocks,
            baseFont: baseFont,
            textColor: textColor
        )
        let firstComposedRange = (result.string as NSString)
            .rangeOfComposedCharacterSequence(at: 0)
        var effectiveRange = NSRange(location: NSNotFound, length: 0)
        let value = result.attribute(
            RenderBridgeAttributes.topLevelChildIndex,
            at: 0,
            longestEffectiveRange: &effectiveRange,
            in: NSRange(location: 0, length: result.length)
        ) as? NSNumber

        XCTAssertEqual(result.string, "😀")
        XCTAssertGreaterThan(firstComposedRange.length, 1, "test must cover a surrogate-pair emoji")
        XCTAssertEqual(value?.intValue, 0)
        XCTAssertEqual(
            effectiveRange,
            firstComposedRange,
            "top-level metadata must not split a leading emoji surrogate pair into separate attribute runs"
        )
    }

    // MARK: - Bold Text Rendering

    /// Bold mark should produce a font with the bold trait.
    func testRender_boldText() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "bold text", "marks": ["bold"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "bold text")

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertNotNil(font, "Should have a font attribute")
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "Font should have bold trait. Got font: \(String(describing: font))"
        )
    }

    // MARK: - Italic Text Rendering

    func testRender_italicText() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "italic text", "marks": ["italic"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "italic text")

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertNotNil(font, "Should have a font attribute")
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitItalic) ?? false,
            "Font should have italic trait. Got font: \(String(describing: font))"
        )
    }

    // MARK: - Bold + Italic Combined

    func testRender_boldItalic() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "bold italic", "marks": ["bold", "italic"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertNotNil(font, "Should have a font attribute")

        let traits = font?.fontDescriptor.symbolicTraits ?? []
        XCTAssertTrue(
            traits.contains(.traitBold),
            "Font should have bold trait. Traits: \(traits)"
        )
        XCTAssertTrue(
            traits.contains(.traitItalic),
            "Font should have italic trait. Traits: \(traits)"
        )
    }

    // MARK: - Underline

    func testRender_underline() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "underlined", "marks": ["underline"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let underline = attrs[.underlineStyle] as? Int
        XCTAssertNotNil(underline, "Should have underline style attribute")
        XCTAssertEqual(
            underline, NSUnderlineStyle.single.rawValue,
            "Underline should be single. Got: \(String(describing: underline))"
        )
    }

    // MARK: - Strikethrough

    func testRender_strikethrough() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "struck", "marks": ["strike"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let strikethrough = attrs[.strikethroughStyle] as? Int
        XCTAssertNotNil(strikethrough, "Should have strikethrough style attribute")
        XCTAssertEqual(
            strikethrough, NSUnderlineStyle.single.rawValue,
            "Strikethrough should be single. Got: \(String(describing: strikethrough))"
        )
    }

    func testRender_linkMarkObjectAppliesVisualLinkStylingWithoutInteractiveAttribute() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "OpenAI", "marks": [{"type": "link", "href": "https://openai.com"}]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        XCTAssertEqual(
            attrs[.underlineStyle] as? Int,
            NSUnderlineStyle.single.rawValue
        )
        XCTAssertEqual(attrs[.foregroundColor] as? UIColor, UIColor.systemBlue)
        XCTAssertNil(attrs[.link])
        XCTAssertEqual(
            attrs[RenderBridgeAttributes.linkHref] as? String,
            "https://openai.com"
        )
    }

    func testRender_linkMarkUsesThemeOverrides() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "OpenAI", "marks": [{"type": "link", "href": "https://openai.com"}]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: EditorTheme(dictionary: [
                "links": [
                    "color": "#445566",
                    "backgroundColor": "#eef6ff",
                    "fontSize": 18,
                    "fontWeight": "700",
                    "fontStyle": "italic",
                    "underline": false,
                ],
            ])
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertEqual(attrs[.foregroundColor] as? UIColor, EditorTheme.color(from: "#445566"))
        XCTAssertEqual(attrs[.backgroundColor] as? UIColor, EditorTheme.color(from: "#eef6ff"))
        XCTAssertNil(attrs[.underlineStyle])
        XCTAssertEqual(font?.pointSize, 18)
        XCTAssertTrue(font?.fontDescriptor.symbolicTraits.contains(.traitBold) == true)
        XCTAssertTrue(font?.fontDescriptor.symbolicTraits.contains(.traitItalic) == true)
        XCTAssertEqual(
            attrs[RenderBridgeAttributes.linkHref] as? String,
            "https://openai.com"
        )
    }

    func testRenderBlocks_withLeadingSeparatorDoesNotDuplicateTopLevelChildIndexOnContent() {
        let blocks: [[[String: Any]]] = [[
            ["type": "blockStart", "nodeType": "paragraph", "depth": 0],
            ["type": "textRun", "text": "Hello", "marks": []],
            ["type": "blockEnd"],
        ]]

        let result = RenderBridge.renderBlocks(
            fromArray: blocks,
            startIndex: 3,
            includeLeadingInterBlockSeparator: true,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "\nHello")
        XCTAssertEqual(
            (result.attribute(RenderBridgeAttributes.topLevelChildIndex, at: 0, effectiveRange: nil)
                as? NSNumber)?.intValue,
            3
        )
        XCTAssertNil(
            result.attribute(RenderBridgeAttributes.topLevelChildIndex, at: 1, effectiveRange: nil),
            "Leading content should not duplicate the separator's top-level child index"
        )
    }

    // MARK: - Code Mark (Monospace)

    func testRender_codeInline() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "code", "marks": ["code"]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        XCTAssertNotNil(font, "Should have a font attribute")
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitMonoSpace) ?? false,
            "Code mark should produce monospace font. Got font: \(String(describing: font))"
        )
    }

    // MARK: - Code Block

    /// A code block with no marks and no theme override must still render as
    /// regular-weight monospace (baseline behavior, must not regress).
    func testRender_codeBlock_plainTextIsRegularMonospace() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "let x", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(fromJSON: json, baseFont: baseFont, textColor: textColor)

        let font = result.attributes(at: 0, effectiveRange: nil)[.font] as? UIFont
        XCTAssertNotNil(font)
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitMonoSpace)
                || font!.fontName.lowercased().contains("mono"),
            "Plain code block text should be monospaced. Got font: \(font!.fontName)"
        )
        XCTAssertFalse(
            font!.fontDescriptor.symbolicTraits.contains(.traitBold),
            "Plain code block text must not be bold. Got font: \(font!.fontName)"
        )
    }

    /// Bold marks inside a code block must survive the monospace substitution
    /// (parity with Android, which layers StyleSpan(BOLD) over the monospace
    /// typeface).
    func testRender_codeBlock_preservesBoldTrait() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "let x", "marks": [{"type": "bold"}]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(fromJSON: json, baseFont: baseFont, textColor: textColor)

        let font = result.attributes(at: 0, effectiveRange: nil)[.font] as? UIFont
        XCTAssertNotNil(font)
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitBold),
            "Bold trait must survive in code blocks; got \(font!.fontName)"
        )
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitMonoSpace)
                || font!.fontName.lowercased().contains("mono"),
            "Code block text should still be monospaced"
        )
    }

    /// Combined bold+italic marks inside a code block must not silently lose
    /// BOTH traits when the mono family lacks a bold-italic face. Bold must
    /// always survive; italic survives whenever the resolved face supports
    /// layering it on top of bold. This uses the system-default mono
    /// substitution path (no theme font family override) so both traits are
    /// expected to survive deterministically.
    func testRender_codeBlock_preservesBoldAndItalicTraits() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "let x", "marks": [{"type": "bold"}, {"type": "italic"}]},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(fromJSON: json, baseFont: baseFont, textColor: textColor)

        let font = result.attributes(at: 0, effectiveRange: nil)[.font] as? UIFont
        XCTAssertNotNil(font)
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitBold),
            "Bold trait must always survive in code blocks, even combined with italic; got \(font!.fontName)"
        )
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitItalic),
            "Italic trait should survive alongside bold on the system-default monospace path; got \(font!.fontName)"
        )
        XCTAssertTrue(
            font!.fontDescriptor.symbolicTraits.contains(.traitMonoSpace)
                || font!.fontName.lowercased().contains("mono"),
            "Code block text should still be monospaced"
        )
    }

    /// Two adjacent code blocks must produce two separate background groups —
    /// the separator newline between blocks carries no codeBlockBackgroundColor.
    func testCodeBlockGrouping_adjacentBlocksAreSeparate() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "first", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "second", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let rendered = RenderBridge.renderElements(fromJSON: json, baseFont: baseFont, textColor: textColor)
        let storage = NSTextStorage(attributedString: rendered)
        let nsString = storage.string as NSString
        // "first\nsecond" — paragraphStart of "second" is 6.
        let group = EditorLayoutManager.codeBlockCharacterRange(
            containing: 6, in: storage, nsString: nsString
        )
        XCTAssertEqual(group.location, 6, "Group must not absorb the preceding block")
        // And the first block's group must stop before the separator:
        let firstGroup = EditorLayoutManager.codeBlockCharacterRange(
            containing: 0, in: storage, nsString: nsString
        )
        XCTAssertEqual(NSMaxRange(firstGroup), 6, "Group may include its own separator paragraph end at most")
        XCTAssertNotEqual(firstGroup, group)
    }

    /// theme.codeBlock.text.fontFamily must not be silently replaced by the
    /// system monospace font.
    func testRender_codeBlock_honorsThemeFontFamily() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "let x", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "codeBlock": [
                "text": [
                    "fontFamily": "Courier New",
                ],
            ],
        ])
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let font = result.attributes(at: 0, effectiveRange: nil)[.font] as? UIFont
        XCTAssertNotNil(font)
        XCTAssertEqual(
            font!.familyName,
            "Courier New",
            "Themed codeBlock.text.fontFamily should be preserved, not overwritten by the system monospace font. Got: \(font!.familyName)"
        )
    }

    // MARK: - Hard Break (Void Inline)

    /// A hardBreak void inline should render as a newline character.
    func testRender_hardBreak() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Line 1", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 7},
            {"type": "textRun", "text": "Line 2", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "Line 1\nLine 2",
            "Hard break should render as newline. Got: '\(result.string)'"
        )

        // Verify the newline character has the void attribute.
        let newlineIndex = 6  // "Line 1" = 6 chars, newline at index 6
        let attrs = result.attributes(at: newlineIndex, effectiveRange: nil)
        let voidType = attrs[RenderBridgeAttributes.voidNodeType] as? String
        XCTAssertEqual(
            voidType, "hardBreak",
            "Newline should have voidNodeType='hardBreak' attribute. Got: \(String(describing: voidType))"
        )
        let docPos = attrs[RenderBridgeAttributes.docPos] as? UInt32
        XCTAssertEqual(
            docPos, 7,
            "Newline should have docPos=7. Got: \(String(describing: docPos))"
        )
    }

    func testRender_hardBreakDoesNotKeepParagraphSpacingBetweenVisualLines() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Line 1", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 7},
            {"type": "textRun", "text": "Line 2", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "spacingAfter": 14,
            ],
        ])
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let leadingStyle = result.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle
        let newlineStyle = result.attribute(.paragraphStyle, at: 6, effectiveRange: nil) as? NSParagraphStyle

        XCTAssertEqual(leadingStyle?.paragraphSpacing ?? -1, 0, accuracy: 0.1)
        XCTAssertEqual(newlineStyle?.paragraphSpacing ?? -1, 0, accuracy: 0.1)
    }

    func testRender_trailingHardBreakAppendsSyntheticPlaceholder() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 2},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "A\n\u{200B}")
        let placeholderIndex = (result.string as NSString).length - 1
        XCTAssertEqual(
            result.attribute(RenderBridgeAttributes.syntheticPlaceholder, at: placeholderIndex, effectiveRange: nil) as? Bool,
            true
        )
    }

    func testRender_trailingHardBreakPlaceholderKeepsBlockquoteBorderAttributes() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 2},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "A\n\u{200B}")

        let placeholderIndex = (result.string as NSString).length - 1
        XCTAssertEqual(
            result.attribute(RenderBridgeAttributes.syntheticPlaceholder, at: placeholderIndex, effectiveRange: nil) as? Bool,
            true
        )
        XCTAssertNotNil(
            result.attribute(RenderBridgeAttributes.blockquoteBorderColor, at: placeholderIndex, effectiveRange: nil),
            "trailing hard-break placeholder inside a blockquote should keep blockquote styling"
        )
    }

    // MARK: - Horizontal Rule (Void Block)

    /// A horizontalRule should render as U+FFFC with an NSTextAttachment.
    func testRender_horizontalRule() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Above", "marks": []},
            {"type": "blockEnd"},
            {"type": "voidBlock", "nodeType": "horizontalRule", "docPos": 7},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Below", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        // The expected structure is: "Above" + "\n" + U+FFFC + "\n" + "Below"
        // The newlines are inter-block separators.
        let string = result.string
        XCTAssertTrue(
            string.contains("\u{FFFC}"),
            "Horizontal rule should contain object replacement character. Got: '\(string)'"
        )

        // Find the FFFC character and check its attributes.
        if let fffcRange = string.range(of: "\u{FFFC}") {
            let nsRange = NSRange(fffcRange, in: string)
            let attrs = result.attributes(at: nsRange.location, effectiveRange: nil)

            let voidType = attrs[RenderBridgeAttributes.voidNodeType] as? String
            XCTAssertEqual(
                voidType, "horizontalRule",
                "FFFC should have voidNodeType='horizontalRule'. Got: \(String(describing: voidType))"
            )

            let attachment = attrs[.attachment] as? NSTextAttachment
            XCTAssertNotNil(
                attachment,
                "FFFC should have an NSTextAttachment"
            )
            XCTAssertTrue(
                attachment is HorizontalRuleAttachment,
                "Attachment should be HorizontalRuleAttachment. Got: \(String(describing: type(of: attachment)))"
            )
        } else {
            XCTFail("Could not find FFFC character in rendered string")
        }
    }

    func testRender_horizontalRuleCollapsesAdjacentParagraphSpacing() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Above", "marks": []},
            {"type": "blockEnd"},
            {"type": "voidBlock", "nodeType": "horizontalRule", "docPos": 7},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Below", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "spacingAfter": 14,
            ],
            "horizontalRule": [
                "verticalMargin": 10,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let nsString = result.string as NSString
        let aboveRange = nsString.range(of: "Above")
        let hrRange = nsString.range(of: "\u{FFFC}")
        guard aboveRange.location != NSNotFound, hrRange.location != NSNotFound else {
            XCTFail("expected both paragraph text and horizontal rule in rendered output")
            return
        }

        let aboveParagraphStyle = result.attribute(.paragraphStyle, at: aboveRange.location, effectiveRange: nil)
            as? NSParagraphStyle
        let separatorParagraphStyle = result.attribute(
            .paragraphStyle,
            at: hrRange.location + hrRange.length,
            effectiveRange: nil
        ) as? NSParagraphStyle
        let attachment = result.attribute(.attachment, at: hrRange.location, effectiveRange: nil)
            as? HorizontalRuleAttachment

        XCTAssertEqual(attachment?.verticalPadding ?? 0, 10, accuracy: 0.1)
        XCTAssertEqual(aboveParagraphStyle?.paragraphSpacing ?? -1, 4, accuracy: 0.1)
        XCTAssertEqual(separatorParagraphStyle?.paragraphSpacing ?? -1, 4, accuracy: 0.1)
    }

    // MARK: - Multiple Paragraphs

    /// Two consecutive paragraphs should be separated by a newline.
    func testRender_multipleParagraphs() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "First", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Second", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "First\nSecond",
            "Two paragraphs should be separated by a newline"
        )
    }

    // MARK: - Mixed Marks in Same Paragraph

    /// A paragraph with mixed styled runs should produce the correct combined string
    /// with different attributes at different ranges.
    func testRender_mixedMarksInParagraph() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "normal ", "marks": []},
            {"type": "textRun", "text": "bold", "marks": ["bold"]},
            {"type": "textRun", "text": " end", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "normal bold end")

        // Check "normal " (offset 0) has base font, not bold.
        let normalAttrs = result.attributes(at: 0, effectiveRange: nil)
        let normalFont = normalAttrs[.font] as? UIFont
        XCTAssertFalse(
            normalFont?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? true,
            "'normal' should not be bold"
        )

        // Check "bold" (offset 7) has bold font.
        let boldAttrs = result.attributes(at: 7, effectiveRange: nil)
        let boldFont = boldAttrs[.font] as? UIFont
        XCTAssertTrue(
            boldFont?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "'bold' should have bold trait"
        )

        // Check " end" (offset 11) has base font, not bold.
        let endAttrs = result.attributes(at: 11, effectiveRange: nil)
        let endFont = endAttrs[.font] as? UIFont
        XCTAssertFalse(
            endFont?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? true,
            "'end' should not be bold"
        )
    }

    // MARK: - Ordered List

    /// Ordered list items should reserve gutter space without injecting marker text.
    func testRender_orderedListItem() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "First item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 2, "total": 2, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Second item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "First item\nSecond item")

        let firstAttrs = result.attributes(at: 0, effectiveRange: nil)
        let firstStyle = firstAttrs[.paragraphStyle] as? NSParagraphStyle
        XCTAssertNotNil(firstAttrs[RenderBridgeAttributes.listContext])
        XCTAssertEqual(firstStyle?.firstLineHeadIndent, 48.0 + LayoutConstants.listMarkerWidth)
        XCTAssertEqual(firstStyle?.headIndent, 48.0 + LayoutConstants.listMarkerWidth)
    }

    // MARK: - Unordered List

    func testRender_unorderedListItem() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Bullet item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "Bullet item")
        XCTAssertNotNil(result.attribute(RenderBridgeAttributes.listContext, at: 0, effectiveRange: nil))
    }

    func testRender_unorderedListMarkerUsesLargerFontThanItemText() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Bullet item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let textFont = result.attribute(.font, at: 0, effectiveRange: nil) as? UIFont
        XCTAssertEqual(textFont?.pointSize, baseFont.pointSize)
        XCTAssertNotNil(result.attribute(RenderBridgeAttributes.listContext, at: 0, effectiveRange: nil))
    }

    func testRender_emptyUnorderedListItemDoesNotInsertParagraphNewlineAfterMarker() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "\\u200B", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "\u{200B}",
            "An empty list item should render only its placeholder text. Got: '\(result.string)'"
        )
        XCTAssertNotNil(result.attribute(RenderBridgeAttributes.listContext, at: 0, effectiveRange: nil))
    }

    func testRender_emptyParagraphAfterListUsesItsOwnParagraphStyle() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "\\u200B", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "A\n\u{200B}")

        let placeholderIndex = (result.string as NSString).length - 1
        let placeholderStyle = result.attribute(
            .paragraphStyle,
            at: placeholderIndex,
            effectiveRange: nil
        ) as? NSParagraphStyle

        XCTAssertNotNil(placeholderStyle, "Empty paragraph placeholder should carry paragraph style")
        XCTAssertEqual(placeholderStyle?.firstLineHeadIndent, 0)
        XCTAssertEqual(placeholderStyle?.headIndent, 0)
    }

    func testRender_secondParagraphInListItemDoesNotGetListMarkerContext() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "\\u200B", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertNotNil(
            result.attribute(RenderBridgeAttributes.listMarkerContext, at: 0, effectiveRange: nil),
            "The first paragraph in a list item should keep its marker context"
        )
        XCTAssertNil(
            result.attribute(RenderBridgeAttributes.listMarkerContext, at: 2, effectiveRange: nil),
            "The second paragraph in a list item should not render a separate list marker"
        )
    }

    // MARK: - Invalid JSON

    func testRender_invalidJSON() {
        let result = RenderBridge.renderElements(
            fromJSON: "not valid json",
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "",
            "Invalid JSON should produce empty attributed string"
        )
    }

    func testRender_emptyArray() {
        let result = RenderBridge.renderElements(
            fromJSON: "[]",
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(
            result.string, "",
            "Empty array should produce empty attributed string"
        )
    }

    // MARK: - Mark Attributes Isolated Tests

    /// Test attributesForMarks directly to verify all mark combinations.
    func testAttributesForMarks_noMarks() {
        let attrs = RenderBridge.attributesForMarks([], baseFont: baseFont, textColor: textColor)
        let font = attrs[.font] as? UIFont
        XCTAssertEqual(font, baseFont, "No marks should use base font")
        XCTAssertNil(attrs[.underlineStyle], "No marks should have no underline")
        XCTAssertNil(attrs[.strikethroughStyle], "No marks should have no strikethrough")
    }

    func testAttributesForMarks_strongAlias() {
        // "strong" is an alias for "bold"
        let attrs = RenderBridge.attributesForMarks(
            ["strong"],
            baseFont: baseFont,
            textColor: textColor
        )
        let font = attrs[.font] as? UIFont
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "'strong' should produce bold font"
        )
    }

    func testAttributesForMarks_emAlias() {
        // "em" is an alias for "italic"
        let attrs = RenderBridge.attributesForMarks(
            ["em"],
            baseFont: baseFont,
            textColor: textColor
        )
        let font = attrs[.font] as? UIFont
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitItalic) ?? false,
            "'em' should produce italic font"
        )
    }

    func testAttributesForMarks_strikethroughAlias() {
        // "strikethrough" is an alias for "strike"
        let attrs = RenderBridge.attributesForMarks(
            ["strikethrough"],
            baseFont: baseFont,
            textColor: textColor
        )
        let strikethrough = attrs[.strikethroughStyle] as? Int
        XCTAssertEqual(
            strikethrough, NSUnderlineStyle.single.rawValue,
            "'strikethrough' should produce strikethrough style"
        )
    }

    func testAttributesForMarks_allCombined() {
        let attrs = RenderBridge.attributesForMarks(
            ["bold", "italic", "underline", "strike"],
            baseFont: baseFont,
            textColor: textColor
        )
        let font = attrs[.font] as? UIFont
        let traits = font?.fontDescriptor.symbolicTraits ?? []
        XCTAssertTrue(traits.contains(.traitBold), "Should have bold")
        XCTAssertTrue(traits.contains(.traitItalic), "Should have italic")
        XCTAssertEqual(
            attrs[.underlineStyle] as? Int,
            NSUnderlineStyle.single.rawValue,
            "Should have underline"
        )
        XCTAssertEqual(
            attrs[.strikethroughStyle] as? Int,
            NSUnderlineStyle.single.rawValue,
            "Should have strikethrough"
        )
    }

    func testAttributesForMarks_unknownMarkIgnored() {
        let attrs = RenderBridge.attributesForMarks(
            ["unknownMark"],
            baseFont: baseFont,
            textColor: textColor
        )
        let font = attrs[.font] as? UIFont
        XCTAssertEqual(
            font, baseFont,
            "Unknown marks should be ignored, producing base font"
        )
    }

    // MARK: - Paragraph Style Tests

    func testParagraphStyle_depth0() {
        let ctx = BlockContext(nodeType: "paragraph", depth: 0, listContext: nil)
        let style = RenderBridge.paragraphStyleForBlock(ctx, blockStack: [ctx])
        XCTAssertEqual(
            style.firstLineHeadIndent, 0,
            "Depth 0 paragraph should have 0 indentation"
        )
        XCTAssertEqual(
            style.headIndent, 0,
            "Depth 0 paragraph should have 0 head indent"
        )
    }

    func testParagraphStyle_depth2() {
        let ctx = BlockContext(nodeType: "paragraph", depth: 2, listContext: nil)
        let style = RenderBridge.paragraphStyleForBlock(ctx, blockStack: [ctx])
        let expectedIndent: CGFloat = 2 * 24.0  // 2 * indentPerDepth
        XCTAssertEqual(
            style.firstLineHeadIndent, expectedIndent,
            "Depth 2 paragraph should have \(expectedIndent) first line indent"
        )
    }

    func testParagraphStyle_listItem() {
        let listCtx: [String: Any] = [
            "ordered": true,
            "index": 1,
            "total": 3,
            "start": 1,
            "isFirst": true,
            "isLast": false,
        ]
        let ctx = BlockContext(nodeType: "listItem", depth: 1, listContext: listCtx)
        let style = RenderBridge.paragraphStyleForBlock(ctx, blockStack: [ctx])

        let baseIndent: CGFloat = 1 * 24.0  // depth * indentPerDepth
        XCTAssertEqual(
            style.firstLineHeadIndent, baseIndent + LayoutConstants.listMarkerWidth,
            "List item first line indent should reserve marker width"
        )
        XCTAssertEqual(
            style.headIndent, baseIndent + LayoutConstants.listMarkerWidth,
            "List item head indent should include marker width"
        )
    }

    func testParagraphStyle_listBaseIndentMultiplierCanCollapseTopLevelIndent() {
        let listCtx: [String: Any] = [
            "ordered": false,
            "index": 1,
            "total": 1,
            "start": 1,
            "isFirst": true,
            "isLast": true,
        ]
        let topLevelCtx = BlockContext(nodeType: "paragraph", depth: 1, listContext: listCtx)
        let nestedCtx = BlockContext(nodeType: "paragraph", depth: 2, listContext: listCtx)
        let theme = EditorTheme(dictionary: [
            "list": [
                "indent": 24,
                "baseIndentMultiplier": 0,
            ],
        ])

        let topLevelStyle = RenderBridge.paragraphStyleForBlock(
            topLevelCtx,
            blockStack: [topLevelCtx],
            theme: theme,
            baseFont: baseFont
        )
        let nestedStyle = RenderBridge.paragraphStyleForBlock(
            nestedCtx,
            blockStack: [nestedCtx],
            theme: theme,
            baseFont: baseFont
        )

        XCTAssertEqual(
            topLevelStyle.firstLineHeadIndent,
            LayoutConstants.listMarkerWidth,
            accuracy: 0.1,
            "Top-level list items should be flush-left apart from the marker gutter"
        )
        XCTAssertEqual(
            topLevelStyle.headIndent,
            LayoutConstants.listMarkerWidth,
            accuracy: 0.1,
            "Wrapped lines should align with the marker gutter when the base indent multiplier is zero"
        )
        XCTAssertEqual(
            nestedStyle.headIndent - topLevelStyle.headIndent,
            24,
            accuracy: 0.1,
            "Nested list levels should still add one indent unit each"
        )
    }

    func testParagraphStyle_unorderedMarkerScaleDoesNotWidenTextGutter() {
        let baseContext = BlockContext(
            nodeType: "listItem",
            depth: 1,
            listContext: [
                "ordered": false,
                "index": 1,
                "total": 1,
                "start": 1,
                "isFirst": true,
                "isLast": true,
            ]
        )
        let baseTheme = EditorTheme(dictionary: [
            "list": [
                "indent": 24,
                "markerScale": 1,
            ],
        ])
        let scaledTheme = EditorTheme(dictionary: [
            "list": [
                "indent": 24,
                "markerScale": 2,
            ],
        ])

        let largeBaseFont = UIFont.systemFont(ofSize: 40)
        let baseStyle = RenderBridge.paragraphStyleForBlock(
            baseContext,
            blockStack: [baseContext],
            theme: baseTheme,
            baseFont: largeBaseFont
        )
        let scaledStyle = RenderBridge.paragraphStyleForBlock(
            baseContext,
            blockStack: [baseContext],
            theme: scaledTheme,
            baseFont: largeBaseFont
        )

        XCTAssertEqual(baseStyle.headIndent, scaledStyle.headIndent, accuracy: 0.1)
        XCTAssertEqual(baseStyle.firstLineHeadIndent, scaledStyle.firstLineHeadIndent, accuracy: 0.1)
    }

    func testParagraphStyle_blockquoteUsesQuoteIndent() {
        let quote = BlockContext(nodeType: "blockquote", depth: 0, listContext: nil)
        let paragraph = BlockContext(nodeType: "paragraph", depth: 1, listContext: nil)
        let theme = EditorTheme(dictionary: [
            "blockquote": [
                "indent": 20,
                "borderColor": "#aa5500",
                "borderWidth": 4,
                "markerGap": 10,
            ],
        ])

        let style = RenderBridge.paragraphStyleForBlock(
            paragraph,
            blockStack: [quote, paragraph],
            theme: theme,
            baseFont: baseFont
        )

        XCTAssertEqual(style.firstLineHeadIndent, 20, accuracy: 0.1)
        XCTAssertEqual(style.headIndent, 20, accuracy: 0.1)
    }

    func testParagraphStyle_nestedListItemInsideBlockquoteAddsListIndent() {
        let quote = BlockContext(nodeType: "blockquote", depth: 0, listContext: nil)
        let parentListItem = BlockContext(
            nodeType: "listItem",
            depth: 1,
            listContext: ["ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false]
        )
        let parentParagraph = BlockContext(nodeType: "paragraph", depth: 2, listContext: nil)
        let nestedListItem = BlockContext(
            nodeType: "listItem",
            depth: 2,
            listContext: ["ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true]
        )
        let nestedParagraph = BlockContext(nodeType: "paragraph", depth: 3, listContext: nil)

        let parentStyle = RenderBridge.paragraphStyleForBlock(
            parentParagraph,
            blockStack: [quote, parentListItem, parentParagraph],
            theme: nil,
            baseFont: baseFont
        )
        let nestedStyle = RenderBridge.paragraphStyleForBlock(
            nestedParagraph,
            blockStack: [quote, parentListItem, nestedListItem, nestedParagraph],
            theme: nil,
            baseFont: baseFont
        )

        XCTAssertGreaterThan(
            nestedStyle.headIndent,
            parentStyle.headIndent,
            "nested list item inside a blockquote should indent more than its parent item"
        )
        XCTAssertGreaterThan(
            nestedStyle.firstLineHeadIndent,
            parentStyle.firstLineHeadIndent,
            "nested list marker should also move inward inside a blockquote"
        )
    }

    func testParagraphStyle_firstLevelListInsideBlockquoteAddsListIndentInsideQuote() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Quoted item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )
        let style = result.attribute(.paragraphStyle, at: 0, effectiveRange: nil) as? NSParagraphStyle
        let quote = BlockContext(nodeType: "blockquote", depth: 0, listContext: nil)
        let quotedParagraph = BlockContext(nodeType: "paragraph", depth: 1, listContext: nil)
        let quotedListParagraph = BlockContext(
            nodeType: "paragraph",
            depth: 2,
            listContext: ["ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true]
        )
        let plainQuotedStyle = RenderBridge.paragraphStyleForBlock(
            quotedParagraph,
            blockStack: [quote, quotedParagraph],
            theme: nil,
            baseFont: baseFont
        )
        let expectedStyle = RenderBridge.paragraphStyleForBlock(
            quotedListParagraph,
            blockStack: [quote, quotedListParagraph],
            theme: nil,
            baseFont: baseFont
        )

        XCTAssertEqual(
            style?.headIndent ?? -1,
            expectedStyle.headIndent,
            accuracy: 0.1,
            "first-level list paragraphs inside a blockquote should keep their extra list indent"
        )
        XCTAssertEqual(
            style?.firstLineHeadIndent ?? -1,
            expectedStyle.firstLineHeadIndent,
            accuracy: 0.1,
            "first-level quoted list markers should keep their extra list indent"
        )
        XCTAssertGreaterThan(
            style?.headIndent ?? -1,
            plainQuotedStyle.headIndent,
            "quoted list text should indent further than plain quoted text"
        )
        XCTAssertGreaterThan(
            style?.firstLineHeadIndent ?? -1,
            plainQuotedStyle.firstLineHeadIndent,
            "quoted list marker gutter should indent further than plain quoted text"
        )
    }

    func testRender_blockquoteAppliesBorderAttributesAndTextTheme() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Quoted", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: EditorTheme(dictionary: [
                "blockquote": [
                    "indent": 20,
                    "borderColor": "#aa5500",
                    "borderWidth": 4,
                    "markerGap": 10,
                    "text": [
                        "color": "#334455",
                    ],
                ],
            ])
        )
        let expectedTextColor = UIColor(
            red: 51.0 / 255.0,
            green: 68.0 / 255.0,
            blue: 85.0 / 255.0,
            alpha: 1
        )
        let expectedBorderColor = UIColor(
            red: 170.0 / 255.0,
            green: 85.0 / 255.0,
            blue: 0.0,
            alpha: 1
        )
        var foundStyledRun = false
        result.enumerateAttributes(
            in: NSRange(location: 0, length: result.length),
            options: []
        ) { attrs, _, stop in
            guard attrs[RenderBridgeAttributes.blockquoteBorderColor] != nil else { return }
            XCTAssertEqual(attrs[.foregroundColor] as? UIColor, expectedTextColor)
            XCTAssertEqual(attrs[RenderBridgeAttributes.blockquoteBorderColor] as? UIColor, expectedBorderColor)
            XCTAssertEqual(
                (attrs[RenderBridgeAttributes.blockquoteBorderWidth] as? NSNumber)?.doubleValue ?? 0,
                4,
                accuracy: 0.1
            )
            XCTAssertEqual(
                (attrs[RenderBridgeAttributes.blockquoteMarkerGap] as? NSNumber)?.doubleValue ?? 0,
                10,
                accuracy: 0.1
            )
            foundStyledRun = true
            stop.pointee = true
        }

        XCTAssertTrue(foundStyledRun, "Expected a rendered run carrying blockquote border attributes")
    }

    func testRender_blockquoteDoesNotInsertExtraLeadingParagraphBreak() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Hello", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "World", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, "Hello\nWorld")
    }

    // MARK: - List Marker Generation

    func testListMarker_ordered() {
        let ctx: [String: Any] = ["ordered": true, "index": 3]
        let marker = RenderBridge.listMarkerString(listContext: ctx)
        XCTAssertEqual(marker, "3. ", "Ordered list item 3 should produce '3. '")
    }

    func testListMarker_unordered() {
        let ctx: [String: Any] = ["ordered": false, "index": 1]
        let marker = RenderBridge.listMarkerString(listContext: ctx)
        XCTAssertEqual(marker, "\u{2022} ", "Unordered list should produce bullet + space")
    }

    // MARK: - Opaque Atoms

    func testRender_opaqueInlineAtom() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "before ", "marks": []},
            {"type": "opaqueInlineAtom", "label": "widget", "docPos": 8},
            {"type": "textRun", "text": " after", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertTrue(
            result.string.contains("[widget]"),
            "Opaque inline atom should render as '[widget]'. Got: '\(result.string)'"
        )
    }

    func testRender_mentionInlineAtomUsesVisibleLabelAndTheme() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Hello ", "marks": []},
            {"type": "opaqueInlineAtom", "nodeType": "mention", "label": "@Alice", "docPos": 7},
            {"type": "textRun", "text": "!", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "mentions": [
                "textColor": "#112233",
                "backgroundColor": "#ddeeff",
                "fontWeight": "bold",
            ],
        ])
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        XCTAssertTrue(
            result.string.contains("@Alice"),
            "Mention inline atom should render its visible label. Got: '\(result.string)'"
        )
        XCTAssertFalse(
            result.string.contains("[@Alice]"),
            "Mention inline atom should not render using generic opaque brackets. Got: '\(result.string)'"
        )

        let mentionRange = (result.string as NSString).range(of: "@Alice")
        XCTAssertNotEqual(mentionRange.location, NSNotFound)

        let attrs = result.attributes(at: mentionRange.location, effectiveRange: nil)
        XCTAssertEqual(
            attrs[.foregroundColor] as? UIColor,
            UIColor(
                red: 0x11 as CGFloat / 255.0,
                green: 0x22 as CGFloat / 255.0,
                blue: 0x33 as CGFloat / 255.0,
                alpha: 1.0
            )
        )
        XCTAssertEqual(
            attrs[.backgroundColor] as? UIColor,
            UIColor(
                red: 0xdd as CGFloat / 255.0,
                green: 0xee as CGFloat / 255.0,
                blue: 0xff as CGFloat / 255.0,
                alpha: 1.0
            )
        )
        let font = attrs[.font] as? UIFont
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "Mention theme should be able to request a bold font"
        )
    }

    func testRender_mentionInlineAtomMergesElementMentionThemeOverride() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {
                "type": "opaqueInlineAtom",
                "nodeType": "mention",
                "label": "@Alice",
                "docPos": 1,
                "mentionTheme": {"textColor": "#445566"}
            },
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "mentions": [
                "textColor": "#112233",
                "backgroundColor": "#ddeeff",
                "fontWeight": "bold",
            ],
        ])
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        XCTAssertEqual(result.string, "@Alice")

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        XCTAssertEqual(
            attrs[.foregroundColor] as? UIColor,
            UIColor(
                red: 0x44 as CGFloat / 255.0,
                green: 0x55 as CGFloat / 255.0,
                blue: 0x66 as CGFloat / 255.0,
                alpha: 1.0
            )
        )
        XCTAssertEqual(
            attrs[.backgroundColor] as? UIColor,
            UIColor(
                red: 0xdd as CGFloat / 255.0,
                green: 0xee as CGFloat / 255.0,
                blue: 0xff as CGFloat / 255.0,
                alpha: 1.0
            )
        )
        let font = attrs[.font] as? UIFont
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "Mention override should preserve global bold styling. Got: \(String(describing: font))"
        )
    }

    func testRender_opaqueBlockAtom() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Above", "marks": []},
            {"type": "blockEnd"},
            {"type": "opaqueBlockAtom", "label": "codeBlock", "docPos": 7}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertTrue(
            result.string.contains("[codeBlock]"),
            "Opaque block atom should render as '[codeBlock]'. Got: '\(result.string)'"
        )
    }

    // MARK: - Theme Rendering

    func testRender_themeOverridesParagraphTypography() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Styled", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "text": [
                "fontFamily": "Courier",
                "fontSize": 18,
                "color": "#112233",
            ],
            "paragraph": [
                "lineHeight": 28,
                "spacingAfter": 14,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        let color = attrs[.foregroundColor] as? UIColor
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(font?.pointSize ?? 0, 18, accuracy: 0.1)
        XCTAssertEqual(color, EditorTheme.color(from: "#112233"))
        XCTAssertEqual(paragraphStyle?.minimumLineHeight ?? 0, 28, accuracy: 0.1)
        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? 0, 14, accuracy: 0.1)
    }

    func testRender_themeOverridesSpecificHeadingLevelTypography() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "h2", "depth": 0},
            {"type": "textRun", "text": "Section title", "marks": []},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "text": [
                "fontSize": 16,
                "color": "#112233",
            ],
            "headings": [
                "h2": [
                    "fontSize": 28,
                    "fontWeight": "700",
                    "color": "#445566",
                    "lineHeight": 34,
                    "spacingAfter": 12,
                ],
                "h4": [
                    "fontSize": 18,
                    "color": "#AA5500",
                ],
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let font = attrs[.font] as? UIFont
        let color = attrs[.foregroundColor] as? UIColor
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(font?.pointSize ?? 0, 28, accuracy: 0.1)
        XCTAssertTrue(
            font?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "Configured h2 heading should resolve to a bold font"
        )
        XCTAssertEqual(color, EditorTheme.color(from: "#445566"))
        XCTAssertEqual(paragraphStyle?.minimumLineHeight ?? 0, 34, accuracy: 0.1)
        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? 0, 12, accuracy: 0.1)
    }

    func testRender_listItemUsesListItemSpacingWhenParagraphSpacingUnset() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "First item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 2, "total": 2, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Second item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "list": [
                "itemSpacing": 14,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? 0, 14, accuracy: 0.1)
    }

    func testRender_listItemSpacingOverridesParagraphSpacingForSiblingListItems() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "First item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 2, "total": 2, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Second item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "spacingAfter": 14,
            ],
            "list": [
                "itemSpacing": 6,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let nsString = result.string as NSString
        let firstRange = nsString.range(of: "First item")
        XCTAssertNotEqual(firstRange.location, NSNotFound)

        let attrs = result.attributes(at: firstRange.location, effectiveRange: nil)
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? -1, 6, accuracy: 0.1)
    }

    func testRender_nestedFirstListItemDoesNotKeepParentParagraphSpacingWhenItemSpacingIsZero() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Parent item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Nested item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "spacingAfter": 14,
            ],
            "list": [
                "itemSpacing": 0,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let nsString = result.string as NSString
        let parentRange = nsString.range(of: "Parent item")
        XCTAssertNotEqual(parentRange.location, NSNotFound)

        let attrs = result.attributes(at: parentRange.location, effectiveRange: nil)
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? -1, 0, accuracy: 0.1)
    }

    func testRender_nestedSiblingListItemsUseListItemSpacingInsteadOfParagraphSpacing() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Parent item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Child one", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 2, "total": 2, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Child two", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "spacingAfter": 14,
            ],
            "list": [
                "itemSpacing": 6,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let nsString = result.string as NSString
        let childRange = nsString.range(of: "Child one")
        XCTAssertNotEqual(childRange.location, NSNotFound)

        let attrs = result.attributes(at: childRange.location, effectiveRange: nil)
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle

        XCTAssertEqual(paragraphStyle?.paragraphSpacing ?? -1, 6, accuracy: 0.1)
    }

    func testRender_themeOverridesHorizontalRuleMetrics() {
        let json = """
        [
            {"type": "voidBlock", "nodeType": "horizontalRule", "docPos": 0}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "horizontalRule": [
                "color": "#445566",
                "thickness": 3,
                "verticalMargin": 12,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attachment = result.attribute(.attachment, at: 0, effectiveRange: nil)
            as? HorizontalRuleAttachment
        XCTAssertEqual(attachment?.lineColor, EditorTheme.color(from: "#445566"))
        XCTAssertEqual(attachment?.lineHeight ?? 0, 3, accuracy: 0.1)
        XCTAssertEqual(attachment?.verticalPadding ?? 0, 12, accuracy: 0.1)
    }

    func testListMarkerDrawingRectUsesParagraphLineBox() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 10, width: 160, height: 28)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let baselineY: CGFloat = 28.140625
        let rect = EditorLayoutManager.markerDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            origin: CGPoint(x: 0, y: 0)
        )
        let typographicHeight = markerFont.ascender - markerFont.descender
        let leading = max(markerFont.lineHeight - typographicHeight, 0)
        let expectedY = baselineY - markerFont.ascender - (leading / 2.0)

        XCTAssertEqual(rect.origin.x, 4, accuracy: 0.1)
        XCTAssertEqual(rect.origin.y, expectedY, accuracy: 0.1)
        XCTAssertEqual(rect.height, markerFont.lineHeight, accuracy: 0.1)
    }

    func testListMarkerDrawingRectUsesFullLineFragmentWhenGlyphsUseShorterRect() {
        let markerFont = baseFont.withSize(18)
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 17)
        let baselineY: CGFloat = 30.140625
        let rect = EditorLayoutManager.markerDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            origin: CGPoint(x: 0, y: 0)
        )
        let typographicHeight = markerFont.ascender - markerFont.descender
        let leading = max(markerFont.lineHeight - typographicHeight, 0)
        let expectedY = baselineY - markerFont.ascender - (leading / 2.0)

        XCTAssertEqual(rect.origin.x, 4, accuracy: 0.1)
        XCTAssertEqual(rect.origin.y, expectedY, accuracy: 0.1)
        XCTAssertEqual(rect.height, markerFont.lineHeight, accuracy: 0.1)
    }

    func testListMarkerDrawingRectFallsBackToLineFragmentWhenUsedRectIsEmpty() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 10, width: 160, height: 28)
        let rect = EditorLayoutManager.markerDrawingRect(
            usedRect: CGRect(x: 24, y: 10, width: 160, height: 0),
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: 28.140625,
            markerFont: markerFont,
            origin: CGPoint(x: 0, y: 0)
        )
        let typographicHeight = markerFont.ascender - markerFont.descender
        let leading = max(markerFont.lineHeight - typographicHeight, 0)
        let expectedY = 28.140625 - markerFont.ascender - (leading / 2.0)

        XCTAssertEqual(rect.origin.x, 4, accuracy: 0.1)
        XCTAssertEqual(rect.origin.y, expectedY, accuracy: 0.1)
    }

    func testOrderedMarkerDrawingOriginAlignsToBaselineWithoutParagraphLineHeight() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let baselineY: CGFloat = 30.140625
        let markerText = "12. "

        let point = EditorLayoutManager.orderedMarkerDrawingOrigin(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            markerText: markerText,
            origin: .zero
        )
        let markerWidth = ceil(("12." as NSString).size(withAttributes: [
            .font: markerFont,
        ]).width)

        XCTAssertEqual(point.x, usedRect.minX - 4.0 - markerWidth, accuracy: 0.1)
        XCTAssertEqual(point.y, baselineY - markerFont.ascender, accuracy: 0.1)
    }

    func testOrderedMarkerDrawingOriginIgnoresTrailingSpaceForHorizontalAlignment() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let baselineY: CGFloat = 30.140625
        let markerText = "12. "

        let point = EditorLayoutManager.orderedMarkerDrawingOrigin(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            markerText: markerText,
            origin: .zero
        )
        let visibleWidth = ceil(("12." as NSString).size(withAttributes: [
            .font: markerFont,
        ]).width)
        let fullWidth = ceil((markerText as NSString).size(withAttributes: [
            .font: markerFont,
        ]).width)

        XCTAssertEqual(point.x, usedRect.minX - 4.0 - visibleWidth, accuracy: 0.1)
        XCTAssertNotEqual(point.x, usedRect.minX - 4.0 - fullWidth, accuracy: 0.1)
    }

    func testListMarkerBaseFontUsesParagraphFontInsteadOfLeadingBoldRun() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Bold", "marks": ["bold"]},
            {"type": "textRun", "text": " start", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let textFont = attrs[.font] as? UIFont
        let markerBaseFont = attrs[RenderBridgeAttributes.listMarkerBaseFont] as? UIFont

        XCTAssertTrue(
            textFont?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "First text run should still be bold"
        )
        XCTAssertNotNil(markerBaseFont, "List marker should carry its paragraph base font")
        XCTAssertFalse(
            markerBaseFont?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "Marker base font should ignore inline bold marks on the first run"
        )
        XCTAssertEqual(markerBaseFont?.pointSize ?? 0, baseFont.pointSize, accuracy: 0.1)
    }

    func testListMarkerParagraphStylePreservesThemedLineHeight() {
        let sourceStyle = NSMutableParagraphStyle()
        sourceStyle.minimumLineHeight = 28
        sourceStyle.maximumLineHeight = 28

        let markerStyle = EditorLayoutManager.markerParagraphStyle(from: [
            .paragraphStyle: sourceStyle,
        ])

        XCTAssertEqual(markerStyle.minimumLineHeight, 28, accuracy: 0.1)
        XCTAssertEqual(markerStyle.maximumLineHeight, 28, accuracy: 0.1)
        XCTAssertEqual(markerStyle.alignment, .right)
        XCTAssertEqual(markerStyle.lineBreakMode, .byClipping)
        XCTAssertEqual(markerStyle.firstLineHeadIndent, 0, accuracy: 0.1)
        XCTAssertEqual(markerStyle.headIndent, 0, accuracy: 0.1)
        XCTAssertEqual(markerStyle.tailIndent, 0, accuracy: 0.1)
    }

    func testUnorderedBulletDrawingRectCentersBulletOnTextMidline() {
        let rect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: CGRect(x: 24, y: 14, width: 160, height: 19),
            lineFragmentRect: CGRect(x: 24, y: 8, width: 160, height: 32),
            markerWidth: 20,
            baselineY: 28.140625,
            baseFont: baseFont,
            markerScale: 2,
            origin: .zero
        )
        let targetMidline = 28.140625 - ((baseFont.xHeight > 0 ? baseFont.xHeight : baseFont.capHeight) / 2.0)

        XCTAssertEqual(rect.midY, targetMidline, accuracy: 0.1)
        XCTAssertGreaterThan(rect.width, 0)
        XCTAssertGreaterThan(rect.height, 0)
    }

    func testUnorderedBulletDrawingRectPreservesTextSideGapAcrossMarkerScales() {
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let baselineY: CGFloat = 28.140625

        let normalRect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            baseFont: baseFont,
            markerScale: 1,
            origin: .zero
        )
        let scaledRect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            baseFont: baseFont,
            markerScale: 2,
            origin: .zero
        )

        XCTAssertEqual(usedRect.minX - normalRect.maxX, usedRect.minX - scaledRect.maxX, accuracy: 0.1)
        XCTAssertEqual(usedRect.minX - scaledRect.maxX, LayoutConstants.listMarkerTextGap, accuracy: 0.1)
    }

    func testUnorderedBulletDrawingRectReproducesTallLineHeightListItem() {
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
            "list": [
                "markerScale": 2,
            ],
        ])
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Bullet item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let textFont = attrs[.font] as? UIFont ?? baseFont
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle
        let markerScale = (attrs[RenderBridgeAttributes.listMarkerScale] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? 1
        let bulletRect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: CGRect(x: 24, y: 14, width: 160, height: 19),
            lineFragmentRect: CGRect(x: 24, y: 8, width: 160, height: 32),
            markerWidth: 20,
            baselineY: 28.140625,
            baseFont: textFont,
            markerScale: markerScale,
            origin: .zero
        )
        let expectedCenterY = 28.140625 - ((textFont.xHeight > 0 ? textFont.xHeight : textFont.capHeight) / 2.0)

        XCTAssertNotNil(attrs[RenderBridgeAttributes.listMarkerContext])
        XCTAssertEqual(paragraphStyle?.minimumLineHeight ?? 0, 32, accuracy: 0.1)
        XCTAssertEqual(paragraphStyle?.maximumLineHeight ?? 0, 32, accuracy: 0.1)
        XCTAssertEqual(bulletRect.midY, expectedCenterY, accuracy: 0.1)
        XCTAssertGreaterThan(bulletRect.width, 0)
        XCTAssertGreaterThan(bulletRect.height, 0)
        XCTAssertEqual(bulletRect.width, bulletRect.height, accuracy: 0.1)
    }

    func testOrderedListMarkerBaselineOffsetIsNeutral() {
        let orderedContext: [String: Any] = ["ordered": true]

        let offset = EditorLayoutManager.markerBaselineOffset(
            for: orderedContext,
            baseFont: baseFont,
            markerFont: baseFont
        )

        XCTAssertEqual(offset, 0, accuracy: 0.1)
    }

    func testMarkerBaseFontPrefersStoredParagraphFont() {
        let boldDescriptor = baseFont.fontDescriptor.withSymbolicTraits(.traitBold)
            ?? baseFont.fontDescriptor
        let boldFont = UIFont(descriptor: boldDescriptor, size: baseFont.pointSize)
        let resolved = EditorLayoutManager.markerBaseFont(from: [
            .font: boldFont,
            RenderBridgeAttributes.listMarkerBaseFont: baseFont,
        ])

        XCTAssertFalse(
            resolved.fontDescriptor.symbolicTraits.contains(.traitBold),
            "Stored paragraph font should win over the inline bold run font"
        )
        XCTAssertEqual(resolved.pointSize, baseFont.pointSize, accuracy: 0.1)
    }

    // MARK: - HorizontalRuleAttachment

    func testHorizontalRuleAttachment_bounds() {
        let attachment = HorizontalRuleAttachment()
        let proposedRect = CGRect(x: 0, y: 0, width: 320, height: 20)
        let bounds = attachment.attachmentBounds(
            for: nil,
            proposedLineFragment: proposedRect,
            glyphPosition: .zero,
            characterIndex: 0
        )

        XCTAssertEqual(
            bounds.width, 320,
            "Attachment width should match proposed line fragment width"
        )
        let expectedHeight = 1.0 + (8.0 * 2)  // line + padding
        XCTAssertEqual(
            bounds.height, expectedHeight,
            "Attachment height should be line height + 2 * vertical padding"
        )
    }

    func testHorizontalRuleAttachment_rendersImage() {
        let attachment = HorizontalRuleAttachment()
        attachment.lineColor = .red
        let bounds = CGRect(x: 0, y: 0, width: 200, height: 17)
        let image = attachment.image(
            forBounds: bounds,
            textContainer: nil,
            characterIndex: 0
        )
        XCTAssertNotNil(image, "HorizontalRuleAttachment should produce a non-nil image")
    }

    // MARK: - Height Measurement

    func testMeasureHeightForSingleParagraph() {
        let renderJSON = """
        [
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"Hello world"},
            {"type":"blockEnd"}
        ]
        """
        let height = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: nil,
            width: 375
        )
        XCTAssertGreaterThan(height, 0, "Single paragraph should have positive height")
    }

    func testMeasureHeightFromBackgroundWaitsForMainThreadMeasurement() {
        let finished = expectation(description: "background measurement finished")
        let started = DispatchSemaphore(value: 0)
        let completion = DispatchSemaphore(value: 0)
        let renderJSON = """
        [
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"Measured on main"},
            {"type":"blockEnd"}
        ]
        """

        DispatchQueue.global(qos: .userInitiated).async {
            started.signal()
            _ = RenderBridge.measureHeight(
                forRenderJSON: renderJSON,
                themeJSON: nil,
                width: 320
            )
            completion.signal()
            finished.fulfill()
        }

        XCTAssertEqual(started.wait(timeout: .now() + 1), .success)
        XCTAssertEqual(
            completion.wait(timeout: .now() + 0.1),
            .timedOut,
            "background callers must synchronously marshal UIKit measurement to the main thread"
        )
        wait(for: [finished], timeout: 1)
    }

    func testMeasureHeightForEmptyContent() {
        let renderJSON = "[]"
        let height = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: nil,
            width: 375
        )
        XCTAssertEqual(height, 0, "Empty content should have zero height")
    }

    func testMeasureHeightRespectsWidth() {
        let longText = String(repeating: "word ", count: 100)
        let renderJSON = """
        [
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"\(longText)"},
            {"type":"blockEnd"}
        ]
        """
        let narrowHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: nil,
            width: 100
        )
        let wideHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: nil,
            width: 1000
        )
        XCTAssertGreaterThan(narrowHeight, wideHeight, "Narrower width should produce taller height")
    }

    func testMeasureHeightRespectsThemeFontSize() {
        let renderJSON = """
        [
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"Hello world"},
            {"type":"blockEnd"}
        ]
        """
        let smallTheme = """
        {"text":{"fontSize":12}}
        """
        let largeTheme = """
        {"text":{"fontSize":32}}
        """
        let smallHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: smallTheme,
            width: 375
        )
        let largeHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: largeTheme,
            width: 375
        )
        XCTAssertGreaterThan(largeHeight, smallHeight, "Larger font should produce taller height")
    }

    func testMeasureHeightRespectsContentInsets() {
        let renderJSON = """
        [
            {"type":"blockStart","nodeType":"paragraph","depth":0},
            {"type":"textRun","text":"Hello world"},
            {"type":"blockEnd"}
        ]
        """
        let noInsetHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: nil,
            width: 375
        )
        let insetTheme = """
        {"contentInsets":{"top":20,"bottom":20}}
        """
        let insetHeight = RenderBridge.measureHeight(
            forRenderJSON: renderJSON,
            themeJSON: insetTheme,
            width: 375
        )
        XCTAssertEqual(insetHeight, noInsetHeight + 40, accuracy: 1.0, "Content insets should add to height")
    }

    func testRender_imageAttachmentHonorsPreferredDimensions() {
        let json = """
        [
            {"type": "voidBlock", "nodeType": "image", "docPos": 1, "attrs": {
                "src": "https://example.com/cat.png",
                "width": 140,
                "height": 80
            }}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        XCTAssertEqual(result.string, LayoutConstants.objectReplacementCharacter)

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let attachment = attrs[.attachment] as? NSTextAttachment
        XCTAssertNotNil(attachment, "Image render should produce an attachment")

        let bounds = attachment?.attachmentBounds(
            for: nil,
            proposedLineFragment: CGRect(x: 0, y: 0, width: 320, height: 24),
            glyphPosition: .zero,
            characterIndex: 0
        )

        XCTAssertEqual(bounds?.width ?? 0, 140, accuracy: 0.1)
        XCTAssertEqual(bounds?.height ?? 0, 80, accuracy: 0.1)
    }
}

private func imagePolicy(
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

private func onePixelImage() -> UIImage {
    let renderer = UIGraphicsImageRenderer(size: CGSize(width: 1, height: 1))
    return renderer.image { context in
        UIColor.red.setFill()
        context.fill(CGRect(x: 0, y: 0, width: 1, height: 1))
    }
}

private func paddedBackingImage(bytesPerRow: Int, height: Int) -> UIImage {
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

private func imageRenderJSON(source: String) -> String {
    """
    [{"type":"voidBlock","nodeType":"image","docPos":1,"attrs":{"src":"\(source)"}}]
    """
}

private final class ManualImageTimeoutScheduler {
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

private final class ConcurrentImageTimeoutScheduler {
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

    private let lock = NSLock()
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

private final class ManualImageClock {
    private let lock = NSLock()
    private var value: TimeInterval = 0
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

private final class DeadlineAdvancingImageDecoder: ImageDataDecoding {
    private let clock: ManualImageClock
    private let image: UIImage

    init(clock: ManualImageClock, image: UIImage) {
        self.clock = clock
        self.image = image
    }

    func decode(_ data: Data, maxDimension: Int) -> UIImage? {
        clock.advance(to: 61)
        return image
    }
}

private final class ManualImageDeliveryScheduler {
    private let condition = NSCondition()
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

private final class BlockingImageDecoder: ImageDataDecoding {
    private let condition = NSCondition()
    private let result: UIImage?
    private var permits = 0
    private var concurrentDecodes = 0
    private(set) var decodeCount = 0
    private(set) var maximumConcurrentDecodes = 0

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

private final class RecordingImageDecoder: ImageDataDecoding {
    private let lock = NSLock()
    private let result: UIImage?
    private(set) var decodeCount = 0
    private(set) var calledOnMainThread: Bool?

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

private final class ImmediateImageTransport: ImageLoadingTransport {
    let result: Result<Data, Error>
    private(set) var receivedPolicy: ImageLoadingPolicy?

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

private final class HoldingImageTransport: ImageLoadingTransport {
    private let lock = NSLock()
    private var completions: [(Result<Data, Error>) -> Void] = []
    private(set) var requestCount = 0
    private(set) var cancelCount = 0
    private(set) var receivedPolicies: [ImageLoadingPolicy] = []

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
