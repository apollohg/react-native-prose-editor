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
import org.junit.After
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
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import uniffi.editor_core.FfiError
import uniffi.editor_core.FfiJsonResult
import uniffi.editor_core.FfiUnitResult

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class NativeEditorModuleDestroyTest : NativeEditorModuleTestFixture() {
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
    fun `module destroy retains collaboration transport for retryable and malformed results`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"room\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        NativeCollaborationTransportRegistry.transportFactoryForTesting = { id, sink ->
            AndroidCollaborationTransport(
                editorId = id,
                backend = backend,
                socketFactory = neverSocketFactory(),
                eventSink = sink,
            )
        }
        assertNull(
            NativeCollaborationTransportRegistry.configure(
                collaborationRuntimeToken,
                editorId,
                "{\"url\":\"wss://collab.example/room\",\"connect\":false}",
            )
        )
        val retryable = FfiUnitResult(
            null,
            FfiError("operation", "OPERATION_INVALID", "retry", null, null, null, null, null),
        )

        assertEquals(retryable, destroyEditorV2FromModule(editorId) { retryable })
        assertTrue(NativeCollaborationTransportRegistry.containsForTesting(editorId))
        val malformed = destroyEditorV2FromModule(editorId) {
            FfiUnitResult(true, retryable.error)
        }
        assertEquals("FFI_RESULT_INVALID", malformed.error?.code)
        assertTrue(NativeCollaborationTransportRegistry.containsForTesting(editorId))

        val terminal = destroyEditorV2FromModule(editorId) { FfiUnitResult(true, null) }
        assertEquals(true, terminal.value)
        assertFalse(NativeCollaborationTransportRegistry.containsForTesting(editorId))

        assertNull(
            NativeCollaborationTransportRegistry.configure(
                collaborationRuntimeToken,
                editorId,
                "{\"url\":\"wss://collab.example/room\",\"connect\":false}",
            )
        )
        val alreadyTerminal = destroyEditorV2FromModule(editorId) {
            FfiUnitResult(
                null,
                FfiError(
                    "lifecycle",
                    "ENGINE_DESTROYED",
                    "already destroyed",
                    null,
                    null,
                    null,
                    null,
                    null,
                ),
            )
        }
        assertEquals("ENGINE_DESTROYED", alreadyTerminal.error?.code)
        assertFalse(NativeCollaborationTransportRegistry.containsForTesting(editorId))
    }

    @Test
    fun `runtime destroy invalidates configure waiting on retirement`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            "{\"initialization\":{\"type\":\"room\"}}",
            null,
        ) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val transports = mutableListOf<AndroidCollaborationTransport>()
        NativeCollaborationTransportRegistry.transportFactoryForTesting = { id, sink ->
            AndroidCollaborationTransport(
                editorId = id,
                backend = backend,
                socketFactory = neverSocketFactory(),
                eventSink = sink,
            ).also(transports::add)
        }
        val config = "{\"url\":\"wss://collab.example/room\",\"connect\":true}"
        assertNull(
            NativeCollaborationTransportRegistry.configure(
                collaborationRuntimeToken,
                editorId,
                config,
            )
        )
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        transports.single().enqueueForTesting {
            entered.countDown()
            release.await(2, TimeUnit.SECONDS)
        }
        assertTrue(entered.await(2, TimeUnit.SECONDS))
        assertNull(
            NativeCollaborationTransportRegistry.configure(
                collaborationRuntimeToken,
                editorId,
                null,
            )
        )
        val replacementResult = AtomicReference<EditorV2Error?>()
        val replacementDone = CountDownLatch(1)
        Thread {
            replacementResult.set(
                NativeCollaborationTransportRegistry.configure(
                    collaborationRuntimeToken,
                    editorId,
                    config,
                )
            )
            replacementDone.countDown()
        }.start()

        NativeCollaborationTransportRegistry.destroyRuntime(collaborationRuntimeToken)
        release.countDown()
        assertTrue(replacementDone.await(2, TimeUnit.SECONDS))
        assertEquals("ENGINE_DESTROYED", replacementResult.get()?.code)
        assertFalse(NativeCollaborationTransportRegistry.containsForTesting(editorId))
        assertEquals(1, transports.size)
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
}
