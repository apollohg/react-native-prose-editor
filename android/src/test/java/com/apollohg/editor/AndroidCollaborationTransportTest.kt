package com.apollohg.editor

import okio.ByteString
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Every teardown path must emit the close handshake that reaps peer presence. */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class AndroidCollaborationTransportTest {

    private class RecordingSocket(
        private val callbacks: CollaborationSocketCallbacks,
        private val closeSucceeds: Boolean,
    ) : CollaborationSocket {
        val events = mutableListOf<String>()

        override fun connect() {
            events.add("connect")
            callbacks.onOpen(null)
        }

        override fun send(data: ByteString): Boolean = true

        override fun send(text: String): Boolean = true

        fun peerClosing(code: Int) {
            callbacks.onClosing(code)
        }

        fun peerClosed(code: Int) {
            callbacks.onClosed(code)
        }

        override fun close(code: Int, reason: String?): Boolean {
            events.add("close($code)")
            return closeSucceeds
        }

        override fun cancel() {
            events.add("cancel")
        }
    }

    private class RecordingSocketFactory(
        private val closeSucceeds: Boolean = true,
    ) : CollaborationSocketFactory {
        val sockets = mutableListOf<RecordingSocket>()

        override fun makeSocket(
            url: String,
            protocols: List<String>,
            callbacks: CollaborationSocketCallbacks,
        ): CollaborationSocket = RecordingSocket(callbacks, closeSucceeds).also { sockets.add(it) }
    }

    private fun connectedTransport(
        factory: RecordingSocketFactory,
    ): Pair<AndroidCollaborationTransport, FakeEditorV2Backend> {
        val backend = FakeEditorV2Backend()
        backend.collaborationGenerationToOpen = GENERATION
        val created = backend.create(ROOM_CONFIG_JSON, null) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val transport = AndroidCollaborationTransport(
            editorId = editorId,
            backend = backend,
            socketFactory = factory,
            clock = CollaborationMonotonicClock { 0L },
        )
        transport.configure(config(CONNECTED_URL, connect = true))
        assertEquals(
            "the fixture must reach an open socket before teardown is exercised",
            1,
            factory.sockets.size,
        )
        return transport to backend
    }

    private fun config(url: String, connect: Boolean) =
        NativeCollaborationTransportConfig.parse(url, connect, protocolAdapter = null)

    private fun goingAwayClose() =
        listOf("connect", "close(${AndroidCollaborationTransport.WEBSOCKET_CLOSE_GOING_AWAY})")

    @Test
    fun `destroy closes the socket with a going-away handshake`() {
        val factory = RecordingSocketFactory()
        val (transport, _) = connectedTransport(factory)

        transport.destroy()

        assertEquals(
            "destroy must complete the close handshake, not drop the connection",
            goingAwayClose(),
            factory.sockets.single().events,
        )
    }

    @Test
    fun `entering background closes the socket with a going-away handshake`() {
        val factory = RecordingSocketFactory()
        val (transport, _) = connectedTransport(factory)

        transport.enterBackground()
        transport.awaitIdleForTesting()

        assertEquals(goingAwayClose(), factory.sockets.single().events)
        transport.destroy()
    }

    @Test
    fun `replacing the configuration closes the retired socket`() {
        val factory = RecordingSocketFactory()
        val (transport, _) = connectedTransport(factory)

        transport.configure(config(RETIRED_URL, connect = false))

        assertEquals(goingAwayClose(), factory.sockets.first().events)
        transport.destroy()
    }

    @Test
    fun `a refused close still releases the socket`() {
        val factory = RecordingSocketFactory(closeSucceeds = false)
        val (transport, _) = connectedTransport(factory)

        transport.destroy()

        assertEquals(
            "a socket that cannot enqueue a close frame must still be cancelled",
            goingAwayClose() + "cancel",
            factory.sockets.single().events,
        )
    }

    @Test
    fun `peer closing retains ownership until closed`() {
        val factory = RecordingSocketFactory()
        val (transport, backend) = connectedTransport(factory)
        val socket = factory.sockets.single()

        socket.peerClosing(1000)
        transport.awaitIdleForTesting()
        assertEquals(0, backend.calls.count { it == "collaborationSocketClose" })

        socket.peerClosed(1000)
        transport.awaitIdleForTesting()
        assertEquals(1, backend.calls.count { it == "collaborationSocketClose" })
        transport.destroy()
    }

    @Test
    fun `destroy detaches the native collaboration session`() {
        val factory = RecordingSocketFactory()
        val (transport, backend) = connectedTransport(factory)

        transport.destroy()

        assertTrue(
            "the native session must be detached on teardown",
            backend.calls.contains("collaborationDetach"),
        )
    }

    @Test
    fun `host state requests never wait for a blocked worker`() {
        val factory = RecordingSocketFactory()
        val (transport, _) = connectedTransport(factory)
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        transport.enqueueForTesting {
            entered.countDown()
            release.await(2, TimeUnit.SECONDS)
        }
        assertTrue(entered.await(2, TimeUnit.SECONDS))

        val startedAt = System.nanoTime()
        transport.requestHostState(AndroidCollaborationTransport.HostState.BACKGROUND)
        transport.requestHostState(AndroidCollaborationTransport.HostState.DETACHED)
        transport.requestHostState(AndroidCollaborationTransport.HostState.FOREGROUND)
        val elapsedMillis = TimeUnit.NANOSECONDS.toMillis(System.nanoTime() - startedAt)

        assertTrue("lifecycle callbacks must return immediately", elapsedMillis < 100)
        release.countDown()
        transport.awaitIdleForTesting()
        assertEquals(
            AndroidCollaborationTransport.HostState.FOREGROUND,
            transport.hostStateForTesting(),
        )
        transport.destroyAsync()
        assertTrue(transport.awaitDestroyedForTesting())
    }

    @Test
    fun `failed replacement remains retryable and publishes config only after reattach`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create(ROOM_CONFIG_JSON, null) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val transport = AndroidCollaborationTransport(
            editorId = editorId,
            backend = backend,
            socketFactory = RecordingSocketFactory(),
            clock = CollaborationMonotonicClock { 0L },
        )
        val retained = config(RETIRED_URL, connect = false)
        val replacement = config(CONNECTED_URL, connect = true)
        assertEquals(null, transport.configure(retained))
        backend.nextCollaborationReattachError = EditorV2Error(
            "transport",
            "REATTACH_FAILED",
            "retry",
        )

        assertEquals("REATTACH_FAILED", transport.configure(replacement)?.code)
        assertEquals(retained, transport.configForTesting())
        assertEquals(null, transport.configure(replacement))
        assertEquals(replacement, transport.configForTesting())
        transport.destroyAsync()
        assertTrue(transport.awaitDestroyedForTesting())
    }

    @Test
    fun `destroy async is idempotent`() {
        val factory = RecordingSocketFactory()
        val (transport, backend) = connectedTransport(factory)
        val detachCountBeforeDestroy = backend.calls.count { it == "collaborationDetach" }

        transport.destroyAsync()
        transport.destroyAsync()

        assertTrue(transport.awaitDestroyedForTesting())
        assertEquals(
            detachCountBeforeDestroy + 1,
            backend.calls.count { it == "collaborationDetach" },
        )
    }

    private companion object {
        const val GENERATION = "1"
        const val CONNECTED_URL = "wss://collab.example/room"
        const val RETIRED_URL = "wss://collab.example/other-room"
        const val ROOM_CONFIG_JSON = """{"initialization":{"type":"room"}}"""
    }
}
