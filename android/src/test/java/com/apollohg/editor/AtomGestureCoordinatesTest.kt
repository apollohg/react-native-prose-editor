package com.apollohg.editor

import android.app.Activity
import android.content.Context
import android.os.Looper
import android.view.View
import android.widget.FrameLayout
import com.facebook.react.R
import com.facebook.react.views.view.ReactViewGroup
import expo.modules.core.ModuleRegistry
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.ModulesProvider
import expo.modules.kotlin.modules.Module
import java.lang.ref.WeakReference
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "xhdpi")
class AtomGestureCoordinatesTest {
    @Test
    fun `atom events include visible React host coordinates after native scrolling`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val reactContext = Class.forName("com.facebook.react.bridge.BridgeReactContext")
            .getConstructor(Context::class.java).newInstance(activity) as Context
        val provider = object : ModulesProvider {
            override fun getModulesMap(): Map<Class<out Module>, String?> = emptyMap()
        }
        val appContext = AppContext::class.java.constructors.first { it.parameterTypes.size == 3 }
            .newInstance(provider, ModuleRegistry(emptyList(), emptyList()), WeakReference(reactContext)) as AppContext
        val editor = NativeEditorExpoView(reactContext, appContext)
        val events = mutableListOf<Map<String, Any>>()
        editor.onAtomLayoutForTesting = { events.add(it) }
        activity.setContentView(editor, FrameLayout.LayoutParams(400, 300))
        val view = editor.richTextView
        view.applyTheme(EditorTheme.fromJson("""{"contentInsets":{"top":18}}"""))
        view.applyAtomRenderConfiguration(AtomRenderConfiguration(setOf("counterCard"), mapOf("counterCard" to 100f), emptyMap()))
        view.editorEditText.applyRenderJSON(
            (0..4).joinToString(prefix = "[", postfix = "]") {
                """{"type":"voidBlock","nodeType":"counterCard","docPos":${it * 2 + 1},"atomId":"counter-$it"}"""
            }
        )
        editor.measure(View.MeasureSpec.makeMeasureSpec(400, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(300, View.MeasureSpec.EXACTLY))
        editor.layout(0, 0, 400, 300)
        shadowOf(Looper.getMainLooper()).idle()
        val host = ReactViewGroup(reactContext).apply {
            setTag(R.id.view_tag_native_id, "prose-atom:counter-2")
            layout(0, 0, 400, 200)
        }
        editor.addAtomChild(host, 0)
        val eventsBeforeScroll = events.size
        view.editorScrollView.scrollTo(0, 120)
        assertEquals(120, view.editorScrollView.scrollY)
        assertTrue(events.size > eventsBeforeScroll)
        val hostLocation = IntArray(2).also(host::getLocationOnScreen)
        val editorLocation = IntArray(2).also(editor::getLocationOnScreen)
        val density = editor.resources.displayMetrics.density
        @Suppress("UNCHECKED_CAST")
        val positions = events.last()["positions"] as List<Map<String, Any>>
        val position = positions.single { it["key"] == "counter-2" }
        assertEquals((hostLocation[0] - editorLocation[0]) / density, position["hostX"])
        assertEquals((hostLocation[1] - editorLocation[1]) / density, position["hostY"])
    }
}
