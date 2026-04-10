package com.apollohg.editor

import android.app.Instrumentation
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import androidx.test.platform.app.InstrumentationRegistry
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.editor_core.*

@RunWith(AndroidJUnit4::class)
@LargeTest
class NativeDevicePerformanceTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = ApplicationProvider.getApplicationContext<Context>()
    private val baseFontSize = 16f
    private val textColor = Color.BLACK

    @Test
    fun performance_renderBridgeLargeDocument() {
        val renderJson = NativePerformanceFixtureFactory.largeRenderJson()

        val stats = measureOperation("renderBridgeLargeDocument") {
            val spannable = runOnMainSyncWithResult {
                RenderBridge.buildSpannable(
                    renderJson,
                    baseFontSize,
                    textColor,
                    density = context.resources.displayMetrics.density
                )
            }

            assertTrue("rendered spannable should not be empty", spannable.isNotEmpty())
            if (spannable.isNotEmpty()) {
                runOnMainSyncWithResult {
                    spannable.getSpans(0, minOf(spannable.length, 1), Any::class.java)
                }
            }
        }

        reportStats(stats)
        assertTrue("average render time should be positive", stats.averageMillis > 0.0)
    }

    @Test
    fun performance_applyUpdateJsonLargeDocument() {
        val updateJson = NativePerformanceFixtureFactory.largeUpdateJson()
        val editText = runOnMainSyncWithResult {
            EditorEditText(context).apply {
                layoutParams = ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT
                )
                setBaseStyle(baseFontSize, textColor, Color.WHITE)
                applyUpdateJSON(updateJson, notifyListener = false)
                layoutView(this, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
            }
        }

        val stats = measureOperation("applyUpdateJsonLargeDocument") {
            runOnMainSyncWithResult {
                editText.applyUpdateJSON(updateJson, notifyListener = false)
                layoutView(editText, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
            }

            assertFalse("edit text should contain rendered content", editText.text.isNullOrEmpty())
            assertNotNull("edit text should have a layout after applying update", editText.layout)
        }

        reportStats(stats)
        assertTrue("average applyUpdateJSON time should be positive", stats.averageMillis > 0.0)
    }

    @Test
    fun performance_applyRenderPatchLargeDocument() {
        val initialUpdateJson = NativePerformanceFixtureFactory.largeUpdateJson()
        val patchedUpdateJson = NativePerformanceFixtureFactory.largePatchedUpdateJson()
        val editText = runOnMainSyncWithResult {
            EditorEditText(context).apply {
                layoutParams = ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT
                )
                setBaseStyle(baseFontSize, textColor, Color.WHITE)
            }
        }

        val stats = measureOperation(
            name = "applyRenderPatchLargeDocument",
            beforeEach = {
                runOnMainSyncWithResult {
                    editText.applyUpdateJSON(initialUpdateJson, notifyListener = false)
                    layoutView(editText, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
                }
            }
        ) {
            runOnMainSyncWithResult {
                editText.applyUpdateJSON(patchedUpdateJson, notifyListener = false)
                layoutView(editText, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
            }

            assertFalse("edit text should contain rendered content", editText.text.isNullOrEmpty())
        }

        reportStats(stats)
        assertTrue("average patch apply time should be positive", stats.averageMillis > 0.0)
    }

    @Test
    fun performance_typingRoundTripLargeDocument() {
        val editorId = editorCreate("{}").toLong()
        try {
            NativePerformanceFixtureFactory.loadLargeDocumentIntoEditor(editorId.toULong())
            val editText = runOnMainSyncWithResult {
                EditorEditText(context).apply {
                    layoutParams = ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.WRAP_CONTENT
                    )
                    setBaseStyle(baseFontSize, textColor, Color.WHITE)
                    bindEditor(editorId)
                    layoutView(this, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
                }
            }
            val typingOffset = NativePerformanceFixtureFactory.typingCursorOffset(editText.text ?: "")

            val stats = measureOperation("typingRoundTripLargeDocument") {
                runOnMainSyncWithResult {
                    editText.setSelection(typingOffset)
                    editText.handleTextCommit("!")
                    editText.handleDelete(1, 0)
                    layoutView(editText, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
                }

                assertFalse("edit text should contain rendered content", editText.text.isNullOrEmpty())
                assertNotNull("edit text should have a layout after typing", editText.layout)
            }

            reportStats(stats)
            assertTrue("average typing time should be positive", stats.averageMillis > 0.0)
        } finally {
            editorDestroy(editorId.toULong())
        }
    }

    @Test
    fun performance_paragraphSplitRoundTripLargeDocument() {
        val editorId = editorCreate("{}").toLong()
        try {
            val largeDocumentJson = NativePerformanceFixtureFactory.largeDocumentJson()
            NativePerformanceFixtureFactory.loadLargeDocumentIntoEditor(editorId.toULong())
            val editText = runOnMainSyncWithResult {
                EditorEditText(context).apply {
                    layoutParams = ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.WRAP_CONTENT
                    )
                    setBaseStyle(baseFontSize, textColor, Color.WHITE)
                    bindEditor(editorId)
                    layoutView(this, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
                }
            }
            val splitOffset = NativePerformanceFixtureFactory.typingCursorOffset(editText.text ?: "")

            val stats = measureOperation(
                name = "paragraphSplitRoundTripLargeDocument",
                beforeEach = {
                    runOnMainSyncWithResult {
                        editorSetJson(editorId.toULong(), largeDocumentJson)
                        editText.applyUpdateJSON(editorGetCurrentState(editorId.toULong()), notifyListener = false)
                        editText.setSelection(splitOffset)
                        layoutView(editText, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
                    }
                }
            ) {
                runOnMainSyncWithResult {
                    editText.handleTextCommit("\n")
                    layoutView(editText, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
                }

                assertNotNull("edit text should have a layout after paragraph split", editText.layout)
            }

            reportStats(stats)
            assertTrue("average paragraph split time should be positive", stats.averageMillis > 0.0)
        } finally {
            editorDestroy(editorId.toULong())
        }
    }

    @Test
    fun performance_selectionScrubLargeDocument() {
        val editorId = editorCreate("{}").toLong()
        try {
            NativePerformanceFixtureFactory.loadLargeDocumentIntoEditor(editorId.toULong())
            val editText = runOnMainSyncWithResult {
                EditorEditText(context).apply {
                    layoutParams = ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.WRAP_CONTENT
                    )
                    setBaseStyle(baseFontSize, textColor, Color.WHITE)
                    bindEditor(editorId)
                    layoutView(this, widthPx = 1080, heightPx = 2400, heightMode = View.MeasureSpec.AT_MOST)
                }
            }
            val scrubOffsets = NativePerformanceFixtureFactory.selectionScrubOffsets(
                editText.text ?: "",
                points = 48
            )

            val stats = measureOperation("selectionScrubLargeDocument") {
                runOnMainSyncWithResult {
                    for (offset in scrubOffsets) {
                        editText.setSelection(offset)
                    }
                }

                assertEquals(
                    "selection should land on the final scrub offset",
                    scrubOffsets.last(),
                    editText.selectionStart
                )
            }

            reportStats(stats)
            assertTrue("average selection scrub time should be positive", stats.averageMillis > 0.0)
        } finally {
            editorDestroy(editorId.toULong())
        }
    }

    @Test
    fun performance_remoteSelectionOverlayRefreshMultiPeerLargeDocument() {
        val updateJson = NativePerformanceFixtureFactory.largeUpdateJson()
        val richTextView = runOnMainSyncWithResult {
            RichTextEditorView(context).apply {
                configure(textSizePx = baseFontSize, textColor = textColor, backgroundColor = Color.WHITE)
                setRemoteSelectionEditorIdForTesting(1L)
                setRemoteSelectionScalarResolverForTesting { _, docPos -> docPos }
                editorEditText.applyUpdateJSON(updateJson, notifyListener = false)
                layoutView(this, widthPx = 1080, heightPx = 1600, heightMode = View.MeasureSpec.EXACTLY)
            }
        }

        val totalScalar = richTextView.editorEditText.text?.length ?: 0
        val selections = NativePerformanceFixtureFactory.remoteSelections(
            totalScalar,
            peerCount = 24,
            selectionWidth = 24
        )
        runOnMainSyncWithResult {
            richTextView.setRemoteSelections(selections)
            layoutView(richTextView, widthPx = 1080, heightPx = 1600, heightMode = View.MeasureSpec.EXACTLY)
        }

        val bitmap = Bitmap.createBitmap(1080, 1600, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)

        val stats = measureOperation("remoteSelectionOverlayRefreshMultiPeerLargeDocument") {
            val hasVisibleCaret = runOnMainSyncWithResult {
                bitmap.eraseColor(Color.TRANSPARENT)
                richTextView.setRemoteSelections(selections)
                layoutView(richTextView, widthPx = 1080, heightPx = 1600, heightMode = View.MeasureSpec.EXACTLY)
                richTextView.draw(canvas)
                richTextView.remoteSelectionDebugSnapshotsForTesting().any { it.caretRect != null }
            }

            assertTrue("remote selection overlay should resolve visible carets", hasVisibleCaret)
            assertEquals("expected one snapshot per peer", selections.size, richTextView.remoteSelectionDebugSnapshotsForTesting().size)
        }

        reportStats(stats)
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

        return TimingStats(name, samples)
    }

    private fun reportStats(stats: TimingStats) {
        instrumentation.sendStatus(
            0,
            Bundle().apply {
                putString(
                    Instrumentation.REPORT_KEY_STREAMRESULT,
                    stats.summaryString(tag = "NativeDevicePerformanceTest") + "\n"
                )
            }
        )
    }

    @Suppress("UNCHECKED_CAST")
    private fun <T> runOnMainSyncWithResult(block: () -> T): T {
        val result = AtomicReference<Any?>()
        val error = AtomicReference<Throwable?>()
        instrumentation.runOnMainSync {
            try {
                result.set(block())
            } catch (throwable: Throwable) {
                error.set(throwable)
            }
        }
        error.get()?.let { throw it }
        return result.get() as T
    }
}
