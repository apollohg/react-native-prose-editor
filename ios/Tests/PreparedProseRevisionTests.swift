import XCTest
import UIKit
import CoreText

final class PreparedProseRevisionTests: XCTestCase {
    enum FixtureError: Error { case expected }

    func imageLayout(attachments: [ViewerImageAttachment]) -> PreparedProseLayout {
        PreparedProseLayout(
            key: ProseLayoutKey(
                semanticKey: "image-layout",
                widthPixels: 400,
                themeDigest: "theme",
                nativeFontRevision: 0,
                fontEnvironmentRevision: 0,
                displayScale: 2,
                attachmentRevision: 0,
                generationIdentity: "image-layout",
                semanticGenerationIdentity: "image-layout"
            ),
            size: CGSize(width: 200, height: 1_600),
            blocks: [],
            imageAttachments: attachments,
            retainedBytes: 0
        )
    }

    func flushMain(until condition: () -> Bool) {
        let deadline = Date().addingTimeInterval(1)
        repeat {
            let flushed = expectation(description: "flush main queue")
            DispatchQueue.main.async { flushed.fulfill() }
            wait(for: [flushed], timeout: 1)
        } while !condition() && Date() < deadline
    }

    func imageDataURI() -> String {
        let image = UIGraphicsImageRenderer(size: CGSize(width: 1, height: 1)).image { context in
            UIColor.black.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 1, height: 1))
        }
        return "data:image/png;base64,\(image.pngData()!.base64EncodedString())"
    }

    func foregroundColor(in layout: PreparedProseLayout) throws -> UIColor {
        let line = try XCTUnwrap(layout.blocks.flatMap(\.fragments).compactMap(\.line).first)
        let run = try XCTUnwrap((CTLineGetGlyphRuns(line) as? [CTRun])?.first)
        let attributes = CTRunGetAttributes(run) as? [NSAttributedString.Key: Any]
        let color = try XCTUnwrap(
            attributes?[kCTForegroundColorAttributeName as NSAttributedString.Key]
        )
        return UIColor(cgColor: color as! CGColor)
    }
}
