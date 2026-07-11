package com.apollohg.editor

import android.graphics.Bitmap
import android.os.Looper
import java.io.ByteArrayInputStream
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

    private class FakeConnection(url: URL, private val bytes: ByteArray) : HttpURLConnection(url) {
        var disconnected = false
        override fun getResponseCode(): Int = 200
        override fun getInputStream() = ByteArrayInputStream(bytes)
        override fun disconnect() { disconnected = true }
        override fun usingProxy(): Boolean = false
        override fun connect() = Unit
    }
}
