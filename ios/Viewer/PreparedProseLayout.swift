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

/// Fabric recycles component views, so the handoff owner must identify both
/// the mounted surface and the component instance within it.
struct FabricSurfaceToken: Hashable {
    let surfaceId: Int64
    let componentTag: Int64
}

struct FabricLeaseKey: Hashable {
    let surface: FabricSurfaceToken
    let layout: ProseLayoutKey
}

struct FabricGenerationToken: Hashable {
    let surface: FabricSurfaceToken
    let generationIdentity: String
}

/// This identity intentionally excludes the surface owner: completed immutable
/// layouts may be shared, while leases remain surface-scoped.
struct ProseMountKey: Hashable {
    let generationIdentity: String
    let widthPixels: Int
    let displayScaleBits: UInt64

    init(generationIdentity: String, widthPixels: Int, displayScale: CGFloat) {
        self.init(
            generationIdentity: generationIdentity,
            widthPixels: widthPixels,
            displayScaleBits: Double(displayScale).bitPattern
        )
    }

    init(generationIdentity: String, widthPixels: Int, displayScaleBits: UInt64) {
        self.generationIdentity = generationIdentity
        self.widthPixels = widthPixels
        self.displayScaleBits = displayScaleBits
    }
}

enum PreparedProseFragmentKind: String, Hashable {
    case text
    case marker
    case background
    case border
    case rule
    case atom
}

/// A fully prepared paint operation. Core Text lines, colours, metrics, and
/// rectangles are all frozen before this reaches the drawing view.
final class PreparedProseFragment {
    let kind: PreparedProseFragmentKind
    let line: CTLine?
    /// Core Text baseline measured down from the artifact's top edge.
    let origin: CGPoint
    let bounds: CGRect
    let color: CGColor?
    let cornerRadius: CGFloat
    let strokeWidth: CGFloat
    let label: String?
    let checked: Bool

    init(
        kind: PreparedProseFragmentKind,
        line: CTLine? = nil,
        origin: CGPoint = .zero,
        bounds: CGRect,
        color: CGColor? = nil,
        cornerRadius: CGFloat = 0,
        strokeWidth: CGFloat = 0,
        label: String? = nil,
        checked: Bool = false
    ) {
        self.kind = kind
        self.line = line
        self.origin = origin
        self.bounds = bounds
        self.color = color
        self.cornerRadius = cornerRadius
        self.strokeWidth = strokeWidth
        self.label = label
        self.checked = checked
    }
}

/// A vertical-culling unit, sorted by its top edge. It owns every paint
/// operation needed for one semantic block so draw(_:) never shapes text.
final class PreparedProseBlock {
    let fragments: [PreparedProseFragment]
    let bounds: CGRect

    init(fragments: [PreparedProseFragment], bounds: CGRect) {
        self.fragments = fragments
        self.bounds = bounds
    }

    /// Compatibility initializer retained for Task 3 test seams.
    convenience init(line: CTLine, origin: CGPoint, range _: NSRange, bounds: CGRect) {
        self.init(fragments: [.init(kind: .text, line: line, origin: origin, bounds: bounds)], bounds: bounds)
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
