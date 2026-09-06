package com.apollohg.editor

import android.graphics.Bitmap
import android.os.Handler
import android.os.Looper
import android.util.LruCache
import java.security.MessageDigest
import java.util.concurrent.Callable
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.Semaphore
import java.util.concurrent.Future
import java.util.concurrent.FutureTask
import java.util.concurrent.PriorityBlockingQueue
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

internal object RenderImageLoader {
    // Public policy values are per-policy upper bounds. These process-wide
    // ceilings keep aggregate work bounded when many differently configured
    // editor/viewer instances coexist; contention may yield lower concurrency.
    private const val GLOBAL_WORKERS = 4
    private const val GLOBAL_QUEUE_CAPACITY = 256
    private const val GLOBAL_ADMISSION_LIMIT = GLOBAL_WORKERS + GLOBAL_QUEUE_CAPACITY
    private const val REJECTION_NOTIFICATION_LIMIT = 64
    private const val CACHE_ENTRY_OVERHEAD_BYTES = 64

    internal class CacheKey(val digest: ByteArray) {
        override fun equals(other: Any?): Boolean =
            other is CacheKey && digest.contentEquals(other.digest)

        override fun hashCode(): Int = digest.contentHashCode()
    }

    internal data class RequestKey(val digest: CacheKey, val policy: ImageLoadingPolicy)
    internal class PreparedSource internal constructor(
        internal val source: String,
        internal val policy: ImageLoadingPolicy
    )
    internal class LoadHandle(private val cancelAction: () -> Unit) {
        private val finished = AtomicBoolean(false)
        private val finishListeners = mutableListOf<() -> Unit>()

        fun cancel() {
            try {
                runCatching { cancelAction() }
            } finally {
                finish()
            }
        }

        fun onFinished(listener: () -> Unit) {
            val invokeNow = synchronized(finishListeners) {
                if (finished.get()) true else {
                    finishListeners += listener
                    false
                }
            }
            if (invokeNow) listener()
        }

        internal fun finish() {
            if (!finished.compareAndSet(false, true)) return
            val listeners = synchronized(finishListeners) {
                finishListeners.toList().also { finishListeners.clear() }
            }
            listeners.forEach { runCatching { it() } }
        }
    }
    private data class Callback(
        val id: Long,
        val cancelled: AtomicBoolean,
        val admissionReleased: AtomicBoolean,
        val handle: LoadHandle,
        val ownerId: Long?,
        val ownerLimitBytes: Long,
        val priority: DecodedBitmapPriority,
        val deliver: (DecodedBitmapLease?) -> Unit,
    )
    private class PendingRequest(
        val key: RequestKey,
        val source: String,
        val callbacks: MutableList<Callback>,
        val cancellation: RenderImageDecoder.Cancellation,
        val startedAtMs: Long
    ) {
        @Volatile var priority: DecodedBitmapPriority = callbacks.first().priority
        val deadlineMs: Long = deadlineAfter(startedAtMs, key.policy.requestTimeoutMs)
        var future: Future<*>? = null
        var timeoutFuture: ScheduledFuture<*>? = null
        var submitted = false
        var dispatching = false
        val started = AtomicBoolean(false)
        val terminal = AtomicBoolean(false)
        val workerSlotReleased = AtomicBoolean(false)
    }
    private class PrioritizedTask(
        private val request: PendingRequest,
        private val sequence: Long,
        action: () -> Unit,
    ) : FutureTask<Unit>(Callable { action(); Unit }), Comparable<PrioritizedTask> {
        override fun compareTo(other: PrioritizedTask): Int {
            val priority = request.priority.compareTo(other.request.priority)
            return if (priority != 0) priority else sequence.compareTo(other.sequence)
        }
    }
    internal class BoundedPriorityQueue<E>(capacity: Int) : PriorityBlockingQueue<E>() {
        private val permits = Semaphore(capacity)

        override fun offer(element: E): Boolean {
            if (!permits.tryAcquire()) return false
            return try {
                super.offer(element).also { added -> if (!added) permits.release() }
            } catch (throwable: Throwable) {
                permits.release()
                throw throwable
            }
        }

        override fun poll(): E? = super.poll()?.also { permits.release() }

        override fun poll(timeout: Long, unit: TimeUnit): E? =
            super.poll(timeout, unit)?.also { permits.release() }

        override fun take(): E = super.take().also { permits.release() }

        override fun remove(element: E?): Boolean =
            super.remove(element).also { removed -> if (removed) permits.release() }

        override fun clear() {
            while (poll() != null) Unit
        }

        override fun drainTo(target: MutableCollection<in E>): Int =
            super.drainTo(target).also(permits::release)

        override fun drainTo(target: MutableCollection<in E>, maxElements: Int): Int =
            super.drainTo(target, maxElements).also(permits::release)
    }
    private data class PolicyState(
        var submittedCount: Int = 0,
        val pending: java.util.ArrayDeque<PendingRequest> = java.util.ArrayDeque()
    )

    private val cache = object : LruCache<CacheKey, DecodedBitmapLease>(32 * 1024 * 1024) {
        override fun sizeOf(key: CacheKey, value: DecodedBitmapLease): Int =
            saturatingAdd(value.byteCount, key.digest.size.toLong() + CACHE_ENTRY_OVERHEAD_BYTES)
                .coerceAtMost(Int.MAX_VALUE.toLong())
                .toInt()

        override fun entryRemoved(
            evicted: Boolean,
            key: CacheKey,
            oldValue: DecodedBitmapLease,
            newValue: DecodedBitmapLease?,
        ) {
            if (oldValue !== newValue) oldValue.close()
        }
    }

    /** Counts the allocation once even when cache and mounted leases share it. */
    private fun decodedAllocationBytes(bitmap: Bitmap): Long {
        val allocation = runCatching { bitmap.allocationByteCount.toLong() }.getOrNull()
        if (allocation != null && allocation >= 0) return allocation
        val pixels = bitmap.width.coerceAtLeast(0).toLong()
        val rows = bitmap.height.coerceAtLeast(0).toLong()
        return if (pixels == 0L || rows == 0L) 0L
        else if (pixels > Long.MAX_VALUE / rows || pixels * rows > Long.MAX_VALUE / 4L) Long.MAX_VALUE
        else pixels * rows * 4L
    }

    private fun saturatingAdd(left: Long, right: Long): Long =
        if (right > 0 && left > Long.MAX_VALUE - right) Long.MAX_VALUE else left + right
    private val lock = Any()
    private val inFlight = mutableMapOf<RequestKey, PendingRequest>()
    private val policyStates = mutableMapOf<ImageLoadingPolicy, PolicyState>()
    private val readyToSubmit = java.util.ArrayDeque<PendingRequest>()
    private val rejectionLock = Any()
    private val rejectionNotifications = java.util.ArrayDeque<Callback>()
    private var rejectionDrainPosted = false
    private var admissionCount = 0
    private val nextCallbackId = AtomicLong()
    private val submissionRejectionCount = AtomicLong()
    private val submissionSequence = AtomicLong()
    private val digestConstructionCount = AtomicLong()
    private val mainHandler by lazy { Handler(Looper.getMainLooper()) }
    private var globalExecutor = createGlobalExecutor()
    private val timeoutScheduler = Executors.newSingleThreadScheduledExecutor { runnable ->
        Thread(runnable, "native-editor-image-deadline").apply { isDaemon = true }
    }

    init {
        DecodedBitmapBudget.shared().setPressureHandler {
            synchronized(cache) { cache.evictAll() }
        }
    }

    @Volatile
    internal var decodeSourceOverride: ((String, ImageLoadingPolicy) -> Bitmap?)? = null

    @Volatile
    internal var beforeWorkerReturnOverride: ((String) -> Unit)? = null

    @Volatile
    internal var deadlineExecutionGateOverride: (() -> Unit)? = null

    @Volatile
    internal var beforeCacheCommitOverride: (() -> Unit)? = null

    @Volatile
    internal var beforeTerminalClaimOverride: (() -> Unit)? = null

    @Volatile
    internal var decodedDeliveryPostedOverride: (() -> Unit)? = null

    @Volatile
    internal var monotonicClockOverride: MonotonicClock? = null

    @Volatile
    internal var beforeDigestOverride: (() -> Unit)? = null

    internal fun isCachedForTesting(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
    ): Boolean = prepare(source, policy)?.let {
        synchronized(cache) { cache.get(cacheKey(it.source, it.policy)) != null }
    } ?: false

    internal fun prepare(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
    ): PreparedSource? {
        if (source.regionMatches(0, "data:image/", 0, "data:image/".length, ignoreCase = true) &&
            RenderImageDecoder.preflightDataUrl(source, policy) == null
        ) return null
        return PreparedSource(source, policy)
    }

    internal fun cacheKeyByteCountForTesting(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
    ): Int = cacheKey(source, policy).digest.size

    internal fun cacheEntryCountForTesting(): Int = synchronized(cache) { cache.snapshot().size }

    internal fun cacheRetainedCostForTesting(): Int = synchronized(cache) { cache.size() }

    internal fun digestConstructionCountForTesting(): Long = digestConstructionCount.get()

    internal fun resetForTesting() {
        synchronized(cache) {
            cache.evictAll()
        }
        val pending: List<PendingRequest>
        val executor: ThreadPoolExecutor
        synchronized(lock) {
            pending = inFlight.values.toList()
            inFlight.clear()
            policyStates.clear()
            readyToSubmit.clear()
            admissionCount = 0
            executor = globalExecutor
            globalExecutor = createGlobalExecutor()
        }
        pending.forEach { request ->
            runCatching { request.cancellation.cancel() }
            runCatching { request.future?.cancel(true) }
            runCatching { request.timeoutFuture?.cancel(false) }
        }
        executor.shutdownNow()
        val rejected = synchronized(rejectionLock) {
            rejectionNotifications.toList().also {
                rejectionNotifications.clear()
                rejectionDrainPosted = false
            }
        }
        rejected.forEach { it.handle.finish() }
        submissionRejectionCount.set(0)
        digestConstructionCount.set(0)
        decodeSourceOverride = null
        beforeWorkerReturnOverride = null
        deadlineExecutionGateOverride = null
        beforeCacheCommitOverride = null
        beforeTerminalClaimOverride = null
        decodedDeliveryPostedOverride = null
        monotonicClockOverride = null
        beforeDigestOverride = null
    }

    internal fun executionResourceCountForTesting(): Int = 1

    internal fun globalQueuedTaskCountForTesting(): Int = globalExecutor.queue.size

    internal fun globalQueueLimitForTesting(): Int = GLOBAL_QUEUE_CAPACITY

    internal fun globalActiveWorkerCountForTesting(): Int = globalExecutor.activeCount

    internal fun globalWorkerLimitForTesting(): Int = GLOBAL_WORKERS

    internal fun globalAdmissionCountForTesting(): Int = synchronized(lock) { admissionCount }

    internal fun globalAdmissionLimitForTesting(): Int = GLOBAL_ADMISSION_LIMIT

    internal fun rejectionNotificationCountForTesting(): Int =
        synchronized(rejectionLock) { rejectionNotifications.size }

    internal fun rejectionNotificationLimitForTesting(): Int = REJECTION_NOTIFICATION_LIMIT

    internal fun submissionRejectionCountForTesting(): Long = submissionRejectionCount.get()

    internal fun loadLease(
        source: String,
        policy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT,
        ownerId: Long,
        priority: DecodedBitmapPriority,
        onLoaded: (DecodedBitmapLease?) -> Unit,
    ): LoadHandle = loadInternal(
        source,
        policy,
        null,
        ownerId,
        priority,
        onLoaded,
    )

    internal fun loadLease(
        prepared: PreparedSource,
        ownerId: Long,
        priority: DecodedBitmapPriority,
        onLoaded: (DecodedBitmapLease?) -> Unit,
    ): LoadHandle = loadInternal(
        prepared.source,
        prepared.policy,
        prepared,
        ownerId,
        priority,
        onLoaded,
    )

    private fun loadInternal(
        source: String,
        policy: ImageLoadingPolicy,
        prepared: PreparedSource?,
        ownerId: Long?,
        priority: DecodedBitmapPriority,
        onLoaded: (DecodedBitmapLease?) -> Unit,
    ): LoadHandle {
        val cancelled = AtomicBoolean(false)
        var requestKey: RequestKey? = null
        lateinit var callback: Callback
        val handle = LoadHandle {
            cancelled.set(true)
            requestKey?.let { cancelCallback(it, callback) }
        }
        callback = Callback(
            nextCallbackId.incrementAndGet(),
            cancelled,
            AtomicBoolean(false),
            handle,
            ownerId,
            policy.maxDecodedBytes.toLong(),
            priority,
            onLoaded,
        )
        val admitted = synchronized(lock) {
            if (admissionCount >= GLOBAL_ADMISSION_LIMIT) {
                false
            } else {
                admissionCount += 1
                true
            }
        }
        if (!admitted) {
            enqueueRejectionNotification(callback)
            return handle
        }
        handle.onFinished {
            releaseAdmission(callback)
        }
        val requestedAtMs = monotonicNowMs()
        val deadlineMs = deadlineAfter(requestedAtMs, policy.requestTimeoutMs)
        val resolvedSource = prepared ?: prepare(source, policy)
        if (resolvedSource == null) {
            releaseAdmission(callback)
            postCallbacks(listOf(callback), null)
            return handle
        }
        if (monotonicNowMs() >= deadlineMs) {
            releaseAdmission(callback)
            postCallbacks(listOf(callback), null)
            return handle
        }
        val resolvedRequestKey = RequestKey(
            cacheKey(resolvedSource.source, resolvedSource.policy),
            resolvedSource.policy
        )
        requestKey = resolvedRequestKey
        if (monotonicNowMs() >= deadlineMs) {
            releaseAdmission(callback)
            postCallbacks(listOf(callback), null)
            return handle
        }
        synchronized(cache) {
            cache.get(resolvedRequestKey.digest)?.let { cached -> leaseForCallback(cached, callback) }
        }?.let { lease ->
            scheduleCachedDelivery(callback, lease, deadlineMs)
            return handle
        }
        var drain = false
        var reject = false
        var createdRequest: PendingRequest? = null
        synchronized(lock) {
            val existing = inFlight[resolvedRequestKey]
            if (existing != null) {
                existing.callbacks += callback
                if (
                    callback.priority == DecodedBitmapPriority.VISIBLE &&
                    existing.priority != DecodedBitmapPriority.VISIBLE
                ) {
                    existing.priority = DecodedBitmapPriority.VISIBLE
                    promoteRequestLocked(existing)
                }
            } else {
                val pending = PendingRequest(
                    key = resolvedRequestKey,
                    source = source,
                    callbacks = mutableListOf(callback),
                    cancellation = RenderImageDecoder.Cancellation(),
                    startedAtMs = requestedAtMs
                )
                val state = policyStates.getOrPut(policy) { PolicyState() }
                when {
                    state.submittedCount < policy.maxConcurrentRequests -> {
                        createdRequest = pending
                        inFlight[resolvedRequestKey] = pending
                        state.submittedCount += 1
                        pending.submitted = true
                        enqueueReadyLocked(pending)
                        drain = true
                    }
                    state.pending.size < policy.maxPendingRequests -> {
                        createdRequest = pending
                        inFlight[resolvedRequestKey] = pending
                        if (pending.priority == DecodedBitmapPriority.VISIBLE) {
                            state.pending.addFirst(pending)
                        } else {
                            state.pending.addLast(pending)
                        }
                    }
                    else -> reject = true
                }
            }
        }
        createdRequest?.let(::scheduleDeadline)
        if (reject) {
            enqueueRejectionNotification(callback)
        } else if (drain) {
            drainSubmissions()
        }
        return handle
    }

    private fun promoteRequestLocked(request: PendingRequest) {
        val state = policyStates[request.key.policy]
        if (!request.submitted) {
            if (state?.pending?.remove(request) == true) state.pending.addFirst(request)
            return
        }
        if (readyToSubmit.remove(request)) {
            readyToSubmit.addFirst(request)
            return
        }
        val future = request.future ?: return
        if (!request.started.get() && globalExecutor.remove(future as Runnable)) {
            try {
                globalExecutor.execute(future)
            } catch (_: RejectedExecutionException) {
                submissionRejectionCount.incrementAndGet()
                request.future = null
                enqueueReadyLocked(request)
                mainHandler.post { drainSubmissions() }
            }
        }
    }

    private fun enqueueReadyLocked(request: PendingRequest) {
        if (request.priority == DecodedBitmapPriority.VISIBLE) readyToSubmit.addFirst(request)
        else readyToSubmit.addLast(request)
    }

    private fun drainSubmissions() {
        while (true) {
            val request = synchronized(lock) {
                readyToSubmit.pollFirst()?.also { it.dispatching = true }
            } ?: return
            if (!submitRequest(request)) {
                synchronized(lock) {
                    request.dispatching = false
                    if (inFlight[request.key] === request) {
                        readyToSubmit.addFirst(request)
                    } else {
                        releaseRequestSlotLocked(request)
                    }
                }
                return
            }
        }
    }

    private fun submitRequest(request: PendingRequest): Boolean {
        try {
            val future = PrioritizedTask(
                request,
                submissionSequence.incrementAndGet(),
            ) {
                request.started.set(true)
                var bitmap: DecodedBitmapLease? = null
                try {
                    bitmap = decode(request)
                } catch (_: Exception) {
                    bitmap = null
                } catch (_: OutOfMemoryError) {
                    bitmap = null
                } finally {
                    completeDecodedRequest(request, bitmap)
                    beforeWorkerReturnOverride?.invoke(request.source)
                }
            }
            globalExecutor.execute(future)
            var orphaned = false
            synchronized(lock) {
                request.dispatching = false
                request.future = future
                orphaned = inFlight[request.key] !== request
            }
            if (orphaned) cancelOrphanedSubmission(request, future)
            return true
        } catch (_: RejectedExecutionException) {
            submissionRejectionCount.incrementAndGet()
            return false
        }
    }

    private fun cancelOrphanedSubmission(request: PendingRequest, future: Future<*>) {
        runCatching { request.cancellation.cancel() }
        val removed = !request.started.get() &&
            runCatching { globalExecutor.remove(future as Runnable) }.getOrDefault(false)
        if (removed) {
            runCatching { future.cancel(false) }
            if (!request.terminal.get()) {
                synchronized(lock) { releaseRequestSlotLocked(request) }
                mainHandler.post { drainSubmissions() }
            }
        } else if (request.started.get()) {
            runCatching { future.cancel(true) }
        }
    }

    private fun completeDecodedRequest(request: PendingRequest, bitmap: DecodedBitmapLease?) {
        synchronized(lock) {
            releaseRequestSlotLocked(request)
        }
        mainHandler.post { drainSubmissions() }
        if (request.terminal.get()) {
            bitmap?.close()
            return
        }
        if (bitmap == null || request.cancellation.isCancelled() ||
            monotonicNowMs() >= request.deadlineMs
        ) {
            claimRequestOutcome(request, null, deliverInline = false)
            return
        }
        decodedDeliveryPostedOverride?.invoke()
        if (!mainHandler.post {
                if (request.terminal.get()) return@post
                beforeCacheCommitOverride?.invoke()
                val deliverable = bitmap.takeIf {
                    !request.cancellation.isCancelled() &&
                        monotonicNowMs() < request.deadlineMs
                }
                beforeTerminalClaimOverride?.invoke()
                claimRequestOutcome(request, deliverable, deliverInline = true)
            }
        ) {
            claimRequestOutcome(request, null, deliverInline = false)
        }
    }

    private fun claimRequestOutcome(
        request: PendingRequest,
        candidateBitmap: DecodedBitmapLease?,
        deliverInline: Boolean
    ): Boolean {
        if (!request.terminal.compareAndSet(false, true)) {
            candidateBitmap?.close()
            return false
        }
        request.timeoutFuture?.cancel(false)
        val callbacks: List<Callback>
        synchronized(lock) {
            if (inFlight[request.key] === request) inFlight.remove(request.key)
            releaseRequestSlotLocked(request)
            callbacks = request.callbacks.toList()
        }
        val resolvedBitmap = candidateBitmap?.takeIf {
            !request.cancellation.isCancelled() &&
                monotonicNowMs() < request.deadlineMs &&
                callbacks.any { callback -> !callback.cancelled.get() }
        }
        if (resolvedBitmap == null) {
            candidateBitmap?.close()
            request.cancellation.cancel()
        }
        val deliveries = callbacks.map { callback ->
            callback to resolvedBitmap?.let { leaseForCallback(it, callback) }
        }
        if (resolvedBitmap != null) {
            synchronized(cache) { cache.put(request.key.digest, resolvedBitmap) }
        }
        callbacks.forEach(::releaseAdmission)
        if (deliverInline) {
            deliveries.forEach { (callback, lease) -> deliverCallback(callback, lease) }
            drainSubmissions()
        } else {
            postDeliveries(deliveries)
        }
        return true
    }

    private fun scheduleDeadline(request: PendingRequest) {
        val delayMs = (request.deadlineMs - monotonicNowMs()).coerceAtLeast(0L)
        val future = timeoutScheduler.schedule(
            {
                deadlineExecutionGateOverride?.invoke()
                expireRequest(request)
            },
            delayMs,
            TimeUnit.MILLISECONDS
        )
        request.timeoutFuture = future
        if (request.terminal.get()) future.cancel(false)
    }

    private fun scheduleCachedDelivery(
        callback: Callback,
        lease: DecodedBitmapLease,
        deadlineMs: Long,
    ) {
        val terminal = AtomicBoolean(false)
        val delayMs = (deadlineMs - monotonicNowMs()).coerceAtLeast(0L)
        val timeoutFuture = timeoutScheduler.schedule(
            {
                deadlineExecutionGateOverride?.invoke()
                if (terminal.compareAndSet(false, true)) {
                    lease.close()
                    releaseAdmission(callback)
                    postCallbacks(listOf(callback), null)
                }
            },
            delayMs,
            TimeUnit.MILLISECONDS
        )
        if (!mainHandler.post {
                if (!terminal.compareAndSet(false, true)) return@post
                timeoutFuture.cancel(false)
                releaseAdmission(callback)
                val result = lease.takeIf {
                    !callback.cancelled.get() && monotonicNowMs() < deadlineMs
                }
                if (result == null) lease.close()
                deliverCallback(callback, result)
            }
        ) {
            timeoutFuture.cancel(false)
            terminal.set(true)
            lease.close()
            callback.handle.finish()
        }
    }

    private fun expireRequest(request: PendingRequest) {
        if (!claimRequestOutcome(request, null, deliverInline = false)) return
        val future = request.future
        if (future != null) {
            if (!request.started.get()) runCatching { globalExecutor.remove(future as Runnable) }
            runCatching { future.cancel(true) }
        }
    }

    private fun postCallbacks(callbacks: List<Callback>, bitmap: DecodedBitmapLease?) {
        val deliveries = callbacks.map { callback ->
            callback to bitmap?.let { leaseForCallback(it, callback) }
        }
        postDeliveries(deliveries)
    }

    private fun postDeliveries(deliveries: List<Pair<Callback, DecodedBitmapLease?>>) {
        if (!mainHandler.post {
                deliveries.forEach { (callback, lease) -> deliverCallback(callback, lease) }
                drainSubmissions()
            }
        ) {
            deliveries.forEach { (callback, lease) ->
                lease?.close()
                callback.handle.finish()
            }
            drainSubmissions()
        }
    }

    private fun enqueueRejectionNotification(callback: Callback) {
        var postDrain = false
        val dropNotification = synchronized(rejectionLock) {
            if (rejectionNotifications.size >= REJECTION_NOTIFICATION_LIMIT) {
                true
            } else {
                rejectionNotifications.addLast(callback)
                if (!rejectionDrainPosted) {
                    rejectionDrainPosted = true
                    postDrain = true
                }
                false
            }
        }
        if (dropNotification) {
            // Rejection delivery itself is bounded. Once the retained main-thread batch
            // is full, shed the notification but always finish the handle. Calling the
            // consumer inline would violate async/main-thread delivery and enable retry
            // recursion under sustained overload.
            callback.handle.finish()
        } else if (postDrain && !mainHandler.post { drainRejectionNotifications() }) {
            // A stopped looper cannot honor the delivery contract; finish without
            // invoking consumer code on the caller thread.
            takeRejectionNotifications().forEach { it.handle.finish() }
        }
    }

    private fun drainRejectionNotifications() {
        takeRejectionNotifications().forEach { deliverCallback(it, null) }
    }

    private fun takeRejectionNotifications(): List<Callback> =
        synchronized(rejectionLock) {
            rejectionNotifications.toList().also {
                rejectionNotifications.clear()
                rejectionDrainPosted = false
            }
        }

    private fun deliverCallback(callback: Callback, lease: DecodedBitmapLease?) {
        try {
            if (!callback.cancelled.get()) callback.deliver(lease) else lease?.close()
        } catch (_: Exception) {
            lease?.close()
            // Consumer failures must not crash the delivery runnable or retain admission.
        } finally {
            callback.handle.finish()
        }
    }

    private fun releaseAdmission(callback: Callback) {
        if (!callback.admissionReleased.compareAndSet(false, true)) return
        synchronized(lock) {
            admissionCount = (admissionCount - 1).coerceAtLeast(0)
        }
    }

    private fun cancelCallback(key: RequestKey, callback: Callback) {
        callback.cancelled.set(true)
        var requestToCancel: PendingRequest? = null
        synchronized(lock) {
            val request = inFlight[key] ?: return@synchronized
            request.callbacks.removeAll { it.id == callback.id }
            if (request.callbacks.isNotEmpty()) return@synchronized
            if (!request.terminal.compareAndSet(false, true)) return@synchronized
            inFlight.remove(key)
            requestToCancel = request
            releaseRequestSlotLocked(request)
        }
        val request = requestToCancel ?: return
        runCatching { request.timeoutFuture?.cancel(false) }
        runCatching { request.cancellation.cancel() }
        val future = request.future
        if (future != null) {
            if (!request.started.get()) runCatching { globalExecutor.remove(future as Runnable) }
            runCatching { future.cancel(true) }
        }
        drainSubmissions()
    }

    private fun releaseRequestSlotLocked(request: PendingRequest) {
        if (!request.submitted) {
            policyStates[request.key.policy]?.pending?.remove(request)
            removePolicyStateIfEmptyLocked(request.key.policy)
            return
        }
        if (!request.workerSlotReleased.compareAndSet(false, true)) return
        readyToSubmit.remove(request)
        releaseSubmittedSlotLocked(request.key.policy)
    }

    private fun releaseSubmittedSlotLocked(policy: ImageLoadingPolicy) {
        val state = policyStates[policy] ?: return
        state.submittedCount = (state.submittedCount - 1).coerceAtLeast(0)
        val next = state.pending.pollFirst()
        if (next != null) {
            state.submittedCount += 1
            next.submitted = true
            enqueueReadyLocked(next)
        }
        removePolicyStateIfEmptyLocked(policy)
    }

    private fun removePolicyStateIfEmptyLocked(policy: ImageLoadingPolicy) {
        val state = policyStates[policy] ?: return
        if (state.submittedCount == 0 && state.pending.isEmpty()) policyStates.remove(policy)
    }

    private fun createGlobalExecutor() = object : ThreadPoolExecutor(
        GLOBAL_WORKERS,
        GLOBAL_WORKERS,
        30L,
        TimeUnit.SECONDS,
        BoundedPriorityQueue<Runnable>(GLOBAL_QUEUE_CAPACITY)
    ) {
        override fun afterExecute(runnable: Runnable?, throwable: Throwable?) {
            super.afterExecute(runnable, throwable)
            // A transient rejection is requeued. Signal again only after a worker task
            // has returned; posting to main lets the worker dequeue its next task first.
            mainHandler.post { drainSubmissions() }
        }
    }.apply { allowCoreThreadTimeOut(true) }

    private fun decode(request: PendingRequest): DecodedBitmapLease? {
        decodeSourceOverride?.let { override ->
            val bitmap = try {
                override(request.source, request.key.policy)
            } catch (_: OutOfMemoryError) {
                null
            } ?: return null
            val bytes = decodedAllocationBytes(bitmap)
            val reservation = DecodedBitmapBudget.shared().reserve(
                bytes,
                request.priority,
            ) ?: return null
            return reservation.commit(bitmap, bytes)
        }
        return RenderImageDecoder.decodeSourceLease(
                request.source,
                request.key.policy,
                request.cancellation,
                monotonicClockOverride ?: systemMonotonicClock,
                request.deadlineMs,
                request.priority,
            )
    }

    private fun leaseForCallback(
        lease: DecodedBitmapLease,
        callback: Callback,
    ): DecodedBitmapLease? = callback.ownerId?.let { ownerId ->
        lease.fork(ownerId, callback.ownerLimitBytes, callback.priority)
    } ?: lease.forkUnowned()

    private fun monotonicNowMs(): Long =
        (monotonicClockOverride ?: systemMonotonicClock).elapsedRealtime()

    private fun cacheKey(source: String, policy: ImageLoadingPolicy): CacheKey {
        digestConstructionCount.incrementAndGet()
        beforeDigestOverride?.invoke()
        val digest = MessageDigest.getInstance("SHA-256")
        digest.update(source.toByteArray(Charsets.UTF_8))
        digest.update(ImageLoadingPolicy.canonicalBytes(policy))
        return CacheKey(digest.digest())
    }
}
