import UIKit

/// A mention activated from an embedded prose viewer.
public struct ProseViewerMention {
    public let docPos: UInt32
    public let label: String
    public let attrs: [String: Any]
}

/// Interaction callbacks for an embedded prose viewer.
public protocol ProseViewerInteractionDelegate: AnyObject {
    func proseViewer(_ view: ProseViewerView, didTapLink href: String, text: String)
    func proseViewer(_ view: ProseViewerView, didTapMention mention: ProseViewerMention)
    func proseViewer(_ view: ProseViewerView, didFail error: ProseViewerError)
}

public extension ProseViewerInteractionDelegate {
    func proseViewer(_ view: ProseViewerView, didFail error: ProseViewerError) {}
}

/// A direct Core Text viewer. Its first finite measurement or layout prepares an immutable artifact.
public final class ProseViewerView: UIView {
    public weak var interactionDelegate: ProseViewerInteractionDelegate?

    private let layoutRegistry: PreparedProseLayoutRegistry
    private let drawingView = PreparedProseDrawingView(frame: .zero)
    private var request: ProseViewerRequest?
    private var compiledDocument: ViewerDocument?
    private var ownedLayout: PreparedProseLayout?
    private lazy var preparedInstrumentationOwner = "direct-\(ObjectIdentifier(self))"
    private var pendingError: ProseViewerError?
    private var errorWasReported = false
    private let attachmentRevisions = ViewerAttachmentRevisionState()
    private let fontEnvironment = ViewerFontEnvironment.shared
    private var fontEnvironmentObserver: NSObjectProtocol?
    private lazy var viewerImagePipeline = ViewerImagePipeline(policy: .default)

    internal var drawingViewForTesting: PreparedProseDrawingView { drawingView }
    internal var linkTapsEnabled = true {
        didSet {
            guard oldValue != linkTapsEnabled else { return }
            drawingView.linkInteractionsEnabled = linkTapsEnabled
        }
    }
    /// Mounted host total; the layout cache intentionally excludes this
    /// mutable sidecar because it is not shared immutable layout state.
    internal var preparedSurfaceRetainedBytesForTesting: Int {
        PreparedProseDrawingView.saturatingAdd(
            PreparedProseDrawingView.saturatingAdd(
                ownedLayout?.retainedBytes ?? 0,
                attachmentRevisions.retainedPublicationBytesForTesting
            ),
            drawingView.retainedImagePixelsBytesForTesting
        )
    }

    public override init(frame: CGRect) {
        layoutRegistry = .shared
        super.init(frame: frame)
        setup()
    }

    init(frame: CGRect = .zero, layoutRegistry: PreparedProseLayoutRegistry) {
        self.layoutRegistry = layoutRegistry
        super.init(frame: frame)
        setup()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("ProseViewerView does not support NSCoder") }

    private func setup() {
        drawingView.backgroundColor = .clear
        drawingView.isOpaque = false
        drawingView.linkInteractionsEnabled = linkTapsEnabled
        drawingView.onActivateInteraction = { [weak self] interaction in self?.activate(interaction) ?? false }
        viewerImagePipeline.onPixels = { [weak self] attachment, image in
            guard let self, self.request?.semanticGenerationIdentity == self.viewerImageGeneration else { return }
            self.drawingView.imagePixels[attachment.id] = image
        }
        viewerImagePipeline.onIntrinsicMetadata = { [weak self] attachment, size in
            self?.applyIntrinsicImageMetadata(attachment, size: size)
        }
        viewerImagePipeline.onResourceFailure = { [weak self] attachment in self?.reportResourceFailureIfNeeded(attachment) }
        fontEnvironmentObserver = NotificationCenter.default.addObserver(
            forName: ViewerFontEnvironment.didInvalidateNotification,
            object: fontEnvironment,
            queue: .main
        ) { [weak self] note in
            self?.applyFontEnvironmentRevision(note.userInfo?["revision"] as? UInt64 ?? 0)
        }
        isAccessibilityElement = false
        addSubview(drawingView)
    }

    deinit {
        layoutRegistry.releaseDirectMounted(preparedInstrumentationOwner)
        if let fontEnvironmentObserver { NotificationCenter.default.removeObserver(fontEnvironmentObserver) }
    }

    @discardableResult
    private func activate(_ interaction: PreparedProseInteraction) -> Bool {
        switch interaction.kind {
        case .link:
            guard linkTapsEnabled, let href = interaction.href else { return false }
            interactionDelegate?.proseViewer(self, didTapLink: href, text: interaction.visibleText)
            return true
        case .mention:
            guard
                let docPos = interaction.docPos,
                let attrsJSON = interaction.attrsJSON,
                let attrs = Self.parseMentionAttrs(attrsJSON)
            else {
                interactionDelegate?.proseViewer(
                    self,
                    didFail: .compiler(
                        domain: "viewer",
                        code: "INVALID_MENTION_ATTRIBUTES",
                        message: "The prepared mention attributes are not a JSON object."
                    )
                )
                return false
            }
            interactionDelegate?.proseViewer(
                self,
                didTapMention: ProseViewerMention(docPos: docPos, label: interaction.label, attrs: attrs)
            )
            return true
        }
    }

    private static func parseMentionAttrs(_ json: String) -> [String: Any]? {
        guard let data = json.data(using: .utf8) else { return nil }
        return (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
    }

    @discardableResult
    internal func activatePreparedInteractionForTesting(_ interaction: PreparedProseInteraction) -> Bool {
        activate(interaction)
    }

    private func installPreparedLayout(_ layout: PreparedProseLayout?) {
        ownedLayout = layout
        if let layout {
            layoutRegistry.registerDirectMounted(preparedInstrumentationOwner, layout: layout)
        } else {
            layoutRegistry.releaseDirectMounted(preparedInstrumentationOwner)
        }
        PreparedProseInstrumentation.retained(.sidecars, scope: "direct-\(ObjectIdentifier(self))", bytes: attachmentRevisions.retainedPublicationBytesForTesting)
        drawingView.install(layout: layout)
    }

    /// Compiles once for this immutable generation. The first finite measurement prepares layout.
    @discardableResult
    public func apply(source: ProseViewerSource, configuration: ProseViewerConfiguration) -> Bool {
        let fontRevision = fontEnvironment.revision
        if let request,
           request.source == source,
           request.configuration == configuration {
            guard request.fontEnvironmentRevision != fontRevision else { return pendingError == nil }
            self.request = ProseViewerRequest(
                source: request.source,
                configuration: request.configuration,
                nativeFontRevision: request.nativeFontRevision,
                nativeFontScale: fontEnvironment.fontScale(for: fontRevision),
                fontEnvironmentRevision: fontRevision,
                attachmentRevision: request.attachmentRevision
            )
            invalidateIntrinsicContentSize()
            setNeedsLayout()
            return pendingError == nil
        }
        let nextRequest = ProseViewerRequest(
            source: source,
            configuration: configuration,
            nativeFontScale: fontEnvironment.fontScale(for: fontRevision),
            fontEnvironmentRevision: fontRevision,
            attachmentRevision: 0
        )
        PreparedProseInstrumentation.invalidated(.content)
        _ = attachmentRevisions.beginSemanticGeneration(nextRequest.semanticGenerationIdentity)
        request = nextRequest
        compiledDocument = nil
        viewerImagePipeline.cancel()
        drawingView.imagePixels = [:]
        installPreparedLayout(nil)
        pendingError = nil
        errorWasReported = false
        do {
            compiledDocument = try layoutRegistry.compileDocument(request: nextRequest)
            invalidateIntrinsicContentSize()
            setNeedsLayout()
            return true
        } catch let error as ProseViewerError {
            pendingError = error
            invalidateIntrinsicContentSize()
            setNeedsLayout()
            return false
        } catch {
            pendingError = .layout(message: String(describing: error))
            invalidateIntrinsicContentSize()
            setNeedsLayout()
            return false
        }
    }

    public override func sizeThatFits(_ size: CGSize) -> CGSize {
        let layout = preparedLayout(width: size.width, scale: displayScale)
        guard size.width.isFinite, size.width > 0 else { return .zero }
        let hostHeight = size.height
        let height = hostHeight.isFinite && hostHeight >= 0 ? min(layout.size.height, hostHeight) : layout.size.height
        return CGSize(width: layout.size.width, height: height)
    }

    public override var intrinsicContentSize: CGSize {
        guard bounds.width.isFinite, bounds.width > 0 else {
            return CGSize(width: UIView.noIntrinsicMetric, height: UIView.noIntrinsicMetric)
        }
        return preparedLayout(width: bounds.width, scale: displayScale).size
    }

    public override func systemLayoutSizeFitting(_ targetSize: CGSize) -> CGSize {
        sizeThatFits(targetSize)
    }

    public override func layoutSubviews() {
        super.layoutSubviews()
        drawingView.frame = bounds
        let scale = displayScale
        if let request,
           let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: bounds.width, scale: scale),
           !ownedLayoutMatches(request: request, widthPixels: widthPixels, scale: scale) {
            _ = preparedLayout(width: bounds.width, scale: scale)
        } else {
            drawingView.install(layout: ownedLayout)
        }
        requestVisibleImageAttachments()
    }

    public override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil else {
            layoutRegistry.releaseDirectMounted(preparedInstrumentationOwner)
            viewerImagePipeline.cancel()
            // Request cancellation is not a semantic replacement. Preserve
            // publication bits and revision for a later remount.
            drawingView.imagePixels = [:]
            return
        }
        if let ownedLayout {
            layoutRegistry.registerDirectMounted(preparedInstrumentationOwner, layout: ownedLayout)
        }
        requestVisibleImageAttachments()
    }

    /// Releases this surface's artifact ownership without clearing its delegate.
    public func prepareForReuse() {
        PreparedProseInstrumentation.invalidated(.reuse)
        request = nil
        compiledDocument = nil
        installPreparedLayout(nil)
        pendingError = nil
        errorWasReported = false
        viewerImagePipeline.cancel()
        attachmentRevisions.reset()
        drawingView.imagePixels = [:]
        invalidateIntrinsicContentSize()
        setNeedsLayout()
    }

    private var displayScale: CGFloat {
        let scale = window?.screen.scale ?? UIScreen.main.scale
        return scale.isFinite && scale > 0 ? scale : 1
    }

    private func ownedLayoutMatches(request: ProseViewerRequest, widthPixels: Int, scale: CGFloat) -> Bool {
        guard let ownedLayout else { return false }
        return ownedLayout.key.generationIdentity == request.generationIdentity
            && ownedLayout.key.widthPixels == widthPixels
            && ownedLayout.key.displayScaleBits == Double(scale).bitPattern
    }

    @discardableResult
    private func preparedLayout(width: CGFloat, scale: CGFloat) -> PreparedProseLayout {
        guard let request else {
            let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: width, scale: scale) ?? 0
            let errorWidth = widthPixels > 0
                ? ProseLayoutMetrics.canonicalWidth(widthPixels: widthPixels, scale: scale)
                : 0
            let empty = PreparedProseLayout.error(
                key: ProseLayoutKey(
                    semanticKey: "empty",
                    widthPixels: widthPixels,
                    themeDigest: "",
                    nativeFontRevision: 0,
                    fontEnvironmentRevision: 0,
                    displayScale: scale,
                    attachmentRevision: 0,
                    generationIdentity: "empty",
                    semanticGenerationIdentity: "empty"
                ),
                width: errorWidth,
                error: .hostContract(message: "No prose viewer source has been applied.")
            )
            installPreparedLayout(empty)
            return empty
        }
        if let pendingError {
            let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: width, scale: scale) ?? 0
            let safeWidth = widthPixels > 0
                ? ProseLayoutMetrics.canonicalWidth(widthPixels: widthPixels, scale: scale)
                : 0
            let errorLayout = PreparedProseLayout.error(
                key: ProseLayoutKey(
                    semanticKey: "error:" + request.compiledCacheKey,
                    widthPixels: widthPixels,
                    themeDigest: request.themeDigest,
                    nativeFontRevision: request.nativeFontRevision,
                    fontEnvironmentRevision: request.fontEnvironmentRevision,
                    displayScale: scale,
                    attachmentRevision: request.attachmentRevision,
                    generationIdentity: request.generationIdentity,
                    semanticGenerationIdentity: request.semanticGenerationIdentity
                ),
                width: safeWidth,
                error: pendingError
            )
            installPreparedLayout(errorLayout)
            reportErrorIfNeeded(pendingError)
            return errorLayout
        }
        let layout = layoutRegistry.measure(
            request: request,
            widthPoints: width,
            scale: scale,
            compiledDocument: compiledDocument,
            measurementImageState: attachmentRevisions
        )
        installPreparedLayout(layout)
        reportErrorIfNeeded(layout.error)
        return layout
    }

    private var viewerImageGeneration: String? { request?.semanticGenerationIdentity }

    private func configureImageGeneration(for layout: PreparedProseLayout) {
        guard let request else { return }
        _ = attachmentRevisions.beginSemanticGeneration(request.semanticGenerationIdentity)
        attachmentRevisions.admit(attachmentCount: layout.imageAttachments.count)
        viewerImagePipeline.begin(
            generation: request.semanticGenerationIdentity,
            imagesEnabled: request.configuration.imagesEnabled,
            policy: ImageLoadingPolicy.from(json: request.configuration.imagePolicyJSON)
        )
    }

    private func requestVisibleImageAttachments() {
        guard let layout = ownedLayout, drawingView.window != nil,
              !drawingView.isHidden, drawingView.alpha > 0,
              let window = drawingView.window
        else { return }
        let visibleRect = drawingView.convert(window.bounds, from: window).intersection(drawingView.bounds)
        guard visibleRect.origin.x.isFinite, visibleRect.origin.y.isFinite,
              visibleRect.size.width.isFinite, visibleRect.size.height.isFinite,
              !visibleRect.isNull, !visibleRect.isEmpty else { return }
        configureImageGeneration(for: layout)
        viewerImagePipeline.updateVisibleRect(visibleRect, attachments: layout.imageAttachments)
    }

    private func applyIntrinsicImageMetadata(_ attachment: ViewerImageAttachment, size: CGSize) {
        guard let request,
              viewerImagePipeline.acceptsCompletion(generation: request.semanticGenerationIdentity),
              attachmentRevisions.recordIntrinsicSize(size, for: attachment.id, ordinal: attachment.ordinal, declaredSize: attachment.declaredSize)
        else { return }
        self.request = ProseViewerRequest(
            source: request.source,
            configuration: request.configuration,
            nativeFontRevision: request.nativeFontRevision,
            nativeFontScale: request.nativeFontScale,
            fontEnvironmentRevision: request.fontEnvironmentRevision,
            attachmentRevision: attachmentRevisions.revision
        )
        PreparedProseInstrumentation.invalidated(.attachment)
        PreparedProseInstrumentation.retained(.sidecars, scope: "direct-\(ObjectIdentifier(self))", bytes: attachmentRevisions.retainedPublicationBytesForTesting)
        // Metadata is the sole image completion allowed to reflow. Pixels stay
        // in the drawing cache; this schedules exactly one replacement key.
        invalidateIntrinsicContentSize()
        setNeedsLayout()
    }

    private func applyFontEnvironmentRevision(_ revision: UInt64) {
        guard let request, revision > request.fontEnvironmentRevision else { return }
        self.request = ProseViewerRequest(
            source: request.source,
            configuration: request.configuration,
            nativeFontRevision: request.nativeFontRevision,
            nativeFontScale: fontEnvironment.fontScale(for: revision),
            fontEnvironmentRevision: revision,
            attachmentRevision: request.attachmentRevision
        )
        PreparedProseInstrumentation.invalidated(.font)
        invalidateIntrinsicContentSize()
        setNeedsLayout()
    }

    private func reportErrorIfNeeded(_ error: ProseViewerError?) {
        guard let error, !errorWasReported else { return }
        errorWasReported = true
        interactionDelegate?.proseViewer(self, didFail: error)
    }

    private func reportResourceFailureIfNeeded(_ attachment: ViewerImageAttachment) {
        guard attachmentRevisions.recordResourceFailure(for: attachment.ordinal)
        else { return }
        interactionDelegate?.proseViewer(self, didFail: .resource)
    }

}
