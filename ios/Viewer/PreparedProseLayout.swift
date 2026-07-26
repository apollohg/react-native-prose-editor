import CoreText
import UIKit

/// Validates the physical-width identity before it crosses into an integer key.
enum ProseLayoutMetrics {
    static func widthPixels(widthPoints: CGFloat, scale: CGFloat) -> Int? {
        guard widthPoints.isFinite, widthPoints > 0, scale.isFinite, scale > 0 else {
            return nil
        }
        let physicalWidth = widthPoints * scale
        guard physicalWidth.isFinite, physicalWidth > 0 else { return nil }
        let roundedWidth = physicalWidth.rounded()
        // `CGFloat(Int.max)` rounds up to 2^63 on 64-bit platforms. Its predecessor
        // is the largest floating-point value that remains safe to convert to `Int`.
        guard roundedWidth.isFinite,
              roundedWidth > 0,
              roundedWidth <= CGFloat(Int.max).nextDown
        else {
            return nil
        }
        return Int(roundedWidth)
    }

    static func canonicalWidth(widthPixels: Int, scale: CGFloat) -> CGFloat {
        CGFloat(widthPixels) / scale
    }
}

struct ProseLayoutKey: Hashable {
    let semanticKey: String
    let widthPixels: Int
    let themeDigest: String
    let nativeFontRevision: UInt64
    let fontEnvironmentRevision: UInt64
    let displayScaleBits: UInt64
    let attachmentRevision: UInt64
    let generationIdentity: String

    init(
        semanticKey: String,
        widthPixels: Int,
        themeDigest: String,
        nativeFontRevision: UInt64,
        fontEnvironmentRevision: UInt64,
        displayScale: CGFloat,
        attachmentRevision: UInt64,
        generationIdentity: String
    ) {
        self.semanticKey = semanticKey
        self.widthPixels = widthPixels
        self.themeDigest = themeDigest
        self.nativeFontRevision = nativeFontRevision
        self.fontEnvironmentRevision = fontEnvironmentRevision
        self.displayScaleBits = Double(displayScale).bitPattern
        self.attachmentRevision = attachmentRevision
        self.generationIdentity = generationIdentity
    }
}

final class PreparedProseBlock {
    let line: CTLine
    /// Core Text baseline measured down from the artifact's top edge.
    let origin: CGPoint
    let range: NSRange
    let bounds: CGRect

    init(line: CTLine, origin: CGPoint, range: NSRange, bounds: CGRect) {
        self.line = line
        self.origin = origin
        self.range = range
        self.bounds = bounds
    }
}

struct PreparedProseInteraction: Hashable {}
struct PreparedProseAccessibilityNode: Hashable {}

public final class PreparedProseLayout: NSObject {
    let key: ProseLayoutKey
    let size: CGSize
    let blocks: [PreparedProseBlock]
    let interactions: [PreparedProseInteraction]
    let accessibilityNodes: [PreparedProseAccessibilityNode]
    let retainedBytes: Int
    let error: ProseViewerError?

    init(
        key: ProseLayoutKey,
        size: CGSize,
        blocks: [PreparedProseBlock],
        interactions: [PreparedProseInteraction] = [],
        accessibilityNodes: [PreparedProseAccessibilityNode] = [],
        retainedBytes: Int,
        error: ProseViewerError? = nil
    ) {
        self.key = key
        self.size = size
        self.blocks = blocks
        self.interactions = interactions
        self.accessibilityNodes = accessibilityNodes
        self.retainedBytes = retainedBytes
        self.error = error
        super.init()
    }

    static func error(key: ProseLayoutKey, width: CGFloat, error: ProseViewerError) -> PreparedProseLayout {
        PreparedProseLayout(key: key, size: CGSize(width: width, height: 0), blocks: [], retainedBytes: 0, error: error)
    }
}
