package com.apollohg.editor

import android.app.Activity
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Rect
import android.os.Bundle
import android.view.inputmethod.EditorInfo
import android.widget.FrameLayout
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import com.apollohg.editor.viewer.PreparedProseAccessibilityNode
import com.apollohg.editor.viewer.PreparedProseDrawingView
import com.apollohg.editor.viewer.PreparedProseInteraction
import com.apollohg.editor.viewer.PreparedProseLayout
import com.apollohg.editor.viewer.ProseLayoutKey

class AndroidApi24SmokeActivity : Activity() {
    private lateinit var editor: EditorEditText
    private lateinit var viewer: PreparedProseDrawingView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        editor = EditorEditText(this)
        viewer = PreparedProseDrawingView(this).apply {
            install(preparedLayout())
        }
        setContentView(FrameLayout(this).apply {
            addView(editor, FrameLayout.LayoutParams(320, 96))
            addView(
                viewer,
                FrameLayout.LayoutParams(200, 48).apply {
                    leftMargin = 12
                    topMargin = 108
                },
            )
        })
    }

    fun runApi24SmokeAssertions() {
        check(editor.requestFocus())
        val inputConnection = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
        check(inputConnection.commitText("x", 1))
        check(editor.text?.toString() == "x")
        check(inputConnection.deleteSurroundingText(1, 0))
        check(editor.text?.isEmpty() == true)
        editor.draw(Canvas(Bitmap.createBitmap(320, 96, Bitmap.Config.ARGB_8888)))

        check(viewer.width == 200)
        viewer.draw(Canvas(Bitmap.createBitmap(200, 48, Bitmap.Config.ARGB_8888)))
        val node = requireNotNull(viewer.accessibilityNodeProvider.createAccessibilityNodeInfo(1))
        check(AccessibilityNodeInfoCompat.wrap(node).isScreenReaderFocusable)
    }

    private fun preparedLayout() = PreparedProseLayout(
        key = ProseLayoutKey(
            semanticKey = "api-24-smoke",
            widthPx = 176,
            themeDigest = "fixture",
            nativeFontRevision = 0,
            fontEnvironmentRevision = 0,
            densityBits = resources.displayMetrics.density.toRawBits().toLong(),
            attachmentRevision = 0,
            generationIdentity = "api-24-smoke",
        ),
        widthPx = 176,
        heightPx = 48,
        blocks = emptyList(),
        interactions = listOf(
            PreparedProseInteraction(
                kind = PreparedProseInteraction.Kind.LINK,
                rects = listOf(Rect(12, 0, 80, 40)),
                href = "https://example.test",
                visibleText = "link",
                label = "link",
            ),
        ),
        accessibilityNodes = listOf(
            PreparedProseAccessibilityNode(
                interactionIndex = 0,
                role = PreparedProseAccessibilityNode.Role.LINK,
                label = "link",
                bounds = Rect(12, 0, 80, 40),
            ),
        ),
        retainedBytes = 0,
    )
}
