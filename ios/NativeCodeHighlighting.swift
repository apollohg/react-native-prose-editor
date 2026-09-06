import Foundation

public struct NativeCodeHighlightRange {
    public let start: Int
    public let length: Int
    public let color: UInt32
    public let fontStyle: UInt8

    public init(start: Int, length: Int, color: UInt32, fontStyle: UInt8) {
        self.start = start
        self.length = length
        self.color = color
        self.fontStyle = fontStyle
    }
}

public protocol NativeCodeHighlightingProvider: AnyObject {
    var id: String { get }
    var version: Int { get }
    func highlight(text: String, language: String?, theme: String) throws -> [NativeCodeHighlightRange]
}

public enum NativeCodeHighlightingRegistry {
    private static let lock = NSLock()
    private static var providers: [String: NativeCodeHighlightingProvider] = [:]

    public static func register(provider: NativeCodeHighlightingProvider) throws {
        guard !provider.id.isEmpty, provider.version == 1 else {
            throw failure("Code highlighting provider must have an ID and support version 1")
        }
        lock.lock()
        defer { lock.unlock() }
        providers[provider.id] = provider
    }

    public static func provider(id: String) throws -> NativeCodeHighlightingProvider {
        lock.lock()
        defer { lock.unlock() }
        guard let provider = providers[id] else {
            throw failure("Code highlighting provider '\(id)' is unavailable. Install and import its native package, then rebuild the app.")
        }
        return provider
    }

    static func validRanges(text: String, ranges: [NativeCodeHighlightRange]) -> Bool {
        let units = Array(text.utf16)
        func isBoundary(_ offset: Int) -> Bool {
            offset == 0 || offset == units.count || !(0xD800...0xDBFF).contains(units[offset - 1]) || !(0xDC00...0xDFFF).contains(units[offset])
        }
        var end = 0
        for range in ranges {
            guard range.start >= end, range.length > 0, range.start <= units.count,
                  range.length <= units.count - range.start, range.fontStyle <= 7,
                  isBoundary(range.start), isBoundary(range.start + range.length) else { return false }
            end = range.start + range.length
        }
        return true
    }

    private static func failure(_ message: String) -> NSError {
        NSError(domain: "NativeCodeHighlighting", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }
}

struct NativeCodeHighlightBlock {
    let start: Int
    let text: String
    let language: String?
}

struct NativeHighlightedCodeBlock {
    let block: NativeCodeHighlightBlock
    let ranges: [NativeCodeHighlightRange]
}

final class NativeCodeHighlightingSession {
    private struct Request {
        let generation: UInt64
        let provider: NativeCodeHighlightingProvider
        let theme: String
        let blocks: [NativeCodeHighlightBlock]
        let completion: (Result<[NativeHighlightedCodeBlock], Error>) -> Void
    }

    private static let queue = DispatchQueue(label: "editor.code-highlighting", qos: .userInitiated)
    private let lock = NSLock()
    private var generation: UInt64 = 0
    private var pending: Request?
    private var running = false

    func cancel() {
        precondition(Thread.isMainThread, "Highlighting sessions must be updated on the main thread")
        lock.lock()
        generation &+= 1
        pending = nil
        lock.unlock()
    }

    func update(provider id: String, theme: String, blocks: [NativeCodeHighlightBlock],
                completion: @escaping (Result<[NativeHighlightedCodeBlock], Error>) -> Void) throws {
        cancel()
        let provider = try NativeCodeHighlightingRegistry.provider(id: id)
        lock.lock()
        pending = Request(generation: generation, provider: provider, theme: theme, blocks: blocks, completion: completion)
        let schedule = !running
        running = true
        lock.unlock()
        if schedule { Self.queue.async { [weak self] in self?.drain() } }
    }

    private func current(_ value: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return generation == value
    }

    private func takeRequest() -> Request? {
        lock.lock()
        defer { lock.unlock() }
        let request = pending
        pending = nil
        if request == nil { running = false }
        return request
    }

    private func scheduleNext() {
        lock.lock()
        let schedule = pending != nil
        running = schedule
        lock.unlock()
        if schedule { Self.queue.async { [weak self] in self?.drain() } }
    }

    private func drain() {
        guard let request = takeRequest() else { return }
        defer { scheduleNext() }
        let result = Result<[NativeHighlightedCodeBlock], Error> {
            var output: [NativeHighlightedCodeBlock] = []
            for block in request.blocks {
                guard current(request.generation) else { return [] }
                let ranges = try request.provider.highlight(text: block.text, language: block.language, theme: request.theme)
                guard NativeCodeHighlightingRegistry.validRanges(text: block.text, ranges: ranges) else {
                    throw NSError(domain: "NativeCodeHighlighting", code: 2, userInfo: [NSLocalizedDescriptionKey: "Code highlighting provider returned invalid UTF-16 ranges"])
                }
                output.append(NativeHighlightedCodeBlock(block: block, ranges: ranges))
            }
            return output
        }
        guard current(request.generation) else { return }
        DispatchQueue.main.async { [weak self] in
            guard self?.current(request.generation) == true else { return }
            request.completion(result)
        }
    }
}
