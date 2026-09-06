package com.apollohg.editor
import android.graphics.Bitmap
import android.os.Looper
import java.io.ByteArrayInputStream
import java.io.File
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.CopyOnWriteArrayList
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

internal abstract class RenderImageLoaderPolicyTestFixture {
    protected fun securityFixtures(): JSONObject {
        val configuredPath: String = System.getenv("SECURITY_FIXTURE_PATH") ?: ""
        val configured = configuredPath.takeIf { it.isNotEmpty() }?.let { File(it) }
        val workingDirectory = requireNotNull(System.getProperty("user.dir"))
        val fixture = configured ?: generateSequence(File(workingDirectory)) {
            it.parentFile
        }.map { File(it, "scripts/tests/security-contract-fixtures.json") }
            .first { it.isFile }
        return JSONObject(fixture.readText())
    }

    @After
    fun tearDown() {
        RenderImageLoader.resetForTesting()
        RenderImageDecoder.resetForTesting()
    }

    protected fun drainMainUntil(latch: CountDownLatch) {
        repeat(100) {
            shadowOf(Looper.getMainLooper()).idle()
            if (latch.count > 0) Thread.sleep(10)
        }
    }

    protected class FakeConnection(
        url: URL,
        private val bytes: ByteArray = byteArrayOf(),
        private val status: Int = 200,
        private val declaredLength: Long = bytes.size.toLong(),
        private val stream: InputStream = ByteArrayInputStream(bytes),
        private val throwOnDisconnect: Boolean = false
    ) : HttpURLConnection(url) {
        var disconnected = false
        override fun getResponseCode(): Int = status
        override fun getContentLengthLong(): Long = declaredLength
        override fun getInputStream() = stream
        override fun disconnect() {
            disconnected = true
            if (throwOnDisconnect) error("disconnect failure")
        }
        override fun usingProxy(): Boolean = false
        override fun connect() = Unit
    }

    protected class BlockingInputStream : InputStream() {
        val readStarted = CountDownLatch(1)
        val closed = CountDownLatch(1)
        override fun read(): Int {
            readStarted.countDown()
            closed.await(2, TimeUnit.SECONDS)
            return -1
        }
        override fun close() {
            closed.countDown()
        }
    }

    protected class FakeMonotonicClock : MonotonicClock {
        private var nowMs = 0L
        override fun elapsedRealtime(): Long = nowMs
        fun advance(milliseconds: Long) {
            nowMs += milliseconds
        }
    }

    protected class TrickleInputStream(
        private val clock: FakeMonotonicClock,
        private val byteEveryMs: Long
    ) : InputStream() {
        override fun read(): Int {
            clock.advance(byteEveryMs)
            return 1
        }

        override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
            buffer[offset] = read().toByte()
            return 1
        }
    }
}
