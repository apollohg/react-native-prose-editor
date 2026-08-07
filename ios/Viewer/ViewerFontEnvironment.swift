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
    private var scaleByRevision: [UInt64: CGFloat] = [0: 1]
    private(set) var revision: UInt64 = 0
    var onInvalidated: ((UInt64) -> Void)?

    private enum GenericFamily {
        case system
        case serif
        case monospaced
        case rounded
    }

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

    @objc(refreshContentSizeCategory)
    public func refreshContentSizeCategory() {
        guard Thread.isMainThread else { return }
        invalidateContentSizeCategory(UIApplication.shared.preferredContentSizeCategory)
    }

    @objc(currentFontScale)
    public func currentFontScale() -> CGFloat { currentScale() }

    public func invalidateRegisteredFonts() { invalidate(scale: currentScale()) }

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

    /// Test/internal inspection only. Unlike `shouldWarnForMissingFamily`,
    /// this does not insert, evict, or otherwise mutate warning dedup state.
    func hasMissingFamilyWarning(_ family: String, semanticGeneration: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return missingWarningsBySemanticGeneration[semanticGeneration]?.contains(family) ?? false
    }

    /// The sole viewer font-family resolution contract. Theme paints and
    /// inline marks both reach this path, so availability, deterministic
    /// fallback, and semantic-generation warning scope cannot diverge.
    func resolveFont(
        family: String?,
        size: CGFloat,
        fallback: UIFont,
        weight: UIFont.Weight? = nil,
        additionalTraits: UIFontDescriptor.SymbolicTraits = [],
        semanticGeneration: String
    ) -> UIFont {
        let resolvedSize = size.isFinite && size > 0 ? size : fallback.pointSize
        let normalized = family?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let base: UIFont
        if normalized.isEmpty {
            base = fallback.withSize(resolvedSize)
        } else if let generic = Self.genericFamily(for: normalized) {
            base = Self.genericFont(
                generic,
                size: resolvedSize,
                weight: weight ?? (additionalTraits.contains(.traitBold) ? .bold : .regular)
            )
        } else if let registered = UIFont(name: normalized, size: resolvedSize) {
            base = registered
        } else {
            if shouldWarnForMissingFamily(normalized, semanticGeneration: semanticGeneration) {
                NSLog("PreparedProseViewer: requested font family %@ is unavailable; using system fallback", normalized)
            }
            base = fallback.withSize(resolvedSize)
        }
        return Self.applyingTraits(additionalTraits, to: base)
    }

    func resolveFont(
        style: EditorTextStyle?,
        fallback: UIFont,
        fontScale: CGFloat,
        semanticGeneration: String
    ) -> UIFont {
        guard let style else { return fallback }
        let size = style.fontSize.map { $0 * fontScale } ?? fallback.pointSize
        var traits: UIFontDescriptor.SymbolicTraits = []
        if EditorTheme.shouldApplyBoldTrait(style.fontWeight) { traits.insert(.traitBold) }
        if style.fontStyle == "italic" { traits.insert(.traitItalic) }
        return resolveFont(
            family: style.fontFamily,
            size: size,
            fallback: fallback,
            weight: style.fontWeight.map { EditorTheme.fontWeight(from: $0) },
            additionalTraits: traits,
            semanticGeneration: semanticGeneration
        )
    }

    private static func genericFamily(for family: String) -> GenericFamily? {
        switch family.lowercased() {
        case "default", "system", "system-ui", "sans", "sans-serif", "ui-sans-serif",
             "cursive", "casual", "sans-serif-smallcaps", "sans-serif-condensed",
             "sans-serif-light", "sans-serif-medium", "sans-serif-black", "sans-serif-thin",
             "sans-serif-condensed-light":
            return .system
        case "serif", "ui-serif":
            return .serif
        case "monospace", "monospaced", "ui-monospace", "serif-monospace":
            return .monospaced
        case "ui-rounded":
            return .rounded
        default:
            return nil
        }
    }

    private static func genericFont(
        _ family: GenericFamily,
        size: CGFloat,
        weight: UIFont.Weight
    ) -> UIFont {
        let system = UIFont.systemFont(ofSize: size, weight: weight)
        switch family {
        case .system:
            return system
        case .serif:
            guard let descriptor = system.fontDescriptor.withDesign(.serif) else { return system }
            return UIFont(descriptor: descriptor, size: size)
        case .monospaced:
            return UIFont.monospacedSystemFont(ofSize: size, weight: weight)
        case .rounded:
            guard let descriptor = system.fontDescriptor.withDesign(.rounded) else { return system }
            return UIFont(descriptor: descriptor, size: size)
        }
    }

    /// Shared emphasis-satisfaction contract for editor and prepared viewer
    /// font resolution. Callers must not accept a face that contains only a
    /// subset of the requested bold/italic marks.
    static func satisfiesRequestedEmphasis(
        _ font: UIFont,
        requestedTraits: UIFontDescriptor.SymbolicTraits
    ) -> Bool {
        let requested = requestedTraits.intersection(Self.emphasisTraits)
        return font.fontDescriptor.symbolicTraits.intersection(requested) == requested
    }

    private static let emphasisTraits: UIFontDescriptor.SymbolicTraits = [.traitBold, .traitItalic]

    /// Apply requested traits one at a time. Retain a custom/inherited face
    /// only when it can express the complete requested emphasis set; otherwise
    /// deterministically choose a system or monospaced system fallback that
    /// preserves the whole set.
    private static func applyingTraits(
        _ additionalTraits: UIFontDescriptor.SymbolicTraits,
        to font: UIFont
    ) -> UIFont {
        let requested = font.fontDescriptor.symbolicTraits.union(additionalTraits)
        func applySequentially(to source: UIFont) -> UIFont {
            var resolved = source
            for trait in [UIFontDescriptor.SymbolicTraits.traitBold, .traitItalic] where requested.contains(trait) && !resolved.fontDescriptor.symbolicTraits.contains(trait) {
                guard let descriptor = resolved.fontDescriptor.withSymbolicTraits(resolved.fontDescriptor.symbolicTraits.union([trait])) else { continue }
                resolved = UIFont(descriptor: descriptor, size: resolved.pointSize)
            }
            return resolved
        }

        let sequential = applySequentially(to: font)
        if satisfiesRequestedEmphasis(sequential, requestedTraits: requested) {
            return sequential
        }
        let fallback = prefersMonospacedFallback(for: font)
            ? UIFont.monospacedSystemFont(
                ofSize: font.pointSize,
                weight: requested.contains(.traitBold) ? .bold : .regular
            )
            : UIFont.systemFont(
                ofSize: font.pointSize,
                weight: requested.contains(.traitBold) ? .bold : .regular
            )
        return applySequentially(to: fallback)
    }

    private static func prefersMonospacedFallback(for font: UIFont) -> Bool {
        if font.fontDescriptor.symbolicTraits.contains(.traitMonoSpace) { return true }
        let name = "\(font.fontName) \(font.familyName)".lowercased()
        return name.contains("mono") || name.contains("courier")
    }

    private func invalidateContentSizeCategory(_ category: UIContentSizeCategory?) {
        guard Thread.isMainThread else { return }
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
