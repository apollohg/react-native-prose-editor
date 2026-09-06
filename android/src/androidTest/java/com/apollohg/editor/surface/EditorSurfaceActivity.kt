package com.apollohg.editor.surface

import android.app.Activity
import android.graphics.Color
import android.os.Bundle
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.LinearLayout
import android.widget.TextView
import com.apollohg.editor.EditorEditText
import com.apollohg.editor.EditorTheme
import com.apollohg.editor.EditorV2Adapter
import com.apollohg.editor.RichTextEditorView
import com.apollohg.editor.createPairedV2TestEditor
import com.apollohg.editor.releasePairedV2TestEditor

class EditorSurfaceActivity : Activity() {
    internal lateinit var richTextView: RichTextEditorView
    internal val editor: EditorEditText get() = richTextView.editorEditText
    internal lateinit var adapter: EditorV2Adapter
    private var token = 0L

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
        val pair = createPairedV2TestEditor()
        adapter = pair.first
        token = pair.second
        adapter.setContentHtml("<p>Type here with the keyboard. This paragraph has different padding on its left and right edges.</p><blockquote><p>Nested block boxes share the text layout used for selection and caret placement.</p></blockquote><pre><code>const answer = 42;\nconsole.log(answer);</code></pre>" + "<p>Scroll down, select a word, and continue typing in the production editor.</p>".repeat(10))
        richTextView = RichTextEditorView(this).apply {
            applyTheme(EditorTheme.fromJson("""{"version":1,"styles":{"text":{"fontSize":18,"color":"#1d303aff"},"content":{"padding":12,"backgroundColor":"#ffffffff"},"paragraph":{"paddingLeft":16,"paddingRight":64,"paddingTop":8,"paddingBottom":8,"marginBottom":8,"backgroundColor":"#eff5f8ff"},"blockquote":{"paddingLeft":20,"paddingRight":8,"borderLeftWidth":4,"borderLeftColor":"#276b80ff","backgroundColor":"#e6f0f4ff"},"codeBlock":{"padding":16,"backgroundColor":"#e9edf1ff","borderRadius":8}}}"""))
            editorId = token
        }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            fitsSystemWindows = true
            setBackgroundColor(Color.WHITE)
            addView(TextView(this@EditorSurfaceActivity).apply {
                text = "Production Android editor"
                textSize = 21f
                setPadding(24, 24, 24, 16)
            })
            addView(richTextView, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        })
    }

    override fun onDestroy() {
        richTextView.editorId = 0
        releasePairedV2TestEditor(token)
        super.onDestroy()
    }
}
