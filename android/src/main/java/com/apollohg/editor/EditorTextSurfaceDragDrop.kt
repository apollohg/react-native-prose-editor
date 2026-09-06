package com.apollohg.editor

import android.view.DragEvent

internal class EditorTextSurfaceDragDrop(private val surface: EditorTextSurface) {
    private data class Session(val editorId: Long, val driver: EditorV2Driver, var dropped: Boolean = false)
    private var session: Session? = null

    fun dispose() { session = null }

    fun onDragEvent(event: DragEvent): Boolean {
        val editor = surface as? EditorEditText ?: return false
        if (event.action == DragEvent.ACTION_DRAG_STARTED) {
            session = null
            if (event.localState === editor || event.clipDescription?.hasMimeType("text/*") != true) return false
            val driver = editor.v2Driver ?: return false
            val candidate = Session(editor.editorId, driver)
            if (!isCurrent(editor, candidate)) return false
            session = candidate
            return true
        }
        val active = session ?: return false
        if (event.action == DragEvent.ACTION_DRAG_ENDED) {
            session = null
            return true
        }
        if (event.localState === editor || !isCurrent(editor, active)) {
            session = null
            return false
        }
        return when (event.action) {
            DragEvent.ACTION_DRAG_ENTERED -> { editor.requestFocus(); true }
            DragEvent.ACTION_DRAG_LOCATION -> {
                if (event.x.isFinite() && event.y.isFinite()) {
                    editor.bringPointIntoView(editor.getOffsetForPosition(event.x, event.y))
                }
                true
            }
            DragEvent.ACTION_DRAG_EXITED -> true
            DragEvent.ACTION_DROP -> drop(editor, active, event)
            else -> false
        }
    }

    private fun isCurrent(editor: EditorEditText, active: Session): Boolean =
        editor.isEnabled && editor.isEditable && editor.hasLiveEditor() &&
            editor.editorId == active.editorId && editor.v2Driver === active.driver &&
            (active.driver !is EditorV2Adapter || editor.ownsNativeBinding(active.driver))

    private fun drop(editor: EditorEditText, active: Session, event: DragEvent): Boolean {
        if (active.dropped || !event.x.isFinite() || !event.y.isFinite()) return false
        active.dropped = true
        val clip = event.clipData ?: return false
        val value = try {
            (0 until clip.itemCount).joinToString("\n") { clip.getItemAt(it).coerceToText(editor.context)?.toString().orEmpty() }
        } catch (_: SecurityException) { return false }
        if (value.isEmpty()) return false
        if (!editor.prepareForExternalInteractionMutation() || session !== active || !isCurrent(editor, active)) return false
        editor.requestFocus()
        if (session !== active || !isCurrent(editor, active)) return false
        val offset = editor.getOffsetForPosition(event.x, event.y)
        if (editor.isCollapsedAtomBoundarySelection(offset, offset)) return false
        val scalar = PositionBridge.utf16ToScalar(offset, editor.text.toString())
        val update = active.driver.replaceTextRange(scalar, scalar, value)
        if (session !== active || !isCurrent(editor, active)) return false
        editor.applyNonOptimisticRustUpdate(active.driver, update)
        return update != null
    }
}
