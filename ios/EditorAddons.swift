import UIKit

struct NativeMentionSuggestion {
    let key: String
    let title: String
    let subtitle: String?
    let label: String
    let attrs: [String: Any]

    init?(dictionary: [String: Any]) {
        guard let key = dictionary["key"] as? String,
              let title = dictionary["title"] as? String,
              let label = dictionary["label"] as? String
        else {
            return nil
        }

        self.key = key
        self.title = title
        self.subtitle = dictionary["subtitle"] as? String
        self.label = label
        self.attrs = dictionary["attrs"] as? [String: Any] ?? [:]
    }

    func displayLabel(trigger: String) -> String {
        let trimmedLabel = label.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trigger.isEmpty, !trimmedLabel.hasPrefix(trigger) else {
            return trimmedLabel
        }
        return "\(trigger)\(trimmedLabel)"
    }
}

struct NativeMentionsAddonConfig {
    let trigger: String
    let suggestions: [NativeMentionSuggestion]
    let theme: EditorMentionTheme?
    let resolveSelectionAttrs: Bool
    let resolveTheme: Bool

    init?(dictionary: [String: Any]) {
        let trigger = (dictionary["trigger"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
        self.trigger = (trigger?.isEmpty == false ? trigger : "@") ?? "@"
        self.suggestions = ((dictionary["suggestions"] as? [[String: Any]]) ?? []).compactMap(NativeMentionSuggestion.init(dictionary:))
        self.resolveSelectionAttrs = dictionary["resolveSelectionAttrs"] as? Bool ?? false
        self.resolveTheme = dictionary["resolveTheme"] as? Bool ?? false
        if let theme = dictionary["theme"] as? [String: Any] {
            self.theme = EditorMentionTheme(dictionary: theme)
        } else {
            self.theme = nil
        }
    }
}

struct NativeEditorAddons {
    var codeHighlighting: NativeCodeHighlightConfiguration? = nil
    let mentions: NativeMentionsAddonConfig?

    static func from(json: String?) -> NativeEditorAddons {
        guard let json,
              let data = json.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return NativeEditorAddons(mentions: nil)
        }

        return NativeEditorAddons(
            codeHighlighting: NativeCodeHighlightConfiguration.from(raw["codeHighlighting"]),
            mentions: (raw["mentions"] as? [String: Any]).flatMap(NativeMentionsAddonConfig.init(dictionary:))
        )
    }
}

struct MentionQueryState: Equatable {
    let query: String
    let trigger: String
    let anchor: UInt32
    let head: UInt32
}

func isMentionIdentifierScalar(_ scalar: Unicode.Scalar) -> Bool {
    CharacterSet.alphanumerics.contains(scalar) || scalar == "_" || scalar == "-"
}

func resolveMentionQueryState(
    in text: String,
    cursorScalar: UInt32,
    trigger: String,
    isCaretInsideMention: Bool
) -> MentionQueryState? {
    guard !isCaretInsideMention else { return nil }

    let scalars = Array(text.unicodeScalars)
    let scalarCount = UInt32(scalars.count)
    guard cursorScalar <= scalarCount else { return nil }

    let triggerScalars = Array(trigger.unicodeScalars)
    guard triggerScalars.count == 1, let triggerScalar = triggerScalars.first else {
        return nil
    }

    var start = Int(cursorScalar)
    while start > 0 {
        let previous = scalars[start - 1]
        if previous.properties.isWhitespace
            || previous == "\n"
            || previous == "\u{FFFC}"
            || (!isMentionIdentifierScalar(previous) && previous != triggerScalar)
        {
            break
        }
        start -= 1
    }

    guard start < scalars.count, scalars[start] == triggerScalar else {
        return nil
    }
    if start > 0 {
        let previous = scalars[start - 1]
        if isMentionIdentifierScalar(previous) {
            return nil
        }
    }

    let queryStart = start + 1
    let cursor = Int(cursorScalar)
    guard queryStart <= cursor else { return nil }
    let query = String(String.UnicodeScalarView(scalars[queryStart..<cursor]))
    if query.unicodeScalars.contains(where: {
        $0.properties.isWhitespace || $0 == "\n" || $0 == "\u{FFFC}"
    }) {
        return nil
    }

    return MentionQueryState(
        query: query,
        trigger: trigger,
        anchor: UInt32(start),
        head: cursorScalar
    )
}

final class MentionSuggestionChipButton: UIButton {
    private static let horizontalContentInset: CGFloat = 8
    private static let verticalContentInset: CGFloat = 8

    private let titleLabelView = UILabel()
    private let subtitleLabelView = UILabel()
    private let stackView = UIStackView()
    private var displayLabel: String
    private var theme: EditorMentionTheme?
    private var toolbarAppearance: EditorToolbarAppearance = .custom

    private(set) var suggestion: NativeMentionSuggestion

    init(
        suggestion: NativeMentionSuggestion,
        trigger: String = "@",
        theme: EditorMentionTheme?,
        toolbarAppearance: EditorToolbarAppearance = .custom
    ) {
        self.suggestion = suggestion
        self.displayLabel = suggestion.displayLabel(trigger: trigger)
        self.theme = theme
        self.toolbarAppearance = toolbarAppearance
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        layer.cornerRadius = 12
        clipsToBounds = true
        if #available(iOS 15.0, *) {
            var configuration = UIButton.Configuration.plain()
            configuration.contentInsets = .zero
            self.configuration = configuration
        }

        titleLabelView.translatesAutoresizingMaskIntoConstraints = false
        titleLabelView.isUserInteractionEnabled = false
        titleLabelView.font = .systemFont(ofSize: 14, weight: .semibold)
        titleLabelView.text = displayLabel
        titleLabelView.numberOfLines = 1

        subtitleLabelView.translatesAutoresizingMaskIntoConstraints = false
        subtitleLabelView.isUserInteractionEnabled = false
        subtitleLabelView.font = .systemFont(ofSize: 12)
        subtitleLabelView.text = suggestion.subtitle
        subtitleLabelView.numberOfLines = 1
        subtitleLabelView.isHidden = suggestion.subtitle == nil

        stackView.translatesAutoresizingMaskIntoConstraints = false
        stackView.isUserInteractionEnabled = false
        stackView.axis = .vertical
        stackView.alignment = .fill
        stackView.spacing = 1
        stackView.addArrangedSubview(titleLabelView)
        stackView.addArrangedSubview(subtitleLabelView)
        addSubview(stackView)

        NSLayoutConstraint.activate([
            stackView.topAnchor.constraint(equalTo: topAnchor, constant: Self.verticalContentInset),
            stackView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Self.horizontalContentInset),
            stackView.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -Self.horizontalContentInset),
            stackView.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -Self.verticalContentInset),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 40),
        ])

        addTarget(self, action: #selector(handleTouchDown), for: [.touchDown, .touchDragEnter])
        addTarget(self, action: #selector(handleTouchUp), for: [.touchCancel, .touchDragExit, .touchUpInside, .touchUpOutside])
        apply(theme: theme, toolbarAppearance: toolbarAppearance)
        updateAppearance(highlighted: false)
    }

    required init?(coder: NSCoder) {
        return nil
    }

    func update(suggestion: NativeMentionSuggestion, trigger: String) {
        self.suggestion = suggestion
        displayLabel = suggestion.displayLabel(trigger: trigger)
        titleLabelView.text = displayLabel
        subtitleLabelView.text = suggestion.subtitle
        subtitleLabelView.isHidden = suggestion.subtitle == nil
        updateAppearance(highlighted: isHighlighted)
    }

    func apply(theme: EditorMentionTheme?, toolbarAppearance: EditorToolbarAppearance = .custom) {
        self.theme = theme
        self.toolbarAppearance = toolbarAppearance
        let option = theme?.suggestions?.option
        layer.cornerRadius = option?.borderRadius ?? 12
        layer.borderColor = (option?.borderColor ?? UIColor.clear).cgColor
        layer.borderWidth = option?.borderWidth ?? 0
        subtitleLabelView.isHidden = suggestion.subtitle == nil
        updateAppearance(highlighted: isHighlighted)
    }

    override var isHighlighted: Bool {
        didSet {
            updateAppearance(highlighted: isHighlighted)
        }
    }

    override func tintColorDidChange() {
        super.tintColorDidChange()
        if toolbarAppearance == .native {
            updateAppearance(highlighted: isHighlighted)
        }
    }

    @objc private func handleTouchDown() {
        updateAppearance(highlighted: true)
    }

    @objc private func handleTouchUp() {
        updateAppearance(highlighted: false)
    }

    private func updateAppearance(highlighted: Bool) {
        if toolbarAppearance == .native {
            layer.cornerRadius = 18
            layer.borderColor = UIColor.clear.cgColor
            layer.borderWidth = 0
            #if compiler(>=6.2)
            if #available(iOS 26.0, *) {
                stackView.isHidden = true
                backgroundColor = .clear
                var configuration = highlighted
                    ? UIButton.Configuration.prominentGlass()
                    : UIButton.Configuration.glass()
                configuration.cornerStyle = .capsule
                configuration.contentInsets = NSDirectionalEdgeInsets(
                    top: Self.verticalContentInset,
                    leading: Self.horizontalContentInset,
                    bottom: Self.verticalContentInset,
                    trailing: Self.horizontalContentInset
                )
                configuration.title = displayLabel
                configuration.subtitle = suggestion.subtitle
                configuration.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { incoming in
                    var outgoing = incoming
                    outgoing.font = .systemFont(ofSize: 14, weight: .semibold)
                    return outgoing
                }
                self.configuration = configuration
                return
            }
            #endif
            stackView.isHidden = false
            backgroundColor = highlighted
                ? UIColor.white.withAlphaComponent(0.18)
                : .clear
            titleLabelView.textColor = tintColor
            subtitleLabelView.textColor = tintColor.withAlphaComponent(0.72)
            return
        }

        stackView.isHidden = false
        if #available(iOS 15.0, *) {
            var configuration = UIButton.Configuration.plain()
            configuration.contentInsets = .zero
            self.configuration = configuration
        }
        let option = theme?.suggestions?.option
        backgroundColor = highlighted
            ? (option?.highlightedBackgroundColor ?? UIColor.systemBlue.withAlphaComponent(0.12))
            : (option?.backgroundColor ?? UIColor.secondarySystemBackground)
        titleLabelView.textColor = highlighted
            ? (option?.highlightedTextColor ?? option?.textColor ?? .label)
            : (option?.textColor ?? .label)
        subtitleLabelView.textColor = option?.secondaryTextColor ?? .secondaryLabel
    }

    func contentViewsAllowTouchPassthroughForTesting() -> Bool {
        !stackView.isUserInteractionEnabled
            && !titleLabelView.isUserInteractionEnabled
            && !subtitleLabelView.isUserInteractionEnabled
    }

    func usesNativeAppearanceForTesting() -> Bool {
        toolbarAppearance == .native
    }

    func titleTextColorForTesting() -> UIColor? {
        titleLabelView.textColor
    }

    func subtitleTextColorForTesting() -> UIColor? {
        subtitleLabelView.textColor
    }

    func usesNativeGlassTextRenderingForTesting() -> Bool {
        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            return toolbarAppearance == .native
                && stackView.isHidden
                && configuration?.title == displayLabel
        }
        #endif
        return false
    }

    func titleTextForTesting() -> String? {
        titleLabelView.text
    }

    func usesNativeGlassSemiboldTitleForTesting() -> Bool {
        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            return toolbarAppearance == .native
                && stackView.isHidden
                && configuration?.titleTextAttributesTransformer != nil
        }
        #endif
        return false
    }
}
