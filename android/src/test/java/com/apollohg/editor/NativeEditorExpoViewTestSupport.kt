package com.apollohg.editor

import android.app.Activity
import android.content.Context
import android.view.inputmethod.EditorInfo
import expo.modules.core.ModuleRegistry
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.ModulesProvider
import expo.modules.kotlin.modules.Module
import org.json.JSONArray
import org.json.JSONObject
import java.lang.ref.WeakReference

abstract class NativeEditorExpoViewTestSupport {
    protected fun renderUpdateJson(text: String): String =
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
            .put("documentVersion", "1")
            .toString()

    protected fun atomicRenderUpdateJson(text: String, revision: String): String =
        JSONObject()
            .put(
                "renderBlocks",
                JSONArray().put(
                    JSONArray()
                        .put(JSONObject().put("type", "blockStart").put("nodeType", "paragraph").put("depth", 0))
                        .put(JSONObject().put("type", "textRun").put("text", text).put("marks", JSONArray()))
                        .put(JSONObject().put("type", "blockEnd"))
                )
            )
            .put("renderPatch", JSONObject.NULL)
            .put("selection", JSONObject().put("type", "text").put("anchor", 1).put("head", 1).put("anchorScalar", 0).put("headScalar", 0))
            .put(
                "activeState",
                JSONObject()
                    .put("marks", JSONObject())
                    .put("markAttrs", JSONObject())
                    .put("nodes", JSONObject().put("paragraph", true))
                    .put("commands", JSONObject())
                    .put("allowedMarks", JSONArray().put("bold"))
                    .put("insertableNodes", JSONArray().put("hardBreak"))
            )
            .put("historyState", JSONObject().put("canUndo", true).put("canRedo", false))
            .put("documentVersion", revision)
            .put("stateRevision", revision)
            .put("scalarLength", text.length)
            .put("documentIsEmpty", text.isEmpty())
            .toString()

    protected fun commitBoundText(view: NativeEditorExpoView, text: String): Boolean {
        val editText = view.richTextView.editorEditText
        editText.setSelection(editText.selectionStart.coerceAtLeast(0))
        val inputConnection = editText.onCreateInputConnection(EditorInfo()) ?: return false
        return inputConnection.commitText(text, 1)
    }

    internal fun attachAdapterForViewTest(
        backend: FakeEditorV2Backend,
        configJson: String = "{\"initialization\":{\"type\":\"localEmpty\"}}"
    ): EditorV2Adapter {
        val created = backend.create(configJson, null)
            as EditorV2CallResult.Ok
        return EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false
        )!!
    }

    protected data class TestExpoContext(
        val context: Context,
        val appContext: AppContext
    )

    protected fun testExpoContext(
        context: Context,
        currentActivity: Activity? = null
    ): TestExpoContext {
        val resolvedCurrentActivity = currentActivity ?: context as? Activity
        val reactContext = Class
            .forName("com.facebook.react.bridge.BridgeReactContext")
            .getConstructor(Context::class.java)
            .newInstance(context) as Context

        if (resolvedCurrentActivity != null) {
            reactContext.javaClass
                .getMethod("onHostResume", Activity::class.java)
                .invoke(reactContext, resolvedCurrentActivity)
        }

        val modulesProvider = object : ModulesProvider {
            override fun getModulesMap(): Map<Class<out Module>, String?> = emptyMap()
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
