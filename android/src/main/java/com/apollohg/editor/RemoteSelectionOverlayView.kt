package com.apollohg.editor

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.util.AttributeSet
import android.util.TypedValue
import android.view.View
import androidx.appcompat.content.res.AppCompatResources
import com.google.android.material.R as MaterialR
import org.json.JSONArray
import uniffi.editor_core.editorDocToScalar

data class RemoteSelectionDecoration(
    val clientId: Int,
    val anchor: Int,
    val head: Int,
    val color: Int,
    val name: String?,
    val isFocused: Boolean,
) {
    companion object {
        fun fromJson(context: Context, json: String?): List<RemoteSelectionDecoration> {
            if (json.isNullOrBlank()) return emptyList()
            val array = try {
                JSONArray(json)
            } catch (_: Throwable) {
                return emptyList()
            }
            val fallbackColor = resolveFallbackColor(context)

            return buildList {
                for (index in 0 until array.length()) {
                    val item = array.optJSONObject(index) ?: continue
                    val color = parseColor(item.optString("color", ""), fallbackColor)
                    add(
                        RemoteSelectionDecoration(
                            clientId = item.optInt("clientId", 0),
                            anchor = item.optInt("anchor", 0),
                            head = item.optInt("head", 0),
                            color = color,
                            name = item.optString("name").takeIf { it.isNotBlank() },
                            isFocused = item.optBoolean("isFocused", false),
                        )
                    )
                }
            }
        }

        private fun parseColor(raw: String, fallbackColor: Int): Int {
            return try {
                Color.parseColor(raw)
            } catch (_: Throwable) {
                fallbackColor
            }
        }

        private fun resolveFallbackColor(context: Context): Int {
            val typedValue = TypedValue()
            val attrs = intArrayOf(
                MaterialR.attr.colorPrimary,
                MaterialR.attr.colorSecondary,
                android.R.attr.colorAccent,
                android.R.attr.textColorPrimary
            )
            for (attr in attrs) {
                if (!context.theme.resolveAttribute(attr, typedValue, true)) {
                    continue
                }
                if (typedValue.resourceId != 0) {
                    AppCompatResources.getColorStateList(context, typedValue.resourceId)
                        ?.defaultColor
                        ?.let { return it }
                } else if (typedValue.type in TypedValue.TYPE_FIRST_COLOR_INT..TypedValue.TYPE_LAST_COLOR_INT) {
                    return typedValue.data
                }
            }
            return Color.TRANSPARENT
        }
    }
}

data class RemoteSelectionDebugSnapshot(
    val clientId: Int,
    val caretRect: RectF?,
)

class RemoteSelectionOverlayView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0,
) : View(context, attrs, defStyleAttr) {

    private var editorView: RichTextEditorView? = null
    private var remoteSelections: List<RemoteSelectionDecoration> = emptyList()
    internal var editorIdOverrideForTesting: Long? = null
    internal var docToScalarResolver: (Long, Int) -> Int = { editorId, docPos ->
        editorDocToScalar(editorId.toULong(), docPos.toUInt()).toInt()
    }
    private val selectionPath = Path()
    private val selectionPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.FILL
    }
    private val caretPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.FILL
    }

    init {
        setWillNotDraw(false)
        isClickable = false
        isFocusable = false
    }

    fun bind(editorView: RichTextEditorView) {
        this.editorView = editorView
    }

    fun setRemoteSelections(selections: List<RemoteSelectionDecoration>) {
        remoteSelections = selections
        invalidate()
    }

    fun debugSnapshotsForTesting(): List<RemoteSelectionDebugSnapshot> {
        val editorView = editorView ?: return emptyList()
        val editorId = resolvedEditorId(editorView)
        if (editorId == 0L || remoteSelections.isEmpty()) return emptyList()

        val editText = editorView.editorEditText
        val layout = editText.layout ?: return emptyList()
        val text = editText.text?.toString() ?: return emptyList()
        val baseX = (editorView.editorViewport.left + editorView.editorScrollView.left + editText.left).toFloat() +
            editText.compoundPaddingLeft
        val baseY = (editorView.editorViewport.top + editorView.editorScrollView.top + editText.top).toFloat() +
            editText.compoundPaddingTop - editorView.editorScrollView.scrollY
        val caretWidth = maxOf(2f, 2f * resources.displayMetrics.density / 2f)

        return remoteSelections.map { selection ->
            RemoteSelectionDebugSnapshot(
                clientId = selection.clientId,
                caretRect = caretRectForSelection(
                    selection = selection,
                    editorId = editorId,
                    text = text,
                    layout = layout,
                    baseX = baseX,
                    baseY = baseY,
                    caretWidth = caretWidth,
                ),
            )
        }
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)

        val editorView = editorView ?: return
        val editorId = resolvedEditorId(editorView)
        if (editorId == 0L || remoteSelections.isEmpty()) return

        val editText = editorView.editorEditText
        val layout = editText.layout ?: return
        val text = editText.text?.toString() ?: return
        val baseX = (editorView.editorViewport.left + editorView.editorScrollView.left + editText.left).toFloat() +
            editText.compoundPaddingLeft
        val baseY = (editorView.editorViewport.top + editorView.editorScrollView.top + editText.top).toFloat() +
            editText.compoundPaddingTop - editorView.editorScrollView.scrollY
        val caretWidth = maxOf(2f, 2f * resources.displayMetrics.density / 2f)

        for (selection in remoteSelections) {
            val startDoc = minOf(selection.anchor, selection.head)
            val endDoc = maxOf(selection.anchor, selection.head)
            val startScalar = docToScalarResolver(editorId, startDoc)
            val endScalar = docToScalarResolver(editorId, endDoc)
            val startUtf16 = PositionBridge.scalarToUtf16(startScalar, text).coerceIn(0, text.length)
            val endUtf16 = PositionBridge.scalarToUtf16(endScalar, text).coerceIn(0, text.length)

            selectionPaint.color = withAlpha(selection.color, 0.18f)
            caretPaint.color = selection.color

            if (startUtf16 != endUtf16) {
                selectionPath.reset()
                layout.getSelectionPath(startUtf16, endUtf16, selectionPath)
                canvas.save()
                canvas.translate(baseX, baseY)
                canvas.drawPath(selectionPath, selectionPaint)
                canvas.restore()
            }

            if (!selection.isFocused) {
                continue
            }

            val caretRect = caretRectForSelection(
                selection = selection,
                editorId = editorId,
                text = text,
                layout = layout,
                baseX = baseX,
                baseY = baseY,
                caretWidth = caretWidth,
            ) ?: continue
            canvas.drawRoundRect(
                caretRect.left,
                caretRect.top,
                caretRect.right,
                caretRect.bottom,
                caretWidth / 2f,
                caretWidth / 2f,
                caretPaint
            )
        }
    }

    private fun caretRectForSelection(
        selection: RemoteSelectionDecoration,
        editorId: Long,
        text: String,
        layout: android.text.Layout,
        baseX: Float,
        baseY: Float,
        caretWidth: Float,
    ): RectF? {
        if (!selection.isFocused) return null

        val endDoc = maxOf(selection.anchor, selection.head)
        val endScalar = docToScalarResolver(editorId, endDoc)
        val endUtf16 = PositionBridge.scalarToUtf16(endScalar, text).coerceIn(0, text.length)
        val line = layout.getLineForOffset(endUtf16.coerceAtMost(maxOf(text.length - 1, 0)))
        val horizontal = layout.getPrimaryHorizontal(endUtf16)
        val caretLeft = baseX + horizontal
        val caretTop = baseY + layout.getLineTop(line)
        val caretBottom = baseY + layout.getLineBottom(line)
        return RectF(caretLeft, caretTop, caretLeft + caretWidth, caretBottom)
    }

    private fun withAlpha(color: Int, alphaFraction: Float): Int {
        val alpha = (255f * alphaFraction).toInt().coerceIn(0, 255)
        return Color.argb(alpha, Color.red(color), Color.green(color), Color.blue(color))
    }

    private fun resolvedEditorId(editorView: RichTextEditorView): Long =
        editorIdOverrideForTesting ?: editorView.editorId
}
