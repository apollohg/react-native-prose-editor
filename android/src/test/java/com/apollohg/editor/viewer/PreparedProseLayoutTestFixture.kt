package com.apollohg.editor.viewer
import android.graphics.Canvas
import android.graphics.Bitmap
import android.graphics.Rect
import android.app.Activity
import android.os.Looper
import android.text.StaticLayout
import android.text.TextPaint
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.view.accessibility.AccessibilityNodeInfo
import com.apollohg.editor.PreparedProseRecyclerHarness
import com.apollohg.editor.PreparedProseBenchmarkConfiguration
import com.apollohg.editor.PreparedProsePerformanceGates
import com.apollohg.editor.ProseViewerConfiguration
import com.apollohg.editor.ProseViewerError
import com.apollohg.editor.ProseViewerErrorCode
import com.apollohg.editor.ProseViewerInteractionListenerAdapter
import com.apollohg.editor.ProseViewerMention
import com.apollohg.editor.ProseViewerSource
import com.apollohg.editor.ProseViewerView
import com.apollohg.editor.OrderedListMarkerSpan
import com.apollohg.editor.RenderBridge
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Robolectric
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.io.File
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject

internal abstract class PreparedProseLayoutTestFixture {
    protected val context
        get() = RuntimeEnvironment.getApplication()

    protected fun JSONArray.toWarmWindows() = List(length()) { index ->
        getJSONObject(index).let { window ->
            fun ids(key: String) = window.getJSONArray(key).let { values ->
                List(values.length()) { valueIndex -> values.getString(valueIndex) }
            }
            PreparedProseRecyclerHarness.WarmWindow(window.getString("id"), ids("primeIds"), ids("warmIds"))
        }
    }

    protected fun windowEvidence(
        window: PreparedProseRecyclerHarness.WarmWindow,
        phase: String,
        ids: List<String>,
    ) = JSONObject()
        .put("windowId", window.id)
        .put("phase", phase)
        .put("entryIds", JSONArray(ids))

    protected fun drainMainLooperUntil(predicate: () -> Boolean) {
        repeat(600) {
            if (predicate()) return
            shadowOf(Looper.getMainLooper()).idleFor(16, TimeUnit.MILLISECONDS)
        }
        assertTrue("expected attached RecyclerView lifecycle to complete", predicate())
    }

    protected fun testRegistry(engine: AndroidProseLayoutEngine): PreparedProseLayoutRegistry =
        PreparedProseLayoutRegistry(CountingDocumentCompiler(::testDocument), engine)

    protected fun testDocument(request: ProseViewerRequest): ViewerDocument = ViewerDocument(
        semanticKey = request.compiledCacheKey,
        blocks = listOf(
            ViewerBlock(
                nodeType = "paragraph",
                depth = 0,
                inBlockquote = false,
                listContext = null,
                listItemBoundary = null,
                inlines = listOf(ViewerInline.Text(request.source.value, emptyList())),
            )
        ),
        isEmpty = request.source.value.isEmpty(),
        retainedBytes = request.source.value.length.toLong(),
    )

    protected fun jsonSource(value: String) = ProseViewerSource.Json(value)

    protected fun configuration() = ProseViewerConfiguration(configJson = "{}")

    protected fun request(value: String) = ProseViewerRequest(jsonSource(value), configuration())

    protected fun testLayoutKey(generation: String) = ProseLayoutKey(
        semanticKey = generation,
        widthPx = 320,
        themeDigest = "theme",
        nativeFontRevision = 0,
        fontEnvironmentRevision = 0,
        densityBits = 1f.toRawBits().toLong(),
        attachmentRevision = 0,
        generationIdentity = generation,
    )

    protected fun testArtifact(key: ProseLayoutKey, retainedBytes: Long) = PreparedProseLayout(
        key = key,
        widthPx = key.widthPx,
        heightPx = 1,
        blocks = emptyList(),
        retainedBytes = retainedBytes,
    )

    protected fun exactWidth(width: Int) = View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY)

    protected fun unspecifiedWidth() = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)

    protected fun unspecifiedHeight() = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)

    protected fun mountVisible(
        parent: CapturingAccessibilityParent,
        child: View,
        width: Int = 320,
        height: Int = 200,
    ) {
        parent.addView(child)
        (child as? ProseViewerView)?.accessibilityVisibilityForTesting = { true }
        parent.measure(
            View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY),
        )
        parent.layout(0, 0, width, height)
        child.layout(0, 0, width, height)
    }

    protected class CapturingAccessibilityParent(context: android.content.Context) : ViewGroup(context) {
        init {
            shadowOf(context.getSystemService(AccessibilityManager::class.java)).setEnabled(true)
        }

        val eventTypes = mutableListOf<Int>()
        private val changeTypes = mutableListOf<Int>()
        var onEvent: ((AccessibilityEvent) -> Unit)? = null

        override fun requestSendAccessibilityEvent(child: View, event: AccessibilityEvent): Boolean {
            onEvent?.invoke(event)
            eventTypes += event.eventType
            changeTypes += event.contentChangeTypes
            return true
        }

        fun clearEvents() {
            eventTypes.clear()
            changeTypes.clear()
        }

        fun subtreeChangeCount(): Int = eventTypes.indices.count { index ->
            eventTypes[index] == AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED &&
                changeTypes[index] == AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE
        }

        override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) = Unit
    }
}

internal class CountingDocumentCompiler(
    private val compile: (ProseViewerRequest) -> ViewerDocument,
) : (ProseViewerRequest) -> ViewerDocument {
    var failures = 0
        private set

    override fun invoke(request: ProseViewerRequest): ViewerDocument = try {
        compile(request)
    } catch (error: ProseViewerError) {
        failures += 1
        throw error
    }
}

internal class CountingLayoutEngine : AndroidProseLayoutEngine {
    private val delegate = StaticLayoutAndroidProseLayoutEngine()
    var preparationCount = 0
        private set

    override fun prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        theme: PreparedProseTheme,
        widthPx: Int,
        density: Float,
        collapsesWhenEmpty: Boolean,
    ): PreparedProseLayout {
        preparationCount += 1
        return delegate.prepare(document, key, theme, widthPx, density, collapsesWhenEmpty)
    }
}

internal class LinkLayoutEngine : AndroidProseLayoutEngine {
    private val delegate = StaticLayoutAndroidProseLayoutEngine()

    override fun prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        theme: PreparedProseTheme,
        widthPx: Int,
        density: Float,
        collapsesWhenEmpty: Boolean,
    ): PreparedProseLayout = delegate.prepare(
        document,
        key,
        theme,
        widthPx,
        density,
        collapsesWhenEmpty,
    ).copy(
        interactions = listOf(
            PreparedProseInteraction(
                kind = PreparedProseInteraction.Kind.LINK,
                rects = listOf(Rect(0, 0, 20, 20)),
                href = "https://example.test",
                visibleText = "link-$widthPx",
                label = "link-$widthPx",
            ),
        ),
        accessibilityNodes = listOf(
            PreparedProseAccessibilityNode(
                interactionIndex = 0,
                role = PreparedProseAccessibilityNode.Role.LINK,
                label = "link-$widthPx",
                bounds = Rect(0, 0, 20, 20),
            ),
        ),
    )
}
