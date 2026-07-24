package com.apollohg.editor

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString

internal data class CollaborationSocketCallbacks(
    val onOpen: (negotiatedProtocol: String?) -> Unit,
    val onBinaryMessage: (ByteArray) -> Unit,
    val onTextMessage: (String) -> Unit,
    val onClose: (Int) -> Unit,
    val onFailure: () -> Unit,
)

internal interface CollaborationSocket {
    fun connect()
    fun send(data: ByteString): Boolean
    fun send(text: String): Boolean
    fun close(code: Int, reason: String?): Boolean
    fun cancel()
}

internal interface CollaborationSocketFactory {
    fun makeSocket(
        url: String,
        protocols: List<String>,
        callbacks: CollaborationSocketCallbacks,
    ): CollaborationSocket
}

internal class OkHttpCollaborationSocketFactory(
    private val client: OkHttpClient = OkHttpClient(),
) : CollaborationSocketFactory {
    override fun makeSocket(
        url: String,
        protocols: List<String>,
        callbacks: CollaborationSocketCallbacks,
    ): CollaborationSocket = OkHttpCollaborationSocket(client, url, protocols, callbacks)
}

private class OkHttpCollaborationSocket(
    private val client: OkHttpClient,
    private val url: String,
    private val protocols: List<String>,
    private val callbacks: CollaborationSocketCallbacks,
) : CollaborationSocket {
    private var webSocket: WebSocket? = null

    override fun connect() {
        check(webSocket == null)
        val request = Request.Builder().url(url).apply {
            if (protocols.isNotEmpty()) {
                header("Sec-WebSocket-Protocol", protocols.joinToString(", "))
            }
        }.build()
        webSocket = client.newWebSocket(
            request,
            object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) {
                    callbacks.onOpen(response.header("Sec-WebSocket-Protocol"))
                }

                override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                    callbacks.onBinaryMessage(bytes.toByteArray())
                }

                override fun onMessage(webSocket: WebSocket, text: String) {
                    callbacks.onTextMessage(text)
                }

                override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                    callbacks.onClose(code)
                }

                override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                    callbacks.onClose(code)
                }

                override fun onFailure(webSocket: WebSocket, error: Throwable, response: Response?) {
                    callbacks.onFailure()
                }
            },
        )
    }

    override fun send(data: ByteString): Boolean = webSocket?.send(data) == true
    override fun send(text: String): Boolean = webSocket?.send(text) == true

    override fun close(code: Int, reason: String?): Boolean =
        webSocket?.close(code, reason) == true

    override fun cancel() {
        webSocket?.cancel()
    }
}
