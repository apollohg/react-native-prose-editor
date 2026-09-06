package com.apollohg.editor.surface

import android.content.Intent
import android.graphics.PointF
import android.os.SystemClock
import android.view.MotionEvent
import android.view.ViewConfiguration
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class EditorSurfaceSelectionTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Test
    fun longPressAndDragUsesProductionBlockCoordinates() {
        ActivityScenario.launch<EditorSurfaceActivity>(Intent(instrumentation.targetContext, EditorSurfaceActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)).use { scenario ->
            instrumentation.waitForIdleSync()
            var start = PointF()
            var end = PointF()
            scenario.onActivity { activity ->
                activity.editor.showSoftInputOnFocus = false
                activity.editor.requestFocus()
                activity.editor.setSelection(0)
                val location = IntArray(2).also(activity.editor::getLocationOnScreen)
                fun point(offset: Int): PointF {
                    val layout = activity.editor.layout
                    val line = layout.getLineForOffset(offset)
                    return PointF(location[0] + activity.editor.totalPaddingLeft + layout.getPrimaryHorizontal(offset),
                        location[1] + activity.editor.totalPaddingTop + (layout.getLineBaseline(line) + layout.getLineAscent(line) / 2f))
                }
                start = point(6)
                end = point(60)
            }
            val down = SystemClock.uptimeMillis()
            fun send(action: Int, point: PointF) {
                val event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, point.x, point.y, 0)
                assertTrue(instrumentation.uiAutomation.injectInputEvent(event, true))
                event.recycle()
            }
            send(MotionEvent.ACTION_DOWN, start)
            SystemClock.sleep(ViewConfiguration.getLongPressTimeout().toLong() + 150)
            send(MotionEvent.ACTION_MOVE, end)
            send(MotionEvent.ACTION_UP, end)
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                assertEquals(5, activity.editor.selectionStart)
                assertEquals(60, activity.editor.selectionEnd)
                assertEquals(0, activity.editor.scrollY)
            }
        }
    }
}
