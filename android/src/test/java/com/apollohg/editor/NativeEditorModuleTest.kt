package com.apollohg.editor

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativeEditorModuleTest {
    @Test
    fun `native unsigned argument helpers reject negative values`() {
        assertNull(nativeULong(-1))
        assertNull(nativeUInt(-1))
    }

    @Test
    fun `native unsigned argument helpers keep non-negative values`() {
        assertEquals(0UL, nativeULong(0))
        assertEquals(42UL, nativeULong(42))
        assertEquals(0U, nativeUInt(0))
        assertEquals(42U, nativeUInt(42))
    }

    @Test
    fun `native argument error returns bridge parseable json`() {
        assertEquals("{\"error\":\"invalid editor id\"}", nativeArgumentError("editor id"))
        assertEquals("{\"error\":\"invalid position\"}", nativeArgumentError("position"))
    }

    @Test
    fun `structured editor creation accepts only a positive integral id`() {
        assertEquals(42UL, createdEditorId("{\"editorId\":42}"))
        assertNull(createdEditorId("{\"error\":{\"code\":\"CONFIG_INVALID\"}}"))
        assertNull(createdEditorId("{\"editorId\":0}"))
        assertNull(createdEditorId("{\"editorId\":1.5}"))
        assertNull(createdEditorId("{\"editorId\":true}"))
        assertNull(createdEditorId("not json"))
    }

    @Test
    fun `structured editor creation registers only successful ids and preserves envelope`() {
        val marked = mutableListOf<Long>()
        val success = "{\"editorId\":7}"
        val failure = "{\"error\":{\"code\":\"SCHEMA_INVALID\"}}"

        assertEquals(success, registerCreatedEditorResult(success, marked::add))
        assertEquals(failure, registerCreatedEditorResult(failure, marked::add))
        assertEquals(listOf(7L), marked)
    }
}
