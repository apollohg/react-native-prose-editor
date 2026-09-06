import UIKit

extension EditorAccessoryToolbarView {
    func setNativeToolbarContentOffsetXForTesting(_ offsetX: CGFloat) {
        scrollView.contentOffset.x = offsetX
    }

    func mentionButtonAtForTesting(_ index: Int) -> MentionSuggestionChipButton? {
        mentionButtons.indices.contains(index) ? mentionButtons[index] : nil
    }

    func buttonCountForTesting() -> Int {
        buttonBindings.count
    }

    func buttonLabelForTesting(_ index: Int) -> String? {
        buttonBindings.indices.contains(index) ? buttonBindings[index].button.accessibilityLabel : nil
    }

    func buttonIsEnabledForTesting(_ index: Int) -> Bool? {
        buttonBindings.indices.contains(index) ? buttonBindings[index].button.isEnabled : nil
    }

    func buttonTintColorForTesting(_ index: Int) -> UIColor? {
        buttonBindings.indices.contains(index) ? buttonBindings[index].button.tintColor : nil
    }

    func buttonFontSizeForTesting(_ index: Int) -> CGFloat? {
        buttonBindings.indices.contains(index) ? buttonBindings[index].button.titleLabel?.font.pointSize : nil
    }

    func buttonCornerRadiusForTesting(_ index: Int) -> CGFloat? {
        buttonBindings.indices.contains(index) ? buttonBindings[index].button.layer.cornerRadius : nil
    }

    func buttonBackgroundColorForTesting(_ index: Int) -> UIColor? {
        guard buttonBindings.indices.contains(index) else { return nil }
        let button = buttonBindings[index].button
        if #available(iOS 15.0, *) {
            return button.configuration?.background.backgroundColor
        }
        return button.backgroundColor
    }

    func buttonLabelsForPlacementForTesting(_ rawPlacement: String) -> [String] {
        guard let placement = ToolbarItemPlacement(rawValue: rawPlacement) else { return [] }
        switch placement {
        case .start:
            return startPinnedStackView.arrangedSubviews.compactMap {
                ($0 as? UIButton)?.accessibilityLabel
            }
        case .end:
            return endPinnedStackView.arrangedSubviews.compactMap {
                ($0 as? UIButton)?.accessibilityLabel
            }
        case .scroll:
            return stackView.arrangedSubviews.compactMap {
                ($0 as? UIButton)?.accessibilityLabel
            }
        }
    }

    /// How many distinct sources paint a background behind the button at
    /// `index`. More than one stacks shapes into a double halo.
    func buttonBackgroundSourceCountForTesting(_ index: Int) -> Int {
        guard buttonBindings.indices.contains(index) else { return 0 }
        let button = buttonBindings[index].button
        var sources = 0
        if Self.paintsBackground(button.backgroundColor) {
            sources += 1
        }
        if #available(iOS 15.0, *),
           Self.paintsBackground(button.configuration?.background.backgroundColor)
        {
            sources += 1
        }
        return sources
    }

    private static func paintsBackground(_ color: UIColor?) -> Bool {
        guard let color else { return false }
        var alpha: CGFloat = 0
        color.getRed(nil, green: nil, blue: nil, alpha: &alpha)
        return alpha > 0.01
    }

    func triggerButtonTapForTesting(_ index: Int) {
        guard buttonBindings.indices.contains(index) else { return }
        buttonBindings[index].button.sendActions(for: .touchUpInside)
    }

    func firstButtonTitleColorForTesting(_ state: UIControl.State) -> UIColor? {
        buttonBindings.first?.button.titleColor(for: state)
    }

    func applyBoldStateForTesting(active: Bool, enabled: Bool) {
        apply(
            state: NativeToolbarState(
                marks: ["bold": active],
                nodes: [:],
                commands: [:],
                allowedMarks: enabled ? ["bold"] : [],
                insertableNodes: [],
                canUndo: false,
                canRedo: false
            )
        )
    }

    func triggerMentionSuggestionTapForTesting(at index: Int) {
        guard mentionButtons.indices.contains(index) else { return }
        onSelectMentionSuggestion?(mentionButtons[index].suggestion)
    }

}
