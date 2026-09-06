package com.apollohg.editor.prototype

import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.text.Layout
import android.text.SpannedString
import android.text.StaticLayout
import android.text.TextDirectionHeuristics
import android.text.TextPaint
import com.apollohg.editor.PositionBridge

internal data class PrototypeInsets(val left: Int, val right: Int, val top: Int = 12, val bottom: Int = 12)

internal class PrototypeBlockLayout(
    text: CharSequence,
    private val width: Int,
    paint: TextPaint,
    private val afterBlockHeight: (Int) -> Int = { 0 },
    insets: (Int) -> PrototypeInsets,
) {
    private data class Block(val start: Int, val end: Int, val x: Int, val y: Int, val textBottom: Int, val bottom: Int, val layout: StaticLayout)
    private val content = SpannedString(text)
    private val plainText = content.toString()
    private val blocks: List<Block>
    val height: Int

    init {
        val result = mutableListOf<Block>()
        var start = 0
        var y = 0
        do {
            val end = plainText.indexOf('\n', start).let { if (it < 0) content.length else it }
            val edges = insets(result.size)
            val paragraph = content.subSequence(start, end)
            val layout = StaticLayout.Builder.obtain(paragraph, 0, paragraph.length, TextPaint(paint), (width - edges.left - edges.right).coerceAtLeast(1))
                .setIncludePad(false)
                .setAlignment(Layout.Alignment.ALIGN_NORMAL)
                .setTextDirection(TextDirectionHeuristics.FIRSTSTRONG_LTR)
                .build()
            val textY = y + edges.top
            y = textY + layout.height + edges.bottom
            val textBottom = y
            y += afterBlockHeight(result.size)
            result += Block(start, end, edges.left, textY, textBottom, y, layout)
            start = end + 1
        } while (start <= content.length)
        blocks = result
        height = y
    }

    fun offsetAt(x: Float, y: Float): Int {
        val block = blocks.firstOrNull { y < it.bottom } ?: blocks.last()
        val line = block.layout.getLineForVertical((y - block.y).toInt())
        val local = block.layout.getOffsetForHorizontal(line, x - block.x)
        return PositionBridge.snapToGraphemeBoundary(block.start + local, plainText)
    }

    fun caret(offset: Int): RectF {
        val safe = PositionBridge.snapToScalarBoundary(offset, plainText, true)
        val block = blocks.firstOrNull { safe <= it.end } ?: blocks.last()
        val local = (safe - block.start).coerceIn(0, block.end - block.start)
        val line = block.layout.getLineForOffset(local)
        val x = block.x + block.layout.getPrimaryHorizontal(local)
        return RectF(x, (block.y + block.layout.getLineTop(line)).toFloat(), x + 2, (block.y + block.layout.getLineBottom(line)).toFloat())
    }

    fun moveVertically(offset: Int, down: Boolean): Int {
        val safe = PositionBridge.snapToScalarBoundary(offset, plainText, true)
        val index = blocks.indexOfFirst { safe <= it.end }.let { if (it < 0) blocks.lastIndex else it }
        val block = blocks[index]
        val local = (safe - block.start).coerceIn(0, block.end - block.start)
        val line = block.layout.getLineForOffset(local)
        val nextLine = line + if (down) 1 else -1
        val target = if (nextLine in 0 until block.layout.lineCount) block else blocks.getOrNull(index + if (down) 1 else -1) ?: return safe
        val targetLine = if (target === block) nextLine else if (down) 0 else target.layout.lineCount - 1
        val x = caret(safe).left
        val result = target.start + target.layout.getOffsetForHorizontal(targetLine, x - target.x)
        return PositionBridge.snapToGraphemeBoundary(result, plainText)
    }

    fun selection(anchor: Int, head: Int): Path {
        val (start, end) = PositionBridge.snapRangeToScalarBoundaries(anchor, head, plainText)
        val path = Path()
        if (start == end) return path
        for (block in blocks) {
            if (start > block.end || end <= block.start) continue
            val localStart = (start - block.start).coerceIn(0, block.end - block.start)
            val localEnd = (end - block.start).coerceIn(0, block.end - block.start)
            val localPath = Path()
            block.layout.getSelectionPath(localStart, localEnd, localPath)
            localPath.offset(block.x.toFloat(), block.y.toFloat())
            path.addPath(localPath)
            if (end > block.end && block.end < content.length) {
                val cursor = caret(block.end)
                val rtl = block.layout.getParagraphDirection(block.layout.lineCount - 1) < 0
                val edge = if (rtl) block.x.toFloat() else (block.x + block.layout.width).toFloat()
                path.addRect(minOf(cursor.left, edge), cursor.top, maxOf(cursor.left + 2, edge), cursor.bottom, Path.Direction.CW)
            }
        }
        return path
    }

    fun afterBlockBounds(index: Int): RectF {
        val block = blocks[index]
        return RectF(block.x.toFloat(), block.textBottom.toFloat(), (block.x + block.layout.width).toFloat(), block.bottom.toFloat())
    }

    fun draw(canvas: Canvas) {
        val background = Paint().apply { color = Color.rgb(236, 244, 246) }
        val guide = Paint().apply { color = Color.rgb(113, 159, 167); strokeWidth = 1f }
        for (block in blocks) {
            val top = block.y.toFloat()
            val bottom = top + block.layout.height
            canvas.drawRect(0f, top, width.toFloat(), bottom, background)
            canvas.drawLine(block.x.toFloat(), top, block.x.toFloat(), bottom, guide)
            canvas.drawLine((block.x + block.layout.width).toFloat(), top, (block.x + block.layout.width).toFloat(), bottom, guide)
            canvas.save()
            canvas.translate(block.x.toFloat(), top)
            block.layout.draw(canvas)
            canvas.restore()
        }
    }
}
