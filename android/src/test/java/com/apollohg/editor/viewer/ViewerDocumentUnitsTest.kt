package com.apollohg.editor.viewer

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class ViewerDocumentUnitsTest {
    @Test
    fun `optional string separates an absent key, a JSON null, and the literal text null`() {
        val json = JSONObject("""{"present":"task","explicitNull":null,"literal":"null"}""")

        assertEquals("task", json.optionalString("present"))
        assertNull("an absent key must not produce a value", json.optionalString("absent"))
        // optString(key, null) routes JSONObject.NULL through String.valueOf,
        // which yields the four-character text "null" instead of no value.
        assertNull("a present JSON null must not coerce to text", json.optionalString("explicitNull"))
        // A quoted "null" is real content, so it must survive that same guard.
        assertEquals("null", json.optionalString("literal"))
    }

    @Test
    fun `list context kind reads through the same absent and JSON null guard`() {
        assertEquals("task", listContext("""{"ordered":false,"kind":"task"}""")?.kind)
        assertNull("an absent kind must leave the list unclassified", listContext("""{"ordered":false}""")?.kind)
        assertNull("a JSON null kind must not classify the list as \"null\"", listContext("""{"ordered":false,"kind":null}""")?.kind)
    }
}
