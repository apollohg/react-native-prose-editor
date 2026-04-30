package com.apollohg.editor

import android.content.Context
import android.view.View
import expo.modules.core.ModuleRegistry
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.ModulesProvider
import expo.modules.kotlin.modules.Module
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.lang.ref.WeakReference

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativeProseViewerExpoViewTest {
    @Test
    fun `viewer measure ignores stale exact parent height`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeProseViewerExpoView(expoContext.context, expoContext.appContext)
        view.suppressContentHeightEventsForTesting = true
        view.setRenderJson(paragraphRenderJson(LONG_MESSAGE_TEXT))

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val wrapHeightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        view.measure(widthSpec, wrapHeightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
        val contentHeight = view.measuredHeight

        assertTrue(contentHeight > 0)

        val staleExactHeightSpec = View.MeasureSpec.makeMeasureSpec(
            contentHeight + 480,
            View.MeasureSpec.EXACTLY
        )
        view.measure(widthSpec, staleExactHeightSpec)

        assertEquals(contentHeight, view.measuredHeight)
    }

    private data class TestExpoContext(
        val context: Context,
        val appContext: AppContext
    )

    private fun testExpoContext(context: Context): TestExpoContext {
        val reactContext = Class
            .forName("com.facebook.react.bridge.BridgeReactContext")
            .getConstructor(Context::class.java)
            .newInstance(context) as Context

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

    private fun paragraphRenderJson(text: String): String =
        """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"$text","marks":[]},
          {"type":"blockEnd"}
        ]
        """.trimIndent()

    private companion object {
        private val LONG_MESSAGE_TEXT = List(80) {
            "Long Android viewer message"
        }.joinToString(" ")
    }
}
