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
}

struct NativeMentionsAddonConfig {
    let trigger: String
    let suggestions: [NativeMentionSuggestion]
    let theme: EditorMentionTheme?

    init?(dictionary: [String: Any]) {
        let trigger = (dictionary["trigger"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines)
        self.trigger = (trigger?.isEmpty == false ? trigger : "@") ?? "@"
        self.suggestions = ((dictionary["suggestions"] as? [[String: Any]]) ?? []).compactMap(NativeMentionSuggestion.init(dictionary:))
        if let theme = dictionary["theme"] as? [String: Any] {
            self.theme = EditorMentionTheme(dictionary: theme)
        } else {
            self.theme = nil
        }
    }
}

struct NativeEditorAddons {
    let mentions: NativeMentionsAddonConfig?

    static func from(json: String?) -> NativeEditorAddons {
        guard let json,
              let data = json.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return NativeEditorAddons(mentions: nil)
        }

        return NativeEditorAddons(
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
    private let titleLabelView = UILabel()
    private let subtitleLabelView = UILabel()
    private let stackView = UIStackView()
    private var theme: EditorMentionTheme?
    private var toolbarAppearance: EditorToolbarAppearance = .custom

    let suggestion: NativeMentionSuggestion

    init(
        suggestion: NativeMentionSuggestion,
        theme: EditorMentionTheme?,
        toolbarAppearance: EditorToolbarAppearance = .custom
    ) {
        self.suggestion = suggestion
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
        titleLabelView.text = suggestion.label
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
            stackView.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            stackView.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            stackView.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            stackView.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
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

    func apply(theme: EditorMentionTheme?, toolbarAppearance: EditorToolbarAppearance = .custom) {
        self.theme = theme
        self.toolbarAppearance = toolbarAppearance
        layer.cornerRadius = theme?.borderRadius ?? 12
        layer.borderColor = (theme?.borderColor ?? UIColor.clear).cgColor
        layer.borderWidth = theme?.borderWidth ?? 0
        subtitleLabelView.isHidden = suggestion.subtitle == nil
        updateAppearance(highlighted: isHighlighted)
    }

    override var isHighlighted: Bool {
        didSet {
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
            backgroundColor = highlighted
                ? UIColor.white.withAlphaComponent(0.18)
                : .clear
            titleLabelView.textColor = .label
            subtitleLabelView.textColor = .secondaryLabel
            return
        }

        backgroundColor = highlighted
            ? (theme?.optionHighlightedBackgroundColor ?? UIColor.systemBlue.withAlphaComponent(0.12))
            : (theme?.backgroundColor ?? UIColor.secondarySystemBackground)
        titleLabelView.textColor = highlighted
            ? (theme?.optionHighlightedTextColor ?? theme?.optionTextColor ?? .label)
            : (theme?.optionTextColor ?? theme?.textColor ?? .label)
        subtitleLabelView.textColor = theme?.optionSecondaryTextColor ?? .secondaryLabel
    }

    func contentViewsAllowTouchPassthroughForTesting() -> Bool {
        !stackView.isUserInteractionEnabled
            && !titleLabelView.isUserInteractionEnabled
            && !subtitleLabelView.isUserInteractionEnabled
    }

    func usesNativeAppearanceForTesting() -> Bool {
        toolbarAppearance == .native
    }
}
