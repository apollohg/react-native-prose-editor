import CoreText
import UIKit

@objc public protocol PreparedProseDrawingViewInteractionDelegate: AnyObject {
    func preparedProseDrawingView(_ view: PreparedProseDrawingView, didActivateLink href: String, text: String) -> Bool
    func preparedProseDrawingView(_ view: PreparedProseDrawingView, didActivateMention docPos: UInt32, label: String) -> Bool
}

/// A rendering-only view: it consumes already prepared Core Text lines.
@objc(PREPPreparedProseDrawingView)
public final class PreparedProseDrawingView: UIView {
    var imagePixels: [String: UIImage] = [:] {
        didSet {
            PreparedProseInstrumentation.retained(.image, scope: "drawing-\(ObjectIdentifier(self))", bytes: retainedImagePixelsBytesForTesting)
            setNeedsDisplay()
        }
    }
    /// This map owns only mapping/reference overhead. The shared native image
    /// cache is the sole owner charged for a decoded CGImage allocation.
    internal var retainedImagePixelsBytesForTesting: Int {
        Self.retainedImagePixelMappingBytes(entryCount: imagePixels.count)
    }
    internal var preparedSurfaceRetainedBytesForTesting: Int {
        Self.saturatingAdd(
            Self.saturatingAdd(layout?.retainedBytes ?? 0, imageRevisions.retainedPublicationBytesForTesting),
            retainedImagePixelsBytesForTesting
        )
    }
    internal static let imagePixelMapRetainedBytes = 48
    internal static let imagePixelEntryRetainedBytes = 48
    internal static func retainedImagePixelMappingBytes(entryCount: Int) -> Int {
        guard entryCount > 0 else { return 0 }
        return saturatingAdd(
            imagePixelMapRetainedBytes,
            saturatingMultiply(entryCount, imagePixelEntryRetainedBytes)
        )
    }
    @objc public static let imageMetadataDidResolve = Notification.Name("com.apollohg.editor.viewer.imageMetadataDidResolve")
    @objc public static let imageResourceDidFail = Notification.Name("com.apollohg.editor.viewer.imageResourceDidFail")
    private lazy var imagePipeline = ViewerImagePipeline(policy: .default)
    private var imageRevisions = ViewerAttachmentRevisionState()
    private var imageGeneration = ""
    private var imageConfiguration: (enabled: Bool, policy: ImageLoadingPolicy) = (false, .default)
    var layout: PreparedProseLayout? {
        didSet {
            guard oldValue !== layout else { return }
            PreparedProseInstrumentation.retained(.sidecars, scope: "drawing-\(ObjectIdentifier(self))", bytes: imageRevisions.retainedPublicationBytesForTesting)
            invalidateAccessibilityNodes()
            setNeedsDisplay()
        }
    }

    @objc public func install(layout: PreparedProseLayout?) { self.layout = layout }

    @objc(configureImagesWithGeneration:imagesEnabled:policyJSON:)
    public func configureImages(generation: String, imagesEnabled: Bool, policyJSON: String?) {
        imageGeneration = generation
        imageConfiguration = (imagesEnabled, ImageLoadingPolicy.from(json: policyJSON))
        imagePipeline.onPixels = { [weak self] attachment, image in self?.imagePixels[attachment.id] = image }
        imagePipeline.onIntrinsicMetadata = { [weak self] attachment, size in
            guard let self,
                  self.imagePipeline.acceptsCompletion(generation: generation),
                  self.imageRevisions.recordIntrinsicSize(size, for: attachment.id, ordinal: attachment.ordinal, declaredSize: attachment.declaredSize)
            else { return }
            NotificationCenter.default.post(
                name: Self.imageMetadataDidResolve,
                object: self,
                userInfo: ["generation": generation, "revision": self.imageRevisions.revision]
            )
        }
        imagePipeline.onResourceFailure = { [weak self] attachment in
            guard let self,
                  self.imagePipeline.acceptsCompletion(generation: generation),
                  self.imageRevisions.recordResourceFailure(for: attachment.ordinal)
            else { return }
            NotificationCenter.default.post(
                name: Self.imageResourceDidFail,
                object: self,
                userInfo: ["generation": generation, "attachment": attachment.id]
            )
        }
        imagePipeline.begin(
            generation: imageGeneration,
            imagesEnabled: imageConfiguration.enabled,
            policy: imageConfiguration.policy
        )
        imageRevisions.admit(attachmentCount: layout?.imageAttachments.count ?? 0)
    }

    /// Phase one of Fabric setup: semantic props/state have been accepted but
    /// no mounted artifact exists yet. This clears active intrinsic fallback
    /// before measurement/preparation and phase two only binds ordinals.
    @objc(beginSemanticImageGeneration:)
    public func beginSemanticImageGeneration(_ generation: String) {
        guard imageRevisions.beginSemanticGeneration(generation) else { return }
        imageGeneration = ""
        imagePipeline.cancel()
        imagePixels = [:]
    }

    /// Fabric has already reset this sidecar during Yoga preparation. Mount
    /// only transfers the stable surface owner; it must not reopen metadata or
    /// error publication by resetting a second time.
    @objc(bindFabricAttachmentStateSurfaceId:componentTag:leaseHandle:)
    public func bindFabricAttachmentState(surfaceId: Int64, componentTag: Int64, leaseHandle: UInt64) {
        guard let state = FabricAttachmentSidecars.state(
            for: .init(surfaceId: surfaceId, componentTag: componentTag),
            leaseHandle: leaseHandle
        ) else { return }
        imageRevisions = state
    }

    @objc public func updateConfiguredImagesForVisibleWindow() {
        guard let layout, let window, !isHidden, alpha > 0 else { return }
        let visible = convert(window.bounds, from: window).intersection(bounds)
        guard visible.origin.x.isFinite, visible.origin.y.isFinite,
              visible.size.width.isFinite, visible.size.height.isFinite,
              !visible.isNull, !visible.isEmpty else { return }
        imagePipeline.updateVisibleRect(visible, attachments: layout.imageAttachments)
    }

    @objc public func cancelConfiguredImages() {
        imageGeneration = ""
        imagePipeline.cancel()
        imagePixels = [:]
    }

    /// A semantic prop replacement starts a new source-qualified publication
    /// generation. Attachment-revision replacements deliberately do not call
    /// this: they are the one reflow being deduplicated.
    @objc public func resetIntrinsicImagePublication() {
        imageRevisions.reset()
    }

    @objc public var errorDomain: String? { layout?.error?.domain }
    @objc public var errorCode: String? { layout?.error?.code }
    @objc public var errorMessage: String? { layout?.error?.message }
    /// The owner chooses its delivery channel (UIKit delegate or Fabric event).
    var onActivateInteraction: ((PreparedProseInteraction) -> Bool)?
    @objc public weak var interactionDelegate: PreparedProseDrawingViewInteractionDelegate?
    @objc public var linkInteractionsEnabled = true {
        didSet {
            guard oldValue != linkInteractionsEnabled else { return }
            invalidateAccessibilityNodes()
        }
    }
    private var accessibilityElementsByIndex: [Int: PreparedProseDrawingAccessibilityElement] = [:]
    internal var materializedAccessibilityElementCountForTesting: Int { accessibilityElementsByIndex.count }

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

    internal static func pixelAllocationBytes(for image: UIImage) -> Int {
        if let cgImage = image.cgImage {
            return saturatingMultiply(cgImage.bytesPerRow, cgImage.height)
        }
        let width = image.size.width * image.scale
        let height = image.size.height * image.scale
        guard width.isFinite, height.isFinite, width > 0, height > 0 else { return 0 }
        let pixels = min(Double(Int.max), width.rounded(.up) * height.rounded(.up))
        return saturatingMultiply(Int(pixels), 4)
    }

    internal static func saturatingAdd(_ left: Int, _ right: Int) -> Int {
        guard left > Int.max - right else { return Int.max }
        return left + right
    }

    internal static func saturatingMultiply(_ left: Int, _ right: Int) -> Int {
        guard left > 0, right > 0 else { return 0 }
        guard left <= Int.max / right else { return Int.max }
        return left * right
    }

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
        _ = activate(interaction)
    }

    @discardableResult
    private func activate(_ interaction: PreparedProseInteraction) -> Bool {
        if let onActivateInteraction { return onActivateInteraction(interaction) }
        switch interaction.kind {
        case .link:
            guard linkInteractionsEnabled, let href = interaction.href else { return false }
            return interactionDelegate?.preparedProseDrawingView(self, didActivateLink: href, text: interaction.visibleText) ?? false
        case .mention:
            guard let docPos = interaction.docPos else { return false }
            return interactionDelegate?.preparedProseDrawingView(self, didActivateMention: docPos, label: interaction.label) ?? false
        }
    }

    public override func accessibilityElementCount() -> Int { accessibilityNodes.count }

    public override func accessibilityElement(at index: Int) -> Any? {
        guard accessibilityNodes.indices.contains(index) else { return nil }
        if let existing = accessibilityElementsByIndex[index] { return existing }
        let element = PreparedProseDrawingAccessibilityElement(container: self, index: index)
        accessibilityElementsByIndex[index] = element
        return element
    }

    public override func index(ofAccessibilityElement element: Any) -> Int {
        guard let element = element as? PreparedProseDrawingAccessibilityElement,
              element.drawingView === self
        else { return NSNotFound }
        return element.index
    }

    private var accessibilityNodes: [PreparedProseAccessibilityNode] {
        layout?.accessibilityNodes.filter { linkInteractionsEnabled || $0.role != .link } ?? []
    }

    fileprivate func accessibilityNode(at index: Int) -> PreparedProseAccessibilityNode? {
        accessibilityNodes[safe: index]
    }

    fileprivate func accessibilityFrame(for node: PreparedProseAccessibilityNode) -> CGRect {
        UIAccessibility.convertToScreenCoordinates(node.bounds, in: self)
    }

    fileprivate func activateAccessibilityNode(at index: Int) -> Bool {
        guard let node = accessibilityNode(at: index),
              let interaction = layout?.interactions[safe: node.interactionIndex]
        else { return false }
        return activate(interaction)
    }

    private func invalidateAccessibilityNodes() {
        accessibilityElementsByIndex.removeAll(keepingCapacity: true)
        UIAccessibility.post(notification: .layoutChanged, argument: nil)
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
        let drawStarted = PreparedProseInstrumentation.now()
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
        var visibleBlockCount = 0
        for index in lower..<blocks.count {
            let block = blocks[index]
            guard block.bounds.minY <= rect.maxY else { break }
            visibleFragments.append(contentsOf: block.fragments)
            visibleBlockCount += 1
        }
        // Keep paint phases global across the visible range: a nested code
        // background must never cover a quote border from an adjacent block.
        for fragment in visibleFragments { drawBackground(fragment, in: context) }
        for fragment in visibleFragments { drawBorderOrRule(fragment, in: context, scale: scale) }
        for fragment in visibleFragments { drawForeground(fragment, in: context) }
        context.restoreGState()
        PreparedProseInstrumentation.drew(drawStarted, visibleBlocks: visibleBlockCount)
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
        guard fragment.kind == .background || fragment.kind == .atom || fragment.kind == .image else { return }
        let rect = drawingRect(for: fragment)
        context.setFillColor(fragment.color ?? UIColor.clear.cgColor)
        context.addPath(UIBezierPath(roundedRect: rect, cornerRadius: fragment.cornerRadius).cgPath)
        context.drawPath(using: .fill)
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
            context.addPath(
                UIBezierPath(
                    roundedRect: rect.insetBy(dx: inset, dy: inset),
                    cornerRadius: max(0, fragment.cornerRadius - inset)
                ).cgPath
            )
            context.drawPath(using: .stroke)
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
        case .image:
            guard let layout,
                  let attachment = layout.imageAttachments.first(where: { $0.bounds == fragment.bounds }),
                  let image = imagePixels[attachment.id] else { return }
            image.draw(in: drawingRect(for: fragment))
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

private final class PreparedProseDrawingAccessibilityElement: UIAccessibilityElement {
    weak var drawingView: PreparedProseDrawingView?
    let index: Int

    init(container: PreparedProseDrawingView, index: Int) {
        drawingView = container
        self.index = index
        super.init(accessibilityContainer: container)
    }

    private var node: PreparedProseAccessibilityNode? { drawingView?.accessibilityNode(at: index) }
    override var accessibilityLabel: String? {
        get { node?.label }
        set { }
    }
    override var accessibilityTraits: UIAccessibilityTraits {
        get {
            guard let node else { return .none }
            return node.role == .link ? .link : .button
        }
        set { }
    }
    override var accessibilityFrame: CGRect {
        get {
            guard let drawingView, let node else { return .zero }
            return drawingView.accessibilityFrame(for: node)
        }
        set { }
    }
    override func accessibilityActivate() -> Bool { drawingView?.activateAccessibilityNode(at: index) ?? false }
}

private extension Array {
    subscript(safe index: Int) -> Element? { indices.contains(index) ? self[index] : nil }
}
