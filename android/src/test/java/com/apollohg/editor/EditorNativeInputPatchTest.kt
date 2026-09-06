package com.apollohg.editor

import android.os.Looper
import android.view.inputmethod.EditorInfo
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorNativeInputPatchTest : EditorInputConnectionTestFixture() {
    @Test
    fun `native owner patches include optimistic typing at each block start`() {
        val harness = structuredDeleteHarness("<p>Alpha</p><p>Beta</p>")
        try {
            harness.adapter.claimNativeBindingIfUnowned(1L)
            val editor = harness.editText
            editor.setSelection(0)
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            input.commitText("A", 1)
            assertEquals(editor.imeTraceSnapshotForTesting().joinToString("\n"), "AAlpha\nBeta", editor.text.toString())
            assertTrue(editor.lastRenderAppliedPatchForTesting)
            input.commitText(" ", 1)
            assertEquals("<p>A Alpha</p><p>Beta</p>", harness.adapter.documentHtml())
            assertEquals("A Alpha\nBeta", editor.text.toString())
            input.setSelection(8, 8)
            input.commitText("😀", 1)
            assertEquals("<p>A Alpha</p><p>😀Beta</p>", harness.adapter.documentHtml())
            assertEquals("A Alpha\n😀Beta", editor.text.toString())
            assertTrue(editor.lastRenderAppliedPatchForTesting)
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `native owner patches include committed composition at a block start`() {
        val harness = structuredDeleteHarness("<p>Alpha</p><p>Beta</p>")
        try {
            harness.adapter.claimNativeBindingIfUnowned(1L)
            val editor = harness.editText
            editor.setSelection(6)
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            input.setComposingText("に", 1)
            input.setComposingText("日本", 1)
            input.commitText("日本", 1)
            shadowOf(Looper.getMainLooper()).idle()
            assertEquals("<p>Alpha</p><p>日本Beta</p>", harness.adapter.documentHtml())
            assertEquals(editor.imeTraceSnapshotForTesting().joinToString("\n"), "Alpha\n日本Beta", editor.text.toString())
        } finally { harness.adapter.destroy() }
    }
}
