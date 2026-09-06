import UIKit

final class EditorAccessoryToolbarView: UIInputView {
    private static let baseHeight: CGFloat = 50
    private static let mentionRowHeight: CGFloat = 52
    private static let contentSpacing: CGFloat = 6
    private static let contentHorizontalInset: CGFloat = 12
    private static let defaultHorizontalInset: CGFloat = 0
    private static let defaultKeyboardOffset: CGFloat = 0
    private static let chromeTransitionDuration: TimeInterval = 0.18
    static let nativeDisabledButtonOpacity: CGFloat = 0.46

    struct ButtonBinding {
        let item: NativeToolbarItem
        let button: UIButton
        let widthConstraint: NSLayoutConstraint
        let heightConstraint: NSLayoutConstraint
    }

    struct VisibleToolbarItemsByPlacement {
        let start: [NativeToolbarItem]
        let scroll: [NativeToolbarItem]
        let end: [NativeToolbarItem]
    }

    private let chromeView = UIView()
    private let blurView = UIVisualEffectView(effect: nil)
    private let glassTintView = UIView()
    let bodyStackView = UIStackView()
    let startPinnedStackView = UIStackView()
    private let contentStackView = UIStackView()
    let endPinnedStackView = UIStackView()
    private let mentionScrollView = UIScrollView()
    private let mentionStackView = UIStackView()
    let scrollView = UIScrollView()
    let stackView = UIStackView()
    private var chromeLeadingConstraint: NSLayoutConstraint?
    private var chromeTrailingConstraint: NSLayoutConstraint?
    private var chromeBottomConstraint: NSLayoutConstraint?
    private var mentionRowHeightConstraint: NSLayoutConstraint?
    private var scrollViewHeightConstraint: NSLayoutConstraint?
    var buttonBindings: [ButtonBinding] = []
    var separators: [UIView] = []
    var mentionButtons: [MentionSuggestionChipButton] = []
    var items: [NativeToolbarItem] = NativeToolbarItem.defaults
    var expandedGroupKey: String?
    var currentState = NativeToolbarState.empty
    var theme: EditorToolbarTheme?
    private var mentionTheme: EditorMentionTheme?
    private var didAnimateChromeTransition = false
    var editMenuPresenter: AnyObject?
    var onPressItem: ((NativeToolbarItem) -> Void)?
    var onSelectMentionSuggestion: ((NativeMentionSuggestion) -> Void)?
    var isShowingMentionSuggestions: Bool {
        !mentionButtons.isEmpty && !mentionScrollView.isHidden && scrollView.isHidden
    }
    var usesNativeAppearanceForTesting: Bool {
        resolvedAppearance == .native
    }
    var usesUIGlassEffectForTesting: Bool {
#if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            return blurView.effect is UIGlassEffect
        }
#endif
        return false
    }
    var chromeBorderWidthForTesting: CGFloat {
        chromeView.layer.borderWidth
    }
    var nativeChromeIsTransparentForTesting: Bool {
        blurView.isHidden
            && glassTintView.isHidden
            && chromeView.layer.borderWidth == 0
            && chromeView.layer.shadowOpacity == 0
            && (chromeView.backgroundColor ?? .clear) == .clear
    }
    var didAnimateChromeTransitionForTesting: Bool {
        didAnimateChromeTransition
    }
    var startPinnedStackViewFrameForTesting: CGRect {
        startPinnedStackView.frame
    }
    var endPinnedStackViewFrameForTesting: CGRect {
        endPinnedStackView.frame
    }
    var contentStackViewFrameForTesting: CGRect {
        contentStackView.frame
    }
    var nativeToolbarVisibleWidthForTesting: CGFloat {
        scrollView.bounds.width
    }
    var nativeToolbarContentWidthForTesting: CGFloat {
        max(scrollView.contentSize.width, stackView.bounds.width)
    }
    var nativeToolbarContentOffsetXForTesting: CGFloat {
        scrollView.contentOffset.x
    }
    var selectedButtonCountForTesting: Int {
        buttonBindings.filter(\.button.isSelected).count
    }
    var editMenuPresentationRequestCountForTesting: Int {
        guard #available(iOS 16.0, *) else { return 0 }
        return (editMenuPresenter as? ToolbarEditMenuPresenter)?.presentationRequestCount ?? 0
    }

    override var intrinsicContentSize: CGSize {
        let contentHeight = mentionButtons.isEmpty ? resolvedToolbarHeight : Self.mentionRowHeight
        return CGSize(
            width: UIView.noIntrinsicMetric,
            height: contentHeight + resolvedKeyboardOffset
        )
    }

    convenience init(frame: CGRect) {
        self.init(frame: frame, inputViewStyle: .keyboard)
    }

    override init(frame: CGRect, inputViewStyle: UIInputView.Style) {
        super.init(frame: frame, inputViewStyle: inputViewStyle)
        translatesAutoresizingMaskIntoConstraints = false
        autoresizingMask = [.flexibleHeight]
        backgroundColor = .clear
        isOpaque = false
        allowsSelfSizing = true
        setupView()
        if #available(iOS 16.0, *) {
            let presenter = ToolbarEditMenuPresenter()
            editMenuPresenter = presenter
            addInteraction(presenter.interaction)
        }
        rebuildButtons()
    }

    required init?(coder: NSCoder) {
        return nil
    }

    func setItems(_ items: [NativeToolbarItem]) {
        self.items = items
        if let expandedGroupKey,
           !items.contains(where: {
               $0.type == .group && $0.key == expandedGroupKey && ($0.presentation ?? .expand) == .expand
           })
        {
            self.expandedGroupKey = nil
        }
        rebuildButtons()
    }
    func setItemsJSONForTesting(_ json: String) {
        setItems(NativeToolbarItem.from(json: json))
    }
    func applyStateJSONForTesting(_ json: String) {
        guard let state = NativeToolbarState(updateJSON: json) else { return }
        apply(state: state)
    }

    func apply(mentionTheme: EditorMentionTheme?) {
        self.mentionTheme = mentionTheme
        for button in mentionButtons {
            button.apply(theme: mentionTheme, toolbarAppearance: resolvedAppearance)
        }
    }

    func apply(theme: EditorToolbarTheme?) {
        apply(theme: theme, animateChrome: false)
    }

    func apply(theme: EditorToolbarTheme?, animateChrome: Bool) {
        self.theme = theme
        let usesNativeAppearance = resolvedAppearance == .native
        let usesTransparentMentionChrome = self.usesTransparentMentionChrome
        let targetBlurHidden = usesTransparentMentionChrome || !usesNativeAppearance
        let targetBlurAlpha: CGFloat = usesNativeAppearance && !usesTransparentMentionChrome ? resolvedEffectAlpha : 0
        let targetBlurEffect = usesNativeAppearance && !usesTransparentMentionChrome ? resolvedBlurEffect() : nil
        let targetGlassHidden = usesTransparentMentionChrome || !usesNativeAppearance
        let targetGlassBackground = usesNativeAppearance && !usesTransparentMentionChrome
            ? UIColor.systemBackground.withAlphaComponent(resolvedGlassTintAlpha)
            : .clear
        let targetGlassAlpha: CGFloat = targetGlassHidden ? 0 : 1
        let targetBorderColor = usesTransparentMentionChrome ? UIColor.clear : resolvedBorderColor
        let targetBorderWidth: CGFloat = usesTransparentMentionChrome
            ? 0
            : (usesNativeAppearance
            ? (1 / UIScreen.main.scale)
            : resolvedBorderWidth)
        let targetClipsToBounds =
            !usesTransparentMentionChrome
            && (usesNativeAppearance || resolvedBorderRadius > 0)
        let targetShadowOpacity: Float =
            usesNativeAppearance && !usesTransparentMentionChrome ? 0.08 : 0
        let targetShadowRadius: CGFloat =
            usesNativeAppearance && !usesTransparentMentionChrome ? 10 : 0

        chromeView.backgroundColor = usesNativeAppearance
            ? .clear
            : (theme?.backgroundColor ?? .systemBackground)
        chromeView.tintColor = usesNativeAppearance
            ? nil
            : (theme?.buttonColor ?? tintColor)
        chromeView.isOpaque = false
        chromeView.layer.cornerRadius = resolvedBorderRadius
        if #available(iOS 13.0, *) {
            chromeView.layer.cornerCurve = .continuous
        }
        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            let cornerConfig: UICornerConfiguration = usesNativeAppearance
                ? .capsule(maximumRadius: 24)
                : .uniformCorners(radius: .fixed(Double(resolvedBorderRadius)))
            chromeView.cornerConfiguration = cornerConfig
            blurView.cornerConfiguration = cornerConfig
            glassTintView.cornerConfiguration = cornerConfig
        }
        #endif
        chromeView.layer.shadowOffset = CGSize(width: 0, height: 2)
        chromeView.layer.shadowColor = UIColor.black.cgColor

        let applyChromeProperties = {
            self.blurView.alpha = targetBlurAlpha
            self.glassTintView.alpha = targetGlassAlpha
            self.chromeView.layer.borderColor = targetBorderColor.cgColor
            self.chromeView.layer.borderWidth = targetBorderWidth
            self.chromeView.layer.shadowOpacity = targetShadowOpacity
            self.chromeView.layer.shadowRadius = targetShadowRadius
        }
        let finishChromeProperties = {
            self.blurView.isHidden = targetBlurHidden
            self.blurView.effect = targetBlurEffect
            self.blurView.alpha = targetBlurAlpha
            self.glassTintView.isHidden = targetGlassHidden
            self.glassTintView.backgroundColor = targetGlassHidden ? .clear : targetGlassBackground
            self.glassTintView.alpha = targetGlassAlpha
            self.chromeView.layer.borderColor = targetBorderColor.cgColor
            self.chromeView.layer.borderWidth = targetBorderWidth
            self.chromeView.layer.shadowOpacity = targetShadowOpacity
            self.chromeView.layer.shadowRadius = targetShadowRadius
            self.chromeView.clipsToBounds = targetClipsToBounds
        }

        let shouldAnimateChrome = animateChrome && UIView.areAnimationsEnabled && window != nil
        didAnimateChromeTransition = shouldAnimateChrome
        if shouldAnimateChrome {
            let blurWasHidden = blurView.isHidden
            let glassWasHidden = glassTintView.isHidden
            if !targetBlurHidden {
                blurView.effect = targetBlurEffect
            }
            if !targetBlurHidden || !blurWasHidden {
                blurView.isHidden = false
            }
            if blurWasHidden && !targetBlurHidden {
                blurView.alpha = 0
            }
            if !targetGlassHidden {
                glassTintView.backgroundColor = targetGlassBackground
            }
            if !targetGlassHidden || !glassWasHidden {
                glassTintView.isHidden = false
            }
            if glassWasHidden && !targetGlassHidden {
                glassTintView.alpha = 0
            }
            chromeView.clipsToBounds = targetClipsToBounds
            UIView.animate(
                withDuration: Self.chromeTransitionDuration,
                delay: 0,
                options: [.beginFromCurrentState, .allowUserInteraction, .curveEaseOut],
                animations: applyChromeProperties,
                completion: { _ in
                    finishChromeProperties()
                }
            )
        } else {
            finishChromeProperties()
        }

        chromeLeadingConstraint?.constant = resolvedHorizontalInset
        chromeTrailingConstraint?.constant = -resolvedHorizontalInset
        chromeBottomConstraint?.constant = -resolvedKeyboardOffset
        scrollViewHeightConstraint?.constant = resolvedToolbarHeight
        invalidateIntrinsicContentSize()
        for separator in separators {
            separator.backgroundColor = usesNativeAppearance
                ? UIColor.separator.withAlphaComponent(0.45)
                : (theme?.separatorColor ?? .separator)
        }
        for binding in buttonBindings {
            binding.button.layer.cornerRadius = resolvedButtonBorderRadius(for: binding.item)
            binding.widthConstraint.constant = resolvedButtonSize
            binding.heightConstraint.constant = resolvedButtonSize
        }
        for button in mentionButtons {
            button.apply(theme: mentionTheme, toolbarAppearance: resolvedAppearance)
        }
        apply(state: currentState)
    }

    @discardableResult
    func setMentionSuggestions(
        _ suggestions: [NativeMentionSuggestion],
        trigger: String = "@"
    ) -> Bool {
        let hadSuggestions = !mentionButtons.isEmpty
        var existingButtonsByKey: [String: MentionSuggestionChipButton] = [:]
        for button in mentionButtons where existingButtonsByKey[button.suggestion.key] == nil {
            existingButtonsByKey[button.suggestion.key] = button
        }
        let nextButtons = suggestions.prefix(8).map { suggestion in
            if let button = existingButtonsByKey.removeValue(forKey: suggestion.key) {
                button.update(suggestion: suggestion, trigger: trigger)
                return button
            }
            let button = MentionSuggestionChipButton(
                suggestion: suggestion,
                trigger: trigger,
                theme: mentionTheme,
                toolbarAppearance: resolvedAppearance
            )
            button.addTarget(self, action: #selector(handleSelectMentionSuggestion(_:)), for: .touchUpInside)
            return button
        }

        for button in mentionButtons where !nextButtons.contains(where: { $0 === button }) {
            mentionStackView.removeArrangedSubview(button)
            button.removeFromSuperview()
        }
        for (index, button) in nextButtons.enumerated() {
            if mentionStackView.arrangedSubviews.indices.contains(index),
               mentionStackView.arrangedSubviews[index] === button
            {
                continue
            }
            if mentionStackView.arrangedSubviews.contains(where: { $0 === button }) {
                mentionStackView.removeArrangedSubview(button)
            }
            mentionStackView.insertArrangedSubview(button, at: index)
        }
        mentionButtons = nextButtons

        let hasSuggestions = !mentionButtons.isEmpty
        mentionScrollView.isHidden = !hasSuggestions
        scrollView.isHidden = hasSuggestions
        mentionRowHeightConstraint?.constant = hasSuggestions ? Self.mentionRowHeight : 0
        apply(theme: theme, animateChrome: hadSuggestions != hasSuggestions)
        invalidateIntrinsicContentSize()
        setNeedsLayout()
        return hadSuggestions != hasSuggestions
    }

    func apply(state: NativeToolbarState) {
        currentState = state
        for binding in buttonBindings {
            let buttonState = buttonState(for: binding.item, state: state)
            binding.button.isEnabled = buttonState.enabled
            binding.button.isSelected = buttonState.active
            binding.button.accessibilityTraits = buttonState.active ? [.button, .selected] : .button
            updateButtonAppearance(binding.button, item: binding.item, enabled: buttonState.enabled, active: buttonState.active)
        }
        if #available(iOS 16.0, *) {
            (editMenuPresenter as? ToolbarEditMenuPresenter)?.reloadVisibleMenu()
        }
    }

    var firstButtonAlphaForTesting: CGFloat {
        buttonBindings.first?.button.alpha ?? 0
    }
    var firstButtonTintColorForTesting: UIColor? {
        buttonBindings.first?.button.tintColor
    }
    var firstButtonTintAdjustmentModeForTesting: UIView.TintAdjustmentMode {
        buttonBindings.first?.button.tintAdjustmentMode ?? .automatic
    }

    private func setupView() {
        chromeView.translatesAutoresizingMaskIntoConstraints = false
        chromeView.backgroundColor = .systemBackground
        chromeView.layer.borderColor = UIColor.separator.cgColor
        chromeView.layer.borderWidth = 0.5
        chromeView.isOpaque = false
        addSubview(chromeView)

        blurView.translatesAutoresizingMaskIntoConstraints = false
        blurView.isHidden = true
        blurView.isUserInteractionEnabled = false
        blurView.clipsToBounds = true
        chromeView.addSubview(blurView)

        glassTintView.translatesAutoresizingMaskIntoConstraints = false
        glassTintView.isHidden = true
        glassTintView.isUserInteractionEnabled = false
        chromeView.addSubview(glassTintView)

        bodyStackView.translatesAutoresizingMaskIntoConstraints = false
        bodyStackView.axis = .horizontal
        bodyStackView.alignment = .fill
        bodyStackView.spacing = 0
        chromeView.addSubview(bodyStackView)

        startPinnedStackView.translatesAutoresizingMaskIntoConstraints = false
        startPinnedStackView.axis = .horizontal
        startPinnedStackView.alignment = .center
        startPinnedStackView.spacing = 6
        startPinnedStackView.directionalLayoutMargins = NSDirectionalEdgeInsets(
            top: 0,
            leading: Self.contentHorizontalInset,
            bottom: 0,
            trailing: 0
        )
        startPinnedStackView.isLayoutMarginsRelativeArrangement = true
        startPinnedStackView.setContentHuggingPriority(.required, for: .horizontal)
        startPinnedStackView.setContentCompressionResistancePriority(.required, for: .horizontal)
        bodyStackView.addArrangedSubview(startPinnedStackView)

        // contentStackView occupies the middle arranged-subview slot between the pinned
        // start/end stacks. Arranged subviews are laid out side-by-side by construction,
        // so the pinned items never overlap the scrolling middle.
        contentStackView.translatesAutoresizingMaskIntoConstraints = false
        contentStackView.axis = .vertical
        contentStackView.spacing = 0
        contentStackView.setContentHuggingPriority(.defaultLow, for: .horizontal)
        contentStackView.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        bodyStackView.addArrangedSubview(contentStackView)

        endPinnedStackView.translatesAutoresizingMaskIntoConstraints = false
        endPinnedStackView.axis = .horizontal
        endPinnedStackView.alignment = .center
        endPinnedStackView.spacing = 6
        endPinnedStackView.directionalLayoutMargins = NSDirectionalEdgeInsets(
            top: 0,
            leading: 0,
            bottom: 0,
            trailing: Self.contentHorizontalInset
        )
        endPinnedStackView.isLayoutMarginsRelativeArrangement = true
        endPinnedStackView.setContentHuggingPriority(.required, for: .horizontal)
        endPinnedStackView.setContentCompressionResistancePriority(.required, for: .horizontal)
        bodyStackView.addArrangedSubview(endPinnedStackView)

        mentionScrollView.translatesAutoresizingMaskIntoConstraints = false
        mentionScrollView.showsHorizontalScrollIndicator = false
        mentionScrollView.alwaysBounceHorizontal = true
        mentionScrollView.isHidden = true
        contentStackView.addArrangedSubview(mentionScrollView)

        mentionStackView.translatesAutoresizingMaskIntoConstraints = false
        mentionStackView.axis = .horizontal
        mentionStackView.alignment = .fill
        mentionStackView.spacing = 8
        mentionScrollView.addSubview(mentionStackView)

        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.showsHorizontalScrollIndicator = false
        scrollView.alwaysBounceHorizontal = true
        contentStackView.addArrangedSubview(scrollView)

        stackView.translatesAutoresizingMaskIntoConstraints = false
        stackView.axis = .horizontal
        stackView.alignment = .center
        stackView.spacing = 6
        scrollView.addSubview(stackView)

        let leading = chromeView.leadingAnchor.constraint(
            equalTo: leadingAnchor,
            constant: Self.defaultHorizontalInset
        )
        let trailing = chromeView.trailingAnchor.constraint(
            equalTo: trailingAnchor,
            constant: -Self.defaultHorizontalInset
        )
        let bottom = chromeView.bottomAnchor.constraint(
            equalTo: safeAreaLayoutGuide.bottomAnchor,
            constant: -Self.defaultKeyboardOffset
        )
        chromeLeadingConstraint = leading
        chromeTrailingConstraint = trailing
        chromeBottomConstraint = bottom
        let mentionHeight = mentionScrollView.heightAnchor.constraint(equalToConstant: 0)
        mentionRowHeightConstraint = mentionHeight
        let scrollViewHeight = scrollView.heightAnchor.constraint(equalToConstant: resolvedToolbarHeight)
        scrollViewHeightConstraint = scrollViewHeight

        NSLayoutConstraint.activate([
            chromeView.topAnchor.constraint(equalTo: topAnchor),
            leading,
            trailing,
            bottom,

            blurView.topAnchor.constraint(equalTo: chromeView.topAnchor),
            blurView.leadingAnchor.constraint(equalTo: chromeView.leadingAnchor),
            blurView.trailingAnchor.constraint(equalTo: chromeView.trailingAnchor),
            blurView.bottomAnchor.constraint(equalTo: chromeView.bottomAnchor),

            glassTintView.topAnchor.constraint(equalTo: chromeView.topAnchor),
            glassTintView.leadingAnchor.constraint(equalTo: chromeView.leadingAnchor),
            glassTintView.trailingAnchor.constraint(equalTo: chromeView.trailingAnchor),
            glassTintView.bottomAnchor.constraint(equalTo: chromeView.bottomAnchor),

            bodyStackView.topAnchor.constraint(equalTo: chromeView.topAnchor, constant: 6),
            bodyStackView.leadingAnchor.constraint(equalTo: chromeView.leadingAnchor),
            bodyStackView.trailingAnchor.constraint(equalTo: chromeView.trailingAnchor),
            bodyStackView.bottomAnchor.constraint(equalTo: chromeView.safeAreaLayoutGuide.bottomAnchor, constant: -6),

            mentionHeight,

            mentionStackView.topAnchor.constraint(equalTo: mentionScrollView.contentLayoutGuide.topAnchor),
            mentionStackView.leadingAnchor.constraint(
                equalTo: mentionScrollView.contentLayoutGuide.leadingAnchor,
                constant: Self.contentHorizontalInset
            ),
            mentionStackView.trailingAnchor.constraint(
                equalTo: mentionScrollView.contentLayoutGuide.trailingAnchor,
                constant: -Self.contentHorizontalInset
            ),
            mentionStackView.bottomAnchor.constraint(equalTo: mentionScrollView.contentLayoutGuide.bottomAnchor),
            mentionStackView.heightAnchor.constraint(equalTo: mentionScrollView.frameLayoutGuide.heightAnchor),

            stackView.topAnchor.constraint(equalTo: scrollView.contentLayoutGuide.topAnchor, constant: 6),
            stackView.leadingAnchor.constraint(
                equalTo: scrollView.contentLayoutGuide.leadingAnchor,
                constant: Self.contentHorizontalInset
            ),
            stackView.trailingAnchor.constraint(
                equalTo: scrollView.contentLayoutGuide.trailingAnchor,
                constant: -Self.contentHorizontalInset
            ),
            stackView.bottomAnchor.constraint(equalTo: scrollView.contentLayoutGuide.bottomAnchor, constant: -6),
            stackView.heightAnchor.constraint(equalTo: scrollView.frameLayoutGuide.heightAnchor, constant: -12),
            scrollViewHeight,
        ])

    }

    var resolvedAppearance: EditorToolbarAppearance {
        theme?.appearance ?? .custom
    }

    private var resolvedHorizontalInset: CGFloat {
        theme?.resolvedHorizontalInset ?? Self.defaultHorizontalInset
    }

    private var resolvedKeyboardOffset: CGFloat {
        theme?.resolvedKeyboardOffset ?? Self.defaultKeyboardOffset
    }

    private var resolvedBorderRadius: CGFloat {
        theme?.resolvedBorderRadius ?? 0
    }

    private var resolvedBorderWidth: CGFloat {
        theme?.resolvedBorderWidth ?? 0.5
    }

    private var resolvedToolbarHeight: CGFloat {
        max(theme?.height ?? Self.baseHeight, 1)
    }

    var resolvedButtonSize: CGFloat {
        if theme?.height == nil {
            return 36
        }
        return max(1, min(40, resolvedToolbarHeight - 4))
    }

    var resolvedButtonBorderRadius: CGFloat {
        theme?.resolvedButtonBorderRadius ?? 8
    }

    private var usesTransparentMentionChrome: Bool {
        guard resolvedAppearance == .native, !mentionButtons.isEmpty else { return false }
        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            return true
        }
        #endif
        return false
    }

    private var resolvedEffectAlpha: CGFloat {
        if #available(iOS 26.0, *), resolvedAppearance == .native {
            return 1
        }
        return resolvedAppearance == .native ? 0.72 : 1
    }

    private var resolvedGlassTintAlpha: CGFloat {
        if #available(iOS 26.0, *), resolvedAppearance == .native {
            return 0
        }
        return resolvedAppearance == .native ? 0.12 : 0
    }

    var resolvedGlassEffectTintColor: UIColor {
        return .clear
    }

    private var resolvedBorderColor: UIColor {
        if resolvedAppearance != .native {
            return theme?.borderColor ?? UIColor.separator
        }
        if #available(iOS 26.0, *) {
            return .clear
        }
        return UIColor.separator.withAlphaComponent(0.22)
    }

    @objc private func handleSelectMentionSuggestion(_ sender: MentionSuggestionChipButton) {
        onSelectMentionSuggestion?(sender.suggestion)
    }

}

/// Keeps iOS keyboard integrations on the inputAccessoryView path when the
/// visible toolbar is rendered outside the native keyboard accessory.
