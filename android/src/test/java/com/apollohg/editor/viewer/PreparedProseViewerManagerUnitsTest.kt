package com.apollohg.editor.viewer

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PreparedProseViewerManagerUnitsTest {
    @Test
    fun `fabric constraints remain physical pixels until yoga output`() {
        val density = 2.625f
        val widthPixels = 891f

        assertEquals(891, fabricConstraintPixels(widthPixels))
        assertEquals(339.42856f, fabricPixelsToDp(widthPixels, density)!!, 0.0001f)
        assertEquals(200f, fabricPixelsToDp(525f, density)!!, 0.0001f)
        assertNull(fabricConstraintPixels(Float.POSITIVE_INFINITY))
        assertNull(fabricPixelsToDp(100f, 0f))
    }
}
