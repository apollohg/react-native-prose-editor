import XCTest

final class CodeHighlightingTests: XCTestCase {
    func testValidatesUTF16RangesBeforeApplyingProviderOutput() {
        let text = "a😀b"
        XCTAssertTrue(NativeCodeHighlightingRegistry.validRanges(text: text, ranges: [.init(start: 1, length: 2, color: 0x112233ff, fontStyle: 1)]))
        XCTAssertFalse(NativeCodeHighlightingRegistry.validRanges(text: text, ranges: [.init(start: 1, length: 1, color: 0x112233ff, fontStyle: 0)]))
        XCTAssertFalse(NativeCodeHighlightingRegistry.validRanges(text: text, ranges: [.init(start: 3, length: 2, color: 0x112233ff, fontStyle: 0)]))
        XCTAssertFalse(NativeCodeHighlightingRegistry.validRanges(text: text, ranges: [
            .init(start: 0, length: 3, color: 0x112233ff, fontStyle: 0),
            .init(start: 1, length: 2, color: 0x112233ff, fontStyle: 0),
        ]))
    }

    func testRejectsIncompatibleProvidersAndResolvesRegisteredProvider() throws {
        let provider = TestHighlighter(version: 1)
        try NativeCodeHighlightingRegistry.register(provider: provider)
        XCTAssertTrue(try NativeCodeHighlightingRegistry.provider(id: "test-provider") === provider)
        XCTAssertThrowsError(try NativeCodeHighlightingRegistry.provider(id: "absent-provider"))
        XCTAssertThrowsError(try NativeCodeHighlightingRegistry.register(provider: TestHighlighter(version: 2)))
    }
}

private final class TestHighlighter: NativeCodeHighlightingProvider {
    let id = "test-provider"
    let version: Int
    init(version: Int) { self.version = version }
    func highlight(text: String, language: String?, theme: String) throws -> [NativeCodeHighlightRange] { [] }
}

final class CodeHighlightingSessionTests: XCTestCase {
    func testYieldsToAnotherSessionBeforeProcessingReplacement() throws {
        let provider = FairnessHighlighter()
        try NativeCodeHighlightingRegistry.register(provider: provider)
        let first = NativeCodeHighlightingSession()
        let second = NativeCodeHighlightingSession()
        let blocks = [NativeCodeHighlightBlock(start: 0, text: "code", language: nil)]
        let done = expectation(description: "both sessions")
        done.expectedFulfillmentCount = 2
        try first.update(provider: provider.id, theme: "blocked", blocks: blocks) { _ in
            XCTFail("Stale request was delivered")
        }
        XCTAssertEqual(provider.entered.wait(timeout: .now() + 2), .success)
        try second.update(provider: provider.id, theme: "other", blocks: blocks) { _ in done.fulfill() }
        try first.update(provider: provider.id, theme: "replacement", blocks: blocks) { _ in done.fulfill() }
        provider.release.signal()
        wait(for: [done], timeout: 3)
        XCTAssertEqual(provider.calls, ["blocked", "other", "replacement"])
    }

    func testOnlyLatestRequestIsDeliveredOffTheParserQueue() throws {
        let entered = DispatchSemaphore(value: 0)
        let release = DispatchSemaphore(value: 0)
        let provider = BlockingHighlighter(entered: entered, release: release)
        try NativeCodeHighlightingRegistry.register(provider: provider)
        let session = NativeCodeHighlightingSession()
        let done = expectation(description: "latest")
        try session.update(provider: provider.id, theme: "old", blocks: [.init(start: 0, text: "old", language: "rust")]) { _ in
            XCTFail("Stale request was delivered")
        }
        XCTAssertEqual(entered.wait(timeout: .now() + 2), .success)
        try session.update(provider: provider.id, theme: "new", blocks: [.init(start: 4, text: "new", language: "swift")]) { result in
            XCTAssertTrue(Thread.isMainThread)
            XCTAssertEqual(try? result.get().first?.block.start, 4)
            done.fulfill()
        }
        release.signal()
        wait(for: [done], timeout: 3)
    }
}

private final class BlockingHighlighter: NativeCodeHighlightingProvider {
    let id = "session-test-provider"
    let version = 1
    let entered: DispatchSemaphore
    let release: DispatchSemaphore
    init(entered: DispatchSemaphore, release: DispatchSemaphore) { self.entered = entered; self.release = release }
    func highlight(text: String, language: String?, theme: String) throws -> [NativeCodeHighlightRange] {
        XCTAssertFalse(Thread.isMainThread)
        if theme == "old" { entered.signal(); _ = release.wait(timeout: .now() + 3) }
        return [.init(start: 0, length: text.utf16.count, color: 0xff0000ff, fontStyle: 0)]
    }
}

private final class FairnessHighlighter: NativeCodeHighlightingProvider {
    let id = "session-fairness-provider"
    let version = 1
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    var calls: [String] = []

    func highlight(text: String, language: String?, theme: String) throws -> [NativeCodeHighlightRange] {
        calls.append(theme)
        if theme == "blocked" { entered.signal(); _ = release.wait(timeout: .now() + 3) }
        return []
    }
}
