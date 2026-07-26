import UIKit

/// Process font/Dynamic Type environment. Callers retain the revision in the
/// request key; this object never mutates a prepared artifact in place.
final class ViewerFontEnvironment {
    static let shared = ViewerFontEnvironment()
    private let lock = NSLock()
    private let notificationCenter: NotificationCenter
    private var tokens: [NSObjectProtocol] = []
    private var missingWarnings = Set<String>()
    private(set) var revision: UInt64 = 0
    var onInvalidated: ((UInt64) -> Void)?

    init(notificationCenter: NotificationCenter = .default) {
        self.notificationCenter = notificationCenter
        tokens = [
            notificationCenter.addObserver(forName: UIContentSizeCategory.didChangeNotification, object: nil, queue: .main) { [weak self] _ in self?.invalidate() },
            notificationCenter.addObserver(forName: .init("com.apple.fonts.changed"), object: nil, queue: .main) { [weak self] _ in self?.invalidateRegisteredFonts() },
        ]
    }

    deinit { tokens.forEach(notificationCenter.removeObserver) }

    func invalidateRegisteredFonts() { invalidate() }

    func shouldWarnForMissingFamily(_ family: String, semanticGeneration: String) -> Bool {
        let key = "\(revision)\u{1f}\(semanticGeneration)\u{1f}\(family)"
        lock.lock(); defer { lock.unlock() }
        return missingWarnings.insert(key).inserted
    }

    private func invalidate() {
        lock.lock()
        revision &+= 1
        missingWarnings.removeAll()
        let revision = revision
        lock.unlock()
        onInvalidated?(revision)
    }
}
