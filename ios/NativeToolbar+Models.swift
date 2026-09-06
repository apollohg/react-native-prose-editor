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
