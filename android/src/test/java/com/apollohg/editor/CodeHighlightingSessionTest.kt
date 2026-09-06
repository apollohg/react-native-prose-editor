package com.apollohg.editor

import android.os.Looper
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.LooperMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [29])
@LooperMode(LooperMode.Mode.PAUSED)
class CodeHighlightingSessionTest {
    @Test fun yieldsToAnotherSessionBeforeProcessingReplacement() {
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val parsed = CountDownLatch(3)
        val calls = Collections.synchronizedList(mutableListOf<String>())
        val provider = object : CodeHighlightingProvider {
            override val id = "session-fairness-provider"
            override val version = 1
            override fun highlight(text: String, language: String?, theme: String): List<CodeHighlightRange> {
                assertNotEquals(Looper.getMainLooper(), Looper.myLooper())
                calls.add(theme)
                if (theme == "blocked") {
                    entered.countDown()
                    assertTrue(release.await(3, TimeUnit.SECONDS))
                }
                parsed.countDown()
                return emptyList()
            }
        }
        CodeHighlightingRegistry.register(provider)
        val first = CodeHighlightingSession()
        val second = CodeHighlightingSession()
        val blocks = listOf(CodeHighlightBlock(0, "code", null))
        val delivered = mutableListOf<String>()
        first.update(provider.id, "blocked", blocks) { fail("Stale request was delivered") }
        assertTrue(entered.await(2, TimeUnit.SECONDS))
        second.update(provider.id, "other", blocks) { delivered.add("other") }
        first.update(provider.id, "replacement", blocks) { delivered.add("replacement") }
        release.countDown()
        assertTrue(parsed.await(3, TimeUnit.SECONDS))
        assertEquals(listOf("blocked", "other", "replacement"), calls.toList())
        first.cancel()
        second.cancel()
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(delivered.isEmpty())
    }
}
