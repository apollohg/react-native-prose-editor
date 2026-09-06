package com.apollohg.editor
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Typeface
import android.text.Annotation
import android.text.Layout
import android.text.Spanned
import android.text.SpannableStringBuilder
import android.text.StaticLayout
import android.text.TextPaint
import android.util.Base64
import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import kotlin.math.abs
import android.text.style.AbsoluteSizeSpan
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.LeadingMarginSpan
import android.text.style.StrikethroughSpan
import android.text.style.StyleSpan
import android.text.style.TypefaceSpan
import android.text.style.URLSpan
import android.text.style.UnderlineSpan
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class RenderBridgeTest : RenderBridgeTestFixture() {
    @Test
    fun `initial render policy change reloads images preserving selection and scroll`() {
        RenderImageLoader.resetForTesting()
        val decodeCount = AtomicInteger(0)
        val initialDecodeStarted = CountDownLatch(1)
        val reloadedDecodeStarted = CountDownLatch(1)
        val release = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            when (decodeCount.incrementAndGet()) {
                1 -> initialDecodeStarted.countDown()
                2 -> reloadedDecodeStarted.countDown()
            }
            release.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val editor = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication())
        val json = """
            [
              {"type":"blockStart","nodeType":"paragraph","depth":0},
              {"type":"textRun","text":"hello","marks":[]},
              {"type":"blockEnd"},
              {"type":"voidBlock","nodeType":"image","docPos":7,"attrs":{"src":"https://example.com/policy.png"}}
            ]
        """.trimIndent()
        try {
            editor.applyRenderJSON(json)
            assertTrue(initialDecodeStarted.await(2, TimeUnit.SECONDS))
            editor.layoutParams = ViewGroup.LayoutParams(320, 24)
            editor.measure(
                View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(24, View.MeasureSpec.EXACTLY)
            )
            editor.layout(0, 0, editor.measuredWidth, editor.measuredHeight)
            editor.setSelection(2)
            editor.scrollTo(0, 7)
            assertEquals(7, editor.scrollY)

            editor.setImageLoadingPolicyJson("""{"maxSourceBytes":1234}""")

            assertTrue(reloadedDecodeStarted.await(2, TimeUnit.SECONDS))
            assertEquals(2, decodeCount.get())
            assertEquals(2, editor.selectionStart)
            assertEquals(2, editor.selectionEnd)
            assertEquals(7, editor.scrollY)
        } finally {
            release.countDown()
            RenderImageLoader.resetForTesting()
        }
    }

    @Test
    fun `detach and reattach restarts existing image loads`() {
        RenderImageLoader.resetForTesting()
        val decodeCount = AtomicInteger(0)
        val firstDecodeStarted = CountDownLatch(1)
        val restartedDecodeStarted = CountDownLatch(1)
        val release = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            when (decodeCount.incrementAndGet()) {
                1 -> firstDecodeStarted.countDown()
                2 -> restartedDecodeStarted.countDown()
            }
            release.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val editor = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication())
        val json = """[{"type":"voidBlock","nodeType":"image","docPos":1,"attrs":{"src":"https://example.com/attach.png"}}]"""
        try {
            editor.applyRenderJSON(json)
            assertTrue(firstDecodeStarted.await(2, TimeUnit.SECONDS))
            invokeLifecycle(editor, "onDetachedFromWindow")
            invokeLifecycle(editor, "onAttachedToWindow")

            assertTrue(restartedDecodeStarted.await(2, TimeUnit.SECONDS))
            assertEquals(2, decodeCount.get())
        } finally {
            release.countDown()
            RenderImageLoader.resetForTesting()
        }
    }

    @Test
    fun `completed image load handles are released from editor`() {
        RenderImageLoader.resetForTesting()
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val editor = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication())
        editor.applyRenderJSON(
            """[{"type":"voidBlock","nodeType":"image","docPos":1,"attrs":{"src":"https://example.com/done.png"}}]"""
        )

        repeat(100) {
            org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
            if (editor.activeImageLoadHandleCountForTesting() > 0) Thread.sleep(10)
        }

        assertEquals(0, editor.activeImageLoadHandleCountForTesting())
        RenderImageLoader.resetForTesting()
    }

    @Test
    fun `render - image span honors preferred dimensions`() {
        val json = """
        [
            {"type": "voidBlock", "nodeType": "image", "docPos": 1, "attrs": {
                "src": "https://example.com/cat.png",
                "width": 140,
                "height": 80
            }}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, density = 1f)

        assertTrue(
            "Image should contain object replacement character. Got: '$result'",
            result.toString().contains("\uFFFC")
        )

        val imageSpans = result.getSpans(0, result.length, BlockImageSpan::class.java)
        assertEquals("Should have one BlockImageSpan", 1, imageSpans.size)

        val (widthPx, heightPx) = imageSpans.single().currentSizePx()
        assertEquals(140, widthPx)
        assertEquals(80, heightPx)
    }

    @Test
    fun `layout - image leaves themed spacing before following block`() {
        fun imageGap(spacingAfter: Int): Float {
            val json = """
                [
                  {"type":"voidBlock","nodeType":"image","docPos":1,"attrs":{"src":"data:image/png;base64,%","width":140,"height":80}},
                  {"type":"blockStart","nodeType":"paragraph","depth":0},
                  {"type":"textRun","text":"After","marks":[]},
                  {"type":"blockEnd"}
                ]
            """.trimIndent()
            val theme = EditorTheme.fromJson(
                """{"text":{"spacingAfter":$spacingAfter}}"""
            )
            val result = RenderBridge.buildSpannable(
                json,
                baseFontSize,
                textColor,
                theme,
                density = 1f,
            )
            val layout = StaticLayout.Builder
                .obtain(result, 0, result.length, TextPaint().apply { textSize = baseFontSize }, 320)
                .setIncludePad(false)
                .build()
            layout.draw(
                Canvas(Bitmap.createBitmap(320, layout.height, Bitmap.Config.ARGB_8888))
            )

            val imageSpan = result.getSpans(0, result.length, BlockImageSpan::class.java).single()
            val imageBottom = requireNotNull(imageSpan.currentDrawRect()).bottom
            val followingLine = layout.getLineForOffset(result.indexOf("After"))
            return layout.getLineTop(followingLine) - imageBottom
        }

        val unspacedGap = imageGap(spacingAfter = 0)
        val themedGap = imageGap(spacingAfter = 12)

        assertEquals(12f, themedGap - unspacedGap, 0.5f)
    }

    @Test
    fun `render - oversized preferred image dimensions scale to host width`() {
        val hostView = TextView(org.robolectric.RuntimeEnvironment.getApplication()).apply {
            measure(
                View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
            )
            layout(0, 0, measuredWidth, measuredHeight)
        }
        val json = """
        [
            {"type": "voidBlock", "nodeType": "image", "docPos": 1, "attrs": {
                "src": "https://example.com/cat.png",
                "width": 4000,
                "height": 2000
            }}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(
            json,
            baseFontSize,
            textColor,
            density = 1f,
            hostView = hostView
        )

        val imageSpan = result.getSpans(0, result.length, BlockImageSpan::class.java).single()
        val (widthPx, heightPx) = imageSpan.currentSizePx()
        assertTrue(widthPx <= hostView.width)
        assertTrue(abs(heightPx - (widthPx / 2)) <= 1)
    }

    @Test
    fun `render - non finite and overflowing image dimensions are rejected`() {
        assertEquals(null, org.json.JSONObject("""{"width":"Infinity"}""").optPositiveFiniteFloat("width"))
        assertEquals(null, org.json.JSONObject("""{"width":2147483648}""").optPositiveFiniteFloat("width"))

        val span = BlockImageSpan(
            source = "https://example.com/cat.png",
            hostView = null,
            density = Float.MAX_VALUE,
            preferredWidthDp = Float.MAX_VALUE,
            preferredHeightDp = Float.MAX_VALUE
        )
        val (width, height) = span.currentSizePx()
        assertTrue(width in 1..Int.MAX_VALUE)
        assertTrue(height in 1..Int.MAX_VALUE)
    }

    @Test
    fun `render - data url image span decodes off main without measurement side effects`() {
        RenderImageLoader.resetForTesting()
        val dataUrl =
            "data:image/gif;base64,R0lGODdhAQABAIAAAP///////ywAAAAAAQABAAACAkQBADs="
        val decodeStarted = CountDownLatch(1)
        val releaseDecode = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            decodeStarted.countDown()
            releaseDecode.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val hostView = TextView(org.robolectric.RuntimeEnvironment.getApplication()).apply {
            measure(
                View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
            )
            layout(0, 0, measuredWidth, measuredHeight)
        }
        val json = """
        [
            {"type": "voidBlock", "nodeType": "image", "docPos": 1, "attrs": {
                "src": "$dataUrl"
            }}
        ]
        """.trimIndent()

        try {
            val result = RenderBridge.buildSpannable(
                json,
                baseFontSize,
                textColor,
                density = 1f,
                hostView = hostView
            )
            assertTrue(decodeStarted.await(2, TimeUnit.SECONDS))
            val imageSpan = result.getSpans(0, result.length, BlockImageSpan::class.java).single()
            val beforeMeasure = imageSpan.currentSizePx()
            imageSpan.getSize(Paint(), result, 0, 1, null)
            assertEquals(beforeMeasure, imageSpan.currentSizePx())

            releaseDecode.countDown()
            repeat(20) {
                org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
                if (imageSpan.currentSizePx().first != 1) Thread.sleep(10)
            }
            assertEquals(1 to 1, imageSpan.currentSizePx())
        } finally {
            releaseDecode.countDown()
            RenderImageLoader.resetForTesting()
        }
    }

    @Test
    fun `retired image span rejects a late decoded lease`() {
        RenderImageLoader.resetForTesting()
        val decodeStarted = CountDownLatch(1)
        val releaseDecode = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            decodeStarted.countDown()
            releaseDecode.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val editor = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication())
        val span = BlockImageSpan(
            "data:image/png;base64,AQ==",
            editor,
            density = 1f,
            preferredWidthDp = null,
            preferredHeightDp = null,
        )

        try {
            assertTrue(decodeStarted.await(2, TimeUnit.SECONDS))
            span.close()
            releaseDecode.countDown()
            repeat(20) {
                org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
                Thread.sleep(5)
            }
            assertEquals(
                0,
                DecodedBitmapBudget.shared()
                    .retainedOwnerBytesForTesting(editor.decodedBitmapOwnerId),
            )
        } finally {
            releaseDecode.countDown()
            span.close()
            RenderImageLoader.resetForTesting()
        }
    }

    @Test
    fun `render - image loader deduplicates concurrent remote loads`() {
        RenderImageLoader.resetForTesting()
        val decodeCount = AtomicInteger(0)
        val decodeStarted = CountDownLatch(1)
        val releaseDecode = CountDownLatch(1)
        val callbacks = CountDownLatch(2)
        val bitmap = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        val loaded = mutableListOf<Bitmap?>()

        RenderImageLoader.decodeSourceOverride = { _, _ ->
            decodeCount.incrementAndGet()
            decodeStarted.countDown()
            assertTrue(releaseDecode.await(2, TimeUnit.SECONDS))
            bitmap
        }

        try {
            RenderImageLoader.load("https://example.com/cat.png") {
                synchronized(loaded) {
                    loaded += it
                }
                callbacks.countDown()
            }
            assertTrue(decodeStarted.await(2, TimeUnit.SECONDS))
            RenderImageLoader.load("https://example.com/cat.png") {
                synchronized(loaded) {
                    loaded += it
                }
                callbacks.countDown()
            }

            releaseDecode.countDown()
            repeat(20) {
                org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
                if (callbacks.count > 0L) Thread.sleep(10)
            }
            assertEquals(0L, callbacks.count)
            assertEquals(1, decodeCount.get())
            assertEquals(2, loaded.size)
            assertTrue(loaded.all { loadedBitmap -> loadedBitmap === bitmap })
        } finally {
            releaseDecode.countDown()
            RenderImageLoader.resetForTesting()
        }
    }

    @Test
    fun `render - large images are downsampled for decode`() {
        assertEquals(1, RenderImageDecoder.calculateInSampleSize(width = 1024, height = 768))
        assertEquals(2, RenderImageDecoder.calculateInSampleSize(width = 4096, height = 2048))
        assertEquals(4, RenderImageDecoder.calculateInSampleSize(width = 8192, height = 4096))
    }
}
