package com.apollohg.editor.prototype

import android.app.Activity
import android.graphics.Color
import android.os.Bundle
import android.view.ViewGroup
import android.view.WindowManager
import android.view.inputmethod.BaseInputConnection
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

class PrototypeEditorActivity : Activity() {
    internal lateinit var session: PrototypeDocumentSession
    internal lateinit var editor: PrototypeEditorView
    internal lateinit var scroller: ScrollView
    internal lateinit var atom: Button
    private lateinit var status: TextView
    private var expandedAtom = false
    private var atomTaps = 0
    private val Int.dp get() = (this * resources.displayMetrics.density).toInt()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_RESIZE)
        session = PrototypeDocumentSession(listOf(
            "This paragraph has 16dp of left padding and 64dp on the right. Type here and watch the text wrap within its own guides.",
            "This paragraph has 48dp of left padding and 16dp on the right. Long-press text, then drag to select across both paragraphs. The native atom between them belongs to this same scrolling content. Try resizing it, scrolling, and typing above it. " +
                "Keep typing here to add lines. The text uses its own available width, while cursor positions and touch selection come from the exact same measured lines. ".repeat(8),
        ))
        editor = PrototypeEditorView(this, session)
        atom = Button(this).apply {
            text = "Native atom · tap to count: 0"
            isAllCaps = false
            setBackgroundColor(Color.rgb(224, 235, 214))
            setOnClickListener { text = "Native atom · tap to count: ${++atomTaps}" }
        }
        editor.mountAtom(atom, 96.dp)
        status = TextView(this).apply {
            textSize = 12f
            setPadding(12.dp, 4.dp, 12.dp, 4.dp)
            setTextColor(Color.DKGRAY)
            maxLines = 4
        }
        val toolbar = LinearLayout(this).apply {
            addView(Button(this@PrototypeEditorActivity).apply {
                text = "Select across"
                isAllCaps = false
                setOnClickListener {
                    val boundary = session.editable.indexOf('\n').takeIf { it >= 0 } ?: 0
                    editor.requestFocus()
                    session.setSelection((boundary - 12).coerceAtLeast(0), (boundary + 20).coerceAtMost(session.editable.length))
                }
            }, LinearLayout.LayoutParams(0, 48.dp, 1f))
            addView(Button(this@PrototypeEditorActivity).apply {
                text = "Atom size"
                isAllCaps = false
                setOnClickListener {
                    expandedAtom = !expandedAtom
                    resizeAtom(if (expandedAtom) 160.dp else 72.dp)
                }
            }, LinearLayout.LayoutParams(0, 48.dp, 1f))
            addView(Button(this@PrototypeEditorActivity).apply {
                text = "Reset"
                isAllCaps = false
                setOnClickListener { recreate() }
            }, LinearLayout.LayoutParams(0, 48.dp, 0.7f))
        }
        scroller = ScrollView(this).apply {
            isFillViewport = false
            addView(editor, ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT))
            setOnScrollChangeListener { _, _, _, _, _ -> editor.onViewportChanged() }
        }
        setContentView(LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.WHITE)
            fitsSystemWindows = true
            addView(TextView(this@PrototypeEditorActivity).apply {
                text = "Android layout prototype"
                textSize = 22f
                setTextColor(Color.rgb(20, 70, 79))
                setPadding(12.dp, 16.dp, 12.dp, 6.dp)
            })
            addView(TextView(this@PrototypeEditorActivity).apply {
                text = "One text buffer · independent block widths · native child"
                textSize = 13f
                setPadding(12.dp, 0, 12.dp, 8.dp)
            })
            addView(toolbar)
            addView(status)
            addView(scroller, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        })
        editor.onStateChanged = {
            updateStatus()
            editor.post { if (editor.hasFocus()) editor.revealCaret() }
        }
        updateStatus()
    }

    internal fun resizeAtom(heightPx: Int) {
        val anchor = editor.documentLayout.offsetAt(0f, scroller.scrollY.toFloat())
        val anchorY = editor.documentLayout.caret(anchor).top
        val oldScroll = scroller.scrollY
        editor.mountAtom(atom, heightPx)
        editor.post {
            val delta = editor.documentLayout.caret(anchor).top - anchorY
            scroller.scrollTo(0, oldScroll + delta.toInt())
        }
    }

    private fun updateStatus() {
        val start = BaseInputConnection.getComposingSpanStart(session.editable)
        val end = BaseInputConnection.getComposingSpanEnd(session.editable)
        val composition = if (start >= 0 && end >= start) session.editable.subSequence(start, end).toString() else "none"
        status.text = "Selection ${session.selectionStart}…${session.selectionEnd} · composing: $composition\n" +
            "Rust: ${session.committedText.length} UTF16 units · displayed: ${session.editable.length}\n" +
            if (session.committedText == session.editable.toString()) "Committed text matches display" else "Composition is transient; Rust retains committed text"
    }

    override fun onDestroy() {
        editor.onStateChanged = null
        session.onChange = null
        session.close()
        super.onDestroy()
    }
}
