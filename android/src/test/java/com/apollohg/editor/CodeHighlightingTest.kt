package com.apollohg.editor

import org.junit.Assert.*
import org.junit.Test

class CodeHighlightingTest {
    @Test fun validatesUtf16RangesBeforeApplyingProviderOutput() {
        val text = "a😀b"
        assertTrue(CodeHighlightingRegistry.validRanges(text, listOf(CodeHighlightRange(1, 2, 0x112233ff, 1))))
        assertFalse(CodeHighlightingRegistry.validRanges(text, listOf(CodeHighlightRange(1, 1, 0x112233ff, 0))))
        assertFalse(CodeHighlightingRegistry.validRanges(text, listOf(CodeHighlightRange(3, 2, 0x112233ff, 0))))
        assertFalse(CodeHighlightingRegistry.validRanges(text, listOf(CodeHighlightRange(0, 3, 0x112233ff, 0), CodeHighlightRange(1, 2, 0x112233ff, 0))))
    }

    @Test fun rejectsIncompatibleProvidersAndResolvesRegisteredProvider() {
        val provider = object : CodeHighlightingProvider {
            override val id = "test-provider"
            override val version = 1
            override fun highlight(text: String, language: String?, theme: String) = emptyList<CodeHighlightRange>()
        }
        CodeHighlightingRegistry.register(provider)
        assertSame(provider, CodeHighlightingRegistry.provider("test-provider"))
        assertThrows(IllegalArgumentException::class.java) { CodeHighlightingRegistry.provider("absent-provider") }
        assertThrows(IllegalArgumentException::class.java) {
            CodeHighlightingRegistry.register(object : CodeHighlightingProvider by provider { override val version = 2 })
        }
    }
}
