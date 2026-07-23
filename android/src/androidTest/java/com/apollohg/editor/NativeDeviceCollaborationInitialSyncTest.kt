package com.apollohg.editor

import android.app.Activity
import android.app.Instrumentation
import android.content.Context
import android.graphics.Color
import android.os.SystemClock
import android.view.ViewGroup
import android.widget.FrameLayout
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import expo.modules.core.ModuleRegistry
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.ModulesProvider
import expo.modules.kotlin.modules.Module
import java.lang.ref.WeakReference
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
@LargeTest
class NativeDeviceCollaborationInitialSyncTest {
    private val instrumentation: Instrumentation = InstrumentationRegistry.getInstrumentation()

    @Test
    fun propDrivenReplaceUpdateDisplaysRemoteDocumentWithoutTyping() {
        ActivityScenario.launch(NativeEditorOutsideTapActivity::class.java).use { scenario ->
            val editorRef = AtomicReference<NativeEditorExpoView>()
            val (adapter, editorId) = createV2Editor()

            try {
                scenario.onActivity { activity ->
                    editorRef.set(createMountedEditor(activity, editorId))
                }
                instrumentation.waitForIdleSync()
                waitUntil("editor should bind initial id") {
                    editorRef.get().richTextView.editorEditText.editorId == editorId
                }

                val updateJson = AtomicReference<String>()
                scenario.onActivity {
                    replaceDocumentV2(adapter, documentJson("Remote replace sync"))
                    val update = adapter.refreshFromRustState(null)
                    if (update == null) return@onActivity
                    updateJson.set(update)
                    editorRef.get().setPendingEditorUpdateJson(update)
                    editorRef.get().setPendingEditorUpdateEditorId(editorId)
                    editorRef.get().setPendingEditorUpdateRevision(1)
                    editorRef.get().applyPendingEditorUpdateIfNeeded()
                }
                instrumentation.waitForIdleSync()

                waitUntil(
                    "replace update should render remote document",
                    detail = {
                        "text=${editorRef.get().richTextView.editorEditText.text} " +
                            "trace=${editorRef.get().richTextView.editorEditText.imeTraceSnapshotForTesting().joinToString("|")} " +
                            "update=${updateJson.get()}"
                    }
                ) {
                    editorRef.get().richTextView.editorEditText.text.toString() ==
                        "Remote replace sync"
                }
            } finally {
                releasePairedV2TestEditor(editorId)
            }
        }
    }

    @Test
    fun propDrivenResetUpdateDisplaysRemoteDocumentWithoutTyping() {
        ActivityScenario.launch(NativeEditorOutsideTapActivity::class.java).use { scenario ->
            val editorRef = AtomicReference<NativeEditorExpoView>()
            val (adapter, editorId) = createV2Editor()

            try {
                scenario.onActivity { activity ->
                    editorRef.set(createMountedEditor(activity, editorId))
                }
                instrumentation.waitForIdleSync()
                waitUntil("editor should bind initial id") {
                    editorRef.get().richTextView.editorEditText.editorId == editorId
                }

                val updateJson = AtomicReference<String>()
                scenario.onActivity {
                    if (adapter.setContentJson(documentJson("Remote reset sync")) == null) return@onActivity
                    val update = atomicRenderSnapshot(adapter)
                    updateJson.set(update)
                    editorRef.get().setPendingEditorResetUpdateJson(update)
                    editorRef.get().setPendingEditorResetUpdateEditorId(editorId)
                    editorRef.get().setPendingEditorResetUpdateRevision(1)
                    editorRef.get().applyPendingEditorResetUpdateIfNeeded()
                }
                instrumentation.waitForIdleSync()

                waitUntil(
                    "reset update should render remote document",
                    detail = {
                        "text=${editorRef.get().richTextView.editorEditText.text} " +
                            "trace=${editorRef.get().richTextView.editorEditText.imeTraceSnapshotForTesting().joinToString("|")} " +
                            "update=${updateJson.get()}"
                    }
                ) {
                    editorRef.get().richTextView.editorEditText.text.toString() ==
                        "Remote reset sync"
                }

                // A malformed external snapshot still reports an editor error, but the test
                // host has no Catalyst instance. Delivery must be dropped without crashing.
                adapter.adoptExternalRender("{}")
                instrumentation.waitForIdleSync()
                assertTrue(
                    "editor error queue should drain after the React instance is unavailable",
                    editorRef.get().pendingEditorErrorEventCountForTesting() == 0
                )
            } finally {
                releasePairedV2TestEditor(editorId)
            }
        }
    }

    private fun createV2Editor(): Pair<EditorV2Adapter, Long> {
        return createPairedV2TestEditor()
    }

    private fun atomicRenderSnapshot(adapter: EditorV2Adapter): String =
        when (val result = UniffiEditorV2Backend.renderUpdate(adapter.editorId, null, null)) {
            is EditorV2CallResult.Ok -> result.value
            is EditorV2CallResult.Err ->
                error("v2 renderUpdate failed: ${result.error.code}: ${result.error.message}")
        }

    private fun replaceDocumentV2(adapter: EditorV2Adapter, documentJson: String) {
        val requestJson = JSONObject()
            .put("version", 1)
            .put("requestId", 1)
            .put("setJson", JSONObject(documentJson))
            .put("history", "undoableBoundary")
            .toString()
        when (val result = UniffiEditorV2Backend.replaceDocument(adapter.editorId, requestJson)) {
            is EditorV2CallResult.Ok -> Unit
            is EditorV2CallResult.Err ->
                error("v2 replaceDocument failed: ${result.error.code}: ${result.error.message}")
        }
    }

    private fun createMountedEditor(
        activity: Activity,
        editorId: Long
    ): NativeEditorExpoView {
        initializeSoLoaderIfAvailable(activity)
        val root = FrameLayout(activity).apply {
            setBackgroundColor(Color.WHITE)
        }
        val expoContext = testExpoContext(activity)
        val editor = NativeEditorExpoView(expoContext.context, expoContext.appContext).apply {
            clipToPadding = false
            setShowToolbar(false)
            onFocusChangeForTesting = {}
            onAddonEventForTesting = {}
            onEditorUpdateForTesting = {}
            onEditorReadyForTesting = {}
        }
        root.addView(
            editor,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                dp(activity, 240)
            ).apply {
                topMargin = dp(activity, 48)
                leftMargin = dp(activity, 16)
                rightMargin = dp(activity, 16)
            }
        )
        activity.setContentView(root)
        editor.setEditorId(editorId)
        return editor
    }

    private fun documentJson(text: String): String =
        JSONObject()
            .put("type", "doc")
            .put(
                "content",
                JSONArray().put(
                    JSONObject()
                        .put("type", "paragraph")
                        .put(
                            "content",
                            JSONArray().put(
                                JSONObject()
                                    .put("type", "text")
                                    .put("text", text)
                            )
                        )
                )
            )
            .toString()

    private fun waitUntil(
        description: String,
        timeoutMs: Long = 4_000,
        detail: () -> String = { "" },
        condition: () -> Boolean
    ) {
        val start = SystemClock.uptimeMillis()
        while (SystemClock.uptimeMillis() - start < timeoutMs) {
            instrumentation.waitForIdleSync()
            if (condition()) return
            SystemClock.sleep(50)
        }
        assertTrue("$description\n${detail()}", condition())
    }

    private fun dp(context: Context, value: Int): Int =
        (value * context.resources.displayMetrics.density).toInt()

    private fun initializeSoLoaderIfAvailable(context: Context) {
        try {
            Class
                .forName("com.facebook.soloader.SoLoader")
                .getMethod(
                    "init",
                    Context::class.java,
                    Boolean::class.javaPrimitiveType
                )
                .invoke(null, context, false)
        } catch (_: Throwable) {
            // Some test classpaths do not expose SoLoader directly; in that case the view can
            // still be exercised as long as the React Native draw path does not require it.
        }
    }

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

    private data class TestExpoContext(
        val context: Context,
        val appContext: AppContext
    )
}
