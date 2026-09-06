package com.apollohg.editor
import android.graphics.Color
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Rect
import android.text.SpannableStringBuilder
import android.text.StaticLayout
import android.text.TextPaint
import android.text.Spanned
import android.text.style.ForegroundColorSpan
import android.text.style.LeadingMarginSpan
import android.widget.LinearLayout
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class RichTextEditorViewRenderPatchesTest : RichTextEditorViewTestFixture() {
    @Test
    fun `apply update json resolves patch-only payload for middle paragraph split`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(paragraphRenderBlock("Alpha"))
            put(paragraphRenderBlock("Beta"))
            put(paragraphRenderBlock("Gamma"))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        val patchedBlocks = JSONArray().apply {
            put(paragraphRenderBlock("Alpha"))
            put(paragraphRenderBlock("Beta"))
            put(paragraphRenderBlock("\u200B"))
            put(paragraphRenderBlock("Gamma"))
        }
        val renderPatch = JSONObject()
            .put("startIndex", 1)
            .put("deleteCount", 2)
            .put(
                "renderBlocks",
                JSONArray().apply {
                    put(paragraphRenderBlock("Beta"))
                    put(paragraphRenderBlock("\u200B"))
                    put(paragraphRenderBlock("Gamma"))
                }
            )

        editText.applyUpdateJSON(
            renderUpdateJson(
                patchedBlocks,
                includeFullRenderBlocks = false,
                renderPatch = renderPatch
            ),
            notifyListener = false
        )

        assertEquals("Alpha\nBeta\n\u200B\nGamma", editText.text?.toString())
        assertFalse(editText.lastRenderAppliedPatch())
    }

    @Test
    fun `count changing patch falls back before later indexed patch`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(paragraphRenderBlock("Alpha"))
            put(paragraphRenderBlock("Beta"))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        val insertPatch = JSONObject()
            .put("startIndex", 0)
            .put("deleteCount", 0)
            .put("renderBlocks", JSONArray().put(paragraphRenderBlock("Extra")))
        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = insertPatch
            ),
            notifyListener = false
        )

        assertEquals("Extra\nAlpha\nBeta", editText.text?.toString())
        assertFalse(editText.lastRenderAppliedPatch())

        val replacePatch = JSONObject()
            .put("startIndex", 1)
            .put("deleteCount", 1)
            .put("renderBlocks", JSONArray().put(paragraphRenderBlock("Updated")))
        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = replacePatch
            ),
            notifyListener = false
        )

        assertEquals("Extra\nUpdated\nBeta", editText.text?.toString())
        assertTrue(editText.lastRenderAppliedPatch())
    }

    @Test
    fun `registered atom types do not disable paragraph patching`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        assertTrue(editText.applyAtomRenderConfiguration(AtomRenderConfiguration.fromJson(
            """{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"""
        )))
        editText.applyUpdateJSON(
            renderUpdateJson(JSONArray().put(paragraphRenderBlock("Before"))),
            notifyListener = false
        )
        val renderPatch = JSONObject()
            .put("startIndex", 0)
            .put("deleteCount", 1)
            .put("renderBlocks", JSONArray().put(paragraphRenderBlock("After")))

        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = renderPatch
            ),
            notifyListener = false
        )

        assertEquals("After", editText.text.toString())
        assertTrue(editText.lastRenderAppliedPatch())
    }

    @Test
    fun `atom patch without stable ids falls back to a full render`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        assertTrue(editText.applyAtomRenderConfiguration(AtomRenderConfiguration.fromJson(
            """{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"""
        )))
        fun atomBlock(docPos: Int): JSONArray = JSONArray().put(
            JSONObject()
                .put("type", "voidBlock")
                .put("nodeType", "counterCard")
                .put("docPos", docPos)
        )
        val initialBlocks = JSONArray()
            .put(atomBlock(1))
            .put(atomBlock(2))
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        val renderPatch = JSONObject()
            .put("startIndex", 1)
            .put("deleteCount", 1)
            .put("renderBlocks", JSONArray().put(atomBlock(3)))
        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = renderPatch
            ),
            notifyListener = false
        )

        val content = editText.text as Spanned
        val atomKeys = content.getSpans(0, content.length, AtomBlockSpan::class.java)
            .map { it.atomKey }
        assertEquals(listOf("counterCard:0", "counterCard:1"), atomKeys)
        assertFalse(editText.lastRenderAppliedPatch())
    }

    @Test
    fun `nested list return patches preserve list and blockquote layout`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(blockquoteRenderBlock("Quoted"))
            put(nestedListRenderBlock(ListRenderState.INITIAL))
            put(paragraphRenderBlock("After"))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        fun leftOf(text: String): Float {
            val content = editText.text as Spanned
            val offset = content.toString().indexOf(text)
            val layout = StaticLayout.Builder
                .obtain(content, 0, content.length, TextPaint().apply { textSize = 16f }, 800)
                .setIncludePad(false)
                .build()
            return layout.getPrimaryHorizontal(offset)
        }

        fun leadingMarginCount(text: String): Int {
            val content = editText.text as Spanned
            val offset = content.toString().indexOf(text)
            return content.getSpans(offset, offset + 1, LeadingMarginSpan::class.java).size
        }

        fun quoteSpanEnd(): Int {
            val content = editText.text as Spanned
            return content.getSpanEnd(content.getSpans(0, content.length, BlockquoteSpan::class.java).single())
        }

        fun applyListPatch(state: ListRenderState) {
            val renderPatch = JSONObject()
                .put("startIndex", 1)
                .put("deleteCount", 1)
                .put("renderBlocks", JSONArray().put(nestedListRenderBlock(state)))
            editText.applyUpdateJSON(
                renderUpdateJson(
                    JSONArray(),
                    includeFullRenderBlocks = false,
                    renderPatch = renderPatch
                ),
                notifyListener = false
            )
            assertTrue(editText.lastRenderAppliedPatch())
        }

        val expectedText = "Quoted\n• First\n• Second\n• Nested\n• \u200B\nAfter"
        val initialQuoteLeft = leftOf("Quoted")
        val initialRootLeft = leftOf("First")
        val initialSecondLeft = leftOf("Second")
        val initialNestedLeft = leftOf("Nested")
        val initialAfterLeft = leftOf("After")
        val initialRootMargins = leadingMarginCount("First")
        val initialNestedMargins = leadingMarginCount("Nested")

        applyListPatch(ListRenderState.NESTED_EMPTY)

        assertEquals(expectedText, editText.text.toString())
        assertEquals(initialQuoteLeft, leftOf("Quoted"), 0.01f)
        assertEquals(initialRootLeft, leftOf("First"), 0.01f)
        assertEquals(initialSecondLeft, leftOf("Second"), 0.01f)
        assertEquals(initialNestedLeft, leftOf("Nested"), 0.01f)
        assertEquals(initialNestedLeft, leftOf("\u200B"), 0.01f)
        assertEquals(initialAfterLeft, leftOf("After"), 0.01f)
        assertEquals(initialRootMargins, leadingMarginCount("First"))
        assertEquals(initialNestedMargins, leadingMarginCount("Nested"))
        assertEquals(editText.text.toString().indexOf("• First"), quoteSpanEnd())

        applyListPatch(ListRenderState.PARENT_EMPTY)

        assertEquals(expectedText, editText.text.toString())
        assertEquals(initialQuoteLeft, leftOf("Quoted"), 0.01f)
        assertEquals(initialRootLeft, leftOf("First"), 0.01f)
        assertEquals(initialSecondLeft, leftOf("Second"), 0.01f)
        assertEquals(initialNestedLeft, leftOf("Nested"), 0.01f)
        assertEquals(initialRootLeft, leftOf("\u200B"), 0.01f)
        assertEquals(initialAfterLeft, leftOf("After"), 0.01f)
        assertEquals(initialRootMargins, leadingMarginCount("First"))
        assertEquals(initialNestedMargins, leadingMarginCount("Nested"))
        assertEquals(editText.text.toString().indexOf("• First"), quoteSpanEnd())
    }

    @Test
    fun `paragraph patch preserves following blockquote and list paragraph spans`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(paragraphRenderBlock("Before"))
            put(paragraphRenderBlock("Editable paragraph"))
            put(blockquoteRenderBlock("Quoted"))
            put(nestedListRenderBlock(ListRenderState.INITIAL))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        fun paragraphSpanCounts(): Pair<Int, Int> {
            val content = editText.text as Spanned
            val quoteOffset = content.toString().indexOf("Quoted")
            val listOffset = content.toString().indexOf("First")
            return content.getSpans(
                quoteOffset,
                quoteOffset + 1,
                BlockquoteSpan::class.java
            ).size to content.getSpans(
                listOffset,
                listOffset + 1,
                LeadingMarginSpan::class.java
            ).size
        }

        val initialSpanCounts = paragraphSpanCounts()
        val renderPatch = JSONObject()
            .put("startIndex", 1)
            .put("deleteCount", 1)
            .put("renderBlocks", JSONArray().put(paragraphRenderBlock("Edited")))

        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = renderPatch
            ),
            notifyListener = false
        )

        assertTrue(editText.lastRenderAppliedPatch())
        assertEquals("Before\nEdited\nQuoted\n• First\n• Second\n• Nested", editText.text.toString())
        assertEquals(initialSpanCounts, paragraphSpanCounts())
    }

    @Test
    fun `blockquote patch preserves following list paragraph spans`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(blockquoteRenderBlock("Quoted content"))
            put(nestedListRenderBlock(ListRenderState.INITIAL))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        fun listParagraphSpanCounts(): Pair<Int, Int> {
            val content = editText.text as Spanned
            val rootOffset = content.toString().indexOf("First")
            val nestedOffset = content.toString().indexOf("Nested")
            return content.getSpans(
                rootOffset,
                rootOffset + 1,
                LeadingMarginSpan::class.java
            ).size to content.getSpans(
                nestedOffset,
                nestedOffset + 1,
                LeadingMarginSpan::class.java
            ).size
        }

        val initialSpanCounts = listParagraphSpanCounts()
        val renderPatch = JSONObject()
            .put("startIndex", 0)
            .put("deleteCount", 1)
            .put("renderBlocks", JSONArray().put(blockquoteRenderBlock("Quote")))

        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = renderPatch
            ),
            notifyListener = false
        )

        assertTrue(editText.lastRenderAppliedPatch())
        assertEquals("Quote\n• First\n• Second\n• Nested", editText.text.toString())
        assertEquals(initialSpanCounts, listParagraphSpanCounts())
    }

    @Test
    fun `terminal list patch preserves preceding blockquote span`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(blockquoteRenderBlock("Quoted"))
            put(nestedListRenderBlock(ListRenderState.INITIAL))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        fun leftOf(text: String): Float {
            val content = editText.text as Spanned
            val offset = content.toString().indexOf(text)
            val layout = StaticLayout.Builder
                .obtain(content, 0, content.length, TextPaint().apply { textSize = 16f }, 800)
                .setIncludePad(false)
                .build()
            return layout.getPrimaryHorizontal(offset)
        }

        val initialRootLeft = leftOf("First")
        val initialNestedLeft = leftOf("Nested")

        fun applyListPatch(state: ListRenderState) {
            val renderPatch = JSONObject()
                .put("startIndex", 1)
                .put("deleteCount", 1)
                .put("renderBlocks", JSONArray().put(nestedListRenderBlock(state)))
            editText.applyUpdateJSON(
                renderUpdateJson(
                    JSONArray(),
                    includeFullRenderBlocks = false,
                    renderPatch = renderPatch
                ),
                notifyListener = false
            )
        }

        applyListPatch(ListRenderState.NESTED_EMPTY)
        applyListPatch(ListRenderState.PARENT_EMPTY)

        val content = editText.text as Spanned
        assertEquals(1, content.getSpans(0, content.length, BlockquoteSpan::class.java).size)
        assertEquals(initialRootLeft, leftOf("First"), 0.01f)
        assertEquals(initialNestedLeft, leftOf("Nested"), 0.01f)
    }

    @Test
    fun `apply update json skips render work when render blocks are unchanged`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            captureApplyUpdateTraceForTesting = true
        }
        val initialBlocks = JSONArray().apply {
            put(paragraphRenderBlock("Alpha"))
            put(paragraphRenderBlock("Beta"))
        }

        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)
        editText.lastApplyUpdateTrace()

        val selectionOnlyUpdate = JSONObject()
            .put("renderBlocks", JSONArray(initialBlocks.toString()))
            .toString()

        editText.applyUpdateJSON(selectionOnlyUpdate, notifyListener = false)

        val trace = editText.lastApplyUpdateTrace()
        assertNotNull(trace)
        assertTrue(trace?.skippedRender == true)
        assertFalse(trace?.usedPatch == true)
        assertEquals(0L, trace?.buildRenderNanos)
        assertEquals(0L, trace?.applyRenderNanos)
        assertEquals("Alpha\nBeta", editText.text?.toString())
    }

    @Test
    fun `appearance change forces full render for an empty render patch`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val blocks = JSONArray().put(paragraphRenderBlock("Alpha"))
        editText.applyTheme(EditorTheme.fromJson("""{"text":{"color":"#112233"}}"""))
        editText.applyUpdateJSON(renderUpdateJson(blocks), notifyListener = false)

        editText.applyTheme(EditorTheme.fromJson("""{"text":{"color":"#DDEEFF"}}"""))
        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = JSONObject()
                    .put("startIndex", 0)
                    .put("deleteCount", 0)
                    .put("renderBlocks", JSONArray())
            ),
            notifyListener = false
        )

        val color = editText.text
            ?.getSpans(0, 1, ForegroundColorSpan::class.java)
            ?.firstOrNull()
            ?.foregroundColor
        assertEquals(Color.parseColor("#DDEEFF"), color)
        assertFalse(editText.lastRenderAppliedPatch())
    }
}
