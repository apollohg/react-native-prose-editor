import UIKit
import ImageIO
import CryptoKit

struct BlockContext {
    let nodeType: String
    let depth: UInt8
    var listContext: [String: Any]?
    var topLevelChildIndex: Int? = nil
    var listMarkerContext: [String: Any]? = nil
    var markerPending: Bool = false
    var styleStart: Int = 0
    var language: String? = nil
}

// MARK: - HorizontalRuleAttachment

/// NSTextAttachment subclass that draws a horizontal separator line.
///
/// The attachment renders as a thin line across the available width with
/// vertical padding. Used for `horizontalRule` void block elements.
final class HorizontalRuleAttachment: NSTextAttachment {

    var styleBox: EditorStyleBox?
    var lineColor: UIColor = .separator
    var lineHeight: CGFloat = LayoutConstants.horizontalRuleHeight
    var verticalPadding: CGFloat = LayoutConstants.horizontalRuleVerticalPadding

    override func attachmentBounds(
        for textContainer: NSTextContainer?,
        proposedLineFragment lineFrag: CGRect,
        glyphPosition position: CGPoint,
        characterIndex charIndex: Int
    ) -> CGRect {
        let totalHeight = lineHeight + (verticalPadding * 2) + (styleBox?.inset.top ?? 0) + (styleBox?.inset.bottom ?? 0)
        return CGRect(
            x: 0,
            y: 0,
            width: lineFrag.width,
            height: totalHeight
        )
    }

    override func image(
        forBounds imageBounds: CGRect,
        textContainer: NSTextContainer?,
        characterIndex charIndex: Int
    ) -> UIImage? {
        let renderer = UIGraphicsImageRenderer(bounds: imageBounds)
        return renderer.image { context in
            if let styleBox { styleBox.draw(in: imageBounds, context: context.cgContext); return }
            lineColor.setFill()
            let lineY = imageBounds.midY - (lineHeight / 2)
            let lineRect = CGRect(
                x: imageBounds.origin.x,
                y: lineY,
                width: imageBounds.width,
                height: lineHeight
            )
            context.fill(lineRect)
        }
    }
}

final class AtomBlockAttachment: NSTextAttachment {
    let atomKey: String
    let nodeType: String
    let docPos: UInt32
    var reservedHeight: CGFloat

    init(atomKey: String, nodeType: String, docPos: UInt32, reservedHeight: CGFloat) {
        self.atomKey = atomKey
        self.nodeType = nodeType
        self.docPos = docPos
        self.reservedHeight = reservedHeight
        super.init(data: nil, ofType: nil)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func attachmentBounds(
        for textContainer: NSTextContainer?,
        proposedLineFragment lineFrag: CGRect,
        glyphPosition position: CGPoint,
        characterIndex charIndex: Int
    ) -> CGRect {
        CGRect(x: 0, y: 0, width: lineFrag.width, height: reservedHeight)
    }

    override func image(
        forBounds imageBounds: CGRect,
        textContainer: NSTextContainer?,
        characterIndex charIndex: Int
    ) -> UIImage? {
        nil
    }
}

final class BlockImageAttachment: NSTextAttachment {
    var styleBox: EditorStyleBox?
    let source: String
    let placeholderTint: UIColor
    private weak var loadOwner: RenderImageLoadOwner?
    private var loadReceipt: RenderImageLoadOwner.ImageLoadReceipt?
    var preferredWidth: CGFloat?
    var preferredHeight: CGFloat?
    private var loadedImage: UIImage?

    init(
        source: String,
        placeholderTint: UIColor,
        preferredWidth: CGFloat?,
        preferredHeight: CGFloat?
    ) {
        self.source = source
        self.placeholderTint = placeholderTint
        self.preferredWidth = preferredWidth
        self.preferredHeight = preferredHeight
        self.loadOwner = RenderImageLoadOwner.current
        super.init(data: nil, ofType: nil)
        loadImageIfNeeded()
    }

    required init?(coder: NSCoder) {
        return nil
    }

    deinit {
        loadReceipt?.cancel()
    }

    func setPreferredSize(width: CGFloat, height: CGFloat) {
        preferredWidth = width
        preferredHeight = height
    }

    func previewImage() -> UIImage? {
        loadedImage ?? image
    }

    override func attachmentBounds(
        for textContainer: NSTextContainer?,
        proposedLineFragment lineFrag: CGRect,
        glyphPosition position: CGPoint,
        characterIndex charIndex: Int
    ) -> CGRect {
        let lineFragmentWidth = lineFrag.width.isFinite ? lineFrag.width : 0
        let containerWidth = textContainer.map {
            max(0, $0.size.width - ($0.lineFragmentPadding * 2))
        } ?? 0
        let widthCandidates = [lineFragmentWidth, containerWidth].filter { $0.isFinite && $0 > 0 }
        let inset = styleBox?.inset ?? .zero
        let maxWidth = styleBox == nil ? max(160, widthCandidates.min() ?? 160) : max(1, (widthCandidates.min() ?? 160) - inset.left - inset.right)
        let fallbackAspectRatio = loadedImage.flatMap { image -> CGFloat? in
            let imageSize = image.size
            guard imageSize.width > 0, imageSize.height > 0 else { return nil }
            return imageSize.height / imageSize.width
        } ?? 0.56

        var resolvedWidth = preferredWidth
        var resolvedHeight = preferredHeight

        if resolvedWidth == nil, resolvedHeight == nil, let loadedImage {
            let imageSize = loadedImage.size
            if imageSize.width > 0, imageSize.height > 0 {
                resolvedWidth = imageSize.width
                resolvedHeight = imageSize.height
            }
        } else if resolvedWidth == nil, let resolvedHeight {
            resolvedWidth = resolvedHeight / fallbackAspectRatio
        } else if resolvedHeight == nil, let resolvedWidth {
            resolvedHeight = resolvedWidth * fallbackAspectRatio
        }

        let width = max(1, resolvedWidth ?? maxWidth)
        let height = max(1, resolvedHeight ?? min(180, maxWidth * fallbackAspectRatio))
        let scale = min(1, maxWidth / width)
        return CGRect(x: 0, y: 0, width: width * scale + inset.left + inset.right, height: height * scale + inset.top + inset.bottom)
    }

    override func image(
        forBounds imageBounds: CGRect,
        textContainer: NSTextContainer?,
        characterIndex charIndex: Int
    ) -> UIImage? {
        if let styleBox {
            return UIGraphicsImageRenderer(bounds: imageBounds).image { renderer in
                styleBox.draw(in: imageBounds, context: renderer.cgContext)
                renderer.cgContext.addPath(styleBox.path(in: imageBounds, inner: true).cgPath)
                renderer.cgContext.clip()
                renderer.cgContext.clip(to: imageBounds.inset(by: styleBox.inset))
                if let loadedImage {
                    loadedImage.draw(in: styleBox.imageRect(loadedImage.size, in: imageBounds))
                } else {
                    let iconBounds = imageBounds.insetBy(dx: imageBounds.width * 0.35, dy: imageBounds.height * 0.35)
                    UIImage(systemName: "photo")?.withTintColor(placeholderTint, renderingMode: .alwaysOriginal).draw(in: iconBounds)
                }
            }
        }
        if let loadedImage { return loadedImage }

        let renderer = UIGraphicsImageRenderer(bounds: imageBounds)
        return renderer.image { _ in
            let path = UIBezierPath(roundedRect: imageBounds, cornerRadius: 12)
            UIColor.secondarySystemFill.setFill()
            path.fill()

            let iconSize = min(imageBounds.width, imageBounds.height) * 0.28
            let iconOrigin = CGPoint(
                x: imageBounds.midX - (iconSize / 2),
                y: imageBounds.midY - (iconSize / 2)
            )
            let iconRect = CGRect(origin: iconOrigin, size: CGSize(width: iconSize, height: iconSize))

            if #available(iOS 13.0, *) {
                let config = UIImage.SymbolConfiguration(pointSize: iconSize, weight: .medium)
                let icon = UIImage(systemName: "photo", withConfiguration: config)?
                    .withTintColor(placeholderTint.withAlphaComponent(0.7), renderingMode: .alwaysOriginal)
                icon?.draw(in: iconRect)
            }
        }
    }

    private func loadImageIfNeeded() {
        guard let loadOwner else { return }
        let cacheKey = RenderImageCache.key(source: source, policy: loadOwner.policy)
        if let cached = RenderImageCache.cache.image(forKey: cacheKey) {
            loadedImage = cached
            image = cached
            return
        }
        loadReceipt = loadOwner.startImageLoad(source: source) { [weak self] image in
            guard let self,
                  let image
            else {
                return
            }
            self.loadedImage = image
            self.image = image
            self.loadReceipt = nil
            NotificationCenter.default.post(name: .editorImageAttachmentDidLoad, object: self)
        }
    }
}
