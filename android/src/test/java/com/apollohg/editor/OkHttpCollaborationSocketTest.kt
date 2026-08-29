package com.apollohg.editor

import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class OkHttpCollaborationSocketTest {
    @Test
    fun `peer initiated close is acknowledged before terminal callback`() {
        val serverClosed = CountDownLatch(1)
        val server = MockWebServer()
        server.enqueue(
            MockResponse().withWebSocketUpgrade(
                object : WebSocketListener() {
                    override fun onOpen(webSocket: WebSocket, response: Response) {
                        webSocket.close(1000, "done")
                    }

                    override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                        serverClosed.countDown()
                    }
                }
            )
        )
        server.start()
        val closing = CountDownLatch(1)
        val closed = CountDownLatch(1)
        val socket = OkHttpCollaborationSocketFactory().makeSocket(
            server.url("/").toString().replaceFirst("http", "ws"),
            emptyList(),
            CollaborationSocketCallbacks(
                onOpen = {},
                onBinaryMessage = {},
                onTextMessage = {},
                onClosing = { code ->
                    if (code == 1000) closing.countDown()
                },
                onClosed = { code ->
                    if (code == 1000) closed.countDown()
                },
                onFailure = {},
            ),
        )

        try {
            socket.connect()

            assertTrue(closing.await(2, TimeUnit.SECONDS))
            assertTrue(closed.await(2, TimeUnit.SECONDS))
            assertTrue(serverClosed.await(2, TimeUnit.SECONDS))
        } finally {
            socket.cancel()
            server.shutdown()
        }
    }
}
