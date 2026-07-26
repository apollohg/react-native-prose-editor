import CoreText
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
    private var lastContentSizeCategory: UIContentSizeCategory?
    private var scaleByRevision: [UInt64: CGFloat] = [0: Self.scale(for: UIApplication.shared.preferredContentSizeCategory)]
    private(set) var revision: UInt64 = 0
    var onInvalidated: ((UInt64) -> Void)?

    override convenience init() { self.init(notificationCenter: .default) }

    init(notificationCenter: NotificationCenter) {
        self.notificationCenter = notificationCenter
        super.init()
        tokens = [
            notificationCenter.addObserver(forName: UIContentSizeCategory.didChangeNotification, object: nil, queue: .main) { [weak self] note in
                self?.invalidateContentSizeCategory(note.userInfo?[UIContentSizeCategory.newValueUserInfoKey] as? UIContentSizeCategory)
            },
            // This is the documented Core Text notification. The former
            // string literal was private/undocumented and did not reliably
            // observe registrations made by Core Text font loaders.
            notificationCenter.addObserver(forName: Self.registeredFontsDidChangeNotification, object: nil, queue: .main) { [weak self] _ in self?.invalidateRegisteredFonts() },
        ]
    }

    deinit { tokens.forEach(notificationCenter.removeObserver) }

    static let registeredFontsDidChangeNotification = Notification.Name(kCTFontManagerRegisteredFontsChangedNotification as String)

    func invalidateRegisteredFonts() { invalidate(scale: currentScale()) }

    func fontScale(for revision: UInt64) -> CGFloat {
        lock.lock()
        defer { lock.unlock() }
        return scaleByRevision[revision] ?? scaleByRevision[self.revision] ?? 1
    }

    func shouldWarnForMissingFamily(_ family: String, semanticGeneration: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        // A layout/environment revision is not a new semantic request. Keep
        // this bounded process registry keyed solely by semantic generation
        // and family so attachment/font reinstalls cannot re-log.
        let inserted = missingWarnings.insert("\(semanticGeneration)\u{1f}\(family)").inserted
        while missingWarnings.count > 512, let oldest = missingWarnings.sorted().first {
            missingWarnings.remove(oldest)
        }
        return inserted
    }

    private func invalidateContentSizeCategory(_ category: UIContentSizeCategory?) {
        let resolvedCategory = category ?? UIApplication.shared.preferredContentSizeCategory
        lock.lock()
        let isDuplicate = lastContentSizeCategory == resolvedCategory
        lastContentSizeCategory = resolvedCategory
        lock.unlock()
        guard !isDuplicate else { return }
        invalidate(scale: Self.scale(for: resolvedCategory))
    }

    private func currentScale() -> CGFloat {
        lock.lock()
        let scale = scaleByRevision[revision] ?? 1
        lock.unlock()
        return scale
    }

    private func invalidate(scale: CGFloat) {
        let resolvedScale = scale.isFinite && scale > 0 ? scale : 1
        lock.lock()
        revision &+= 1
        missingWarnings.removeAll()
        scaleByRevision[revision] = resolvedScale
        while scaleByRevision.count > 64, let oldest = scaleByRevision.keys.sorted().first, oldest != revision {
            scaleByRevision.removeValue(forKey: oldest)
        }
        let nextRevision = revision
        lock.unlock()
        onInvalidated?(nextRevision)
        notificationCenter.post(
            name: Self.didInvalidateNotification,
            object: self,
            userInfo: ["revision": nextRevision, "scale": resolvedScale]
        )
    }

    private static func scale(for category: UIContentSizeCategory) -> CGFloat {
        UIFontMetrics.default.scaledValue(for: 1, compatibleWith: UITraitCollection(preferredContentSizeCategory: category))
    }
}
