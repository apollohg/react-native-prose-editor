import UIKit

/// Process font/Dynamic Type environment. Revision snapshots make resolved
/// metrics deterministic for every prepared generation, including Fabric.
@objc(PREPViewerFontEnvironment)
public final class ViewerFontEnvironment: NSObject {
    @objc public static let didInvalidateNotification = Notification.Name("com.apollohg.editor.viewer.fontEnvironmentDidInvalidate")
    @objc(sharedEnvironment) public class var sharedEnvironment: ViewerFontEnvironment { shared }
    static let shared = ViewerFontEnvironment()

    private let lock = NSLock()
    private let notificationCenter: NotificationCenter
    private var tokens: [NSObjectProtocol] = []
    private var missingWarnings = Set<String>()
    private var scaleByRevision: [UInt64: CGFloat] = [0: Self.scale(for: UIApplication.shared.preferredContentSizeCategory)]
    private(set) var revision: UInt64 = 0
    var onInvalidated: ((UInt64) -> Void)?

    override convenience init() { self.init(notificationCenter: .default) }

    init(notificationCenter: NotificationCenter) {
        self.notificationCenter = notificationCenter
        super.init()
        tokens = [
            notificationCenter.addObserver(forName: UIContentSizeCategory.didChangeNotification, object: nil, queue: .main) { [weak self] note in
                self?.invalidate(contentSizeCategory: note.userInfo?[UIContentSizeCategory.newValueUserInfoKey] as? UIContentSizeCategory)
            },
            notificationCenter.addObserver(forName: .init("com.apple.fonts.changed"), object: nil, queue: .main) { [weak self] _ in self?.invalidateRegisteredFonts() },
        ]
    }

    deinit { tokens.forEach(notificationCenter.removeObserver) }

    func invalidateRegisteredFonts() { invalidate(contentSizeCategory: nil) }

    func fontScale(for revision: UInt64) -> CGFloat {
        lock.lock()
        defer { lock.unlock() }
        return scaleByRevision[revision] ?? scaleByRevision[self.revision] ?? 1
    }

    func shouldWarnForMissingFamily(_ family: String, semanticGeneration: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        let inserted = missingWarnings.insert("\(revision)\u{1f}\(semanticGeneration)\u{1f}\(family)").inserted
        while missingWarnings.count > 512, let oldest = missingWarnings.sorted().first {
            missingWarnings.remove(oldest)
        }
        return inserted
    }

    private func invalidate(contentSizeCategory: UIContentSizeCategory?) {
        lock.lock()
        revision &+= 1
        missingWarnings.removeAll()
        scaleByRevision[revision] = Self.scale(for: contentSizeCategory ?? UIApplication.shared.preferredContentSizeCategory)
        while scaleByRevision.count > 64, let oldest = scaleByRevision.keys.sorted().first, oldest != revision {
            scaleByRevision.removeValue(forKey: oldest)
        }
        let nextRevision = revision
        lock.unlock()
        onInvalidated?(nextRevision)
        notificationCenter.post(name: Self.didInvalidateNotification, object: self, userInfo: ["revision": nextRevision])
    }

    private static func scale(for category: UIContentSizeCategory) -> CGFloat {
        UIFontMetrics.default.scaledValue(for: 1, compatibleWith: UITraitCollection(preferredContentSizeCategory: category))
    }
}
