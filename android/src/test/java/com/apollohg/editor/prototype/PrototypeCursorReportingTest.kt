package com.apollohg.editor.prototype

import android.app.Activity
import android.content.Context
import android.view.View
import android.view.inputmethod.CursorAnchorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements
import org.robolectric.shadow.api.Shadow
import org.robolectric.shadows.ShadowInputMethodManager

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], shadows = [PrototypeCursorReportingTest.RecordingInputMethodManager::class])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PrototypeCursorReportingTest {
    @Implements(InputMethodManager::class)
    class RecordingInputMethodManager : ShadowInputMethodManager() {
        val updates = mutableListOf<CursorAnchorInfo>()
        @Implementation
        fun updateCursorAnchorInfo(view: View, info: CursorAnchorInfo) { updates += info }
    }

    @Test
    fun `scrolling preserves monitoring and subsequent edits report new caret geometry`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        PrototypeDocumentSession(listOf("First", "Second")).use { session ->
            val editor = PrototypeEditorView(activity, session)
            activity.setContentView(editor)
            editor.measure(View.MeasureSpec.makeMeasureSpec(380, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(1000, View.MeasureSpec.AT_MOST))
            editor.layout(0, 0, editor.measuredWidth, editor.measuredHeight)
            val manager = activity.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
            val recorder = Shadow.extract<RecordingInputMethodManager>(manager)
            editor.requestCursorUpdates(InputConnection.CURSOR_UPDATE_MONITOR)
            editor.onViewportChanged()
            val afterScroll = recorder.updates.size
            assertTrue(afterScroll > 0)
            session.setSelection(8, 8)
            assertTrue(recorder.updates.size > afterScroll)
            val report = recorder.updates.last()
            val caret = editor.documentLayout.caret(8)
            assertEquals(8, report.selectionEnd)
            assertEquals(caret.left, report.insertionMarkerHorizontal, 0.01f)
            assertEquals(caret.top, report.insertionMarkerTop, 0.01f)
            editor.requestCursorUpdates(0)
            val stopped = recorder.updates.size
            session.setSelection(9, 9)
            editor.onViewportChanged()
            assertEquals(stopped, recorder.updates.size)
        }
    }
}
