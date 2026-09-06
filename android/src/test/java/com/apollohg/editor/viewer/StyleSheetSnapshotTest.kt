package com.apollohg.editor.viewer

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.view.View
import com.apollohg.editor.EditorEditText
import com.apollohg.editor.EditorTheme
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class StyleSheetSnapshotTest {
    @Test
    fun `capture matching editor and prepared style fixtures`() {
        val themeJson = """{"version":1,"styles":{"content":{"paddingTop":16,"paddingRight":16,"paddingBottom":16,"paddingLeft":16,"backgroundColor":"#ffffffff"},"text":{"fontSize":17,"lineHeight":25,"color":"#243147ff"},"h1":{"fontSize":28,"lineHeight":36,"marginBottom":12},"paragraph":{"marginBottom":10},"blockquote":{"paddingTop":10,"paddingRight":12,"paddingBottom":10,"paddingLeft":12,"marginBottom":12,"backgroundColor":"#edf3faff","borderTopWidth":1,"borderRightWidth":0,"borderBottomWidth":1,"borderLeftWidth":4,"borderTopColor":"#91a7baff","borderBottomColor":"#91a7baff","borderLeftColor":"#526d87ff","borderTopRightRadius":12,"borderBottomRightRadius":12},"codeBlock":{"fontSize":14,"lineHeight":22,"color":"#e2e8f0ff","backgroundColor":"#132235ff","paddingTop":12,"paddingRight":12,"paddingBottom":12,"paddingLeft":12,"borderTopLeftRadius":12,"borderTopRightRadius":12,"borderBottomLeftRadius":12,"borderBottomRightRadius":12}}}"""
        val elements = """[{"type":"blockStart","nodeType":"h1","depth":0},{"type":"textRun","text":"Styles in context","marks":[]},{"type":"blockEnd"},{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"One shared stylesheet controls typography, spacing and block appearance.","marks":[]},{"type":"blockEnd"},{"type":"blockStart","nodeType":"blockquote","depth":0},{"type":"blockStart","nodeType":"paragraph","depth":1},{"type":"textRun","text":"A wrapped quotation with its own continuous background and asymmetric border.","marks":[]},{"type":"blockEnd"},{"type":"blockEnd"},{"type":"blockStart","nodeType":"codeBlock","depth":0,"language":"rust"},{"type":"textRun","text":"fn main() {\n    println!(\"Hello\");\n}","marks":[]},{"type":"blockEnd"}]"""
        val context = RuntimeEnvironment.getApplication()
        val editor = EditorEditText(context).apply {
            includeFontPadding = false
            setBaseStyle(17f, Color.BLACK, Color.WHITE)
            applyTheme(EditorTheme.fromJson(themeJson))
            applyRenderJSON(elements)
            measure(View.MeasureSpec.makeMeasureSpec(380, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(1000, View.MeasureSpec.AT_MOST))
            layout(0, 0, measuredWidth, measuredHeight)
        }
        val editorBitmap = Bitmap.createBitmap(380, editor.height, Bitmap.Config.ARGB_8888)
        editor.draw(Canvas(editorBitmap))
        org.junit.Assert.assertEquals(25, editor.layout.getLineBaseline(4) - editor.layout.getLineBaseline(3))
        org.junit.Assert.assertEquals(25, editor.layout.getLineBaseline(5) - editor.layout.getLineBaseline(4))
        java.io.FileOutputStream("/tmp/android-styles-editor.png").use { editorBitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        fun block(type: String, text: String, containers: List<ViewerContainerAncestor> = emptyList()) = ViewerBlock(type, if (containers.isEmpty()) 0 else 1, containers.isNotEmpty(), null, null, listOf(ViewerInline.Text(text, emptyList())), containers = containers)
        val document = ViewerDocument("snapshot", listOf(
            block("h1", "Styles in context"),
            block("paragraph", "One shared stylesheet controls typography, spacing and block appearance."),
            block("paragraph", "A wrapped quotation with its own continuous background and asymmetric border.", listOf(ViewerContainerAncestor(1, "blockquote", 2, 2))),
            block("codeBlock", "fn main() {\n    println!(\"Hello\");\n}"),
        ), false, 0)
        val key = ProseLayoutKey("snapshot", 380, "snapshot", 0, 0, 0, 0, "snapshot")
        val prepared = StaticLayoutAndroidProseLayoutEngine().prepare(document, key, PreparedProseTheme.resolve(themeJson, 1f), 380, 1f, false)
        val viewer = PreparedProseDrawingView(context).apply { install(prepared); layout(0, 0, 380, prepared.heightPx) }
        val viewerBitmap = Bitmap.createBitmap(380, prepared.heightPx, Bitmap.Config.ARGB_8888)
        viewer.draw(Canvas(viewerBitmap))
        java.io.FileOutputStream("/tmp/android-styles-viewer.png").use { viewerBitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        assertTrue(editorBitmap.height > 200)
        assertTrue(viewerBitmap.height > 200)
    }
}
