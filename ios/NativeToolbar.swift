import UIKit

struct NativeToolbarState {
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

enum ToolbarCommand: String {
    case indentList
    case outdentList
    case undo
    case redo
}

enum ToolbarListType: String {
    case bullet_list
    case ordered_list
    case bulletList
    case orderedList
}

enum ToolbarDefaultIconId: String {
    case bold
    case italic
    case underline
    case strike
    case link
    case image
    case h1
    case h2
    case h3
    case h4
    case h5
    case h6
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

enum ToolbarItemKind: String {
    case mark
    case heading
    case blockquote
    case list
    case command
    case node
    case action
    case group
    case separator
}

enum ToolbarGroupPresentation: String {
    case expand
    case menu
}

enum ToolbarItemPlacement: String {
    case start
    case scroll
    case end
}

struct NativeToolbarIcon {
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
        .image: "photo",
        .blockquote: "text.quote",
        .bulletList: "list.bullet",
        .orderedList: "list.number",
        .indentList: "increase.indent",
        .outdentList: "decrease.indent",
        .lineBreak: "return.left",
        .horizontalRule: "minus",
        .h1: "paragraphsign",
        .h2: "paragraphsign",
        .h3: "paragraphsign",
        .h4: "paragraphsign",
        .h5: "paragraphsign",
        .h6: "paragraphsign",
        .undo: "arrow.uturn.backward",
        .redo: "arrow.uturn.forward",
    ]

    private static let defaultGlyphs: [ToolbarDefaultIconId: String] = [
        .bold: "B",
        .italic: "I",
        .underline: "U",
        .strike: "S",
        .link: "🔗",
        .image: "🖼",
        .h1: "H1",
        .h2: "H2",
        .h3: "H3",
        .h4: "H4",
        .h5: "H5",
        .h6: "H6",
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

struct NativeToolbarItem {
    let type: ToolbarItemKind
    var key: String? = nil
    var label: String? = nil
    var icon: NativeToolbarIcon? = nil
    var mark: String? = nil
    var headingLevel: Int? = nil
    var listType: ToolbarListType? = nil
    var command: ToolbarCommand? = nil
    var nodeType: String? = nil
    var isActive: Bool = false
    var isDisabled: Bool = false
    var placement: ToolbarItemPlacement? = nil
    var presentation: ToolbarGroupPresentation? = nil
    var items: [NativeToolbarItem] = []
    var buttonStyle: EditorToolbarButtonStyle? = nil
    var parentGroupKey: String? = nil

    static let defaults: [NativeToolbarItem] = [
        NativeToolbarItem(type: .mark, label: "Bold", icon: .defaultIcon(.bold), mark: "bold"),
        NativeToolbarItem(type: .mark, label: "Italic", icon: .defaultIcon(.italic), mark: "italic"),
        NativeToolbarItem(type: .mark, label: "Underline", icon: .defaultIcon(.underline), mark: "underline"),
        NativeToolbarItem(type: .mark, label: "Strikethrough", icon: .defaultIcon(.strike), mark: "strike"),
        NativeToolbarItem(type: .blockquote, label: "Blockquote", icon: .defaultIcon(.blockquote)),
        NativeToolbarItem(type: .separator),
        NativeToolbarItem(type: .list, label: "Bullet List", icon: .defaultIcon(.bulletList), listType: .bullet_list),
        NativeToolbarItem(type: .list, label: "Ordered List", icon: .defaultIcon(.orderedList), listType: .ordered_list),
        NativeToolbarItem(type: .command, label: "Indent List", icon: .defaultIcon(.indentList), command: .indentList),
        NativeToolbarItem(type: .command, label: "Outdent List", icon: .defaultIcon(.outdentList), command: .outdentList),
        NativeToolbarItem(type: .node, label: "Line Break", icon: .defaultIcon(.lineBreak), nodeType: "hard_break"),
        NativeToolbarItem(type: .node, label: "Horizontal Rule", icon: .defaultIcon(.horizontalRule), nodeType: "horizontal_rule"),
        NativeToolbarItem(type: .separator),
        NativeToolbarItem(type: .command, label: "Undo", icon: .defaultIcon(.undo), command: .undo),
        NativeToolbarItem(type: .command, label: "Redo", icon: .defaultIcon(.redo), command: .redo),
    ]

    private static func parse(
        rawItem: [String: Any],
        allowGroup: Bool = true,
        allowSeparator: Bool = true
    ) -> NativeToolbarItem? {
        guard let rawType = rawItem["type"] as? String,
              let type = ToolbarItemKind(rawValue: rawType)
        else {
            return nil
        }

        let key = rawItem["key"] as? String
        let placement = (rawItem["placement"] as? String)
            .flatMap(ToolbarItemPlacement.init(rawValue:))
        let buttonStyle = (rawItem["buttonStyle"] as? [String: Any]).map(
            EditorToolbarButtonStyle.init(dictionary:)
        )
        switch type {
        case .separator:
            guard allowSeparator else { return nil }
            return NativeToolbarItem(
                type: .separator,
                key: key,
                placement: placement,
                buttonStyle: buttonStyle
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
                placement: placement,
                buttonStyle: buttonStyle
            )
        case .heading:
            guard let level = (rawItem["level"] as? NSNumber)?.intValue,
                  (1...6).contains(level),
                  let label = rawItem["label"] as? String,
                  let icon = NativeToolbarIcon.from(jsonValue: rawItem["icon"])
            else {
                return nil
            }
            return NativeToolbarItem(
                type: .heading,
                key: key,
                label: label,
                icon: icon,
                headingLevel: level,
                placement: placement,
                buttonStyle: buttonStyle
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
                placement: placement,
                buttonStyle: buttonStyle
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
                listType: listType,
                placement: placement,
                buttonStyle: buttonStyle
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
                command: command,
                placement: placement,
                buttonStyle: buttonStyle
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
                nodeType: nodeType,
                placement: placement,
                buttonStyle: buttonStyle
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
                isActive: (rawItem["isActive"] as? Bool) ?? false,
                isDisabled: (rawItem["isDisabled"] as? Bool) ?? false,
                placement: placement,
                buttonStyle: buttonStyle
            )
        case .group:
            guard allowGroup,
                  let key,
                  let label = rawItem["label"] as? String,
                  let icon = NativeToolbarIcon.from(jsonValue: rawItem["icon"]),
                  let rawChildren = rawItem["items"] as? [[String: Any]]
            else {
                return nil
            }
            let presentation = (rawItem["presentation"] as? String)
                .flatMap(ToolbarGroupPresentation.init(rawValue:))
                ?? .expand
            let children = rawChildren.compactMap {
                parse(rawItem: $0, allowGroup: false, allowSeparator: false)
            }
            guard !children.isEmpty else { return nil }
            return NativeToolbarItem(
                type: .group,
                key: key,
                label: label,
                icon: icon,
                placement: placement,
                presentation: presentation,
                items: children,
                buttonStyle: buttonStyle
            )
        }
    }

    static func from(json: String?) -> [NativeToolbarItem] {
        guard let json,
              let data = json.data(using: .utf8),
              let rawItems = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else {
            return defaults
        }

        let parsed = rawItems.compactMap { parse(rawItem: $0) }
        return parsed.isEmpty ? defaults : parsed
    }

    func resolvedKey(index: Int) -> String {
        if let key {
            return key
        }
        switch type {
        case .mark:
            return "mark:\(mark ?? ""):\(index)"
        case .heading:
            return "heading:\(headingLevel ?? 0):\(index)"
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
        case .group:
            return "group:\(key ?? ""):\(index)"
        case .separator:
            return "separator:\(index)"
        }
    }

    func with(
        parentGroupKey: String?,
        inheritedPlacement: ToolbarItemPlacement? = nil
    ) -> NativeToolbarItem {
        var copy = self
        copy.placement = placement ?? inheritedPlacement
        copy.parentGroupKey = parentGroupKey
        return copy
    }
}

@available(iOS 16.0, *)
private final class ToolbarEditMenuPresenter: NSObject, UIEditMenuInteractionDelegate {
    private final class Presentation {
        weak var sourceButton: UIButton?
        let menuProvider: () -> UIMenu?

        init(sourceButton: UIButton, menuProvider: @escaping () -> UIMenu?) {
            self.sourceButton = sourceButton
            self.menuProvider = menuProvider
        }
    }

    lazy var interaction = UIEditMenuInteraction(delegate: self)

    private var presentations: [String: Presentation] = [:]
    private var activePresentationIdentifier: String?
    private(set) var presentationRequestCount = 0

    func toggle(from sourceButton: UIButton, menuProvider: @escaping () -> UIMenu?) {
        if activePresentation?.sourceButton === sourceButton {
            interaction.dismissMenu()
            return
        }
        guard let hostView = interaction.view else { return }
        let identifier = UUID().uuidString
        presentations[identifier] = Presentation(
            sourceButton: sourceButton,
            menuProvider: menuProvider
        )
        activePresentationIdentifier = identifier
        presentationRequestCount += 1
        let sourcePoint = sourceButton.convert(
            CGPoint(x: sourceButton.bounds.midX, y: sourceButton.bounds.midY),
            to: hostView
        )
        interaction.presentEditMenu(
            with: UIEditMenuConfiguration(identifier: identifier as NSString, sourcePoint: sourcePoint)
        )
    }

    func reloadVisibleMenu() {
        interaction.reloadVisibleMenu()
    }

    func dismiss() {
        interaction.dismissMenu()
        presentations.removeAll()
        activePresentationIdentifier = nil
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction,
        menuFor configuration: UIEditMenuConfiguration,
        suggestedActions: [UIMenuElement]
    ) -> UIMenu? {
        presentation(for: configuration)?.menuProvider()
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction,
        targetRectFor configuration: UIEditMenuConfiguration
    ) -> CGRect {
        guard let sourceButton = presentation(for: configuration)?.sourceButton,
              let hostView = interaction.view
        else {
            return .null
        }
        return sourceButton.convert(sourceButton.bounds, to: hostView)
    }

    func editMenuInteraction(
        _ interaction: UIEditMenuInteraction,
        willDismissMenuFor configuration: UIEditMenuConfiguration,
        animator: any UIEditMenuInteractionAnimating
    ) {
        guard let identifier = identifier(for: configuration) else { return }
        animator.addCompletion { [weak self] in
            guard let self else { return }
            self.presentations.removeValue(forKey: identifier)
            if self.activePresentationIdentifier == identifier {
                self.activePresentationIdentifier = nil
            }
        }
    }

    private var activePresentation: Presentation? {
        guard let activePresentationIdentifier else { return nil }
        return presentations[activePresentationIdentifier]
    }

    private func presentation(for configuration: UIEditMenuConfiguration) -> Presentation? {
        guard let identifier = identifier(for: configuration) else { return activePresentation }
        return presentations[identifier]
    }

    private func identifier(for configuration: UIEditMenuConfiguration) -> String? {
        (configuration.identifier as? NSString).map(String.init)
    }
}

final class EditorAccessoryToolbarView: UIInputView {
    private static let baseHeight: CGFloat = 50
    private static let mentionRowHeight: CGFloat = 52
    private static let contentSpacing: CGFloat = 6
    private static let contentHorizontalInset: CGFloat = 12
    private static let defaultHorizontalInset: CGFloat = 0
    private static let defaultKeyboardOffset: CGFloat = 0
    private static let chromeTransitionDuration: TimeInterval = 0.18
    private static let nativeDisabledButtonOpacity: CGFloat = 0.46

    private struct ButtonBinding {
        let item: NativeToolbarItem
        let button: UIButton
        let widthConstraint: NSLayoutConstraint
        let heightConstraint: NSLayoutConstraint
    }

    private struct VisibleToolbarItemsByPlacement {
        let start: [NativeToolbarItem]
        let scroll: [NativeToolbarItem]
        let end: [NativeToolbarItem]
    }

    private let chromeView = UIView()
    private let blurView = UIVisualEffectView(effect: nil)
    private let glassTintView = UIView()
    private let bodyStackView = UIStackView()
    private let startPinnedStackView = UIStackView()
    private let contentStackView = UIStackView()
    private let endPinnedStackView = UIStackView()
    private let mentionScrollView = UIScrollView()
    private let mentionStackView = UIStackView()
    private let scrollView = UIScrollView()
    private let stackView = UIStackView()
    private var chromeLeadingConstraint: NSLayoutConstraint?
    private var chromeTrailingConstraint: NSLayoutConstraint?
    private var chromeBottomConstraint: NSLayoutConstraint?
    private var mentionRowHeightConstraint: NSLayoutConstraint?
    private var scrollViewHeightConstraint: NSLayoutConstraint?
    private var buttonBindings: [ButtonBinding] = []
    private var separators: [UIView] = []
    private var mentionButtons: [MentionSuggestionChipButton] = []
    private var items: [NativeToolbarItem] = NativeToolbarItem.defaults
    private var expandedGroupKey: String?
    private var currentState = NativeToolbarState.empty
    private var theme: EditorToolbarTheme?
    private var mentionTheme: EditorMentionTheme?
    private var didAnimateChromeTransition = false
    private var editMenuPresenter: AnyObject?
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
    func setNativeToolbarContentOffsetXForTesting(_ offsetX: CGFloat) {
        scrollView.contentOffset.x = offsetX
    }
    var selectedButtonCountForTesting: Int {
        buttonBindings.filter(\.button.isSelected).count
    }
    var editMenuPresentationRequestCountForTesting: Int {
        guard #available(iOS 16.0, *) else { return 0 }
        return (editMenuPresenter as? ToolbarEditMenuPresenter)?.presentationRequestCount ?? 0
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

    private func apply(theme: EditorToolbarTheme?, animateChrome: Bool) {
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
    func firstButtonTitleColorForTesting(_ state: UIControl.State) -> UIColor? {
        buttonBindings.first?.button.titleColor(for: state)
    }
    var firstButtonTintAdjustmentModeForTesting: UIView.TintAdjustmentMode {
        buttonBindings.first?.button.tintAdjustmentMode ?? .automatic
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

    private func rebuildButtons() {
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

    private func rebuildButtons(items: [NativeToolbarItem], in container: UIStackView) {
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

    private func buttonState(
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

    private func updateButtonAppearance(
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

    private var resolvedToolbarHeight: CGFloat {
        max(theme?.height ?? Self.baseHeight, 1)
    }

    private var resolvedButtonSize: CGFloat {
        if theme?.height == nil {
            return 36
        }
        return max(1, min(40, resolvedToolbarHeight - 4))
    }

    private var resolvedButtonBorderRadius: CGFloat {
        theme?.resolvedButtonBorderRadius ?? 8
    }

    private func resolvedButtonBorderRadius(for item: NativeToolbarItem) -> CGFloat {
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

    private var usesTransparentMentionChrome: Bool {
        guard resolvedAppearance == .native, !mentionButtons.isEmpty else { return false }
        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            return true
        }
        #endif
        return false
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

    @objc private func handleSelectMentionSuggestion(_ sender: MentionSuggestionChipButton) {
        onSelectMentionSuggestion?(sender.suggestion)
    }

    func triggerMentionSuggestionTapForTesting(at index: Int) {
        guard mentionButtons.indices.contains(index) else { return }
        onSelectMentionSuggestion?(mentionButtons[index].suggestion)
    }
}

/// Keeps iOS keyboard integrations on the inputAccessoryView path when the
/// visible toolbar is rendered outside the native keyboard accessory.
final class EditorAccessoryPlaceholderView: UIView {
    override init(frame: CGRect) {
        super.init(
            frame: CGRect(
                x: frame.origin.x,
                y: frame.origin.y,
                width: frame.width,
                height: 0
            )
        )
        commonInit()
    }

    required init?(coder: NSCoder) {
        return nil
    }

    override var intrinsicContentSize: CGSize {
        CGSize(width: UIView.noIntrinsicMetric, height: 0)
    }

    override func sizeThatFits(_ size: CGSize) -> CGSize {
        CGSize(width: size.width, height: 0)
    }

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        false
    }

    private func commonInit() {
        frame.size.height = 0
        backgroundColor = .clear
        isOpaque = false
        isUserInteractionEnabled = false
        autoresizingMask = [.flexibleWidth]
    }
}
