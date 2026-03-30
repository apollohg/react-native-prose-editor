import ExpoModulesCore
import UIKit
import os

private struct NativeToolbarState {
    let marks: [String: Bool]
    let nodes: [String: Bool]
    let commands: [String: Bool]
    let allowedMarks: Set<String>
    let insertableNodes: Set<String>
    let canUndo: Bool
    let canRedo: Bool

    static let empty = NativeToolbarState(
        marks: [:],
        nodes: [:],
        commands: [:],
        allowedMarks: [],
        insertableNodes: [],
        canUndo: false,
        canRedo: false
    )

    init(
        marks: [String: Bool],
        nodes: [String: Bool],
        commands: [String: Bool],
        allowedMarks: Set<String>,
        insertableNodes: Set<String>,
        canUndo: Bool,
        canRedo: Bool
    ) {
        self.marks = marks
        self.nodes = nodes
        self.commands = commands
        self.allowedMarks = allowedMarks
        self.insertableNodes = insertableNodes
        self.canUndo = canUndo
        self.canRedo = canRedo
    }

    init?(updateJSON: String) {
        guard let data = updateJSON.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }

        let activeState = raw["activeState"] as? [String: Any] ?? [:]
        let historyState = raw["historyState"] as? [String: Any] ?? [:]

        self.init(
            marks: NativeToolbarState.boolMap(from: activeState["marks"]),
            nodes: NativeToolbarState.boolMap(from: activeState["nodes"]),
            commands: NativeToolbarState.boolMap(from: activeState["commands"]),
            allowedMarks: Set((activeState["allowedMarks"] as? [String]) ?? []),
            insertableNodes: Set((activeState["insertableNodes"] as? [String]) ?? []),
            canUndo: (historyState["canUndo"] as? Bool) ?? false,
            canRedo: (historyState["canRedo"] as? Bool) ?? false
        )
    }

    private static func boolMap(from value: Any?) -> [String: Bool] {
        guard let map = value as? [String: Any] else { return [:] }
        var result: [String: Bool] = [:]
        for (key, rawValue) in map {
            if let bool = rawValue as? Bool {
                result[key] = bool
            } else if let number = rawValue as? NSNumber {
                result[key] = number.boolValue
            }
        }
        return result
    }
}

private enum ToolbarCommand: String {
    case indentList
    case outdentList
    case undo
    case redo
}

private enum ToolbarListType: String {
    case bulletList
    case orderedList
}

private enum ToolbarDefaultIconId: String {
    case bold
    case italic
    case underline
    case strike
    case bulletList
    case orderedList
    case indentList
    case outdentList
    case lineBreak
    case horizontalRule
    case undo
    case redo
}

private enum ToolbarItemKind: String {
    case mark
    case list
    case command
    case node
    case action
    case separator
}

private struct NativeToolbarIcon {
    let defaultId: ToolbarDefaultIconId?
    let glyphText: String?
    let iosSymbolName: String?
    let fallbackText: String?

    private static let defaultSFSymbolNames: [ToolbarDefaultIconId: String] = [
        .bold: "bold",
        .italic: "italic",
        .underline: "underline",
        .strike: "strikethrough",
        .bulletList: "list.bullet",
        .orderedList: "list.number",
        .indentList: "increase.indent",
        .outdentList: "decrease.indent",
        .lineBreak: "return.left",
        .horizontalRule: "minus",
        .undo: "arrow.uturn.backward",
        .redo: "arrow.uturn.forward",
    ]

    private static let defaultGlyphs: [ToolbarDefaultIconId: String] = [
        .bold: "B",
        .italic: "I",
        .underline: "U",
        .strike: "S",
        .bulletList: "•≡",
        .orderedList: "1.",
        .indentList: "→",
        .outdentList: "←",
        .lineBreak: "↵",
        .horizontalRule: "—",
        .undo: "↩",
        .redo: "↪",
    ]

    static func defaultIcon(_ id: ToolbarDefaultIconId) -> NativeToolbarIcon {
        NativeToolbarIcon(defaultId: id, glyphText: nil, iosSymbolName: nil, fallbackText: nil)
    }

    static func glyph(_ text: String) -> NativeToolbarIcon {
        NativeToolbarIcon(defaultId: nil, glyphText: text, iosSymbolName: nil, fallbackText: nil)
    }

    static func platform(iosSymbolName: String?, fallbackText: String?) -> NativeToolbarIcon {
        NativeToolbarIcon(
            defaultId: nil,
            glyphText: nil,
            iosSymbolName: iosSymbolName,
            fallbackText: fallbackText
        )
    }

    static func from(jsonValue: Any?) -> NativeToolbarIcon? {
        guard let raw = jsonValue as? [String: Any],
              let rawType = raw["type"] as? String
        else {
            return nil
        }

        switch rawType {
        case "default":
            guard let rawId = raw["id"] as? String,
                  let id = ToolbarDefaultIconId(rawValue: rawId)
            else {
                return nil
            }
            return .defaultIcon(id)
        case "glyph":
            guard let text = raw["text"] as? String, !text.isEmpty else {
                return nil
            }
            return .glyph(text)
        case "platform":
            let iosSymbolName = ((raw["ios"] as? [String: Any]).flatMap { iosRaw -> String? in
                guard (iosRaw["type"] as? String) == "sfSymbol",
                      let name = iosRaw["name"] as? String,
                      !name.isEmpty
                else {
                    return nil
                }
                return name
            })
            let fallbackText = raw["fallbackText"] as? String
            guard iosSymbolName != nil || fallbackText != nil else {
                return nil
            }
            return .platform(iosSymbolName: iosSymbolName, fallbackText: fallbackText)
        default:
            return nil
        }
    }

    func resolvedSFSymbolName() -> String? {
        if let iosSymbolName, !iosSymbolName.isEmpty {
            return iosSymbolName
        }
        guard let defaultId else { return nil }
        return Self.defaultSFSymbolNames[defaultId]
    }

    func resolvedGlyphText() -> String? {
        if let glyphText, !glyphText.isEmpty {
            return glyphText
        }
        if let fallbackText, !fallbackText.isEmpty {
            return fallbackText
        }
        guard let defaultId else { return nil }
        return Self.defaultGlyphs[defaultId]
    }
}

private struct NativeToolbarItem {
    let type: ToolbarItemKind
    let key: String?
    let label: String?
    let icon: NativeToolbarIcon?
    let mark: String?
    let listType: ToolbarListType?
    let command: ToolbarCommand?
    let nodeType: String?
    let isActive: Bool
    let isDisabled: Bool

    static let defaults: [NativeToolbarItem] = [
        NativeToolbarItem(type: .mark, key: nil, label: "Bold", icon: .defaultIcon(.bold), mark: "bold", listType: nil, command: nil, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .mark, key: nil, label: "Italic", icon: .defaultIcon(.italic), mark: "italic", listType: nil, command: nil, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .mark, key: nil, label: "Underline", icon: .defaultIcon(.underline), mark: "underline", listType: nil, command: nil, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .mark, key: nil, label: "Strikethrough", icon: .defaultIcon(.strike), mark: "strike", listType: nil, command: nil, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .separator, key: nil, label: nil, icon: nil, mark: nil, listType: nil, command: nil, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .list, key: nil, label: "Bullet List", icon: .defaultIcon(.bulletList), mark: nil, listType: .bulletList, command: nil, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .list, key: nil, label: "Ordered List", icon: .defaultIcon(.orderedList), mark: nil, listType: .orderedList, command: nil, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .command, key: nil, label: "Indent List", icon: .defaultIcon(.indentList), mark: nil, listType: nil, command: .indentList, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .command, key: nil, label: "Outdent List", icon: .defaultIcon(.outdentList), mark: nil, listType: nil, command: .outdentList, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .node, key: nil, label: "Line Break", icon: .defaultIcon(.lineBreak), mark: nil, listType: nil, command: nil, nodeType: "hardBreak", isActive: false, isDisabled: false),
        NativeToolbarItem(type: .node, key: nil, label: "Horizontal Rule", icon: .defaultIcon(.horizontalRule), mark: nil, listType: nil, command: nil, nodeType: "horizontalRule", isActive: false, isDisabled: false),
        NativeToolbarItem(type: .separator, key: nil, label: nil, icon: nil, mark: nil, listType: nil, command: nil, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .command, key: nil, label: "Undo", icon: .defaultIcon(.undo), mark: nil, listType: nil, command: .undo, nodeType: nil, isActive: false, isDisabled: false),
        NativeToolbarItem(type: .command, key: nil, label: "Redo", icon: .defaultIcon(.redo), mark: nil, listType: nil, command: .redo, nodeType: nil, isActive: false, isDisabled: false),
    ]

    static func from(json: String?) -> [NativeToolbarItem] {
        guard let json,
              let data = json.data(using: .utf8),
              let rawItems = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else {
            return defaults
        }

        let parsed = rawItems.compactMap { rawItem -> NativeToolbarItem? in
            guard let rawType = rawItem["type"] as? String,
                  let type = ToolbarItemKind(rawValue: rawType)
            else {
                return nil
            }

            let key = rawItem["key"] as? String
            switch type {
            case .separator:
                return NativeToolbarItem(
                    type: .separator,
                    key: key,
                    label: nil,
                    icon: nil,
                    mark: nil,
                    listType: nil,
                    command: nil,
                    nodeType: nil,
                    isActive: false,
                    isDisabled: false
                )
            case .mark:
                guard let mark = rawItem["mark"] as? String,
                      let label = rawItem["label"] as? String,
                      let icon = NativeToolbarIcon.from(jsonValue: rawItem["icon"])
                else {
                    return nil
                }
                return NativeToolbarItem(
                    type: .mark,
                    key: key,
                    label: label,
                    icon: icon,
                    mark: mark,
                    listType: nil,
                    command: nil,
                    nodeType: nil,
                    isActive: false,
                    isDisabled: false
                )
            case .list:
                guard let listTypeRaw = rawItem["listType"] as? String,
                      let listType = ToolbarListType(rawValue: listTypeRaw),
                      let label = rawItem["label"] as? String,
                      let icon = NativeToolbarIcon.from(jsonValue: rawItem["icon"])
                else {
                    return nil
                }
                return NativeToolbarItem(
                    type: .list,
                    key: key,
                    label: label,
                    icon: icon,
                    mark: nil,
                    listType: listType,
                    command: nil,
                    nodeType: nil,
                    isActive: false,
                    isDisabled: false
                )
            case .command:
                guard let commandRaw = rawItem["command"] as? String,
                      let command = ToolbarCommand(rawValue: commandRaw),
                      let label = rawItem["label"] as? String,
                      let icon = NativeToolbarIcon.from(jsonValue: rawItem["icon"])
                else {
                    return nil
                }
                return NativeToolbarItem(
                    type: .command,
                    key: key,
                    label: label,
                    icon: icon,
                    mark: nil,
                    listType: nil,
                    command: command,
                    nodeType: nil,
                    isActive: false,
                    isDisabled: false
                )
            case .node:
                guard let nodeType = rawItem["nodeType"] as? String,
                      let label = rawItem["label"] as? String,
                      let icon = NativeToolbarIcon.from(jsonValue: rawItem["icon"])
                else {
                    return nil
                }
                return NativeToolbarItem(
                    type: .node,
                    key: key,
                    label: label,
                    icon: icon,
                    mark: nil,
                    listType: nil,
                    command: nil,
                    nodeType: nodeType,
                    isActive: false,
                    isDisabled: false
                )
            case .action:
                guard let key,
                      let label = rawItem["label"] as? String,
                      let icon = NativeToolbarIcon.from(jsonValue: rawItem["icon"])
                else {
                    return nil
                }
                return NativeToolbarItem(
                    type: .action,
                    key: key,
                    label: label,
                    icon: icon,
                    mark: nil,
                    listType: nil,
                    command: nil,
                    nodeType: nil,
                    isActive: (rawItem["isActive"] as? Bool) ?? false,
                    isDisabled: (rawItem["isDisabled"] as? Bool) ?? false
                )
            }
        }

        return parsed.isEmpty ? defaults : parsed
    }

    func resolvedKey(index: Int) -> String {
        if let key {
            return key
        }
        switch type {
        case .mark:
            return "mark:\(mark ?? ""):\(index)"
        case .list:
            return "list:\(listType?.rawValue ?? ""):\(index)"
        case .command:
            return "command:\(command?.rawValue ?? ""):\(index)"
        case .node:
            return "node:\(nodeType ?? ""):\(index)"
        case .action:
            return "action:\(key ?? ""):\(index)"
        case .separator:
            return "separator:\(index)"
        }
    }
}

final class EditorAccessoryToolbarView: UIView {
    private static let baseHeight: CGFloat = 50
    private static let mentionRowHeight: CGFloat = 52
    private static let contentSpacing: CGFloat = 6
    private static let defaultHorizontalInset: CGFloat = 0
    private static let defaultKeyboardOffset: CGFloat = 0

    private struct ButtonBinding {
        let item: NativeToolbarItem
        let button: UIButton
    }

    private let chromeView = UIView()
    private let contentStackView = UIStackView()
    private let mentionScrollView = UIScrollView()
    private let mentionStackView = UIStackView()
    private let scrollView = UIScrollView()
    private let stackView = UIStackView()
    private var chromeLeadingConstraint: NSLayoutConstraint?
    private var chromeTrailingConstraint: NSLayoutConstraint?
    private var chromeBottomConstraint: NSLayoutConstraint?
    private var mentionRowHeightConstraint: NSLayoutConstraint?
    private var buttonBindings: [ButtonBinding] = []
    private var separators: [UIView] = []
    private var mentionButtons: [MentionSuggestionChipButton] = []
    private var items: [NativeToolbarItem] = NativeToolbarItem.defaults
    private var currentState = NativeToolbarState.empty
    private var theme: EditorToolbarTheme?
    private var mentionTheme: EditorMentionTheme?
    fileprivate var onPressItem: ((NativeToolbarItem) -> Void)?
    var onSelectMentionSuggestion: ((NativeMentionSuggestion) -> Void)?
    var isShowingMentionSuggestions: Bool {
        !mentionButtons.isEmpty && !mentionScrollView.isHidden && scrollView.isHidden
    }

    override var intrinsicContentSize: CGSize {
        let contentHeight = mentionButtons.isEmpty ? Self.baseHeight : Self.mentionRowHeight
        return CGSize(
            width: UIView.noIntrinsicMetric,
            height: contentHeight + (theme?.keyboardOffset ?? Self.defaultKeyboardOffset)
        )
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        translatesAutoresizingMaskIntoConstraints = false
        autoresizingMask = [.flexibleHeight]
        backgroundColor = .clear
        setupView()
        rebuildButtons()
    }

    required init?(coder: NSCoder) {
        return nil
    }

    fileprivate func setItems(_ items: [NativeToolbarItem]) {
        self.items = items
        rebuildButtons()
    }

    func apply(mentionTheme: EditorMentionTheme?) {
        self.mentionTheme = mentionTheme
        for button in mentionButtons {
            button.apply(theme: mentionTheme)
        }
    }

    func apply(theme: EditorToolbarTheme?) {
        self.theme = theme
        chromeView.backgroundColor = theme?.backgroundColor ?? .systemBackground
        chromeView.layer.borderColor = (theme?.borderColor ?? UIColor.separator).cgColor
        chromeView.layer.borderWidth = theme?.borderWidth ?? 0.5
        chromeView.layer.cornerRadius = theme?.borderRadius ?? 0
        chromeView.clipsToBounds = (theme?.borderRadius ?? 0) > 0
        chromeLeadingConstraint?.constant = theme?.horizontalInset ?? Self.defaultHorizontalInset
        chromeTrailingConstraint?.constant = -(theme?.horizontalInset ?? Self.defaultHorizontalInset)
        chromeBottomConstraint?.constant = -(theme?.keyboardOffset ?? Self.defaultKeyboardOffset)
        invalidateIntrinsicContentSize()
        for separator in separators {
            separator.backgroundColor = theme?.separatorColor ?? .separator
        }
        for binding in buttonBindings {
            binding.button.layer.cornerRadius = theme?.buttonBorderRadius ?? 8
        }
        for button in mentionButtons {
            button.apply(theme: mentionTheme)
        }
        apply(state: currentState)
    }

    @discardableResult
    func setMentionSuggestions(_ suggestions: [NativeMentionSuggestion]) -> Bool {
        let hadSuggestions = !mentionButtons.isEmpty

        mentionButtons.forEach { button in
            mentionStackView.removeArrangedSubview(button)
            button.removeFromSuperview()
        }
        mentionButtons.removeAll()

        for suggestion in suggestions.prefix(8) {
            let button = MentionSuggestionChipButton(suggestion: suggestion, theme: mentionTheme)
            button.addTarget(self, action: #selector(handleSelectMentionSuggestion(_:)), for: .touchUpInside)
            mentionButtons.append(button)
            mentionStackView.addArrangedSubview(button)
        }

        let hasSuggestions = !mentionButtons.isEmpty
        mentionScrollView.isHidden = !hasSuggestions
        scrollView.isHidden = hasSuggestions
        mentionRowHeightConstraint?.constant = hasSuggestions ? Self.mentionRowHeight : 0
        invalidateIntrinsicContentSize()
        setNeedsLayout()
        return hadSuggestions != hasSuggestions
    }

    fileprivate func apply(state: NativeToolbarState) {
        currentState = state
        for binding in buttonBindings {
            let buttonState = buttonState(for: binding.item, state: state)
            binding.button.isEnabled = buttonState.enabled
            binding.button.accessibilityTraits = buttonState.active ? [.button, .selected] : .button
            updateButtonAppearance(
                binding.button,
                enabled: buttonState.enabled,
                active: buttonState.active
            )
        }
    }

    private func setupView() {
        chromeView.translatesAutoresizingMaskIntoConstraints = false
        chromeView.backgroundColor = .systemBackground
        chromeView.layer.borderColor = UIColor.separator.cgColor
        chromeView.layer.borderWidth = 0.5
        addSubview(chromeView)

        contentStackView.translatesAutoresizingMaskIntoConstraints = false
        contentStackView.axis = .vertical
        contentStackView.spacing = 0
        chromeView.addSubview(contentStackView)

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

        NSLayoutConstraint.activate([
            chromeView.topAnchor.constraint(equalTo: topAnchor),
            leading,
            trailing,
            bottom,

            contentStackView.topAnchor.constraint(equalTo: chromeView.topAnchor, constant: 6),
            contentStackView.leadingAnchor.constraint(equalTo: chromeView.leadingAnchor),
            contentStackView.trailingAnchor.constraint(equalTo: chromeView.trailingAnchor),
            contentStackView.bottomAnchor.constraint(equalTo: chromeView.safeAreaLayoutGuide.bottomAnchor, constant: -6),

            mentionHeight,

            mentionStackView.topAnchor.constraint(equalTo: mentionScrollView.contentLayoutGuide.topAnchor),
            mentionStackView.leadingAnchor.constraint(equalTo: mentionScrollView.contentLayoutGuide.leadingAnchor, constant: 12),
            mentionStackView.trailingAnchor.constraint(equalTo: mentionScrollView.contentLayoutGuide.trailingAnchor, constant: -12),
            mentionStackView.bottomAnchor.constraint(equalTo: mentionScrollView.contentLayoutGuide.bottomAnchor),
            mentionStackView.heightAnchor.constraint(equalTo: mentionScrollView.frameLayoutGuide.heightAnchor),

            stackView.topAnchor.constraint(equalTo: scrollView.contentLayoutGuide.topAnchor, constant: 6),
            stackView.leadingAnchor.constraint(equalTo: scrollView.contentLayoutGuide.leadingAnchor, constant: 12),
            stackView.trailingAnchor.constraint(equalTo: scrollView.contentLayoutGuide.trailingAnchor, constant: -12),
            stackView.bottomAnchor.constraint(equalTo: scrollView.contentLayoutGuide.bottomAnchor, constant: -6),
            stackView.heightAnchor.constraint(equalTo: scrollView.frameLayoutGuide.heightAnchor, constant: -12),
            scrollView.heightAnchor.constraint(equalToConstant: Self.baseHeight),
        ])

    }

    private func rebuildButtons() {
        buttonBindings.removeAll()
        separators.removeAll()
        for arrangedSubview in stackView.arrangedSubviews {
            stackView.removeArrangedSubview(arrangedSubview)
            arrangedSubview.removeFromSuperview()
        }

        let compactItems = items.enumerated().filter { index, item in
            guard item.type == .separator else { return true }
            guard index > 0, index < items.count - 1 else { return false }
            return items[index - 1].type != .separator && items[index + 1].type != .separator
        }.map(\.element)

        for item in compactItems {
            if item.type == .separator {
                stackView.addArrangedSubview(makeSeparator())
                continue
            }

            let button = makeButton(item: item)
            buttonBindings.append(ButtonBinding(item: item, button: button))
            stackView.addArrangedSubview(button)
        }

        apply(theme: theme)
        apply(state: currentState)
    }

    private func makeButton(item: NativeToolbarItem) -> UIButton {
        let button = UIButton(type: .system)
        button.translatesAutoresizingMaskIntoConstraints = false
        button.titleLabel?.font = .systemFont(ofSize: 16, weight: .semibold)
        button.accessibilityLabel = item.label
        button.layer.cornerRadius = theme?.buttonBorderRadius ?? 8
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
            button.setPreferredSymbolConfiguration(
                UIImage.SymbolConfiguration(pointSize: 16, weight: .semibold),
                forImageIn: .normal
            )
        } else {
            button.setImage(nil, for: .normal)
            button.setTitle(item.icon?.resolvedGlyphText() ?? "?", for: .normal)
        }
        button.widthAnchor.constraint(greaterThanOrEqualToConstant: 36).isActive = true
        button.heightAnchor.constraint(equalToConstant: 36).isActive = true
        button.addAction(UIAction { [weak self] _ in
            self?.onPressItem?(item)
        }, for: .touchUpInside)
        updateButtonAppearance(button, enabled: true, active: false)
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

    private func buttonState(
        for item: NativeToolbarItem,
        state: NativeToolbarState
    ) -> (enabled: Bool, active: Bool) {
        let isInList = state.nodes["bulletList"] == true || state.nodes["orderedList"] == true

        switch item.type {
        case .mark:
            let mark = item.mark ?? ""
            return (
                enabled: state.allowedMarks.contains(mark),
                active: state.marks[mark] == true
            )
        case .list:
            switch item.listType {
            case .bulletList:
                return (
                    enabled: state.commands["wrapBulletList"] == true,
                    active: state.nodes["bulletList"] == true
                )
            case .orderedList:
                return (
                    enabled: state.commands["wrapOrderedList"] == true,
                    active: state.nodes["orderedList"] == true
                )
            case .none:
                return (enabled: false, active: false)
            }
        case .command:
            switch item.command {
            case .indentList:
                return (
                    enabled: isInList && state.commands["indentList"] == true,
                    active: false
                )
            case .outdentList:
                return (
                    enabled: isInList && state.commands["outdentList"] == true,
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
        case .separator:
            return (enabled: false, active: false)
        }
    }

    private func updateButtonAppearance(_ button: UIButton, enabled: Bool, active: Bool) {
        let tintColor: UIColor
        if !enabled {
            tintColor = theme?.buttonDisabledColor ?? .tertiaryLabel
        } else if active {
            tintColor = theme?.buttonActiveColor ?? .systemBlue
        } else {
            tintColor = theme?.buttonColor ?? .secondaryLabel
        }

        button.tintColor = tintColor
        button.setTitleColor(tintColor, for: .normal)
        button.backgroundColor = active
            ? (theme?.buttonActiveBackgroundColor ?? UIColor.systemBlue.withAlphaComponent(0.12))
            : .clear
    }

    @objc private func handleSelectMentionSuggestion(_ sender: MentionSuggestionChipButton) {
        onSelectMentionSuggestion?(sender.suggestion)
    }

    func triggerMentionSuggestionTapForTesting(at index: Int) {
        guard mentionButtons.indices.contains(index) else { return }
        onSelectMentionSuggestion?(mentionButtons[index].suggestion)
    }
}

class NativeEditorExpoView: ExpoView, EditorTextViewDelegate, UIGestureRecognizerDelegate {

    private static let updateLog = Logger(
        subsystem: "com.apollohg.prose-editor",
        category: "view-command"
    )

    // MARK: - Subviews

    let richTextView: RichTextEditorView
    private let accessoryToolbar = EditorAccessoryToolbarView()
    private var toolbarFrameInWindow: CGRect?
    private var didApplyAutoFocus = false
    private var toolbarState = NativeToolbarState.empty
    private var showsToolbar = true
    private var toolbarPlacement = "keyboard"
    private var heightBehavior: EditorHeightBehavior = .fixed
    private var lastAutoGrowWidth: CGFloat = 0
    private var addons = NativeEditorAddons(mentions: nil)
    private var mentionQueryState: MentionQueryState?
    private var lastMentionEventJSON: String?
    private var pendingEditorUpdateJSON: String?
    private var pendingEditorUpdateRevision = 0
    private var appliedEditorUpdateRevision = 0
    private lazy var outsideTapGestureRecognizer: UITapGestureRecognizer = {
        let recognizer = UITapGestureRecognizer(
            target: self,
            action: #selector(handleOutsideTap(_:))
        )
        recognizer.cancelsTouchesInView = false
        recognizer.delegate = self
        return recognizer
    }()
    private weak var gestureWindow: UIWindow?

    /// Guard flag to suppress echo: when JS applies an update via the view
    /// command, the resulting delegate callback must NOT be re-dispatched
    /// back to JS.
    var isApplyingJSUpdate = false

    // MARK: - Event Dispatchers (wired by Expo Modules via reflection)

    let onEditorUpdate = EventDispatcher()
    let onSelectionChange = EventDispatcher()
    let onFocusChange = EventDispatcher()
    let onContentHeightChange = EventDispatcher()
    let onToolbarAction = EventDispatcher()
    let onAddonEvent = EventDispatcher()
    private var lastEmittedContentHeight: CGFloat = 0

    // MARK: - Initialization

    required init(appContext: AppContext? = nil) {
        richTextView = RichTextEditorView(frame: .zero)
        super.init(appContext: appContext)
        richTextView.onHeightMayChange = { [weak self] in
            guard let self, self.heightBehavior == .autoGrow else { return }
            self.invalidateIntrinsicContentSize()
            self.superview?.setNeedsLayout()
            self.emitContentHeightIfNeeded(force: true)
        }
        richTextView.textView.editorDelegate = self
        configureAccessoryToolbar()

        // Observe UITextView focus changes via NotificationCenter.
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(textViewDidBeginEditing(_:)),
            name: UITextView.textDidBeginEditingNotification,
            object: richTextView.textView
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(textViewDidEndEditing(_:)),
            name: UITextView.textDidEndEditingNotification,
            object: richTextView.textView
        )

        addSubview(richTextView)
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    // MARK: - Layout

    override var intrinsicContentSize: CGSize {
        guard heightBehavior == .autoGrow else {
            return CGSize(width: UIView.noIntrinsicMetric, height: UIView.noIntrinsicMetric)
        }
        return richTextView.intrinsicContentSize
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        richTextView.frame = bounds
        guard heightBehavior == .autoGrow else { return }
        let currentWidth = bounds.width.rounded(.towardZero)
        guard currentWidth != lastAutoGrowWidth else { return }
        lastAutoGrowWidth = currentWidth
        invalidateIntrinsicContentSize()
        emitContentHeightIfNeeded(force: true)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if richTextView.textView.isFirstResponder {
            installOutsideTapRecognizerIfNeeded()
        } else {
            uninstallOutsideTapRecognizer()
        }
    }

    // MARK: - Editor Binding

    func setEditorId(_ id: UInt64) {
        richTextView.editorId = id
        if id != 0 {
            let stateJSON = editorGetCurrentState(id: id)
            if let state = NativeToolbarState(updateJSON: stateJSON) {
                toolbarState = state
                accessoryToolbar.apply(state: state)
            } else {
                toolbarState = .empty
                accessoryToolbar.apply(state: .empty)
            }
        } else {
            toolbarState = .empty
            accessoryToolbar.apply(state: .empty)
        }
        refreshMentionQuery()
    }

    func setThemeJson(_ themeJson: String?) {
        let theme = EditorTheme.from(json: themeJson)
        richTextView.applyTheme(theme)
        accessoryToolbar.apply(theme: theme?.toolbar)
        accessoryToolbar.apply(mentionTheme: theme?.mentions ?? addons.mentions?.theme)
        if richTextView.textView.isFirstResponder,
           richTextView.textView.inputAccessoryView === accessoryToolbar
        {
            richTextView.textView.reloadInputViews()
        }
    }

    func setAddonsJson(_ addonsJson: String?) {
        addons = NativeEditorAddons.from(json: addonsJson)
        accessoryToolbar.apply(mentionTheme: richTextView.textView.theme?.mentions ?? addons.mentions?.theme)
        refreshMentionQuery()
    }

    func setEditable(_ editable: Bool) {
        richTextView.textView.isEditable = editable
        updateAccessoryToolbarVisibility()
    }

    func setAutoFocus(_ autoFocus: Bool) {
        guard autoFocus, !didApplyAutoFocus else { return }
        didApplyAutoFocus = true
        focus()
    }

    func setShowToolbar(_ showToolbar: Bool) {
        showsToolbar = showToolbar
        updateAccessoryToolbarVisibility()
    }

    func setToolbarPlacement(_ toolbarPlacement: String?) {
        self.toolbarPlacement = toolbarPlacement == "inline" ? "inline" : "keyboard"
        updateAccessoryToolbarVisibility()
    }

    func setHeightBehavior(_ rawHeightBehavior: String) {
        let nextBehavior = EditorHeightBehavior(rawValue: rawHeightBehavior) ?? .fixed
        guard nextBehavior != heightBehavior else { return }
        heightBehavior = nextBehavior
        richTextView.heightBehavior = nextBehavior
        invalidateIntrinsicContentSize()
        setNeedsLayout()
        if nextBehavior == .autoGrow {
            emitContentHeightIfNeeded(force: true)
        }
    }

    private func emitContentHeightIfNeeded(force: Bool = false) {
        guard heightBehavior == .autoGrow else { return }
        let contentHeight = ceil(richTextView.intrinsicContentSize.height)
        guard contentHeight > 0 else { return }
        guard force || abs(contentHeight - lastEmittedContentHeight) > 0.5 else { return }
        lastEmittedContentHeight = contentHeight
        onContentHeightChange(["contentHeight": contentHeight])
    }

    func setToolbarButtonsJson(_ toolbarButtonsJson: String?) {
        accessoryToolbar.setItems(NativeToolbarItem.from(json: toolbarButtonsJson))
    }

    func setToolbarFrameJson(_ toolbarFrameJson: String?) {
        guard let toolbarFrameJson,
              let data = toolbarFrameJson.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let x = (raw["x"] as? NSNumber)?.doubleValue,
              let y = (raw["y"] as? NSNumber)?.doubleValue,
              let width = (raw["width"] as? NSNumber)?.doubleValue,
              let height = (raw["height"] as? NSNumber)?.doubleValue
        else {
            toolbarFrameInWindow = nil
            return
        }

        toolbarFrameInWindow = CGRect(x: x, y: y, width: width, height: height)
    }

    func setPendingEditorUpdateJson(_ editorUpdateJson: String?) {
        pendingEditorUpdateJSON = editorUpdateJson
    }

    func setPendingEditorUpdateRevision(_ editorUpdateRevision: Int) {
        pendingEditorUpdateRevision = editorUpdateRevision
    }

    func applyPendingEditorUpdateIfNeeded() {
        guard pendingEditorUpdateRevision != 0 else { return }
        guard pendingEditorUpdateRevision != appliedEditorUpdateRevision else { return }
        guard let updateJSON = pendingEditorUpdateJSON else { return }
        appliedEditorUpdateRevision = pendingEditorUpdateRevision
        applyEditorUpdate(updateJSON)
    }

    // MARK: - View Commands

    /// Apply an editor update from JS. Sets the echo-suppression flag so the
    /// resulting delegate callback is NOT re-dispatched back to JS.
    func applyEditorUpdate(_ updateJson: String) {
        Self.updateLog.debug("[applyEditorUpdate.begin] bytes=\(updateJson.utf8.count)")
        isApplyingJSUpdate = true
        richTextView.textView.applyUpdateJSON(updateJson)
        isApplyingJSUpdate = false
        Self.updateLog.debug(
            "[applyEditorUpdate.end] textState=\(self.richTextView.textView.textStorage.string.count)"
        )
    }

    // MARK: - Focus Commands

    func focus() {
        richTextView.textView.becomeFirstResponder()
    }

    func blur() {
        richTextView.textView.resignFirstResponder()
    }

    // MARK: - Focus Notifications

    @objc private func textViewDidBeginEditing(_ notification: Notification) {
        installOutsideTapRecognizerIfNeeded()
        refreshMentionQuery()
        onFocusChange(["isFocused": true])
    }

    @objc private func textViewDidEndEditing(_ notification: Notification) {
        uninstallOutsideTapRecognizer()
        clearMentionQueryStateAndHidePopover()
        onFocusChange(["isFocused": false])
    }

    @objc private func handleOutsideTap(_ recognizer: UITapGestureRecognizer) {
        guard recognizer.state == .ended else { return }
        guard richTextView.textView.isFirstResponder else { return }
        blur()
    }

    private func installOutsideTapRecognizerIfNeeded() {
        guard let window else { return }
        if gestureWindow === window, window.gestureRecognizers?.contains(outsideTapGestureRecognizer) == true {
            return
        }
        uninstallOutsideTapRecognizer()
        window.addGestureRecognizer(outsideTapGestureRecognizer)
        gestureWindow = window
    }

    private func uninstallOutsideTapRecognizer() {
        if let window = gestureWindow {
            window.removeGestureRecognizer(outsideTapGestureRecognizer)
        }
        gestureWindow = nil
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        guard gestureRecognizer === outsideTapGestureRecognizer else { return true }
        if let touchedView = touch.view, touchedView.isDescendant(of: self) {
            return false
        }
        if let touchedView = touch.view, touchedView.isDescendant(of: accessoryToolbar) {
            return false
        }
        if let toolbarFrameInWindow,
           let window = gestureWindow,
           toolbarFrameInWindow.contains(touch.location(in: window))
        {
            return false
        }
        return true
    }

    // MARK: - EditorTextViewDelegate

    func editorTextView(_ textView: EditorTextView, selectionDidChange anchor: UInt32, head: UInt32) {
        refreshToolbarStateFromEditorSelection()
        refreshMentionQuery()
        onSelectionChange(["anchor": Int(anchor), "head": Int(head)])
    }

    func editorTextView(_ textView: EditorTextView, didReceiveUpdate updateJSON: String) {
        if let state = NativeToolbarState(updateJSON: updateJSON) {
            toolbarState = state
            accessoryToolbar.apply(state: state)
        }
        refreshMentionQuery()
        guard !isApplyingJSUpdate else { return }
        Self.updateLog.debug("[didReceiveUpdate] bytes=\(updateJSON.utf8.count)")
        onEditorUpdate(["updateJson": updateJSON])
    }

    private func refreshToolbarStateFromEditorSelection() {
        guard richTextView.editorId != 0 else { return }
        let stateJSON = editorGetCurrentState(id: richTextView.editorId)
        guard let state = NativeToolbarState(updateJSON: stateJSON) else { return }
        toolbarState = state
        accessoryToolbar.apply(state: state)
    }

    private func configureAccessoryToolbar() {
        accessoryToolbar.onPressItem = { [weak self] item in
            self?.handleToolbarItemPress(item)
        }
        accessoryToolbar.onSelectMentionSuggestion = { [weak self] suggestion in
            self?.insertMentionSuggestion(suggestion)
        }
        accessoryToolbar.apply(state: toolbarState)
        updateAccessoryToolbarVisibility()
    }

    private func refreshMentionQuery() {
        guard richTextView.editorId != 0,
              richTextView.textView.isFirstResponder,
              let mentions = addons.mentions
        else {
            clearMentionQueryStateAndHidePopover()
            return
        }

        guard let queryState = currentMentionQueryState(trigger: mentions.trigger) else {
            emitMentionQueryChange(query: "", trigger: mentions.trigger, anchor: 0, head: 0, isActive: false)
            clearMentionQueryStateAndHidePopover()
            return
        }

        let suggestions = filteredMentionSuggestions(for: queryState, config: mentions)
        mentionQueryState = queryState
        accessoryToolbar.apply(mentionTheme: richTextView.textView.theme?.mentions ?? mentions.theme)
        let didChangeToolbarHeight = accessoryToolbar.setMentionSuggestions(suggestions)
        if didChangeToolbarHeight,
           richTextView.textView.isFirstResponder,
           richTextView.textView.inputAccessoryView === accessoryToolbar
        {
            richTextView.textView.reloadInputViews()
        }
        emitMentionQueryChange(
            query: queryState.query,
            trigger: queryState.trigger,
            anchor: queryState.anchor,
            head: queryState.head,
            isActive: true
        )
    }

    private func clearMentionQueryStateAndHidePopover() {
        mentionQueryState = nil
        let didChangeToolbarHeight = accessoryToolbar.setMentionSuggestions([])
        if didChangeToolbarHeight,
           richTextView.textView.isFirstResponder,
           richTextView.textView.inputAccessoryView === accessoryToolbar
        {
            richTextView.textView.reloadInputViews()
        }
    }

    private func emitMentionQueryChange(
        query: String,
        trigger: String,
        anchor: UInt32,
        head: UInt32,
        isActive: Bool
    ) {
        let payload: [String: Any] = [
            "type": "mentionsQueryChange",
            "query": query,
            "trigger": trigger,
            "range": [
                "anchor": Int(anchor),
                "head": Int(head),
            ],
            "isActive": isActive,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }
        guard json != lastMentionEventJSON else { return }
        lastMentionEventJSON = json
        onAddonEvent(["eventJson": json])
    }

    private func emitMentionSelect(trigger: String, suggestion: NativeMentionSuggestion) {
        let payload: [String: Any] = [
            "type": "mentionsSelect",
            "trigger": trigger,
            "suggestionKey": suggestion.key,
            "attrs": suggestion.attrs,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }
        onAddonEvent(["eventJson": json])
    }

    private func filteredMentionSuggestions(
        for queryState: MentionQueryState,
        config: NativeMentionsAddonConfig
    ) -> [NativeMentionSuggestion] {
        let query = queryState.query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else {
            return config.suggestions
        }

        return config.suggestions.filter { suggestion in
            suggestion.title.lowercased().contains(query)
                || suggestion.label.lowercased().contains(query)
                || (suggestion.subtitle?.lowercased().contains(query) ?? false)
        }
    }

    private func currentMentionQueryState(trigger: String) -> MentionQueryState? {
        guard let selectedTextRange = richTextView.textView.selectedTextRange,
              selectedTextRange.isEmpty
        else {
            return nil
        }

        let currentText = richTextView.textView.text ?? ""
        let cursorUtf16Offset = richTextView.textView.offset(
            from: richTextView.textView.beginningOfDocument,
            to: selectedTextRange.start
        )
        let visibleCursorScalar = PositionBridge.utf16OffsetToScalar(
            cursorUtf16Offset,
            in: currentText
        )

        guard let visibleQueryState = resolveMentionQueryState(
            in: currentText,
            cursorScalar: visibleCursorScalar,
            trigger: trigger,
            isCaretInsideMention: isCaretInsideMention(
                cursorScalar: PositionBridge.textViewToScalar(
                    selectedTextRange.start,
                    in: richTextView.textView
                )
            )
        ) else {
            return nil
        }

        let anchorUtf16Offset = PositionBridge.scalarToUtf16Offset(
            visibleQueryState.anchor,
            in: currentText
        )
        let headUtf16Offset = PositionBridge.scalarToUtf16Offset(
            visibleQueryState.head,
            in: currentText
        )

        return MentionQueryState(
            query: visibleQueryState.query,
            trigger: visibleQueryState.trigger,
            anchor: PositionBridge.utf16OffsetToScalar(
                anchorUtf16Offset,
                in: richTextView.textView
            ),
            head: PositionBridge.utf16OffsetToScalar(
                headUtf16Offset,
                in: richTextView.textView
            )
        )
    }

    private func isCaretInsideMention(cursorScalar: UInt32) -> Bool {
        let utf16Offset = PositionBridge.scalarToUtf16Offset(
            cursorScalar,
            in: richTextView.textView.text ?? ""
        )
        let textStorage = richTextView.textView.textStorage
        guard textStorage.length > 0 else { return false }
        let candidateOffsets = [
            min(max(utf16Offset, 0), max(textStorage.length - 1, 0)),
            min(max(utf16Offset - 1, 0), max(textStorage.length - 1, 0)),
        ]

        for offset in candidateOffsets where offset >= 0 && offset < textStorage.length {
            if let nodeType = textStorage.attribute(RenderBridgeAttributes.voidNodeType, at: offset, effectiveRange: nil) as? String,
               nodeType == "mention" {
                return true
            }
        }
        return false
    }

    private func insertMentionSuggestion(_ suggestion: NativeMentionSuggestion) {
        guard let mentions = addons.mentions,
              let queryState = mentionQueryState
        else {
            return
        }

        var attrs = suggestion.attrs
        if attrs["label"] == nil {
            attrs["label"] = suggestion.label
        }
        let payload: [String: Any] = [
            "type": "doc",
            "content": [[
                "type": "mention",
                "attrs": attrs,
            ]],
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }

        let updateJSON = editorInsertContentJsonAtSelectionScalar(
            id: richTextView.editorId,
            scalarAnchor: queryState.anchor,
            scalarHead: queryState.head,
            json: json
        )
        richTextView.textView.applyUpdateJSON(updateJSON)
        emitMentionSelect(trigger: mentions.trigger, suggestion: suggestion)
        lastMentionEventJSON = nil
        clearMentionQueryStateAndHidePopover()
    }

    func setMentionQueryStateForTesting(_ state: MentionQueryState?) {
        mentionQueryState = state
    }

    func currentMentionQueryStateForTesting(trigger: String) -> MentionQueryState? {
        currentMentionQueryState(trigger: trigger)
    }

    func setMentionSuggestionsForTesting(_ suggestions: [NativeMentionSuggestion]) {
        accessoryToolbar.setMentionSuggestions(suggestions)
    }

    func triggerMentionSuggestionTapForTesting(at index: Int) {
        accessoryToolbar.triggerMentionSuggestionTapForTesting(at: index)
    }
    private func updateAccessoryToolbarVisibility() {
        let nextAccessoryView: UIView? = showsToolbar &&
            toolbarPlacement == "keyboard" &&
            richTextView.textView.isEditable
            ? accessoryToolbar
            : nil
        if richTextView.textView.inputAccessoryView !== nextAccessoryView {
            richTextView.textView.inputAccessoryView = nextAccessoryView
            if richTextView.textView.isFirstResponder {
                richTextView.textView.reloadInputViews()
            }
        }
    }

    private func handleListToggle(_ listType: String) {
        let isActive = toolbarState.nodes[listType] == true
        richTextView.textView.performToolbarToggleList(listType, isActive: isActive)
    }

    private func handleToolbarItemPress(_ item: NativeToolbarItem) {
        switch item.type {
        case .mark:
            guard let mark = item.mark else { return }
            richTextView.textView.performToolbarToggleMark(mark)
        case .list:
            guard let listType = item.listType?.rawValue else { return }
            handleListToggle(listType)
        case .command:
            switch item.command {
            case .indentList:
                richTextView.textView.performToolbarIndentListItem()
            case .outdentList:
                richTextView.textView.performToolbarOutdentListItem()
            case .undo:
                richTextView.textView.performToolbarUndo()
            case .redo:
                richTextView.textView.performToolbarRedo()
            case .none:
                break
            }
        case .node:
            guard let nodeType = item.nodeType else { return }
            richTextView.textView.performToolbarInsertNode(nodeType)
        case .action:
            guard let key = item.key else { return }
            onToolbarAction(["key": key])
        case .separator:
            break
        }
    }
}
