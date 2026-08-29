package com.apollohg.editor

import android.text.NoCopySpan
import android.text.Selection
import android.text.Spannable
import android.text.SpannableString
import android.text.Spanned
import android.text.style.AbsoluteSizeSpan
import android.view.inputmethod.BaseInputConnection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class ImeTextCoordinateMapperTest {
    @Test
    fun `maps rendered placeholders out of IME visible UTF 16 coordinates`() {
        val mapper = ImeTextCoordinateMapper.build("\u200Ba\uD83D\uDE00\u200Bb", generation = 7)

        assertEquals(7L, mapper.generation)
        assertEquals("a\uD83D\uDE00b", mapper.visibleText.toString())
        assertEquals(0, mapper.rawToIme(1))
        assertEquals(3, mapper.rawToIme(4))
        assertEquals(4, mapper.rawToIme(6))
        assertEquals(4, mapper.imeToRaw(3, ImeTextCoordinateMapper.Affinity.BEFORE))
        assertEquals(5, mapper.imeToRaw(3, ImeTextCoordinateMapper.Affinity.AFTER))
    }

    @Test
    fun `affinity selects either side of consecutive invisible placeholders`() {
        val mapper = ImeTextCoordinateMapper.build("a\u200B\u200Bb", generation = 2)

        assertEquals("ab", mapper.visibleText.toString())
        assertEquals(1, mapper.imeToRaw(1, ImeTextCoordinateMapper.Affinity.BEFORE))
        assertEquals(3, mapper.imeToRaw(1, ImeTextCoordinateMapper.Affinity.AFTER))
        assertEquals(0, mapper.imeToRaw(-20, ImeTextCoordinateMapper.Affinity.BEFORE))
        assertEquals(4, mapper.imeToRaw(20, ImeTextCoordinateMapper.Affinity.AFTER))
    }

    @Test
    fun `trailing invisible placeholders retain both end affinities`() {
        val mapper = ImeTextCoordinateMapper.build("a\u200B", generation = 6)

        assertEquals("a", mapper.visibleText.toString())
        assertEquals(1, mapper.imeToRaw(1, ImeTextCoordinateMapper.Affinity.BEFORE))
        assertEquals(2, mapper.imeToRaw(1, ImeTextCoordinateMapper.Affinity.AFTER))
    }

    @Test
    fun `visible text retains spans across removed placeholders`() {
        val raw = SpannableString("A\u200BB").apply {
            setSpan(AbsoluteSizeSpan(28), 0, length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }

        val visible = ImeTextCoordinateMapper.build(raw, generation = 3).visibleText

        assertTrue(visible is Spanned)
        val span = (visible as Spanned).getSpans(0, visible.length, AbsoluteSizeSpan::class.java).single()
        assertEquals(0, visible.getSpanStart(span))
        assertEquals(2, visible.getSpanEnd(span))
    }

    @Test
    fun `document object replacements remain visible to the IME`() {
        val mapper = ImeTextCoordinateMapper.build("a\uFFFCb", generation = 4)

        assertEquals("a\uFFFCb", mapper.visibleText.toString())
        assertEquals(2, mapper.rawToIme(2))
    }

    @Test
    fun `IME snapshots exclude Android selection and composing markers`() {
        val raw = SpannableString("abc")
        val watcherMarker = NoCopySpan.Concrete()
        Selection.setSelection(raw, 1, 2)
        BaseInputConnection.setComposingSpans(raw)
        raw.setSpan(watcherMarker, 0, raw.length, Spanned.SPAN_INCLUSIVE_INCLUSIVE)

        val visible = ImeTextCoordinateMapper.build(raw, generation = 5).visibleText

        assertEquals(-1, Selection.getSelectionStart(visible))
        assertEquals(-1, Selection.getSelectionEnd(visible))
        assertEquals(-1, BaseInputConnection.getComposingSpanStart(visible as Spannable))
        assertEquals(-1, BaseInputConnection.getComposingSpanEnd(visible))
        assertEquals(-1, visible.getSpanStart(watcherMarker))
    }
}
