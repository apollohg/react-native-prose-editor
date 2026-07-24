package com.apollohg.editor

import android.os.Looper
import android.content.Context
import android.view.inputmethod.EditorInfo
import java.math.BigDecimal
import java.lang.ref.WeakReference
import expo.modules.core.ModuleRegistry
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.ModulesProvider
import expo.modules.kotlin.modules.Module
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.util.concurrent.atomic.AtomicBoolean
import uniffi.editor_core.FfiError
import uniffi.editor_core.FfiJsonResult
import uniffi.editor_core.FfiUnitResult

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativeEditorModuleTest {
    @Test
    fun `module create with both result sides cleans extractable session without registering it`() {
        val cleanupHandles = mutableListOf<String>()

        val result = createEditorV2FromModule(
            configJson = "{\"initialization\":{\"type\":\"localEmpty\"}}",
            snapshotState = null,
            create = { _, _ ->
                FfiJsonResult(
                    "{\"editorId\":\"900001\"}",
                    FfiError("engine", "FAILED", "malformed", null, null, null, null, null),
                )
            },
            destroy = { editorId ->
                cleanupHandles += editorId
                FfiUnitResult(true, null)
            },
        )

        val error = result["error"] as Map<*, *>
        assertEquals("boundary", error["domain"])
        assertEquals("FFI_RESULT_INVALID", error["code"])
        assertEquals(listOf("900001"), cleanupHandles)
        assertNull(EditorV2Registry.viewTokenForHandle("900001"))
    }

    @Test
    fun `module create with neither result side leaves no pairing or cleanup attempt`() {
        val cleanupHandles = mutableListOf<String>()

        val result = createEditorV2FromModule(
            configJson = "{\"initialization\":{\"type\":\"localEmpty\"}}",
            snapshotState = null,
            create = { _, _ -> FfiJsonResult(null, null) },
            destroy = { editorId ->
                cleanupHandles += editorId
                FfiUnitResult(true, null)
            },
        )

        val error = result["error"] as Map<*, *>
        assertEquals("FFI_RESULT_INVALID", error["code"])
        assertTrue(cleanupHandles.isEmpty())
        assertNull(EditorV2Registry.viewTokenForHandle("900002"))
    }

    @Test
    fun `module create with invalid value cleans extractable session without registering it`() {
        val cleanupHandles = mutableListOf<String>()

        val result = createEditorV2FromModule(
            configJson = "{\"initialization\":{\"type\":\"localEmpty\"}}",
            snapshotState = null,
            create = { _, _ -> FfiJsonResult("{\"editorId\":\"900003\",\"unexpected\":true}", null) },
            destroy = { editorId ->
                cleanupHandles += editorId
                FfiUnitResult(true, null)
            },
        )

        val error = result["error"] as Map<*, *>
        assertEquals("FFI_RESULT_INVALID", error["code"])
        assertEquals(listOf("900003"), cleanupHandles)
        assertNull(EditorV2Registry.viewTokenForHandle("900003"))
    }

    @Test
    fun `module destroy reserves before ffi and rolls back after retryable failure`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        NativeEditorViewRegistry.markEditorCreated(viewToken)
        var preparationDuringDestroy: String? = null

        try {
            val result = destroyEditorV2FromModule(adapter.editorId) {
                preparationDuringDestroy = NativeEditorViewRegistry.prepareForCommandJSON(viewToken)
                FfiUnitResult(
                    null,
                    FfiError("operation", "OPERATION_INVALID", "retryable", null, null, null, null, null),
                )
            }

            assertEquals("OPERATION_INVALID", result.error?.code)
            assertTrue(preparationDuringDestroy!!.contains("\"blockedReason\":\"destroyed\""))
            assertTrue(NativeEditorViewRegistry.prepareForCommandJSON(viewToken).contains("\"ready\":true"))
            assertEquals(viewToken, EditorV2Registry.viewTokenForHandle(adapter.editorId))
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(viewToken)
        }
    }

    @Test
    fun `throwing handle reservation hook does not strand destroy retry`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        NativeEditorViewRegistry.markEditorCreated(viewToken)
        var destroyAttempts = 0
        var finalizationCalls = 0
        EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = { handle ->
            if (handle == adapter.editorId) throw IllegalStateException("test hook failure")
        }
        NativeEditorViewRegistry.onFinalizeDestroyForTesting = { finalizedToken ->
            if (finalizedToken == viewToken) finalizationCalls += 1
        }

        try {
            val destroy: (String) -> FfiUnitResult = {
                destroyAttempts += 1
                if (destroyAttempts == 1) {
                    FfiUnitResult(
                        null,
                        FfiError("operation", "OPERATION_INVALID", "retryable", null, null, null, null, null),
                    )
                } else {
                    FfiUnitResult(true, null)
                }
            }

            val first = destroyEditorV2FromModule(adapter.editorId, destroy)
            assertEquals("retryable", first.error?.message)
            assertEquals(viewToken, EditorV2Registry.viewTokenForHandle(adapter.editorId))
            assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(adapter.editorId))
            assertTrue(NativeEditorViewRegistry.prepareForCommandJSON(viewToken).contains("\"ready\":true"))

            EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = null
            val retry = destroyEditorV2FromModule(adapter.editorId, destroy)
            assertEquals(true, retry.value)
            assertNull(retry.error)
            assertEquals(2, destroyAttempts)
            assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(adapter.editorId))
            assertEquals(1, finalizationCalls)
            assertFalse(NativeEditorViewRegistry.isDestroyed(viewToken))
        } finally {
            EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = null
            NativeEditorViewRegistry.onFinalizeDestroyForTesting = null
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(viewToken)
        }
    }

    @Test
    fun `throwing pair removal hook preserves terminal result and finalizes destroy`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        NativeEditorViewRegistry.markEditorCreated(viewToken)
        var destroyAttempts = 0
        var finalizationCalls = 0
        EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = { handle ->
            if (handle == adapter.editorId) throw IllegalStateException("test hook failure")
        }
        NativeEditorViewRegistry.onFinalizeDestroyForTesting = { finalizedToken ->
            if (finalizedToken == viewToken) finalizationCalls += 1
        }

        try {
            val destroy: (String) -> FfiUnitResult = {
                destroyAttempts += 1
                FfiUnitResult(true, null)
            }

            val result = destroyEditorV2FromModule(adapter.editorId, destroy)
            assertEquals(true, result.value)
            assertNull(result.error)
            assertNull(EditorV2Registry.viewTokenForHandle(adapter.editorId))
            assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(adapter.editorId))
            assertEquals(1, finalizationCalls)
            assertFalse(NativeEditorViewRegistry.isDestroyed(viewToken))

            EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = null
            val subsequent = destroyEditorV2FromModule(adapter.editorId, destroy)
            assertEquals(true, subsequent.value)
            assertNull(subsequent.error)
            assertEquals(2, destroyAttempts)
            assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(adapter.editorId))
        } finally {
            EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = null
            NativeEditorViewRegistry.onFinalizeDestroyForTesting = null
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(viewToken)
        }
    }

    @Test
    fun `handle transaction blocks contender before pairing lookup`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        NativeEditorViewRegistry.markEditorCreated(viewToken)
        val reservationAcquired = java.util.concurrent.CountDownLatch(1)
        val releaseOwner = java.util.concurrent.CountDownLatch(1)
        val ownerFinished = java.util.concurrent.CountDownLatch(1)
        val destroyAttempts = java.util.concurrent.atomic.AtomicInteger(0)
        EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = { handle ->
            if (handle == adapter.editorId) {
                reservationAcquired.countDown()
                releaseOwner.await()
            }
        }

        try {
            val destroy: (String) -> FfiUnitResult = {
                destroyAttempts.incrementAndGet()
                FfiUnitResult(true, null)
            }
            val owner = Thread {
                val result = destroyEditorV2FromModule(adapter.editorId, destroy)
                assertEquals(true, result.value)
                assertNull(result.error)
                ownerFinished.countDown()
            }
            owner.start()
            assertTrue(reservationAcquired.await(1, java.util.concurrent.TimeUnit.SECONDS))

            val contender = destroyEditorV2FromModule(adapter.editorId, destroy)
            assertNull(contender.value)
            assertEquals("operation", contender.error?.domain)
            assertEquals("OPERATION_INVALID", contender.error?.code)
            assertEquals("destroy already in progress", contender.error?.message)
            assertEquals(0, destroyAttempts.get())

            releaseOwner.countDown()
            assertTrue(ownerFinished.await(1, java.util.concurrent.TimeUnit.SECONDS))
            assertEquals(1, destroyAttempts.get())
            assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(adapter.editorId))
        } finally {
            EditorV2Registry.onHandleDestroyReservationAcquiredForTesting = null
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(viewToken)
        }
    }

    @Test
    fun `handle transaction blocks contender after ffi and after pairing removal`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        NativeEditorViewRegistry.markEditorCreated(viewToken)
        val ffiReturned = java.util.concurrent.CountDownLatch(1)
        val releaseAfterFfi = java.util.concurrent.CountDownLatch(1)
        val pairRemoved = java.util.concurrent.CountDownLatch(1)
        val releaseAfterPairRemoval = java.util.concurrent.CountDownLatch(1)
        val ownerFinished = java.util.concurrent.CountDownLatch(1)
        val destroyAttempts = java.util.concurrent.atomic.AtomicInteger(0)
        EditorV2Registry.onDestroyFfiResultReceivedForTesting = { handle ->
            if (handle == adapter.editorId) {
                ffiReturned.countDown()
                releaseAfterFfi.await()
            }
        }
        EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = { handle ->
            if (handle == adapter.editorId) {
                pairRemoved.countDown()
                releaseAfterPairRemoval.await()
            }
        }

        try {
            val destroy: (String) -> FfiUnitResult = {
                destroyAttempts.incrementAndGet()
                FfiUnitResult(true, null)
            }
            val owner = Thread {
                val result = destroyEditorV2FromModule(adapter.editorId, destroy)
                assertEquals(true, result.value)
                assertNull(result.error)
                ownerFinished.countDown()
            }
            owner.start()
            assertTrue(ffiReturned.await(1, java.util.concurrent.TimeUnit.SECONDS))

            val afterFfi = destroyEditorV2FromModule(adapter.editorId, destroy)
            assertEquals("OPERATION_INVALID", afterFfi.error?.code)
            assertEquals("destroy already in progress", afterFfi.error?.message)
            assertEquals(1, destroyAttempts.get())

            releaseAfterFfi.countDown()
            assertTrue(pairRemoved.await(1, java.util.concurrent.TimeUnit.SECONDS))
            assertNull(EditorV2Registry.viewTokenForHandle(adapter.editorId))

            val afterPairRemoval = destroyEditorV2FromModule(adapter.editorId, destroy)
            assertEquals("OPERATION_INVALID", afterPairRemoval.error?.code)
            assertEquals("destroy already in progress", afterPairRemoval.error?.message)
            assertEquals(1, destroyAttempts.get())

            releaseAfterPairRemoval.countDown()
            assertTrue(ownerFinished.await(1, java.util.concurrent.TimeUnit.SECONDS))
            assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(adapter.editorId))
        } finally {
            EditorV2Registry.onDestroyFfiResultReceivedForTesting = null
            EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting = null
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(viewToken)
        }
    }

    @Test
    fun `handle transaction preserves lifecycle terminal result for paired and unpaired editors`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        NativeEditorViewRegistry.markEditorCreated(viewToken)
        val lifecycle = FfiError(
            "lifecycle",
            "ENGINE_DESTROYED",
            "already destroyed by the engine",
            "request-7",
            "3",
            null,
            null,
            "{\"source\":\"test\"}",
        )

        try {
            val paired = destroyEditorV2FromModule(adapter.editorId) {
                FfiUnitResult(null, lifecycle)
            }
            assertNull(paired.value)
            assertEquals(lifecycle, paired.error)
            assertNull(EditorV2Registry.viewTokenForHandle(adapter.editorId))

            val unpairedHandle = "9000111"
            val unpaired = destroyEditorV2FromModule(unpairedHandle) {
                FfiUnitResult(null, lifecycle)
            }
            assertNull(unpaired.value)
            assertEquals(lifecycle, unpaired.error)
            assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(unpairedHandle))
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(viewToken)
        }
    }

    @Test
    fun `unpaired handle transaction matches paired rollback and retry contention`() {
        val handle = "9000112"
        val ffiEntered = java.util.concurrent.CountDownLatch(1)
        val releaseOwner = java.util.concurrent.CountDownLatch(1)
        val ownerFinished = java.util.concurrent.CountDownLatch(1)
        val destroyAttempts = java.util.concurrent.atomic.AtomicInteger(0)
        val ownerResult = java.util.concurrent.atomic.AtomicReference<FfiUnitResult>()
        val destroy: (String) -> FfiUnitResult = { editorId ->
            assertEquals(handle, editorId)
            if (destroyAttempts.incrementAndGet() == 1) {
                ffiEntered.countDown()
                releaseOwner.await()
                FfiUnitResult(
                    null,
                    FfiError(
                        "operation",
                        "OPERATION_INVALID",
                        "owner retryable destroy failure",
                        null,
                        null,
                        null,
                        null,
                        null,
                    ),
                )
            } else {
                FfiUnitResult(true, null)
            }
        }

        val owner = Thread {
            ownerResult.set(destroyEditorV2FromModule(handle, destroy))
            ownerFinished.countDown()
        }
        owner.start()
        assertTrue(ffiEntered.await(1, java.util.concurrent.TimeUnit.SECONDS))

        val contender = destroyEditorV2FromModule(handle, destroy)
        assertNull(contender.value)
        assertEquals("operation", contender.error?.domain)
        assertEquals("OPERATION_INVALID", contender.error?.code)
        assertEquals("destroy already in progress", contender.error?.message)
        assertEquals(1, destroyAttempts.get())

        releaseOwner.countDown()
        assertTrue(ownerFinished.await(1, java.util.concurrent.TimeUnit.SECONDS))
        assertEquals("owner retryable destroy failure", ownerResult.get().error?.message)
        assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle))

        val retry = destroyEditorV2FromModule(handle, destroy)
        assertEquals(true, retry.value)
        assertNull(retry.error)
        assertEquals(2, destroyAttempts.get())
        assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(handle))
    }

    @Test
    fun `module destroy contention returns retryable error then owner success finalizes once`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        NativeEditorViewRegistry.markEditorCreated(viewToken)
        var finalizationChecks = 0
        NativeEditorViewRegistry.onFinalizeDestroyForTesting = { finalizedToken ->
            if (finalizedToken == viewToken) {
                finalizationChecks += 1
                assertNull(EditorV2Registry.viewTokenForHandle(adapter.editorId))
                assertNull(EditorV2Registry.adapterForViewToken(viewToken))
                assertTrue(NativeEditorViewRegistry.isDestroyed(viewToken))
                assertTrue(
                    NativeEditorViewRegistry.prepareForCommandJSON(viewToken)
                        .contains("\"ready\":false"),
                )
            }
        }
        val firstFfiEntered = java.util.concurrent.CountDownLatch(1)
        val releaseFirstFfi = java.util.concurrent.CountDownLatch(1)
        val firstDestroyFinished = java.util.concurrent.CountDownLatch(1)
        val destroyAttempts = java.util.concurrent.atomic.AtomicInteger(0)
        val ownerResult = java.util.concurrent.atomic.AtomicReference<FfiUnitResult>()

        val destroy: (String) -> FfiUnitResult = {
            if (destroyAttempts.incrementAndGet() == 1) {
                firstFfiEntered.countDown()
                releaseFirstFfi.await()
            }
            FfiUnitResult(true, null)
        }

        try {
            val worker = Thread {
                ownerResult.set(destroyEditorV2FromModule(adapter.editorId, destroy))
                firstDestroyFinished.countDown()
            }
            worker.start()
            assertTrue(firstFfiEntered.await(1, java.util.concurrent.TimeUnit.SECONDS))

            val second = destroyEditorV2FromModule(adapter.editorId, destroy)
            assertNull(second.value)
            assertEquals("operation", second.error?.domain)
            assertEquals("OPERATION_INVALID", second.error?.code)
            assertEquals("destroy already in progress", second.error?.message)
            assertEquals(1, destroyAttempts.get())

            releaseFirstFfi.countDown()
            assertTrue(firstDestroyFinished.await(1, java.util.concurrent.TimeUnit.SECONDS))

            assertEquals(true, ownerResult.get().value)
            assertNull(ownerResult.get().error)
            assertEquals(1, destroyAttempts.get())
            assertEquals(1, finalizationChecks)
            assertNull(EditorV2Registry.viewTokenForHandle(adapter.editorId))
        } finally {
            NativeEditorViewRegistry.onFinalizeDestroyForTesting = null
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(viewToken)
        }
    }

    @Test
    fun `module destroy contention allows retry after owner rollback`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        NativeEditorViewRegistry.markEditorCreated(viewToken)
        val firstFfiEntered = java.util.concurrent.CountDownLatch(1)
        val releaseFirstFfi = java.util.concurrent.CountDownLatch(1)
        val firstDestroyFinished = java.util.concurrent.CountDownLatch(1)
        val destroyAttempts = java.util.concurrent.atomic.AtomicInteger(0)
        val ownerResult = java.util.concurrent.atomic.AtomicReference<FfiUnitResult>()

        val destroy: (String) -> FfiUnitResult = {
            val attempt = destroyAttempts.incrementAndGet()
            if (attempt == 1) {
                firstFfiEntered.countDown()
                releaseFirstFfi.await()
                FfiUnitResult(
                    null,
                    FfiError(
                        "operation",
                        "OPERATION_INVALID",
                        "owner retryable destroy failure",
                        null,
                        null,
                        null,
                        null,
                        null,
                    ),
                )
            } else {
                FfiUnitResult(true, null)
            }
        }

        try {
            val worker = Thread {
                ownerResult.set(destroyEditorV2FromModule(adapter.editorId, destroy))
                firstDestroyFinished.countDown()
            }
            worker.start()
            assertTrue(firstFfiEntered.await(1, java.util.concurrent.TimeUnit.SECONDS))

            val contention = destroyEditorV2FromModule(adapter.editorId, destroy)
            assertNull(contention.value)
            assertEquals("operation", contention.error?.domain)
            assertEquals("OPERATION_INVALID", contention.error?.code)
            assertEquals("destroy already in progress", contention.error?.message)
            assertEquals(1, destroyAttempts.get())

            releaseFirstFfi.countDown()
            assertTrue(firstDestroyFinished.await(1, java.util.concurrent.TimeUnit.SECONDS))
            assertEquals("owner retryable destroy failure", ownerResult.get().error?.message)
            assertEquals(viewToken, EditorV2Registry.viewTokenForHandle(adapter.editorId))
            assertFalse(NativeEditorViewRegistry.isDestroyed(viewToken))
            assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(adapter.editorId))

            val retry = destroyEditorV2FromModule(adapter.editorId, destroy)
            assertEquals(true, retry.value)
            assertNull(retry.error)
            assertEquals(2, destroyAttempts.get())
            assertNull(EditorV2Registry.viewTokenForHandle(adapter.editorId))
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(viewToken)
        }
    }

    @Test
    fun `destroy reservation acquisition classifies contention atomically`() {
        val editorId = 9_000_007L
        NativeEditorViewRegistry.markEditorCreated(editorId)
        try {
            assertEquals(
                NativeEditorDestroyReservationResult.RESERVED,
                NativeEditorViewRegistry.acquireDestroyReservation(editorId),
            )
            assertEquals(
                NativeEditorDestroyReservationResult.ALREADY_IN_PROGRESS,
                NativeEditorViewRegistry.acquireDestroyReservation(editorId),
            )
            NativeEditorViewRegistry.rollbackDestroy(editorId)
            assertEquals(
                NativeEditorDestroyReservationResult.RESERVED,
                NativeEditorViewRegistry.acquireDestroyReservation(editorId),
            )
        } finally {
            NativeEditorViewRegistry.rollbackDestroy(editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
        }
    }

    @Test
    fun `module destroy retains a pairing after ffi failure and drops it after retry succeeds`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        var destroyAttempts = 0

        try {
            val first = destroyEditorV2FromModule(adapter.editorId) {
                destroyAttempts += 1
                FfiUnitResult(
                    null,
                    FfiError(
                        "operation",
                        "OPERATION_INVALID",
                        "temporary destroy failure",
                        null,
                        null,
                        null,
                        null,
                        null,
                    ),
                )
            }

            assertEquals("OPERATION_INVALID", first.error?.code)
            assertEquals(viewToken, EditorV2Registry.viewTokenForHandle(adapter.editorId))
            assertTrue(EditorV2Registry.adapterForViewToken(viewToken) === adapter)
            assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(adapter.editorId))

            val second = destroyEditorV2FromModule(adapter.editorId) {
                destroyAttempts += 1
                FfiUnitResult(true, null)
            }

            assertEquals(true, second.value)
            assertNull(EditorV2Registry.viewTokenForHandle(adapter.editorId))
            assertNull(EditorV2Registry.adapterForViewToken(viewToken))
            assertEquals(2, destroyAttempts)
        } finally {
            EditorV2Registry.remove(adapter.editorId)
        }
    }

    @Test
    fun `module destroy with neither value nor error retains pairing until retry succeeds`() {
        assertMalformedDestroyResultRetainsPairUntilRetry(FfiUnitResult(null, null))
    }

    @Test
    fun `module destroy with both value and error retains pairing until retry succeeds`() {
        assertMalformedDestroyResultRetainsPairUntilRetry(
            FfiUnitResult(
                true,
                FfiError(
                    "lifecycle",
                    "ENGINE_DESTROYED",
                    "malformed destroy result",
                    null,
                    null,
                    null,
                    null,
                    null,
                ),
            ),
        )
    }

    private fun assertMalformedDestroyResultRetainsPairUntilRetry(
        malformedResult: FfiUnitResult,
    ) {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        var destroyAttempts = 0

        try {
            val first = destroyEditorV2FromModule(adapter.editorId) {
                destroyAttempts += 1
                malformedResult
            }

            assertNull(first.value)
            assertEquals("boundary", first.error?.domain)
            assertEquals("FFI_RESULT_INVALID", first.error?.code)
            assertEquals(viewToken, EditorV2Registry.viewTokenForHandle(adapter.editorId))
            assertTrue(EditorV2Registry.adapterForViewToken(viewToken) === adapter)

            val second = destroyEditorV2FromModule(adapter.editorId) {
                destroyAttempts += 1
                FfiUnitResult(true, null)
            }

            assertEquals(true, second.value)
            assertNull(EditorV2Registry.viewTokenForHandle(adapter.editorId))
            assertNull(EditorV2Registry.adapterForViewToken(viewToken))
            assertEquals(2, destroyAttempts)
        } finally {
            EditorV2Registry.remove(adapter.editorId)
        }
    }

    @Test
    fun `off main module destroy cancels queued adapter error before timeout cleanup drains`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"localEmpty\"},\"policy\":{\"readOnly\":true}}",
            null,
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false,
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        val errors = mutableListOf<Map<String, Any>>()
        val completed = AtomicBoolean(false)

        try {
            NativeEditorViewRegistry.markEditorCreated(viewToken)
            view.onEditorErrorForTesting = { errors += it }
            view.onAddonEventForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)

            val inputConnection = view.richTextView.editorEditText
                .onCreateInputConnection(EditorInfo())
            assertNotNull(inputConnection)
            assertTrue(inputConnection!!.commitText("x", 1))
            assertEquals(1, view.pendingEditorErrorEventCountForTesting())

            val worker = Thread {
                val result = destroyEditorV2FromModule(adapter.editorId) { editorId ->
                    assertEquals(adapter.editorId, editorId)
                    adapter.destroy()
                    FfiUnitResult(true, null)
                }
                assertEquals(true, result.value)
                completed.set(true)
            }
            worker.start()
            worker.join(1_000)

            assertFalse("module destroy must not deadlock waiting for main", worker.isAlive)
            assertTrue(completed.get())
            assertNotNull("owner release must defer view cleanup to main", view.editorErrorCallbackTokenForTesting())
            assertEquals(1, view.pendingEditorErrorEventCountForTesting())
            assertTrue(
                "the canonical handle stays owned until deferred view cleanup releases its reservation",
                EditorV2Registry.isHandleDestroyReservedForTesting(adapter.editorId),
            )
            var contenderFfiCalls = 0
            val contender = destroyEditorV2FromModule(adapter.editorId) {
                contenderFfiCalls += 1
                FfiUnitResult(true, null)
            }
            assertNull(contender.value)
            assertEquals("operation", contender.error?.domain)
            assertEquals("OPERATION_INVALID", contender.error?.code)
            assertEquals("destroy already in progress", contender.error?.message)
            assertEquals(0, contenderFfiCalls)

            shadowOf(Looper.getMainLooper()).idle()

            assertTrue(errors.isEmpty())
            assertEquals(0, view.pendingEditorErrorEventCountForTesting())
            assertEquals(0L, view.richTextView.editorId)
            assertFalse(EditorV2Registry.isHandleDestroyReservedForTesting(adapter.editorId))
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `v2 u32 parser admits only exact finite integral values`() {
        assertEquals(UInt.MAX_VALUE, exactV2U32(4_294_967_295L))
        assertEquals(0u, exactV2U32(0))
        for (value in listOf<Number>(
            -1,
            1.5,
            Double.NaN,
            Double.POSITIVE_INFINITY,
            4_294_967_296L,
            BigDecimal("1.0000000000000000001"),
        )) {
            assertNull("u32 $value must be rejected", exactV2U32(value))
        }
    }

    @Test
    fun `render probe flattens v2 render blocks into a render elements array`() {
        val update = JSONObject()
            .put(
                "renderBlocks",
                JSONArray()
                    .put(
                        JSONArray()
                            .put(JSONObject().put("type", "blockStart").put("nodeType", "paragraph"))
                            .put(JSONObject().put("type", "textRun").put("text", "Hello"))
                            .put(JSONObject().put("type", "blockEnd"))
                    )
                    .put(
                        JSONArray()
                            .put(JSONObject().put("type", "blockStart").put("nodeType", "paragraph"))
                            .put(JSONObject().put("type", "blockEnd"))
                    )
            )
            .toString()

        val elements = JSONArray(renderElementsJsonFromUpdate(update))

        assertEquals(5, elements.length())
        assertEquals("blockStart", elements.getJSONObject(0).getString("type"))
        assertEquals("Hello", elements.getJSONObject(1).getString("text"))
        assertEquals("blockEnd", elements.getJSONObject(4).getString("type"))
    }

    @Test
    fun `render probe passes through an already flat render elements payload`() {
        val flat = JSONArray()
            .put(JSONObject().put("type", "textRun").put("text", "Hi"))
        val update = JSONObject().put("renderElements", flat).toString()

        assertEquals(flat.toString(), renderElementsJsonFromUpdate(update))
    }

    @Test
    fun `render probe reports a boundary error when the update carries no render payload`() {
        val parsed = JSONObject(renderElementsJsonFromUpdate("{\"historyState\":{}}"))

        val error = parsed.getJSONObject("error")
        assertEquals("boundary", error.getString("domain"))
        assertEquals("FFI_RESULT_INVALID", error.getString("code"))
    }

    @Test
    fun `render probe reports a boundary error for invalid update json`() {
        val parsed = JSONObject(renderElementsJsonFromUpdate("not json"))

        assertTrue(parsed.getJSONObject("error").getString("message").isNotEmpty())
    }

    private data class TestExpoContext(
        val context: Context,
        val appContext: AppContext,
    )

    private fun testExpoContext(context: Context): TestExpoContext {
        val reactContext = Class
            .forName("com.facebook.react.bridge.BridgeReactContext")
            .getConstructor(Context::class.java)
            .newInstance(context) as Context
        val modulesProvider = object : ModulesProvider {
            override fun getModulesList(): List<Class<out Module>> = emptyList()
        }
        val constructor = AppContext::class.java.constructors.first { candidate ->
            candidate.parameterTypes.size == 3
        }
        val appContext = constructor.newInstance(
            modulesProvider,
            ModuleRegistry(emptyList(), emptyList()),
            WeakReference(reactContext),
        ) as AppContext
        return TestExpoContext(reactContext, appContext)
    }
}
