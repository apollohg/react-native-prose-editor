import ExpoModulesCore
import UIKit

/// Expo adapter over the public UIKit prose viewer.
final class NativeProseViewerExpoView: ExpoView {
    let onContentHeightChange = EventDispatcher()
    let onPressLink = EventDispatcher()
    let onPressMention = EventDispatcher()

    private let viewer = ProseViewerView(frame: .zero)
    private var lastRenderJSON: String?
    private var lastThemeJSON: String?
    private var lastEmittedContentHeight: CGFloat = 0
    private var lastMeasuredWidth: CGFloat = 0
    private var collapsesWhenEmpty = true

    internal var viewerForTesting: ProseViewerView { viewer }

    required init(appContext: AppContext? = nil) {
        super.init(appContext: appContext)
        viewer.interactionDelegate = self
        viewer.opensLinksAutomatically = true
        viewer.setCollapsesWhenEmpty(true)
        viewer.contentInset = UIEdgeInsets(top: 8, left: 0, bottom: 8, right: 0)
        viewer.onContentHeightChange = { [weak self] measuredHeight in
            self?.emitContentHeightIfNeeded(measuredHeight: measuredHeight, force: true)
        }
        addSubview(viewer)
    }

    var imageLoadingPolicy: ImageLoadingPolicy {
        viewer.imageLoadingPolicyForHost
    }

    func setImageLoadingPolicyJson(_ json: String?) {
        viewer.setImageLoadingPolicy(json: json)
    }

    func setEnableLinkTaps(_ enabled: Bool?) {
        viewer.linkTapsEnabled = enabled ?? true
    }

    func setInterceptLinkTaps(_ intercept: Bool?) {
        viewer.opensLinksAutomatically = !(intercept ?? false)
    }

    func setCollapsesWhenEmpty(_ collapses: Bool?) {
        let nextValue = collapses ?? true
        guard collapsesWhenEmpty != nextValue else { return }
        collapsesWhenEmpty = nextValue
        viewer.setCollapsesWhenEmpty(nextValue)
        setNeedsLayout()
        emitContentHeightIfNeeded(force: true)
    }

    func setRenderJson(_ renderJson: String?) {
        guard lastRenderJSON != renderJson else { return }
        lastRenderJSON = renderJson
        applyToViewer()
    }

    func setThemeJson(_ themeJson: String?) {
        guard lastThemeJSON != themeJson else { return }
        lastThemeJSON = themeJson
        let theme = EditorTheme.from(json: themeJson)
        let cornerRadius = theme?.borderRadius ?? 0
        layer.cornerRadius = cornerRadius
        clipsToBounds = cornerRadius > 0
        applyToViewer()
    }

    private func applyToViewer() {
        viewer.apply(
            renderJson: lastRenderJSON ?? "[]",
            themeJson: lastThemeJSON ?? "{}"
        )
        lastMeasuredWidth = 0
        invalidateIntrinsicContentSize()
        setNeedsLayout()
    }

    override var intrinsicContentSize: CGSize {
        if viewer.isContentCollapsedForHost {
            return CGSize(width: UIView.noIntrinsicMetric, height: 0)
        }
        guard lastEmittedContentHeight > 0 else {
            return CGSize(
                width: UIView.noIntrinsicMetric,
                height: UIView.noIntrinsicMetric
            )
        }
        return CGSize(width: UIView.noIntrinsicMetric, height: lastEmittedContentHeight)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        viewer.frame = bounds

        let currentWidth = ceil(bounds.width)
        guard abs(currentWidth - lastMeasuredWidth) > 0.5 else { return }
        lastMeasuredWidth = currentWidth
        emitContentHeightIfNeeded(force: true)
    }

    private func emitContentHeightIfNeeded(
        measuredHeight: CGFloat? = nil,
        force: Bool = false
    ) {
        let contentHeight: CGFloat
        if viewer.isContentCollapsedForHost {
            contentHeight = 0
        } else {
            guard bounds.width > 0 else { return }
            let fittedHeight = measuredHeight
                ?? viewer.measuredHeight(forWidth: bounds.width)
            contentHeight = ceil(fittedHeight)
            guard contentHeight > 0 else { return }
        }
        guard force || abs(contentHeight - lastEmittedContentHeight) > 0.5 else {
            return
        }
        lastEmittedContentHeight = contentHeight
        invalidateIntrinsicContentSize()
        onContentHeightChange(["contentHeight": contentHeight])
    }
}

extension NativeProseViewerExpoView: ProseViewerInteractionDelegate {
    func proseViewer(_ view: ProseViewerView, didTapLink href: String, text: String) {
        onPressLink(["href": href, "text": text])
    }

    func proseViewer(
        _ view: ProseViewerView,
        didTapMention docPos: Int,
        label: String
    ) {
        onPressMention(["docPos": docPos, "label": label])
    }
}
