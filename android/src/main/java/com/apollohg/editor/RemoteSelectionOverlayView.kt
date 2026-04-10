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
    private data class CachedSelectionGeometry(
        val clientId: Int,
        val selectionPath: Path?,
        val selectionColor: Int,
        val caretRect: RectF?,
        val caretColor: Int,
    )

    private data class GeometrySnapshot(
        val editorId: Long,
        val text: String,
        val layoutWidth: Int,
        val layoutHeight: Int,
        val baseX: Int,
        val baseY: Int,
        val width: Int,
        val height: Int,
        val selections: List<RemoteSelectionDecoration>,
    )

    private data class GeometryContext(
        val snapshot: GeometrySnapshot,
        val layout: android.text.Layout,
        val caretWidth: Float,
    )

    private var editorView: RichTextEditorView? = null
    private var remoteSelections: List<RemoteSelectionDecoration> = emptyList()
    private var cachedSnapshot: GeometrySnapshot? = null
    private var cachedGeometry: List<CachedSelectionGeometry> = emptyList()
    internal var editorIdOverrideForTesting: Long? = null
    internal var docToScalarResolver: (Long, Int) -> Int = { editorId, docPos ->
        editorDocToScalar(editorId.toULong(), docPos.toUInt()).toInt()
    }
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
        invalidateGeometry()
    }

    fun setRemoteSelections(selections: List<RemoteSelectionDecoration>) {
        if (remoteSelections == selections) {
            return
        }
        remoteSelections = selections
        invalidateGeometry()
        refreshGeometry()
    }

    fun refreshGeometry() {
        ensureGeometry()
        invalidate()
    }

    fun hasSelectionsOrCachedGeometry(): Boolean {
        return remoteSelections.isNotEmpty() || cachedGeometry.isNotEmpty()
    }

    fun debugSnapshotsForTesting(): List<RemoteSelectionDebugSnapshot> {
        return ensureGeometry().map { geometry ->
            RemoteSelectionDebugSnapshot(
                clientId = geometry.clientId,
                caretRect = geometry.caretRect?.let(::RectF),
            )
        }
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val geometry = ensureGeometry()
        if (geometry.isEmpty()) return

        for (entry in geometry) {
            entry.selectionPath?.let { path ->
                selectionPaint.color = entry.selectionColor
                canvas.drawPath(path, selectionPaint)
            }

            entry.caretRect?.let { caretRect ->
                caretPaint.color = entry.caretColor
                val cornerRadius = maxOf(1f, caretRect.width() / 2f)
                canvas.drawRoundRect(
                    caretRect.left,
                    caretRect.top,
                    caretRect.right,
                    caretRect.bottom,
                    cornerRadius,
                    cornerRadius,
                    caretPaint
                )
            }
        }
    }

    private fun ensureGeometry(): List<CachedSelectionGeometry> {
        val context = buildGeometryContext() ?: run {
            cachedSnapshot = null
            cachedGeometry = emptyList()
            return emptyList()
        }
        if (cachedSnapshot == context.snapshot) {
            return cachedGeometry
        }

        val text = context.snapshot.text
        val editorId = context.snapshot.editorId
        val geometry = remoteSelections.map { selection ->
            val startDoc = minOf(selection.anchor, selection.head)
            val endDoc = maxOf(selection.anchor, selection.head)
            val startScalar = docToScalarResolver(editorId, startDoc)
            val endScalar = docToScalarResolver(editorId, endDoc)
            val startUtf16 = PositionBridge.scalarToUtf16(startScalar, text).coerceIn(0, text.length)
            val endUtf16 = PositionBridge.scalarToUtf16(endScalar, text).coerceIn(0, text.length)

            val selectionPath = if (startUtf16 != endUtf16) {
                Path().apply {
                    context.layout.getSelectionPath(startUtf16, endUtf16, this)
                    offset(context.snapshot.baseX.toFloat(), context.snapshot.baseY.toFloat())
                }
            } else {
                null
            }

            CachedSelectionGeometry(
                clientId = selection.clientId,
                selectionPath = selectionPath,
                selectionColor = withAlpha(selection.color, 0.18f),
                caretRect = caretRectForOffset(
                    endUtf16 = endUtf16,
                    textLength = text.length,
                    layout = context.layout,
                    baseX = context.snapshot.baseX.toFloat(),
                    baseY = context.snapshot.baseY.toFloat(),
                    caretWidth = context.caretWidth,
                    isFocused = selection.isFocused,
                ),
                caretColor = selection.color,
            )
        }

        cachedSnapshot = context.snapshot
        cachedGeometry = geometry
        return geometry
    }

    private fun buildGeometryContext(): GeometryContext? {
        val editorView = editorView ?: return null
        val editorId = resolvedEditorId(editorView)
        if (editorId == 0L || remoteSelections.isEmpty()) return null

        val editText = editorView.editorEditText
        val layout = editText.layout ?: return null
        val text = editText.text?.toString() ?: return null
        val baseX = editorView.editorViewport.left + editorView.editorScrollView.left + editText.left +
            editText.compoundPaddingLeft
        val baseY = editorView.editorViewport.top + editorView.editorScrollView.top + editText.top +
            editText.compoundPaddingTop - editorView.editorScrollView.scrollY
        val caretWidth = maxOf(2f, resources.displayMetrics.density)

        return GeometryContext(
            snapshot = GeometrySnapshot(
                editorId = editorId,
                text = text,
                layoutWidth = layout.width,
                layoutHeight = layout.height,
                baseX = baseX,
                baseY = baseY,
                width = width,
                height = height,
                selections = remoteSelections,
            ),
            layout = layout,
            caretWidth = caretWidth,
        )
    }

    private fun invalidateGeometry() {
        cachedSnapshot = null
    }

    private fun caretRectForOffset(
        endUtf16: Int,
        textLength: Int,
        layout: android.text.Layout,
        baseX: Float,
        baseY: Float,
        caretWidth: Float,
        isFocused: Boolean,
    ): RectF? {
        if (!isFocused) return null

        val clampedOffset = endUtf16.coerceIn(0, textLength)
        val lineLookupOffset = clampedOffset.coerceAtMost(maxOf(textLength - 1, 0))
        val line = layout.getLineForOffset(lineLookupOffset)
        val horizontal = layout.getPrimaryHorizontal(clampedOffset)
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
