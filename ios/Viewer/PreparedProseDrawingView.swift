import CoreText
import UIKit

@objc public protocol PreparedProseDrawingViewInteractionDelegate: AnyObject {
    func preparedProseDrawingView(_ view: PreparedProseDrawingView, didActivateLink href: String, text: String)
    func preparedProseDrawingView(_ view: PreparedProseDrawingView, didActivateMention docPos: UInt32, label: String)
}

/// A rendering-only view: it consumes already prepared Core Text lines.
@objc(PREPPreparedProseDrawingView)
public final class PreparedProseDrawingView: UIView {
    var layout: PreparedProseLayout? {
        didSet { setNeedsDisplay() }
    }

    @objc public func install(layout: PreparedProseLayout?) { self.layout = layout }

    @objc var errorDomain: String? { layout?.error?.domain }
    @objc var errorCode: String? { layout?.error?.code }
    @objc var errorMessage: String? { layout?.error?.message }
    /// The owner chooses its delivery channel (UIKit delegate or Fabric event).
    var onActivateInteraction: ((PreparedProseInteraction) -> Void)?
    @objc public weak var interactionDelegate: PreparedProseDrawingViewInteractionDelegate?
    var linkInteractionsEnabled = true

    private lazy var tapRecognizer: UITapGestureRecognizer = {
        let recognizer = UITapGestureRecognizer(target: self, action: #selector(handleTap(_:)))
        recognizer.cancelsTouchesInView = false
        return recognizer
    }()

    public override init(frame: CGRect) {
        super.init(frame: frame)
        isAccessibilityElement = false
        addGestureRecognizer(tapRecognizer)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("PreparedProseDrawingView does not support NSCoder") }

    func interaction(at point: CGPoint) -> PreparedProseInteraction? {
        guard let layout else { return nil }
        return layout.interactions.first { interaction in
            (linkInteractionsEnabled || interaction.kind != .link) && interaction.rects.contains { $0.contains(point) }
        }
    }

    @objc private func handleTap(_ recognizer: UITapGestureRecognizer) {
        guard recognizer.state == .ended,
              let interaction = interaction(at: recognizer.location(in: self))
        else { return }
        onActivateInteraction?(interaction)
        switch interaction.kind {
        case .link:
            if let href = interaction.href { interactionDelegate?.preparedProseDrawingView(self, didActivateLink: href, text: interaction.visibleText) }
        case .mention:
            if let docPos = interaction.docPos { interactionDelegate?.preparedProseDrawingView(self, didActivateMention: docPos, label: interaction.label) }
        }
    }

    /// Converts an artifact-top baseline to the flipped Core Graphics coordinate system.
    /// The view bounds, not the intrinsic artifact height, defines the flip origin.
    static func textPosition(
        baselineFromArtifactTop: CGFloat,
        in bounds: CGRect,
        artifactHeight _: CGFloat
    ) -> CGPoint {
        CGPoint(x: 0, y: bounds.height - baselineFromArtifactTop)
    }

    public override func draw(_ rect: CGRect) {
        guard let layout, let context = UIGraphicsGetCurrentContext(), !layout.blocks.isEmpty else { return }
        let blocks = layout.blocks
        var lower = 0
        var upper = blocks.count
        while lower < upper {
            let middle = (lower + upper) / 2
            if blocks[middle].bounds.maxY < rect.minY { lower = middle + 1 } else { upper = middle }
        }

        context.saveGState()
        context.translateBy(x: 0, y: bounds.height)
        context.scaleBy(x: 1, y: -1)
        let scale = CGFloat(Double(bitPattern: layout.key.displayScaleBits))
        var visibleFragments: [PreparedProseFragment] = []
        for index in lower..<blocks.count {
            let block = blocks[index]
            guard block.bounds.minY <= rect.maxY else { break }
            visibleFragments.append(contentsOf: block.fragments)
        }
        // Keep paint phases global across the visible range: a nested code
        // background must never cover a quote border from an adjacent block.
        for fragment in visibleFragments { drawBackground(fragment, in: context) }
        for fragment in visibleFragments { drawBorderOrRule(fragment, in: context, scale: scale) }
        for fragment in visibleFragments { drawForeground(fragment, in: context) }
        context.restoreGState()
    }

    private func drawingRect(for fragment: PreparedProseFragment) -> CGRect {
        CGRect(
            x: fragment.bounds.minX,
            y: bounds.height - fragment.bounds.maxY,
            width: fragment.bounds.width,
            height: fragment.bounds.height
        )
    }

    private func drawBackground(_ fragment: PreparedProseFragment, in context: CGContext) {
        guard fragment.kind == .background || fragment.kind == .atom else { return }
        let rect = drawingRect(for: fragment)
        context.setFillColor(fragment.color ?? UIColor.clear.cgColor)
        context.fill(UIBezierPath(roundedRect: rect, cornerRadius: fragment.cornerRadius).cgPath)
    }

    private func drawBorderOrRule(_ fragment: PreparedProseFragment, in context: CGContext, scale: CGFloat) {
        let rect = drawingRect(for: fragment)
        switch fragment.kind {
        case .border:
            context.setFillColor(fragment.color ?? UIColor.clear.cgColor)
            context.fill(rect)
        case .rule:
            let unit = scale.isFinite && scale > 0 ? 1 / scale : 1
            let alignedY = (rect.minY / unit).rounded() * unit
            context.setFillColor(fragment.color ?? UIColor.clear.cgColor)
            context.fill(CGRect(x: rect.minX, y: alignedY, width: rect.width, height: max(unit, rect.height)))
        case .atom where fragment.strokeWidth > 0:
            context.setStrokeColor(fragment.borderColor ?? fragment.color ?? UIColor.clear.cgColor)
            context.setLineWidth(fragment.strokeWidth)
            let inset = fragment.strokeWidth / 2
            context.stroke(UIBezierPath(roundedRect: rect.insetBy(dx: inset, dy: inset), cornerRadius: max(0, fragment.cornerRadius - inset)).cgPath)
        default:
            break
        }
    }

    private func drawForeground(_ fragment: PreparedProseFragment, in context: CGContext) {
        let rect = CGRect(
            x: fragment.bounds.minX,
            y: bounds.height - fragment.bounds.maxY,
            width: fragment.bounds.width,
            height: fragment.bounds.height
        )
        switch fragment.kind {
        case .text:
            guard let line = fragment.line else { return }
            context.textPosition = CGPoint(x: fragment.origin.x, y: bounds.height - fragment.origin.y)
            CTLineDraw(line, context)
        case .atom:
            guard let line = fragment.line else { return }
            context.textPosition = CGPoint(x: fragment.origin.x, y: bounds.height - fragment.origin.y)
            CTLineDraw(line, context)
        case .marker:
            if let line = fragment.line {
                context.textPosition = CGPoint(x: fragment.origin.x, y: bounds.height - fragment.origin.y)
                CTLineDraw(line, context)
            } else {
                drawTaskMarker(in: rect, checked: fragment.checked, color: UIColor(cgColor: fragment.color ?? UIColor.label.cgColor))
            }
        case .strike:
            context.setFillColor(fragment.color ?? UIColor.clear.cgColor)
            context.fill(rect)
        case .background, .border, .rule:
            break
        }
    }

    private func drawTaskMarker(in rect: CGRect, checked: Bool, color: UIColor) {
        let inset = max(1, rect.height * 0.2)
        let box = CGRect(x: rect.minX + inset, y: rect.minY + inset, width: rect.height - inset * 2, height: rect.height - inset * 2)
        let path = UIBezierPath(roundedRect: box, cornerRadius: box.width * 0.2)
        color.setStroke()
        path.lineWidth = max(1, box.width * 0.1)
        path.stroke()
        guard checked else { return }
        let check = UIBezierPath()
        check.move(to: CGPoint(x: box.minX + box.width * 0.2, y: box.midY))
        check.addLine(to: CGPoint(x: box.minX + box.width * 0.43, y: box.maxY - box.height * 0.2))
        check.addLine(to: CGPoint(x: box.maxX - box.width * 0.16, y: box.minY + box.height * 0.2))
        check.lineCapStyle = .round
        check.lineJoinStyle = .round
        check.lineWidth = max(1.4, box.width * 0.12)
        color.setStroke()
        check.stroke()
    }
}
