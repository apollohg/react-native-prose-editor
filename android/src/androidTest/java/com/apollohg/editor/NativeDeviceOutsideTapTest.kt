package com.apollohg.editor

import android.app.Activity
import android.app.Instrumentation
import android.content.Context
import android.graphics.Color
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import expo.modules.core.ModuleRegistry
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.ModulesProvider
import expo.modules.kotlin.modules.Module
import java.lang.ref.WeakReference
import java.util.Collections
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
@LargeTest
class NativeDeviceOutsideTapTest {
    private val instrumentation: Instrumentation = InstrumentationRegistry.getInstrumentation()

    @Test
    fun tappingOutsideFocusedEditorBlursEditor() {
        ActivityScenario.launch(NativeEditorOutsideTapActivity::class.java).use { scenario ->
            val editorRef = AtomicReference<NativeEditorExpoView>()
            val outsideTargetRef = AtomicReference<View>()
            val outsideTapTrace = Collections.synchronizedList(mutableListOf<String>())
            val editorId = createV2EditorId()

            try {
                scenario.onActivity { activity ->
                    val root = FrameLayout(activity).apply {
                        setBackgroundColor(Color.WHITE)
                        isFocusable = true
                        isFocusableInTouchMode = true
                    }
                    val outsideTarget = View(activity).apply {
                        setBackgroundColor(Color.rgb(238, 238, 238))
                    }
                    val expoContext = testExpoContext(activity)
                    val editor = NativeEditorExpoView(expoContext.context, expoContext.appContext).apply {
                        clipToPadding = false
                        setShowToolbar(false)
                    }

                    editor.onFocusChangeForTesting = {}
                    editor.onAddonEventForTesting = {}
                    editor.onEditorUpdateForTesting = {}
                    editor.onEditorReadyForTesting = {}
                    editor.onOutsideTapTraceForTesting = { event ->
                        outsideTapTrace.add(event)
                    }
                    root.addView(
                        editor,
                        FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 220)
                        ).apply {
                            topMargin = dp(activity, 48)
                            leftMargin = dp(activity, 16)
                            rightMargin = dp(activity, 16)
                        }
                    )
                    root.addView(
                        outsideTarget,
                        FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 180)
                        ).apply {
                            topMargin = dp(activity, 320)
                            leftMargin = dp(activity, 16)
                            rightMargin = dp(activity, 16)
                        }
                    )
                    activity.setContentView(root)

                    editor.setEditorId(editorId)
                    editor.richTextView.editorEditText.applyUpdateJSON(
                        renderUpdateJson("Tap outside should blur"),
                        notifyListener = false
                    )
                    editor.richTextView.editorEditText.setSelection(0)

                    outsideTargetRef.set(outsideTarget)
                    editorRef.set(editor)
                }

                instrumentation.waitForIdleSync()

                scenario.onActivity { activity ->
                    editorRef.get().focus()
                    activity.window.decorView.post {
                        editorRef.get().installOutsideTapBlurHandlerForTesting()
                    }
                }
                instrumentation.waitForIdleSync()
                waitUntil("editor should gain focus") {
                    editorRef.get().richTextView.editorEditText.hasFocus()
                }

                tapCenterOnScreen(outsideTargetRef.get())
                instrumentation.waitForIdleSync()

                waitUntil(
                    "editor should lose focus after outside tap",
                    detail = { outsideTapTrace.joinToString(separator = "\n") }
                ) {
                    !editorRef.get().richTextView.editorEditText.hasFocus()
                }

                assertTrue(
                    outsideTapTrace.joinToString(separator = "\n"),
                    outsideTapTrace.any { it.contains("OUTSIDE_EDITOR") }
                )
                assertFalse(editorRef.get().richTextView.editorEditText.hasFocus())
            } finally {
                scenario.onActivity {
                    editorRef.get()?.uninstallOutsideTapBlurHandlerForTesting()
                }
                EditorV2Registry.destroyPair(editorId)
            }
        }
    }

    @Test
    fun tappingInlineToolbarFramePreservesEditorFocus() {
        ActivityScenario.launch(NativeEditorOutsideTapActivity::class.java).use { scenario ->
            val editorRef = AtomicReference<NativeEditorExpoView>()
            val toolbarTargetRef = AtomicReference<View>()
            val outsideTapTrace = Collections.synchronizedList(mutableListOf<String>())
            val editorId = createV2EditorId()

            try {
                scenario.onActivity { activity ->
                    val root = FrameLayout(activity).apply {
                        setBackgroundColor(Color.WHITE)
                        isFocusable = true
                        isFocusableInTouchMode = true
                    }
                    val toolbarTarget = View(activity).apply {
                        setBackgroundColor(Color.rgb(238, 238, 238))
                    }
                    val expoContext = testExpoContext(activity)
                    val editor = NativeEditorExpoView(expoContext.context, expoContext.appContext).apply {
                        clipToPadding = false
                        setShowToolbar(false)
                    }

                    editor.onFocusChangeForTesting = {}
                    editor.onAddonEventForTesting = {}
                    editor.onEditorUpdateForTesting = {}
                    editor.onEditorReadyForTesting = {}
                    editor.onOutsideTapTraceForTesting = { event ->
                        outsideTapTrace.add(event)
                    }
                    root.addView(
                        editor,
                        FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 220)
                        ).apply {
                            topMargin = dp(activity, 48)
                            leftMargin = dp(activity, 16)
                            rightMargin = dp(activity, 16)
                        }
                    )
                    root.addView(
                        toolbarTarget,
                        FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 52)
                        ).apply {
                            topMargin = dp(activity, 320)
                            leftMargin = dp(activity, 16)
                            rightMargin = dp(activity, 16)
                        }
                    )
                    activity.setContentView(root)

                    editor.setEditorId(editorId)
                    editor.richTextView.editorEditText.applyUpdateJSON(
                        renderUpdateJson("Toolbar tap should preserve focus"),
                        notifyListener = false
                    )
                    editor.richTextView.editorEditText.setSelection(0)

                    toolbarTargetRef.set(toolbarTarget)
                    editorRef.set(editor)
                }

                instrumentation.waitForIdleSync()

                scenario.onActivity { activity ->
                    applyToolbarFrameFromView(activity, editorRef.get(), toolbarTargetRef.get())
                    editorRef.get().focus()
                    activity.window.decorView.post {
                        editorRef.get().installOutsideTapBlurHandlerForTesting()
                    }
                }
                instrumentation.waitForIdleSync()
                waitUntil("editor should gain focus") {
                    editorRef.get().richTextView.editorEditText.hasFocus()
                }

                scenario.onActivity { activity ->
                    recordTapGeometry(activity, "toolbar", toolbarTargetRef.get(), outsideTapTrace)
                }
                tapCenterOnScreen(toolbarTargetRef.get(), "toolbar", outsideTapTrace)
                SystemClock.sleep(400)
                instrumentation.waitForIdleSync()

                assertTrue(
                    outsideTapTrace.joinToString(separator = "\n"),
                    outsideTapTrace.any { it.contains("PRESERVE_FOCUS") }
                )
                assertTrue(
                    "editor should keep focus after toolbar tap\n" +
                        outsideTapTrace.joinToString(separator = "\n"),
                    editorRef.get().richTextView.editorEditText.hasFocus()
                )
            } finally {
                scenario.onActivity {
                    editorRef.get()?.uninstallOutsideTapBlurHandlerForTesting()
                }
                EditorV2Registry.destroyPair(editorId)
            }
        }
    }

    @Test
    fun swipingOutsideFocusedEditorInScrollViewKeepsEditorFocused() {
        ActivityScenario.launch(NativeEditorOutsideTapActivity::class.java).use { scenario ->
            val editorRef = AtomicReference<NativeEditorExpoView>()
            val scrollViewRef = AtomicReference<ScrollView>()
            val outsideTargetRef = AtomicReference<View>()
            val initialScrollY = AtomicInteger()
            val outsideTapTrace = Collections.synchronizedList(mutableListOf<String>())
            val editorId = createV2EditorId()

            try {
                scenario.onActivity { activity ->
                    val scrollView = ScrollView(activity).apply {
                        setBackgroundColor(Color.WHITE)
                        isFillViewport = true
                    }
                    val content = LinearLayout(activity).apply {
                        orientation = LinearLayout.VERTICAL
                        setBackgroundColor(Color.WHITE)
                        isFocusable = true
                        isFocusableInTouchMode = true
                    }
                    val expoContext = testExpoContext(activity)
                    val editor = NativeEditorExpoView(expoContext.context, expoContext.appContext).apply {
                        clipToPadding = false
                        setShowToolbar(false)
                    }
                    val outsideTarget = View(activity).apply {
                        setBackgroundColor(Color.rgb(238, 238, 238))
                    }

                    editor.onFocusChangeForTesting = {}
                    editor.onAddonEventForTesting = {}
                    editor.onEditorUpdateForTesting = {}
                    editor.onEditorReadyForTesting = {}
                    editor.onOutsideTapTraceForTesting = { event ->
                        outsideTapTrace.add(event)
                    }

                    scrollView.addView(
                        content,
                        FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            ViewGroup.LayoutParams.WRAP_CONTENT
                        )
                    )
                    content.addView(
                        View(activity),
                        LinearLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 48)
                        )
                    )
                    content.addView(
                        editor,
                        LinearLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 220)
                        ).apply {
                            leftMargin = dp(activity, 16)
                            rightMargin = dp(activity, 16)
                        }
                    )
                    content.addView(
                        outsideTarget,
                        LinearLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 720)
                        ).apply {
                            leftMargin = dp(activity, 16)
                            rightMargin = dp(activity, 16)
                        }
                    )
                    content.addView(
                        View(activity),
                        LinearLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 1200)
                        )
                    )
                    activity.setContentView(scrollView)

                    editor.setEditorId(editorId)
                    editor.richTextView.editorEditText.applyUpdateJSON(
                        renderUpdateJson("Swipe outside should scroll without blur"),
                        notifyListener = false
                    )
                    editor.richTextView.editorEditText.setSelection(0)

                    scrollViewRef.set(scrollView)
                    outsideTargetRef.set(outsideTarget)
                    editorRef.set(editor)
                }

                instrumentation.waitForIdleSync()

                scenario.onActivity { activity ->
                    editorRef.get().focus()
                    activity.window.decorView.post {
                        editorRef.get().installOutsideTapBlurHandlerForTesting()
                    }
                }
                instrumentation.waitForIdleSync()
                waitUntil("editor should gain focus") {
                    editorRef.get().richTextView.editorEditText.hasFocus()
                }
                waitUntil("outside target should be laid out") {
                    outsideTargetRef.get().isShown && outsideTargetRef.get().height > 0
                }

                scenario.onActivity {
                    initialScrollY.set(scrollViewRef.get().scrollY)
                    recordScrollGeometry(
                        "before swipe",
                        scrollViewRef.get(),
                        editorRef.get(),
                        outsideTargetRef.get(),
                        outsideTapTrace
                    )
                }
                swipeUpFromView(
                    outsideTargetRef.get(),
                    startOffsetDp = 120,
                    distanceDp = 180,
                    label = "outside scroll",
                    trace = outsideTapTrace
                )
                instrumentation.waitForIdleSync()

                waitUntil(
                    "scroll view should move after outside swipe",
                    detail = { outsideTapTrace.joinToString(separator = "\n") }
                ) {
                    scrollViewRef.get().scrollY > initialScrollY.get()
                }

                SystemClock.sleep(700)
                instrumentation.waitForIdleSync()
                scenario.onActivity {
                    recordScrollGeometry(
                        "after swipe",
                        scrollViewRef.get(),
                        editorRef.get(),
                        outsideTargetRef.get(),
                        outsideTapTrace
                    )
                }

                assertFalse(
                    outsideTapTrace.joinToString(separator = "\n"),
                    outsideTapTrace.any { it.contains("complete blur focusedAfter=false") }
                )
                assertTrue(
                    "editor should keep focus after scrolling outside it\n" +
                        outsideTapTrace.joinToString(separator = "\n"),
                    editorRef.get().richTextView.editorEditText.hasFocus()
                )
            } finally {
                scenario.onActivity {
                    editorRef.get()?.uninstallOutsideTapBlurHandlerForTesting()
                }
                EditorV2Registry.destroyPair(editorId)
            }
        }
    }

    @Test
    fun swipingChatListBehindStickyComposerKeepsEditorFocused() {
        ActivityScenario.launch(NativeEditorOutsideTapActivity::class.java).use { scenario ->
            val editorRef = AtomicReference<NativeEditorExpoView>()
            val scrollViewRef = AtomicReference<ScrollView>()
            val outsideTargetRef = AtomicReference<View>()
            val initialScrollY = AtomicInteger()
            val outsideTapTrace = Collections.synchronizedList(mutableListOf<String>())
            val editorId = createV2EditorId()

            try {
                scenario.onActivity { activity ->
                    val root = FrameLayout(activity).apply {
                        setBackgroundColor(Color.WHITE)
                    }
                    val scrollView = ScrollView(activity).apply {
                        setBackgroundColor(Color.WHITE)
                        isFillViewport = true
                    }
                    val content = LinearLayout(activity).apply {
                        orientation = LinearLayout.VERTICAL
                        setBackgroundColor(Color.WHITE)
                        isFocusable = true
                        isFocusableInTouchMode = true
                    }
                    val outsideTarget = View(activity).apply {
                        setBackgroundColor(Color.rgb(238, 238, 238))
                    }
                    val expoContext = testExpoContext(activity)
                    val editor = NativeEditorExpoView(expoContext.context, expoContext.appContext).apply {
                        clipToPadding = false
                        setShowToolbar(false)
                        setBackgroundColor(Color.WHITE)
                    }

                    editor.onFocusChangeForTesting = {}
                    editor.onAddonEventForTesting = {}
                    editor.onEditorUpdateForTesting = {}
                    editor.onEditorReadyForTesting = {}
                    editor.onOutsideTapTraceForTesting = { event ->
                        outsideTapTrace.add(event)
                    }

                    scrollView.addView(
                        content,
                        FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            ViewGroup.LayoutParams.WRAP_CONTENT
                        )
                    )
                    content.addView(
                        View(activity),
                        LinearLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 24)
                        )
                    )
                    content.addView(
                        outsideTarget,
                        LinearLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 680)
                        ).apply {
                            leftMargin = dp(activity, 16)
                            rightMargin = dp(activity, 16)
                        }
                    )
                    content.addView(
                        View(activity),
                        LinearLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 1600)
                        )
                    )
                    root.addView(
                        scrollView,
                        FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            ViewGroup.LayoutParams.MATCH_PARENT
                        )
                    )
                    root.addView(
                        editor,
                        FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            dp(activity, 180)
                        ).apply {
                            gravity = android.view.Gravity.BOTTOM
                            leftMargin = dp(activity, 16)
                            rightMargin = dp(activity, 16)
                            bottomMargin = dp(activity, 16)
                        }
                    )
                    activity.setContentView(root)

                    editor.setEditorId(editorId)
                    editor.richTextView.editorEditText.applyUpdateJSON(
                        renderUpdateJson("Sticky composer should stay focused while chat scrolls"),
                        notifyListener = false
                    )
                    editor.richTextView.editorEditText.setSelection(0)

                    scrollViewRef.set(scrollView)
                    outsideTargetRef.set(outsideTarget)
                    editorRef.set(editor)
                }

                instrumentation.waitForIdleSync()

                scenario.onActivity { activity ->
                    editorRef.get().focus()
                    activity.window.decorView.post {
                        editorRef.get().installOutsideTapBlurHandlerForTesting()
                    }
                }
                instrumentation.waitForIdleSync()
                waitUntil("editor should gain focus") {
                    editorRef.get().richTextView.editorEditText.hasFocus()
                }
                waitUntil("outside target should be laid out") {
                    outsideTargetRef.get().isShown && outsideTargetRef.get().height > 0
                }

                scenario.onActivity {
                    initialScrollY.set(scrollViewRef.get().scrollY)
                    recordScrollGeometry(
                        "sticky before swipe",
                        scrollViewRef.get(),
                        editorRef.get(),
                        outsideTargetRef.get(),
                        outsideTapTrace
                    )
                }
                swipeUpFromView(
                    outsideTargetRef.get(),
                    startOffsetDp = 260,
                    distanceDp = 220,
                    label = "chat list scroll",
                    trace = outsideTapTrace
                )
                instrumentation.waitForIdleSync()

                waitUntil(
                    "chat list should scroll behind sticky composer",
                    detail = { outsideTapTrace.joinToString(separator = "\n") }
                ) {
                    scrollViewRef.get().scrollY > initialScrollY.get()
                }

                SystemClock.sleep(700)
                instrumentation.waitForIdleSync()
                scenario.onActivity {
                    recordScrollGeometry(
                        "sticky after swipe",
                        scrollViewRef.get(),
                        editorRef.get(),
                        outsideTargetRef.get(),
                        outsideTapTrace
                    )
                }

                assertFalse(
                    outsideTapTrace.joinToString(separator = "\n"),
                    outsideTapTrace.any { it.contains("complete blur focusedAfter=false") }
                )
                assertTrue(
                    "sticky editor should keep focus after chat list scroll\n" +
                        outsideTapTrace.joinToString(separator = "\n"),
                    editorRef.get().richTextView.editorEditText.hasFocus()
                )
            } finally {
                scenario.onActivity {
                    editorRef.get()?.uninstallOutsideTapBlurHandlerForTesting()
                }
                EditorV2Registry.destroyPair(editorId)
            }
        }
    }

    private fun createV2EditorId(): Long = createPairedV2TestEditor().second

    private fun tapCenterOnScreen(
        view: View,
        label: String? = null,
        trace: MutableList<String>? = null
    ) {
        val location = IntArray(2)
        view.getLocationOnScreen(location)
        val x = location[0] + view.width / 2f
        val y = location[1] + view.height / 2f
        if (label != null && trace != null) {
            trace.add(
                "$label tap center=${x.toInt()},${y.toInt()} " +
                    "loc=${location[0]},${location[1]} size=${view.width}x${view.height} shown=${view.isShown}"
            )
        }
        val downTime = SystemClock.uptimeMillis()
        val down = MotionEvent.obtain(downTime, downTime, MotionEvent.ACTION_DOWN, x, y, 0)
        val up = MotionEvent.obtain(downTime, SystemClock.uptimeMillis(), MotionEvent.ACTION_UP, x, y, 0)
        instrumentation.sendPointerSync(down)
        instrumentation.sendPointerSync(up)
        down.recycle()
        up.recycle()
    }

    private fun swipeUpFromView(
        view: View,
        startOffsetDp: Int,
        distanceDp: Int,
        label: String,
        trace: MutableList<String>
    ) {
        val location = IntArray(2)
        view.getLocationOnScreen(location)
        val safeHeight = view.height.coerceAtLeast(2)
        val startOffsetPx = dp(view.context, startOffsetDp).coerceIn(1, safeHeight - 1)
        val distancePx = dp(view.context, distanceDp)
        val x = location[0] + view.width / 2f
        val startY = location[1] + startOffsetPx.toFloat()
        val endY = startY - distancePx
        trace.add(
            "$label swipe start=${x.toInt()},${startY.toInt()} end=${x.toInt()},${endY.toInt()} " +
                "loc=${location[0]},${location[1]} size=${view.width}x${view.height} shown=${view.isShown}"
        )

        val downTime = SystemClock.uptimeMillis()
        var eventTime = downTime
        val down = MotionEvent.obtain(downTime, eventTime, MotionEvent.ACTION_DOWN, x, startY, 0)
        instrumentation.sendPointerSync(down)
        down.recycle()

        val steps = 6
        for (step in 1..steps) {
            SystemClock.sleep(16)
            eventTime = SystemClock.uptimeMillis()
            val fraction = step.toFloat() / steps
            val y = startY + (endY - startY) * fraction
            val move = MotionEvent.obtain(downTime, eventTime, MotionEvent.ACTION_MOVE, x, y, 0)
            instrumentation.sendPointerSync(move)
            move.recycle()
        }

        SystemClock.sleep(16)
        eventTime = SystemClock.uptimeMillis()
        val up = MotionEvent.obtain(downTime, eventTime, MotionEvent.ACTION_UP, x, endY, 0)
        instrumentation.sendPointerSync(up)
        up.recycle()
    }

    private fun waitUntil(
        description: String,
        timeoutMs: Long = 3_000L,
        detail: () -> String = { "" },
        predicate: () -> Boolean
    ) {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            instrumentation.waitForIdleSync()
            if (predicate()) return
            SystemClock.sleep(50)
        }
        val detailText = detail()
        assertTrue(
            if (detailText.isBlank()) description else "$description\n$detailText",
            predicate()
        )
    }

    private fun applyToolbarFrameFromView(
        activity: Activity,
        editor: NativeEditorExpoView,
        view: View
    ) {
        val location = IntArray(2)
        view.getLocationOnScreen(location)
        val visibleWindowFrame = android.graphics.Rect()
        view.getWindowVisibleDisplayFrame(visibleWindowFrame)
        val density = activity.resources.displayMetrics.density
        editor.setToolbarFrameJson(
            JSONObject()
                .put("x", (location[0] - visibleWindowFrame.left) / density)
                .put("y", (location[1] - visibleWindowFrame.top) / density)
                .put("width", view.width / density)
                .put("height", view.height / density)
                .toString()
        )
    }

    private fun recordTapGeometry(
        activity: Activity,
        label: String,
        view: View,
        trace: MutableList<String>
    ) {
        val location = IntArray(2)
        view.getLocationOnScreen(location)
        val content = activity.findViewById<FrameLayout>(android.R.id.content)
        val lastChild = content.getChildAt(content.childCount - 1)
        trace.add(
            "$label geometry loc=${location[0]},${location[1]} size=${view.width}x${view.height} " +
                "shown=${view.isShown} contentChildren=${content.childCount} " +
                "lastChild=${lastChild?.javaClass?.name ?: "null"} lastShown=${lastChild?.isShown}"
        )
    }

    private fun recordScrollGeometry(
        label: String,
        scrollView: ScrollView,
        editor: NativeEditorExpoView,
        outsideTarget: View,
        trace: MutableList<String>
    ) {
        val editorLocation = IntArray(2)
        val targetLocation = IntArray(2)
        editor.richTextView.editorEditText.getLocationOnScreen(editorLocation)
        outsideTarget.getLocationOnScreen(targetLocation)
        trace.add(
            "$label scrollY=${scrollView.scrollY} " +
                "editorTop=${editorLocation[1]} targetTop=${targetLocation[1]} " +
                "focused=${editor.richTextView.editorEditText.hasFocus()}"
        )
    }

    private fun renderUpdateJson(text: String): String =
        JSONObject()
            .put(
                "renderBlocks",
                JSONArray().put(
                    JSONArray()
                        .put(
                            JSONObject()
                                .put("type", "blockStart")
                                .put("nodeType", "paragraph")
                                .put("depth", 0)
                        )
                        .put(
                            JSONObject()
                                .put("type", "textRun")
                                .put("text", text)
                                .put("marks", JSONArray())
                        )
                        .put(JSONObject().put("type", "blockEnd"))
                )
            )
            .toString()

    private fun dp(context: Context, value: Int): Int =
        (value * context.resources.displayMetrics.density).toInt()

    private data class TestExpoContext(
        val context: Context,
        val appContext: AppContext
    )

    private fun testExpoContext(activity: Activity): TestExpoContext {
        val reactContext = Class
            .forName("com.facebook.react.bridge.BridgeReactContext")
            .getConstructor(Context::class.java)
            .newInstance(activity) as Context

        reactContext.javaClass
            .getMethod("onHostResume", Activity::class.java)
            .invoke(reactContext, activity)

        val modulesProvider = object : ModulesProvider {
            override fun getModulesList(): List<Class<out Module>> = emptyList()
        }
        val constructor = AppContext::class.java.constructors.first { constructor ->
            constructor.parameterTypes.size == 3
        }
        val appContext = constructor.newInstance(
            modulesProvider,
            ModuleRegistry(emptyList(), emptyList()),
            WeakReference(reactContext)
        ) as AppContext
        return TestExpoContext(reactContext, appContext)
    }
}
