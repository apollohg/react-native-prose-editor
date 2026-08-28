import ExpoModulesCore
import UIKit

enum NativeEditorDestroyReservationResult: Equatable {
    case reserved
    case alreadyInProgress
    case unavailable
}

final class NativeEditorViewRegistry {
    static let shared = NativeEditorViewRegistry()

    private final class RegisteredView {
        weak var view: NativeEditorExpoView?
        let order: UInt64

        init(view: NativeEditorExpoView, order: UInt64) {
            self.view = view
            self.order = order
        }
    }

    private var viewsByEditorId: [UInt64: [RegisteredView]] = [:]
    private var nextRegistrationOrder: UInt64 = 0
    private var activeEditorIds: Set<UInt64> = []
    private var destroyingEditorIds: Set<UInt64> = []
    var onFinalizeDestroyForTesting: ((UInt64) throws -> Void)?

    private init() {}

    func markEditorCreated(editorId: UInt64) {
        guard editorId != 0 else { return }
        _ = performOnMain {
            activeEditorIds.insert(editorId)
        }
    }

    func rebaseAfterRemoteCommit(editorId: UInt64) {
        guard editorId != 0 else { return }
        DispatchQueue.main.async { [weak self] in
            self?.applyRemoteCommitRefresh(editorId: editorId)
        }
    }

    func applyRemoteCommitRefresh(editorId: UInt64) {
        dispatchPrecondition(condition: .onQueue(.main))
        liveRegisteredViews(editorId: editorId).forEach { $0.view?.applyRemoteCommitRefresh() }
    }

    func isDestroyed(editorId: UInt64) -> Bool {
        guard editorId != 0 else { return false }
        return performOnMain {
            if destroyingEditorIds.contains(editorId) { return true }
            if activeEditorIds.contains(editorId) {
                return false
            }
            return EditorV2Registry.adapter(forLegacyId: editorId) == nil
        }
    }

    @discardableResult
    func register(editorId: UInt64, view: NativeEditorExpoView) -> Bool {
        guard editorId != 0 else { return false }
        return performOnMain {
            guard !destroyingEditorIds.contains(editorId) else { return false }
            if !activeEditorIds.contains(editorId) {
                guard EditorV2Registry.adapter(forLegacyId: editorId) != nil else {
                    return false
                }
                activeEditorIds.insert(editorId)
            }
            var views = liveRegisteredViews(editorId: editorId)
            guard !views.contains(where: { $0.view === view }) else { return true }
            nextRegistrationOrder &+= 1
            views.append(RegisteredView(view: view, order: nextRegistrationOrder))
            viewsByEditorId[editorId] = views
            return true
        }
    }

    func unregister(editorId: UInt64, view: NativeEditorExpoView) {
        guard editorId != 0 else { return }
        performOnMain {
            let views = liveRegisteredViews(editorId: editorId).filter { $0.view !== view }
            if views.isEmpty {
                viewsByEditorId.removeValue(forKey: editorId)
            } else {
                viewsByEditorId[editorId] = views
            }
        }
    }

    func nativeOwnerReleased(editorId: UInt64, by view: NativeEditorExpoView) {
        guard editorId != 0 else { return }
        performOnMain {
            guard !destroyingEditorIds.contains(editorId) else { return }
            let survivor = liveRegisteredViews(editorId: editorId)
                .reversed()
                .compactMap(\.view)
                .first { $0 !== view && $0.window != nil }
            survivor?.claimNativeOwnershipAndCatchUp(editorId: editorId)
        }
    }

    func invalidateDestroyedEditor(editorId: UInt64) {
        guard reserveDestroy(editorId: editorId) else { return }
        finalizeDestroy(editorId: editorId)
    }

    /// Install the reversible, in-flight destroy gate before crossing the
    /// FFI boundary. The active view state remains intact until finalization
    /// so a retryable destroy can roll back without reconstructing bindings.
    func acquireDestroyReservation(editorId: UInt64) -> NativeEditorDestroyReservationResult {
        guard editorId != 0 else { return .unavailable }
        return performOnMain {
            guard !destroyingEditorIds.contains(editorId) else { return .alreadyInProgress }
            guard activeEditorIds.contains(editorId)
                || EditorV2Registry.adapter(forLegacyId: editorId) != nil
            else {
                return .unavailable
            }
            destroyingEditorIds.insert(editorId)
            return .reserved
        }
    }

    @discardableResult
    func reserveDestroy(editorId: UInt64) -> Bool {
        acquireDestroyReservation(editorId: editorId) == .reserved
    }

    func rollbackDestroy(editorId: UInt64) {
        guard editorId != 0 else { return }
        _ = performOnMain {
            destroyingEditorIds.remove(editorId)
        }
    }

    func isDestroyReserved(editorId: UInt64) -> Bool {
        guard editorId != 0 else { return false }
        return performOnMain {
            destroyingEditorIds.contains(editorId)
        }
    }

    func finalizeDestroy(editorId: UInt64) {
        guard editorId != 0 else { return }
        performOnMain {
            guard destroyingEditorIds.contains(editorId) else { return }
            invokeDestroyTestingHook(onFinalizeDestroyForTesting, editorId: editorId)
            activeEditorIds.remove(editorId)
            let views = viewsByEditorId.removeValue(forKey: editorId)?.compactMap(\.view) ?? []
            views.forEach { $0.handleEditorDestroyed(editorId) }
            destroyingEditorIds.remove(editorId)
        }
    }

    func destroy(editorId: UInt64, operation: () -> Void) {
        guard reserveDestroy(editorId: editorId) else { return }
        operation()
        finalizeDestroy(editorId: editorId)
    }

    func prepareForCommandJSON(editorId: UInt64) -> String {
        let prepare = { () -> String in
            if self.destroyingEditorIds.contains(editorId) {
                return Self.commandPreparationJSON(ready: false, blockedReason: "destroying")
            }
            if !self.activeEditorIds.contains(editorId),
               EditorV2Registry.adapter(forLegacyId: editorId) == nil
            {
                return Self.commandPreparationJSON(ready: false, blockedReason: "destroyed")
            }
            let views = self.liveRegisteredViews(editorId: editorId).compactMap(\.view)
            guard let view = views.reversed().first(where: {
                $0.ownsNativeBinding(editorId: editorId)
            }) ?? views.last else {
                self.viewsByEditorId.removeValue(forKey: editorId)
                return Self.commandPreparationJSON(ready: true)
            }
            return view.prepareForEditorCommandJSON()
        }

        return performOnMain(prepare)
    }

    private func liveRegisteredViews(editorId: UInt64) -> [RegisteredView] {
        let views = viewsByEditorId[editorId]?.filter { $0.view != nil } ?? []
        if views.isEmpty {
            viewsByEditorId.removeValue(forKey: editorId)
        } else {
            viewsByEditorId[editorId] = views
        }
        return views.sorted { $0.order < $1.order }
    }

    static func commandPreparationJSON(
        ready: Bool,
        updateJSON: String? = nil,
        blockedReason: String? = nil
    ) -> String {
        var payload: [String: Any] = ["ready": ready]
        if let updateJSON {
            payload["updateJSON"] = updateJSON
        }
        if let blockedReason {
            payload["blockedReason"] = blockedReason
        }
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            if let blockedReason {
                return ready
                    ? "{\"ready\":true,\"blockedReason\":\"\(blockedReason)\"}"
                    : "{\"ready\":false,\"blockedReason\":\"\(blockedReason)\"}"
            }
            return ready ? "{\"ready\":true}" : "{\"ready\":false}"
        }
        return json
    }

    private func performOnMain<T>(_ work: () -> T) -> T {
        if Thread.isMainThread {
            return work()
        }
        return DispatchQueue.main.sync(execute: work)
    }
}

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

enum EditorNodeTypes {
    static func listItemType(for listType: String) -> String {
        switch listType {
        case "bullet_list", "ordered_list":
            return "list_item"
        case "task_list":
            return "task_item"
        case "taskList":
            return "taskItem"
        default:
            return "listItem"
        }
    }

    static func isHardBreak(_ nodeType: String?) -> Bool {
        nodeType == "hardBreak" || nodeType == "hard_break"
    }

    static func isHorizontalRule(_ nodeType: String?) -> Bool {
        nodeType == "horizontalRule" || nodeType == "horizontal_rule"
    }

    static func isListItem(_ nodeType: String) -> Bool {
        nodeType == "listItem" || nodeType == "list_item"
            || nodeType == "taskItem" || nodeType == "task_item"
    }

    static func isListContainer(_ nodeType: String) -> Bool {
        nodeType == "bulletList" || nodeType == "bullet_list"
            || nodeType == "orderedList" || nodeType == "ordered_list"
            || nodeType == "taskList" || nodeType == "task_list"
    }

    static func preferredHardBreak(in insertableNodes: Set<String>) -> String {
        insertableNodes.contains("hard_break") ? "hard_break" : "hardBreak"
    }
}

private enum ToolbarListType: String {
    case bullet_list
    case ordered_list
    case bulletList
    case orderedList
}

private enum ToolbarDefaultIconId: String {
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

private enum ToolbarItemKind: String {
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

private enum ToolbarGroupPresentation: String {
    case expand
    case menu
}

private enum ToolbarItemPlacement: String {
    case start
    case scroll
    case end
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

private struct NativeToolbarItem {
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

    fileprivate func setItems(_ items: [NativeToolbarItem]) {
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

    fileprivate func apply(state: NativeToolbarState) {
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

class NativeEditorExpoView: ExpoView, EditorTextViewDelegate, UIGestureRecognizerDelegate {

    // MARK: - Subviews

    let richTextView: RichTextEditorView
    private let accessoryToolbar = EditorAccessoryToolbarView(
        frame: .zero,
        inputViewStyle: .keyboard
    )
    private let accessoryPlaceholder = EditorAccessoryPlaceholderView(frame: .zero)
    private var toolbarFramesInWindow: [CGRect] = []
    private var lastToolbarTouchUptime: TimeInterval = -Double.infinity
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
    private var desiredThemeJSON: String?
    private let imageLoadOwner = RenderImageLoadOwner(policy: .default)
    private var lastThemeJSON: String?
    private var lastAddonsJSON: String?
    private var lastRemoteSelectionsJSON: String?
    private var lastToolbarItemsJSON: String?
    private var lastToolbarFrameJSON: String?
    private var pendingEditorUpdateJSON: String?
    private var pendingEditorUpdateEditorId: String?
    private var pendingEditorUpdateRevision = 0
    private var appliedEditorUpdateRevision = 0
    private var renderedDocumentRevision: UInt64?
    private var pendingEditorUpdateRetryScheduled = false
    private var pendingEditorUpdateRetryEditorId: UInt64?
    private var pendingEditorUpdateRetryGeneration: UInt64 = 0
    /// Internal-only fallback for boundary rejections that cannot reach an
    /// adapter callback because the paired adapter is absent. Task 15 owns
    /// application-visible event wiring; these deterministic records do not
    /// dispatch an Expo event.
    private var editorUpdateInternalRejections: [String] = []
    private var pendingViewCommandUpdateJSON: String?
    private var pendingViewCommandUpdateEditorId: UInt64?
    private var pendingViewCommandUpdateRetryScheduled = false
    private var pendingViewCommandUpdateRetryGeneration: UInt64 = 0
    private var pendingEditableRetryValue: Bool?
    private var pendingEditableRetryEditorId: UInt64?
    private var pendingEditableRetryScheduled = false
    private var pendingEditableRetryGeneration: UInt64 = 0
    private var pendingThemeRetryJSON: String?
    private var pendingThemeRetryEditorId: UInt64?
    private var pendingThemeRetryScheduled = false
    private var pendingThemeRetryGeneration: UInt64 = 0
    private var pendingAccessoryRetryActions: [PendingAccessoryRetryAction] = []
    private var invalidatedAccessoryRetryActions = Set<PendingAccessoryRetryAction>()
    private var pendingAccessoryRetryEditorId: UInt64?
    private var pendingAccessoryRetryScheduled = false
    private var pendingAccessoryRetryGeneration: UInt64 = 0
    private var pendingMentionSuggestionRetry: PendingMentionSuggestionRetry?
    private var pendingMentionSuggestionRetryScheduled = false
    private var pendingMentionSuggestionRetryGeneration: UInt64 = 0
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
    let onEditorError = EventDispatcher()
    let onExternalTextCompositionEnd = EventDispatcher()
    /// Native integration tests capture the exact payload production sends to
    /// Expo without assigning adapter callbacks directly.
    var onEditorErrorForTesting: (([String: Any]) -> Void)?
    var onExternalTextCompositionEndForTesting: (([String: Any]) -> Void)?
    private var autonomousErrorBindingAdapter: EditorV2Adapter?
    private var autonomousErrorBindingEditorId: String?
    private var autonomousErrorBindingToken: UUID?
    private var autonomousErrorBindingGeneration: UInt64 = 0
    private var pendingAutonomousErrors: [UUID: PendingAutonomousError] = [:]
    private var lastEmittedContentHeight: CGFloat = 0
    private var cachedAutoGrowContentHeight: CGFloat = 0
    private var lastAddonEventJSONForTestingValue: String?

    private enum EditorUpdateApplyOutcome {
        case applied
        case retryableDeferred
        case rejected
    }

    private enum PendingAccessoryRetryAction: Hashable {
        case reloadInputViews
        case refreshMentionQuery
        case clearMentionQueryState
        case updateAccessoryToolbarVisibility
    }

    private struct PendingMentionSuggestionRetry {
        let suggestionKey: String
        let editorId: UInt64
        let trigger: String
        let query: String
        let anchor: UInt32
        let head: UInt32
        let documentVersion: String?
        let textSnapshot: String
    }

    private struct PendingAutonomousError {
        let adapter: EditorV2Adapter
        let editorId: String
        let token: UUID
        let generation: UInt64
        let error: FfiError
    }

    private struct MentionRetryTextDiff {
        let start: Int
        let oldEnd: Int
        let newEnd: Int
    }

    // MARK: - Initialization

    required init(appContext: AppContext? = nil) {
        richTextView = RichTextEditorView(frame: .zero)
        super.init(appContext: appContext)
        richTextView.imageLoadOwner = imageLoadOwner
        richTextView.onHeightMayChange = { [weak self] measuredHeight in
            guard let self, self.heightBehavior == .autoGrow else { return }
            self.cachedAutoGrowContentHeight = measuredHeight
            self.invalidateIntrinsicContentSize()
            self.emitContentHeightIfNeeded(force: true, measuredHeight: measuredHeight)
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
        richTextView.textView.editorDelegate = nil
        if let resultJSON = richTextView.textView.discardTransientNativeInputForEditorRebind() {
            dispatchExternalTextCompositionEnd(resultJSON)
        }
        let editorId = richTextView.editorId
        let releasedNativeOwner = ownsNativeBinding(editorId: editorId)
        clearAutonomousErrorBinding()
        NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: self)
        if releasedNativeOwner {
            NativeEditorViewRegistry.shared.nativeOwnerReleased(editorId: editorId, by: self)
        }
        imageLoadOwner.cancelAll()
        NotificationCenter.default.removeObserver(self)
    }

    // MARK: - Layout

    override var intrinsicContentSize: CGSize {
        guard heightBehavior == .autoGrow else {
            return CGSize(width: UIView.noIntrinsicMetric, height: UIView.noIntrinsicMetric)
        }
        if cachedAutoGrowContentHeight > 0 {
            return CGSize(width: UIView.noIntrinsicMetric, height: cachedAutoGrowContentHeight)
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
        cachedAutoGrowContentHeight = 0
        invalidateIntrinsicContentSize()
        emitContentHeightIfNeeded(force: true)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil {
            let editorId = richTextView.editorId
            let releasedNativeOwner = ownsNativeBinding(editorId: editorId)
            clearAutonomousErrorBinding()
            if releasedNativeOwner {
                NativeEditorViewRegistry.shared.nativeOwnerReleased(editorId: editorId, by: self)
            }
        } else {
            ensureAutonomousErrorBinding()
            applyRemoteCommitRefresh()
        }
        if richTextView.textView.isFirstResponder {
            installOutsideTapRecognizerIfNeeded()
        } else {
            uninstallOutsideTapRecognizer()
        }
    }

    // MARK: - Editor Binding

    func handleEditorDestroyed(_ editorId: UInt64) {
        guard editorId != 0 else { return }
        guard richTextView.editorId == editorId || richTextView.textView.editorId == editorId else {
            NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: self)
            return
        }

        richTextView.textView.discardTransientNativeInputForEditorRebind()
        clearAutonomousErrorBinding()
        NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: self)
        clearPendingEditorUpdateRetries()
        clearPendingViewCommandUpdateRetry()
        clearPendingEditableRetry()
        clearPendingThemeRetry()
        clearPendingAccessoryRetry()
        clearPendingMentionSuggestionRetry()
        lastMentionEventJSON = nil
        _ = richTextView.textView.resignFirstResponder()
        richTextView.editorId = 0
        mentionQueryState = nil
        _ = accessoryToolbar.setMentionSuggestions([])
        toolbarState = .empty
        accessoryToolbar.apply(state: .empty)
        uninstallOutsideTapRecognizer()
        refreshSystemAssistantToolbarIfNeeded()
    }

    func setEditorId(_ id: UInt64) {
        let previousEditorId = richTextView.editorId
        if id != 0 && NativeEditorViewRegistry.shared.isDestroyed(editorId: id) {
            if previousEditorId == id {
                handleEditorDestroyed(id)
            } else {
                setEditorId(0)
            }
            return
        }
        guard previousEditorId != id else {
            if id != 0 {
                if !NativeEditorViewRegistry.shared.register(editorId: id, view: self) {
                    handleEditorDestroyed(id)
                } else {
                    ensureAutonomousErrorBinding()
                }
            }
            return
        }
        if previousEditorId != id {
            richTextView.textView.discardTransientNativeInputForEditorRebind()
            let releasedNativeOwner = ownsNativeBinding(editorId: previousEditorId)
            clearAutonomousErrorBinding()
            NativeEditorViewRegistry.shared.unregister(editorId: previousEditorId, view: self)
            if releasedNativeOwner {
                NativeEditorViewRegistry.shared.nativeOwnerReleased(
                    editorId: previousEditorId,
                    by: self
                )
            }
            clearPendingEditorUpdateRetries()
            clearPendingViewCommandUpdateRetry()
            clearPendingEditableRetry()
            clearPendingThemeRetry()
            clearPendingAccessoryRetry()
            clearPendingMentionSuggestionRetry()
        }
        var initialBindUpdateJSON: String?
        if id != 0 {
            guard NativeEditorViewRegistry.shared.register(editorId: id, view: self) else {
                handleEditorDestroyed(id)
                return
            }
            bindAutonomousError(adapter: EditorV2Registry.adapter(forLegacyId: id), editorId: id)
            initialBindUpdateJSON = EditorV2Registry.adapter(forLegacyId: id)?.initialUpdateJSON()
        }
        // Bind the editor with the same adopted snapshot used for toolbar
        // state. The text view must not perform an independent state read.
        imageLoadOwner.withCurrent {
            richTextView.bindEditor(id: id, initialUpdateJSON: initialBindUpdateJSON)
        }
        if id != 0 {
            if let initialBindUpdateJSON,
               let state = NativeToolbarState(updateJSON: initialBindUpdateJSON)
            {
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
        if desiredThemeJSON != lastThemeJSON {
            setThemeJson(desiredThemeJSON)
        }
        refreshSystemAssistantToolbarIfNeeded()
        refreshMentionQuery()
    }

    // MARK: - Autonomous adapter errors

    func ownsNativeBinding(editorId: UInt64) -> Bool {
        guard editorId != 0,
              richTextView.editorId == editorId,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId)
        else { return false }
        let autonomousOwner = autonomousErrorBindingAdapter === adapter
            && autonomousErrorBindingToken.map { adapter.isNativeBindingOwner(token: $0) } == true
        return autonomousOwner || richTextView.textView.ownsNativeBinding(adapter)
    }

    func claimNativeOwnershipAndCatchUp(editorId: UInt64) {
        guard window != nil, richTextView.editorId == editorId else { return }
        ensureAutonomousErrorBinding()
        applyRemoteCommitRefresh()
    }

    private func ensureAutonomousErrorBinding() {
        let editorId = richTextView.editorId
        guard editorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId)
        else { return }
        guard autonomousErrorBindingAdapter !== adapter
            || autonomousErrorBindingEditorId != adapter.editorId
            || autonomousErrorBindingToken.map({ adapter.isAutonomousErrorOwner(token: $0) }) != true
        else { return }
        clearAutonomousErrorBinding()
        bindAutonomousError(adapter: adapter, editorId: editorId)
    }

    private func bindAutonomousError(adapter: EditorV2Adapter?, editorId: UInt64) {
        guard let adapter,
              let canonicalEditorId = v2CanonicalUInt64String(adapter.editorId),
              canonicalEditorId == String(editorId),
              !adapter.isDestroyed
        else { return }
        let token = UUID()
        let generation = autonomousErrorBindingGeneration
        autonomousErrorBindingAdapter = adapter
        autonomousErrorBindingEditorId = canonicalEditorId
        autonomousErrorBindingToken = token
        adapter.bindAutonomousErrorOwner(token: token) { [weak self, weak adapter] error in
            let enqueue = {
                guard let self, let adapter else { return }
                self.enqueueAutonomousError(
                    error,
                    from: adapter,
                    editorId: canonicalEditorId,
                    token: token,
                    generation: generation
                )
            }
            if Thread.isMainThread {
                enqueue()
            } else {
                DispatchQueue.main.async(execute: enqueue)
            }
        }
    }

    private func clearAutonomousErrorBinding() {
        autonomousErrorBindingGeneration &+= 1
        pendingAutonomousErrors.removeAll()
        if let adapter = autonomousErrorBindingAdapter,
           let token = autonomousErrorBindingToken
        {
            adapter.clearAutonomousErrorOwner(token: token)
        }
        autonomousErrorBindingAdapter = nil
        autonomousErrorBindingEditorId = nil
        autonomousErrorBindingToken = nil
    }

    private func enqueueAutonomousError(
        _ error: FfiError,
        from adapter: EditorV2Adapter,
        editorId: String,
        token: UUID,
        generation: UInt64
    ) {
        guard isLiveAutonomousErrorBinding(
            adapter: adapter,
            editorId: editorId,
            token: token,
            generation: generation
        ) else { return }
        let dispatchId = UUID()
        pendingAutonomousErrors[dispatchId] = PendingAutonomousError(
            adapter: adapter,
            editorId: editorId,
            token: token,
            generation: generation,
            error: error
        )
        DispatchQueue.main.async { [weak self] in
            self?.dispatchAutonomousError(id: dispatchId)
        }
    }

    private func dispatchAutonomousError(id: UUID) {
        // Remove before invoking Expo/test code. Reentrant state changes or a
        // duplicate callback cannot deliver this particular failure twice.
        guard let pending = pendingAutonomousErrors.removeValue(forKey: id),
              isLiveAutonomousErrorBinding(
                adapter: pending.adapter,
                editorId: pending.editorId,
                token: pending.token,
                generation: pending.generation
              )
        else { return }
        let payload = NativeEditorExpoView.autonomousErrorEventPayload(
            editorId: pending.editorId,
            error: pending.error
        )
        if let onEditorErrorForTesting {
            onEditorErrorForTesting(payload)
        } else {
            onEditorError(payload)
        }
    }

    private func isLiveAutonomousErrorBinding(
        adapter: EditorV2Adapter,
        editorId: String,
        token: UUID,
        generation: UInt64
    ) -> Bool {
        guard autonomousErrorBindingGeneration == generation,
              autonomousErrorBindingAdapter === adapter,
              autonomousErrorBindingEditorId == editorId,
              autonomousErrorBindingToken == token,
              adapter.isAutonomousErrorOwner(token: token),
              !adapter.isDestroyed,
              let nativeEditorId = UInt64(editorId),
              !NativeEditorViewRegistry.shared.isDestroyed(editorId: nativeEditorId),
              EditorV2Registry.adapter(forLegacyId: nativeEditorId) === adapter
        else { return false }
        return true
    }

    private static func autonomousErrorEventPayload(editorId: String, error: FfiError) -> [String: Any] {
        let errorRecord: [String: Any] = [
            "domain": error.domain,
            "code": error.code,
            "message": error.message,
            "requestId": error.requestId ?? NSNull(),
            "operationIndex": error.operationIndex ?? NSNull(),
            "limit": error.limit ?? NSNull(),
            "actual": error.actual ?? NSNull(),
            "detailsJson": error.detailsJson ?? NSNull(),
        ]
        let payload: [String: Any] = [
            "editorId": editorId,
            "error": errorRecord,
        ]
        return payload
    }

    func setThemeJson(_ themeJson: String?) {
        desiredThemeJSON = themeJson
        guard lastThemeJSON != themeJson else {
            clearPendingThemeRetry()
            return
        }
        let theme = EditorTheme.from(json: themeJson)
        guard imageLoadOwner.withCurrent({ richTextView.applyTheme(theme) }) else {
            scheduleThemeRetry(themeJson)
            return
        }
        lastThemeJSON = themeJson
        clearPendingThemeRetry()
        accessoryToolbar.apply(theme: theme?.toolbar)
        accessoryToolbar.apply(mentionTheme: theme?.mentions ?? addons.mentions?.theme)
        refreshSystemAssistantToolbarIfNeeded()
        if richTextView.textView.isFirstResponder,
           (richTextView.textView.inputAccessoryView === accessoryToolbar || shouldUseSystemAssistantToolbar)
        {
            reloadInputViewsAfterPreparingOrRetry()
        }
    }

    private func clearPendingEditorUpdateRetries() {
        pendingEditorUpdateJSON = nil
        pendingEditorUpdateEditorId = nil
        pendingEditorUpdateRevision = 0
        appliedEditorUpdateRevision = 0
        renderedDocumentRevision = nil
        pendingEditorUpdateRetryScheduled = false
        pendingEditorUpdateRetryEditorId = nil
        pendingEditorUpdateRetryGeneration &+= 1
    }

    private func clearPendingViewCommandUpdateRetry() {
        pendingViewCommandUpdateJSON = nil
        pendingViewCommandUpdateEditorId = nil
        pendingViewCommandUpdateRetryScheduled = false
        pendingViewCommandUpdateRetryGeneration &+= 1
    }

    private func clearPendingEditableRetry() {
        pendingEditableRetryValue = nil
        pendingEditableRetryEditorId = nil
        pendingEditableRetryScheduled = false
        pendingEditableRetryGeneration &+= 1
    }

    private func clearPendingThemeRetry() {
        pendingThemeRetryJSON = nil
        pendingThemeRetryEditorId = nil
        pendingThemeRetryScheduled = false
        pendingThemeRetryGeneration &+= 1
    }

    private func clearPendingAccessoryRetry() {
        pendingAccessoryRetryActions = []
        invalidatedAccessoryRetryActions.removeAll()
        pendingAccessoryRetryEditorId = nil
        pendingAccessoryRetryScheduled = false
        pendingAccessoryRetryGeneration &+= 1
    }

    private func clearPendingMentionSuggestionRetry() {
        pendingMentionSuggestionRetry = nil
        pendingMentionSuggestionRetryScheduled = false
        pendingMentionSuggestionRetryGeneration &+= 1
    }

    private func scheduleThemeRetry(_ themeJson: String?) {
        pendingThemeRetryJSON = themeJson
        pendingThemeRetryEditorId = richTextView.editorId
        guard !pendingThemeRetryScheduled else { return }
        pendingThemeRetryScheduled = true
        pendingThemeRetryGeneration &+= 1
        let retryGeneration = pendingThemeRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingThemeRetryGeneration else { return }
            let retryJSON = self.pendingThemeRetryJSON
            self.pendingThemeRetryJSON = nil
            let retryEditorId = self.pendingThemeRetryEditorId
            self.pendingThemeRetryEditorId = nil
            self.pendingThemeRetryScheduled = false
            guard retryEditorId == self.richTextView.editorId else {
                self.clearPendingThemeRetry()
                return
            }
            guard retryJSON == self.desiredThemeJSON else {
                self.clearPendingThemeRetry()
                return
            }
            self.setThemeJson(retryJSON)
        }
    }

    private func prepareForInputAccessoryMutationOrRetry(_ action: PendingAccessoryRetryAction) -> Bool {
        guard richTextView.editorId != 0, richTextView.textView.isFirstResponder else {
            return true
        }
        guard richTextView.textView.prepareForExternalEditorUpdate() else {
            scheduleAccessoryRetry(action)
            return false
        }
        return true
    }

    private func reloadInputViewsAfterPreparingOrRetry() {
        guard prepareForInputAccessoryMutationOrRetry(.reloadInputViews) else { return }
        richTextView.textView.reloadInputViews()
        markAccessoryMutationSucceeded(.reloadInputViews)
    }

    private func scheduleAccessoryRetry(_ action: PendingAccessoryRetryAction) {
        invalidatedAccessoryRetryActions.remove(action)
        pendingAccessoryRetryActions.removeAll { $0 == action }
        pendingAccessoryRetryActions.append(action)
        pendingAccessoryRetryEditorId = richTextView.editorId
        guard !pendingAccessoryRetryScheduled else { return }
        pendingAccessoryRetryScheduled = true
        pendingAccessoryRetryGeneration &+= 1
        let retryGeneration = pendingAccessoryRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingAccessoryRetryGeneration else { return }
            guard self.pendingAccessoryRetryEditorId == self.richTextView.editorId else {
                self.clearPendingAccessoryRetry()
                return
            }
            let actions = self.pendingAccessoryRetryActions
            self.pendingAccessoryRetryActions = []
            self.pendingAccessoryRetryEditorId = nil
            self.pendingAccessoryRetryScheduled = false
            for index in actions.indices {
                let action = actions[index]
                guard retryGeneration == self.pendingAccessoryRetryGeneration else { return }
                guard !self.invalidatedAccessoryRetryActions.contains(action) else {
                    self.invalidatedAccessoryRetryActions.remove(action)
                    continue
                }
                let generationBeforeAction = self.pendingAccessoryRetryGeneration
                self.performAccessoryRetryAction(action)
                guard self.pendingAccessoryRetryGeneration == generationBeforeAction else {
                    let remainingIndex = actions.index(after: index)
                    if remainingIndex < actions.endIndex {
                        self.requeueUnprocessedAccessoryRetryActions(actions[remainingIndex...])
                    }
                    return
                }
            }
            self.invalidatedAccessoryRetryActions.subtract(actions)
        }
    }

    private func requeueUnprocessedAccessoryRetryActions(
        _ actions: ArraySlice<PendingAccessoryRetryAction>
    ) {
        for action in actions {
            guard !invalidatedAccessoryRetryActions.contains(action) else {
                invalidatedAccessoryRetryActions.remove(action)
                continue
            }
            pendingAccessoryRetryActions.removeAll { $0 == action }
            pendingAccessoryRetryActions.append(action)
        }
        if !pendingAccessoryRetryActions.isEmpty {
            pendingAccessoryRetryEditorId = richTextView.editorId
        }
    }

    private func performAccessoryRetryAction(_ action: PendingAccessoryRetryAction) {
        switch action {
        case .reloadInputViews:
            reloadInputViewsAfterPreparingOrRetry()
        case .refreshMentionQuery:
            refreshMentionQuery()
        case .clearMentionQueryState:
            clearMentionQueryStateAndHidePopover()
        case .updateAccessoryToolbarVisibility:
            updateAccessoryToolbarVisibility()
        }
    }

    private func markAccessoryMutationSucceeded(_ action: PendingAccessoryRetryAction) {
        var invalidated: Set<PendingAccessoryRetryAction> = [action]
        switch action {
        case .refreshMentionQuery:
            invalidated.insert(.clearMentionQueryState)
        case .clearMentionQueryState:
            if !hasActiveMentionQueryForCurrentAddons() {
                invalidated.insert(.refreshMentionQuery)
            }
        case .reloadInputViews, .updateAccessoryToolbarVisibility:
            break
        }
        invalidatePendingAccessoryRetries(invalidated)
    }

    private func invalidatePendingAccessoryRetries(_ actions: Set<PendingAccessoryRetryAction>) {
        guard !actions.isEmpty else { return }
        invalidatedAccessoryRetryActions.formUnion(actions)
        pendingAccessoryRetryActions.removeAll { actions.contains($0) }
    }

    private func hasActiveMentionQueryForCurrentAddons() -> Bool {
        guard richTextView.editorId != 0,
              richTextView.textView.isFirstResponder,
              let mentions = addons.mentions
        else {
            return false
        }
        return currentMentionQueryState(trigger: mentions.trigger) != nil
    }

    func setAddonsJson(_ addonsJson: String?) {
        guard lastAddonsJSON != addonsJson else { return }
        lastAddonsJSON = addonsJson
        addons = NativeEditorAddons.from(json: addonsJson)
        accessoryToolbar.apply(mentionTheme: richTextView.textView.theme?.mentions ?? addons.mentions?.theme)
        refreshMentionQuery()
    }

    var imageLoadingPolicy: ImageLoadingPolicy {
        imageLoadOwner.policy
    }

    func setImageLoadingPolicyJson(_ json: String?) {
        let policy = ImageLoadingPolicy.from(json: json)
        guard policy != imageLoadOwner.policy else { return }
        imageLoadOwner.updatePolicy(policy)
        richTextView.textView.imageLoadingPolicyDidChange()
        guard richTextView.editorId != 0 else { return }
        imageLoadOwner.withCurrent {
            _ = richTextView.textView.applyUpdateJSON(
                EditorV2Shadow.getCurrentState(id: richTextView.editorId),
                notifyDelegate: false
            )
        }
    }

    func setRemoteSelectionsJson(_ remoteSelectionsJson: String?) {
        guard lastRemoteSelectionsJSON != remoteSelectionsJson else { return }
        lastRemoteSelectionsJSON = remoteSelectionsJson
        richTextView.setRemoteSelections(RemoteSelectionDecoration.from(json: remoteSelectionsJson))
    }

    func setEditable(_ editable: Bool) {
        if !editable, richTextView.textView.isEditable {
            richTextView.textView.cancelExternalTextCompositionForLifecycleIfNeeded()
        }
        if !editable,
           richTextView.textView.isEditable,
           richTextView.editorId != 0,
           !richTextView.textView.prepareForExternalEditorUpdate()
        {
            scheduleEditableRetry(editable)
            return
        }
        pendingEditableRetryValue = nil
        pendingEditableRetryEditorId = nil
        pendingEditableRetryScheduled = false
        richTextView.textView.isEditable = editable
        updateAccessoryToolbarVisibility()
    }

    func setAccessibilityLabel(_ label: String?) {
        richTextView.textView.accessibilityLabel = label
    }

    func setAccessibilityHint(_ hint: String?) {
        richTextView.textView.accessibilityHint = hint
    }

    private func scheduleEditableRetry(_ editable: Bool) {
        pendingEditableRetryValue = editable
        pendingEditableRetryEditorId = richTextView.editorId
        guard !pendingEditableRetryScheduled else { return }
        pendingEditableRetryScheduled = true
        pendingEditableRetryGeneration &+= 1
        let retryGeneration = pendingEditableRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingEditableRetryGeneration else { return }
            guard let pendingEditable = self.pendingEditableRetryValue else {
                self.pendingEditableRetryScheduled = false
                return
            }
            guard self.pendingEditableRetryEditorId == self.richTextView.editorId else {
                self.clearPendingEditableRetry()
                return
            }
            self.pendingEditableRetryValue = nil
            self.pendingEditableRetryEditorId = nil
            self.pendingEditableRetryScheduled = false
            self.setEditable(pendingEditable)
        }
    }

    func setAutoFocus(_ autoFocus: Bool) {
        guard autoFocus, !didApplyAutoFocus else { return }
        didApplyAutoFocus = true
        focus()
    }

    func setAutoCapitalize(_ autoCapitalize: String?) {
        richTextView.textView.setAutoCapitalize(autoCapitalize)
    }

    func setAutoCorrect(_ autoCorrect: Bool?) {
        richTextView.textView.setAutoCorrect(autoCorrect)
    }

    func setKeyboardType(_ keyboardType: String?) {
        richTextView.textView.setKeyboardType(keyboardType)
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
        if nextBehavior != .autoGrow {
            cachedAutoGrowContentHeight = 0
        }
        richTextView.heightBehavior = nextBehavior
        invalidateIntrinsicContentSize()
        setNeedsLayout()
        if nextBehavior == .autoGrow {
            emitContentHeightIfNeeded(force: true)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.heightBehavior == .autoGrow else { return }
                self.setNeedsLayout()
                self.layoutIfNeeded()
                let measuredHeight = self.richTextView.remeasureAutoGrowHeight()
                guard measuredHeight > 0 else { return }
                self.cachedAutoGrowContentHeight = measuredHeight
                self.invalidateIntrinsicContentSize()
                self.emitContentHeightIfNeeded(force: true, measuredHeight: measuredHeight)
            }
        }
    }

    func setAllowImageResizing(_ allowImageResizing: Bool) {
        richTextView.allowImageResizing = allowImageResizing
    }

    private func emitContentHeightIfNeeded(force: Bool = false, measuredHeight: CGFloat? = nil) {
        let originatingEditorId = richTextView.editorId
        guard heightBehavior == .autoGrow else { return }
        let resolvedHeight = measuredHeight
            ?? (cachedAutoGrowContentHeight > 0 ? cachedAutoGrowContentHeight : richTextView.intrinsicContentSize.height)
        let contentHeight = ceil(resolvedHeight)
        guard contentHeight > 0 else { return }
        guard force || abs(contentHeight - lastEmittedContentHeight) > 0.5 else { return }
        cachedAutoGrowContentHeight = contentHeight
        lastEmittedContentHeight = contentHeight
        guard let event = Self.editorScopedEventPayload(
            ["contentHeight": contentHeight],
            originatingEditorId: originatingEditorId
        ) else { return }
        onContentHeightChange(event)
    }

    func setToolbarButtonsJson(_ toolbarButtonsJson: String?) {
        guard lastToolbarItemsJSON != toolbarButtonsJson else { return }
        lastToolbarItemsJSON = toolbarButtonsJson
        toolbarItems = NativeToolbarItem.from(json: toolbarButtonsJson)
        accessoryToolbar.setItems(toolbarItems)
        refreshSystemAssistantToolbarIfNeeded()
    }

    func setToolbarFrameJson(_ toolbarFrameJson: String?) {
        guard lastToolbarFrameJSON != toolbarFrameJson else { return }
        lastToolbarFrameJSON = toolbarFrameJson
        guard let toolbarFrameJson,
              let data = toolbarFrameJson.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            toolbarFramesInWindow = []
            return
        }

        if let frameDictionaries = raw["frames"] as? [[String: Any]] {
            toolbarFramesInWindow = frameDictionaries.compactMap(Self.toolbarFrame(from:))
            return
        }

        toolbarFramesInWindow = Self.toolbarFrame(from: raw).map { [$0] } ?? []
    }

    private static func toolbarFrame(from raw: [String: Any]) -> CGRect? {
        guard let x = (raw["x"] as? NSNumber)?.doubleValue,
              let y = (raw["y"] as? NSNumber)?.doubleValue,
              let width = (raw["width"] as? NSNumber)?.doubleValue,
              let height = (raw["height"] as? NSNumber)?.doubleValue,
              width > 0,
              height > 0
        else {
            return nil
        }

        return CGRect(x: x, y: y, width: width, height: height)
    }

    func setPendingEditorUpdateJson(_ editorUpdateJson: String?) {
        pendingEditorUpdateJSON = editorUpdateJson
        if editorUpdateJson == nil {
            pendingEditorUpdateEditorId = nil
        }
    }

    func setPendingEditorUpdateEditorId(_ editorUpdateEditorId: String?) {
        guard let editorUpdateEditorId,
              let canonicalEditorId = v2CanonicalUInt64String(editorUpdateEditorId),
              canonicalEditorId != "0"
        else {
            pendingEditorUpdateEditorId = nil
            return
        }
        pendingEditorUpdateEditorId = canonicalEditorId
    }

    func setPendingEditorUpdateRevision(_ editorUpdateRevision: Int) {
        pendingEditorUpdateRevision = editorUpdateRevision
    }

    func applyPendingEditorUpdateIfNeeded() {
        guard pendingEditorUpdateRevision != 0 else { return }
        guard pendingEditorUpdateRevision != appliedEditorUpdateRevision else { return }
        let pendingRevision = pendingEditorUpdateRevision
        guard let updateJSON = pendingEditorUpdateJSON else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update JSON is missing",
                fallbackClassification: "missingUpdateJSON"
            )
            consumePendingEditorUpdate(revision: pendingRevision)
            return
        }
        switch applyEditorUpdateOutcome(updateJSON, sourceEditorId: pendingEditorUpdateEditorId) {
        case .applied:
            consumePendingEditorUpdate(revision: pendingRevision)
        case .retryableDeferred:
            schedulePendingEditorUpdateRetry()
        case .rejected:
            // Mark this prop revision consumed before discarding its envelope:
            // OnViewDidUpdateProps can run again with the same values, but a
            // permanent rejection must not report or retry a second time.
            if let sourceEditorId = pendingEditorUpdateEditorId,
               richTextView.editorId != 0,
               sourceEditorId != String(richTextView.editorId)
            {
                pendingEditorUpdateEditorId = nil
            }
            consumePendingEditorUpdate(revision: pendingRevision)
        }
    }

    private func consumePendingEditorUpdate(revision: Int) {
        appliedEditorUpdateRevision = revision
        pendingEditorUpdateJSON = nil
        pendingEditorUpdateRevision = 0
        pendingEditorUpdateRetryScheduled = false
        pendingEditorUpdateRetryEditorId = nil
        pendingEditorUpdateRetryGeneration &+= 1
    }

    private func schedulePendingEditorUpdateRetry() {
        guard !pendingEditorUpdateRetryScheduled else { return }
        pendingEditorUpdateRetryEditorId = richTextView.editorId
        pendingEditorUpdateRetryScheduled = true
        pendingEditorUpdateRetryGeneration &+= 1
        let retryGeneration = pendingEditorUpdateRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingEditorUpdateRetryGeneration else {
                return
            }
            guard self.pendingEditorUpdateRetryEditorId == self.richTextView.editorId else {
                self.pendingEditorUpdateRetryScheduled = false
                self.clearPendingEditorUpdateRetries()
                return
            }
            self.pendingEditorUpdateRetryScheduled = false
            self.pendingEditorUpdateRetryEditorId = nil
            self.applyPendingEditorUpdateIfNeeded()
        }
    }

    // MARK: - View Commands

    func beginExternalTextComposition(sessionId: String) -> String {
        richTextView.textView.beginExternalTextComposition(sessionId: sessionId)
    }

    func updateExternalTextComposition(sessionId: String, text: String) -> String {
        richTextView.textView.updateExternalTextComposition(sessionId: sessionId, text: text)
    }

    func commitExternalTextComposition(sessionId: String, finalText: String) -> String {
        richTextView.textView.commitExternalTextComposition(
            sessionId: sessionId,
            finalText: finalText
        )
    }

    func cancelExternalTextComposition(sessionId: String, cause: String) -> String {
        richTextView.textView.cancelExternalTextComposition(sessionId: sessionId, cause: cause)
    }

    private func reportRejectedEditorUpdateEnvelope(
        _ message: String,
        fallbackClassification: String
    ) {
        if let adapter = EditorV2Registry.adapter(forLegacyId: richTextView.editorId) {
            adapter.rejectExternalRenderEnvelope(message)
        } else {
            editorUpdateInternalRejections.append(
                "boundary/FFI_RESULT_INVALID/\(fallbackClassification)"
            )
        }
    }

    func applyRemoteCommitRefresh() {
        // Preparing an external update commits a live composition. The commit
        // re-bases the adapter itself, so leave the half-typed word alone.
        guard !richTextView.textView.hasPendingCompositionForExternalRefresh else { return }
        let boundEditorId = richTextView.editorId
        guard boundEditorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: boundEditorId),
              !adapter.isDestroyed
        else {
            return
        }
        let autonomousOwner = autonomousErrorBindingAdapter === adapter
            && autonomousErrorBindingToken.map { adapter.isNativeBindingOwner(token: $0) } == true
        guard autonomousOwner || richTextView.textView.ownsNativeBinding(adapter) else { return }
        let preflight = richTextView.textView.prepareForExternalEditorUpdateResult()
        guard preflight.ready else { return }
        guard let update = preflight.adoptedUpdateJSON
            ?? adapter.refreshFromRustState(mirrorSelection: nil)
        else {
            return
        }
        if !richTextView.textView.applyUpdateJSON(update),
           let recovery = adapter.recoverNativeRender() {
            richTextView.textView.applyUpdateJSON(recovery)
        }
    }

    private func applyEditorUpdateOutcome(
        _ updateJson: String,
        sourceEditorId: String?
    ) -> EditorUpdateApplyOutcome {
        let boundEditorId = richTextView.editorId
        guard boundEditorId != 0 else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update has no bound adapter",
                fallbackClassification: "missingAdapter"
            )
            return .rejected
        }
        guard let sourceEditorId else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update source id is missing or malformed",
                fallbackClassification: "malformedSourceEditorId"
            )
            return .rejected
        }
        guard sourceEditorId == String(boundEditorId) else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update source does not match the bound canonical editor id",
                fallbackClassification: "sourceEditorMismatch"
            )
            return .rejected
        }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: boundEditorId) else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update adapter is missing",
                fallbackClassification: "missingAdapter"
            )
            return .rejected
        }
        guard !adapter.isDestroyed else {
            adapter.rejectExternalRenderEnvelope("external editor update adapter is destroyed")
            return .rejected
        }
        // A malformed external envelope is permanent, even if UIKit is in a
        // transient composition that would otherwise make application
        // retryable. Classify it before entering composition preflight.
        guard adapter.validateExternalRender(updateJson) else {
            return .rejected
        }
        if isSupersededEditorUpdate(updateJson) {
            return .applied
        }
        let preflight = richTextView.textView.prepareForExternalEditorUpdateResult()
        guard preflight.ready else {
            return .retryableDeferred
        }
        // A preflight commit already rendered and adopted a current atomic
        // snapshot. Reuse it rather than issuing a second current-state read.
        let adoptedUpdateJSON = preflight.adoptedUpdateJSON
            ?? adapter.adoptExternalRender(updateJson)
        guard let adoptedUpdateJSON else {
            // The adapter owns strict-parser and destroyed-race reporting.
            // Do not add a second view-side record for the same rejection.
            return .rejected
        }
        isApplyingJSUpdate = true
        defer { isApplyingJSUpdate = false }
        imageLoadOwner.withCurrent {
            // The adapter cache and the payload are paired by the same
            // editor-scoped call above; do not let the view display a render
            // whose revision has not already been adopted for native input.
            _ = richTextView.textView.applyUpdateJSON(adoptedUpdateJSON)
        }
        return .applied
    }

    /// Apply an editor update from JS. Sets the echo-suppression flag so the
    /// resulting delegate callback is NOT re-dispatched back to JS.
    @discardableResult
    func applyEditorUpdate(_ updateJson: String) -> Bool {
        let sourceEditorId = richTextView.editorId == 0 ? nil : String(richTextView.editorId)
        switch applyEditorUpdateOutcome(updateJson, sourceEditorId: sourceEditorId) {
        case .applied:
            return true
        case .retryableDeferred:
            scheduleViewCommandUpdateRetry(updateJson, sourceEditorId: sourceEditorId)
            return false
        case .rejected:
            return false
        }
    }

    private func scheduleViewCommandUpdateRetry(_ updateJson: String, sourceEditorId: String?) {
        pendingViewCommandUpdateJSON = updateJson
        pendingViewCommandUpdateEditorId = richTextView.editorId
        guard !pendingViewCommandUpdateRetryScheduled else { return }
        pendingViewCommandUpdateRetryScheduled = true
        pendingViewCommandUpdateRetryGeneration &+= 1
        let retryGeneration = pendingViewCommandUpdateRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingViewCommandUpdateRetryGeneration else {
                return
            }
            guard self.pendingViewCommandUpdateJSON != nil else {
                self.pendingViewCommandUpdateRetryScheduled = false
                return
            }
            guard self.pendingViewCommandUpdateEditorId == self.richTextView.editorId else {
                self.pendingViewCommandUpdateJSON = nil
                self.pendingViewCommandUpdateEditorId = nil
                self.pendingViewCommandUpdateRetryScheduled = false
                return
            }
            guard self.richTextView.editorId != 0 else {
                self.pendingViewCommandUpdateJSON = nil
                self.pendingViewCommandUpdateEditorId = nil
                self.pendingViewCommandUpdateRetryScheduled = false
                return
            }
            let updateJSON = self.pendingViewCommandUpdateJSON
            self.pendingViewCommandUpdateJSON = nil
            self.pendingViewCommandUpdateEditorId = nil
            self.pendingViewCommandUpdateRetryScheduled = false
            guard let updateJSON else { return }
            switch self.applyEditorUpdateOutcome(
                updateJSON,
                sourceEditorId: sourceEditorId
            ) {
            case .applied, .rejected:
                return
            case .retryableDeferred:
                self.scheduleViewCommandUpdateRetry(updateJSON, sourceEditorId: sourceEditorId)
            }
        }
    }

    func prepareForEditorCommandJSON() -> String {
        isApplyingJSUpdate = true
        defer { isApplyingJSUpdate = false }
        let preparation = richTextView.textView.prepareForExternalEditorCommand()
        return NativeEditorViewRegistry.commandPreparationJSON(
            ready: preparation.ready,
            updateJSON: preparation.updateJSON,
            blockedReason: preparation.blockedReason
        )
    }

    // MARK: - Focus Commands

    func focus() {
        _ = richTextView.textView.becomeFirstResponder()
    }

    func blur() {
        clearRecentToolbarTouch()
        _ = richTextView.textView.resignFirstResponder()
    }

    func getCaretRectJson() -> String? {
        layoutIfNeeded()
        richTextView.layoutIfNeeded()

        guard let caretRect = richTextView.currentCaretRect() else {
            return nil
        }
        let editorRect = richTextView.convert(caretRect, to: self)
        let payload: [String: Any] = [
            "x": editorRect.minX,
            "y": editorRect.minY,
            "width": editorRect.width,
            "height": editorRect.height,
            "editorWidth": bounds.width,
            "editorHeight": bounds.height,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return nil
        }
        return json
    }

    // MARK: - Focus Notifications

    @objc private func textViewDidBeginEditing(_ notification: Notification) {
        let originatingEditorId = richTextView.textView.editorId
        installOutsideTapRecognizerIfNeeded()
        richTextView.textView.refreshSelectionVisualState()
        refreshMentionQuery()
        guard let event = Self.editorScopedEventPayload(
            ["isFocused": true],
            originatingEditorId: originatingEditorId
        ) else { return }
        onFocusChange(event)
    }

    @objc private func textViewDidEndEditing(_ notification: Notification) {
        let originatingEditorId = richTextView.textView.editorId
        if consumeToolbarFocusPreservationForBlur() {
            DispatchQueue.main.async { [weak self] in
                _ = self?.richTextView.textView.becomeFirstResponder()
            }
            return
        }

        uninstallOutsideTapRecognizer()
        richTextView.textView.refreshSelectionVisualState()
        clearMentionQueryStateAndHidePopover()
        guard let event = Self.editorScopedEventPayload(
            ["isFocused": false],
            originatingEditorId: originatingEditorId
        ) else { return }
        onFocusChange(event)
    }

    @objc private func handleOutsideTap(_ recognizer: UITapGestureRecognizer) {
        guard recognizer.state == .ended else { return }
        guard richTextView.textView.isFirstResponder else { return }
        guard let tapWindow = gestureWindow ?? window else { return }
        let locationInWindow = recognizer.location(in: tapWindow)
        guard shouldHandleOutsideTap(locationInWindow: locationInWindow, touchedView: nil) else {
            return
        }
        clearRecentToolbarTouch()
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
        let locationInWindow = touch.location(in: tapWindow)
        return prepareOutsideTapForFocusHandling(
            locationInWindow: locationInWindow,
            touchedView: touch.view
        )
    }

    private func prepareOutsideTapForFocusHandling(
        locationInWindow: CGPoint,
        touchedView: UIView?
    ) -> Bool {
        if isLocationInStandaloneToolbarFrame(locationInWindow) {
            markRecentToolbarTouch()
        }
        let result = shouldHandleOutsideTap(
            locationInWindow: locationInWindow,
            touchedView: touchedView
        )
        if result {
            clearRecentToolbarTouch()
        }
        return result
    }

    private func markRecentToolbarTouch() {
        lastToolbarTouchUptime = ProcessInfo.processInfo.systemUptime
    }

    private func clearRecentToolbarTouch() {
        lastToolbarTouchUptime = -Double.infinity
    }

    private func shouldPreserveFocusAfterToolbarTouch() -> Bool {
        ProcessInfo.processInfo.systemUptime - lastToolbarTouchUptime <= 0.75
    }

    private func consumeToolbarFocusPreservationForBlur() -> Bool {
        guard shouldPreserveFocusAfterToolbarTouch() else { return false }
        clearRecentToolbarTouch()
        return true
    }

    private func isLocationInStandaloneToolbarFrame(_ locationInWindow: CGPoint) -> Bool {
        toolbarFramesInWindow.contains(where: { $0.contains(locationInWindow) })
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
        if isLocationInStandaloneToolbarFrame(locationInWindow) {
            return false
        }
        return true
    }

    // MARK: - EditorTextViewDelegate

    func editorTextView(
        _ textView: EditorTextView,
        didEndExternalTextComposition resultJSON: String
    ) {
        dispatchExternalTextCompositionEnd(resultJSON)
    }

    private func dispatchExternalTextCompositionEnd(_ resultJSON: String) {
        let payload: [String: Any] = [
            "editorId": String(richTextView.editorId),
            "resultJson": resultJSON,
        ]
        if let onExternalTextCompositionEndForTesting {
            onExternalTextCompositionEndForTesting(payload)
        } else {
            onExternalTextCompositionEnd(payload)
        }
    }

    func editorTextView(_ textView: EditorTextView, selectionDidChange anchor: UInt32, head: UInt32) {
        let originatingEditorId = textView.editorId
        let stateJSON = refreshToolbarStateFromEditorSelection()
        refreshSystemAssistantToolbarIfNeeded()
        refreshMentionQuery()
        richTextView.refreshRemoteSelections()
        var event: [String: Any] = ["anchor": Int(anchor), "head": Int(head)]
        if let stateJSON {
            event["stateJson"] = stateJSON
        }
        guard let scopedEvent = Self.editorScopedEventPayload(
            event,
            originatingEditorId: originatingEditorId
        ) else { return }
        onSelectionChange(scopedEvent)
    }

    func editorTextView(_ textView: EditorTextView, didReceiveUpdate updateJSON: String) {
        if let revision = documentVersion(fromUpdateJSON: updateJSON).flatMap(UInt64.init) {
            renderedDocumentRevision = revision
        }
        // Capture both fields from the same committed atomic update before
        // any view work can cause a rebind. The event must never relabel A's
        // update as B merely because the host changes editorId afterwards.
        let nativeCommitEvent = Self.nativeCommitEventPayload(
            originatingEditorId: String(textView.editorId),
            updateJSON: updateJSON
        )
        if let state = NativeToolbarState(updateJSON: updateJSON) {
            toolbarState = state
            accessoryToolbar.apply(state: state)
            refreshSystemAssistantToolbarIfNeeded()
        }
        refreshMentionQuery()
        richTextView.refreshRemoteSelections()
        guard !isApplyingJSUpdate else { return }
        guard let nativeCommitEvent else { return }
        onEditorUpdate(nativeCommitEvent)
    }

    /// The canonical JS commit contract. `originatingEditorId` is captured
    /// synchronously from the text view which applied `updateJSON`; it is not
    /// read from the host view after asynchronous rebind work.
    static func nativeCommitEventPayload(
        originatingEditorId: String,
        updateJSON: String
    ) -> [String: Any]? {
        guard let editorId = v2CanonicalUInt64String(originatingEditorId),
              editorId != "0",
              let nativeEditorId = UInt64(editorId),
              let data = updateJSON.data(using: .utf8),
              let update = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let rawRevision = update["documentVersion"] as? String,
              let documentRevision = v2CanonicalUInt64String(rawRevision),
              let revision = UInt64(documentRevision),
              let atomicUpdateJSON = EditorV2Registry.adapter(forLegacyId: nativeEditorId)?
                .atomicRenderJSON(matchingDocumentRevision: revision)
        else {
            return nil
        }
        return [
            "editorId": editorId,
            "documentRevision": documentRevision,
            "updateJson": atomicUpdateJSON,
        ]
    }

    /// Every non-commit view event is labelled with the editor that produced
    /// it, captured before refresh/rebind work can change the view binding.
    static func editorScopedEventPayload(
        _ payload: [String: Any],
        originatingEditorId: UInt64
    ) -> [String: Any]? {
        guard originatingEditorId != 0,
              let editorId = v2CanonicalUInt64String(String(originatingEditorId))
        else {
            return nil
        }
        var scopedPayload = payload
        scopedPayload["editorId"] = editorId
        return scopedPayload
    }

    @discardableResult
    private func refreshToolbarStateFromEditorSelection() -> String? {
        guard richTextView.editorId != 0 else { return nil }
        let stateJSON = EditorV2Shadow.getSelectionState(id: richTextView.editorId)
        guard let state = NativeToolbarState(updateJSON: stateJSON) else { return nil }
        toolbarState = state
        accessoryToolbar.apply(state: state)
        return stateJSON
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
        guard prepareForInputAccessoryMutationOrRetry(.refreshMentionQuery) else { return }

        guard let queryState = currentMentionQueryState(trigger: mentions.trigger) else {
            emitMentionQueryChange(query: "", trigger: mentions.trigger, anchor: 0, head: 0, isActive: false)
            clearMentionQueryStateAndHidePopover()
            return
        }

        let suggestions = filteredMentionSuggestions(for: queryState, config: mentions)
        mentionQueryState = queryState
        accessoryToolbar.apply(mentionTheme: richTextView.textView.theme?.mentions ?? mentions.theme)
        let didChangeToolbarHeight = accessoryToolbar.setMentionSuggestions(
            suggestions,
            trigger: mentions.trigger
        )
        refreshSystemAssistantToolbarIfNeeded()
        if didChangeToolbarHeight,
           richTextView.textView.isFirstResponder,
           richTextView.textView.inputAccessoryView === accessoryToolbar
        {
            richTextView.textView.reloadInputViews()
        }
        markAccessoryMutationSucceeded(.refreshMentionQuery)
        emitMentionQueryChange(
            query: queryState.query,
            trigger: queryState.trigger,
            anchor: queryState.anchor,
            head: queryState.head,
            isActive: true
        )
    }

    private func clearMentionQueryStateAndHidePopover() {
        guard prepareForInputAccessoryMutationOrRetry(.clearMentionQueryState) else { return }
        mentionQueryState = nil
        let didChangeToolbarHeight = accessoryToolbar.setMentionSuggestions([])
        refreshSystemAssistantToolbarIfNeeded()
        if didChangeToolbarHeight,
           richTextView.textView.isFirstResponder,
           richTextView.textView.inputAccessoryView === accessoryToolbar
        {
            richTextView.textView.reloadInputViews()
        }
        markAccessoryMutationSucceeded(.clearMentionQueryState)
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
        dispatchAddonEvent(json)
    }

    private func resolvedMentionAttrs(
        trigger: String,
        suggestion: NativeMentionSuggestion
    ) -> [String: Any] {
        var attrs = suggestion.attrs
        if attrs["label"] == nil {
            attrs["label"] = suggestion.label
        }
        if attrs["mentionSuggestionChar"] == nil {
            attrs["mentionSuggestionChar"] = trigger
        }
        return attrs
    }

    private func emitMentionSelect(
        trigger: String,
        suggestion: NativeMentionSuggestion,
        attrs: [String: Any]
    ) {
        let payload: [String: Any] = [
            "type": "mentionsSelect",
            "trigger": trigger,
            "suggestionKey": suggestion.key,
            "attrs": attrs,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }
        dispatchAddonEvent(json)
    }

    private func emitMentionSelectRequest(
        trigger: String,
        suggestion: NativeMentionSuggestion,
        attrs: [String: Any],
        range: MentionQueryState,
        preflightUpdateJSON: String? = nil
    ) {
        var payload: [String: Any] = [
            "type": "mentionsSelectRequest",
            "trigger": trigger,
            "suggestionKey": suggestion.key,
            "attrs": attrs,
            "range": [
                "anchor": Int(range.anchor),
                "head": Int(range.head),
            ],
        ]
        if let preflightUpdateJSON {
            payload["updateJson"] = preflightUpdateJSON
        }
        if let documentVersion = documentVersion(fromUpdateJSON: preflightUpdateJSON) {
            payload["documentVersion"] = documentVersion
        }
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }
        dispatchAddonEvent(json)
    }

    private func dispatchAddonEvent(_ json: String) {
        let originatingEditorId = richTextView.editorId
        lastAddonEventJSONForTestingValue = json
        guard let event = Self.editorScopedEventPayload(
            ["eventJson": json],
            originatingEditorId: originatingEditorId
        ) else { return }
        onAddonEvent(event)
    }

    private func isSupersededEditorUpdate(_ updateJSON: String) -> Bool {
        guard let rendered = renderedDocumentRevision,
              let incoming = documentVersion(fromUpdateJSON: updateJSON).flatMap(UInt64.init)
        else {
            return false
        }
        return incoming < rendered
    }

    private func documentVersion(fromUpdateJSON updateJSON: String?) -> String? {
        guard let updateJSON,
              let data = updateJSON.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }
        return (raw["documentVersion"] as? String).flatMap(v2CanonicalUInt64String)
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

    private func insertMentionSuggestion(
        _ suggestion: NativeMentionSuggestion
    ) {
        insertMentionSuggestion(suggestionKey: suggestion.key)
    }

    private func insertMentionSuggestion(
        retryScope: PendingMentionSuggestionRetry
    ) {
        insertMentionSuggestion(
            suggestionKey: retryScope.suggestionKey,
            retryScope: retryScope
        )
    }

    private func insertMentionSuggestion(
        suggestionKey: String,
        retryScope: PendingMentionSuggestionRetry? = nil
    ) {
        guard let mentions = addons.mentions,
              mentionQueryState != nil
        else {
            return
        }
        if let retryScope,
           !isMentionSuggestionRetryScopeCurrent(retryScope)
        {
            return
        }

        let scopedQueryState = currentMentionQueryState(trigger: mentions.trigger) ?? mentionQueryState
        guard let scopedQueryState else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        let preparation = richTextView.textView.prepareForExternalEditorCommand()
        guard preparation.ready else {
            scheduleMentionSuggestionRetry(
                PendingMentionSuggestionRetry(
                    suggestionKey: suggestionKey,
                    editorId: richTextView.editorId,
                    trigger: mentions.trigger,
                    query: scopedQueryState.query,
                    anchor: scopedQueryState.anchor,
                    head: scopedQueryState.head,
                    documentVersion: currentDocumentVersion(),
                    textSnapshot: richTextView.textView.text ?? ""
                )
            )
            return
        }
        let queryState = currentMentionQueryState(trigger: mentions.trigger)
            ?? (richTextView.textView.isFirstResponder ? nil : mentionQueryState)
        guard let queryState else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        if let retryScope,
           !doesMentionQueryState(
                queryState,
                match: retryScope,
                acceptingPreflightDocumentVersion: documentVersion(fromUpdateJSON: preparation.updateJSON),
                currentText: richTextView.textView.text ?? ""
           )
        {
            return
        }
        guard let currentSuggestion = filteredMentionSuggestions(
            for: queryState,
            config: mentions
        ).first(where: { $0.key == suggestionKey }) else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        mentionQueryState = queryState

        let attrs = resolvedMentionAttrs(trigger: mentions.trigger, suggestion: currentSuggestion)
        if mentions.resolveSelectionAttrs || mentions.resolveTheme {
            emitMentionSelectRequest(
                trigger: mentions.trigger,
                suggestion: currentSuggestion,
                attrs: attrs,
                range: queryState,
                preflightUpdateJSON: preparation.updateJSON
            )
            lastMentionEventJSON = nil
            clearMentionQueryStateAndHidePopover()
            return
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

        let updateJSON = EditorV2Shadow.insertContentJsonAtSelectionScalar(
            id: richTextView.editorId,
            scalarAnchor: queryState.anchor,
            scalarHead: queryState.head,
            json: json
        )
        imageLoadOwner.withCurrent {
            _ = richTextView.textView.applyUpdateJSON(updateJSON)
        }
        emitMentionSelect(trigger: mentions.trigger, suggestion: currentSuggestion, attrs: attrs)
        lastMentionEventJSON = nil
        clearMentionQueryStateAndHidePopover()
    }

    private func scheduleMentionSuggestionRetry(_ retry: PendingMentionSuggestionRetry) {
        pendingMentionSuggestionRetry = retry
        guard !pendingMentionSuggestionRetryScheduled else { return }
        pendingMentionSuggestionRetryScheduled = true
        pendingMentionSuggestionRetryGeneration &+= 1
        let retryGeneration = pendingMentionSuggestionRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingMentionSuggestionRetryGeneration else { return }
            guard let retry = self.pendingMentionSuggestionRetry else {
                self.pendingMentionSuggestionRetryScheduled = false
                return
            }
            guard retry.editorId == self.richTextView.editorId else {
                self.clearPendingMentionSuggestionRetry()
                return
            }
            self.pendingMentionSuggestionRetry = nil
            self.pendingMentionSuggestionRetryScheduled = false
            self.insertMentionSuggestion(retryScope: retry)
        }
    }

    private func isMentionSuggestionRetryScopeCurrent(
        _ retry: PendingMentionSuggestionRetry
    ) -> Bool {
        guard retry.editorId == richTextView.editorId,
              addons.mentions?.trigger == retry.trigger
        else {
            return false
        }
        let queryState = currentMentionQueryState(trigger: retry.trigger) ?? mentionQueryState
        guard let queryState else { return false }
        guard doesMentionQueryStateMatchRetryIdentity(queryState, match: retry) else {
            return false
        }
        return isMentionSuggestionRetryDocumentVersionCurrent(retry)
    }

    private func doesMentionQueryState(
        _ queryState: MentionQueryState,
        match retry: PendingMentionSuggestionRetry,
        acceptingPreflightDocumentVersion preflightDocumentVersion: String? = nil,
        currentText: String? = nil
    ) -> Bool {
        guard doesMentionQueryStateMatchRetryIdentity(queryState, match: retry) else {
            return false
        }

        let currentVersion = currentDocumentVersion()
        var acceptedPreflightVersionChange = false
        if let retryVersion = retry.documentVersion,
           let currentVersion,
           currentVersion != retryVersion
        {
            guard let preflightDocumentVersion,
                  currentVersion == preflightDocumentVersion
            else {
                return false
            }
            acceptedPreflightVersionChange = true
        }

        if queryState.anchor == retry.anchor && queryState.head == retry.head {
            return true
        }

        guard acceptedPreflightVersionChange else {
            return false
        }

        guard let currentText,
              let diff = mentionRetryTextDiff(
                from: retry.textSnapshot,
                to: currentText
              ),
              let mappedRange = mappedMentionRetryRange(retry, through: diff)
        else {
            return false
        }

        return queryState.anchor == mappedRange.anchor && queryState.head == mappedRange.head
    }

    private func doesMentionQueryStateMatchRetryIdentity(
        _ queryState: MentionQueryState,
        match retry: PendingMentionSuggestionRetry
    ) -> Bool {
        queryState.trigger == retry.trigger && queryState.query == retry.query
    }

    private func isMentionSuggestionRetryDocumentVersionCurrent(
        _ retry: PendingMentionSuggestionRetry
    ) -> Bool {
        let currentVersion = currentDocumentVersion()
        if let retryVersion = retry.documentVersion,
           let currentVersion,
           currentVersion != retryVersion
        {
            return false
        }
        return true
    }

    private func mentionRetryTextDiff(
        from oldText: String,
        to newText: String
    ) -> MentionRetryTextDiff? {
        let oldScalars = Array(oldText.unicodeScalars)
        let newScalars = Array(newText.unicodeScalars)
        let sharedLength = min(oldScalars.count, newScalars.count)

        var prefix = 0
        while prefix < sharedLength,
              oldScalars[prefix] == newScalars[prefix]
        {
            prefix += 1
        }

        var oldEnd = oldScalars.count
        var newEnd = newScalars.count
        while oldEnd > prefix,
              newEnd > prefix,
              oldScalars[oldEnd - 1] == newScalars[newEnd - 1]
        {
            oldEnd -= 1
            newEnd -= 1
        }

        guard prefix != oldEnd || prefix != newEnd else {
            return nil
        }

        return MentionRetryTextDiff(
            start: prefix,
            oldEnd: oldEnd,
            newEnd: newEnd
        )
    }

    private func mappedMentionRetryRange(
        _ retry: PendingMentionSuggestionRetry,
        through diff: MentionRetryTextDiff
    ) -> (anchor: UInt32, head: UInt32)? {
        let anchor = Int(retry.anchor)
        let head = Int(retry.head)
        guard anchor <= head else { return nil }

        if head <= diff.start {
            return (retry.anchor, retry.head)
        }

        if anchor >= diff.oldEnd {
            let delta = diff.newEnd - diff.oldEnd
            let mappedAnchor = anchor + delta
            let mappedHead = head + delta
            guard mappedAnchor >= 0,
                  mappedHead >= mappedAnchor,
                  mappedHead <= Int(UInt32.max)
            else {
                return nil
            }
            return (UInt32(mappedAnchor), UInt32(mappedHead))
        }

        return nil
    }

    private func currentDocumentVersion() -> String? {
        guard richTextView.editorId != 0 else { return nil }
        return documentVersion(fromUpdateJSON: EditorV2Shadow.getCurrentState(id: richTextView.editorId))
    }

    func setMentionQueryStateForTesting(_ state: MentionQueryState?) {
        mentionQueryState = state
    }

    func currentMentionQueryStateForTesting(trigger: String) -> MentionQueryState? {
        currentMentionQueryState(trigger: trigger)
    }

    func setMentionSuggestionsForTesting(_ suggestions: [NativeMentionSuggestion]) {
        accessoryToolbar.setMentionSuggestions(
            suggestions,
            trigger: mentionQueryState?.trigger ?? "@"
        )
    }

    func isShowingMentionSuggestionsForTesting() -> Bool {
        accessoryToolbar.isShowingMentionSuggestions
    }

    func lastAddonEventJSONForTesting() -> String? {
        lastAddonEventJSONForTestingValue
    }

    func triggerMentionSuggestionTapForTesting(at index: Int) {
        accessoryToolbar.triggerMentionSuggestionTapForTesting(at: index)
    }

    func inputAccessoryViewForTesting() -> UIView? {
        richTextView.textView.inputAccessoryView
    }

    func isUsingAccessoryToolbarForTesting() -> Bool {
        richTextView.textView.inputAccessoryView === accessoryToolbar
    }

    func isUsingAccessoryPlaceholderForTesting() -> Bool {
        richTextView.textView.inputAccessoryView === accessoryPlaceholder
    }

    func markRecentToolbarTouchForTesting() {
        markRecentToolbarTouch()
    }

    func shouldPreserveFocusAfterToolbarTouchForTesting() -> Bool {
        shouldPreserveFocusAfterToolbarTouch()
    }

    func consumeToolbarFocusPreservationForTesting() -> Bool {
        consumeToolbarFocusPreservationForBlur()
    }

    func prepareOutsideTapForFocusHandlingForTesting(
        locationInWindow: CGPoint,
        touchedView: UIView? = nil
    ) -> Bool {
        prepareOutsideTapForFocusHandling(
            locationInWindow: locationInWindow,
            touchedView: touchedView
        )
    }

    private func updateAccessoryToolbarVisibility() {
        guard prepareForInputAccessoryMutationOrRetry(.updateAccessoryToolbarVisibility) else { return }
        refreshSystemAssistantToolbarIfNeeded()
        let nextAccessoryView: UIView?
        if showsToolbar &&
            toolbarPlacement == "keyboard" &&
            richTextView.textView.isEditable &&
            !shouldUseSystemAssistantToolbar
        {
            nextAccessoryView = accessoryToolbar
        } else if richTextView.textView.isEditable && !shouldUseSystemAssistantToolbar {
            nextAccessoryView = accessoryPlaceholder
        } else {
            nextAccessoryView = nil
        }
        if richTextView.textView.inputAccessoryView !== nextAccessoryView {
            richTextView.textView.inputAccessoryView = nextAccessoryView
            if richTextView.textView.isFirstResponder {
                richTextView.textView.reloadInputViews()
            }
        }
        markAccessoryMutationSucceeded(.updateAccessoryToolbarVisibility)
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
        let originatingEditorId = richTextView.editorId
        switch item.type {
        case .mark:
            guard let mark = item.mark else { return }
            richTextView.textView.performToolbarToggleMark(mark)
        case .heading:
            guard let level = item.headingLevel else { return }
            richTextView.textView.performToolbarToggleHeading(level)
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
            guard let event = Self.editorScopedEventPayload(
                ["key": key],
                originatingEditorId: originatingEditorId
            ) else { return }
            onToolbarAction(event)
        case .group:
            break
        case .separator:
            break
        }
    }
}
