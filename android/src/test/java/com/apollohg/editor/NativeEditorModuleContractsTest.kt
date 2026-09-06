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
internal class NativeEditorModuleContractsTest : NativeEditorModuleTestFixture() {
    @Test
    fun `activity detach preserves collaboration identity sequence and emitter`() {
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
        val events = mutableListOf<Map<String, Any?>>()
        NativeCollaborationTransportRegistry.setEventEmitter(
            collaborationRuntimeToken,
            events::add,
        )
        assertNull(
            NativeCollaborationTransportRegistry.configure(
                collaborationRuntimeToken,
                editorId,
                "{\"url\":\"wss://collab.example/room\",\"connect\":false}",
            )
        )
        val identity = requireNotNull(
            NativeCollaborationTransportRegistry.identityForTesting(editorId)
        ) as AndroidCollaborationTransport
        val retainedConfig = identity.configForTesting()
        NativeCollaborationTransportRegistry.emitErrorForTesting(
            editorId,
            EditorV2Error("transport", "FIRST", "first"),
        )

        NativeCollaborationTransportRegistry.detachHost(collaborationRuntimeToken)
        NativeCollaborationTransportRegistry.attachHost(collaborationRuntimeToken)
        NativeCollaborationTransportRegistry.awaitIdleForTesting(editorId)
        NativeCollaborationTransportRegistry.emitErrorForTesting(
            editorId,
            EditorV2Error("transport", "SECOND", "second"),
        )

        assertEquals(identity, NativeCollaborationTransportRegistry.identityForTesting(editorId))
        assertEquals(retainedConfig, identity.configForTesting())
        assertTrue(NativeCollaborationTransportRegistry.hasEventEmitterForTesting())
        assertEquals(listOf("1", "2"), events.map { it["eventSequence"] })
    }

    @Test
    fun `collaboration state and peers recursively bridge nested JSON values`() {
        val state = jsonValueToJs(
            JSONObject()
                .put(
                    "nested",
                    JSONArray()
                        .put(JSONObject().put("values", JSONArray().put(1).put(JSONObject.NULL)))
                        .put(true),
                )
        ) as Map<*, *>
        val peers = jsonValueToJs(
            JSONArray().put(
                JSONObject().put(
                    "awareness",
                    JSONObject().put("cursor", JSONArray().put(4).put(7)),
                )
            )
        ) as List<*>

        assertEquals(
            listOf(mapOf("values" to listOf(1, null)), true),
            state["nested"],
        )
        assertEquals(
            listOf(mapOf("awareness" to mapOf("cursor" to listOf(4, 7)))),
            peers,
        )
        assertFalse(containsOrgJsonValue(state))
        assertFalse(containsOrgJsonValue(peers))
    }

    @Test
    fun `stale runtime lifecycle cannot mutate replacement runtime`() {
        val staleToken = collaborationRuntimeToken
        val currentToken = NativeCollaborationTransportRegistry.activateRuntime()
        collaborationRuntimeToken = currentToken
        NativeCollaborationTransportRegistry.attachHost(currentToken)

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
        val currentEvents = mutableListOf<Map<String, Any?>>()
        val staleEvents = mutableListOf<Map<String, Any?>>()
        NativeCollaborationTransportRegistry.setEventEmitter(currentToken, currentEvents::add)
        assertNull(
            NativeCollaborationTransportRegistry.configure(
                currentToken,
                editorId,
                "{\"url\":\"wss://collab.example/room\",\"connect\":false}",
            )
        )

        NativeCollaborationTransportRegistry.setEventEmitter(staleToken, staleEvents::add)
        assertEquals(
            "ENGINE_DESTROYED",
            NativeCollaborationTransportRegistry.configure(staleToken, editorId, null)?.code,
        )
        assertEquals(
            "ENGINE_DESTROYED",
            NativeCollaborationTransportRegistry.resolveProtocolAdapter(
                staleToken,
                editorId,
                "stale-attempt",
                "1",
                "{\"action\":\"reject\"}",
            )?.code,
        )
        NativeCollaborationTransportRegistry.detachHost(staleToken)
        NativeCollaborationTransportRegistry.destroyRuntime(staleToken)
        NativeCollaborationTransportRegistry.awaitIdleForTesting(editorId)
        val transport = requireNotNull(
            NativeCollaborationTransportRegistry.identityForTesting(editorId)
        ) as AndroidCollaborationTransport
        NativeCollaborationTransportRegistry.emitErrorForTesting(
            editorId,
            EditorV2Error("transport", "CURRENT", "current"),
        )

        assertTrue(NativeCollaborationTransportRegistry.containsForTesting(editorId))
        assertEquals(
            AndroidCollaborationTransport.HostState.FOREGROUND,
            transport.hostStateForTesting(),
        )
        assertEquals(1, currentEvents.size)
        assertTrue(staleEvents.isEmpty())
    }

    @Test
    fun `replacement waits for retired transport to detach`() {
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

        val replacementDone = CountDownLatch(1)
        val replacementError = AtomicReference<EditorV2Error?>()
        Thread {
            replacementError.set(
                NativeCollaborationTransportRegistry.configure(
                    collaborationRuntimeToken,
                    editorId,
                    config,
                )
            )
            replacementDone.countDown()
        }.start()

        assertFalse(replacementDone.await(100, TimeUnit.MILLISECONDS))
        release.countDown()
        assertTrue(replacementDone.await(2, TimeUnit.SECONDS))
        assertNull(replacementError.get())
        assertEquals(2, transports.size)
    }

    @Test
    fun `concurrent configure failure cannot remove a later success`() {
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
        val reattachEntered = CountDownLatch(1)
        val releaseReattach = CountDownLatch(1)
        backend.nextCollaborationReattachError = EditorV2Error(
            "transport",
            "REATTACH_FAILED",
            "retry",
        )
        backend.onCollaborationReattach = {
            backend.onCollaborationReattach = null
            reattachEntered.countDown()
            releaseReattach.await(2, TimeUnit.SECONDS)
        }
        val firstResult = AtomicReference<EditorV2Error?>()
        val secondResult = AtomicReference<EditorV2Error?>()
        val firstDone = CountDownLatch(1)
        val secondDone = CountDownLatch(1)
        Thread {
            firstResult.set(
                NativeCollaborationTransportRegistry.configure(
                    collaborationRuntimeToken,
                    editorId,
                    "{\"url\":\"wss://collab.example/first\",\"connect\":true}",
                )
            )
            firstDone.countDown()
        }.start()
        assertTrue(reattachEntered.await(2, TimeUnit.SECONDS))
        Thread {
            secondResult.set(
                NativeCollaborationTransportRegistry.configure(
                    collaborationRuntimeToken,
                    editorId,
                    "{\"url\":\"wss://collab.example/second\",\"connect\":true}",
                )
            )
            secondDone.countDown()
        }.start()

        releaseReattach.countDown()
        assertTrue(firstDone.await(2, TimeUnit.SECONDS))
        assertTrue(secondDone.await(2, TimeUnit.SECONDS))
        assertEquals("REATTACH_FAILED", firstResult.get()?.code)
        assertNull(secondResult.get())
        assertTrue(NativeCollaborationTransportRegistry.containsForTesting(editorId))
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
}
