import ExpoModulesCore
import UIKit

extension NativeEditorExpoView {
    func updateAccessoryToolbarVisibility() {
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

    func refreshSystemAssistantToolbarIfNeeded() {
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

    func handleToolbarItemPress(_ item: NativeToolbarItem) {
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
