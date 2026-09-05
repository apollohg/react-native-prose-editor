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
