import UIKit
import os

extension EditorTextView {
    func ownsNativeBinding(_ adapter: EditorV2Adapter) -> Bool {
        adapter.isNativeBindingOwner(token: nativeBindingToken)
    }

    /// Bind this text view to a Rust editor instance and apply initial content.
    ///
    /// - Parameters:
    ///   - id: The editor ID from `editor_create()`.
    ///   - initialHTML: Optional HTML to set as initial content.
    func bindEditor(
        id: UInt64,
        initialHTML: String? = nil,
        initialUpdateJSON: String? = nil
    ) {
        ensureInternalTextViewDelegate()
        if editorId == id, initialHTML == nil {
            return
        }
        localTextDragState = .idle
        if editorId != id {
            EditorV2Registry.adapter(forLegacyId: editorId)?
                .releaseNativeBindingOwner(token: nativeBindingToken)
            discardTransientNativeInputForEditorRebind()
            invalidateCurrentRenderBlocks()
        }
        editorId = id
        EditorV2Registry.adapter(forLegacyId: id)?
            .claimNativeBindingIfUnowned(token: nativeBindingToken)

        if let initialUpdateJSON {
            applyUpdateJSON(initialUpdateJSON, notifyDelegate: false)
        } else if let html = initialHTML, !html.isEmpty {
            let updateJSON = EditorV2Shadow.setHtml(id: editorId, html: html)
            if !applyUpdateJSON(updateJSON, notifyDelegate: false) {
                let stateJSON = EditorV2Shadow.getCurrentState(id: editorId)
                applyUpdateJSON(stateJSON, notifyDelegate: false)
            }
        } else {
            // Pull current state from Rust (content may already be loaded via bridge).
            let stateJSON = EditorV2Shadow.getCurrentState(id: editorId)
            applyUpdateJSON(stateJSON, notifyDelegate: false)
        }
        replayDesiredInputTraitsIfNeeded()
    }

    /// Unbind from the current editor instance.
    func unbindEditor() {
        codeHighlightingSession.cancel()
        restoreCodeHighlighting()
        discardTransientNativeInputForEditorRebind()
        EditorV2Registry.adapter(forLegacyId: editorId)?
            .releaseNativeBindingOwner(token: nativeBindingToken)
        invalidateCurrentRenderBlocks()
        editorId = 0
    }

}
