import CoreText
import UIKit

/// Performs the width-dependent, immutable Core Text preparation step.
final class CoreTextProseLayoutEngine {
    private let font: UIFont
    private let textColor: UIColor

    init(font: UIFont = .systemFont(ofSize: 17), textColor: UIColor = .label) {
        self.font = font
        self.textColor = textColor
    }

    func prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        widthPoints: CGFloat,
        displayScale: CGFloat
    ) throws -> PreparedProseLayout {
        guard widthPoints.isFinite, widthPoints > 0, displayScale.isFinite, displayScale > 0 else {
            return .error(
                key: key,
                width: max(0, widthPoints.isFinite ? widthPoints : 0),
                error: .hostContract(message: "A finite positive width is required for prose measurement.")
            )
        }
        if document.isEmpty {
            return PreparedProseLayout(
                key: key,
                size: CGSize(width: widthPoints, height: 0),
                blocks: [],
                retainedBytes: document.retainedBytes
            )
        }

        var blocks: [PreparedProseBlock] = []
        var cursorY: CGFloat = 0
        var estimatedBytes = document.retainedBytes
        for paragraph in document.paragraphs {
            let attributed = makeAttributedString(paragraph.text.isEmpty ? "\u{200B}" : paragraph.text)
            let typesetter = CTTypesetterCreateWithAttributedString(attributed)
            let textLength = attributed.length
            var location = 0
            while location < textLength {
                let count = max(1, CTTypesetterSuggestLineBreak(typesetter, location, widthPoints))
                let line = CTTypesetterCreateLine(typesetter, CFRange(location: location, length: count))
                var ascent: CGFloat = 0
                var descent: CGFloat = 0
                var leading: CGFloat = 0
                let lineWidth = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
                let lineHeight = ascent + descent + leading
                let bounds = CGRect(x: 0, y: cursorY, width: min(widthPoints, lineWidth), height: lineHeight)
                blocks.append(
                    PreparedProseBlock(
                        line: line,
                        origin: CGPoint(x: 0, y: cursorY + ascent),
                        range: NSRange(location: location, length: count),
                        bounds: bounds
                    )
                )
                estimatedBytes += 256 + count * MemoryLayout<UInt16>.size
                cursorY += lineHeight
                location += count
            }
        }
        let pixelHeight = ceil(cursorY * displayScale)
        return PreparedProseLayout(
            key: key,
            size: CGSize(width: widthPoints, height: pixelHeight / displayScale),
            blocks: blocks,
            retainedBytes: estimatedBytes
        )
    }

    private func makeAttributedString(_ text: String) -> NSAttributedString {
        NSAttributedString(
            string: text,
            attributes: [
                kCTFontAttributeName as NSAttributedString.Key: CTFontCreateWithName(font.fontName as CFString, font.pointSize, nil),
                kCTForegroundColorAttributeName as NSAttributedString.Key: textColor.cgColor,
            ]
        )
    }
}
