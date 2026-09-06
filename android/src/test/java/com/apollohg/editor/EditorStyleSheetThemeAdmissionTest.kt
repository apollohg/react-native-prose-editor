package com.apollohg.editor

import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class EditorStyleSheetThemeAdmissionTest : NativeEditorExpoViewTestSupport() {
    @Test
    fun `invalid native theme preserves previous presentation`() {
        val context = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(context.context, context.appContext)
        val valid = """{"version":1,"styles":{"paragraph":{"fontSize":21}}}"""
        view.applyThemeJson(valid)
        val previous = view.richTextView.editorEditText.theme
        for (invalid in listOf("""{"version":2,"styles":{}}""", """{"version":"1","styles":{}}""", """{"version":1,"styles":[]}""")) {
            assertThrows(IllegalArgumentException::class.java) { view.applyThemeJson(invalid) }
            assertEquals(valid, view.lastThemeJson)
            assertSame(previous, view.richTextView.editorEditText.theme)
        }
    }
}
