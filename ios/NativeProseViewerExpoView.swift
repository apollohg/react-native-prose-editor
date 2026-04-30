import ExpoModulesCore
import UIKit

final class NativeProseViewerExpoView: ExpoView {
    let onContentHeightChange = EventDispatcher()
    let onPressLink = EventDispatcher()
    let onPressMention = EventDispatcher()

    private let textView = EditorTextView(frame: .zero, textContainer: nil)
    private var lastRenderJSON: String?
    private var lastThemeJSON: String?
    private var lastEmittedContentHeight: CGFloat = 0
    private var lastMeasuredWidth: CGFloat = 0
    private var collapsesWhenEmpty = true
    private var isCollapsedEmptyContent = false
    private var enableLinkTaps = true
    private var interceptLinkTaps = false

    private lazy var interactiveTapRecognizer: UITapGestureRecognizer = {
        let recognizer = UITapGestureRecognizer(
            target: self,
            action: #selector(handleInteractiveTap(_:))
        )
        recognizer.cancelsTouchesInView = false
        return recognizer
    }()

    required init(appContext: AppContext? = nil) {
        super.init(appContext: appContext)
        setupView()
    }

    private func setupView() {
        textView.baseBackgroundColor = .clear
        textView.backgroundColor = .clear
        textView.isEditable = false
        textView.isSelectable = false
        textView.allowImageResizing = false
        textView.heightBehavior = .autoGrow
        textView.onHeightMayChange = { [weak self] measuredHeight in
            self?.emitContentHeightIfNeeded(measuredHeight: measuredHeight, force: true)
        }
        textView.addGestureRecognizer(interactiveTapRecognizer)
        addSubview(textView)
    }

    func setEnableLinkTaps(_ enabled: Bool?) {
        enableLinkTaps = enabled ?? true
    }

    func setInterceptLinkTaps(_ intercept: Bool?) {
        interceptLinkTaps = intercept ?? false
    }

    func setCollapsesWhenEmpty(_ collapses: Bool?) {
        let nextValue = collapses ?? true
        guard collapsesWhenEmpty != nextValue else { return }
        collapsesWhenEmpty = nextValue
        updateCollapsedEmptyState()
        setNeedsLayout()
        emitContentHeightIfNeeded(force: true)
    }

    func setRenderJson(_ renderJson: String?) {
        guard lastRenderJSON != renderJson else { return }
        lastRenderJSON = renderJson
        applyRenderJSON()
    }

    func setThemeJson(_ themeJson: String?) {
        guard lastThemeJSON != themeJson else { return }
        lastThemeJSON = themeJson
        let theme = EditorTheme.from(json: themeJson)
        textView.applyTheme(theme)
        let cornerRadius = theme?.borderRadius ?? 0
        layer.cornerRadius = cornerRadius
        clipsToBounds = cornerRadius > 0
        applyRenderJSON()
    }

    override var intrinsicContentSize: CGSize {
        if isCollapsedEmptyContent {
            return CGSize(width: UIView.noIntrinsicMetric, height: 0)
        }
        guard lastEmittedContentHeight > 0 else {
            return CGSize(width: UIView.noIntrinsicMetric, height: UIView.noIntrinsicMetric)
        }
        return CGSize(width: UIView.noIntrinsicMetric, height: lastEmittedContentHeight)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        if isCollapsedEmptyContent {
            textView.frame = CGRect(x: 0, y: 0, width: bounds.width, height: 0)
            textView.updateAutoGrowHostHeight(0)
        } else {
            textView.frame = bounds
            textView.updateAutoGrowHostHeight(bounds.height)
        }

        let currentWidth = ceil(bounds.width)
        guard abs(currentWidth - lastMeasuredWidth) > 0.5 else { return }
        lastMeasuredWidth = currentWidth
        emitContentHeightIfNeeded(force: true)
    }

    private func applyRenderJSON() {
        updateCollapsedEmptyState()
        textView.applyRenderJSON(lastRenderJSON ?? "[]")
        textView.isHidden = isCollapsedEmptyContent
        lastMeasuredWidth = 0
        invalidateIntrinsicContentSize()
        setNeedsLayout()
    }

    private func updateCollapsedEmptyState() {
        isCollapsedEmptyContent = collapsesWhenEmpty
            && Self.renderJsonContainsOnlyEmptyParagraphs(lastRenderJSON ?? "[]")
        textView.isHidden = isCollapsedEmptyContent
    }

    private func emitContentHeightIfNeeded(
        measuredHeight: CGFloat? = nil,
        force: Bool = false
    ) {
        let contentHeight: CGFloat
        if isCollapsedEmptyContent {
            contentHeight = 0
        } else {
            guard bounds.width > 0 else { return }
            let fittedHeight = measuredHeight
                ?? textView.measuredAutoGrowHeightForTesting(width: bounds.width)
            contentHeight = ceil(fittedHeight)
            guard contentHeight > 0 else { return }
        }
        guard force || abs(contentHeight - lastEmittedContentHeight) > 0.5 else { return }
        lastEmittedContentHeight = contentHeight
        invalidateIntrinsicContentSize()
        onContentHeightChange(["contentHeight": contentHeight])
    }

    @objc private func handleInteractiveTap(_ recognizer: UITapGestureRecognizer) {
        guard recognizer.state == .ended else {
            return
        }

        let location = recognizer.location(in: textView)
        if enableLinkTaps, let link = linkHit(at: location) {
            if interceptLinkTaps {
                onPressLink([
                    "href": link.href,
                    "text": link.text,
                ])
            } else {
                openLink(link.href)
            }
            return
        }

        guard let mention = mentionHit(at: location) else { return }
        onPressMention([
            "docPos": mention.docPos,
            "label": mention.label,
        ])
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
        let attrs = textStorage.attributes(at: characterIndex, effectiveRange: &effectiveRange)
        guard let href = attrs[RenderBridgeAttributes.linkHref] as? String, !href.isEmpty else {
            return nil
        }

        let text = (textStorage.string as NSString).substring(with: effectiveRange)
        return (href: href, text: text)
    }

    private func mentionHit(at location: CGPoint) -> (docPos: Int, label: String)? {
        let textStorage = textView.textStorage
        guard let characterIndex = characterIndex(at: location) else { return nil }

        var effectiveRange = NSRange(location: 0, length: 0)
        let attrs = textStorage.attributes(at: characterIndex, effectiveRange: &effectiveRange)
        guard (attrs[RenderBridgeAttributes.voidNodeType] as? String) == "mention" else {
            return nil
        }

        let docPos =
            (attrs[RenderBridgeAttributes.docPos] as? NSNumber)?.intValue
            ?? Int((attrs[RenderBridgeAttributes.docPos] as? UInt32) ?? 0)
        let label = (textStorage.string as NSString).substring(with: effectiveRange)
        return (docPos: docPos, label: label)
    }

    private func openLink(_ href: String) {
        guard let url = URL(string: href) else { return }
        UIApplication.shared.open(url, options: [:], completionHandler: nil)
    }

    static func renderJsonContainsOnlyEmptyParagraphs(_ renderJson: String) -> Bool {
        guard let data = renderJson.data(using: .utf8),
              let elements = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else {
            return false
        }

        if elements.isEmpty {
            return true
        }

        var hasParagraph = false
        var paragraphIsOpen = false

        for element in elements {
            guard let type = element["type"] as? String else {
                return false
            }

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
                guard paragraphIsOpen else {
                    return false
                }
                paragraphIsOpen = false

            default:
                return false
            }
        }

        return hasParagraph && !paragraphIsOpen
    }
}
