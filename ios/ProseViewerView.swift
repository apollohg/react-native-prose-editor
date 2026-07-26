import UIKit

/// Interaction callbacks for an embedded prose viewer.
public protocol ProseViewerInteractionDelegate: AnyObject {
    func proseViewer(_ view: ProseViewerView, didTapLink href: String, text: String)
    func proseViewer(_ view: ProseViewerView, didTapMention docPos: Int, label: String)
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
    private var ownedLayout: PreparedProseLayout?
    private var pendingError: ProseViewerError?
    private var errorWasReported = false

    // Temporary source compatibility for the legacy Expo adapter. It is deliberately
    // lazy so direct-content users never create a TextKit view; Task 12 removes it.
    private lazy var legacyTextView = EditorTextView(frame: .zero, textContainer: nil)
    private var legacyImageLoadOwner = RenderImageLoadOwner(policy: .default)
    private var legacyCollapsesWhenEmpty = false
    private var legacyCollapsed = false

    internal var drawingViewForTesting: PreparedProseDrawingView { drawingView }
    internal var opensLinksAutomatically = false
    internal var linkTapsEnabled = true
    internal var onContentHeightChange: ((CGFloat) -> Void)?
    internal var contentInset: UIEdgeInsets = .zero
    internal var imageLoadingPolicyForHost: ImageLoadingPolicy { legacyImageLoadOwner.policy }
    internal var isContentCollapsedForHost: Bool { legacyCollapsed }
    internal var renderedTextForTesting: String { legacyTextView.textStorage.string }
    internal var textViewForTesting: EditorTextView { legacyTextView }

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
        addSubview(drawingView)
    }

    /// Compiles once for this immutable generation. The first finite measurement prepares layout.
    @discardableResult
    public func apply(source: ProseViewerSource, configuration: ProseViewerConfiguration) -> Bool {
        let nextRequest = ProseViewerRequest(source: source, configuration: configuration)
        if request == nextRequest, pendingError == nil { return true }
        request = nextRequest
        ownedLayout = nil
        drawingView.layout = nil
        pendingError = nil
        errorWasReported = false
        do {
            _ = try layoutRegistry.compileDocument(request: nextRequest)
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
        guard size.width.isFinite, size.width > 0 else { return .zero }
        let layout = preparedLayout(width: size.width, scale: displayScale)
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
        drawingView.layout = ownedLayout
    }

    /// Releases this surface's artifact ownership without clearing its delegate.
    public func prepareForReuse() {
        request = nil
        ownedLayout = nil
        pendingError = nil
        errorWasReported = false
        drawingView.layout = nil
        legacyImageLoadOwner.cancelAll()
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
            let empty = PreparedProseLayout.error(
                key: ProseLayoutKey(semanticKey: "empty", widthPixels: Int((width * scale).rounded()), themeDigest: "", fontRevision: 0, displayScale: scale, attachmentRevision: 0),
                width: width,
                error: .hostContract(message: "No prose viewer source has been applied.")
            )
            ownedLayout = empty
            return empty
        }
        if let pendingError {
            let errorLayout = PreparedProseLayout.error(
                key: ProseLayoutKey(
                    semanticKey: "error:" + request.compiledCacheKey,
                    widthPixels: Int((width * scale).rounded()),
                    themeDigest: request.themeDigest,
                    fontRevision: request.fontRevision,
                    displayScale: scale,
                    attachmentRevision: request.attachmentRevision
                ),
                width: width,
                error: pendingError
            )
            ownedLayout = errorLayout
            drawingView.layout = errorLayout
            reportErrorIfNeeded(pendingError)
            return errorLayout
        }
        let layout = layoutRegistry.measure(request: request, widthPoints: width, scale: scale)
        ownedLayout = layout
        drawingView.layout = layout
        reportErrorIfNeeded(layout.error)
        return layout
    }

    private func reportErrorIfNeeded(_ error: ProseViewerError?) {
        guard let error, !errorWasReported else { return }
        errorWasReported = true
        interactionDelegate?.proseViewer(self, didFail: error)
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
