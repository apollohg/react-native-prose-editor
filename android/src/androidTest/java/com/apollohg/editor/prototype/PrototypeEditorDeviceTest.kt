package com.apollohg.editor.prototype

import android.content.Intent
import android.graphics.Bitmap
import android.os.SystemClock
import android.view.MotionEvent
import android.view.ViewConfiguration
import android.widget.LinearLayout
import android.view.KeyEvent
import android.view.inputmethod.EditorInfo
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

@RunWith(AndroidJUnit4::class)
class PrototypeEditorDeviceTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    private fun launch() = ActivityScenario.launch<PrototypeEditorActivity>(
        Intent(instrumentation.targetContext, PrototypeEditorActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
    )

    @Test
    fun compositionAndCrossBlockReplacementUseTheDisplayedLayoutAndRealCore() {
        launch().use { scenario ->
            scenario.onActivity { activity ->
                activity.editor.requestFocus()
                val separator = activity.session.committedText.indexOf('\n')
                activity.session.setSelection(separator - 4, separator + 5)
            }
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                val session = activity.session
                val editor = activity.editor
                val connection = editor.activeConnection()
                val before = session.committedText
                val separator = before.indexOf('\n')
                assertFalse(editor.documentLayout.selection(session.selectionStart, session.selectionEnd).isEmpty)
                assertTrue(connection.setComposingText("にほん", 1))
                assertEquals(before, session.committedText)
                assertTrue(connection.setComposingText("日本語", 1))
                val caret = editor.documentLayout.caret(session.selectionEnd)
                assertEquals(session.selectionEnd, editor.documentLayout.offsetAt(caret.left, caret.centerY()))
                assertTrue(connection.commitText("日本語", 1))
                assertEquals(before.take(separator - 4) + "日本語" + before.drop(separator + 5), session.committedText)
                assertEquals(session.committedText, session.editable.toString())
                assertTrue(connection.commitText("\n", 1))
                assertTrue(session.committedText.contains('\n'))
            }
        }
    }

    @Test
    fun nativeChildAndTextShareScrollCoordinatesAndResizeTogether() {
        launch().use { scenario ->
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                activity.scroller.layoutParams = (activity.scroller.layoutParams as LinearLayout.LayoutParams).apply {
                    weight = 0f
                    height = activity.editor.height / 2
                }
            }
            instrumentation.waitForIdleSync()
            var contentTop = 0
            var oldWindowTop = 0
            var scrollDelta = 0
            var oldAtomHeight = 0
            var oldSecondTop = 0f
            scenario.onActivity { activity ->
                assertSame(activity.editor, activity.atom.parent)
                contentTop = activity.atom.top
                oldWindowTop = IntArray(2).also(activity.atom::getLocationOnScreen)[1]
                val oldScroll = activity.scroller.scrollY
                activity.scroller.scrollTo(0, 80)
                scrollDelta = activity.scroller.scrollY - oldScroll
                assertTrue(scrollDelta > 0)
            }
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                assertEquals(contentTop, activity.atom.top)
                val windowTop = IntArray(2).also(activity.atom::getLocationOnScreen)[1]
                assertEquals(oldWindowTop - scrollDelta, windowTop)
                val separator = activity.session.editable.indexOf('\n')
                val secondCaret = activity.editor.documentLayout.caret(separator + 1)
                oldSecondTop = secondCaret.top
                oldAtomHeight = activity.atom.height
                assertTrue(secondCaret.top >= activity.atom.bottom)
                activity.atom.performClick()
                assertTrue(activity.atom.text.toString().endsWith("1"))
                activity.resizeAtom(activity.atom.height + 120)
            }
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                val separator = activity.session.editable.indexOf('\n')
                assertEquals(oldAtomHeight + 120, activity.atom.height)
                assertEquals(oldSecondTop + 120, activity.editor.documentLayout.caret(separator + 1).top, 0.01f)
                assertTrue(activity.editor.documentLayout.caret(separator + 1).top >= activity.atom.bottom)
            }
        }
    }

    @Test
    fun longPressAndDragSelectAcrossTheNativeChild() {
        launch().use { scenario ->
            val points = FloatArray(4)
            var expectedEnd = 0
            scenario.onActivity { activity ->
                val separator = activity.session.editable.indexOf('\n')
                expectedEnd = separator + 8
                val start = activity.editor.documentLayout.caret(3)
                val end = activity.editor.documentLayout.caret(expectedEnd)
                val location = IntArray(2).also(activity.editor::getLocationOnScreen)
                points[0] = location[0] + start.left
                points[1] = location[1] + start.centerY()
                points[2] = location[0] + end.left
                points[3] = location[1] + end.centerY()
            }
            val down = SystemClock.uptimeMillis()
            fun send(action: Int, x: Float, y: Float) {
                val event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, x, y, 0)
                instrumentation.sendPointerSync(event)
                event.recycle()
            }
            send(MotionEvent.ACTION_DOWN, points[0], points[1])
            SystemClock.sleep(ViewConfiguration.getLongPressTimeout().toLong() + 100)
            send(MotionEvent.ACTION_MOVE, points[2], points[3])
            send(MotionEvent.ACTION_UP, points[2], points[3])
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                assertEquals(3, activity.session.selectionStart)
                assertEquals(expectedEnd, activity.session.selectionEnd)
            }
        }
    }

    @Test
    fun resizingAtomAboveViewportPreservesVisibleTextAnchor() {
        launch().use { scenario ->
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                activity.scroller.layoutParams = (activity.scroller.layoutParams as LinearLayout.LayoutParams).apply {
                    weight = 0f
                    height = activity.editor.height / 3
                }
            }
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                val second = activity.session.editable.indexOf('\n') + 1
                activity.scroller.scrollTo(0, activity.editor.documentLayout.caret(second).top.toInt() + 50)
            }
            instrumentation.waitForIdleSync()
            var anchor = 0
            var screenY = 0f
            var oldHeight = 0
            scenario.onActivity { activity ->
                assertTrue(activity.scroller.scrollY >= activity.atom.bottom)
                anchor = activity.editor.documentLayout.offsetAt(0f, activity.scroller.scrollY.toFloat())
                screenY = activity.editor.documentLayout.caret(anchor).top - activity.scroller.scrollY
                oldHeight = activity.atom.height
                activity.resizeAtom(oldHeight + 120)
            }
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                assertEquals(oldHeight + 120, activity.atom.height)
                assertEquals(screenY, activity.editor.documentLayout.caret(anchor).top - activity.scroller.scrollY, 1f)
            }
        }
    }

    @Test
    fun hardwareInputAndScreenshotExerciseTheMountedCustomView() {
        launch().use { scenario ->
            scenario.onActivity { activity ->
                activity.editor.requestFocus()
                activity.session.setSelection(0, 0)
            }
            instrumentation.sendKeyDownUpSync(KeyEvent.KEYCODE_A)
            instrumentation.waitForIdleSync()
            scenario.onActivity { activity ->
                assertTrue(activity.session.committedText.startsWith("aThis"))
                assertEquals(activity.session.committedText, activity.session.editable.toString())
            }
            val image = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
            val file = File(instrumentation.targetContext.getExternalFilesDir(null), "android-layout-prototype.png")
            file.outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
            image.recycle()
        }
    }
}
