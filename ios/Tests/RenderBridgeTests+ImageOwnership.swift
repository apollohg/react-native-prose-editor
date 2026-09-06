import XCTest
import CoreText

extension RenderBridgeTests {
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
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
        let editorId = makeV2Editor()
        defer { destroyV2Editor(id: editorId) }
        _ = EditorV2Shadow.setJson(
            id: editorId,
            json: #"{"type":"doc","content":[{"type":"image","attrs":{"src":"https://example.com/policy.png"}}]}"#
        )
        let state = EditorV2Shadow.getCurrentState(id: editorId)
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

    func testEditorAppliesImageLoadingPolicyJson() {
        let json = """
        {"maxSourceBytes":1234,"connectTimeoutMs":2500,"readTimeoutMs":3500,"maxConcurrentRequests":4,"maxPendingRequests":8,"maxDecodeDimensionPx":640}
        """
        let editor = NativeEditorExpoView()

        editor.setImageLoadingPolicyJson(json)

        XCTAssertEqual(editor.imageLoadingPolicy.maxSourceBytes, 1234)
    }

    func testSetAtomsJsonReappliesTheCurrentRender() throws {
        let config: [String: Any] = [
            "initialization": [
                "type": "localJson",
                "json": [
                    "type": "doc",
                    "content": [["type": "counterCard", "attrs": ["title": "Sample item"]]],
                ],
            ],
            "schema": [
                "nodes": [
                    ["name": "doc", "content": "block+", "role": "doc"],
                    [
                        "name": "paragraph",
                        "content": "text*",
                        "group": "block",
                        "role": "textBlock",
                        "htmlTag": "p",
                    ],
                    ["name": "text", "content": "", "role": "text"],
                    [
                        "name": "counterCard",
                        "content": "",
                        "group": "block",
                        "role": "block",
                        "isVoid": true,
                        "attrs": ["title": ["default": ""]],
                        "html": [
                            "tag": "div",
                            "staticAttrs": ["data-type": "counter-card"],
                            "attrMap": ["title": "data-title"],
                        ],
                    ],
                ],
                "marks": [],
            ],
        ]
        let data = try JSONSerialization.data(withJSONObject: config)
        let editorId = makeV2Editor(configJson: try XCTUnwrap(String(data: data, encoding: .utf8)))
        defer { destroyV2Editor(id: editorId) }
        let editor = NativeEditorExpoView()
        editor.setEditorId(editorId)
        defer { editor.setEditorId(0) }

        XCTAssertNil(editor.richTextView.textView.textStorage.attribute(
            .attachment,
            at: 0,
            effectiveRange: nil
        ))

        editor.setAtomsJson(
            #"{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"#
        )

        let attachment = editor.richTextView.textView.textStorage.attribute(
            .attachment,
            at: 0,
            effectiveRange: nil
        ) as? AtomBlockAttachment
        XCTAssertEqual(attachment?.nodeType, "counterCard")
        XCTAssertEqual(attachment?.reservedHeight, 120)
    }

}
