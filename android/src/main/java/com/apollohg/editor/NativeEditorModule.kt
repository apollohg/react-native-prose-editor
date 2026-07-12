package com.apollohg.editor

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import org.json.JSONObject
import uniffi.editor_core.*

internal fun nativeULong(value: Int): ULong? =
    if (value >= 0) value.toULong() else null

internal fun nativeUInt(value: Int): UInt? =
    if (value >= 0) value.toUInt() else null

internal fun nativeArgumentError(field: String): String =
    "{\"error\":\"invalid $field\"}"

internal fun createdEditorId(resultJson: String): ULong? {
    val result = runCatching { JSONObject(resultJson) }.getOrNull() ?: return null
    if (result.has("error") || !result.has("editorId")) return null
    val matches = Regex("""^\s*\{\s*"editorId"\s*:\s*(0|[1-9][0-9]*)\s*}\s*$""")
        .findAll(resultJson)
        .toList()
    if (matches.size != 1) return null
    val editorId = matches.single().groupValues[1].toLongOrNull() ?: return null
    return editorId.takeIf { it > 0 }?.toULong()
}

internal fun registerCreatedEditorResult(
    resultJson: String,
    markCreated: (Long) -> Unit = NativeEditorViewRegistry::markEditorCreated
): String {
    val editorId = createdEditorId(resultJson) ?: return resultJson
    markCreated(editorId.toLong())
    return resultJson
}

internal fun destroyEditorThenInvalidate(
    editorId: ULong,
    destroy: (ULong) -> Unit = ::editorDestroy,
    beginDestroy: (Long) -> Boolean = NativeEditorViewRegistry::beginDestroy,
    finalizeDestroy: (Long) -> Unit = NativeEditorViewRegistry::finalizeDestroy
) {
    val signedEditorId = editorId.toLong()
    if (!beginDestroy(signedEditorId)) return
    try {
        destroy(editorId)
    } finally {
        finalizeDestroy(signedEditorId)
    }
}

class NativeEditorModule : Module() {
    override fun definition() = ModuleDefinition {
        Name("NativeEditor")

        Function("editorCreateResult") { configJson: String ->
            registerCreatedEditorResult(editorCreateResult(configJson))
        }
        Function("editorDestroy") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function
            destroyEditorThenInvalidate(editorId)
        }
        Function("editorPrepareForCommand") { id: Int ->
            nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            NativeEditorViewRegistry.prepareForCommandJSON(id.toLong())
        }
        Function("collaborationSessionCreate") { configJson: String ->
            collaborationSessionCreate(configJson).toLong()
        }
        Function("collaborationSessionDestroy") { id: Int ->
            val sessionId = nativeULong(id) ?: return@Function
            collaborationSessionDestroy(sessionId)
        }
        Function("collaborationSessionGetDocumentJson") { id: Int ->
            val sessionId = nativeULong(id) ?: return@Function "{}"
            collaborationSessionGetDocumentJson(sessionId)
        }
        Function("collaborationSessionGetEncodedState") { id: Int ->
            val sessionId = nativeULong(id) ?: return@Function "[]"
            collaborationSessionGetEncodedState(sessionId)
        }
        Function("collaborationSessionGetPeersJson") { id: Int ->
            val sessionId = nativeULong(id) ?: return@Function "[]"
            collaborationSessionGetPeersJson(sessionId)
        }
        Function("collaborationSessionStart") { id: Int ->
            val sessionId = nativeULong(id) ?: return@Function nativeArgumentError("session id")
            collaborationSessionStart(sessionId)
        }
        Function("collaborationSessionApplyLocalDocumentJson") { id: Int, json: String ->
            val sessionId = nativeULong(id) ?: return@Function nativeArgumentError("session id")
            collaborationSessionApplyLocalDocumentJson(sessionId, json)
        }
        Function("collaborationSessionApplyEncodedState") { id: Int, encodedStateJson: String ->
            val sessionId = nativeULong(id) ?: return@Function nativeArgumentError("session id")
            collaborationSessionApplyEncodedState(sessionId, encodedStateJson)
        }
        Function("collaborationSessionReplaceEncodedState") { id: Int, encodedStateJson: String ->
            val sessionId = nativeULong(id) ?: return@Function nativeArgumentError("session id")
            collaborationSessionReplaceEncodedState(sessionId, encodedStateJson)
        }
        Function("collaborationSessionHandleMessage") { id: Int, messageJson: String ->
            val sessionId = nativeULong(id) ?: return@Function nativeArgumentError("session id")
            collaborationSessionHandleMessage(sessionId, messageJson)
        }
        Function("collaborationSessionSetLocalAwareness") { id: Int, awarenessJson: String ->
            val sessionId = nativeULong(id) ?: return@Function nativeArgumentError("session id")
            collaborationSessionSetLocalAwareness(sessionId, awarenessJson)
        }
        Function("collaborationSessionClearLocalAwareness") { id: Int ->
            val sessionId = nativeULong(id) ?: return@Function nativeArgumentError("session id")
            collaborationSessionClearLocalAwareness(sessionId)
        }

        Function("editorSetHtml") { id: Int, html: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorSetHtml(editorId, html)
        }
        Function("editorGetHtml") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function ""
            editorGetHtml(editorId)
        }
        Function("editorSetJson") { id: Int, json: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorSetJson(editorId, json)
        }
        Function("editorGetJson") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function "{}"
            editorGetJson(editorId)
        }
        Function("editorGetContentSnapshot") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function "{\"html\":\"\",\"json\":{}}"
            editorGetContentSnapshot(editorId)
        }

        Function("editorInsertText") { id: Int, pos: Int, text: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val position = nativeUInt(pos) ?: return@Function nativeArgumentError("position")
            editorInsertText(editorId, position, text)
        }
        Function("editorReplaceSelectionText") { id: Int, text: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorReplaceSelectionText(editorId, text)
        }
        Function("editorDeleteRange") { id: Int, from: Int, to: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val fromPosition = nativeUInt(from) ?: return@Function nativeArgumentError("position")
            val toPosition = nativeUInt(to) ?: return@Function nativeArgumentError("position")
            editorDeleteRange(editorId, fromPosition, toPosition)
        }
        Function("editorSplitBlock") { id: Int, pos: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val position = nativeUInt(pos) ?: return@Function nativeArgumentError("position")
            editorSplitBlock(editorId, position)
        }
        Function("editorInsertContentHtml") { id: Int, html: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorInsertContentHtml(editorId, html)
        }
        Function("editorReplaceHtml") { id: Int, html: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorReplaceHtml(editorId, html)
        }
        Function("editorReplaceJson") { id: Int, json: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorReplaceJson(editorId, json)
        }
        Function("editorInsertContentJson") { id: Int, json: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorInsertContentJson(editorId, json)
        }
        Function(
            "editorInsertContentJsonAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, json: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorInsertContentJsonAtSelectionScalar(
                editorId,
                anchor,
                head,
                json
            )
        }
        Function("editorWrapInList") { id: Int, listType: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorWrapInList(editorId, listType)
        }
        Function("editorUnwrapFromList") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorUnwrapFromList(editorId)
        }
        Function("editorIndentListItem") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorIndentListItem(editorId)
        }
        Function("editorOutdentListItem") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorOutdentListItem(editorId)
        }
        Function("editorInsertNode") { id: Int, nodeType: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorInsertNode(editorId, nodeType)
        }

        Function("editorInsertTextScalar") { id: Int, scalarPos: Int, text: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val position = nativeUInt(scalarPos) ?: return@Function nativeArgumentError("position")
            editorInsertTextScalar(editorId, position, text)
        }
        Function("editorDeleteScalarRange") { id: Int, scalarFrom: Int, scalarTo: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val fromPosition = nativeUInt(scalarFrom) ?: return@Function nativeArgumentError("position")
            val toPosition = nativeUInt(scalarTo) ?: return@Function nativeArgumentError("position")
            editorDeleteScalarRange(editorId, fromPosition, toPosition)
        }
        Function("editorReplaceTextScalar") { id: Int, scalarFrom: Int, scalarTo: Int, text: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val fromPosition = nativeUInt(scalarFrom) ?: return@Function nativeArgumentError("position")
            val toPosition = nativeUInt(scalarTo) ?: return@Function nativeArgumentError("position")
            editorReplaceTextScalar(editorId, fromPosition, toPosition, text)
        }
        Function("editorSplitBlockScalar") { id: Int, scalarPos: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val position = nativeUInt(scalarPos) ?: return@Function nativeArgumentError("position")
            editorSplitBlockScalar(editorId, position)
        }
        Function("editorDeleteAndSplitScalar") { id: Int, scalarFrom: Int, scalarTo: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val fromPosition = nativeUInt(scalarFrom) ?: return@Function nativeArgumentError("position")
            val toPosition = nativeUInt(scalarTo) ?: return@Function nativeArgumentError("position")
            editorDeleteAndSplitScalar(editorId, fromPosition, toPosition)
        }
        Function(
            "editorToggleMarkAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, markName: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorToggleMarkAtSelectionScalar(
                editorId,
                anchor,
                head,
                markName
            )
        }
        Function(
            "editorSetMarkAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, markName: String, attrsJson: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorSetMarkAtSelectionScalar(
                editorId,
                anchor,
                head,
                markName,
                attrsJson
            )
        }
        Function(
            "editorUnsetMarkAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, markName: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorUnsetMarkAtSelectionScalar(
                editorId,
                anchor,
                head,
                markName
            )
        }
        Function(
            "editorToggleBlockquoteAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorToggleBlockquoteAtSelectionScalar(
                editorId,
                anchor,
                head
            )
        }
        Function(
            "editorToggleCodeBlockAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorToggleCodeBlockAtSelectionScalar(
                editorId,
                anchor,
                head
            )
        }
        Function(
            "editorToggleHeadingAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, level: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            if (level !in 1..6) {
                return@Function "{\"error\":\"invalid heading level\"}"
            }
            editorToggleHeadingAtSelectionScalar(
                editorId,
                anchor,
                head,
                level.toUByte()
            )
        }
        Function(
            "editorWrapInListAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, listType: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorWrapInListAtSelectionScalar(
                editorId,
                anchor,
                head,
                listType
            )
        }
        Function(
            "editorUnwrapFromListAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorUnwrapFromListAtSelectionScalar(
                editorId,
                anchor,
                head
            )
        }
        Function(
            "editorIndentListItemAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorIndentListItemAtSelectionScalar(
                editorId,
                anchor,
                head
            )
        }
        Function(
            "editorOutdentListItemAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorOutdentListItemAtSelectionScalar(
                editorId,
                anchor,
                head
            )
        }
        Function(
            "editorInsertNodeAtSelectionScalar"
        ) { id: Int, scalarAnchor: Int, scalarHead: Int, nodeType: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            val anchor = nativeUInt(scalarAnchor) ?: return@Function nativeArgumentError("position")
            val head = nativeUInt(scalarHead) ?: return@Function nativeArgumentError("position")
            editorInsertNodeAtSelectionScalar(
                editorId,
                anchor,
                head,
                nodeType
            )
        }

        Function("editorToggleMark") { id: Int, markName: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorToggleMark(editorId, markName)
        }
        Function("editorSetMark") { id: Int, markName: String, attrsJson: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorSetMark(editorId, markName, attrsJson)
        }
        Function("editorUnsetMark") { id: Int, markName: String ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorUnsetMark(editorId, markName)
        }
        Function("editorToggleBlockquote") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorToggleBlockquote(editorId)
        }
        Function("editorToggleCodeBlock") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorToggleCodeBlock(editorId)
        }
        Function("editorToggleHeading") { id: Int, level: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            if (level !in 1..6) {
                return@Function "{\"error\":\"invalid heading level\"}"
            }
            editorToggleHeading(editorId, level.toUByte())
        }

        Function("editorSetSelection") { id: Int, anchor: Int, head: Int ->
            val editorId = nativeULong(id) ?: return@Function
            val anchorPosition = nativeUInt(anchor) ?: return@Function
            val headPosition = nativeUInt(head) ?: return@Function
            editorSetSelection(editorId, anchorPosition, headPosition)
        }
        Function("editorSetSelectionScalar") { id: Int, scalarAnchor: Int, scalarHead: Int ->
            val editorId = nativeULong(id) ?: return@Function
            val anchor = nativeUInt(scalarAnchor) ?: return@Function
            val head = nativeUInt(scalarHead) ?: return@Function
            editorSetSelectionScalar(editorId, anchor, head)
        }
        Function("editorGetSelection") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function "{\"type\":\"text\",\"anchor\":0,\"head\":0}"
            editorGetSelection(editorId)
        }
        Function("editorGetSelectionState") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorGetSelectionState(editorId)
        }
        Function("editorDocToScalar") { id: Int, docPos: Int ->
            val editorId = nativeULong(id) ?: return@Function 0
            val position = nativeUInt(docPos) ?: return@Function 0
            editorDocToScalar(editorId, position).toInt()
        }
        Function("editorScalarToDoc") { id: Int, scalar: Int ->
            val editorId = nativeULong(id) ?: return@Function 0
            val position = nativeUInt(scalar) ?: return@Function 0
            editorScalarToDoc(editorId, position).toInt()
        }

        Function("editorGetCurrentState") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorGetCurrentState(editorId)
        }

        Function("editorUndo") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorUndo(editorId)
        }
        Function("editorRedo") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function nativeArgumentError("editor id")
            editorRedo(editorId)
        }
        Function("editorCanUndo") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function false
            editorCanUndo(editorId)
        }
        Function("editorCanRedo") { id: Int ->
            val editorId = nativeULong(id) ?: return@Function false
            editorCanRedo(editorId)
        }
        Function("renderDocumentJson") { configJson: String, json: String ->
            val creation = editorCreateResult(configJson)
            val editorId = createdEditorId(creation) ?: return@Function creation
            try {
                editorSetJson(editorId, json)
            } finally {
                editorDestroy(editorId)
            }
        }
        Function("measureContentHeight") { renderJson: String, themeJson: String?, width: Double ->
            val density = appContext.reactContext?.resources?.displayMetrics?.density ?: 1f
            val height = RenderBridge.measureHeight(
                json = renderJson,
                themeJson = themeJson,
                width = width.toFloat(),
                density = density
            )
            height.toDouble()
        }
        Function("renderDocumentHtml") { configJson: String, html: String ->
            val creation = editorCreateResult(configJson)
            val editorId = createdEditorId(creation) ?: return@Function creation
            try {
                editorSetHtml(editorId, html)
            } finally {
                editorDestroy(editorId)
            }
        }

        View(NativeEditorExpoView::class) {
            Events(
                "onEditorUpdate",
                "onSelectionChange",
                "onFocusChange",
                "onContentHeightChange",
                "onEditorReady",
                "onToolbarAction",
                "onAddonEvent"
            )

            Prop("editorId") { view: NativeEditorExpoView, id: Int ->
                view.setEditorId(nativeULong(id)?.toLong() ?: 0L)
            }
            Prop("editable") { view: NativeEditorExpoView, editable: Boolean ->
                view.setEditable(editable)
            }
            Prop("placeholder") { view: NativeEditorExpoView, placeholder: String ->
                view.richTextView.editorEditText.placeholderText = placeholder
            }
            Prop("autoFocus") { view: NativeEditorExpoView, autoFocus: Boolean ->
                view.setAutoFocus(autoFocus)
            }
            Prop("autoCapitalize") { view: NativeEditorExpoView, autoCapitalize: String? ->
                view.setAutoCapitalize(autoCapitalize)
            }
            Prop("autoCorrect") { view: NativeEditorExpoView, autoCorrect: Boolean? ->
                view.setAutoCorrect(autoCorrect)
            }
            Prop("keyboardType") { view: NativeEditorExpoView, keyboardType: String? ->
                view.setKeyboardType(keyboardType)
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
            Prop("imageLoadingPolicyJson") { view: NativeEditorExpoView, policyJson: String? ->
                view.setImageLoadingPolicyJson(policyJson)
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
            Prop("editorUpdateEditorId") { view: NativeEditorExpoView, editorUpdateEditorId: Int? ->
                view.setPendingEditorUpdateEditorId(editorUpdateEditorId?.let { nativeULong(it)?.toLong() })
            }
            Prop("editorUpdateRevision") { view: NativeEditorExpoView, editorUpdateRevision: Int ->
                view.setPendingEditorUpdateRevision(editorUpdateRevision)
            }
            Prop("editorResetUpdateJson") { view: NativeEditorExpoView, editorResetUpdateJson: String? ->
                view.setPendingEditorResetUpdateJson(editorResetUpdateJson)
            }
            Prop("editorResetUpdateEditorId") { view: NativeEditorExpoView, editorResetUpdateEditorId: Int? ->
                view.setPendingEditorResetUpdateEditorId(editorResetUpdateEditorId?.let { nativeULong(it)?.toLong() })
            }
            Prop("editorResetUpdateRevision") { view: NativeEditorExpoView, editorResetUpdateRevision: Int ->
                view.setPendingEditorResetUpdateRevision(editorResetUpdateRevision)
            }
            OnViewDidUpdateProps { view: NativeEditorExpoView ->
                view.applyPendingEditorResetUpdateIfNeeded()
                view.applyPendingEditorUpdateIfNeeded()
            }

            AsyncFunction("focus") { view: NativeEditorExpoView ->
                view.focus()
            }
            AsyncFunction("blur") { view: NativeEditorExpoView ->
                view.blur()
            }

            AsyncFunction("getCaretRect") { view: NativeEditorExpoView ->
                view.getCaretRectJson()
            }

            AsyncFunction("applyEditorUpdate") { view: NativeEditorExpoView, updateJson: String ->
                view.applyEditorUpdate(updateJson)
            }

            AsyncFunction("applyEditorResetUpdate") { view: NativeEditorExpoView, updateJson: String ->
                view.applyEditorResetUpdate(updateJson)
            }

        }

        View(NativeProseViewerExpoView::class) {
            Name("NativeProseViewer")
            Events("onContentHeightChange", "onPressLink", "onPressMention")

            Prop("renderJson") { view: NativeProseViewerExpoView, renderJson: String? ->
                view.setRenderJson(renderJson)
            }
            Prop("themeJson") { view: NativeProseViewerExpoView, themeJson: String? ->
                view.setThemeJson(themeJson)
            }
            Prop("imageLoadingPolicyJson") { view: NativeProseViewerExpoView, policyJson: String? ->
                view.setImageLoadingPolicyJson(policyJson)
            }
            Prop("collapsesWhenEmpty") {
                view: NativeProseViewerExpoView,
                collapsesWhenEmpty: Boolean? ->
                view.setCollapsesWhenEmpty(collapsesWhenEmpty)
            }
            Prop("enableLinkTaps") { view: NativeProseViewerExpoView, enableLinkTaps: Boolean? ->
                view.setEnableLinkTaps(enableLinkTaps)
            }
            Prop("interceptLinkTaps") { view: NativeProseViewerExpoView, interceptLinkTaps: Boolean? ->
                view.setInterceptLinkTaps(interceptLinkTaps)
            }
        }
    }
}
