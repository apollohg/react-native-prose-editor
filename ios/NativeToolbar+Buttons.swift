import UIKit

extension EditorAccessoryToolbarView {
    func rebuildButtons() {
        if #available(iOS 16.0, *) {
            (editMenuPresenter as? ToolbarEditMenuPresenter)?.dismiss()
        }
        buttonBindings.removeAll()
        separators.removeAll()
        for arrangedSubview in startPinnedStackView.arrangedSubviews {
            startPinnedStackView.removeArrangedSubview(arrangedSubview)
            arrangedSubview.removeFromSuperview()
        }
        for arrangedSubview in stackView.arrangedSubviews {
            stackView.removeArrangedSubview(arrangedSubview)
            arrangedSubview.removeFromSuperview()
        }
        for arrangedSubview in endPinnedStackView.arrangedSubviews {
            endPinnedStackView.removeArrangedSubview(arrangedSubview)
            arrangedSubview.removeFromSuperview()
        }

        let visibleItems = visibleToolbarItemsByPlacement()
        rebuildButtons(items: visibleItems.start, in: startPinnedStackView)
        rebuildButtons(items: visibleItems.scroll, in: stackView)
        rebuildButtons(items: visibleItems.end, in: endPinnedStackView)
        updatePinnedStackParticipation()
        apply(theme: theme)
        apply(state: currentState)
    }

    /// Exclude an empty pinned stack from `bodyStackView` entirely.
    ///
    /// Horizontal hugging cannot hold an empty `UIStackView` at zero width:
    /// hugging only resists growing beyond an *intrinsic* size, and an empty
    /// stack has none. With nothing to hug to, `bodyStackView`'s fill
    /// distribution is free to hand the whole row to an empty pinned stack,
    /// collapsing the scrolling middle that holds every button. Hiding an
    /// arranged subview removes it from the distribution outright, which is
    /// the only unambiguous way to say "this edge takes no space".
    private func updatePinnedStackParticipation() {
        startPinnedStackView.isHidden = startPinnedStackView.arrangedSubviews.isEmpty
        endPinnedStackView.isHidden = endPinnedStackView.arrangedSubviews.isEmpty
    }

    func rebuildButtons(items: [NativeToolbarItem], in container: UIStackView) {
        for item in items {
            if item.type == .separator {
                container.addArrangedSubview(makeSeparator())
                continue
            }

            container.addArrangedSubview(makeButton(item: item))
        }
    }

    private func compactToolbarItems(_ items: [NativeToolbarItem]) -> [NativeToolbarItem] {
        items.enumerated().filter { index, item in
            guard item.type == .separator else { return true }
            guard index > 0, index < items.count - 1 else { return false }
            return items[index - 1].type != .separator && items[index + 1].type != .separator
        }.map(\.element)
    }

    private func visibleToolbarItems() -> [NativeToolbarItem] {
        var visible: [NativeToolbarItem] = []
        for item in compactToolbarItems(items) {
            visible.append(item)
            if item.type == .group,
               (item.presentation ?? .expand) == .expand,
               expandedGroupKey == item.key
            {
                visible.append(contentsOf: item.items.map {
                    $0.with(parentGroupKey: item.key, inheritedPlacement: item.placement)
                })
            }
        }
        return compactToolbarItems(visible)
    }

    private func visibleToolbarItemsByPlacement() -> VisibleToolbarItemsByPlacement {
        var start: [NativeToolbarItem] = []
        var scroll: [NativeToolbarItem] = []
        var end: [NativeToolbarItem] = []

        for item in visibleToolbarItems() {
            switch item.placement ?? .scroll {
            case .start:
                start.append(item)
            case .scroll:
                scroll.append(item)
            case .end:
                end.append(item)
            }
        }

        return VisibleToolbarItemsByPlacement(
            start: compactToolbarItems(start),
            scroll: compactToolbarItems(scroll),
            end: compactToolbarItems(end)
        )
    }

    private func handleToolbarButtonPress(_ item: NativeToolbarItem) {
        switch item.type {
        case .group:
            handleGroupPress(item)
        default:
            onPressItem?(item.with(parentGroupKey: nil))
            if let parentGroupKey = item.parentGroupKey,
               expandedGroupKey == parentGroupKey
            {
                expandedGroupKey = nil
                rebuildButtons()
            }
        }
    }

    private func handleGroupPress(_ item: NativeToolbarItem) {
        guard item.type == .group, !item.items.isEmpty else { return }
        switch item.presentation ?? .expand {
        case .expand:
            expandedGroupKey = expandedGroupKey == item.key ? nil : item.key
            rebuildButtons()
        case .menu:
            break
        }
    }

    private func presentGroupMenu(_ item: NativeToolbarItem, from sourceButton: UIButton) {
        guard #available(iOS 16.0, *),
              let presenter = editMenuPresenter as? ToolbarEditMenuPresenter
        else {
            return
        }
        presenter.toggle(from: sourceButton) { [weak self] in
            self?.makeGroupMenu(item: item)
        }
    }

    private func makeGroupMenu(item: NativeToolbarItem) -> UIMenu? {
        guard item.type == .group else { return nil }
        let actions = item.items.compactMap { child -> UIAction? in
            let state = buttonState(for: child, state: currentState)
            let image = child.icon?.resolvedSFSymbolName().flatMap { UIImage(systemName: $0) }
            let title = child.label ?? child.icon?.resolvedGlyphText() ?? "Item"
            return UIAction(
                title: title,
                image: image,
                identifier: nil,
                discoverabilityTitle: child.label,
                attributes: state.enabled ? [] : [.disabled],
                state: state.active ? .on : .off
            ) { [weak self] _ in
                self?.handleToolbarButtonPress(child)
            }
        }
        guard !actions.isEmpty else { return nil }
        let menu = UIMenu(title: item.label ?? "", children: actions)
        menu.preferredElementSize = .large
        return menu
    }

    private func makeButton(item: NativeToolbarItem) -> UIButton {
        let button = UIButton(type: .system)
        button.translatesAutoresizingMaskIntoConstraints = false
        button.accessibilityLabel = item.label
        button.layer.cornerRadius = resolvedButtonBorderRadius(for: item)
        button.clipsToBounds = true
        if #available(iOS 15.0, *) {
            var configuration = UIButton.Configuration.plain()
            configuration.contentInsets = NSDirectionalEdgeInsets(
                top: 8,
                leading: 10,
                bottom: 8,
                trailing: 10
            )
            button.configuration = configuration
        } else {
            button.contentEdgeInsets = UIEdgeInsets(top: 8, left: 10, bottom: 8, right: 10)
        }
        if let symbolName = item.icon?.resolvedSFSymbolName(),
           let symbolImage = UIImage(systemName: symbolName)
        {
            button.setImage(symbolImage, for: .normal)
            button.setTitle(nil, for: .normal)
        } else {
            button.setImage(nil, for: .normal)
            button.setTitle(item.icon?.resolvedGlyphText() ?? "?", for: .normal)
        }
        let buttonSize = resolvedButtonSize
        let widthConstraint = button.widthAnchor.constraint(greaterThanOrEqualToConstant: buttonSize)
        let heightConstraint = button.heightAnchor.constraint(equalToConstant: buttonSize)
        widthConstraint.isActive = true
        heightConstraint.isActive = true
        if item.type == .group,
           (item.presentation ?? .expand) == .menu,
           #available(iOS 16.0, *)
        {
            button.accessibilityHint = "Shows menu"
            button.addAction(UIAction { [weak self, weak button] _ in
                guard let button else { return }
                self?.presentGroupMenu(item, from: button)
            }, for: .touchUpInside)
        } else {
            button.addAction(UIAction { [weak self] _ in
                self?.handleToolbarButtonPress(item)
            }, for: .touchUpInside)
        }
        updateButtonAppearance(button, item: item, enabled: true, active: false)
        buttonBindings.append(
            ButtonBinding(
                item: item,
                button: button,
                widthConstraint: widthConstraint,
                heightConstraint: heightConstraint
            )
        )
        return button
    }

    private func makeSeparator() -> UIView {
        let separator = UIView()
        separator.translatesAutoresizingMaskIntoConstraints = false
        separator.backgroundColor = .separator
        separator.widthAnchor.constraint(equalToConstant: 1 / UIScreen.main.scale).isActive = true
        separator.heightAnchor.constraint(equalToConstant: 22).isActive = true
        separators.append(separator)
        return separator
    }

}
