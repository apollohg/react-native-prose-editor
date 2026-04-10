package com.apollohg.editor

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import uniffi.editor_core.*

class NativeEditorModule : Module() {
    override fun definition() = ModuleDefinition {
        Name("NativeEditor")

        Function("editorCreate") { configJson: String ->
            editorCreate(configJson).toLong()
        }
        Function("editorDestroy") { id: Int ->
            editorDestroy(id.toULong())
        }
        Function("collaborationSessionCreate") { configJson: String ->
            collaborationSessionCreate(configJson).toLong()
        }
        Function("collaborationSessionDestroy") { id: Int ->
            collaborationSessionDestroy(id.toULong())
        }
        Function("collaborationSessionGetDocumentJson") { id: Int ->
            collaborationSessionGetDocumentJson(id.toULong())
        }
        Function("collaborationSessionGetEncodedState") { id: Int ->
            collaborationSessionGetEncodedState(id.toULong())
        }
        Function("collaborationSessionGetPeersJson") { id: Int ->
            collaborationSessionGetPeersJson(id.toULong())
        }
        Function("collaborationSessionStart") { id: Int ->
            collaborationSessionStart(id.toULong())
        }
        Function("collaborationSessionApplyLocalDocumentJson") { id: Int, json: String ->
            collaborationSessionApplyLocalDocumentJson(id.toULong(), json)
        }
        Function("collaborationSessionApplyEncodedState") { id: Int, encodedStateJson: String ->
            collaborationSessionApplyEncodedState(id.toULong(), encodedStateJson)
        }
        Function("collaborationSessionReplaceEncodedState") { id: Int, encodedStateJson: String ->
            collaborationSessionReplaceEncodedState(id.toULong(), encodedStateJson)
        }
        Function("collaborationSessionHandleMessage") { id: Int, messageJson: String ->
            collaborationSessionHandleMessage(id.toULong(), messageJson)
        }
        Function("collaborationSessionSetLocalAwareness") { id: Int, awarenessJson: String ->
            collaborationSessionSetLocalAwareness(id.toULong(), awarenessJson)
        }
        Function("collaborationSessionClearLocalAwareness") { id: Int ->
            collaborationSessionClearLocalAwareness(id.toULong())
        }

        Function("editorSetHtml") { id: Int, html: String ->
            editorSetHtml(id.toULong(), html)
        }
        Function("editorGetHtml") { id: Int ->
            editorGetHtml(id.toULong())
        }
        Function("editorSetJson") { id: Int, json: String ->
            editorSetJson(id.toULong(), json)
        }
        Function("editorGetJson") { id: Int ->
            editorGetJson(id.toULong())
        }
        Function("editorGetContentSnapshot") { id: Int ->
            editorGetContentSnapshot(id.toULong())
        }

        Function("editorInsertText") { id: Int, pos: Int, text: String ->
            editorInsertText(id.toULong(), pos.toUInt(), text)
        }
        Function("editorReplaceSelectionText") { id: Int, text: String ->
            editorReplaceSelectionText(id.toULong(), text)
        }
        Function("editorDeleteRange") { id: Int, from: Int, to: Int ->
            editorDeleteRange(id.toULong(), from.toUInt(), to.toUInt())
        }
        Function("editorSplitBlock") { id: Int, pos: Int ->
            editorSplitBlock(id.toULong(), pos.toUInt())
        }
        Function("editorInsertContentHtml") { id: Int, html: String ->
            editorInsertContentHtml(id.toULong(), html)
        }
        Function("editorReplaceHtml") { id: Int, html: String ->
            editorReplaceHtml(id.toULong(), html)
        }
        Function("editorReplaceJson") { id: Int, json: String ->
            editorReplaceJson(id.toULong(), json)
        }
        Function("editorInsertContentJson") { id: Int, json: String ->
            editorInsertContentJson(id.toULong(), json)
        }
        Function(
            "editorInsertContentJsonAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, json: String ->
            editorInsertContentJsonAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt(),
                json
            )
        }
        Function("editorWrapInList") { id: Int, listType: String ->
            editorWrapInList(id.toULong(), listType)
        }
        Function("editorUnwrapFromList") { id: Int ->
            editorUnwrapFromList(id.toULong())
        }
        Function("editorIndentListItem") { id: Int ->
            editorIndentListItem(id.toULong())
        }
        Function("editorOutdentListItem") { id: Int ->
            editorOutdentListItem(id.toULong())
        }
        Function("editorInsertNode") { id: Int, nodeType: String ->
            editorInsertNode(id.toULong(), nodeType)
        }

        Function("editorInsertTextScalar") { id: Int, scalarPos: Int, text: String ->
            editorInsertTextScalar(id.toULong(), scalarPos.toUInt(), text)
        }
        Function("editorDeleteScalarRange") { id: Int, scalarFrom: Int, scalarTo: Int ->
            editorDeleteScalarRange(id.toULong(), scalarFrom.toUInt(), scalarTo.toUInt())
        }
        Function("editorReplaceTextScalar") { id: Int, scalarFrom: Int, scalarTo: Int, text: String ->
            editorReplaceTextScalar(id.toULong(), scalarFrom.toUInt(), scalarTo.toUInt(), text)
        }
        Function("editorSplitBlockScalar") { id: Int, scalarPos: Int ->
            editorSplitBlockScalar(id.toULong(), scalarPos.toUInt())
        }
        Function("editorDeleteAndSplitScalar") { id: Int, scalarFrom: Int, scalarTo: Int ->
            editorDeleteAndSplitScalar(id.toULong(), scalarFrom.toUInt(), scalarTo.toUInt())
        }
        Function(
            "editorToggleMarkAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, markName: String ->
            editorToggleMarkAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt(),
                markName
            )
        }
        Function(
            "editorSetMarkAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, markName: String, attrsJson: String ->
            editorSetMarkAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt(),
                markName,
                attrsJson
            )
        }
        Function(
            "editorUnsetMarkAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, markName: String ->
            editorUnsetMarkAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt(),
                markName
            )
        }
        Function(
            "editorToggleBlockquoteAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int ->
            editorToggleBlockquoteAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt()
            )
        }
        Function(
            "editorToggleHeadingAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, level: Int ->
            if (level !in 1..6) {
                return@Function "{\"error\":\"invalid heading level\"}"
            }
            editorToggleHeadingAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt(),
                level.toUByte()
            )
        }
        Function(
            "editorWrapInListAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, listType: String ->
            editorWrapInListAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt(),
                listType
            )
        }
        Function(
            "editorUnwrapFromListAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int ->
            editorUnwrapFromListAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt()
            )
        }
        Function(
            "editorIndentListItemAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int ->
            editorIndentListItemAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt()
            )
        }
        Function(
            "editorOutdentListItemAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int ->
            editorOutdentListItemAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt()
            )
        }
        Function(
            "editorInsertNodeAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, nodeType: String ->
            editorInsertNodeAtSelectionScalar(
                id.toULong(),
                scalarAnchor.toUInt(),
                scalarHead.toUInt(),
                nodeType
            )
        }

        Function("editorToggleMark") { id: Int, markName: String ->
            editorToggleMark(id.toULong(), markName)
        }
        Function("editorSetMark") { id: Int, markName: String, attrsJson: String ->
            editorSetMark(id.toULong(), markName, attrsJson)
        }
        Function("editorUnsetMark") { id: Int, markName: String ->
            editorUnsetMark(id.toULong(), markName)
        }
        Function("editorToggleBlockquote") { id: Int ->
            editorToggleBlockquote(id.toULong())
        }
        Function("editorToggleHeading") { id: Int, level: Int ->
            if (level !in 1..6) {
                return@Function "{\"error\":\"invalid heading level\"}"
            }
            editorToggleHeading(id.toULong(), level.toUByte())
        }

        Function("editorSetSelection") { id: Int, anchor: Int, head: Int ->
            editorSetSelection(id.toULong(), anchor.toUInt(), head.toUInt())
        }
        Function("editorSetSelectionScalar") { id: Int, scalarAnchor: Int, scalarHead: Int ->
            editorSetSelectionScalar(id.toULong(), scalarAnchor.toUInt(), scalarHead.toUInt())
        }
        Function("editorGetSelection") { id: Int ->
            editorGetSelection(id.toULong())
        }
        Function("editorGetSelectionState") { id: Int ->
            editorGetSelectionState(id.toULong())
        }
        Function("editorDocToScalar") { id: Int, docPos: Int ->
            editorDocToScalar(id.toULong(), docPos.toUInt()).toInt()
        }
        Function("editorScalarToDoc") { id: Int, scalar: Int ->
            editorScalarToDoc(id.toULong(), scalar.toUInt()).toInt()
        }

        Function("editorGetCurrentState") { id: Int ->
            editorGetCurrentState(id.toULong())
        }

        Function("editorUndo") { id: Int ->
            editorUndo(id.toULong())
        }
        Function("editorRedo") { id: Int ->
            editorRedo(id.toULong())
        }
        Function("editorCanUndo") { id: Int ->
            editorCanUndo(id.toULong())
        }
        Function("editorCanRedo") { id: Int ->
            editorCanRedo(id.toULong())
        }

        View(NativeEditorExpoView::class) {
            Events(
                "onEditorUpdate",
                "onSelectionChange",
                "onFocusChange",
                "onContentHeightChange",
                "onToolbarAction",
                "onAddonEvent"
            )

            Prop("editorId") { view: NativeEditorExpoView, id: Int ->
                view.setEditorId(id.toLong())
            }
            Prop("editable") { view: NativeEditorExpoView, editable: Boolean ->
                view.richTextView.editorEditText.isEditable = editable
            }
            Prop("placeholder") { view: NativeEditorExpoView, placeholder: String ->
                view.richTextView.editorEditText.placeholderText = placeholder
            }
            Prop("autoFocus") { view: NativeEditorExpoView, autoFocus: Boolean ->
                view.setAutoFocus(autoFocus)
            }
            Prop("showToolbar") { view: NativeEditorExpoView, showToolbar: Boolean ->
                view.setShowToolbar(showToolbar)
            }
            Prop("toolbarPlacement") { view: NativeEditorExpoView, toolbarPlacement: String? ->
                view.setToolbarPlacement(toolbarPlacement)
            }
            Prop("heightBehavior") { view: NativeEditorExpoView, heightBehavior: String ->
                view.setHeightBehavior(heightBehavior)
            }
            Prop("allowImageResizing") { view: NativeEditorExpoView, allowImageResizing: Boolean ->
                view.setAllowImageResizing(allowImageResizing)
            }
            Prop("themeJson") { view: NativeEditorExpoView, themeJson: String? ->
                view.setThemeJson(themeJson)
            }
            Prop("addonsJson") { view: NativeEditorExpoView, addonsJson: String? ->
                view.setAddonsJson(addonsJson)
            }
            Prop("remoteSelectionsJson") { view: NativeEditorExpoView, remoteSelectionsJson: String? ->
                view.setRemoteSelectionsJson(remoteSelectionsJson)
            }
            Prop("toolbarItemsJson") { view: NativeEditorExpoView, toolbarItemsJson: String? ->
                view.setToolbarItemsJson(toolbarItemsJson)
            }
            Prop("toolbarFrameJson") { view: NativeEditorExpoView, toolbarFrameJson: String? ->
                view.setToolbarFrameJson(toolbarFrameJson)
            }
            Prop("editorUpdateJson") { view: NativeEditorExpoView, editorUpdateJson: String? ->
                view.setPendingEditorUpdateJson(editorUpdateJson)
            }
            Prop("editorUpdateRevision") { view: NativeEditorExpoView, editorUpdateRevision: Int ->
                view.setPendingEditorUpdateRevision(editorUpdateRevision)
            }
            OnViewDidUpdateProps { view: NativeEditorExpoView ->
                view.applyPendingEditorUpdateIfNeeded()
            }

            AsyncFunction("focus") { view: NativeEditorExpoView ->
                view.focus()
            }
            AsyncFunction("blur") { view: NativeEditorExpoView ->
                view.blur()
            }

            AsyncFunction("applyEditorUpdate") { view: NativeEditorExpoView, updateJson: String ->
                view.applyEditorUpdate(updateJson)
            }

        }
    }
}
