import XCTest
import CoreText

extension RenderBridgeTests {
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

    func testImageLoaderRendersSVGFromRemoteBytesAndDataURLsWithinDecodeLimit() {
        let data = Data(#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8000 4000"><rect width="8000" height="4000" fill="red"/><text x="100" y="1000" font-size="600">Sample</text></svg>"#.utf8)
        for source in ["https://example.test/svg-\(UUID().uuidString)?token=test", "data:image/svg+xml;base64,\(data.base64EncodedString())"] {
            let owner = RenderImageLoadOwner(
                policy: imagePolicy(maxDecodeDimension: 64),
                transport: ImmediateImageTransport(result: .success(data))
            )
            let completed = expectation(description: "SVG decoded")
            XCTAssertTrue(owner.loadImage(source: source) { image in
                XCTAssertNotNil(image)
                XCTAssertEqual(image?.cgImage?.width, 64)
                XCTAssertEqual(image?.cgImage?.height, 32)
                completed.fulfill()
            })
            wait(for: [completed], timeout: 3)
        }
    }

    func testImageLoaderRendersSVGShapesAndText() throws {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "shapes-and-text", withExtension: "svg"))
        let data = try Data(contentsOf: url)
        let owner = RenderImageLoadOwner(policy: .default, transport: ImmediateImageTransport(result: .success(data)))
        let completed = expectation(description: "SVG fixture decoded")
        XCTAssertTrue(owner.loadImage(source: "https://example.test/fixture-\(UUID().uuidString).svg") { image in
            XCTAssertNotNil(image)
            XCTAssertEqual(image?.cgImage?.width, 320)
            XCTAssertEqual(image?.cgImage?.height, 180)
            if let image {
                let attachment = XCTAttachment(image: image)
                attachment.name = "shapes-and-text-rendered"
                attachment.lifetime = .keepAlways
                self.add(attachment)
            }
            completed.fulfill()
        })
        wait(for: [completed], timeout: 3)
    }

    func testSVGViewportDimensionsAndAspectRatioMapping() throws {
        for aspect in ["xMidYMid meet", "xMidYMid slice", "none"] {
            let data = Data("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"100\" viewBox=\"10 20 100 100\" preserveAspectRatio=\"\(aspect)\"><rect x=\"10\" y=\"20\" width=\"100\" height=\"100\" fill=\"red\"/></svg>".utf8)
            let image = try XCTUnwrap(NativeSVGImageDecoder.decode(data, maxDimension: 200)?.cgImage)
            XCTAssertEqual(image.width, 200)
            XCTAssertEqual(image.height, 100)
            guard image.width == 200, image.height == 100 else { continue }
            XCTAssertEqual(image.bitsPerPixel, 32)
            let pixels = try XCTUnwrap(image.dataProvider?.data)
            let bytes = try XCTUnwrap(CFDataGetBytePtr(pixels))
            XCTAssertEqual(bytes[50 * image.bytesPerRow + 100 * 4 + 3], 255)
            XCTAssertEqual(bytes[50 * image.bytesPerRow + 25 * 4 + 3], aspect == "xMidYMid meet" ? 0 : 255)
        }
    }

    func testSVGRendersLocalGradientAndUseReferences() throws {
        let svg = ##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><defs><linearGradient id="paint"><stop offset="0" stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient><rect id="tile" width="20" height="20" fill="url(#paint)"/></defs><use href="#tile"/></svg>"##
        let image = try XCTUnwrap(NativeSVGImageDecoder.decode(Data(svg.utf8), maxDimension: 64)?.cgImage)
        XCTAssertEqual(image.width, 20)
        let pixels = try XCTUnwrap(image.dataProvider?.data)
        let bytes = try XCTUnwrap(CFDataGetBytePtr(pixels))
        XCTAssertEqual(bytes[10 * image.bytesPerRow + 10 * 4 + 3], 255)
    }

    func testSVGUsesPhysicalUnitsAndInfersMissingViewportDimension() throws {
        let dimensions = [
            (#"width="1in" height="0.5in""#, 96, 48),
            (#"width="120""#, 120, 60),
            (#"height="60""#, 120, 60),
            ("", 200, 100),
        ]
        for (attributes, width, height) in dimensions {
            let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" \(attributes) viewBox=\"0 0 200 100\"><rect width=\"200\" height=\"100\"/></svg>"
            let image = try XCTUnwrap(NativeSVGImageDecoder.decode(Data(svg.utf8), maxDimension: 200)?.cgImage)
            XCTAssertEqual(image.width, width)
            XCTAssertEqual(image.height, height)
        }
    }

    func testSVGRejectsBitmapClipsAndRendersVectorClips() throws {
        let rejected = [
            ##"<mask id="clip"><rect width="1000000" height="1000000" fill="white"/></mask>"##,
            ##"<clipPath id="clip"><rect width="10" height="10"/><path d="M0 0H10V10H0Z" clip-rule="evenodd"/></clipPath>"##,
            ##"<clipPath id="clip"><text x="0" y="10">Sample</text></clipPath>"##,
        ]
        for definition in rejected {
            let attribute = definition.hasPrefix("<mask") ? "mask" : "clip-path"
            let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1000000\" height=\"1000000\"><defs>\(definition)</defs><rect width=\"1000000\" height=\"1000000\" \(attribute)=\"url(#clip)\"/></svg>"
            XCTAssertNil(NativeSVGImageDecoder.decode(Data(svg.utf8), maxDimension: 64))
        }
        let vector = ##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><defs><clipPath id="clip"><rect width="10" height="20"/></clipPath></defs><rect width="20" height="20" fill="red" clip-path="url(#clip)"/></svg>"##
        let image = try XCTUnwrap(NativeSVGImageDecoder.decode(Data(vector.utf8), maxDimension: 64)?.cgImage)
        let pixels = try XCTUnwrap(image.dataProvider?.data)
        let bytes = try XCTUnwrap(CFDataGetBytePtr(pixels))
        XCTAssertEqual(bytes[10 * image.bytesPerRow + 5 * 4 + 3], 255)
        XCTAssertEqual(bytes[10 * image.bytesPerRow + 15 * 4 + 3], 0)
    }

    func testImageLoaderRejectsMalformedOrExternallyReferencingSVG() {
        let inputs = [
            "<svg><path></svg>",
            #"<!DOCTYPE svg [<!ENTITY remote SYSTEM "https://example.test/secret">]><svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text>&remote;</text></svg>"#,
            #"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="https://example.test/a.png"/></svg>"#,
            ##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><use id="loop" href="#loop"/></svg>"##,
            #"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><script>alert(1)</script></svg>"#,
        ]
        for input in inputs {
            let owner = RenderImageLoadOwner(policy: .default, transport: ImmediateImageTransport(result: .success(Data(input.utf8))))
            let completed = expectation(description: "unsupported SVG rejected")
            XCTAssertTrue(owner.loadImage(source: "https://example.test/invalid-\(UUID().uuidString).svg") { image in
                XCTAssertNil(image)
                completed.fulfill()
            })
            wait(for: [completed], timeout: 3)
        }
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

}
