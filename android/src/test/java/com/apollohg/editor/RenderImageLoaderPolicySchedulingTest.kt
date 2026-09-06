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
internal class RenderImageLoaderPolicySchedulingTest : RenderImageLoaderPolicyTestFixture() {
    @Test
    fun `data url admission bounds metadata raw whitespace and observes cancellation`() {
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxSourceBytes = 8)
        assertNull(
            RenderImageDecoder.decodeDataUrlBytes(
                "data:image/png;base64," + "A ".repeat(10_000),
                policy
            )
        )
        assertNull(
            RenderImageDecoder.decodeDataUrlBytes(
                "data:image/" + "x".repeat(257) + ";base64,AQ==",
                policy
            )
        )
        val cancellation = RenderImageDecoder.Cancellation().apply { cancel() }
        assertNull(
            RenderImageDecoder.decodeDataUrlBytes(
                "data:image/png;base64,AQ==",
                policy,
                cancellation
            )
        )
    }

    @Test
    fun `digest time is charged to the absolute request deadline`() {
        val clock = FakeMonotonicClock()
        RenderImageLoader.monotonicClockOverride = clock
        RenderImageLoader.beforeDigestOverride = { clock.advance(31) }
        val decodeInvoked = AtomicBoolean(false)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            decodeInvoked.set(true)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val completed = CountDownLatch(1)
        val finished = AtomicInteger(0)
        var result: Bitmap? = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)

        val handle = RenderImageLoader.load(
            "https://example.com/digest-deadline.png",
            ImageLoadingPolicy.DEFAULT.copy(requestTimeoutMs = 30)
        ) {
            result = it
            completed.countDown()
        }
        handle.onFinished { finished.incrementAndGet() }
        drainMainUntil(completed)

        assertNull(result)
        assertFalse(decodeInvoked.get())
        assertEquals(1L, RenderImageLoader.digestConstructionCountForTesting())
        assertEquals(0, RenderImageLoader.cacheEntryCountForTesting())
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
        assertEquals(1, finished.get())
    }

    @Test
    fun `absolute deadline stops trickle reads`() {
        val clock = FakeMonotonicClock()
        val stream = TrickleInputStream(clock, byteEveryMs = 19_000)
        val policy = ImageLoadingPolicy.DEFAULT.copy(
            maxSourceBytes = 100,
            readTimeoutMs = 20_000,
            requestTimeoutMs = 60_000
        )

        assertNull(RenderImageDecoder.readBounded(stream, policy, clock))
        assertTrue(clock.elapsedRealtime() >= 60_000)
    }

    @Test
    fun `shared whitespace base64 and trickle fixtures execute against Android boundary`() {
        val fixtures = securityFixtures()
        val whitespace = fixtures.getJSONObject("whitespaceBase64")
        assertTrue(whitespace.getBoolean("whitespaceCountsTowardAdmission"))
        assertEquals(
            byteArrayOf(1).toList(),
            RenderImageDecoder.decodeDataUrlBytes(
                whitespace.getString("source"),
                ImageLoadingPolicy.DEFAULT.copy(maxSourceBytes = 1)
            )?.toList()
        )

        val trickle = fixtures.getJSONObject("trickleDeadline")
        val arrivals = trickle.getJSONArray("byteArrivalMs")
        val interval = arrivals.getLong(1) - arrivals.getLong(0)
        val clock = FakeMonotonicClock()
        val policy = ImageLoadingPolicy.DEFAULT.copy(
            maxSourceBytes = 100,
            readTimeoutMs = (interval + 1).toInt(),
            requestTimeoutMs = trickle.getInt("requestTimeoutMs")
        )
        assertNull(RenderImageDecoder.readBounded(TrickleInputStream(clock, interval), policy, clock))
        assertEquals(trickle.getLong("expectedTerminalMs"), clock.elapsedRealtime())
        assertEquals("timeout", trickle.getString("expectedOutcome"))
    }

    @Test
    fun `blocked request deadline releases admission and suppresses stale success`() {
        val stream = BlockingInputStream()
        val connection = FakeConnection(URL("https://example.com/deadline.png"), stream = stream)
        RenderImageDecoder.connectionFactoryOverride = { connection }
        val callbackResult = AtomicReference<Bitmap?>(Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888))
        val completed = CountDownLatch(1)
        val policy = ImageLoadingPolicy.DEFAULT.copy(requestTimeoutMs = 50)

        RenderImageLoader.load("https://example.com/deadline.png", policy) {
            callbackResult.set(it)
            completed.countDown()
        }
        assertTrue(stream.readStarted.await(2, TimeUnit.SECONDS))
        drainMainUntil(completed)

        assertEquals(0L, completed.count)
        assertNull(callbackResult.get())
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
        assertTrue(stream.closed.await(2, TimeUnit.SECONDS))
        assertTrue(connection.disconnected)
    }

    @Test
    fun `decode completing after deadline fails even when deadline scheduler is delayed`() {
        val clock = FakeMonotonicClock()
        RenderImageLoader.monotonicClockOverride = clock
        val schedulerRelease = CountDownLatch(1)
        RenderImageLoader.deadlineExecutionGateOverride = {
            schedulerRelease.await(2, TimeUnit.SECONDS)
        }
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            clock.advance(31)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val completed = CountDownLatch(1)
        var result: Bitmap? = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        val source = "https://example.com/late-decode.png"

        RenderImageLoader.load(
            source,
            ImageLoadingPolicy.DEFAULT.copy(requestTimeoutMs = 30)
        ) {
            result = it
            completed.countDown()
        }
        drainMainUntil(completed)
        schedulerRelease.countDown()

        assertEquals(0L, completed.count)
        assertNull(result)
        assertFalse(RenderImageLoader.isCachedForTesting(source, ImageLoadingPolicy.DEFAULT.copy(requestTimeoutMs = 30)))
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
    }

    @Test
    fun `deadline between validation and cache commit suppresses bitmap`() {
        val clock = FakeMonotonicClock()
        RenderImageLoader.monotonicClockOverride = clock
        val schedulerRelease = CountDownLatch(1)
        RenderImageLoader.deadlineExecutionGateOverride = {
            schedulerRelease.await(2, TimeUnit.SECONDS)
        }
        RenderImageLoader.beforeCacheCommitOverride = { clock.advance(31) }
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val completed = CountDownLatch(1)
        var result: Bitmap? = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        val policy = ImageLoadingPolicy.DEFAULT.copy(requestTimeoutMs = 30)
        val source = "https://example.com/cache-race.png"

        RenderImageLoader.load(source, policy) {
            result = it
            completed.countDown()
        }
        drainMainUntil(completed)
        schedulerRelease.countDown()

        assertEquals(0L, completed.count)
        assertNull(result)
        assertFalse(RenderImageLoader.isCachedForTesting(source, policy))
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
    }

    @Test
    fun `deadline crossing immediately before terminal claim downgrades success`() {
        val clock = FakeMonotonicClock()
        RenderImageLoader.monotonicClockOverride = clock
        val schedulerRelease = CountDownLatch(1)
        RenderImageLoader.deadlineExecutionGateOverride = {
            schedulerRelease.await(2, TimeUnit.SECONDS)
        }
        RenderImageLoader.beforeTerminalClaimOverride = { clock.advance(31) }
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val completed = CountDownLatch(1)
        var result: Bitmap? = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        val policy = ImageLoadingPolicy.DEFAULT.copy(requestTimeoutMs = 30)
        val source = "https://example.com/terminal-race.png"

        RenderImageLoader.load(source, policy) {
            result = it
            completed.countDown()
        }
        drainMainUntil(completed)
        schedulerRelease.countDown()

        assertEquals(0L, completed.count)
        assertNull(result)
        assertFalse(RenderImageLoader.isCachedForTesting(source, policy))
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
    }

    @Test
    fun `main looper delivery after absolute deadline suppresses decoded bitmap`() {
        val clock = FakeMonotonicClock()
        RenderImageLoader.monotonicClockOverride = clock
        val schedulerRelease = CountDownLatch(1)
        val deliveryPosted = CountDownLatch(1)
        RenderImageLoader.deadlineExecutionGateOverride = {
            schedulerRelease.await(2, TimeUnit.SECONDS)
        }
        RenderImageLoader.decodedDeliveryPostedOverride = { deliveryPosted.countDown() }
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val policy = ImageLoadingPolicy.DEFAULT.copy(requestTimeoutMs = 30)
        var result: Bitmap? = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        val completed = CountDownLatch(1)

        RenderImageLoader.load("https://example.com/main-delay.png", policy) {
            result = it
            completed.countDown()
        }
        assertTrue(deliveryPosted.await(2, TimeUnit.SECONDS))
        clock.advance(31)
        drainMainUntil(completed)
        schedulerRelease.countDown()

        assertEquals(0L, completed.count)
        assertNull(result)
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
    }

    @Test
    fun `cached main looper delivery after deadline is suppressed`() {
        val policy = ImageLoadingPolicy.DEFAULT
        val source = "https://example.com/cached-deadline.png"
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val warmed = CountDownLatch(1)
        RenderImageLoader.load(source, policy) { warmed.countDown() }
        drainMainUntil(warmed)
        assertTrue(RenderImageLoader.isCachedForTesting(source, policy))

        val clock = FakeMonotonicClock()
        RenderImageLoader.monotonicClockOverride = clock
        var result: Bitmap? = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        val completed = CountDownLatch(1)
        RenderImageLoader.load(source, policy) {
            result = it
            completed.countDown()
        }
        clock.advance(policy.requestTimeoutMs.toLong() + 1)
        drainMainUntil(completed)

        assertEquals(0L, completed.count)
        assertNull(result)
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
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
    fun `global priority queue capacity is atomic under concurrent offers`() {
        repeat(20) {
            val queue = RenderImageLoader.BoundedPriorityQueue<Int>(1)
            val ready = CountDownLatch(32)
            val start = CountDownLatch(1)
            val executor = Executors.newFixedThreadPool(32)
            repeat(32) { value ->
                executor.execute {
                    ready.countDown()
                    start.await()
                    queue.offer(value)
                }
            }

            assertTrue(ready.await(2, TimeUnit.SECONDS))
            start.countDown()
            executor.shutdown()
            assertTrue(executor.awaitTermination(2, TimeUnit.SECONDS))
            assertTrue(queue.size <= 1)
        }
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
    fun `queued visible decode starts before queued prefetch`() {
        val workerCount = RenderImageLoader.globalWorkerLimitForTesting()
        val blockersStarted = CountDownLatch(workerCount)
        val blockerReleases = List(workerCount) { CountDownLatch(1) }
        val visibleStarted = CountDownLatch(1)
        val completed = CountDownLatch(workerCount + 2)
        val starts = CopyOnWriteArrayList<String>()
        RenderImageLoader.decodeSourceOverride = { source, _ ->
            starts += source
            if (source.contains("blocker")) {
                blockersStarted.countDown()
                blockerReleases[source.substringAfterLast('/').toInt()].await(2, TimeUnit.SECONDS)
            } else if (source.contains("queued-visible")) {
                visibleStarted.countDown()
            }
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val policy = ImageLoadingPolicy.DEFAULT.copy(maxConcurrentRequests = 16)
        val owner = DecodedBitmapBudget.nextOwnerId()
        fun enqueue(source: String, priority: DecodedBitmapPriority) {
            RenderImageLoader.loadLease(source, policy, owner, priority) { lease ->
                lease?.close()
                completed.countDown()
            }
        }
        try {
            repeat(workerCount) { index ->
                enqueue("https://example.com/blocker/$index", DecodedBitmapPriority.PREFETCH)
            }
            assertTrue(blockersStarted.await(2, TimeUnit.SECONDS))
            val prefetch = "https://example.com/queued-prefetch"
            val visible = "https://example.com/queued-visible"
            enqueue(prefetch, DecodedBitmapPriority.PREFETCH)
            enqueue(visible, DecodedBitmapPriority.VISIBLE)

            blockerReleases.first().countDown()
            assertTrue(visibleStarted.await(2, TimeUnit.SECONDS))
            blockerReleases.drop(1).forEach { it.countDown() }
            drainMainUntil(completed)

            assertTrue(starts.indexOf(visible) < starts.indexOf(prefetch))
        } finally {
            blockerReleases.forEach { it.countDown() }
        }
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
}
