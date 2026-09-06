package com.apollohg.editor
import android.app.Activity
import android.os.Handler
import android.os.Looper
import android.view.inputmethod.EditorInfo
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.time.Duration
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

internal abstract class NativeEditorExpoViewControlledUpdateTestFixture : NativeEditorExpoViewTestSupport(){
    protected fun bindFocusedViewForTypingTest(
        activity: Activity,
        view: NativeEditorExpoView,
        viewToken: Long,
        payloads: MutableList<Map<String, Any>>
    ) {
        view.onEditorUpdateForTesting = { payloads += it }
        view.onAddonEventForTesting = {}
        view.onEditorReadyForTesting = {}
        view.onSelectionChangeForTesting = {}
        view.onFocusChangeForTesting = {}
        view.onContentHeightChangeForTesting = {}
        activity.setContentView(view)
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorId(viewToken)
        assertTrue(view.richTextView.editorEditText.requestFocus())
    }

    protected fun NativeEditorExpoView.imeTraceSnapshotForTypingTest(): List<String> =
        richTextView.editorEditText.imeTraceSnapshotForTesting()
}
