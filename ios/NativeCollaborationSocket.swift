import Foundation

struct CollaborationSocketCallbacks {
    let didOpen: (_ negotiatedProtocol: String?) -> Void
    let didClose: (_ code: URLSessionWebSocketTask.CloseCode, _ reason: Data?) -> Void
    let didFail: () -> Void
}

enum CollaborationSocketMessage {
    case binary(Data)
    case text(String)
}

protocol CollaborationSocket: AnyObject {
    func resume()
    func sendBinary(_ data: Data, completion: @escaping (Result<Void, Error>) -> Void)
    func sendText(_ text: String, completion: @escaping (Result<Void, Error>) -> Void)
    func receive(_ completion: @escaping (Result<CollaborationSocketMessage, Error>) -> Void)
    func cancel(code: URLSessionWebSocketTask.CloseCode, reason: Data?)
}

protocol CollaborationSocketFactory {
    func makeSocket(
        url: URL,
        protocols: [String],
        callbacks: CollaborationSocketCallbacks
    ) -> CollaborationSocket
}

struct URLSessionCollaborationSocketFactory: CollaborationSocketFactory {
    func makeSocket(
        url: URL,
        protocols: [String],
        callbacks: CollaborationSocketCallbacks
    ) -> CollaborationSocket {
        NativeCollaborationSocket(url: url, protocols: protocols, callbacks: callbacks)
    }
}

/// URLSession owns only WebSocket I/O. Generation admission, retry policy,
/// ordering, and retained outbound bytes remain in Rust/the transport driver.
final class NativeCollaborationSocket: NSObject, CollaborationSocket, URLSessionWebSocketDelegate {
    private let callbacks: CollaborationSocketCallbacks
    private var session: URLSession!
    private var task: URLSessionWebSocketTask!

    init(url: URL, protocols: [String], callbacks: CollaborationSocketCallbacks) {
        self.callbacks = callbacks
        super.init()
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpShouldSetCookies = false
        configuration.httpCookieStorage = nil
        configuration.urlCredentialStorage = nil
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
        task = session.webSocketTask(with: url, protocols: protocols)
    }

    func resume() {
        task.resume()
    }

    func sendBinary(_ data: Data, completion: @escaping (Result<Void, Error>) -> Void) {
        task.send(.data(data)) { error in
            if let error {
                completion(.failure(error))
            } else {
                completion(.success(()))
            }
        }
    }

    func sendText(_ text: String, completion: @escaping (Result<Void, Error>) -> Void) {
        task.send(.string(text)) { error in
            if let error {
                completion(.failure(error))
            } else {
                completion(.success(()))
            }
        }
    }

    func receive(_ completion: @escaping (Result<CollaborationSocketMessage, Error>) -> Void) {
        task.receive { result in
            switch result {
            case .failure(let error):
                completion(.failure(error))
            case .success(.data(let data)):
                completion(.success(.binary(data)))
            case .success(.string(let text)):
                completion(.success(.text(text)))
            @unknown default:
                completion(.failure(URLError(.cannotDecodeContentData)))
            }
        }
    }

    func cancel(code: URLSessionWebSocketTask.CloseCode, reason: Data?) {
        task.cancel(with: code, reason: reason)
        session.invalidateAndCancel()
    }

    func urlSession(
        _ session: URLSession,
        webSocketTask: URLSessionWebSocketTask,
        didOpenWithProtocol protocol: String?
    ) {
        callbacks.didOpen(`protocol`)
    }

    func urlSession(
        _ session: URLSession,
        webSocketTask: URLSessionWebSocketTask,
        didCloseWith closeCode: URLSessionWebSocketTask.CloseCode,
        reason: Data?
    ) {
        callbacks.didClose(closeCode, reason)
    }

    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        didCompleteWithError error: Error?
    ) {
        guard error != nil else { return }
        callbacks.didFail()
    }
}
