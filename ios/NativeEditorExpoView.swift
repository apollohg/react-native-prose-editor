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
    case link
    case blockquote
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
    case blockquote
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
        .link: "link",
        .blockquote: "text.quote",
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
        .link: "🔗",
        .blockquote: "❝",
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
        NativeToolbarItem(type: .blockquote, key: nil, label: "Blockquote", icon: .defaultIcon(.blockquote), mark: nil, listType: nil, command: nil, nodeType: nil, isActive: false, isDisabled: false),
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
            case .blockquote:
                guard let label = rawItem["label"] as? String,
                      let icon = NativeToolbarIcon.from(jsonValue: rawItem["icon"])
                else {
                    return nil
                }
                return NativeToolbarItem(
                    type: .blockquote,
                    key: key,
                    label: label,
                    icon: icon,
                    mark: nil,
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
        case .blockquote:
            return "blockquote:\(index)"
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

final class EditorAccessoryToolbarView: UIInputView {
    private static let baseHeight: CGFloat = 50
    private static let mentionRowHeight: CGFloat = 52
    private static let contentSpacing: CGFloat = 6
    private static let defaultHorizontalInset: CGFloat = 0
    private static let defaultKeyboardOffset: CGFloat = 0

    private struct ButtonBinding {
        let item: NativeToolbarItem
        let button: UIButton
    }

    private struct BarButtonBinding {
        let item: NativeToolbarItem
        let button: UIBarButtonItem
    }

    private let chromeView = UIView()
    private let blurView = UIVisualEffectView(effect: nil)
    private let glassTintView = UIView()
    private let nativeToolbarScrollView = UIScrollView()
    private let nativeToolbarView = UIToolbar()
    private let contentStackView = UIStackView()
    private let mentionScrollView = UIScrollView()
    private let mentionStackView = UIStackView()
    private let scrollView = UIScrollView()
    private let stackView = UIStackView()
    private var chromeLeadingConstraint: NSLayoutConstraint?
    private var chromeTrailingConstraint: NSLayoutConstraint?
    private var chromeBottomConstraint: NSLayoutConstraint?
    private var nativeToolbarWidthConstraint: NSLayoutConstraint?
    private var mentionRowHeightConstraint: NSLayoutConstraint?
    private var nativeToolbarDidInitializeScrollPosition = false
    private var buttonBindings: [ButtonBinding] = []
    private var barButtonBindings: [BarButtonBinding] = []
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
    var nativeToolbarVisibleWidthForTesting: CGFloat {
        activeNativeToolbarScrollViewForTesting.bounds.width
    }
    var nativeToolbarContentWidthForTesting: CGFloat {
        if usesNativeBarToolbar {
            return max(nativeToolbarScrollView.contentSize.width, nativeToolbarView.bounds.width)
        }
        return max(scrollView.contentSize.width, stackView.bounds.width)
    }
    var nativeToolbarContentOffsetXForTesting: CGFloat {
        activeNativeToolbarScrollViewForTesting.contentOffset.x
    }
    func setNativeToolbarContentOffsetXForTesting(_ offsetX: CGFloat) {
        activeNativeToolbarScrollViewForTesting.contentOffset.x = offsetX
    }
    var selectedButtonCountForTesting: Int {
#if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            if usesNativeBarToolbar {
                return barButtonBindings.filter { $0.button.style == .prominent }.count
            }
        }
#endif
        return buttonBindings.filter(\.button.isSelected).count
    }
    func mentionButtonAtForTesting(_ index: Int) -> MentionSuggestionChipButton? {
        mentionButtons.indices.contains(index) ? mentionButtons[index] : nil
    }

    override var intrinsicContentSize: CGSize {
        let contentHeight = mentionButtons.isEmpty ? Self.baseHeight : Self.mentionRowHeight
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
        rebuildButtons()
    }

    required init?(coder: NSCoder) {
        return nil
    }

    override func didMoveToSuperview() {
        super.didMoveToSuperview()
        refreshNativeHostTransparencyIfNeeded()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        refreshNativeHostTransparencyIfNeeded()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        updateNativeToolbarMetricsIfNeeded()
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
        let usesNativeAppearance = resolvedAppearance == .native
        let hasFloatingGlassButtons = self.usesFloatingGlassButtons
        let usesBarToolbar = usesNativeBarToolbar
        chromeView.backgroundColor = usesNativeAppearance
            ? .clear
            : (theme?.backgroundColor ?? .systemBackground)
        chromeView.tintColor = usesNativeAppearance
            ? nil
            : (theme?.buttonColor ?? tintColor)
        chromeView.isOpaque = false
        blurView.isHidden = usesBarToolbar || !usesNativeAppearance
        blurView.effect = usesNativeAppearance ? resolvedBlurEffect() : nil
        blurView.alpha = usesNativeAppearance ? resolvedEffectAlpha : 1
        glassTintView.isHidden = usesBarToolbar || !usesNativeAppearance
        glassTintView.backgroundColor = usesNativeAppearance
            ? UIColor.systemBackground.withAlphaComponent(resolvedGlassTintAlpha)
            : .clear
        chromeView.layer.borderColor = resolvedBorderColor.cgColor
        chromeView.layer.borderWidth = usesBarToolbar
            ? 0
            : (usesNativeAppearance
            ? (1 / UIScreen.main.scale)
            : resolvedBorderWidth)
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
        chromeView.clipsToBounds = (usesNativeAppearance && !hasFloatingGlassButtons && !usesBarToolbar) || resolvedBorderRadius > 0
        chromeView.layer.shadowOpacity = usesNativeAppearance && !hasFloatingGlassButtons && !usesBarToolbar ? 0.08 : 0
        chromeView.layer.shadowRadius = usesNativeAppearance && !hasFloatingGlassButtons && !usesBarToolbar ? 10 : 0
        chromeView.layer.shadowOffset = CGSize(width: 0, height: 2)
        chromeView.layer.shadowColor = UIColor.black.cgColor
        chromeLeadingConstraint?.constant = resolvedHorizontalInset
        chromeTrailingConstraint?.constant = -resolvedHorizontalInset
        chromeBottomConstraint?.constant = -resolvedKeyboardOffset
        nativeToolbarScrollView.isHidden = !(usesBarToolbar && mentionButtons.isEmpty)
        nativeToolbarView.isHidden = !(usesBarToolbar && mentionButtons.isEmpty)
        nativeToolbarView.tintColor = usesNativeAppearance
            ? nil
            : (theme?.buttonColor ?? tintColor)
        contentStackView.isHidden = usesBarToolbar && mentionButtons.isEmpty
        invalidateIntrinsicContentSize()
        for separator in separators {
            separator.isHidden = hasFloatingGlassButtons
            separator.backgroundColor = usesNativeAppearance
                ? UIColor.separator.withAlphaComponent(0.45)
                : (theme?.separatorColor ?? .separator)
        }
        for binding in buttonBindings {
            binding.button.layer.cornerRadius = resolvedButtonBorderRadius
        }
        for button in mentionButtons {
            button.apply(theme: mentionTheme, toolbarAppearance: resolvedAppearance)
        }
        refreshNativeHostTransparencyIfNeeded()
        updateNativeToolbarMetricsIfNeeded()
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
            let button = MentionSuggestionChipButton(
                suggestion: suggestion,
                theme: mentionTheme,
                toolbarAppearance: resolvedAppearance
            )
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
            binding.button.isSelected = buttonState.active
            binding.button.accessibilityTraits = buttonState.active ? [.button, .selected] : .button
            updateButtonAppearance(binding.button, item: binding.item, enabled: buttonState.enabled, active: buttonState.active)
        }

        #if compiler(>=6.2)
        if #available(iOS 26.0, *), usesNativeBarToolbar {
            for binding in barButtonBindings {
                let state = buttonState(for: binding.item, state: currentState)
                binding.button.isEnabled = state.enabled
                binding.button.isSelected = state.active
                binding.button.style = state.active ? .prominent : .plain
            }
        }
        #endif
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

        nativeToolbarScrollView.translatesAutoresizingMaskIntoConstraints = false
        nativeToolbarScrollView.isHidden = true
        nativeToolbarScrollView.backgroundColor = .clear
        nativeToolbarScrollView.showsHorizontalScrollIndicator = false
        nativeToolbarScrollView.showsVerticalScrollIndicator = false
        nativeToolbarScrollView.alwaysBounceHorizontal = true
        nativeToolbarScrollView.alwaysBounceVertical = false
        chromeView.addSubview(nativeToolbarScrollView)

        nativeToolbarView.translatesAutoresizingMaskIntoConstraints = false
        nativeToolbarView.isHidden = true
        nativeToolbarView.backgroundColor = .clear
        nativeToolbarView.isTranslucent = true
        nativeToolbarView.setContentHuggingPriority(.required, for: .vertical)
        nativeToolbarView.setContentCompressionResistancePriority(.required, for: .vertical)
        nativeToolbarScrollView.addSubview(nativeToolbarView)

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
        let nativeToolbarWidth = nativeToolbarView.widthAnchor.constraint(greaterThanOrEqualToConstant: Self.baseHeight)
        nativeToolbarWidthConstraint = nativeToolbarWidth

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

            nativeToolbarScrollView.topAnchor.constraint(equalTo: chromeView.topAnchor),
            nativeToolbarScrollView.leadingAnchor.constraint(equalTo: chromeView.leadingAnchor),
            nativeToolbarScrollView.trailingAnchor.constraint(equalTo: chromeView.trailingAnchor),
            nativeToolbarScrollView.bottomAnchor.constraint(equalTo: chromeView.bottomAnchor),

            nativeToolbarView.topAnchor.constraint(equalTo: nativeToolbarScrollView.contentLayoutGuide.topAnchor),
            nativeToolbarView.leadingAnchor.constraint(equalTo: nativeToolbarScrollView.contentLayoutGuide.leadingAnchor),
            nativeToolbarView.trailingAnchor.constraint(equalTo: nativeToolbarScrollView.contentLayoutGuide.trailingAnchor),
            nativeToolbarView.bottomAnchor.constraint(equalTo: nativeToolbarScrollView.contentLayoutGuide.bottomAnchor),
            nativeToolbarView.heightAnchor.constraint(equalTo: nativeToolbarScrollView.frameLayoutGuide.heightAnchor),
            nativeToolbarView.heightAnchor.constraint(greaterThanOrEqualToConstant: Self.baseHeight),
            nativeToolbarWidth,

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
        barButtonBindings.removeAll()
        separators.removeAll()
        nativeToolbarDidInitializeScrollPosition = false
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

        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            nativeToolbarView.setItems(makeNativeToolbarItems(from: compactItems), animated: false)
        } else {
            nativeToolbarView.setItems([], animated: false)
        }
        #else
        nativeToolbarView.setItems([], animated: false)
        #endif

        updateNativeToolbarMetricsIfNeeded()
        apply(theme: theme)
        apply(state: currentState)
    }

    private func updateNativeToolbarMetricsIfNeeded() {
#if compiler(>=6.2)
        guard #available(iOS 26.0, *), usesNativeBarToolbar else {
            nativeToolbarWidthConstraint?.constant = Self.baseHeight
            nativeToolbarDidInitializeScrollPosition = false
            return
        }

        let availableWidth = max(chromeView.bounds.width, bounds.width, 1)
        let targetHeight = max(chromeView.bounds.height, Self.baseHeight)
        nativeToolbarView.layoutIfNeeded()
        let fittingSize = nativeToolbarView.sizeThatFits(
            CGSize(width: CGFloat.greatestFiniteMagnitude, height: targetHeight)
        )
        let contentFrames = nativeToolbarView.subviews.compactMap { subview -> CGRect? in
            guard !subview.isHidden,
                  subview.alpha > 0.01,
                  subview.bounds.width > 0,
                  subview.bounds.height > 0
            else {
                return nil
            }
            return subview.frame
        }
        let measuredSubviewWidth: CGFloat
        if let minX = contentFrames.map(\.minX).min(),
           let maxX = contentFrames.map(\.maxX).max()
        {
            measuredSubviewWidth = ceil(maxX + max(0, minX))
        } else {
            measuredSubviewWidth = 0
        }
        let contentWidth = max(ceil(fittingSize.width), measuredSubviewWidth, availableWidth)
        nativeToolbarWidthConstraint?.constant = contentWidth
        nativeToolbarScrollView.alwaysBounceHorizontal = contentWidth > availableWidth
        let minOffsetX = -nativeToolbarScrollView.adjustedContentInset.left
        let maxOffsetX = max(
            minOffsetX,
            contentWidth - nativeToolbarScrollView.bounds.width + nativeToolbarScrollView.adjustedContentInset.right
        )
        let targetOffsetX: CGFloat
        if nativeToolbarDidInitializeScrollPosition {
            targetOffsetX = min(max(nativeToolbarScrollView.contentOffset.x, minOffsetX), maxOffsetX)
        } else {
            targetOffsetX = minOffsetX
            nativeToolbarDidInitializeScrollPosition = true
        }
        if abs(nativeToolbarScrollView.contentOffset.x - targetOffsetX) > 0.5 {
            nativeToolbarScrollView.setContentOffset(
                CGPoint(x: targetOffsetX, y: nativeToolbarScrollView.contentOffset.y),
                animated: false
            )
        }
#else
        nativeToolbarWidthConstraint?.constant = Self.baseHeight
        nativeToolbarDidInitializeScrollPosition = false
#endif
    }

#if compiler(>=6.2)
    @available(iOS 26.0, *)
    private func makeNativeToolbarItems(from compactItems: [NativeToolbarItem]) -> [UIBarButtonItem] {
        var toolbarItems: [UIBarButtonItem] = []
        var previousWasSeparator = true

        for item in compactItems {
            if item.type == .separator {
                if !previousWasSeparator, !toolbarItems.isEmpty {
                    let spacer = UIBarButtonItem(systemItem: .fixedSpace)
                    spacer.width = 0
                    toolbarItems.append(spacer)
                }
                previousWasSeparator = true
                continue
            }

            let state = buttonState(for: item, state: currentState)
            let barButtonItem = makeNativeBarButtonItem(item: item, enabled: state.enabled, active: state.active)
            barButtonBindings.append(BarButtonBinding(item: item, button: barButtonItem))
            toolbarItems.append(barButtonItem)
            previousWasSeparator = false
        }

        return toolbarItems
    }

    @available(iOS 26.0, *)
    private func makeNativeBarButtonItem(
        item: NativeToolbarItem,
        enabled: Bool,
        active: Bool
    ) -> UIBarButtonItem {
        let image = item.icon?.resolvedSFSymbolName().flatMap { UIImage(systemName: $0) }
        let title = image == nil ? item.icon?.resolvedGlyphText() : nil
        let action = UIAction { [weak self] _ in
            self?.onPressItem?(item)
        }
        let barButtonItem = UIBarButtonItem(title: title, image: image, primaryAction: action, menu: nil)

        barButtonItem.accessibilityLabel = item.label
        barButtonItem.isEnabled = enabled
        barButtonItem.isSelected = active
        barButtonItem.style = active ? .prominent : .plain
        barButtonItem.hidesSharedBackground = active
        return barButtonItem
    }
#endif

    private func makeButton(item: NativeToolbarItem) -> UIButton {
        let button = UIButton(type: .system)
        button.translatesAutoresizingMaskIntoConstraints = false
        button.titleLabel?.font = .systemFont(ofSize: 16, weight: .semibold)
        button.accessibilityLabel = item.label
        button.layer.cornerRadius = resolvedButtonBorderRadius
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
        updateButtonAppearance(button, item: item, enabled: true, active: false)
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
        case .blockquote:
            return (
                enabled: state.commands["toggleBlockquote"] == true,
                active: state.nodes["blockquote"] == true
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

    private func updateButtonAppearance(
        _ button: UIButton,
        item: NativeToolbarItem,
        enabled: Bool,
        active: Bool
    ) {
        #if compiler(>=6.2)
        if #available(iOS 26.0, *), usesFloatingGlassButtons {
            var configuration = active
                ? UIButton.Configuration.prominentGlass()
                : UIButton.Configuration.glass()
            configuration.cornerStyle = .capsule
            configuration.contentInsets = NSDirectionalEdgeInsets(
                top: 8,
                leading: 10,
                bottom: 8,
                trailing: 10
            )
            configuration.preferredSymbolConfigurationForImage = UIImage.SymbolConfiguration(
                pointSize: 16,
                weight: .semibold
            )
            if let symbolName = item.icon?.resolvedSFSymbolName(),
               let symbolImage = UIImage(systemName: symbolName)
            {
                configuration.image = symbolImage
                configuration.title = nil
            } else {
                configuration.image = nil
                configuration.title = item.icon?.resolvedGlyphText() ?? "?"
            }
            button.configuration = configuration
            button.titleLabel?.font = UIFont.systemFont(ofSize: 16, weight: .semibold)
            button.alpha = enabled ? 1 : 0.45
            return
        }
        #endif

        if resolvedAppearance == .native {
            button.tintColor = nil
            button.alpha = enabled ? 1 : 0.45
            button.tintAdjustmentMode = enabled ? .automatic : .dimmed
            button.backgroundColor = active
                ? UIColor.white.withAlphaComponent(0.18)
                : .clear
            return
        }

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
        button.alpha = enabled ? 1 : 0.7
        button.backgroundColor = active
            ? (theme?.buttonActiveBackgroundColor ?? UIColor.systemBlue.withAlphaComponent(0.12))
            : .clear
    }

    private var resolvedAppearance: EditorToolbarAppearance {
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

    private var resolvedButtonBorderRadius: CGFloat {
        theme?.resolvedButtonBorderRadius ?? 8
    }

    private var usesFloatingGlassButtons: Bool {
        return false
    }

    private var usesNativeBarToolbar: Bool {
        return false
    }

    private var activeNativeToolbarScrollViewForTesting: UIScrollView {
        usesNativeBarToolbar ? nativeToolbarScrollView : scrollView
    }

    private func resolvedBlurEffect() -> UIVisualEffect {
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

    private var resolvedGlassEffectTintColor: UIColor {
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

    private func refreshNativeHostTransparencyIfNeeded() {
        guard usesFloatingGlassButtons else { return }
        backgroundColor = .clear
        isOpaque = false

        var ancestor: UIView? = self
        while let view = ancestor {
            let className = NSStringFromClass(type(of: view))
            if view === self || className.contains("UIInput") || className.contains("Accessory") {
                view.backgroundColor = .clear
                view.isOpaque = false
            }
            ancestor = view.superview
        }
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
    private let accessoryToolbar = EditorAccessoryToolbarView(
        frame: .zero,
        inputViewStyle: .keyboard
    )
    private var toolbarFrameInWindow: CGRect?
    private var didApplyAutoFocus = false
    private var toolbarState = NativeToolbarState.empty
    private var toolbarItems: [NativeToolbarItem] = NativeToolbarItem.defaults
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
        refreshSystemAssistantToolbarIfNeeded()
        refreshMentionQuery()
    }

    func setThemeJson(_ themeJson: String?) {
        let theme = EditorTheme.from(json: themeJson)
        richTextView.applyTheme(theme)
        accessoryToolbar.apply(theme: theme?.toolbar)
        accessoryToolbar.apply(mentionTheme: theme?.mentions ?? addons.mentions?.theme)
        refreshSystemAssistantToolbarIfNeeded()
        if richTextView.textView.isFirstResponder,
           (richTextView.textView.inputAccessoryView === accessoryToolbar || shouldUseSystemAssistantToolbar)
        {
            richTextView.textView.reloadInputViews()
        }
    }

    func setAddonsJson(_ addonsJson: String?) {
        addons = NativeEditorAddons.from(json: addonsJson)
        accessoryToolbar.apply(mentionTheme: richTextView.textView.theme?.mentions ?? addons.mentions?.theme)
        refreshMentionQuery()
    }

    func setRemoteSelectionsJson(_ remoteSelectionsJson: String?) {
        richTextView.setRemoteSelections(RemoteSelectionDecoration.from(json: remoteSelectionsJson))
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
        toolbarItems = NativeToolbarItem.from(json: toolbarButtonsJson)
        accessoryToolbar.setItems(toolbarItems)
        refreshSystemAssistantToolbarIfNeeded()
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
        guard let tapWindow = gestureWindow ?? window else { return }
        let locationInWindow = recognizer.location(in: tapWindow)
        guard shouldHandleOutsideTap(locationInWindow: locationInWindow, touchedView: nil) else {
            return
        }
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
        guard let tapWindow = gestureWindow ?? window else { return true }
        return shouldHandleOutsideTap(
            locationInWindow: touch.location(in: tapWindow),
            touchedView: touch.view
        )
    }

    private func shouldHandleOutsideTap(
        locationInWindow: CGPoint,
        touchedView: UIView?
    ) -> Bool {
        if let touchedView, touchedView.isDescendant(of: self) {
            return false
        }
        if let tapWindow = gestureWindow ?? window {
            let editorFrameInWindow = convert(bounds, to: tapWindow)
            if editorFrameInWindow.contains(locationInWindow) {
                return false
            }
        }
        if let touchedView, touchedView.isDescendant(of: accessoryToolbar) {
            return false
        }
        if let toolbarFrameInWindow, toolbarFrameInWindow.contains(locationInWindow) {
            return false
        }
        return true
    }

    // MARK: - EditorTextViewDelegate

    func editorTextView(_ textView: EditorTextView, selectionDidChange anchor: UInt32, head: UInt32) {
        refreshToolbarStateFromEditorSelection()
        refreshSystemAssistantToolbarIfNeeded()
        refreshMentionQuery()
        richTextView.refreshRemoteSelections()
        onSelectionChange(["anchor": Int(anchor), "head": Int(head)])
    }

    func editorTextView(_ textView: EditorTextView, didReceiveUpdate updateJSON: String) {
        if let state = NativeToolbarState(updateJSON: updateJSON) {
            toolbarState = state
            accessoryToolbar.apply(state: state)
            refreshSystemAssistantToolbarIfNeeded()
        }
        refreshMentionQuery()
        richTextView.refreshRemoteSelections()
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
        accessoryToolbar.setItems(toolbarItems)
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
        refreshSystemAssistantToolbarIfNeeded()
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
        refreshSystemAssistantToolbarIfNeeded()
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
        refreshSystemAssistantToolbarIfNeeded()
        let nextAccessoryView: UIView? = showsToolbar &&
            toolbarPlacement == "keyboard" &&
            richTextView.textView.isEditable &&
            !shouldUseSystemAssistantToolbar
            ? accessoryToolbar
            : nil
        if richTextView.textView.inputAccessoryView !== nextAccessoryView {
            richTextView.textView.inputAccessoryView = nextAccessoryView
            if richTextView.textView.isFirstResponder {
                richTextView.textView.reloadInputViews()
            }
        }
    }

    private var shouldUseSystemAssistantToolbar: Bool {
        false
    }

    private func refreshSystemAssistantToolbarIfNeeded() {
        guard #available(iOS 26.0, *) else { return }

        let assistantItem = richTextView.textView.inputAssistantItem
        assistantItem.allowsHidingShortcuts = false
        assistantItem.leadingBarButtonGroups = []
        assistantItem.trailingBarButtonGroups = []
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
        case .blockquote:
            richTextView.textView.performToolbarToggleBlockquote()
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
