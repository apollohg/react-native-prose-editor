package com.apollohg.editor.prototype

import android.accessibilityservice.AccessibilityServiceInfo
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Rect
import android.os.SystemClock
import android.provider.Settings
import android.view.MotionEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

@RunWith(AndroidJUnit4::class)
class PrototypeGboardSmokeTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Test
    fun realKeyboardTapComposesAndCommitsThroughRust() {
        val keyboard = Settings.Secure.getString(instrumentation.targetContext.contentResolver, Settings.Secure.DEFAULT_INPUT_METHOD)
        assumeTrue("Gboard-specific smoke fixture", keyboard?.startsWith("com.google.android.inputmethod.latin/") == true)
        val automation = instrumentation.uiAutomation
        val originalFlags = automation.serviceInfo.flags
        automation.serviceInfo = automation.serviceInfo.apply { flags = flags or AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS }
        try {
            ActivityScenario.launch<PrototypeEditorActivity>(Intent(instrumentation.targetContext, PrototypeEditorActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)).use { scenario ->
                instrumentation.waitForIdleSync()
                val point = FloatArray(2)
                scenario.onActivity { activity ->
                    val location = IntArray(2).also(activity.editor::getLocationOnScreen)
                    val caret = activity.editor.documentLayout.caret(0)
                    point[0] = location[0] + caret.left + 1
                    point[1] = location[1] + caret.centerY()
                }
                tap(point[0], point[1])
                tapKeyboardKey(setOf("a"))
                tapKeyboardKey(setOf("space", "space bar"))
                var committed = false
                val deadline = SystemClock.uptimeMillis() + 4000
                while (!committed && SystemClock.uptimeMillis() < deadline) {
                    scenario.onActivity { activity -> committed = activity.session.committedText.startsWith("a ", ignoreCase = true) }
                    if (!committed) SystemClock.sleep(50)
                }
                automation.waitForIdle(300, 5000)
                val bitmap = requireNotNull(automation.takeScreenshot())
                File(instrumentation.targetContext.getExternalFilesDir(null), "android-layout-prototype-keyboard.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                bitmap.recycle()
                scenario.onActivity { activity ->
                    assertTrue("Committed=${activity.session.committedText.take(30)} displayed=${activity.session.editable.take(30)} selection=${activity.session.selectionStart}..${activity.session.selectionEnd}", committed)
                    assertEquals(activity.session.committedText, activity.session.editable.toString())
                }
            }
        } finally {
            automation.serviceInfo = automation.serviceInfo.apply { flags = originalFlags }
        }
    }

    private fun tapKeyboardKey(labels: Set<String>) {
        instrumentation.uiAutomation.waitForIdle(300, 5000)
        val deadline = SystemClock.uptimeMillis() + 8000
        var observed = emptyList<String>()
        while (SystemClock.uptimeMillis() < deadline) {
            val root = instrumentation.uiAutomation.windows.firstOrNull { it.type == AccessibilityWindowInfo.TYPE_INPUT_METHOD }?.root
            if (root != null) {
                val nodes = mutableListOf<AccessibilityNodeInfo>()
                fun visit(node: AccessibilityNodeInfo) {
                    nodes += node
                    for (index in 0 until node.childCount) node.getChild(index)?.let(::visit)
                }
                visit(root)
                observed = nodes.mapNotNull { (it.contentDescription ?: it.text)?.toString() }
                val key = nodes.firstOrNull { node ->
                    val label = (node.contentDescription ?: node.text)?.toString()?.lowercase()
                    node.isVisibleToUser && label in labels
                }
                if (key != null) {
                    val bounds = Rect().also(key::getBoundsInScreen)
                    android.util.Log.i("PrototypeGboard", "Tap $labels at $bounds")
                    tap(bounds.exactCenterX(), bounds.exactCenterY())
                    return
                }
            }
            SystemClock.sleep(100)
        }
        fail("Could not find Gboard key $labels; exposed keys: $observed")
    }

    private fun tap(x: Float, y: Float) {
        val down = SystemClock.uptimeMillis()
        for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
            val event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, x, y, 0)
            assertTrue(instrumentation.uiAutomation.injectInputEvent(event, true))
            event.recycle()
        }
        instrumentation.waitForIdleSync()
    }
}
