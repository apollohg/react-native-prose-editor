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

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class RenderImageLoaderPolicyDecodingTest : RenderImageLoaderPolicyTestFixture() {
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
    fun `sampling also honors decoded byte ceiling without overflow`() {
        assertEquals(
            4,
            RenderImageDecoder.calculateInSampleSize(
                width = 4_096,
                height = 4_096,
                maxWidth = 8_192,
                maxHeight = 8_192,
                maxDecodedBytes = 4L * 1024 * 1024
            )
        )
        assertEquals(
            1 shl 30,
            RenderImageDecoder.calculateInSampleSize(
                width = Int.MAX_VALUE,
                height = Int.MAX_VALUE,
                maxWidth = Int.MAX_VALUE,
                maxHeight = Int.MAX_VALUE,
                maxDecodedBytes = 1
            )
        )
    }

    @Test
    fun `sampling uses ceiling division at the decode dimension boundary`() {
        assertEquals(
            4,
            RenderImageDecoder.calculateInSampleSize(
                width = 4_097,
                height = 2_048,
                maxWidth = 2_048,
                maxHeight = 2_048
            )
        )
    }

    @Test
    fun `actual decoded bitmap is constrained when decoder dimensions are approximate`() {
        DecodedBitmapBudget.shared(org.robolectric.RuntimeEnvironment.getApplication())
        val oversized = Bitmap.createBitmap(4_097, 2_048, Bitmap.Config.ARGB_8888)
        RenderImageDecoder.bitmapDecoderOverride = { _, _ -> oversized }

        val decoded = RenderImageDecoder.decodeSource(
            "data:image/png;base64,AQ==",
            ImageLoadingPolicy.DEFAULT.copy(maxDecodeDimensionPx = 2_048)
        )

        requireNotNull(decoded)
        assertEquals(2_048, decoded.width)
        assertTrue(decoded.height <= 2_048)
    }

    @Test
    fun `actual decoded bitmap is constrained by decoded byte policy`() {
        RenderImageDecoder.bitmapDecoderOverride = { _, _ ->
            Bitmap.createBitmap(32, 32, Bitmap.Config.ARGB_8888)
        }

        val decoded = RenderImageDecoder.decodeSource(
            "data:image/png;base64,AQ==",
            ImageLoadingPolicy.DEFAULT.copy(
                maxDecodeDimensionPx = 128,
                maxDecodedBytes = 1_024,
            ),
        )

        requireNotNull(decoded)
        assertTrue(decoded.allocationByteCount <= 1_024)
    }

    @Test
    fun `sampling maximum integer dimensions never overflows`() {
        assertEquals(
            1 shl 30,
            RenderImageDecoder.calculateInSampleSize(
                width = Int.MAX_VALUE,
                height = Int.MAX_VALUE,
                maxWidth = 1,
                maxHeight = 1
            )
        )
    }

    @Test
    fun `throwing decoder completes callback and permits retry`() {
        val completed = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ -> error("decoder failure") }
        var firstResult: Bitmap? = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        val source = "https://example.com/retry.png"
        RenderImageLoader.load(source, ImageLoadingPolicy.DEFAULT) {
            firstResult = it
            completed.countDown()
        }
        drainMainUntil(completed)
        assertEquals(0L, completed.count)
        assertNull(firstResult)

        val retried = CountDownLatch(1)
        var retryResult: Bitmap? = null
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        RenderImageLoader.load(source, ImageLoadingPolicy.DEFAULT) {
            retryResult = it
            retried.countDown()
        }
        drainMainUntil(retried)

        assertEquals(0L, retried.count)
        assertTrue(retryResult != null)
    }

    @Test
    fun `decoder out of memory is reported as a nonfatal miss`() {
        val completed = CountDownLatch(1)
        var result: Bitmap? = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        RenderImageLoader.decodeSourceOverride = { _, _ -> throw OutOfMemoryError("test") }

        RenderImageLoader.load("https://example.com/oom.png") {
            result = it
            completed.countDown()
        }

        drainMainUntil(completed)
        assertNull(result)
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
        assertEquals(0, RenderImageLoader.cacheEntryCountForTesting())
    }
}
