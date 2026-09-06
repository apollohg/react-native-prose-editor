package com.apollohg.editor

import android.app.Activity
import android.content.Context
import android.view.View
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.widget.FrameLayout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class EditorSurfaceAccessibilityTest {
    private class EventParent(context: Context) : FrameLayout(context) {
        val events = mutableListOf<AccessibilityEvent>()

        override fun onRequestSendAccessibilityEvent(child: View, event: AccessibilityEvent): Boolean {
            events += AccessibilityEvent.obtain(event)
            return super.onRequestSendAccessibilityEvent(child, event)
        }
    }

    @Test
    fun `full production render publishes exact text replacement event`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        shadowOf(activity.getSystemService(AccessibilityManager::class.java)).setEnabled(true)
        val parent = EventParent(activity)
        val editor = EditorEditText(activity)
        parent.addView(editor)
        activity.setContentView(parent)
        editor.applyRenderJSON(render("before"))
        editor.setSelection(0)
        editor.requestFocus()
        val before = editor.text.toString()
        parent.events.clear()

        editor.applyRenderJSON(render("replacement"))

        val events = parent.events.filter { it.eventType == AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED }
        assertEquals(1, events.size)
        val event = events.single()
        assertEquals(before, event.beforeText.toString())
        assertEquals(0, event.fromIndex)
        assertEquals(before.length, event.removedCount)
        assertEquals(editor.text.length, event.addedCount)
        assertEquals(listOf(editor.text.toString()), event.text.map { it.toString() })
        assertEquals("android.widget.EditText", event.className.toString())
    }

    @Test
    fun `full render with identical characters does not announce a text change`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        shadowOf(activity.getSystemService(AccessibilityManager::class.java)).setEnabled(true)
        val parent = EventParent(activity)
        val editor = EditorEditText(activity)
        parent.addView(editor)
        activity.setContentView(parent)
        editor.applyRenderJSON(render("unchanged"))
        editor.requestFocus()
        parent.events.clear()

        editor.applyRenderJSON(render("unchanged"))

        assertTrue(parent.events.none { it.eventType == AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED })
    }

    private fun render(text: String) = """
        [{"type":"blockStart","nodeType":"paragraph","depth":0},
        {"type":"textRun","text":"$text","marks":[]},
        {"type":"blockEnd"}]
    """.trimIndent()
}
