import ExpoModulesCore
import UIKit

final class PendingJSONRetry {
    struct Token {
        let generation: UInt64
        let attempt: Int
    }

    var json: String?
    var editorId: UInt64?
    private var scheduled = false
    var generation: UInt64 = 0
    private(set) var attempts = 0

    func clear() {
        json = nil
        editorId = nil
        scheduled = false
        attempts = 0
        generation &+= 1
    }

    func schedule(
        json: String?,
        editorId: UInt64,
        maxAttempts: Int?
    ) -> Token? {
        self.json = json
        self.editorId = editorId
        guard !scheduled else { return nil }
        if let maxAttempts, attempts >= maxAttempts { return nil }
        attempts += 1
        scheduled = true
        generation &+= 1
        return Token(generation: generation, attempt: attempts)
    }

    func consume(_ token: Token) -> (json: String?, editorId: UInt64?)? {
        guard token.generation == generation else { return nil }
        let result = (json, editorId)
        json = nil
        editorId = nil
        scheduled = false
        return result
    }
}

class NativeEditorExpoView: ExpoView, EditorTextViewDelegate, UIGestureRecognizerDelegate {
    static let layoutEpsilon: CGFloat = 0.5
    static let nativeActionRetryDelay: TimeInterval = 0.016
    static let maxPendingUpdateRetryAttempts = 5

    // MARK: - Subviews

    let richTextView: RichTextEditorView
    let accessoryToolbar = EditorAccessoryToolbarView(
        frame: .zero,
        inputViewStyle: .keyboard
    )
    let accessoryPlaceholder = EditorAccessoryPlaceholderView(frame: .zero)
    var toolbarFramesInWindow: [CGRect] = []
    var lastToolbarTouchUptime: TimeInterval = -Double.infinity
    var didApplyAutoFocus = false
    var toolbarState = NativeToolbarState.empty
    var toolbarItems: [NativeToolbarItem] = NativeToolbarItem.defaults
    var showsToolbar = true
    var toolbarPlacement = "keyboard"
    var heightBehavior: EditorHeightBehavior = .fixed
    private var lastAutoGrowWidth: CGFloat = 0
    var lastPublishedAutoGrowHeight: CGFloat?
    var addons = NativeEditorAddons(mentions: nil)
    var mentionQueryState: MentionQueryState?
    var lastMentionEventJSON: String?
    var desiredThemeJSON: String?
    var desiredAtomsJSON: String?
    let imageLoadOwner = RenderImageLoadOwner(policy: .default)
    var lastThemeJSON: String?
    var lastAddonsJSON: String?
    var lastAtomsJSON: String?
    var lastRemoteSelectionsJSON: String?
    var lastToolbarItemsJSON: String?
    var lastToolbarFrameJSON: String?
    private var isReparentingAtomChild = false
    private var mountedReactChildren: [UIView] = []
    private var mountedAtomKeys: [ObjectIdentifier: String] = [:]
    var lastEditorUpdateJSONProp: String?
    var pendingEditorUpdateResetJSON: String?
    var pendingEditorUpdateJSON: String?
    var pendingEditorUpdateEditorId: String?
    var pendingEditorUpdateRevision = 0
    var appliedEditorUpdateRevision = 0
    var renderedRevision: (document: UInt64, state: UInt64)?
    var pendingEditorUpdateRetryScheduled = false
    var pendingEditorUpdateRetryEditorId: UInt64?
    var pendingEditorUpdateRetryGeneration: UInt64 = 0
    /// Internal-only fallback for boundary rejections that cannot reach an
    /// adapter callback because the paired adapter is absent. Task 15 owns
    /// application-visible event wiring; these deterministic records do not
    /// dispatch an Expo event.
    var editorUpdateInternalRejections: [String] = []
    var pendingViewCommandUpdateJSON: String?
    var pendingViewCommandUpdateEditorId: UInt64?
    var pendingViewCommandUpdateRetryScheduled = false
    var pendingViewCommandUpdateRetryGeneration: UInt64 = 0
    var pendingEditableRetryValue: Bool?
    var pendingEditableRetryEditorId: UInt64?
    var pendingEditableRetryScheduled = false
    var pendingEditableRetryGeneration: UInt64 = 0
    let pendingThemeRetry = PendingJSONRetry()
    let pendingAtomsRetry = PendingJSONRetry()
    var pendingAtomsWakeScheduled = false
    var atomsRetryAttemptsForTesting: Int { pendingAtomsRetry.attempts }
    var blockAtomConfigurationApplyForTesting = false
    var pendingAccessoryRetryActions: [PendingAccessoryRetryAction] = []
    var invalidatedAccessoryRetryActions = Set<PendingAccessoryRetryAction>()
    var pendingAccessoryRetryEditorId: UInt64?
    var pendingAccessoryRetryScheduled = false
    var pendingAccessoryRetryGeneration: UInt64 = 0
    var pendingMentionSuggestionRetry: PendingMentionSuggestionRetry?
    var pendingMentionSuggestionRetryScheduled = false
    var pendingMentionSuggestionRetryGeneration: UInt64 = 0
    lazy var outsideTapGestureRecognizer: UITapGestureRecognizer = {
        let recognizer = UITapGestureRecognizer(
            target: self,
            action: #selector(handleOutsideTap(_:))
        )
        recognizer.cancelsTouchesInView = false
        recognizer.delegate = self
        return recognizer
    }()
    weak var gestureWindow: UIWindow?

    /// Guard flag to suppress echo: when JS applies an update via the view
    /// command, the resulting delegate callback must NOT be re-dispatched
    /// back to JS.
    var isApplyingJSUpdate = false

    // MARK: - Event Dispatchers (wired by Expo Modules via reflection)

    let onEditorUpdate = EventDispatcher()
    let onSelectionChange = EventDispatcher()
    let onFocusChange = EventDispatcher()
    let onContentHeightChange = EventDispatcher()
    let onAtomLayout = EventDispatcher()
    let onToolbarAction = EventDispatcher()
    let onAddonEvent = EventDispatcher()
    let onEditorError = EventDispatcher()
    let onExternalTextCompositionEnd = EventDispatcher()
    /// Native integration tests capture the exact payload production sends to
    /// Expo without assigning adapter callbacks directly.
    var onEditorErrorForTesting: (([String: Any]) -> Void)?
    var onExternalTextCompositionEndForTesting: (([String: Any]) -> Void)?
    var autonomousErrorBindingAdapter: EditorV2Adapter?
    var autonomousErrorBindingEditorId: String?
    var autonomousErrorBindingToken: UUID?
    var autonomousErrorBindingGeneration: UInt64 = 0
    var pendingAutonomousErrors: [UUID: PendingAutonomousError] = [:]
    var lastEmittedContentHeight: CGFloat = 0
    var cachedAutoGrowContentHeight: CGFloat = 0
    var lastAddonEventJSONForTestingValue: String?

    enum EditorUpdateApplyOutcome {
        case applied
        case retryableDeferred
        case rejected
    }

    enum PendingAccessoryRetryAction: Hashable {
        case reloadInputViews
        case refreshMentionQuery
        case clearMentionQueryState
        case updateAccessoryToolbarVisibility
    }

    struct PendingMentionSuggestionRetry {
        let suggestionKey: String
        let editorId: UInt64
        let trigger: String
        let query: String
        let anchor: UInt32
        let head: UInt32
        let documentVersion: String?
        let textSnapshot: String
    }

    struct PendingAutonomousError {
        let adapter: EditorV2Adapter
        let editorId: String
        let token: UUID
        let generation: UInt64
        let error: FfiError
    }

    struct MentionRetryTextDiff {
        let start: Int
        let oldEnd: Int
        let newEnd: Int
    }

    // MARK: - Initialization

    required init(appContext: AppContext? = nil) {
        richTextView = RichTextEditorView(frame: .zero)
        super.init(appContext: appContext)
        richTextView.imageLoadOwner = imageLoadOwner
        richTextView.onHeightMayChange = { [weak self] measuredHeight in
            guard let self, self.heightBehavior == .autoGrow else { return }
            self.cachedAutoGrowContentHeight = measuredHeight
            self.invalidateIntrinsicContentSize()
            self.emitContentHeightIfNeeded(force: true, measuredHeight: measuredHeight)
        }
        richTextView.onAtomContentWidthChange = { [weak self] width in
            self?.emitAtomLayout(width: width)
        }
        richTextView.textView.editorDelegate = self
        richTextView.textView.onExternalUpdateReadinessMayChange = { [weak self] in
            self?.schedulePendingAtomsWakeIfNeeded()
        }
        configureAccessoryToolbar()

        // Observe UITextView focus changes via NotificationCenter.
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(textViewDidBeginEditing(_:)),
            name: UITextView.textDidBeginEditingNotification,
            object: richTextView.textView
        )
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(textViewDidEndEditing(_:)),
            name: UITextView.textDidEndEditingNotification,
            object: richTextView.textView
        )

        addSubview(richTextView)
    }

    deinit {
        richTextView.textView.editorDelegate = nil
        richTextView.textView.onExternalUpdateReadinessMayChange = nil
        if let resultJSON = richTextView.textView.discardTransientNativeInputForEditorRebind() {
            dispatchExternalTextCompositionEnd(resultJSON)
        }
        let editorId = richTextView.editorId
        let releasedNativeOwner = ownsNativeBinding(editorId: editorId)
        clearAutonomousErrorBinding()
        NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: self)
        if releasedNativeOwner {
            NativeEditorViewRegistry.shared.nativeOwnerReleased(editorId: editorId, by: self)
        }
        imageLoadOwner.cancelAll()
        NotificationCenter.default.removeObserver(self)
    }

    // MARK: - Layout

    override func mountChildComponentView(_ childComponentView: UIView, index: Int) {
        let reactIndex = min(max(index, 0), mountedReactChildren.count)
        mountedReactChildren.insert(childComponentView, at: reactIndex)
        let atomKey = Self.atomKey(for: childComponentView)
        if let atomKey {
            mountedAtomKeys[ObjectIdentifier(childComponentView)] = atomKey
            richTextView.mountAtomChild(childComponentView, atomKey: atomKey)
        } else {
            let nativeSubviewCount = subviews.filter { subview in
                !mountedReactChildren.contains(where: { $0 === subview })
            }.count
            let directIndex = nativeSubviewCount + mountedReactChildren[..<reactIndex].filter {
                mountedAtomKeys[ObjectIdentifier($0)] == nil
            }.count
            super.mountChildComponentView(childComponentView, index: directIndex)
        }
    }

    override func unmountChildComponentView(_ childComponentView: UIView, index: Int) {
        if let reactIndex = mountedReactChildren.firstIndex(where: { $0 === childComponentView }) {
            mountedReactChildren.remove(at: reactIndex)
        }
        if mountedAtomKeys.removeValue(forKey: ObjectIdentifier(childComponentView)) != nil {
            _ = richTextView.unmountAtomChild(childComponentView)
            childComponentView.removeFromSuperview()
            return
        }
        let directIndex = subviews.firstIndex(where: { $0 === childComponentView }) ?? index
        super.unmountChildComponentView(childComponentView, index: directIndex)
    }

    override func didAddSubview(_ subview: UIView) {
        super.didAddSubview(subview)
        guard subview !== richTextView,
              !isReparentingAtomChild,
              let atomKey = Self.atomKey(for: subview)
        else { return }
        isReparentingAtomChild = true
        richTextView.mountAtomChild(subview, atomKey: atomKey)
        isReparentingAtomChild = false
    }

    override var intrinsicContentSize: CGSize {
        guard heightBehavior == .autoGrow else {
            return CGSize(width: UIView.noIntrinsicMetric, height: UIView.noIntrinsicMetric)
        }
        if cachedAutoGrowContentHeight > 0 {
            return CGSize(width: UIView.noIntrinsicMetric, height: cachedAutoGrowContentHeight)
        }
        return richTextView.intrinsicContentSize
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        richTextView.frame = bounds
        guard heightBehavior == .autoGrow else { return }
        let currentWidth = bounds.width.rounded(.towardZero)
        guard currentWidth != lastAutoGrowWidth else { return }
        lastAutoGrowWidth = currentWidth
        cachedAutoGrowContentHeight = 0
        invalidateIntrinsicContentSize()
        emitContentHeightIfNeeded(force: true)
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil {
            let editorId = richTextView.editorId
            let releasedNativeOwner = ownsNativeBinding(editorId: editorId)
            clearAutonomousErrorBinding()
            if releasedNativeOwner {
                NativeEditorViewRegistry.shared.nativeOwnerReleased(editorId: editorId, by: self)
            }
        } else {
            ensureAutonomousErrorBinding()
            applyRemoteCommitRefresh()
        }
        if richTextView.textView.isFirstResponder {
            installOutsideTapRecognizerIfNeeded()
        } else {
            uninstallOutsideTapRecognizer()
        }
    }

    // MARK: - Editor Binding

    // MARK: - Autonomous adapter errors

    var imageLoadingPolicy: ImageLoadingPolicy {
        imageLoadOwner.policy
    }

    // MARK: - View Commands

    // MARK: - Focus Commands

    // MARK: - Focus Notifications

    // MARK: - EditorTextViewDelegate

    var shouldUseSystemAssistantToolbar: Bool {
        false
    }

}
