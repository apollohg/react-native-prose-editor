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

    override func draw(_ rect: CGRect) {
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
            context.textPosition = CGPoint(x: block.origin.x, y: layout.size.height - block.origin.y)
            CTLineDraw(block.line, context)
        }
        context.restoreGState()
    }
}
