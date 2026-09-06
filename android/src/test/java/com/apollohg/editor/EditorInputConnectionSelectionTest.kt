package com.apollohg.editor
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Color
import android.os.Bundle
import android.os.Looper
import android.provider.Settings
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.InputType
import android.text.style.AbsoluteSizeSpan
import android.view.MotionEvent
import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CompletionInfo
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.RobolectricTestRunner
import org.robolectric.Robolectric
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorInputConnectionSelectionTest : EditorInputConnectionTestFixture() {
    @Test
    fun `selection updates after restart use IME visible coordinates`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = EditorEditText(activity)
        activity.setContentView(editText)
        editText.setText("\u200Ba")
        editText.setSelection(2)
        assertTrue(editText.requestFocus())

        editText.setPrivateImeOptionsForEditor("mapped-selection")
        shadowOf(Looper.getMainLooper()).idle()

        val trace = editText.imeTraceSnapshotForTesting()
        assertTrue(trace.toString(), trace.any {
            it.contains("updateSelectionAfterRestart:source=privateImeOptions sel=1..1")
        })
    }
}
