import ExpoModulesCore

public class NativeEditorModule: Module {
    public func definition() -> ModuleDefinition {
        Name("NativeEditor")

        Function("editorCreate") { (configJson: String) -> Int in
            Int(editorCreate(configJson: configJson))
        }
        Function("editorDestroy") { (id: Int) in
            editorDestroy(id: UInt64(id))
        }
        Function("editorSetHtml") { (id: Int, html: String) -> String in
            editorSetHtml(id: UInt64(id), html: html)
        }
        Function("editorGetHtml") { (id: Int) -> String in
            editorGetHtml(id: UInt64(id))
        }
        Function("editorSetJson") { (id: Int, json: String) -> String in
            editorSetJson(id: UInt64(id), json: json)
        }
        Function("editorGetJson") { (id: Int) -> String in
            editorGetJson(id: UInt64(id))
        }
        Function("editorInsertText") { (id: Int, pos: Int, text: String) -> String in
            editorInsertText(id: UInt64(id), pos: UInt32(pos), text: text)
        }
        Function("editorInsertTextScalar") { (id: Int, scalarPos: Int, text: String) -> String in
            editorInsertTextScalar(id: UInt64(id), scalarPos: UInt32(scalarPos), text: text)
        }
        Function("editorReplaceSelectionText") { (id: Int, text: String) -> String in
            editorReplaceSelectionText(id: UInt64(id), text: text)
        }
        Function("editorDeleteRange") { (id: Int, from: Int, to: Int) -> String in
            editorDeleteRange(id: UInt64(id), from: UInt32(from), to: UInt32(to))
        }
        Function("editorDeleteScalarRange") { (id: Int, scalarFrom: Int, scalarTo: Int) -> String in
            editorDeleteScalarRange(
                id: UInt64(id),
                scalarFrom: UInt32(scalarFrom),
                scalarTo: UInt32(scalarTo)
            )
        }
        Function(
            "editorReplaceTextScalar"
        ) { (id: Int, scalarFrom: Int, scalarTo: Int, text: String) -> String in
            editorReplaceTextScalar(
                id: UInt64(id),
                scalarFrom: UInt32(scalarFrom),
                scalarTo: UInt32(scalarTo),
                text: text
            )
        }
        Function("editorSplitBlock") { (id: Int, pos: Int) -> String in
            editorSplitBlock(id: UInt64(id), pos: UInt32(pos))
        }
        Function("editorSplitBlockScalar") { (id: Int, scalarPos: Int) -> String in
            editorSplitBlockScalar(id: UInt64(id), scalarPos: UInt32(scalarPos))
        }
        Function("editorDeleteAndSplitScalar") { (id: Int, scalarFrom: Int, scalarTo: Int) -> String in
            editorDeleteAndSplitScalar(
                id: UInt64(id),
                scalarFrom: UInt32(scalarFrom),
                scalarTo: UInt32(scalarTo)
            )
        }
        Function("editorInsertContentHtml") { (id: Int, html: String) -> String in
            editorInsertContentHtml(id: UInt64(id), html: html)
        }
        Function("editorToggleMark") { (id: Int, markName: String) -> String in
            editorToggleMark(id: UInt64(id), markName: markName)
        }
        Function("editorSetSelection") { (id: Int, anchor: Int, head: Int) in
            editorSetSelection(id: UInt64(id), anchor: UInt32(anchor), head: UInt32(head))
        }
        Function("editorSetSelectionScalar") { (id: Int, scalarAnchor: Int, scalarHead: Int) in
            editorSetSelectionScalar(
                id: UInt64(id),
                scalarAnchor: UInt32(scalarAnchor),
                scalarHead: UInt32(scalarHead)
            )
        }
        Function(
            "editorToggleMarkAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, markName: String) -> String in
            editorToggleMarkAtSelectionScalar(
                id: UInt64(id),
                scalarAnchor: UInt32(scalarAnchor),
                scalarHead: UInt32(scalarHead),
                markName: markName
            )
        }
        Function(
            "editorWrapInListAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, listType: String) -> String in
            editorWrapInListAtSelectionScalar(
                id: UInt64(id),
                scalarAnchor: UInt32(scalarAnchor),
                scalarHead: UInt32(scalarHead),
                listType: listType
            )
        }
        Function(
            "editorUnwrapFromListAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int) -> String in
            editorUnwrapFromListAtSelectionScalar(
                id: UInt64(id),
                scalarAnchor: UInt32(scalarAnchor),
                scalarHead: UInt32(scalarHead)
            )
        }
        Function(
            "editorIndentListItemAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int) -> String in
            editorIndentListItemAtSelectionScalar(
                id: UInt64(id),
                scalarAnchor: UInt32(scalarAnchor),
                scalarHead: UInt32(scalarHead)
            )
        }
        Function(
            "editorOutdentListItemAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int) -> String in
            editorOutdentListItemAtSelectionScalar(
                id: UInt64(id),
                scalarAnchor: UInt32(scalarAnchor),
                scalarHead: UInt32(scalarHead)
            )
        }
        Function(
            "editorInsertNodeAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, nodeType: String) -> String in
            editorInsertNodeAtSelectionScalar(
                id: UInt64(id),
                scalarAnchor: UInt32(scalarAnchor),
                scalarHead: UInt32(scalarHead),
                nodeType: nodeType
            )
        }
        Function("editorGetSelection") { (id: Int) -> String in
            editorGetSelection(id: UInt64(id))
        }
        Function("editorDocToScalar") { (id: Int, docPos: Int) -> Int in
            Int(editorDocToScalar(id: UInt64(id), docPos: UInt32(docPos)))
        }
        Function("editorScalarToDoc") { (id: Int, scalar: Int) -> Int in
            Int(editorScalarToDoc(id: UInt64(id), scalar: UInt32(scalar)))
        }
        Function("editorGetCurrentState") { (id: Int) -> String in
            editorGetCurrentState(id: UInt64(id))
        }
        Function("editorUndo") { (id: Int) -> String in
            editorUndo(id: UInt64(id))
        }
        Function("editorRedo") { (id: Int) -> String in
            editorRedo(id: UInt64(id))
        }
        Function("editorCanUndo") { (id: Int) -> Bool in
            editorCanUndo(id: UInt64(id))
        }
        Function("editorCanRedo") { (id: Int) -> Bool in
            editorCanRedo(id: UInt64(id))
        }
        Function("editorReplaceHtml") { (id: Int, html: String) -> String in
            editorReplaceHtml(id: UInt64(id), html: html)
        }
        Function("editorReplaceJson") { (id: Int, json: String) -> String in
            editorReplaceJson(id: UInt64(id), json: json)
        }
        Function("editorInsertContentJson") { (id: Int, json: String) -> String in
            editorInsertContentJson(id: UInt64(id), json: json)
        }
        Function(
            "editorInsertContentJsonAtSelectionScalar"
        ) { (id: Int, scalarAnchor: Int, scalarHead: Int, json: String) -> String in
            editorInsertContentJsonAtSelectionScalar(
                id: UInt64(id),
                scalarAnchor: UInt32(scalarAnchor),
                scalarHead: UInt32(scalarHead),
                json: json
            )
        }
        Function("editorWrapInList") { (id: Int, listType: String) -> String in
            editorWrapInList(id: UInt64(id), listType: listType)
        }
        Function("editorUnwrapFromList") { (id: Int) -> String in
            editorUnwrapFromList(id: UInt64(id))
        }
        Function("editorIndentListItem") { (id: Int) -> String in
            editorIndentListItem(id: UInt64(id))
        }
        Function("editorOutdentListItem") { (id: Int) -> String in
            editorOutdentListItem(id: UInt64(id))
        }
        Function("editorInsertNode") { (id: Int, nodeType: String) -> String in
            editorInsertNode(id: UInt64(id), nodeType: nodeType)
        }

        View(NativeEditorExpoView.self) {
            Events(
                "onEditorUpdate",
                "onSelectionChange",
                "onFocusChange",
                "onContentHeightChange",
                "onToolbarAction",
                "onAddonEvent"
            )

            Prop("editorId") { (view: NativeEditorExpoView, id: Int) in
                view.setEditorId(UInt64(id))
            }
            Prop("editable") { (view: NativeEditorExpoView, editable: Bool) in
                view.setEditable(editable)
            }
            Prop("placeholder") { (view: NativeEditorExpoView, placeholder: String) in
                view.richTextView.textView.placeholder = placeholder
            }
            Prop("autoFocus") { (view: NativeEditorExpoView, autoFocus: Bool) in
                view.setAutoFocus(autoFocus)
            }
            Prop("showToolbar") { (view: NativeEditorExpoView, showToolbar: Bool) in
                view.setShowToolbar(showToolbar)
            }
            Prop("toolbarPlacement") { (view: NativeEditorExpoView, toolbarPlacement: String?) in
                view.setToolbarPlacement(toolbarPlacement)
            }
            Prop("heightBehavior") { (view: NativeEditorExpoView, heightBehavior: String) in
                view.setHeightBehavior(heightBehavior)
            }
            Prop("themeJson") { (view: NativeEditorExpoView, themeJson: String?) in
                view.setThemeJson(themeJson)
            }
            Prop("addonsJson") { (view: NativeEditorExpoView, addonsJson: String?) in
                view.setAddonsJson(addonsJson)
            }
            Prop("toolbarItemsJson") { (view: NativeEditorExpoView, toolbarItemsJson: String?) in
                view.setToolbarButtonsJson(toolbarItemsJson)
            }
            Prop("toolbarFrameJson") { (view: NativeEditorExpoView, toolbarFrameJson: String?) in
                view.setToolbarFrameJson(toolbarFrameJson)
            }
            Prop("editorUpdateJson") { (view: NativeEditorExpoView, editorUpdateJson: String?) in
                view.setPendingEditorUpdateJson(editorUpdateJson)
            }
            Prop("editorUpdateRevision") { (view: NativeEditorExpoView, editorUpdateRevision: Int) in
                view.setPendingEditorUpdateRevision(editorUpdateRevision)
            }
            OnViewDidUpdateProps { (view: NativeEditorExpoView) in
                view.applyPendingEditorUpdateIfNeeded()
            }

            AsyncFunction("applyEditorUpdate") { (view: NativeEditorExpoView, updateJson: String) in
                view.applyEditorUpdate(updateJson)
            }
            AsyncFunction("focus") { (view: NativeEditorExpoView) in
                view.focus()
            }
            AsyncFunction("blur") { (view: NativeEditorExpoView) in
                view.blur()
            }
        }
    }
}
