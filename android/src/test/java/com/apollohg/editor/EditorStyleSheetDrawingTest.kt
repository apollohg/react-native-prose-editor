package com.apollohg.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.RectF
import android.text.StaticLayout
import android.text.TextPaint
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class EditorStyleSheetDrawingTest {
    @Test
    fun `asymmetric edges omit zero side and clip loaded image corners`() {
        val bitmap = Bitmap.createBitmap(80, 60, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        val box = EditorBoxStyle(backgroundColor = Color.YELLOW, border = EditorEdges(4f, 0f, 4f, 6f), borderColors = listOf(Color.RED, Color.BLUE, Color.GREEN, Color.MAGENTA), corners = EditorCorners(12f, 0f, 0f, 12f))
        val bounds = RectF(0f, 0f, 80f, 60f)
        EditorBoxDrawing.draw(canvas, bounds, box)
        assertEquals(Color.TRANSPARENT, bitmap.getPixel(0, 0))
        assertEquals(Color.RED, bitmap.getPixel(40, 1))
        assertEquals(Color.GREEN, bitmap.getPixel(40, 58))
        assertEquals(Color.MAGENTA, bitmap.getPixel(1, 30))
        assertEquals(Color.YELLOW, bitmap.getPixel(79, 30))
        val pixels = Bitmap.createBitmap(80, 60, Bitmap.Config.ARGB_8888).apply { eraseColor(Color.BLUE) }
        EditorBoxDrawing.drawImage(canvas, pixels, bounds, box, "cover")
        assertEquals(Color.TRANSPARENT, bitmap.getPixel(0, 0))
        assertEquals(Color.MAGENTA, bitmap.getPixel(1, 30))
        assertEquals(Color.BLUE, bitmap.getPixel(40, 30))
    }

    @Test
    fun `custom decoration draws requested color below text`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"underline":{"textDecorationColor":"#ff0000ff","textDecorationStyle":"double"}}}""")!!
        val text = RenderBridge.buildSpannable("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"underline","marks":["underline"]},{"type":"blockEnd"}]""", 24f, Color.BLACK, theme, 1f)
        val layout = StaticLayout.Builder.obtain(text, 0, text.length, TextPaint().apply { textSize = 24f }, 200).setIncludePad(false).build()
        val bitmap = Bitmap.createBitmap(200, 60, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        layout.draw(canvas)
        EditorTextDecorationDrawing.draw(canvas, layout)
        val redPixels = (0 until bitmap.width).sumOf { x -> (0 until bitmap.height).count { y -> val color = bitmap.getPixel(x, y); Color.red(color) > 200 && Color.green(color) < 20 && Color.alpha(color) > 128 } }
        assertTrue("red pixels: $redPixels", redPixels > 100)
    }
}
