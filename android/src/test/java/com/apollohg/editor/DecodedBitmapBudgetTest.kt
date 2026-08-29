package com.apollohg.editor

import android.graphics.Bitmap
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class DecodedBitmapBudgetTest {
    private val mib = 1024L * 1024L

    @Test
    fun `process ceiling scales with memory class and remains bounded`() {
        assertEquals(16L * mib, DecodedBitmapBudget.processLimitBytes(128))
        assertEquals(32L * mib, DecodedBitmapBudget.processLimitBytes(256))
        assertEquals(64L * mib, DecodedBitmapBudget.processLimitBytes(512))
        assertEquals(128L * mib, DecodedBitmapBudget.processLimitBytes(2_048))
    }

    @Test
    fun `shared allocation is charged globally once and per retaining owner once`() {
        val budget = DecodedBitmapBudget(16L * mib)
        val bitmap = Bitmap.createBitmap(32, 32, Bitmap.Config.ARGB_8888)
        val cache = requireNotNull(
            budget.reserve(4_096, DecodedBitmapPriority.PREFETCH)?.commit(bitmap, 4_096)
        )
        val first = requireNotNull(cache.fork(10, 8_192, DecodedBitmapPriority.VISIBLE))
        val duplicateForFirst = requireNotNull(cache.fork(10, 8_192, DecodedBitmapPriority.VISIBLE))
        val second = requireNotNull(cache.fork(20, 8_192, DecodedBitmapPriority.VISIBLE))

        assertEquals(4_096, budget.retainedProcessBytesForTesting())
        assertEquals(4_096, budget.retainedOwnerBytesForTesting(10))
        assertEquals(4_096, budget.retainedOwnerBytesForTesting(20))

        cache.close()
        assertEquals(4_096, budget.retainedProcessBytesForTesting())
        first.close()
        assertEquals(4_096, budget.retainedOwnerBytesForTesting(10))
        duplicateForFirst.close()
        assertEquals(0, budget.retainedOwnerBytesForTesting(10))
        second.close()
        assertEquals(0, budget.retainedProcessBytesForTesting())
    }

    @Test
    fun `owner and process ceilings reject without leaking reservations`() {
        val budget = DecodedBitmapBudget(4_096)
        val firstBitmap = Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888)
        val first = requireNotNull(
            budget.reserve(2_048, DecodedBitmapPriority.VISIBLE)?.commit(firstBitmap, 2_048)
        )
        assertNull(first.fork(1, 1_024, DecodedBitmapPriority.VISIBLE))
        assertNull(budget.reserve(2_049, DecodedBitmapPriority.PREFETCH))
        assertEquals(2_048, budget.retainedProcessBytesForTesting())
        first.close()
        assertEquals(0, budget.retainedProcessBytesForTesting())
    }

    @Test
    fun `cache pressure cannot invalidate a mounted child lease`() {
        val budget = DecodedBitmapBudget(4_096)
        val mountedBase = requireNotNull(
            budget.reserve(2_048, DecodedBitmapPriority.VISIBLE)?.commit(
                Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888),
                2_048,
            )
        )
        val mounted = requireNotNull(
            mountedBase.fork(1, 4_096, DecodedBitmapPriority.VISIBLE)
        )
        val cacheOnly = requireNotNull(
            budget.reserve(2_048, DecodedBitmapPriority.VISIBLE)?.commit(
                Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888),
                2_048,
            )
        )
        budget.setPressureHandler { cacheOnly.close() }

        val replacement = requireNotNull(
            budget.reserve(2_048, DecodedBitmapPriority.VISIBLE)?.commit(
                Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888),
                2_048,
            )
        )

        assertEquals(16, mounted.bitmap.width)
        assertEquals(4_096, budget.retainedProcessBytesForTesting())
        mountedBase.close()
        mounted.close()
        replacement.close()
        assertEquals(0, budget.retainedProcessBytesForTesting())
    }

    @Test
    fun `visible reservation sheds retained prefetch before failing process admission`() {
        val budget = DecodedBitmapBudget(4_096)
        val ownerId = 19L
        val cached = requireNotNull(
            budget.reserve(4_096, DecodedBitmapPriority.PREFETCH)?.commit(
                Bitmap.createBitmap(32, 32, Bitmap.Config.ARGB_8888),
                4_096,
            )
        )
        val prefetched = requireNotNull(
            cached.fork(ownerId, 4_096, DecodedBitmapPriority.PREFETCH)
        )
        budget.setPressureHandler { cached.close() }
        budget.setOwnerPressureHandler(ownerId) { prefetched.close() }

        val visible = requireNotNull(budget.reserve(4_096, DecodedBitmapPriority.VISIBLE))

        visible.close()
        assertEquals(0, budget.retainedProcessBytesForTesting())
    }

    @Test
    fun `visible owner admission can release retained prefetch ownership`() {
        val budget = DecodedBitmapBudget(8_192)
        val ownerId = 7L
        val prefetchBase = requireNotNull(
            budget.reserve(2_048, DecodedBitmapPriority.PREFETCH)?.commit(
                Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888),
                2_048,
            )
        )
        val prefetch = requireNotNull(
            prefetchBase.fork(ownerId, 2_048, DecodedBitmapPriority.PREFETCH)
        )
        budget.setOwnerPressureHandler(ownerId) { prefetch.close() }
        val visibleBase = requireNotNull(
            budget.reserve(2_048, DecodedBitmapPriority.VISIBLE)?.commit(
                Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888),
                2_048,
            )
        )

        val visible = requireNotNull(
            visibleBase.fork(ownerId, 2_048, DecodedBitmapPriority.VISIBLE)
        )

        assertEquals(2_048, budget.retainedOwnerBytesForTesting(ownerId))
        prefetchBase.close()
        visible.close()
        visibleBase.close()
        assertEquals(0, budget.retainedProcessBytesForTesting())
    }

    @Test
    fun `reservation reconciliation is checked and idempotently released`() {
        val budget = DecodedBitmapBudget(4_096)
        val reservation = requireNotNull(budget.reserve(1_024, DecodedBitmapPriority.VISIBLE))
        val bitmap = Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888)
        val lease = requireNotNull(reservation.commit(bitmap, 2_048))
        assertEquals(2_048, budget.retainedProcessBytesForTesting())
        lease.close()
        lease.close()
        reservation.close()
        assertEquals(0, budget.retainedProcessBytesForTesting())
    }

    @Test
    fun `overflowing reservations are rejected`() {
        val budget = DecodedBitmapBudget(Long.MAX_VALUE)
        val reservation = budget.reserve(Long.MAX_VALUE, DecodedBitmapPriority.VISIBLE)
        assertNotNull(reservation)
        assertNull(budget.reserve(1, DecodedBitmapPriority.VISIBLE))
        reservation?.close()
    }

    @Test
    fun `impossible visible reservations do not evict retained images`() {
        val budget = DecodedBitmapBudget(4_096)
        var cachePressureCount = 0
        var ownerPressureCount = 0
        budget.setPressureHandler { cachePressureCount += 1 }
        budget.setOwnerPressureHandler(1) { ownerPressureCount += 1 }

        assertNull(budget.reserve(4_097, DecodedBitmapPriority.VISIBLE))
        assertEquals(0, cachePressureCount)
        assertEquals(0, ownerPressureCount)
    }
}
