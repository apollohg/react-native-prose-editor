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

internal abstract class RenderBridgeTestFixture {
    protected fun invokeLifecycle(editor: EditorEditText, name: String) {
        val method = EditorEditText::class.java.getDeclaredMethod(name)
        method.isAccessible = true
        method.invoke(editor)
    }

    protected val baseFontSize = 16f
    protected val textColor = Color.BLACK

    /** A single paragraph with unstyled text should produce the text content. */

    /** Bold mark should produce a StyleSpan with Typeface.BOLD. */

    /** A hardBreak void inline should render as a newline character. */

    /** A horizontalRule should render as FFFC with a HorizontalRuleSpan. */

    /** Two consecutive paragraphs should be separated by a newline. */
}
