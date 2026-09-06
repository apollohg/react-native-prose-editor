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
internal class RenderImageLoaderPolicyTest : RenderImageLoaderPolicyTestFixture() {
    @Test
    fun `image policy defaults and invalid fields match public contract`() {
        val defaults = ImageLoadingPolicy.fromJson(null)
        assertEquals(10 * 1024 * 1024, defaults.maxSourceBytes)
        assertEquals(10_000, defaults.connectTimeoutMs)
        assertEquals(20_000, defaults.readTimeoutMs)
        assertEquals(60_000, defaults.requestTimeoutMs)
        assertEquals(2, defaults.maxConcurrentRequests)
        assertEquals(64, defaults.maxPendingRequests)
        assertEquals(2_048, defaults.maxDecodeDimensionPx)
        assertEquals(32 * 1024 * 1024, defaults.maxDecodedBytes)

        val parsed = ImageLoadingPolicy.fromJson(
            """{"maxSourceBytes":12,"connectTimeoutMs":13,"readTimeoutMs":14,"requestTimeoutMs":15,"maxConcurrentRequests":3,"maxPendingRequests":4,"maxDecodeDimensionPx":16,"maxDecodedBytes":17}"""
        )
        assertEquals(ImageLoadingPolicy(12, 13, 14, 15, 3, 4, 16, 17), parsed)
        assertEquals(defaults, ImageLoadingPolicy.fromJson("""{"maxSourceBytes":0}"""))
        assertEquals(
            defaults,
            ImageLoadingPolicy.fromJson(
                """{"maxSourceBytes":67108865,"connectTimeoutMs":600001,"readTimeoutMs":600001,"requestTimeoutMs":600001,"maxConcurrentRequests":17,"maxPendingRequests":513,"maxDecodeDimensionPx":8193,"maxDecodedBytes":268435457}"""
            )
        )
    }

    @Test
    fun `hostile data url is rejected before digest construction`() {
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxSourceBytes = 8)
        val completed = CountDownLatch(1)
        var result: Bitmap? = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)

        RenderImageLoader.load(
            "data:image/png;base64," + "A ".repeat(10_000),
            policy
        ) {
            result = it
            completed.countDown()
        }
        drainMainUntil(completed)

        assertNull(result)
        assertEquals(0, RenderImageLoader.digestConstructionCountForTesting())
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())

        val encodedOverflow = CountDownLatch(1)
        RenderImageLoader.load("data:image/png;base64," + "A".repeat(13), policy) {
            encodedOverflow.countDown()
        }
        drainMainUntil(encodedOverflow)
        assertEquals(0L, RenderImageLoader.digestConstructionCountForTesting())
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
    }

    @Test
    fun `block image span preflights hostile data url before cache digest construction`() {
        val host = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication()).apply {
            setImageLoadingPolicyJson("""{"maxSourceBytes":8}""")
        }

        BlockImageSpan(
            source = "data:image/png;base64," + "A".repeat(13),
            hostView = host,
            density = 1f,
            preferredWidthDp = null,
            preferredHeightDp = null
        )

        assertEquals(0L, RenderImageLoader.digestConstructionCountForTesting())
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
    }

    @Test
    fun `block image span computes one digest for cache lookup and load`() {
        val release = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            release.await(2, TimeUnit.SECONDS)
            null
        }

        BlockImageSpan(
            source = "https://example.com/one-digest.png",
            hostView = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication()),
            density = 1f,
            preferredWidthDp = null,
            preferredHeightDp = null
        )

        assertEquals(1L, RenderImageLoader.digestConstructionCountForTesting())
        release.countDown()
    }

    @Test
    fun `block image span does not hash when global admission is full`() {
        val release = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            release.await(2, TimeUnit.SECONDS)
            null
        }
        val accepted = (0 until RenderImageLoader.globalAdmissionLimitForTesting()).map { index ->
            RenderImageLoader.load(
                "https://example.com/admission-filler/$index",
                ImageLoadingPolicy.DEFAULT.copy(readTimeoutMs = 10_000 + index)
            ) { }
        }
        val digestsBeforeRejectedSpan = RenderImageLoader.digestConstructionCountForTesting()

        BlockImageSpan(
            source = "https://example.com/not-admitted.png",
            hostView = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication()),
            density = 1f,
            preferredWidthDp = null,
            preferredHeightDp = null
        )

        assertEquals(
            digestsBeforeRejectedSpan,
            RenderImageLoader.digestConstructionCountForTesting()
        )
        accepted.forEach { it.cancel() }
        release.countDown()
    }

    @Test
    fun `cache uses fixed digest keys and accounts retained entry cost`() {
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val first = "https://example.com/" + "a".repeat(2 * 1024 * 1024)
        val second = "https://example.com/" + "b".repeat(2 * 1024 * 1024)
        val completed = CountDownLatch(2)

        RenderImageLoader.load(first) { completed.countDown() }
        RenderImageLoader.load(second) { completed.countDown() }
        drainMainUntil(completed)

        assertEquals(0L, completed.count)
        assertEquals(32, RenderImageLoader.cacheKeyByteCountForTesting(first))
        assertEquals(2, RenderImageLoader.cacheEntryCountForTesting())
        assertTrue(RenderImageLoader.cacheRetainedCostForTesting() < 1_024)
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
}
