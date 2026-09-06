package com.apollohg.editor
import android.app.Activity
import android.graphics.Point
import android.os.Looper
import android.view.MotionEvent
import android.view.Window
import android.view.inputmethod.EditorInfo
import android.widget.FrameLayout
import android.widget.ScrollView
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.time.Duration
import java.util.concurrent.atomic.AtomicBoolean

internal abstract class NativeEditorExpoViewTestFixture : NativeEditorExpoViewTestSupport(){
    protected companion object {
        const val AUTO_GROW_MIN_HEIGHT_PX = 900
    }
    protected fun attachedNativeEditorView(): NativeEditorExpoView {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = FrameLayout(activity)
        activity.setContentView(host)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779904L
        val editText = view.richTextView.editorEditText

        view.onFocusChangeForTesting = {}
        view.onAddonEventForTesting = {}
        host.addView(view, FrameLayout.LayoutParams(200, 200))
        val widthSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            200,
            android.view.View.MeasureSpec.EXACTLY
        )
        val heightSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            200,
            android.view.View.MeasureSpec.EXACTLY
        )
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, 200, 200)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson("ready"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorFocusedForOutsideTapDecisionForTesting(true)
        return view
    }
}
