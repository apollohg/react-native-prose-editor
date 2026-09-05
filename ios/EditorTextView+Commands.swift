import UIKit
import os

extension EditorTextView {
    func performToolbarToggleMark(_ markName: String) {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.toggleMarkAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head,
                markName: markName
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarToggleList(_ listType: String, isActive: Bool) {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = isActive
                ? EditorV2Shadow.unwrapFromListAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head
                )
                : EditorV2Shadow.wrapInListAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head,
                    listType: listType
                )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarToggleBlockquote() {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.toggleBlockquoteAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarToggleHeading(_ level: Int) {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        guard let level = UInt8(exactly: level), (1...6).contains(level) else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.toggleHeadingAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head,
                level: level
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarIndentListItem() {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.indentListItemAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarOutdentListItem() {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.outdentListItemAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarInsertNode(_ nodeType: String) {
        guard prepareForToolbarCommand() else { return }
        performInterceptedInput {
            insertNodeInRust(nodeType)
        }
    }

    func performToolbarUndo() {
        guard prepareForToolbarCommand() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.undo(id: editorId)
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarRedo() {
        guard prepareForToolbarCommand() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.redo(id: editorId)
            applyUpdateJSON(updateJSON)
        }
    }

    private func prepareForToolbarCommand() -> Bool {
        guard editorId != 0 else { return false }
        guard isEditable else { return false }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return false }
        return prepareForExternalEditorUpdate()
    }

    /// Insert text at a scalar position via the Rust editor.
    func insertTextInRust(_ text: String, at scalarPos: UInt32) {
        Self.inputLog.debug(
            "[rust.insertTextScalar] text=\(self.preview(text), privacy: .public) scalarPos=\(scalarPos) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.insertTextScalar(id: editorId, scalarPos: scalarPos, text: text)
        applyUpdateJSON(updateJSON)
    }

    private func replaceTextRangeInRust(from: UInt32, to: UInt32, with text: String) {
        Self.inputLog.debug(
            "[rust.replaceTextScalar] text=\(self.preview(text), privacy: .public) scalar=\(from)-\(to) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.replaceTextScalar(
            id: editorId,
            scalarFrom: from,
            scalarTo: to,
            text: text
        )
        applyUpdateJSON(updateJSON)
    }

    func insertNodeInRust(_ nodeType: String) {
        guard let selection = currentScalarSelection() else { return }
        Self.inputLog.debug(
            "[rust.insertNode] nodeType=\(nodeType, privacy: .public) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.insertNodeAtSelectionScalar(
            id: editorId,
            scalarAnchor: selection.anchor,
            scalarHead: selection.head,
            nodeType: nodeType
        )
        applyUpdateJSON(updateJSON)
    }

    /// Delete a scalar range via the Rust editor.
    func deleteScalarRangeInRust(from: UInt32, to: UInt32) {
        guard from < to else { return }
        Self.inputLog.debug(
            "[rust.deleteScalarRange] scalar=\(from)-\(to) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.deleteScalarRange(id: editorId, scalarFrom: from, scalarTo: to)
        applyUpdateJSON(updateJSON)
    }

    func deleteBackwardAtSelectionScalarInRust(anchor: UInt32, head: UInt32) {
        Self.inputLog.debug(
            "[rust.deleteBackwardAtSelectionScalar] scalar=\(anchor)-\(head) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.deleteBackwardAtSelectionScalar(
            id: editorId,
            scalarAnchor: anchor,
            scalarHead: head
        )
        applyUpdateJSON(updateJSON)
    }

    func toggleTaskItemCheckedAtSelectionScalarInRust(anchor: UInt32, head: UInt32) {
        Self.inputLog.debug(
            "[rust.toggleTaskItemCheckedAtSelectionScalar] scalar=\(anchor)-\(head) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.toggleTaskItemCheckedAtSelectionScalar(
            id: editorId,
            scalarAnchor: anchor,
            scalarHead: head
        )
        applyUpdateJSON(updateJSON)
    }

    /// Delete a document-position range via the Rust editor.
    private func deleteRangeInRust(from: UInt32, to: UInt32) {
        guard from < to else { return }
        Self.inputLog.debug(
            "[rust.deleteRange] doc=\(from)-\(to) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.deleteRange(id: editorId, from: from, to: to)
        applyUpdateJSON(updateJSON)
    }

}
