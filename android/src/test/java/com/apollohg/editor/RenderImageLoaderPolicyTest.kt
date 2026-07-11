package com.apollohg.editor

import android.graphics.Bitmap
import android.os.Looper
import java.io.ByteArrayInputStream
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
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

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class RenderImageLoaderPolicyTest {
    @After
    fun tearDown() {
        RenderImageLoader.resetForTesting()
        RenderImageDecoder.resetForTesting()
    }

    @Test
    fun `image policy defaults and invalid fields match public contract`() {
        val defaults = ImageLoadingPolicy.fromJson(null)
        assertEquals(10 * 1024 * 1024, defaults.maxSourceBytes)
        assertEquals(10_000, defaults.connectTimeoutMs)
        assertEquals(20_000, defaults.readTimeoutMs)
        assertEquals(2, defaults.maxConcurrentRequests)
        assertEquals(64, defaults.maxPendingRequests)
        assertEquals(2_048, defaults.maxDecodeDimensionPx)

        val parsed = ImageLoadingPolicy.fromJson(
            """{"maxSourceBytes":12,"connectTimeoutMs":13,"readTimeoutMs":14,"maxConcurrentRequests":3,"maxPendingRequests":4,"maxDecodeDimensionPx":15}"""
        )
        assertEquals(ImageLoadingPolicy(12, 13, 14, 3, 4, 15), parsed)
        assertEquals(defaults, ImageLoadingPolicy.fromJson("""{"maxSourceBytes":0}"""))
    }

    @Test
    fun `data url and remote streams stop at max source bytes`() {
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxSourceBytes = 3)
        assertNull(RenderImageDecoder.decodeDataUrlBytes("data:image/png;base64,AQIDBA==", policy))
        assertNull(RenderImageDecoder.readBounded(ByteArrayInputStream(byteArrayOf(1, 2, 3, 4)), 3))
        assertEquals(listOf<Byte>(1, 2, 3), RenderImageDecoder.readBounded(
            ByteArrayInputStream(byteArrayOf(1, 2, 3)),
            3
        )?.toList())
    }

    @Test
    fun `remote decoder configures timeouts status and bounded stream before bitmap decode`() {
        val connection = FakeConnection(URL("https://example.com/image.png"), byteArrayOf(1, 2, 3))
        var decodedBytes = 0
        RenderImageDecoder.connectionFactoryOverride = { connection }
        RenderImageDecoder.bitmapDecoderOverride = { bytes, _ ->
            decodedBytes = bytes.size
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val policy = ImageLoadingPolicy.DEFAULT.copy(connectTimeoutMs = 123, readTimeoutMs = 456)

        RenderImageDecoder.decodeSource("https://example.com/image.png", policy)

        assertEquals(123, connection.connectTimeout)
        assertEquals(456, connection.readTimeout)
        assertEquals(3, decodedBytes)
        assertTrue(connection.disconnected)
    }

    @Test
    fun `loader is asynchronous bounded and rejects queue saturation`() {
        val release = CountDownLatch(1)
        val started = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            started.countDown()
            release.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxConcurrentRequests = 1, maxPendingRequests = 1)
        val rejected = AtomicBoolean(false)

        RenderImageLoader.load("https://example.com/1", policy) { }
        assertTrue(started.await(2, TimeUnit.SECONDS))
        RenderImageLoader.load("https://example.com/2", policy) { }
        RenderImageLoader.load("https://example.com/3", policy) { rejected.set(it == null) }
        assertFalse(rejected.get())
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(rejected.get())
        release.countDown()
    }

    @Test
    fun `data decode callback is asynchronous and cancellation suppresses delivery`() {
        val callbacks = AtomicInteger(0)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val handle = RenderImageLoader.load("data:image/png;base64,AQ==", ImageLoadingPolicy.DEFAULT) {
            callbacks.incrementAndGet()
        }
        assertEquals(0, callbacks.get())
        handle.cancel()
        shadowOf(Looper.getMainLooper()).idle()
        Thread.sleep(50)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(0, callbacks.get())
    }

    @Test
    fun `cancelling queued request immediately frees queue capacity`() {
        val release = CountDownLatch(1)
        val firstStarted = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { source, _ ->
            if (source.endsWith("/1")) {
                firstStarted.countDown()
                release.await(2, TimeUnit.SECONDS)
            }
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxConcurrentRequests = 1, maxPendingRequests = 1)
        val thirdLoaded = CountDownLatch(1)
        val thirdRejected = AtomicBoolean(false)

        RenderImageLoader.load("https://example.com/1", policy) { }
        assertTrue(firstStarted.await(2, TimeUnit.SECONDS))
        val queued = RenderImageLoader.load("https://example.com/2", policy) { }
        queued.cancel()
        RenderImageLoader.load("https://example.com/3", policy) {
            thirdRejected.set(it == null)
            thirdLoaded.countDown()
        }
        release.countDown()
        drainMainUntil(thirdLoaded)

        assertFalse(thirdRejected.get())
        assertEquals(0L, thirdLoaded.count)
    }

    @Test
    fun `cancelling running remote load closes stream and disconnects connection`() {
        val stream = BlockingInputStream()
        val connection = FakeConnection(URL("https://example.com/image.png"), stream = stream)
        RenderImageDecoder.connectionFactoryOverride = { connection }
        val handle = RenderImageLoader.load(
            "https://example.com/image.png",
            ImageLoadingPolicy.DEFAULT
        ) { }
        assertTrue(stream.readStarted.await(2, TimeUnit.SECONDS))

        handle.cancel()

        assertTrue(stream.closed.await(2, TimeUnit.SECONDS))
        assertTrue(connection.disconnected)
    }

    @Test
    fun `remote decoder rejects non success and declared or chunked oversize bodies`() {
        var decoded = 0
        RenderImageDecoder.bitmapDecoderOverride = { _, _ ->
            decoded += 1
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxSourceBytes = 3)
        val connections = ArrayDeque<HttpURLConnection>().apply {
            add(FakeConnection(URL("https://example.com/404"), status = 404))
            add(FakeConnection(URL("https://example.com/declared"), bytes = byteArrayOf(1, 2, 3, 4), declaredLength = 4))
            add(FakeConnection(URL("https://example.com/chunked"), bytes = byteArrayOf(1, 2, 3, 4), declaredLength = -1))
        }
        RenderImageDecoder.connectionFactoryOverride = { connections.removeFirst() }

        assertNull(RenderImageDecoder.decodeSource("https://example.com/404", policy))
        assertNull(RenderImageDecoder.decodeSource("https://example.com/declared", policy))
        assertNull(RenderImageDecoder.decodeSource("https://example.com/chunked", policy))
        assertEquals(0, decoded)
    }

    @Test
    fun `configured decode dimension controls sampling`() {
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxDecodeDimensionPx = 512)
        assertEquals(
            4,
            RenderImageDecoder.calculateInSampleSize(
                width = 2_048,
                height = 1_024,
                maxWidth = policy.maxDecodeDimensionPx,
                maxHeight = policy.maxDecodeDimensionPx
            )
        )
    }

    @Test
    fun `idle policy executors are retired`() {
        val completed = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        RenderImageLoader.load("data:image/png;base64,AQ==", ImageLoadingPolicy.DEFAULT) {
            completed.countDown()
        }
        drainMainUntil(completed)

        assertEquals(0, RenderImageLoader.activeExecutorCountForTesting())
    }

    private fun drainMainUntil(latch: CountDownLatch) {
        repeat(100) {
            shadowOf(Looper.getMainLooper()).idle()
            if (latch.count > 0) Thread.sleep(10)
        }
    }

    private class FakeConnection(
        url: URL,
        private val bytes: ByteArray = byteArrayOf(),
        private val status: Int = 200,
        private val declaredLength: Long = bytes.size.toLong(),
        private val stream: InputStream = ByteArrayInputStream(bytes)
    ) : HttpURLConnection(url) {
        var disconnected = false
        override fun getResponseCode(): Int = status
        override fun getContentLengthLong(): Long = declaredLength
        override fun getInputStream() = stream
        override fun disconnect() { disconnected = true }
        override fun usingProxy(): Boolean = false
        override fun connect() = Unit
    }

    private class BlockingInputStream : InputStream() {
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
}
