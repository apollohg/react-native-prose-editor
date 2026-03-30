package com.apollohg.editor

import org.junit.Assert.assertEquals
import org.junit.Test

class EditorInputConnectionTest {

    @Test
    fun `code point delete length matches ascii backspace`() {
        val text = "Hello"
        val cursor = 5

        val beforeUtf16Length = EditorInputConnection.codePointsToUtf16Length(
            text = text,
            fromUtf16Offset = cursor,
            codePointCount = 1,
            forward = false
        )

        assertEquals(1, beforeUtf16Length)
    }

    @Test
    fun `code point delete length counts surrogate pair as two utf16 code units`() {
        val text = "A😀B"
        val cursor = 3

        val beforeUtf16Length = EditorInputConnection.codePointsToUtf16Length(
            text = text,
            fromUtf16Offset = cursor,
            codePointCount = 1,
            forward = false
        )

        assertEquals(2, beforeUtf16Length)
    }

    @Test
    fun `code point forward delete length counts surrogate pair as two utf16 code units`() {
        val text = "A😀B"
        val cursor = 1

        val afterUtf16Length = EditorInputConnection.codePointsToUtf16Length(
            text = text,
            fromUtf16Offset = cursor,
            codePointCount = 1,
            forward = true
        )

        assertEquals(2, afterUtf16Length)
    }
}
