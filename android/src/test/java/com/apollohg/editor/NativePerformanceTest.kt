package com.apollohg.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.view.View
import android.view.ViewGroup
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativePerformanceTest {
    private val context = RuntimeEnvironment.getApplication()
    private val baseFontSize = 16f
    private val textColor = Color.BLACK

    @Test
    fun `performance - render bridge large document`() {
        val renderJson = NativePerformanceFixtureFactory.largeRenderJson()

        val stats = measureOperation("renderBridgeLargeDocument") {
            val spannable = RenderBridge.buildSpannable(
                renderJson,
                baseFontSize,
                textColor,
                density = context.resources.displayMetrics.density
            )

            assertTrue("rendered spannable should not be empty", spannable.isNotEmpty())
            if (spannable.isNotEmpty()) {
                spannable.getSpans(
                    0,
                    minOf(spannable.length, 1),
                    Any::class.java
                )
            }
        }

        assertTrue("average render time should be positive", stats.averageMillis > 0.0)
    }

    @Test
    fun `performance - apply update json large document`() {
        val updateJson = NativePerformanceFixtureFactory.largeUpdateJson()
        val editText = EditorEditText(context).apply {
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
            setBaseStyle(baseFontSize, textColor, Color.WHITE)
            captureApplyUpdateTraceForTesting = true
        }
        val traceSamples = mutableListOf<EditorEditText.ApplyUpdateTrace>()

        editText.applyUpdateJSON(updateJson, notifyListener = false)
        layoutView(editText, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)

        val stats = measureOperation("applyUpdateJsonLargeDocument") {
            editText.applyUpdateJSON(updateJson, notifyListener = false)
            layoutView(editText, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
            editText.lastApplyUpdateTrace()?.let(traceSamples::add)

            assertFalse("edit text should contain rendered content", editText.text.isNullOrEmpty())
            assertNotNull("edit text should have a layout after applying update", editText.layout)
        }
        println(
            ApplyUpdateTraceStats(
                name = "applyUpdateJsonLargeDocument.breakdown",
                traces = traceSamples.takeLast(5)
            ).summaryString()
        )

        assertTrue("average applyUpdateJSON time should be positive", stats.averageMillis > 0.0)
    }

    @Test
    fun `performance - apply render patch large document`() {
        val initialUpdateJson = NativePerformanceFixtureFactory.largeUpdateJson()
        val patchedUpdateJson = NativePerformanceFixtureFactory.largePatchedUpdateJson()
        val editText = EditorEditText(context).apply {
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
            setBaseStyle(baseFontSize, textColor, Color.WHITE)
            captureApplyUpdateTraceForTesting = true
        }
        val traceSamples = mutableListOf<EditorEditText.ApplyUpdateTrace>()

        val stats = measureOperation(
            name = "applyRenderPatchLargeDocument",
            beforeEach = {
                editText.applyUpdateJSON(initialUpdateJson, notifyListener = false)
                layoutView(editText, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
            }
        ) {
            editText.applyUpdateJSON(patchedUpdateJson, notifyListener = false)
            layoutView(editText, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
            editText.lastApplyUpdateTrace()?.let(traceSamples::add)

            assertFalse("edit text should contain rendered content", editText.text.isNullOrEmpty())
        }
        println(
            ApplyUpdateTraceStats(
                name = "applyRenderPatchLargeDocument.breakdown",
                traces = traceSamples.takeLast(5)
            ).summaryString()
        )

        assertTrue("average patch apply time should be positive", stats.averageMillis > 0.0)
    }

    @Test
    fun `performance - remote selection overlay refresh multi peer large document`() {
        val updateJson = NativePerformanceFixtureFactory.largeUpdateJson()
        val richTextView = RichTextEditorView(context).apply {
            configure(textSizePx = baseFontSize, textColor = textColor, backgroundColor = Color.WHITE)
            setRemoteSelectionEditorIdForTesting(1L)
            setRemoteSelectionScalarResolverForTesting { _, docPos -> docPos }
        }

        richTextView.editorEditText.applyUpdateJSON(updateJson, notifyListener = false)
        layoutView(richTextView, widthPx = 1080, heightPx = 1600, heightMode = View.MeasureSpec.EXACTLY)

        val totalScalar = richTextView.editorEditText.text?.length ?: 0
        val selections = NativePerformanceFixtureFactory.remoteSelections(
            totalScalar,
            peerCount = 24,
            selectionWidth = 24
        )
        richTextView.setRemoteSelections(selections)
        layoutView(richTextView, widthPx = 1080, heightPx = 1600, heightMode = View.MeasureSpec.EXACTLY)

        val bitmap = Bitmap.createBitmap(1080, 1600, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)

        val stats = measureOperation("remoteSelectionOverlayRefreshMultiPeerLargeDocument") {
            bitmap.eraseColor(Color.TRANSPARENT)
            richTextView.setRemoteSelections(selections)
            layoutView(richTextView, widthPx = 1080, heightPx = 1600, heightMode = View.MeasureSpec.EXACTLY)
            richTextView.draw(canvas)

            val snapshots = richTextView.remoteSelectionDebugSnapshotsForTesting()
            assertTrue(
                "remote selection overlay should resolve visible carets",
                snapshots.any { it.caretRect != null }
            )
            assertEquals("expected one snapshot per peer", selections.size, snapshots.size)
        }

        assertTrue("average remote selection refresh time should be positive", stats.averageMillis > 0.0)
    }

    private fun layoutView(
        view: View,
        widthPx: Int,
        heightPx: Int,
        heightMode: Int
    ) {
        val widthSpec = View.MeasureSpec.makeMeasureSpec(widthPx, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(heightPx, heightMode)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
    }

    private fun measureOperation(
        name: String,
        warmupIterations: Int = 3,
        measuredIterations: Int = 5,
        beforeEach: (() -> Unit)? = null,
        block: () -> Unit
    ): TimingStats {
        repeat(warmupIterations) {
            beforeEach?.invoke()
            block()
        }

        val samples = MutableList(measuredIterations) {
            beforeEach?.invoke()
            val startedAt = System.nanoTime()
            block()
            System.nanoTime() - startedAt
        }

        val stats = TimingStats(name, samples)
        println(stats.summaryString())
        return stats
    }
}
