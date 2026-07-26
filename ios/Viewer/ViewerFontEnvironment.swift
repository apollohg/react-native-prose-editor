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
    /// Dedupe is scoped to a semantic generation, not an environment/layout
    /// revision. Eviction removes whole old generations, so a retained
    /// generation cannot lose one family's once-only warning independently.
    private var missingWarningsBySemanticGeneration: [String: Set<String>] = [:]
    private var missingWarningGenerationOrder: [String] = []
    private let missingWarningGenerationLimit = 128
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
        var families = missingWarningsBySemanticGeneration[semanticGeneration] ?? []
        let inserted = families.insert(family).inserted
        if inserted {
            if missingWarningsBySemanticGeneration[semanticGeneration] == nil {
                missingWarningGenerationOrder.append(semanticGeneration)
            }
            missingWarningsBySemanticGeneration[semanticGeneration] = families
            while missingWarningGenerationOrder.count > missingWarningGenerationLimit {
                let oldest = missingWarningGenerationOrder.removeFirst()
                missingWarningsBySemanticGeneration.removeValue(forKey: oldest)
            }
        }
        return inserted
    }

    /// The sole viewer font-family resolution contract. Theme paints and
    /// inline marks both reach this path, so availability, deterministic
    /// fallback, and semantic-generation warning scope cannot diverge.
    func resolveFont(
        family: String?,
        size: CGFloat,
        fallback: UIFont,
        additionalTraits: UIFontDescriptor.SymbolicTraits = [],
        semanticGeneration: String
    ) -> UIFont {
        let resolvedSize = size.isFinite && size > 0 ? size : fallback.pointSize
        let normalized = family?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let base: UIFont
        if normalized.isEmpty {
            base = fallback.withSize(resolvedSize)
        } else if let registered = UIFont(name: normalized, size: resolvedSize) {
            base = registered
        } else {
            if shouldWarnForMissingFamily(normalized, semanticGeneration: semanticGeneration) {
                NSLog("PreparedProseViewer: requested font family %@ is unavailable; using system fallback", normalized)
            }
            base = fallback.withSize(resolvedSize)
        }
        return applyingTraits(additionalTraits, to: base)
    }

    func resolveFont(
        style: EditorTextStyle?,
        fallback: UIFont,
        fontScale: CGFloat,
        semanticGeneration: String
    ) -> UIFont {
        guard let style else { return fallback }
        let size = style.fontSize.map { $0 * fontScale } ?? fallback.pointSize
        var deterministicFallback = fallback.withSize(size)
        if (style.fontFamily?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true),
           let weight = style.fontWeight {
            deterministicFallback = UIFont.systemFont(ofSize: size, weight: EditorTheme.fontWeight(from: weight))
        }
        var traits: UIFontDescriptor.SymbolicTraits = []
        if EditorTheme.shouldApplyBoldTrait(style.fontWeight) { traits.insert(.traitBold) }
        if style.fontStyle == "italic" { traits.insert(.traitItalic) }
        return resolveFont(
            family: style.fontFamily,
            size: size,
            fallback: deterministicFallback,
            additionalTraits: traits,
            semanticGeneration: semanticGeneration
        )
    }

    /// Apply traits one at a time. Some custom families expose bold and italic
    /// separately but not a combined face; if either transform cannot be
    /// represented, switch deterministically to a system face and synthesize
    /// the full requested pair rather than silently losing italic.
    private func applyingTraits(_ additionalTraits: UIFontDescriptor.SymbolicTraits, to font: UIFont) -> UIFont {
        let requested = font.fontDescriptor.symbolicTraits.union(additionalTraits)
        let requestedEmphasis: UIFontDescriptor.SymbolicTraits = [.traitBold, .traitItalic]
        func applySequentially(to source: UIFont) -> UIFont {
            var resolved = source
            for trait in [UIFontDescriptor.SymbolicTraits.traitBold, .traitItalic] where requested.contains(trait) && !resolved.fontDescriptor.symbolicTraits.contains(trait) {
                guard let descriptor = resolved.fontDescriptor.withSymbolicTraits(resolved.fontDescriptor.symbolicTraits.union([trait])) else { continue }
                resolved = UIFont(descriptor: descriptor, size: resolved.pointSize)
            }
            return resolved
        }

        let sequential = applySequentially(to: font)
        if sequential.fontDescriptor.symbolicTraits.intersection(requestedEmphasis) == requested.intersection(requestedEmphasis) {
            return sequential
        }
        let systemFallback = UIFont.systemFont(
            ofSize: font.pointSize,
            weight: requested.contains(.traitBold) ? .bold : .regular
        )
        return applySequentially(to: systemFallback)
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
