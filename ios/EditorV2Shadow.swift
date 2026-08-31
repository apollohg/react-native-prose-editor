import Foundation

// MARK: - v2 editor routing seam (production)
//
// Every engine call site inside RichTextEditorView.swift, NativeEditorExpoView
// and NativeEditorModule invokes one `EditorV2Shadow` method instead of a
// UniFFI free function directly. Each method resolves the public editor id to
// its `EditorV2Adapter` through the session pairing registry and routes the
// operation through the typed v2 transactions/results. An unpaired id is a
// destroyed/unknown editor: reads return the legacy empty-shape fallbacks and
// position mappings degrade to identity, matching the adapter's own
// nil-coalesced contract.
enum EditorV2Shadow {
    private static func adapter(for id: UInt64) -> EditorV2Adapter? {
        EditorV2Registry.adapter(forLegacyId: id)
    }

    /// The engine's current selection in scalar positions, resolved from the
    /// authoritative session state (module-level selection-based commands).
    /// Falls back to the document start when no selection was ever synced
    /// (the legacy post-`setContent` cursor-at-start semantics).
    private static func currentScalarSelection(_ adapter: EditorV2Adapter) -> (anchor: UInt32, head: UInt32) {
        guard let json = adapter.selectionJSON(),
              let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return (0, 0)
        }
        func scalar(_ scalarKey: String, docKey: String) -> UInt32? {
            if let value = v2ExactUInt32(object[scalarKey] as? NSNumber) { return value }
            if let doc = v2ExactUInt32(object[docKey] as? NSNumber) {
                return adapter.scalarPosition(forDoc: doc)
            }
            return nil
        }
        guard let anchor = scalar("anchorScalar", docKey: "anchor"),
              let head = scalar("headScalar", docKey: "head")
        else {
            return (0, 0)
        }
        return (anchor, head)
    }

    // MARK: - Content

    static func setHtml(id: UInt64, html: String) -> String {
        adapter(for: id)?.setContentHtml(html) ?? "{}"
    }

    static func setJson(id: UInt64, json: String) -> String {
        adapter(for: id)?.setContentJson(json) ?? "{}"
    }

    static func replaceHtml(id: UInt64, html: String) -> String {
        adapter(for: id)?.replaceContentHtml(html) ?? "{}"
    }

    static func replaceJson(id: UInt64, json: String) -> String {
        adapter(for: id)?.replaceContentJson(json) ?? "{}"
    }

    static func getHtml(id: UInt64) -> String {
        adapter(for: id)?.documentHtml() ?? ""
    }

    static func getJson(id: UInt64) -> String {
        adapter(for: id)?.documentJson() ?? "{}"
    }

    static func getContentSnapshot(id: UInt64) -> String {
        adapter(for: id)?.contentSnapshotJSON() ?? "{\"html\":\"\",\"json\":{}}"
    }

    // MARK: - State / selection reads

    static func getCurrentState(id: UInt64) -> String {
        adapter(for: id)?.currentStateJSON() ?? "{}"
    }

    static func getSelectionState(id: UInt64) -> String {
        adapter(for: id)?.currentStateJSON() ?? "{}"
    }

    static func getSelection(id: UInt64) -> String {
        adapter(for: id)?.selectionJSON() ?? "{\"type\":\"text\",\"anchor\":0,\"head\":0}"
    }

    static func documentRevision(id: UInt64) -> UInt64? {
        adapter(for: id)?.baseDocumentRevision
    }

    static func canUndo(id: UInt64) -> Bool {
        adapter(for: id)?.historyFlags()?.canUndo ?? false
    }

    static func canRedo(id: UInt64) -> Bool {
        adapter(for: id)?.historyFlags()?.canRedo ?? false
    }

    // MARK: - Typing / range edits (scalar currency)

    static func insertTextScalar(id: UInt64, scalarPos: UInt32, text: String) -> String {
        adapter(for: id)?.insertText(text, atScalar: scalarPos) ?? "{}"
    }

    /// Doc-position insert (module `editorInsertText`): maps the doc position
    /// through the lenient engine mapping first.
    static func insertText(id: UInt64, pos: UInt32, text: String) -> String {
        guard let adapter = adapter(for: id),
              let scalar = adapter.scalarPosition(forDoc: pos)
        else {
            return "{}"
        }
        return adapter.insertText(text, atScalar: scalar) ?? "{}"
    }

    static func replaceTextScalar(id: UInt64, scalarFrom: UInt32, scalarTo: UInt32, text: String) -> String {
        adapter(for: id)?.replaceTextRange(from: scalarFrom, to: scalarTo, with: text) ?? "{}"
    }

    /// Replace at the engine's current selection (module
    /// `editorReplaceSelectionText`).
    static func replaceSelectionText(id: UInt64, text: String) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.replaceTextRange(from: selection.anchor, to: selection.head, with: text) ?? "{}"
    }

    static func deleteScalarRange(id: UInt64, scalarFrom: UInt32, scalarTo: UInt32) -> String {
        adapter(for: id)?.deleteScalarRange(from: scalarFrom, to: scalarTo) ?? "{}"
    }

    static func deleteRange(id: UInt64, from: UInt32, to: UInt32) -> String {
        adapter(for: id)?.deleteRange(fromDoc: from, toDoc: to) ?? "{}"
    }

    static func deleteBackwardAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32) -> String {
        adapter(for: id)?.deleteBackward(anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func deleteAndSplitScalar(id: UInt64, scalarFrom: UInt32, scalarTo: UInt32) -> String {
        adapter(for: id)?.deleteAndSplit(from: scalarFrom, to: scalarTo) ?? "{}"
    }

    static func splitBlockScalar(id: UInt64, scalarPos: UInt32) -> String {
        adapter(for: id)?.splitBlock(atScalar: scalarPos) ?? "{}"
    }

    /// Doc-position split (module `editorSplitBlock`).
    static func splitBlock(id: UInt64, pos: UInt32) -> String {
        guard let adapter = adapter(for: id),
              let scalar = adapter.scalarPosition(forDoc: pos)
        else {
            return "{}"
        }
        return adapter.splitBlock(atScalar: scalar) ?? "{}"
    }

    // MARK: - Node/mark commands at an explicit scalar selection

    static func insertNodeAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32, nodeType: String) -> String {
        adapter(for: id)?.insertNode(nodeType, anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func toggleTaskItemCheckedAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32) -> String {
        adapter(for: id)?.toggleTaskItemChecked(anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func toggleMarkAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32, markName: String) -> String {
        adapter(for: id)?.toggleMark(markName, anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func setMarkAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32, markName: String, attrsJson: String) -> String {
        adapter(for: id)?.setMark(markName, attrsJson: attrsJson, anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func unsetMarkAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32, markName: String) -> String {
        adapter(for: id)?.unsetMark(markName, anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func toggleBlockquoteAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32) -> String {
        adapter(for: id)?.toggleBlockquote(anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func toggleCodeBlockAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32) -> String {
        adapter(for: id)?.toggleCodeBlock(anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func toggleHeadingAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32, level: UInt8) -> String {
        adapter(for: id)?.toggleHeading(level: level, anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func wrapInListAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32, listType: String) -> String {
        let itemType = EditorNodeTypes.listItemType(for: listType)
        return adapter(for: id)?.wrapInList(listType: listType, itemType: itemType, anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func unwrapFromListAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32) -> String {
        adapter(for: id)?.unwrapFromList(anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func indentListItemAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32) -> String {
        adapter(for: id)?.indentListItem(anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func outdentListItemAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32) -> String {
        adapter(for: id)?.outdentListItem(anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func moveSelectionAtScalar(
        id: UInt64,
        scalarAnchor: UInt32,
        scalarHead: UInt32,
        destination: UInt32
    ) -> String {
        adapter(for: id)?.moveSelection(
            anchor: scalarAnchor,
            head: scalarHead,
            to: destination
        ) ?? "{}"
    }

    // MARK: - Node/mark commands at the engine's current selection

    static func toggleMark(id: UInt64, markName: String) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.toggleMark(markName, anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    static func setMark(id: UInt64, markName: String, attrsJson: String) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.setMark(markName, attrsJson: attrsJson, anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    static func unsetMark(id: UInt64, markName: String) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.unsetMark(markName, anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    static func toggleBlockquote(id: UInt64) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.toggleBlockquote(anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    static func toggleCodeBlock(id: UInt64) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.toggleCodeBlock(anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    static func toggleHeading(id: UInt64, level: UInt8) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.toggleHeading(level: level, anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    static func wrapInList(id: UInt64, listType: String) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        let itemType = EditorNodeTypes.listItemType(for: listType)
        return adapter.wrapInList(listType: listType, itemType: itemType, anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    static func unwrapFromList(id: UInt64) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.unwrapFromList(anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    static func indentListItem(id: UInt64) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.indentListItem(anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    static func outdentListItem(id: UInt64) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.outdentListItem(anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    static func insertNode(id: UInt64, nodeType: String) -> String {
        guard let adapter = adapter(for: id) else { return "{}" }
        let selection = currentScalarSelection(adapter)
        return adapter.insertNode(nodeType, anchor: selection.anchor, head: selection.head) ?? "{}"
    }

    // MARK: - Content insertion

    static func insertContentHtml(id: UInt64, html: String) -> String {
        adapter(for: id)?.insertContentHtmlAtEngineSelection(html) ?? "{}"
    }

    static func insertContentJson(id: UInt64, json: String) -> String {
        adapter(for: id)?.insertContentJsonAtEngineSelection(json) ?? "{}"
    }

    static func insertContentJsonAtSelectionScalar(id: UInt64, scalarAnchor: UInt32, scalarHead: UInt32, json: String) -> String {
        adapter(for: id)?.insertContentJson(json, anchor: scalarAnchor, head: scalarHead) ?? "{}"
    }

    static func resizeImageAtDocPos(id: UInt64, docPos: UInt32, width: UInt32, height: UInt32) -> String {
        adapter(for: id)?.resizeImage(atDocPos: docPos, width: width, height: height) ?? "{}"
    }

    // MARK: - History

    static func undo(id: UInt64) -> String {
        adapter(for: id)?.undo() ?? "{}"
    }

    static func redo(id: UInt64) -> String {
        adapter(for: id)?.redo() ?? "{}"
    }

    // MARK: - Selection sync / position mapping

    @discardableResult
    static func setSelectionScalar(
        id: UInt64,
        scalarAnchor: UInt32,
        scalarHead: UInt32
    ) -> EditorV2SelectionSync? {
        adapter(for: id)?.syncSelection(anchor: scalarAnchor, head: scalarHead)
    }

    /// Doc-position selection sync (module `editorSetSelection`).
    static func setSelection(id: UInt64, anchor: UInt32, head: UInt32) {
        guard let adapter = adapter(for: id),
              let scalarAnchor = adapter.scalarPosition(forDoc: anchor),
              let scalarHead = adapter.scalarPosition(forDoc: head)
        else {
            return
        }
        adapter.syncSelectionQuiet(anchor: scalarAnchor, head: scalarHead)
    }

    static func scalarToDoc(id: UInt64, scalar: UInt32) -> UInt32 {
        adapter(for: id)?.documentPosition(forScalar: scalar) ?? scalar
    }

    static func docToScalar(id: UInt64, docPos: UInt32) -> UInt32 {
        adapter(for: id)?.scalarPosition(forDoc: docPos) ?? docPos
    }
}
