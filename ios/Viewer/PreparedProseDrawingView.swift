import CoreText
import UIKit

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
        for index in lower..<blocks.count {
            let block = blocks[index]
            guard block.bounds.minY <= rect.maxY else { break }
            let textPosition = Self.textPosition(
                baselineFromArtifactTop: block.origin.y,
                in: bounds,
                artifactHeight: layout.size.height
            )
            context.textPosition = CGPoint(x: block.origin.x, y: textPosition.y)
            CTLineDraw(block.line, context)
        }
        context.restoreGState()
    }
}
