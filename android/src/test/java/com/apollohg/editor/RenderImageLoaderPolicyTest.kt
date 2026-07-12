package com.apollohg.editor

import android.graphics.Bitmap
import android.os.Looper
import java.io.ByteArrayInputStream
import java.io.File
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
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
class RenderImageLoaderPolicyTest {
    private fun securityFixtures(): JSONObject {
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

        val parsed = ImageLoadingPolicy.fromJson(
            """{"maxSourceBytes":12,"connectTimeoutMs":13,"readTimeoutMs":14,"requestTimeoutMs":15,"maxConcurrentRequests":3,"maxPendingRequests":4,"maxDecodeDimensionPx":16}"""
        )
        assertEquals(ImageLoadingPolicy(12, 13, 14, 15, 3, 4, 16), parsed)
        assertEquals(defaults, ImageLoadingPolicy.fromJson("""{"maxSourceBytes":0}"""))
        assertEquals(
            defaults,
            ImageLoadingPolicy.fromJson(
                """{"maxSourceBytes":67108865,"connectTimeoutMs":600001,"readTimeoutMs":600001,"requestTimeoutMs":600001,"maxConcurrentRequests":17,"maxPendingRequests":513,"maxDecodeDimensionPx":8193}"""
            )
        )
    }

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
        assertNull(RenderImageLoader.cached(source, ImageLoadingPolicy.DEFAULT.copy(requestTimeoutMs = 30)))
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
        assertNull(RenderImageLoader.cached(source, policy))
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
        assertNull(RenderImageLoader.cached(source, policy))
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
        val policy = ImageLoadingPolicy.DEFAULT.copy(requestTimeoutMs = 30)
        val source = "https://example.com/cached-deadline.png"
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val warmed = CountDownLatch(1)
        RenderImageLoader.load(source, policy) { warmed.countDown() }
        drainMainUntil(warmed)
        assertTrue(RenderImageLoader.cached(source, policy) != null)

        val clock = FakeMonotonicClock()
        RenderImageLoader.monotonicClockOverride = clock
        val schedulerRelease = CountDownLatch(1)
        RenderImageLoader.deadlineExecutionGateOverride = {
            schedulerRelease.await(2, TimeUnit.SECONDS)
        }
        var result: Bitmap? = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        val completed = CountDownLatch(1)
        RenderImageLoader.load(source, policy) {
            result = it
            completed.countDown()
        }
        clock.advance(31)
        drainMainUntil(completed)
        schedulerRelease.countDown()

        assertEquals(0L, completed.count)
        assertNull(result)
        assertEquals(0, RenderImageLoader.globalAdmissionCountForTesting())
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

    private class FakeMonotonicClock : MonotonicClock {
        private var nowMs = 0L
        override fun elapsedRealtime(): Long = nowMs
        fun advance(milliseconds: Long) {
            nowMs += milliseconds
        }
    }

    private class TrickleInputStream(
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
