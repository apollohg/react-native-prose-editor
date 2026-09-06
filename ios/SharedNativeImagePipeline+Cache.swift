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

    let lock = NSLock()
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
