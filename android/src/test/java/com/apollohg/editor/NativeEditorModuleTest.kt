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
internal class NativeEditorModuleTest : NativeEditorModuleTestFixture() {
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
    fun `transport created while host is detached stays detached until attach`() {
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
        NativeCollaborationTransportRegistry.detachHost(collaborationRuntimeToken)

        assertNull(
            NativeCollaborationTransportRegistry.configure(
                collaborationRuntimeToken,
                editorId,
                "{\"url\":\"wss://collab.example/room\",\"connect\":true}",
            )
        )
        NativeCollaborationTransportRegistry.awaitIdleForTesting(editorId)
        val transport = requireNotNull(
            NativeCollaborationTransportRegistry.identityForTesting(editorId)
        ) as AndroidCollaborationTransport
        assertEquals(
            AndroidCollaborationTransport.HostState.DETACHED,
            transport.hostStateForTesting(),
        )

        NativeCollaborationTransportRegistry.attachHost(collaborationRuntimeToken)
        NativeCollaborationTransportRegistry.awaitIdleForTesting(editorId)
        assertEquals(
            AndroidCollaborationTransport.HostState.FOREGROUND,
            transport.hostStateForTesting(),
        )
    }
}
