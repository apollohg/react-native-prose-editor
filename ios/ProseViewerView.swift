import UIKit

/// Interaction callbacks for an embedded prose viewer.
public protocol ProseViewerInteractionDelegate: AnyObject {
    func proseViewer(_ view: ProseViewerView, didTapLink href: String, text: String)
    func proseViewer(_ view: ProseViewerView, didTapMention docPos: UInt32, label: String)
    func proseViewer(_ view: ProseViewerView, didFail error: ProseViewerError)
}

public extension ProseViewerInteractionDelegate {
    func proseViewer(_ view: ProseViewerView, didFail error: ProseViewerError) {}
}

/// A direct Core Text viewer. Measurement prepares an immutable artifact; layout only installs it.
public final class ProseViewerView: UIView {
    public weak var interactionDelegate: ProseViewerInteractionDelegate?

    private let layoutRegistry: PreparedProseLayoutRegistry
    private let drawingView = PreparedProseDrawingView(frame: .zero)
    private var request: ProseViewerRequest?
    private var compiledDocument: ViewerDocument?
    private var ownedLayout: PreparedProseLayout?
    private var pendingError: ProseViewerError?
    private var errorWasReported = false
    private let attachmentRevisions = ViewerAttachmentRevisionState()
    private let fontEnvironment = ViewerFontEnvironment.shared
    private var fontEnvironmentObserver: NSObjectProtocol?
    private lazy var viewerImagePipeline = ViewerImagePipeline(policy: .default)

    // Temporary source compatibility for the legacy Expo adapter. It is deliberately
    // lazy so direct-content users never create a TextKit view; Task 12 removes it.
    private lazy var legacyTextView = EditorTextView(frame: .zero, textContainer: nil)
    private var legacyImageLoadOwner = RenderImageLoadOwner(policy: .default)
    private var legacyCollapsesWhenEmpty = false
    private var legacyCollapsed = false

    internal var drawingViewForTesting: PreparedProseDrawingView { drawingView }
    internal var opensLinksAutomatically = false
    internal var linkTapsEnabled = true {
        didSet {
            guard oldValue != linkTapsEnabled else { return }
            drawingView.linkInteractionsEnabled = linkTapsEnabled
        }
    }
    internal var onContentHeightChange: ((CGFloat) -> Void)?
    internal var contentInset: UIEdgeInsets = .zero
    internal var imageLoadingPolicyForHost: ImageLoadingPolicy { legacyImageLoadOwner.policy }
    internal var isContentCollapsedForHost: Bool { legacyCollapsed }
    internal var renderedTextForTesting: String { legacyTextView.textStorage.string }
    internal var textViewForTesting: EditorTextView { legacyTextView }
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
            guard let docPos = interaction.docPos else { return false }
            interactionDelegate?.proseViewer(self, didTapMention: docPos, label: interaction.label)
            return true
        }
    }

    private func installPreparedLayout(_ layout: PreparedProseLayout?) {
        ownedLayout = layout
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
        drawingView.install(layout: ownedLayout)
        requestVisibleImageAttachments()
    }

    public override func didMoveToWindow() {
        super.didMoveToWindow()
        guard window != nil else {
            viewerImagePipeline.cancel()
            // Request cancellation is not a semantic replacement. Preserve
            // publication bits and revision for a later remount.
            drawingView.imagePixels = [:]
            return
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
        legacyImageLoadOwner.cancelAll()
        viewerImagePipeline.cancel()
        attachmentRevisions.reset()
        drawingView.imagePixels = [:]
        legacyTextView.removeFromSuperview()
        legacyCollapsed = false
        invalidateIntrinsicContentSize()
        setNeedsLayout()
    }

    private var displayScale: CGFloat {
        let scale = window?.screen.scale ?? UIScreen.main.scale
        return scale.isFinite && scale > 0 ? scale : 1
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

    // MARK: Temporary legacy adapter compatibility

    @discardableResult
    public func apply(renderJson: String, themeJson: String) -> Bool {
        legacyTextView.imageLoadOwner = legacyImageLoadOwner
        legacyTextView.baseTextContainerInset = contentInset
        legacyTextView.textContainerInset = contentInset
        _ = legacyTextView.applyTheme(EditorTheme.from(json: themeJson))
        let accepted = (try? JSONSerialization.jsonObject(with: Data(renderJson.utf8))) is [[String: Any]]
        legacyTextView.applyRenderJSON(accepted ? renderJson : "[]")
        legacyCollapsed = legacyCollapsesWhenEmpty && NativeProseViewerEmptyContent.containsOnlyEmptyParagraphs(renderJson)
        onContentHeightChange?(legacyCollapsed ? 0 : ceil(legacyTextView.measuredAutoGrowHeightForTesting(width: bounds.width)))
        return accepted
    }

    public func setImageLoadingPolicy(json: String?) {
        legacyImageLoadOwner.updatePolicy(ImageLoadingPolicy.from(json: json))
    }

    public func measuredHeight(forWidth width: CGFloat) -> CGFloat {
        guard width > 0 else { return 0 }
        return ceil(legacyTextView.measuredAutoGrowHeightForTesting(width: width))
    }

    public static func measureHeight(renderJson: String, themeJson: String, width: CGFloat) -> CGFloat? {
        guard (try? JSONSerialization.jsonObject(with: Data(renderJson.utf8))) is [[String: Any]] else { return nil }
        return RenderBridge.measureHeight(forRenderJSON: renderJson, themeJSON: themeJson, width: width)
    }

    internal func setCollapsesWhenEmpty(_ collapses: Bool) { legacyCollapsesWhenEmpty = collapses }
    internal static func renderJsonContainsOnlyEmptyParagraphs(_ renderJson: String) -> Bool {
        NativeProseViewerEmptyContent.containsOnlyEmptyParagraphs(renderJson)
    }
}

enum NativeProseViewerEmptyContent {
    static func containsOnlyEmptyParagraphs(_ renderJson: String) -> Bool {
        guard let data = renderJson.data(using: .utf8),
              let elements = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else { return false }
        if elements.isEmpty { return true }
        var openParagraph = false
        for element in elements {
            guard let type = element["type"] as? String else { return false }
            switch type {
            case "blockStart":
                guard !openParagraph, element["nodeType"] as? String == "paragraph" else { return false }
                openParagraph = true
            case "textRun":
                guard openParagraph, (element["text"] as? String)?.allSatisfy({ $0 == "\u{200B}" }) == true else { return false }
            case "blockEnd":
                guard openParagraph else { return false }
                openParagraph = false
            default: return false
            }
        }
        return !openParagraph
    }
}
