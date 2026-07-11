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
    fun `policy churn uses one global bounded execution resource`() {
        val releases = mutableListOf<CountDownLatch>()
        val handles = (1..20).map { index ->
            val release = CountDownLatch(1)
            releases += release
            RenderImageLoader.decodeSourceOverride = { _, _ ->
                release.await(2, TimeUnit.SECONDS)
                Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
            }
            RenderImageLoader.load(
                "https://example.com/churn/$index",
                ImageLoadingPolicy.DEFAULT.copy(
                    maxConcurrentRequests = 1,
                    maxPendingRequests = 1,
                    readTimeoutMs = 1_000 + index
                )
            ) { }
        }

        handles.forEach { it.cancel() }
        releases.forEach { it.countDown() }

        assertEquals(1, RenderImageLoader.executionResourceCountForTesting())
        assertTrue(
            RenderImageLoader.globalQueuedTaskCountForTesting() <=
                RenderImageLoader.globalQueueLimitForTesting()
        )
    }

    @Test
    fun `policy concurrency above global workers is admitted under global ceiling`() {
        val release = CountDownLatch(1)
        val started = CountDownLatch(4)
        val callbacks = CountDownLatch(10)
        val rejected = AtomicBoolean(false)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            started.countDown()
            release.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val policy = ImageLoadingPolicy.DEFAULT.copy(
            maxConcurrentRequests = 10,
            maxPendingRequests = 20
        )

        repeat(10) { index ->
            RenderImageLoader.load("https://example.com/high/$index", policy) {
                if (it == null) rejected.set(true)
                callbacks.countDown()
            }
        }
        assertTrue(started.await(2, TimeUnit.SECONDS))
        assertTrue(
            RenderImageLoader.globalActiveWorkerCountForTesting() <=
                RenderImageLoader.globalWorkerLimitForTesting()
        )
        release.countDown()
        drainMainUntil(callbacks)

        assertEquals(0L, callbacks.count)
        assertFalse(rejected.get())
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
    fun `global admission bounds unique policies and deduped callbacks`() {
        val release = CountDownLatch(1)
        val started = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            started.countDown()
            release.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val rejected = AtomicInteger(0)
        val handles = mutableListOf<RenderImageLoader.LoadHandle>()

        repeat(300) { index ->
            handles += RenderImageLoader.load(
                "https://example.com/unique/$index",
                ImageLoadingPolicy.DEFAULT.copy(readTimeoutMs = 10_000 + index)
            ) { if (it == null) rejected.incrementAndGet() }
        }
        repeat(300) {
            handles += RenderImageLoader.load(
                "https://example.com/deduped",
                ImageLoadingPolicy.DEFAULT
            ) { if (it == null) rejected.incrementAndGet() }
        }
        assertTrue(started.await(2, TimeUnit.SECONDS))
        shadowOf(Looper.getMainLooper()).idle()

        assertTrue(
            RenderImageLoader.globalAdmissionCountForTesting() <=
                RenderImageLoader.globalAdmissionLimitForTesting()
        )
        assertEquals(RenderImageLoader.rejectionNotificationLimitForTesting(), rejected.get())
        handles.forEach { it.cancel() }
        release.countDown()
    }

    @Test
    fun `throwing callback does not block deduped callback or handle completion`() {
        val release = CountDownLatch(1)
        val started = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            started.countDown()
            release.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val source = "https://example.com/callbacks.png"
        val first = RenderImageLoader.load(source, ImageLoadingPolicy.DEFAULT) {
            error("callback failure")
        }
        assertTrue(started.await(2, TimeUnit.SECONDS))
        val secondDelivered = CountDownLatch(1)
        val second = RenderImageLoader.load(source, ImageLoadingPolicy.DEFAULT) {
            secondDelivered.countDown()
        }
        val handlesFinished = CountDownLatch(2)
        first.onFinished { handlesFinished.countDown() }
        second.onFinished { handlesFinished.countDown() }

        release.countDown()
        drainMainUntil(secondDelivered)

        assertEquals(0L, secondDelivered.count)
        assertEquals(0L, handlesFinished.count)
    }

    @Test
    fun `throwing cached callback still finishes handle and releases admission`() {
        val source = "https://example.com/cached-callback.png"
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val warmed = CountDownLatch(1)
        RenderImageLoader.load(source, ImageLoadingPolicy.DEFAULT) { warmed.countDown() }
        drainMainUntil(warmed)

        val handle = RenderImageLoader.load(source, ImageLoadingPolicy.DEFAULT) {
            error("cached callback failure")
        }
        val finished = CountDownLatch(1)
        handle.onFinished { finished.countDown() }
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals(0L, finished.count)
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
    }

    @Test
    fun `throwing global rejection callback still finishes handle`() {
        val release = CountDownLatch(1)
        val started = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            started.countDown()
            release.await(2, TimeUnit.SECONDS)
            null
        }
        val accepted = (0 until RenderImageLoader.globalAdmissionLimitForTesting()).map { index ->
            RenderImageLoader.load(
                "https://example.com/global-rejection/$index",
                ImageLoadingPolicy.DEFAULT.copy(readTimeoutMs = 10_000 + index)
            ) { }
        }
        assertTrue(started.await(2, TimeUnit.SECONDS))
        val rejected = RenderImageLoader.load("https://example.com/global-rejection/overflow") {
            error("global rejection callback failure")
        }
        val finished = CountDownLatch(1)
        rejected.onFinished { finished.countDown() }
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals(0L, finished.count)
        accepted.forEach { it.cancel() }
        release.countDown()
    }

    @Test
    fun `throwing policy rejection callback still finishes handle`() {
        val release = CountDownLatch(1)
        val started = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            started.countDown()
            release.await(2, TimeUnit.SECONDS)
            null
        }
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxConcurrentRequests = 1, maxPendingRequests = 1)
        val accepted = listOf(
            RenderImageLoader.load("https://example.com/policy-rejection/active", policy) { },
            RenderImageLoader.load("https://example.com/policy-rejection/pending", policy) { }
        )
        assertTrue(started.await(2, TimeUnit.SECONDS))
        val rejected = RenderImageLoader.load(
            "https://example.com/policy-rejection/overflow",
            policy
        ) { error("policy rejection callback failure") }
        val finished = CountDownLatch(1)
        rejected.onFinished { finished.countDown() }
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals(0L, finished.count)
        accepted.forEach { it.cancel() }
        release.countDown()
    }

    @Test
    fun `transient full executor rejection is retried after worker return`() {
        val firstSource = "https://example.com/retry-capacity/first"
        val firstRelease = CountDownLatch(1)
        val otherRelease = CountDownLatch(1)
        val beforeFirstReturn = CountDownLatch(1)
        val allowFirstReturn = CountDownLatch(1)
        val firstStarted = CountDownLatch(1)
        val secondDelivered = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { source, _ ->
            if (source == firstSource) {
                firstStarted.countDown()
                while (firstRelease.count > 0) {
                    try {
                        firstRelease.await(2, TimeUnit.SECONDS)
                    } catch (_: InterruptedException) {
                        // Cancellation frees admission while this test worker remains active.
                    }
                }
            } else {
                otherRelease.await(2, TimeUnit.SECONDS)
            }
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        RenderImageLoader.beforeWorkerReturnOverride = { source ->
            if (source == firstSource) {
                beforeFirstReturn.countDown()
                allowFirstReturn.await(2, TimeUnit.SECONDS)
            }
        }
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxConcurrentRequests = 1, maxPendingRequests = 1)
        val first = RenderImageLoader.load(firstSource, policy) { }
        assertTrue(firstStarted.await(2, TimeUnit.SECONDS))
        val fillers = (0 until RenderImageLoader.globalAdmissionLimitForTesting() - 1).map { index ->
            RenderImageLoader.load(
                "https://example.com/retry-capacity/filler/$index",
                ImageLoadingPolicy.DEFAULT.copy(readTimeoutMs = 10_000 + index)
            ) { }
        }
        assertEquals(
            RenderImageLoader.globalQueueLimitForTesting(),
            RenderImageLoader.globalQueuedTaskCountForTesting()
        )
        first.cancel()
        RenderImageLoader.load("https://example.com/retry-capacity/second", policy) {
            secondDelivered.countDown()
        }

        firstRelease.countDown()
        assertTrue(beforeFirstReturn.await(2, TimeUnit.SECONDS))
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(RenderImageLoader.submissionRejectionCountForTesting() > 0)

        allowFirstReturn.countDown()
        otherRelease.countDown()
        drainMainUntil(secondDelivered)

        assertEquals(0L, secondDelivered.count)
        fillers.forEach { it.cancel() }
    }

    @Test
    fun `rejection notifications retain only a bounded batch`() {
        val release = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            release.await(2, TimeUnit.SECONDS)
            null
        }
        val accepted = (0 until RenderImageLoader.globalAdmissionLimitForTesting()).map { index ->
            RenderImageLoader.load(
                "https://example.com/rejection-retention/$index",
                ImageLoadingPolicy.DEFAULT.copy(readTimeoutMs = 10_000 + index)
            ) { }
        }
        repeat(1_000) { index ->
            RenderImageLoader.load("https://example.com/rejection-retention/overflow/$index") { }
        }

        assertTrue(
            RenderImageLoader.rejectionNotificationCountForTesting() <=
                RenderImageLoader.rejectionNotificationLimitForTesting()
        )
        accepted.forEach { it.cancel() }
        release.countDown()
    }

    @Test
    fun `rejection overflow drops notifications without inline or off-main delivery`() {
        val release = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            release.await(2, TimeUnit.SECONDS)
            null
        }
        val accepted = (0 until RenderImageLoader.globalAdmissionLimitForTesting()).map { index ->
            RenderImageLoader.load(
                "https://example.com/rejection-threading/accepted/$index",
                ImageLoadingPolicy.DEFAULT.copy(readTimeoutMs = 10_000 + index)
            ) { }
        }
        val overflowCount = RenderImageLoader.rejectionNotificationLimitForTesting() + 128
        val callbackCount = AtomicInteger(0)
        val offMainCount = AtomicInteger(0)
        val handlesFinished = CountDownLatch(overflowCount)
        val callerFinished = CountDownLatch(1)
        Thread {
            repeat(overflowCount) { index ->
                val handle = RenderImageLoader.load(
                    "https://example.com/rejection-threading/overflow/$index"
                ) {
                    callbackCount.incrementAndGet()
                    if (Looper.myLooper() != Looper.getMainLooper()) offMainCount.incrementAndGet()
                }
                handle.onFinished { handlesFinished.countDown() }
            }
            callerFinished.countDown()
        }.start()

        assertTrue(callerFinished.await(2, TimeUnit.SECONDS))
        assertEquals(0, callbackCount.get())
        assertEquals(
            RenderImageLoader.rejectionNotificationLimitForTesting(),
            RenderImageLoader.rejectionNotificationCountForTesting()
        )

        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(RenderImageLoader.rejectionNotificationLimitForTesting(), callbackCount.get())
        assertEquals(0, offMainCount.get())
        assertEquals(0L, handlesFinished.count)
        accepted.forEach { it.cancel() }
        release.countDown()
    }

    @Test
    fun `mixed cached deduped and policy pending handles all finish`() {
        val cachedSource = "https://example.com/mixed/cached"
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val warmed = CountDownLatch(1)
        RenderImageLoader.load(cachedSource) { warmed.countDown() }
        drainMainUntil(warmed)

        val release = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            release.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxConcurrentRequests = 1, maxPendingRequests = 16)
        val handles = mutableListOf<RenderImageLoader.LoadHandle>()
        handles += RenderImageLoader.load("https://example.com/mixed/deduped", policy) { }
        repeat(10) {
            handles += RenderImageLoader.load("https://example.com/mixed/deduped", policy) { }
        }
        repeat(10) { index ->
            handles += RenderImageLoader.load("https://example.com/mixed/pending/$index", policy) { }
        }
        repeat(10) {
            handles += RenderImageLoader.load(cachedSource) { }
        }
        val finished = CountDownLatch(handles.size)
        handles.forEach { it.onFinished { finished.countDown() } }

        release.countDown()
        drainMainUntil(finished)

        assertEquals(0L, finished.count)
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
    }

    @Test
    fun `throwing disconnect cannot strand admission or handle completion`() {
        val stream = BlockingInputStream()
        val connection = FakeConnection(
            URL("https://example.com/throw-disconnect.png"),
            stream = stream,
            throwOnDisconnect = true
        )
        RenderImageDecoder.connectionFactoryOverride = { connection }
        val handle = RenderImageLoader.load(
            "https://example.com/throw-disconnect.png",
            ImageLoadingPolicy.DEFAULT
        ) { }
        val finished = CountDownLatch(1)
        handle.onFinished { finished.countDown() }
        assertTrue(stream.readStarted.await(2, TimeUnit.SECONDS))

        handle.cancel()

        assertEquals(0L, finished.count)
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
        stream.close()
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
