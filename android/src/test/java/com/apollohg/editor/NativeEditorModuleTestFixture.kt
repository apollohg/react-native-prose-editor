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

internal abstract class NativeEditorModuleTestFixture {
    protected var collaborationRuntimeToken =
        NativeCollaborationTransportRegistry.activateRuntime()

    @After
    fun resetCollaborationRegistry() {
        NativeCollaborationTransportRegistry.destroyRuntime(collaborationRuntimeToken)
        NativeCollaborationTransportRegistry.transportFactoryForTesting = null
        collaborationRuntimeToken = NativeCollaborationTransportRegistry.activateRuntime()
        NativeCollaborationTransportRegistry.attachHost(collaborationRuntimeToken)
    }

    protected fun assertMalformedDestroyResultRetainsPairUntilRetry(
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

    protected data class TestExpoContext(
        val context: Context,
        val appContext: AppContext,
    )

    protected fun neverSocketFactory() = object : CollaborationSocketFactory {
        override fun makeSocket(
            url: String,
            protocols: List<String>,
            callbacks: CollaborationSocketCallbacks,
        ): CollaborationSocket = error("socket must not connect")
    }

    protected fun containsOrgJsonValue(value: Any?): Boolean = when (value) {
        is JSONObject, is JSONArray -> true
        is Map<*, *> -> value.values.any(::containsOrgJsonValue)
        is List<*> -> value.any(::containsOrgJsonValue)
        else -> false
    }

    protected fun testExpoContext(context: Context): TestExpoContext {
        val reactContext = Class
            .forName("com.facebook.react.bridge.BridgeReactContext")
            .getConstructor(Context::class.java)
            .newInstance(context) as Context
        val modulesProvider = object : ModulesProvider {
            override fun getModulesMap(): Map<Class<out Module>, String?> = emptyMap()
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
