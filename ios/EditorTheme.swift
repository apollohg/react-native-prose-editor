import UIKit

struct EditorTextStyle {
    var fontFamily: String?
    var fontSize: CGFloat?
    var fontWeight: String?
    var fontStyle: String?
    var color: UIColor?
    var lineHeight: CGFloat?
    var spacingAfter: CGFloat?

    init(
        fontFamily: String? = nil,
        fontSize: CGFloat? = nil,
        fontWeight: String? = nil,
        fontStyle: String? = nil,
        color: UIColor? = nil,
        lineHeight: CGFloat? = nil,
        spacingAfter: CGFloat? = nil
    ) {
        self.fontFamily = fontFamily
        self.fontSize = fontSize
        self.fontWeight = fontWeight
        self.fontStyle = fontStyle
        self.color = color
        self.lineHeight = lineHeight
        self.spacingAfter = spacingAfter
    }

    init(dictionary: [String: Any]) {
        fontFamily = dictionary["fontFamily"] as? String
        fontSize = EditorTheme.cgFloat(dictionary["fontSize"])
        fontWeight = dictionary["fontWeight"] as? String
        fontStyle = dictionary["fontStyle"] as? String
        color = EditorTheme.color(from: dictionary["color"])
        lineHeight = EditorTheme.cgFloat(dictionary["lineHeight"])
        spacingAfter = EditorTheme.cgFloat(dictionary["spacingAfter"])
    }

    func merged(with override: EditorTextStyle?) -> EditorTextStyle {
        guard let override else { return self }
        return EditorTextStyle(
            fontFamily: override.fontFamily ?? fontFamily,
            fontSize: override.fontSize ?? fontSize,
            fontWeight: override.fontWeight ?? fontWeight,
            fontStyle: override.fontStyle ?? fontStyle,
            color: override.color ?? color,
            lineHeight: override.lineHeight ?? lineHeight,
            spacingAfter: override.spacingAfter ?? spacingAfter
        )
    }

    func resolvedFont(fallback: UIFont) -> UIFont {
        let size = fontSize ?? fallback.pointSize
        var font = fallback.withSize(size)

        if let fontFamily,
           let familyFont = UIFont(name: fontFamily, size: size) {
            font = familyFont
        } else if let fontWeight {
            font = UIFont.systemFont(ofSize: size, weight: EditorTheme.fontWeight(from: fontWeight))
        }

        var traits = font.fontDescriptor.symbolicTraits
        if EditorTheme.shouldApplyBoldTrait(fontWeight) {
            traits.insert(.traitBold)
        }
        if fontStyle == "italic" {
            traits.insert(.traitItalic)
        }

        if traits != font.fontDescriptor.symbolicTraits,
           let descriptor = font.fontDescriptor.withSymbolicTraits(traits) {
            font = UIFont(descriptor: descriptor, size: size)
        }

        return font
    }
}

struct EditorListTheme {
    var indent: CGFloat?
    var baseIndentMultiplier: CGFloat?
    var itemSpacing: CGFloat?
    var markerColor: UIColor?
    var markerScale: CGFloat?

    init(dictionary: [String: Any]) {
        indent = EditorTheme.cgFloat(dictionary["indent"])
        baseIndentMultiplier = EditorTheme.cgFloat(dictionary["baseIndentMultiplier"])
        itemSpacing = EditorTheme.cgFloat(dictionary["itemSpacing"])
        markerColor = EditorTheme.color(from: dictionary["markerColor"])
        markerScale = EditorTheme.cgFloat(dictionary["markerScale"])
    }
}

struct EditorHorizontalRuleTheme {
    var color: UIColor?
    var thickness: CGFloat?
    var verticalMargin: CGFloat?

    init(dictionary: [String: Any]) {
        color = EditorTheme.color(from: dictionary["color"])
        thickness = EditorTheme.cgFloat(dictionary["thickness"])
        verticalMargin = EditorTheme.cgFloat(dictionary["verticalMargin"])
    }
}

struct EditorBlockquoteTheme {
    var text: EditorTextStyle?
    var indent: CGFloat?
    var borderColor: UIColor?
    var borderWidth: CGFloat?
    var markerGap: CGFloat?

    init(dictionary: [String: Any]) {
        if let text = dictionary["text"] as? [String: Any] {
            self.text = EditorTextStyle(dictionary: text)
        }
        indent = EditorTheme.cgFloat(dictionary["indent"])
        borderColor = EditorTheme.color(from: dictionary["borderColor"])
        borderWidth = EditorTheme.cgFloat(dictionary["borderWidth"])
        markerGap = EditorTheme.cgFloat(dictionary["markerGap"])
    }
}

struct EditorMentionTheme {
    var textColor: UIColor?
    var backgroundColor: UIColor?
    var borderColor: UIColor?
    var borderWidth: CGFloat?
    var borderRadius: CGFloat?
    var fontWeight: String?
    var popoverBackgroundColor: UIColor?
    var popoverBorderColor: UIColor?
    var popoverBorderWidth: CGFloat?
    var popoverBorderRadius: CGFloat?
    var popoverShadowColor: UIColor?
    var optionTextColor: UIColor?
    var optionSecondaryTextColor: UIColor?
    var optionHighlightedBackgroundColor: UIColor?
    var optionHighlightedTextColor: UIColor?

    func merged(with override: EditorMentionTheme?) -> EditorMentionTheme {
        guard let override else { return self }
        var merged = self
        merged.textColor = override.textColor ?? merged.textColor
        merged.backgroundColor = override.backgroundColor ?? merged.backgroundColor
        merged.borderColor = override.borderColor ?? merged.borderColor
        merged.borderWidth = override.borderWidth ?? merged.borderWidth
        merged.borderRadius = override.borderRadius ?? merged.borderRadius
        merged.fontWeight = override.fontWeight ?? merged.fontWeight
        merged.popoverBackgroundColor =
            override.popoverBackgroundColor ?? merged.popoverBackgroundColor
        merged.popoverBorderColor = override.popoverBorderColor ?? merged.popoverBorderColor
        merged.popoverBorderWidth = override.popoverBorderWidth ?? merged.popoverBorderWidth
        merged.popoverBorderRadius = override.popoverBorderRadius ?? merged.popoverBorderRadius
        merged.popoverShadowColor = override.popoverShadowColor ?? merged.popoverShadowColor
        merged.optionTextColor = override.optionTextColor ?? merged.optionTextColor
        merged.optionSecondaryTextColor =
            override.optionSecondaryTextColor ?? merged.optionSecondaryTextColor
        merged.optionHighlightedBackgroundColor =
            override.optionHighlightedBackgroundColor ?? merged.optionHighlightedBackgroundColor
        merged.optionHighlightedTextColor =
            override.optionHighlightedTextColor ?? merged.optionHighlightedTextColor
        return merged
    }

    init(dictionary: [String: Any]) {
        textColor = EditorTheme.color(from: dictionary["textColor"])
        backgroundColor = EditorTheme.color(from: dictionary["backgroundColor"])
        borderColor = EditorTheme.color(from: dictionary["borderColor"])
        borderWidth = EditorTheme.cgFloat(dictionary["borderWidth"])
        borderRadius = EditorTheme.cgFloat(dictionary["borderRadius"])
        fontWeight = dictionary["fontWeight"] as? String
        popoverBackgroundColor = EditorTheme.color(from: dictionary["popoverBackgroundColor"])
        popoverBorderColor = EditorTheme.color(from: dictionary["popoverBorderColor"])
        popoverBorderWidth = EditorTheme.cgFloat(dictionary["popoverBorderWidth"])
        popoverBorderRadius = EditorTheme.cgFloat(dictionary["popoverBorderRadius"])
        popoverShadowColor = EditorTheme.color(from: dictionary["popoverShadowColor"])
        optionTextColor = EditorTheme.color(from: dictionary["optionTextColor"])
        optionSecondaryTextColor = EditorTheme.color(from: dictionary["optionSecondaryTextColor"])
        optionHighlightedBackgroundColor = EditorTheme.color(from: dictionary["optionHighlightedBackgroundColor"])
        optionHighlightedTextColor = EditorTheme.color(from: dictionary["optionHighlightedTextColor"])
    }
}

enum EditorToolbarAppearance: String {
    case custom
    case native
}

struct EditorToolbarTheme {
    var appearance: EditorToolbarAppearance?
    var backgroundColor: UIColor?
    var borderColor: UIColor?
    var borderWidth: CGFloat?
    var borderRadius: CGFloat?
    var marginTop: CGFloat?
    var showTopBorder: Bool?
    var keyboardOffset: CGFloat?
    var horizontalInset: CGFloat?
    var separatorColor: UIColor?
    var buttonColor: UIColor?
    var buttonActiveColor: UIColor?
    var buttonDisabledColor: UIColor?
    var buttonActiveBackgroundColor: UIColor?
    var buttonBorderRadius: CGFloat?

    init(dictionary: [String: Any]) {
        appearance = (dictionary["appearance"] as? String).flatMap(EditorToolbarAppearance.init(rawValue:))
        backgroundColor = EditorTheme.color(from: dictionary["backgroundColor"])
        borderColor = EditorTheme.color(from: dictionary["borderColor"])
        borderWidth = EditorTheme.cgFloat(dictionary["borderWidth"])
        borderRadius = EditorTheme.cgFloat(dictionary["borderRadius"])
        marginTop = EditorTheme.cgFloat(dictionary["marginTop"])
        showTopBorder = dictionary["showTopBorder"] as? Bool
        keyboardOffset = EditorTheme.cgFloat(dictionary["keyboardOffset"])
        horizontalInset = EditorTheme.cgFloat(dictionary["horizontalInset"])
        separatorColor = EditorTheme.color(from: dictionary["separatorColor"])
        buttonColor = EditorTheme.color(from: dictionary["buttonColor"])
        buttonActiveColor = EditorTheme.color(from: dictionary["buttonActiveColor"])
        buttonDisabledColor = EditorTheme.color(from: dictionary["buttonDisabledColor"])
        buttonActiveBackgroundColor = EditorTheme.color(from: dictionary["buttonActiveBackgroundColor"])
        buttonBorderRadius = EditorTheme.cgFloat(dictionary["buttonBorderRadius"])
    }

    var resolvedKeyboardOffset: CGFloat {
        keyboardOffset ?? (appearance == .native ? 6 : 0)
    }

    var resolvedHorizontalInset: CGFloat {
        horizontalInset ?? (appearance == .native ? 10 : 0)
    }

    var resolvedBorderRadius: CGFloat {
        borderRadius ?? (appearance == .native ? 20 : 0)
    }

    var resolvedBorderWidth: CGFloat {
        borderWidth ?? (appearance == .native ? 0 : 0.5)
    }

    var resolvedButtonBorderRadius: CGFloat {
        buttonBorderRadius ?? (appearance == .native ? 10 : 8)
    }
}

struct EditorContentInsets {
    var top: CGFloat?
    var right: CGFloat?
    var bottom: CGFloat?
    var left: CGFloat?

    init(dictionary: [String: Any]) {
        top = EditorTheme.cgFloat(dictionary["top"])
        right = EditorTheme.cgFloat(dictionary["right"])
        bottom = EditorTheme.cgFloat(dictionary["bottom"])
        left = EditorTheme.cgFloat(dictionary["left"])
    }
}

struct EditorTheme {
    var text: EditorTextStyle?
    var paragraph: EditorTextStyle?
    var blockquote: EditorBlockquoteTheme?
    var headings: [String: EditorTextStyle] = [:]
    var list: EditorListTheme?
    var horizontalRule: EditorHorizontalRuleTheme?
    var mentions: EditorMentionTheme?
    var toolbar: EditorToolbarTheme?
    var backgroundColor: UIColor?
    var borderRadius: CGFloat?
    var contentInsets: EditorContentInsets?

    static func from(json: String?) -> EditorTheme? {
        guard let json, !json.isEmpty,
              let data = json.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }
        return EditorTheme(dictionary: raw)
    }

    init(dictionary: [String: Any]) {
        if let text = dictionary["text"] as? [String: Any] {
            self.text = EditorTextStyle(dictionary: text)
        }
        if let paragraph = dictionary["paragraph"] as? [String: Any] {
            self.paragraph = EditorTextStyle(dictionary: paragraph)
        }
        if let blockquote = dictionary["blockquote"] as? [String: Any] {
            self.blockquote = EditorBlockquoteTheme(dictionary: blockquote)
        }
        if let headings = dictionary["headings"] as? [String: Any] {
            for level in ["h1", "h2", "h3", "h4", "h5", "h6"] {
                if let style = headings[level] as? [String: Any] {
                    self.headings[level] = EditorTextStyle(dictionary: style)
                }
            }
        }
        if let list = dictionary["list"] as? [String: Any] {
            self.list = EditorListTheme(dictionary: list)
        }
        if let horizontalRule = dictionary["horizontalRule"] as? [String: Any] {
            self.horizontalRule = EditorHorizontalRuleTheme(dictionary: horizontalRule)
        }
        if let mentions = dictionary["mentions"] as? [String: Any] {
            self.mentions = EditorMentionTheme(dictionary: mentions)
        }
        if let toolbar = dictionary["toolbar"] as? [String: Any] {
            self.toolbar = EditorToolbarTheme(dictionary: toolbar)
        }
        backgroundColor = EditorTheme.color(from: dictionary["backgroundColor"])
        borderRadius = EditorTheme.cgFloat(dictionary["borderRadius"])
        if let contentInsets = dictionary["contentInsets"] as? [String: Any] {
            self.contentInsets = EditorContentInsets(dictionary: contentInsets)
        }
    }

    func effectiveTextStyle(for nodeType: String, inBlockquote: Bool = false) -> EditorTextStyle {
        var style = text ?? EditorTextStyle()
        style = style.merged(with: inBlockquote ? blockquote?.text : nil)
        if nodeType == "paragraph" {
            style = style.merged(with: paragraph)
            if paragraph?.lineHeight == nil {
                style.lineHeight = nil
            }
        }
        style = style.merged(with: headings[nodeType])
        return style
    }

    static func cgFloat(_ value: Any?) -> CGFloat? {
        guard let number = value as? NSNumber else { return nil }
        return CGFloat(truncating: number)
    }

    static func fontWeight(from value: String) -> UIFont.Weight {
        switch value {
        case "100": return .ultraLight
        case "200": return .thin
        case "300": return .light
        case "500": return .medium
        case "600": return .semibold
        case "700", "bold": return .bold
        case "800": return .heavy
        case "900": return .black
        default: return .regular
        }
    }

    static func shouldApplyBoldTrait(_ value: String?) -> Bool {
        guard let value else { return false }
        return value == "bold" || Int(value).map { $0 >= 600 } == true
    }

    static func color(from value: Any?) -> UIColor? {
        guard let raw = value as? String else { return nil }
        let string = raw.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()

        if let hexColor = colorFromHex(string) {
            return hexColor
        }
        if let rgbColor = colorFromRGBFunction(string) {
            return rgbColor
        }

        switch string {
        case "black": return .black
        case "white": return .white
        case "red": return .red
        case "green": return .green
        case "blue": return .blue
        case "gray", "grey": return .gray
        case "clear", "transparent": return .clear
        default: return nil
        }
    }

    private static func colorFromHex(_ string: String) -> UIColor? {
        guard string.hasPrefix("#") else { return nil }
        let hex = String(string.dropFirst())

        switch hex.count {
        case 3:
            let chars = Array(hex)
            return UIColor(
                red: component(String(repeating: String(chars[0]), count: 2)),
                green: component(String(repeating: String(chars[1]), count: 2)),
                blue: component(String(repeating: String(chars[2]), count: 2)),
                alpha: 1
            )
        case 4:
            let chars = Array(hex)
            return UIColor(
                red: component(String(repeating: String(chars[0]), count: 2)),
                green: component(String(repeating: String(chars[1]), count: 2)),
                blue: component(String(repeating: String(chars[2]), count: 2)),
                alpha: component(String(repeating: String(chars[3]), count: 2))
            )
        case 6:
            return UIColor(
                red: component(String(hex.prefix(2))),
                green: component(String(hex.dropFirst(2).prefix(2))),
                blue: component(String(hex.dropFirst(4).prefix(2))),
                alpha: 1
            )
        case 8:
            return UIColor(
                red: component(String(hex.prefix(2))),
                green: component(String(hex.dropFirst(2).prefix(2))),
                blue: component(String(hex.dropFirst(4).prefix(2))),
                alpha: component(String(hex.dropFirst(6).prefix(2)))
            )
        default:
            return nil
        }
    }

    private static func colorFromRGBFunction(_ string: String) -> UIColor? {
        let isRGBA = string.hasPrefix("rgba(") && string.hasSuffix(")")
        let isRGB = string.hasPrefix("rgb(") && string.hasSuffix(")")
        guard isRGBA || isRGB else { return nil }

        let start = string.index(string.startIndex, offsetBy: isRGBA ? 5 : 4)
        let end = string.index(before: string.endIndex)
        let parts = string[start..<end]
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }

        guard parts.count == (isRGBA ? 4 : 3),
              let red = Double(parts[0]),
              let green = Double(parts[1]),
              let blue = Double(parts[2])
        else {
            return nil
        }

        let alpha = isRGBA ? (Double(parts[3]) ?? 1) : 1
        return UIColor(
            red: red / 255,
            green: green / 255,
            blue: blue / 255,
            alpha: alpha
        )
    }

    private static func component(_ hex: String) -> CGFloat {
        CGFloat(Int(hex, radix: 16) ?? 0) / 255
    }
}
