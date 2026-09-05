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
    /// Immutable semantic identity used only by generation-scoped diagnostics.
    /// It intentionally excludes layout, attachment, and font replacements.
    let semanticGenerationIdentity: String

    init(
        semanticKey: String,
        widthPixels: Int,
        themeDigest: String,
        nativeFontRevision: UInt64,
        fontEnvironmentRevision: UInt64,
        displayScale: CGFloat,
        attachmentRevision: UInt64,
        generationIdentity: String,
        semanticGenerationIdentity: String
    ) {
        self.semanticKey = semanticKey
        self.widthPixels = widthPixels
        self.themeDigest = themeDigest
        self.nativeFontRevision = nativeFontRevision
        self.fontEnvironmentRevision = fontEnvironmentRevision
        self.displayScaleBits = Double(displayScale).bitPattern
        self.attachmentRevision = attachmentRevision
        self.generationIdentity = generationIdentity
        self.semanticGenerationIdentity = semanticGenerationIdentity
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
    /// Opaque Fabric-state incarnation handle. A recycled surface may later
    /// reuse the same component tag and generation identity; only this exact
    /// handle may consume or retire its Yoga handoff.
    let leaseHandle: UInt64
}

struct FabricGenerationToken: Hashable {
    let surface: FabricSurfaceToken
    let generationIdentity: String
    /// This is carried by custom Fabric state from the shadow-node measurement
    /// to the component view. It is never inferred from registry state.
    let leaseHandle: UInt64

    init(
        surface: FabricSurfaceToken,
        generationIdentity: String,
        leaseHandle: UInt64 = 1
    ) {
        self.surface = surface
        self.generationIdentity = generationIdentity
        self.leaseHandle = leaseHandle
    }
}

/// State-family identity across all semantic, attachment, font, and width
/// revisions. The registry permits at most one committed generation for it.
struct FabricLeaseOwner: Hashable {
    let surface: FabricSurfaceToken
    let leaseHandle: UInt64
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
    case strike
    case image
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
    let borderColor: CGColor?
    let cornerRadius: CGFloat
    let strokeWidth: CGFloat
    let padding: UIEdgeInsets
    let label: String?
    let checked: Bool

    init(
        kind: PreparedProseFragmentKind,
        line: CTLine? = nil,
        origin: CGPoint = .zero,
        bounds: CGRect,
        color: CGColor? = nil,
        borderColor: CGColor? = nil,
        cornerRadius: CGFloat = 0,
        strokeWidth: CGFloat = 0,
        padding: UIEdgeInsets = .zero,
        label: String? = nil,
        checked: Bool = false
    ) {
        self.kind = kind
        self.line = line
        self.origin = origin
        self.bounds = bounds
        self.color = color
        self.borderColor = borderColor
        self.cornerRadius = cornerRadius
        self.strokeWidth = strokeWidth
        self.padding = padding
        self.label = label
        self.checked = checked
    }

    /// Core Text retains opaque shaping data outside Swift's object graph.
    /// Charge a conservative fixed payload per line/fragment (including every
    /// prepared strike rectangle) plus the visible label so narrow documents
    /// cannot evade the prepared-layout cache.
    var estimatedRetainedBytes: Int {
        let lineBytes = line == nil ? 0 : 768
        let labelBytes = (label?.utf8.count ?? 0).layoutSaturatingMultiply(2)
        return 192 + lineBytes + labelBytes + (kind == .atom ? 192 : 0)
    }
}

/// A vertical-culling unit, sorted by its top edge. It owns every paint
/// operation needed for one semantic block so draw(_:) never shapes text.
struct PreparedProseAtomSlot {
    let nodeType: String
    let docPos: UInt32
    let attrsJSON: String
    let bounds: CGRect

    var estimatedRetainedBytes: Int { 128 + nodeType.utf8.count * 2 + attrsJSON.utf8.count * 2 }
}

final class PreparedProseBlock {
    let atomSlot: PreparedProseAtomSlot?
    let fragments: [PreparedProseFragment]
    let bounds: CGRect

    init(fragments: [PreparedProseFragment], bounds: CGRect, atomSlot: PreparedProseAtomSlot? = nil) {
        self.atomSlot = atomSlot
        self.fragments = fragments
        self.bounds = bounds
    }

    var estimatedRetainedBytes: Int {
        160 + fragments.reduce(0) { $0 + $1.estimatedRetainedBytes } + (atomSlot?.estimatedRetainedBytes ?? 0)
    }

    /// Compatibility initializer retained for test seams.
    convenience init(line: CTLine, origin: CGPoint, range _: NSRange, bounds: CGRect) {
        self.init(fragments: [.init(kind: .text, line: line, origin: origin, bounds: bounds)], bounds: bounds)
    }
}

/// Immutable hit payload. Rectangles are in artifact coordinates, ordered by
/// visual line, and never require Core Text work after publication.
struct PreparedProseInteraction: Hashable {
    enum Kind: Hashable { case link, mention }

    let kind: Kind
    let rects: [CGRect]
    let href: String?
    let visibleText: String
    let docPos: UInt32?
    let label: String
    let attrsJSON: String?

    var estimatedRetainedBytes: Int {
        144 + rects.count * 64 + (href?.utf8.count ?? 0) * 2 + visibleText.utf8.count * 2
            + label.utf8.count * 2 + (attrsJSON?.utf8.count ?? 0)
    }
}

/// A lightweight virtual-node descriptor. UIKit elements are deliberately
/// created only when VoiceOver asks the container for an index.
struct PreparedProseAccessibilityNode: Hashable {
    enum Role: Hashable { case text, heading, link, mention, image, separator }

    let interactionIndex: Int?
    let role: Role
    let label: String
    let rects: [CGRect]

    init(interactionIndex: Int?, role: Role, label: String, bounds: CGRect) {
        self.init(interactionIndex: interactionIndex, role: role, label: label, rects: [bounds])
    }

    init(interactionIndex: Int?, role: Role, label: String, rects: [CGRect]) {
        self.interactionIndex = interactionIndex
        self.role = role
        self.label = label
        self.rects = rects
    }

    var bounds: CGRect { rects.reduce(.null) { $0.union($1) } }

    var estimatedRetainedBytes: Int { 96 + rects.count * 64 + label.utf8.count * 2 }
}

public final class PreparedProseLayout: NSObject {
    let key: ProseLayoutKey
    let size: CGSize
    let blocks: [PreparedProseBlock]
    let interactions: [PreparedProseInteraction]
    let accessibilityNodes: [PreparedProseAccessibilityNode]
    let imageAttachments: [ViewerImageAttachment]
    let retainedBytes: Int
    let error: ProseViewerError?

    init(
        key: ProseLayoutKey,
        size: CGSize,
        blocks: [PreparedProseBlock],
        interactions: [PreparedProseInteraction] = [],
        accessibilityNodes: [PreparedProseAccessibilityNode] = [],
        imageAttachments: [ViewerImageAttachment] = [],
        retainedBytes: Int,
        error: ProseViewerError? = nil
    ) {
        self.key = key
        self.size = size
        self.blocks = blocks
        self.interactions = interactions
        self.accessibilityNodes = accessibilityNodes
        self.imageAttachments = imageAttachments
        self.retainedBytes = retainedBytes
        self.error = error
        super.init()
    }

    static func error(key: ProseLayoutKey, width: CGFloat, error: ProseViewerError) -> PreparedProseLayout {
        PreparedProseLayout(key: key, size: CGSize(width: width, height: 0), blocks: [], retainedBytes: 0, error: error)
    }
}

private extension Int {
    func layoutSaturatingMultiply(_ other: Int) -> Int {
        let result = multipliedReportingOverflow(by: other)
        return result.overflow ? Int.max : result.partialValue
    }
}
