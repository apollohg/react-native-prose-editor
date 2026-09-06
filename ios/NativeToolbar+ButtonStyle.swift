import UIKit

extension EditorAccessoryToolbarView {
    func buttonState(
        for item: NativeToolbarItem,
        state: NativeToolbarState
    ) -> (enabled: Bool, active: Bool) {
        switch item.type {
        case .mark:
            let mark = item.mark ?? ""
            return (
                enabled: state.allowedMarks.contains(mark),
                active: state.marks[mark] == true
            )
        case .heading:
            let level = item.headingLevel ?? 0
            let headingType = "h\(level)"
            return (
                enabled: state.commands["toggleHeading\(level)"] == true,
                active: state.nodes[headingType] == true
            )
        case .blockquote:
            return (
                enabled: state.commands["toggleBlockquote"] == true,
                active: state.nodes["blockquote"] == true
            )
        case .list:
            switch item.listType {
            case .bulletList, .bullet_list:
                return (
                    enabled: state.commands["wrapBulletList"] == true,
                    active: state.nodes[item.listType?.rawValue ?? ""] == true
                )
            case .orderedList, .ordered_list:
                return (
                    enabled: state.commands["wrapOrderedList"] == true,
                    active: state.nodes[item.listType?.rawValue ?? ""] == true
                )
            case .none:
                return (enabled: false, active: false)
            }
        case .command:
            switch item.command {
            case .indentList:
                return (
                    enabled: state.commands["indentList"] == true,
                    active: false
                )
            case .outdentList:
                return (
                    enabled: state.commands["outdentList"] == true,
                    active: false
                )
            case .undo:
                return (enabled: state.canUndo, active: false)
            case .redo:
                return (enabled: state.canRedo, active: false)
            case .none:
                return (enabled: false, active: false)
            }
        case .node:
            let nodeType = item.nodeType ?? ""
            return (
                enabled: state.insertableNodes.contains(nodeType),
                active: state.nodes[nodeType] == true
            )
        case .action:
            return (
                enabled: !item.isDisabled,
                active: item.isActive
            )
        case .group:
            let childStates = item.items.map { buttonState(for: $0, state: state) }
            return (
                enabled: childStates.contains { $0.enabled },
                active: childStates.contains { $0.active } ||
                    (
                        (item.presentation ?? .expand) == .expand &&
                            expandedGroupKey == item.key
                    )
            )
        case .separator:
            return (enabled: false, active: false)
        }
    }

    func updateButtonAppearance(
        _ button: UIButton,
        item: NativeToolbarItem,
        enabled: Bool,
        active: Bool
    ) {
        let buttonStyle = item.buttonStyle
        let tintColor: UIColor
        if !enabled {
            tintColor = buttonStyle?.disabledColor
                ?? theme?.buttonDisabledColor
                ?? (resolvedAppearance == .native
                    ? UIColor.label.withAlphaComponent(Self.nativeDisabledButtonOpacity)
                    : .tertiaryLabel)
        } else if active {
            tintColor = buttonStyle?.activeColor
                ?? theme?.buttonActiveColor
                ?? (resolvedAppearance == .native ? self.tintColor : .systemBlue)
        } else {
            tintColor = buttonStyle?.color
                ?? theme?.buttonColor
                ?? (resolvedAppearance == .native ? self.tintColor : .secondaryLabel)
        }

        button.tintColor = tintColor
        button.setTitleColor(tintColor, for: .normal)
        button.setTitleColor(tintColor, for: .disabled)
        button.tintAdjustmentMode = enabled ? .automatic : .normal
        button.alpha = enabled || resolvedAppearance == .native ? 1 : 0.7
        let inactiveBackgroundColor = buttonStyle?.backgroundColor
            ?? theme?.buttonBackgroundColor
            ?? .clear
        let activeBackgroundColor = buttonStyle?.activeBackgroundColor
            ?? theme?.buttonActiveBackgroundColor
            ?? (resolvedAppearance == .native
                ? UIColor.white.withAlphaComponent(0.18)
                : UIColor.systemBlue.withAlphaComponent(0.12))
        let disabledBackgroundColor = buttonStyle?.disabledBackgroundColor
            ?? theme?.buttonDisabledBackgroundColor
            ?? (active ? activeBackgroundColor : inactiveBackgroundColor)
        let backgroundColor: UIColor
        if !enabled {
            backgroundColor = disabledBackgroundColor
        } else if active {
            backgroundColor = activeBackgroundColor
        } else {
            backgroundColor = inactiveBackgroundColor
        }
        let cornerRadius = resolvedButtonBorderRadius(for: item)
        button.layer.cornerRadius = cornerRadius
        applyButtonBackground(
            to: button,
            color: backgroundColor,
            cornerRadius: cornerRadius
        )
        applyButtonIconStyle(to: button, item: item)
    }

    /// Own the configured background to avoid stacking it with UIButton state fills.
    private func applyButtonBackground(
        to button: UIButton,
        color: UIColor,
        cornerRadius: CGFloat
    ) {
        guard #available(iOS 15.0, *), var configuration = button.configuration else {
            button.backgroundColor = color
            return
        }
        var background = UIBackgroundConfiguration.clear()
        background.backgroundColor = color
        background.cornerRadius = cornerRadius
        configuration.background = background
        button.configuration = configuration
        button.backgroundColor = .clear
    }

    func resolvedButtonBorderRadius(for item: NativeToolbarItem) -> CGFloat {
        guard let radius = item.buttonStyle?.borderRadius, radius.isFinite else {
            return max(0, resolvedButtonBorderRadius)
        }
        return max(0, radius)
    }

    private func resolvedButtonIconSize(for item: NativeToolbarItem) -> CGFloat {
        let requestedSize = item.buttonStyle?.iconSize ?? theme?.buttonIconSize
        guard let requestedSize, requestedSize.isFinite, requestedSize > 0 else {
            return 16
        }
        return min(requestedSize, resolvedButtonSize)
    }

    private func applyButtonIconStyle(to button: UIButton, item: NativeToolbarItem) {
        let iconSize = resolvedButtonIconSize(for: item)
        let font = UIFont.systemFont(ofSize: iconSize, weight: .semibold)
        if #available(iOS 15.0, *), var configuration = button.configuration {
            configuration.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer {
                incoming in
                var outgoing = incoming
                outgoing.font = font
                return outgoing
            }
            button.configuration = configuration
            button.updateConfiguration()
        }
        button.titleLabel?.font = font
        if button.image(for: .normal) != nil {
            button.setPreferredSymbolConfiguration(
                UIImage.SymbolConfiguration(pointSize: iconSize, weight: .semibold),
                forImageIn: .normal
            )
        }
    }

    func resolvedBlurEffect() -> UIVisualEffect {
#if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            let effect = UIGlassEffect(style: .regular)
            effect.isInteractive = true
            effect.tintColor = resolvedGlassEffectTintColor
            return effect
        }
#endif
        if #available(iOS 13.0, *) {
            return UIBlurEffect(style: .systemUltraThinMaterial)
        }
        return UIBlurEffect(style: .extraLight)
    }

}
