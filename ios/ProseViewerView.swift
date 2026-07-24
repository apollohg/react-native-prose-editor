import UIKit

/// Interaction callbacks for an embedded prose viewer.
public protocol ProseViewerInteractionDelegate: AnyObject {
    func proseViewer(_ view: ProseViewerView, didTapLink href: String, text: String)
    func proseViewer(_ view: ProseViewerView, didTapMention docPos: Int, label: String)
}

/// Display-only prose viewer for UIKit hosts.
///
/// Input is the flat render-ops JSON array produced by the package render
/// bridge. This view does not create or retain an editor handle.
public final class ProseViewerView: UIView {
    public weak var interactionDelegate: ProseViewerInteractionDelegate?

    private let textView = EditorTextView(frame: .zero, textContainer: nil)
    private let imageLoadOwner = RenderImageLoadOwner(policy: .default)
    private var lastRenderJSON = "[]"
    private var lastThemeJSON: String?
    private var collapsesWhenEmpty = false
    private var isCollapsedEmptyContent = false

    internal var onContentHeightChange: ((CGFloat) -> Void)?
    internal var opensLinksAutomatically = false
    internal var linkTapsEnabled = true
    internal var imageLoadingPolicyForHost: ImageLoadingPolicy { imageLoadOwner.policy }
    internal var isContentCollapsedForHost: Bool { isCollapsedEmptyContent }
    internal var renderedTextForTesting: String { textView.textStorage.string }
    internal var textViewForTesting: EditorTextView { textView }

    internal var contentInset: UIEdgeInsets {
        get { textView.textContainerInset }
        set {
            textView.baseTextContainerInset = newValue
            textView.textContainerInset = newValue
        }
    }

    private lazy var interactiveTapRecognizer: UITapGestureRecognizer = {
        let recognizer = UITapGestureRecognizer(
            target: self,
            action: #selector(handleInteractiveTap(_:))
        )
        recognizer.cancelsTouchesInView = false
        return recognizer
    }()

    public override init(frame: CGRect) {
        super.init(frame: frame)
        setupView()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("ProseViewerView does not support NSCoder")
    }

    deinit {
        imageLoadOwner.cancelAll()
    }

    private func setupView() {
        textView.imageLoadOwner = imageLoadOwner
        textView.baseBackgroundColor = .clear
        textView.backgroundColor = .clear
        textView.isEditable = false
        textView.isSelectable = false
        textView.allowImageResizing = false
        textView.baseTextContainerInset = .zero
        textView.textContainerInset = .zero
        textView.heightBehavior = .autoGrow
        textView.onHeightMayChange = { [weak self] measuredHeight in
            guard let self else { return }
            self.onContentHeightChange?(
                self.isCollapsedEmptyContent ? 0 : ceil(measuredHeight)
            )
        }
        textView.addGestureRecognizer(interactiveTapRecognizer)
        addSubview(textView)
    }

    /// Applies render-ops and theme JSON. Invalid render input clears the view.
    @discardableResult
    public func apply(renderJson: String, themeJson: String) -> Bool {
        let accepted = Self.isRenderOpsArray(renderJson)
        if accepted, renderJson == lastRenderJSON, themeJson == lastThemeJSON {
            return true
        }
        if lastThemeJSON != themeJson {
            lastThemeJSON = themeJson
            _ = textView.applyTheme(EditorTheme.from(json: themeJson))
        }
        lastRenderJSON = accepted ? renderJson : "[]"
        renderCurrentContent()
        return accepted
    }

    /// Updates the bounded image-loading policy from its serialized form.
    public func setImageLoadingPolicy(json: String?) {
        let policy = ImageLoadingPolicy.from(json: json)
        guard policy != imageLoadOwner.policy else { return }
        imageLoadOwner.updatePolicy(policy)
        renderCurrentContent()
    }

    /// Clears content and pending image work for a recycled host view.
    ///
    /// The interaction delegate is retained so cell owners may assign it once.
    public func prepareForReuse() {
        imageLoadOwner.cancelAll()
        lastRenderJSON = "[]"
        lastThemeJSON = nil
        isCollapsedEmptyContent = false
        textView.applyRenderJSON("[]")
        textView.isHidden = false
        invalidateIntrinsicContentSize()
        setNeedsLayout()
    }

    /// Returns the current content height at a UIKit width in points.
    public func measuredHeight(forWidth width: CGFloat) -> CGFloat {
        guard !isCollapsedEmptyContent, width > 0 else { return 0 }
        return ceil(textView.measuredAutoGrowHeightForTesting(width: width))
    }

    /// Measures valid render-ops without creating a viewer.
    public static func measureHeight(
        renderJson: String,
        themeJson: String,
        width: CGFloat
    ) -> CGFloat? {
        guard isRenderOpsArray(renderJson) else { return nil }
        return RenderBridge.measureHeight(
            forRenderJSON: renderJson,
            themeJSON: themeJson,
            width: width
        )
    }

    internal func setCollapsesWhenEmpty(_ collapses: Bool) {
        guard collapsesWhenEmpty != collapses else { return }
        collapsesWhenEmpty = collapses
        renderCurrentContent()
    }

    internal static func renderJsonContainsOnlyEmptyParagraphs(_ renderJson: String) -> Bool {
        NativeProseViewerEmptyContent.containsOnlyEmptyParagraphs(renderJson)
    }

    private func renderCurrentContent() {
        isCollapsedEmptyContent = collapsesWhenEmpty
            && Self.renderJsonContainsOnlyEmptyParagraphs(lastRenderJSON)
        imageLoadOwner.withCurrent {
            textView.applyRenderJSON(lastRenderJSON)
        }
        textView.isHidden = isCollapsedEmptyContent
        invalidateIntrinsicContentSize()
        setNeedsLayout()
    }

    public override func layoutSubviews() {
        super.layoutSubviews()
        if isCollapsedEmptyContent {
            textView.frame = CGRect(x: 0, y: 0, width: bounds.width, height: 0)
            textView.updateAutoGrowHostHeight(0)
        } else {
            textView.frame = bounds
            textView.updateAutoGrowHostHeight(bounds.height)
        }
    }

    private static func isRenderOpsArray(_ renderJson: String) -> Bool {
        guard let data = renderJson.data(using: .utf8),
              (try? JSONSerialization.jsonObject(with: data)) is [[String: Any]]
        else {
            return false
        }
        return true
    }

    @objc private func handleInteractiveTap(_ recognizer: UITapGestureRecognizer) {
        guard recognizer.state == .ended else { return }
        handleTap(at: recognizer.location(in: textView))
    }

    internal func handleTapForTesting(at location: CGPoint) {
        handleTap(at: location)
    }

    private func handleTap(at location: CGPoint) {
        if linkTapsEnabled, let link = linkHit(at: location) {
            if opensLinksAutomatically {
                openLink(link.href)
            } else {
                interactionDelegate?.proseViewer(
                    self,
                    didTapLink: link.href,
                    text: link.text
                )
            }
            return
        }
        guard let mention = mentionHit(at: location) else { return }
        interactionDelegate?.proseViewer(
            self,
            didTapMention: mention.docPos,
            label: mention.label
        )
    }

    private func characterIndex(at location: CGPoint) -> Int? {
        let textStorage = textView.textStorage
        guard textStorage.length > 0 else { return nil }

        let layoutManager = textView.layoutManager
        let textContainer = textView.textContainer
        var containerPoint = location
        containerPoint.x -= textView.textContainerInset.left
        containerPoint.y -= textView.textContainerInset.top

        let usedRect = layoutManager.usedRect(for: textContainer)
        guard usedRect.insetBy(dx: -6, dy: -6).contains(containerPoint) else {
            return nil
        }

        let glyphIndex = layoutManager.glyphIndex(for: containerPoint, in: textContainer)
        guard glyphIndex < layoutManager.numberOfGlyphs else { return nil }
        let characterIndex = layoutManager.characterIndexForGlyph(at: glyphIndex)
        guard characterIndex < textStorage.length else { return nil }
        return characterIndex
    }

    private func linkHit(at location: CGPoint) -> (href: String, text: String)? {
        let textStorage = textView.textStorage
        guard let characterIndex = characterIndex(at: location) else { return nil }

        var effectiveRange = NSRange(location: 0, length: 0)
        let attributes = textStorage.attributes(
            at: characterIndex,
            effectiveRange: &effectiveRange
        )
        guard let href = attributes[RenderBridgeAttributes.linkHref] as? String,
              !href.isEmpty
        else {
            return nil
        }

        let text = (textStorage.string as NSString).substring(with: effectiveRange)
        return (href, text)
    }

    private func mentionHit(at location: CGPoint) -> (docPos: Int, label: String)? {
        let textStorage = textView.textStorage
        guard let characterIndex = characterIndex(at: location) else { return nil }

        var effectiveRange = NSRange(location: 0, length: 0)
        let attributes = textStorage.attributes(
            at: characterIndex,
            effectiveRange: &effectiveRange
        )
        guard (attributes[RenderBridgeAttributes.voidNodeType] as? String) == "mention" else {
            return nil
        }

        let docPos =
            (attributes[RenderBridgeAttributes.docPos] as? NSNumber)?.intValue
            ?? Int((attributes[RenderBridgeAttributes.docPos] as? UInt32) ?? 0)
        let label = (textStorage.string as NSString).substring(with: effectiveRange)
        return (docPos, label)
    }

    private func openLink(_ href: String) {
        guard let url = URL(string: href) else { return }
        UIApplication.shared.open(url, options: [:], completionHandler: nil)
    }
}

enum NativeProseViewerEmptyContent {
    static func containsOnlyEmptyParagraphs(_ renderJson: String) -> Bool {
        guard let data = renderJson.data(using: .utf8),
              let elements = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else {
            return false
        }

        if elements.isEmpty { return true }

        var hasParagraph = false
        var paragraphIsOpen = false

        for element in elements {
            guard let type = element["type"] as? String else { return false }
            switch type {
            case "blockStart":
                guard !paragraphIsOpen,
                      element["nodeType"] as? String == "paragraph",
                      (element["depth"] as? NSNumber)?.intValue == 0
                else {
                    return false
                }
                paragraphIsOpen = true
                hasParagraph = true
            case "textRun":
                guard paragraphIsOpen,
                      let text = element["text"] as? String,
                      text.allSatisfy({ $0 == "\u{200B}" })
                else {
                    return false
                }
            case "blockEnd":
                guard paragraphIsOpen else { return false }
                paragraphIsOpen = false
            default:
                return false
            }
        }

        return hasParagraph && !paragraphIsOpen
    }
}
