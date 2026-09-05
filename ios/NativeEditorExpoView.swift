import ExpoModulesCore
import UIKit

private final class PendingJSONRetry {
    struct Token {
        let generation: UInt64
        let attempt: Int
    }

    private var json: String?
    private var editorId: UInt64?
    private var scheduled = false
    private var generation: UInt64 = 0
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
    private static let layoutEpsilon: CGFloat = 0.5
    private static let nativeActionRetryDelay: TimeInterval = 0.016
    private static let maxPendingUpdateRetryAttempts = 5

    // MARK: - Subviews

    let richTextView: RichTextEditorView
    private let accessoryToolbar = EditorAccessoryToolbarView(
        frame: .zero,
        inputViewStyle: .keyboard
    )
    private let accessoryPlaceholder = EditorAccessoryPlaceholderView(frame: .zero)
    private var toolbarFramesInWindow: [CGRect] = []
    private var lastToolbarTouchUptime: TimeInterval = -Double.infinity
    private var didApplyAutoFocus = false
    private var toolbarState = NativeToolbarState.empty
    private var toolbarItems: [NativeToolbarItem] = NativeToolbarItem.defaults
    private var showsToolbar = true
    private var toolbarPlacement = "keyboard"
    private var heightBehavior: EditorHeightBehavior = .fixed
    private var lastAutoGrowWidth: CGFloat = 0
    private var lastPublishedAutoGrowHeight: CGFloat?
    private var addons = NativeEditorAddons(mentions: nil)
    private var mentionQueryState: MentionQueryState?
    private var lastMentionEventJSON: String?
    private var desiredThemeJSON: String?
    private var desiredAtomsJSON: String?
    private let imageLoadOwner = RenderImageLoadOwner(policy: .default)
    private var lastThemeJSON: String?
    private var lastAddonsJSON: String?
    private var lastAtomsJSON: String?
    private var lastRemoteSelectionsJSON: String?
    private var lastToolbarItemsJSON: String?
    private var lastToolbarFrameJSON: String?
    private var isReparentingAtomChild = false
    private var mountedReactChildren: [UIView] = []
    private var mountedAtomKeys: [ObjectIdentifier: String] = [:]
    private var pendingEditorUpdateJSON: String?
    private var pendingEditorUpdateEditorId: String?
    private var pendingEditorUpdateRevision = 0
    private var appliedEditorUpdateRevision = 0
    private var renderedRevision: (document: UInt64, state: UInt64)?
    private var pendingEditorUpdateRetryScheduled = false
    private var pendingEditorUpdateRetryEditorId: UInt64?
    private var pendingEditorUpdateRetryGeneration: UInt64 = 0
    /// Internal-only fallback for boundary rejections that cannot reach an
    /// adapter callback because the paired adapter is absent. Task 15 owns
    /// application-visible event wiring; these deterministic records do not
    /// dispatch an Expo event.
    private var editorUpdateInternalRejections: [String] = []
    private var pendingViewCommandUpdateJSON: String?
    private var pendingViewCommandUpdateEditorId: UInt64?
    private var pendingViewCommandUpdateRetryScheduled = false
    private var pendingViewCommandUpdateRetryGeneration: UInt64 = 0
    private var pendingEditableRetryValue: Bool?
    private var pendingEditableRetryEditorId: UInt64?
    private var pendingEditableRetryScheduled = false
    private var pendingEditableRetryGeneration: UInt64 = 0
    private let pendingThemeRetry = PendingJSONRetry()
    private let pendingAtomsRetry = PendingJSONRetry()
    private var pendingAtomsWakeScheduled = false
    var atomsRetryAttemptsForTesting: Int { pendingAtomsRetry.attempts }
    var blockAtomConfigurationApplyForTesting = false
    private var pendingAccessoryRetryActions: [PendingAccessoryRetryAction] = []
    private var invalidatedAccessoryRetryActions = Set<PendingAccessoryRetryAction>()
    private var pendingAccessoryRetryEditorId: UInt64?
    private var pendingAccessoryRetryScheduled = false
    private var pendingAccessoryRetryGeneration: UInt64 = 0
    private var pendingMentionSuggestionRetry: PendingMentionSuggestionRetry?
    private var pendingMentionSuggestionRetryScheduled = false
    private var pendingMentionSuggestionRetryGeneration: UInt64 = 0
    private lazy var outsideTapGestureRecognizer: UITapGestureRecognizer = {
        let recognizer = UITapGestureRecognizer(
            target: self,
            action: #selector(handleOutsideTap(_:))
        )
        recognizer.cancelsTouchesInView = false
        recognizer.delegate = self
        return recognizer
    }()
    private weak var gestureWindow: UIWindow?

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
    private var autonomousErrorBindingAdapter: EditorV2Adapter?
    private var autonomousErrorBindingEditorId: String?
    private var autonomousErrorBindingToken: UUID?
    private var autonomousErrorBindingGeneration: UInt64 = 0
    private var pendingAutonomousErrors: [UUID: PendingAutonomousError] = [:]
    private var lastEmittedContentHeight: CGFloat = 0
    private var cachedAutoGrowContentHeight: CGFloat = 0
    private var lastAddonEventJSONForTestingValue: String?

    private enum EditorUpdateApplyOutcome {
        case applied
        case retryableDeferred
        case rejected
    }

    private enum PendingAccessoryRetryAction: Hashable {
        case reloadInputViews
        case refreshMentionQuery
        case clearMentionQueryState
        case updateAccessoryToolbarVisibility
    }

    private struct PendingMentionSuggestionRetry {
        let suggestionKey: String
        let editorId: UInt64
        let trigger: String
        let query: String
        let anchor: UInt32
        let head: UInt32
        let documentVersion: String?
        let textSnapshot: String
    }

    private struct PendingAutonomousError {
        let adapter: EditorV2Adapter
        let editorId: String
        let token: UUID
        let generation: UInt64
        let error: FfiError
    }

    private struct MentionRetryTextDiff {
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

    func handleEditorDestroyed(_ editorId: UInt64) {
        guard editorId != 0 else { return }
        guard richTextView.editorId == editorId || richTextView.textView.editorId == editorId else {
            NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: self)
            return
        }

        richTextView.textView.discardTransientNativeInputForEditorRebind()
        clearAutonomousErrorBinding()
        NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: self)
        clearPendingEditorUpdateRetries()
        clearPendingViewCommandUpdateRetry()
        clearPendingEditableRetry()
        clearPendingThemeRetry()
        clearPendingAtomsRetry()
        clearPendingAccessoryRetry()
        clearPendingMentionSuggestionRetry()
        lastMentionEventJSON = nil
        _ = richTextView.textView.resignFirstResponder()
        richTextView.editorId = 0
        mentionQueryState = nil
        _ = accessoryToolbar.setMentionSuggestions([])
        toolbarState = .empty
        accessoryToolbar.apply(state: .empty)
        uninstallOutsideTapRecognizer()
        refreshSystemAssistantToolbarIfNeeded()
    }

    func setEditorId(_ id: UInt64) {
        let previousEditorId = richTextView.editorId
        if id != 0 && NativeEditorViewRegistry.shared.isDestroyed(editorId: id) {
            if previousEditorId == id {
                handleEditorDestroyed(id)
            } else {
                setEditorId(0)
            }
            return
        }
        guard previousEditorId != id else {
            if id != 0 {
                if !NativeEditorViewRegistry.shared.register(editorId: id, view: self) {
                    handleEditorDestroyed(id)
                } else {
                    ensureAutonomousErrorBinding()
                }
            }
            return
        }
        if previousEditorId != id {
            richTextView.textView.discardTransientNativeInputForEditorRebind()
            let releasedNativeOwner = ownsNativeBinding(editorId: previousEditorId)
            clearAutonomousErrorBinding()
            NativeEditorViewRegistry.shared.unregister(editorId: previousEditorId, view: self)
            if releasedNativeOwner {
                NativeEditorViewRegistry.shared.nativeOwnerReleased(
                    editorId: previousEditorId,
                    by: self
                )
            }
            clearPendingEditorUpdateRetries()
            clearPendingViewCommandUpdateRetry()
            clearPendingEditableRetry()
            clearPendingThemeRetry()
            clearPendingAtomsRetry()
            clearPendingAccessoryRetry()
            clearPendingMentionSuggestionRetry()
        }
        var initialBindUpdateJSON: String?
        if id != 0 {
            guard NativeEditorViewRegistry.shared.register(editorId: id, view: self) else {
                handleEditorDestroyed(id)
                return
            }
            bindAutonomousError(adapter: EditorV2Registry.adapter(forLegacyId: id), editorId: id)
            initialBindUpdateJSON = EditorV2Registry.adapter(forLegacyId: id)?.initialUpdateJSON()
        }
        // Bind the editor with the same adopted snapshot used for toolbar
        // state. The text view must not perform an independent state read.
        imageLoadOwner.withCurrent {
            richTextView.bindEditor(id: id, initialUpdateJSON: initialBindUpdateJSON)
        }
        if id != 0 {
            richTextView.emitAtomContentWidthIfAvailable(force: true)
        }
        if id != 0 {
            if let initialBindUpdateJSON,
               let state = NativeToolbarState(updateJSON: initialBindUpdateJSON)
            {
                toolbarState = state
                accessoryToolbar.apply(state: state)
            } else {
                toolbarState = .empty
                accessoryToolbar.apply(state: .empty)
            }
        } else {
            toolbarState = .empty
            accessoryToolbar.apply(state: .empty)
        }
        if desiredThemeJSON != lastThemeJSON {
            setThemeJson(desiredThemeJSON)
        }
        if desiredAtomsJSON != lastAtomsJSON {
            setAtomsJson(desiredAtomsJSON)
        }
        refreshSystemAssistantToolbarIfNeeded()
        refreshMentionQuery()
    }

    // MARK: - Autonomous adapter errors

    func ownsNativeBinding(editorId: UInt64) -> Bool {
        guard editorId != 0,
              richTextView.editorId == editorId,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId)
        else { return false }
        let autonomousOwner = autonomousErrorBindingAdapter === adapter
            && autonomousErrorBindingToken.map { adapter.isNativeBindingOwner(token: $0) } == true
        return autonomousOwner || richTextView.textView.ownsNativeBinding(adapter)
    }

    func claimNativeOwnershipAndCatchUp(editorId: UInt64) {
        guard window != nil, richTextView.editorId == editorId else { return }
        ensureAutonomousErrorBinding()
        applyRemoteCommitRefresh()
    }

    private func ensureAutonomousErrorBinding() {
        let editorId = richTextView.editorId
        guard editorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId)
        else { return }
        guard autonomousErrorBindingAdapter !== adapter
            || autonomousErrorBindingEditorId != adapter.editorId
            || autonomousErrorBindingToken.map({ adapter.isAutonomousErrorOwner(token: $0) }) != true
        else { return }
        clearAutonomousErrorBinding()
        bindAutonomousError(adapter: adapter, editorId: editorId)
    }

    private func bindAutonomousError(adapter: EditorV2Adapter?, editorId: UInt64) {
        guard let adapter,
              let canonicalEditorId = v2CanonicalUInt64String(adapter.editorId),
              canonicalEditorId == String(editorId),
              !adapter.isDestroyed
        else { return }
        let token = UUID()
        let generation = autonomousErrorBindingGeneration
        autonomousErrorBindingAdapter = adapter
        autonomousErrorBindingEditorId = canonicalEditorId
        autonomousErrorBindingToken = token
        adapter.bindAutonomousErrorOwner(token: token) { [weak self, weak adapter] error in
            let enqueue = {
                guard let self, let adapter else { return }
                self.enqueueAutonomousError(
                    error,
                    from: adapter,
                    editorId: canonicalEditorId,
                    token: token,
                    generation: generation
                )
            }
            if Thread.isMainThread {
                enqueue()
            } else {
                DispatchQueue.main.async(execute: enqueue)
            }
        }
    }

    private func clearAutonomousErrorBinding() {
        autonomousErrorBindingGeneration &+= 1
        pendingAutonomousErrors.removeAll()
        if let adapter = autonomousErrorBindingAdapter,
           let token = autonomousErrorBindingToken
        {
            adapter.clearAutonomousErrorOwner(token: token)
        }
        autonomousErrorBindingAdapter = nil
        autonomousErrorBindingEditorId = nil
        autonomousErrorBindingToken = nil
    }

    private func enqueueAutonomousError(
        _ error: FfiError,
        from adapter: EditorV2Adapter,
        editorId: String,
        token: UUID,
        generation: UInt64
    ) {
        guard isLiveAutonomousErrorBinding(
            adapter: adapter,
            editorId: editorId,
            token: token,
            generation: generation
        ) else { return }
        let dispatchId = UUID()
        pendingAutonomousErrors[dispatchId] = PendingAutonomousError(
            adapter: adapter,
            editorId: editorId,
            token: token,
            generation: generation,
            error: error
        )
        DispatchQueue.main.async { [weak self] in
            self?.dispatchAutonomousError(id: dispatchId)
        }
    }

    private func dispatchAutonomousError(id: UUID) {
        // Remove before invoking Expo/test code. Reentrant state changes or a
        // duplicate callback cannot deliver this particular failure twice.
        guard let pending = pendingAutonomousErrors.removeValue(forKey: id),
              isLiveAutonomousErrorBinding(
                adapter: pending.adapter,
                editorId: pending.editorId,
                token: pending.token,
                generation: pending.generation
              )
        else { return }
        let payload = NativeEditorExpoView.autonomousErrorEventPayload(
            editorId: pending.editorId,
            error: pending.error
        )
        if let onEditorErrorForTesting {
            onEditorErrorForTesting(payload)
        } else {
            onEditorError(payload)
        }
    }

    private func isLiveAutonomousErrorBinding(
        adapter: EditorV2Adapter,
        editorId: String,
        token: UUID,
        generation: UInt64
    ) -> Bool {
        guard autonomousErrorBindingGeneration == generation,
              autonomousErrorBindingAdapter === adapter,
              autonomousErrorBindingEditorId == editorId,
              autonomousErrorBindingToken == token,
              adapter.isAutonomousErrorOwner(token: token),
              !adapter.isDestroyed,
              let nativeEditorId = UInt64(editorId),
              !NativeEditorViewRegistry.shared.isDestroyed(editorId: nativeEditorId),
              EditorV2Registry.adapter(forLegacyId: nativeEditorId) === adapter
        else { return false }
        return true
    }

    private static func autonomousErrorEventPayload(editorId: String, error: FfiError) -> [String: Any] {
        let errorRecord: [String: Any] = [
            "domain": error.domain,
            "code": error.code,
            "message": error.message,
            "requestId": error.requestId ?? NSNull(),
            "operationIndex": error.operationIndex ?? NSNull(),
            "limit": error.limit ?? NSNull(),
            "actual": error.actual ?? NSNull(),
            "detailsJson": error.detailsJson ?? NSNull(),
        ]
        let payload: [String: Any] = [
            "editorId": editorId,
            "error": errorRecord,
        ]
        return payload
    }

    func setThemeJson(_ themeJson: String?) {
        desiredThemeJSON = themeJson
        guard lastThemeJSON != themeJson else {
            clearPendingThemeRetry()
            return
        }
        let theme = EditorTheme.from(json: themeJson)
        guard imageLoadOwner.withCurrent({ richTextView.applyTheme(theme) }) else {
            scheduleThemeRetry(themeJson)
            return
        }
        lastThemeJSON = themeJson
        clearPendingThemeRetry()
        accessoryToolbar.apply(theme: theme?.toolbar)
        accessoryToolbar.apply(mentionTheme: theme?.mentions ?? addons.mentions?.theme)
        refreshSystemAssistantToolbarIfNeeded()
        if richTextView.textView.isFirstResponder,
           (richTextView.textView.inputAccessoryView === accessoryToolbar || shouldUseSystemAssistantToolbar)
        {
            reloadInputViewsAfterPreparingOrRetry()
        }
    }

    private func clearPendingEditorUpdateRetries() {
        pendingEditorUpdateJSON = nil
        pendingEditorUpdateEditorId = nil
        pendingEditorUpdateRevision = 0
        appliedEditorUpdateRevision = 0
        renderedRevision = nil
        pendingEditorUpdateRetryScheduled = false
        pendingEditorUpdateRetryEditorId = nil
        pendingEditorUpdateRetryGeneration &+= 1
    }

    private func clearPendingViewCommandUpdateRetry() {
        pendingViewCommandUpdateJSON = nil
        pendingViewCommandUpdateEditorId = nil
        pendingViewCommandUpdateRetryScheduled = false
        pendingViewCommandUpdateRetryGeneration &+= 1
    }

    private func clearPendingEditableRetry() {
        pendingEditableRetryValue = nil
        pendingEditableRetryEditorId = nil
        pendingEditableRetryScheduled = false
        pendingEditableRetryGeneration &+= 1
    }

    private func clearPendingThemeRetry() {
        pendingThemeRetry.clear()
    }

    private func clearPendingAtomsRetry() {
        pendingAtomsRetry.clear()
    }

    private func clearPendingAccessoryRetry() {
        pendingAccessoryRetryActions = []
        invalidatedAccessoryRetryActions.removeAll()
        pendingAccessoryRetryEditorId = nil
        pendingAccessoryRetryScheduled = false
        pendingAccessoryRetryGeneration &+= 1
    }

    private func clearPendingMentionSuggestionRetry() {
        pendingMentionSuggestionRetry = nil
        pendingMentionSuggestionRetryScheduled = false
        pendingMentionSuggestionRetryGeneration &+= 1
    }

    private func scheduleThemeRetry(_ themeJson: String?) {
        guard let token = pendingThemeRetry.schedule(
            json: themeJson,
            editorId: richTextView.editorId,
            maxAttempts: nil
        ) else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard let retry = self.pendingThemeRetry.consume(token) else { return }
            guard retry.editorId == self.richTextView.editorId else {
                self.clearPendingThemeRetry()
                return
            }
            guard retry.json == self.desiredThemeJSON else {
                self.clearPendingThemeRetry()
                return
            }
            self.setThemeJson(retry.json)
        }
    }

    private func scheduleAtomsRetry(_ atomsJson: String?) {
        guard let token = pendingAtomsRetry.schedule(
            json: atomsJson,
            editorId: richTextView.editorId,
            maxAttempts: Self.maxPendingUpdateRetryAttempts
        ) else { return }
        let delay = Self.nativeActionRetryDelay * Double(token.attempt)
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self else { return }
            guard let retry = self.pendingAtomsRetry.consume(token) else { return }
            guard retry.editorId == self.richTextView.editorId else {
                self.clearPendingAtomsRetry()
                return
            }
            guard retry.json == self.desiredAtomsJSON else {
                self.clearPendingAtomsRetry()
                return
            }
            self.setAtomsJson(retry.json)
        }
    }

    private func schedulePendingAtomsWakeIfNeeded() {
        guard desiredAtomsJSON != lastAtomsJSON,
              !pendingAtomsWakeScheduled
        else { return }
        clearPendingAtomsRetry()
        pendingAtomsWakeScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.pendingAtomsWakeScheduled = false
            guard self.desiredAtomsJSON != self.lastAtomsJSON else { return }
            self.setAtomsJson(self.desiredAtomsJSON)
        }
    }

    private func prepareForInputAccessoryMutationOrRetry(_ action: PendingAccessoryRetryAction) -> Bool {
        guard richTextView.editorId != 0, richTextView.textView.isFirstResponder else {
            return true
        }
        guard richTextView.textView.prepareForExternalEditorUpdate() else {
            scheduleAccessoryRetry(action)
            return false
        }
        return true
    }

    private func reloadInputViewsAfterPreparingOrRetry() {
        guard prepareForInputAccessoryMutationOrRetry(.reloadInputViews) else { return }
        richTextView.textView.reloadInputViews()
        markAccessoryMutationSucceeded(.reloadInputViews)
    }

    private func scheduleAccessoryRetry(_ action: PendingAccessoryRetryAction) {
        invalidatedAccessoryRetryActions.remove(action)
        pendingAccessoryRetryActions.removeAll { $0 == action }
        pendingAccessoryRetryActions.append(action)
        pendingAccessoryRetryEditorId = richTextView.editorId
        guard !pendingAccessoryRetryScheduled else { return }
        pendingAccessoryRetryScheduled = true
        pendingAccessoryRetryGeneration &+= 1
        let retryGeneration = pendingAccessoryRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingAccessoryRetryGeneration else { return }
            guard self.pendingAccessoryRetryEditorId == self.richTextView.editorId else {
                self.clearPendingAccessoryRetry()
                return
            }
            let actions = self.pendingAccessoryRetryActions
            self.pendingAccessoryRetryActions = []
            self.pendingAccessoryRetryEditorId = nil
            self.pendingAccessoryRetryScheduled = false
            for index in actions.indices {
                let action = actions[index]
                guard retryGeneration == self.pendingAccessoryRetryGeneration else { return }
                guard !self.invalidatedAccessoryRetryActions.contains(action) else {
                    self.invalidatedAccessoryRetryActions.remove(action)
                    continue
                }
                let generationBeforeAction = self.pendingAccessoryRetryGeneration
                self.performAccessoryRetryAction(action)
                guard self.pendingAccessoryRetryGeneration == generationBeforeAction else {
                    let remainingIndex = actions.index(after: index)
                    if remainingIndex < actions.endIndex {
                        self.requeueUnprocessedAccessoryRetryActions(actions[remainingIndex...])
                    }
                    return
                }
            }
            self.invalidatedAccessoryRetryActions.subtract(actions)
        }
    }

    private func requeueUnprocessedAccessoryRetryActions(
        _ actions: ArraySlice<PendingAccessoryRetryAction>
    ) {
        for action in actions {
            guard !invalidatedAccessoryRetryActions.contains(action) else {
                invalidatedAccessoryRetryActions.remove(action)
                continue
            }
            pendingAccessoryRetryActions.removeAll { $0 == action }
            pendingAccessoryRetryActions.append(action)
        }
        if !pendingAccessoryRetryActions.isEmpty {
            pendingAccessoryRetryEditorId = richTextView.editorId
        }
    }

    private func performAccessoryRetryAction(_ action: PendingAccessoryRetryAction) {
        switch action {
        case .reloadInputViews:
            reloadInputViewsAfterPreparingOrRetry()
        case .refreshMentionQuery:
            refreshMentionQuery()
        case .clearMentionQueryState:
            clearMentionQueryStateAndHidePopover()
        case .updateAccessoryToolbarVisibility:
            updateAccessoryToolbarVisibility()
        }
    }

    private func markAccessoryMutationSucceeded(_ action: PendingAccessoryRetryAction) {
        var invalidated: Set<PendingAccessoryRetryAction> = [action]
        switch action {
        case .refreshMentionQuery:
            invalidated.insert(.clearMentionQueryState)
        case .clearMentionQueryState:
            if !hasActiveMentionQueryForCurrentAddons() {
                invalidated.insert(.refreshMentionQuery)
            }
        case .reloadInputViews, .updateAccessoryToolbarVisibility:
            break
        }
        invalidatePendingAccessoryRetries(invalidated)
    }

    private func invalidatePendingAccessoryRetries(_ actions: Set<PendingAccessoryRetryAction>) {
        guard !actions.isEmpty else { return }
        invalidatedAccessoryRetryActions.formUnion(actions)
        pendingAccessoryRetryActions.removeAll { actions.contains($0) }
    }

    private func hasActiveMentionQueryForCurrentAddons() -> Bool {
        guard richTextView.editorId != 0,
              richTextView.textView.isFirstResponder,
              let mentions = addons.mentions
        else {
            return false
        }
        return currentMentionQueryState(trigger: mentions.trigger) != nil
    }

    func setAddonsJson(_ addonsJson: String?) {
        guard lastAddonsJSON != addonsJson else { return }
        lastAddonsJSON = addonsJson
        addons = NativeEditorAddons.from(json: addonsJson)
        accessoryToolbar.apply(mentionTheme: richTextView.textView.theme?.mentions ?? addons.mentions?.theme)
        refreshMentionQuery()
    }

    func setAtomsJson(_ atomsJson: String?) {
        if desiredAtomsJSON != atomsJson {
            clearPendingAtomsRetry()
        }
        desiredAtomsJSON = atomsJson
        guard lastAtomsJSON != atomsJson else {
            clearPendingAtomsRetry()
            return
        }
        let configuration = AtomRenderConfiguration.from(json: atomsJson)
        guard !blockAtomConfigurationApplyForTesting,
              richTextView.applyAtomRenderConfiguration(configuration)
        else {
            scheduleAtomsRetry(atomsJson)
            return
        }
        lastAtomsJSON = atomsJson
        clearPendingAtomsRetry()
    }

    var imageLoadingPolicy: ImageLoadingPolicy {
        imageLoadOwner.policy
    }

    func setImageLoadingPolicyJson(_ json: String?) {
        let policy = ImageLoadingPolicy.from(json: json)
        guard policy != imageLoadOwner.policy else { return }
        imageLoadOwner.updatePolicy(policy)
        richTextView.textView.imageLoadingPolicyDidChange()
        guard richTextView.editorId != 0 else { return }
        imageLoadOwner.withCurrent {
            _ = richTextView.textView.applyUpdateJSON(
                EditorV2Shadow.getCurrentState(id: richTextView.editorId),
                notifyDelegate: false
            )
        }
    }

    func setRemoteSelectionsJson(_ remoteSelectionsJson: String?) {
        guard lastRemoteSelectionsJSON != remoteSelectionsJson else { return }
        lastRemoteSelectionsJSON = remoteSelectionsJson
        richTextView.setRemoteSelections(RemoteSelectionDecoration.from(json: remoteSelectionsJson))
    }

    func setEditable(_ editable: Bool) {
        if !editable, richTextView.textView.isEditable {
            richTextView.textView.cancelExternalTextCompositionForLifecycleIfNeeded()
        }
        if !editable,
           richTextView.textView.isEditable,
           richTextView.editorId != 0,
           !richTextView.textView.prepareForExternalEditorUpdate()
        {
            scheduleEditableRetry(editable)
            return
        }
        pendingEditableRetryValue = nil
        pendingEditableRetryEditorId = nil
        pendingEditableRetryScheduled = false
        richTextView.textView.isEditable = editable
        updateAccessoryToolbarVisibility()
    }

    func setAccessibilityLabel(_ label: String?) {
        richTextView.textView.accessibilityLabel = label
    }

    func setAccessibilityHint(_ hint: String?) {
        richTextView.textView.accessibilityHint = hint
    }

    private func scheduleEditableRetry(_ editable: Bool) {
        pendingEditableRetryValue = editable
        pendingEditableRetryEditorId = richTextView.editorId
        guard !pendingEditableRetryScheduled else { return }
        pendingEditableRetryScheduled = true
        pendingEditableRetryGeneration &+= 1
        let retryGeneration = pendingEditableRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingEditableRetryGeneration else { return }
            guard let pendingEditable = self.pendingEditableRetryValue else {
                self.pendingEditableRetryScheduled = false
                return
            }
            guard self.pendingEditableRetryEditorId == self.richTextView.editorId else {
                self.clearPendingEditableRetry()
                return
            }
            self.pendingEditableRetryValue = nil
            self.pendingEditableRetryEditorId = nil
            self.pendingEditableRetryScheduled = false
            self.setEditable(pendingEditable)
        }
    }

    func setAutoFocus(_ autoFocus: Bool) {
        guard autoFocus, !didApplyAutoFocus else { return }
        didApplyAutoFocus = true
        focus()
    }

    func setAutoCapitalize(_ autoCapitalize: String?) {
        richTextView.textView.setAutoCapitalize(autoCapitalize)
    }

    func setAutoCorrect(_ autoCorrect: Bool?) {
        richTextView.textView.setAutoCorrect(autoCorrect)
    }

    func setKeyboardType(_ keyboardType: String?) {
        richTextView.textView.setKeyboardType(keyboardType)
    }

    func setShowToolbar(_ showToolbar: Bool) {
        showsToolbar = showToolbar
        updateAccessoryToolbarVisibility()
    }

    func setToolbarPlacement(_ toolbarPlacement: String?) {
        self.toolbarPlacement = toolbarPlacement == "inline" ? "inline" : "keyboard"
        updateAccessoryToolbarVisibility()
    }

    func setHeightBehavior(_ rawHeightBehavior: String) {
        let nextBehavior = EditorHeightBehavior(rawValue: rawHeightBehavior) ?? .fixed
        guard nextBehavior != heightBehavior else { return }
        heightBehavior = nextBehavior
        if nextBehavior != .autoGrow {
            cachedAutoGrowContentHeight = 0
            publishAutoGrowStyleHeight(nil)
        }
        richTextView.heightBehavior = nextBehavior
        invalidateIntrinsicContentSize()
        setNeedsLayout()
        if nextBehavior == .autoGrow {
            emitContentHeightIfNeeded(force: true)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.heightBehavior == .autoGrow else { return }
                self.setNeedsLayout()
                self.layoutIfNeeded()
                let measuredHeight = self.richTextView.remeasureAutoGrowHeight()
                guard measuredHeight > 0 else { return }
                self.cachedAutoGrowContentHeight = measuredHeight
                self.invalidateIntrinsicContentSize()
                self.emitContentHeightIfNeeded(force: true, measuredHeight: measuredHeight)
            }
        }
    }

    func setAllowImageResizing(_ allowImageResizing: Bool) {
        richTextView.allowImageResizing = allowImageResizing
    }

    private func emitContentHeightIfNeeded(force: Bool = false, measuredHeight: CGFloat? = nil) {
        let originatingEditorId = richTextView.editorId
        guard heightBehavior == .autoGrow else { return }
        let resolvedHeight = measuredHeight
            ?? (cachedAutoGrowContentHeight > 0 ? cachedAutoGrowContentHeight : richTextView.intrinsicContentSize.height)
        let contentHeight = ceil(resolvedHeight)
        guard contentHeight > 0 else { return }
        publishAutoGrowStyleHeight(contentHeight)
        guard force || abs(contentHeight - lastEmittedContentHeight) > 0.5 else { return }
        cachedAutoGrowContentHeight = contentHeight
        lastEmittedContentHeight = contentHeight
        guard let event = Self.editorScopedEventPayload(
            ["contentHeight": contentHeight],
            originatingEditorId: originatingEditorId
        ) else { return }
        onContentHeightChange(event)
    }

    private func emitAtomLayout(width: CGFloat) {
        guard let event = Self.editorScopedEventPayload(
            ["width": Double(width)],
            originatingEditorId: richTextView.editorId
        ) else { return }
        onAtomLayout(event)
    }

    private static func atomKey(for view: UIView) -> String? {
        let selector = NSSelectorFromString("nativeId")
        let nativeId = view.responds(to: selector) ? view.value(forKey: "nativeId") as? String : nil
        let identifier = nativeId ?? view.accessibilityIdentifier
        let prefix = "prose-atom:"
        guard let identifier,
              identifier.hasPrefix(prefix),
              identifier.count > prefix.count
        else { return nil }
        return String(identifier.dropFirst(prefix.count))
    }

    private func publishAutoGrowStyleHeight(_ height: CGFloat?) {
        if let height {
            if let lastPublishedAutoGrowHeight,
               abs(height - lastPublishedAutoGrowHeight) <= Self.layoutEpsilon
            {
                return
            }
            lastPublishedAutoGrowHeight = height
        } else {
            guard lastPublishedAutoGrowHeight != nil else { return }
            lastPublishedAutoGrowHeight = nil
        }
        let selector = NSSelectorFromString("setStyleSize:height:")
        guard responds(to: selector) else { return }
        _ = perform(selector, with: nil, with: height.map { NSNumber(value: Double($0)) })
    }

    func setToolbarButtonsJson(_ toolbarButtonsJson: String?) {
        guard lastToolbarItemsJSON != toolbarButtonsJson else { return }
        lastToolbarItemsJSON = toolbarButtonsJson
        toolbarItems = NativeToolbarItem.from(json: toolbarButtonsJson)
        accessoryToolbar.setItems(toolbarItems)
        refreshSystemAssistantToolbarIfNeeded()
    }

    func setToolbarFrameJson(_ toolbarFrameJson: String?) {
        guard lastToolbarFrameJSON != toolbarFrameJson else { return }
        lastToolbarFrameJSON = toolbarFrameJson
        guard let toolbarFrameJson,
              let data = toolbarFrameJson.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            toolbarFramesInWindow = []
            return
        }

        if let frameDictionaries = raw["frames"] as? [[String: Any]] {
            toolbarFramesInWindow = frameDictionaries.compactMap(Self.toolbarFrame(from:))
            return
        }

        toolbarFramesInWindow = Self.toolbarFrame(from: raw).map { [$0] } ?? []
    }

    private static func toolbarFrame(from raw: [String: Any]) -> CGRect? {
        guard let x = (raw["x"] as? NSNumber)?.doubleValue,
              let y = (raw["y"] as? NSNumber)?.doubleValue,
              let width = (raw["width"] as? NSNumber)?.doubleValue,
              let height = (raw["height"] as? NSNumber)?.doubleValue,
              width > 0,
              height > 0
        else {
            return nil
        }

        return CGRect(x: x, y: y, width: width, height: height)
    }

    func setPendingEditorUpdateJson(_ editorUpdateJson: String?) {
        pendingEditorUpdateJSON = editorUpdateJson
        if editorUpdateJson == nil {
            pendingEditorUpdateEditorId = nil
        }
    }

    func setPendingEditorUpdateEditorId(_ editorUpdateEditorId: String?) {
        guard let editorUpdateEditorId,
              let canonicalEditorId = v2CanonicalUInt64String(editorUpdateEditorId),
              canonicalEditorId != "0"
        else {
            pendingEditorUpdateEditorId = nil
            return
        }
        pendingEditorUpdateEditorId = canonicalEditorId
    }

    func setPendingEditorUpdateRevision(_ editorUpdateRevision: Int) {
        pendingEditorUpdateRevision = editorUpdateRevision
    }

    func applyPendingEditorUpdateIfNeeded() {
        guard pendingEditorUpdateRevision != 0 else { return }
        guard pendingEditorUpdateRevision != appliedEditorUpdateRevision else { return }
        let pendingRevision = pendingEditorUpdateRevision
        guard let updateJSON = pendingEditorUpdateJSON else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update JSON is missing",
                fallbackClassification: "missingUpdateJSON"
            )
            consumePendingEditorUpdate(revision: pendingRevision)
            return
        }
        switch applyEditorUpdateOutcome(updateJSON, sourceEditorId: pendingEditorUpdateEditorId) {
        case .applied:
            consumePendingEditorUpdate(revision: pendingRevision)
        case .retryableDeferred:
            schedulePendingEditorUpdateRetry()
        case .rejected:
            // Mark this prop revision consumed before discarding its envelope:
            // OnViewDidUpdateProps can run again with the same values, but a
            // permanent rejection must not report or retry a second time.
            if let sourceEditorId = pendingEditorUpdateEditorId,
               richTextView.editorId != 0,
               sourceEditorId != String(richTextView.editorId)
            {
                pendingEditorUpdateEditorId = nil
            }
            consumePendingEditorUpdate(revision: pendingRevision)
        }
    }

    private func consumePendingEditorUpdate(revision: Int) {
        appliedEditorUpdateRevision = revision
        pendingEditorUpdateJSON = nil
        pendingEditorUpdateRevision = 0
        pendingEditorUpdateRetryScheduled = false
        pendingEditorUpdateRetryEditorId = nil
        pendingEditorUpdateRetryGeneration &+= 1
    }

    private func schedulePendingEditorUpdateRetry() {
        guard !pendingEditorUpdateRetryScheduled else { return }
        pendingEditorUpdateRetryEditorId = richTextView.editorId
        pendingEditorUpdateRetryScheduled = true
        pendingEditorUpdateRetryGeneration &+= 1
        let retryGeneration = pendingEditorUpdateRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingEditorUpdateRetryGeneration else {
                return
            }
            guard self.pendingEditorUpdateRetryEditorId == self.richTextView.editorId else {
                self.pendingEditorUpdateRetryScheduled = false
                self.clearPendingEditorUpdateRetries()
                return
            }
            self.pendingEditorUpdateRetryScheduled = false
            self.pendingEditorUpdateRetryEditorId = nil
            self.applyPendingEditorUpdateIfNeeded()
        }
    }

    // MARK: - View Commands

    func beginExternalTextComposition(sessionId: String) -> String {
        richTextView.textView.beginExternalTextComposition(sessionId: sessionId)
    }

    func updateExternalTextComposition(sessionId: String, text: String) -> String {
        richTextView.textView.updateExternalTextComposition(sessionId: sessionId, text: text)
    }

    func commitExternalTextComposition(sessionId: String, finalText: String) -> String {
        richTextView.textView.commitExternalTextComposition(
            sessionId: sessionId,
            finalText: finalText
        )
    }

    func cancelExternalTextComposition(sessionId: String, cause: String) -> String {
        richTextView.textView.cancelExternalTextComposition(sessionId: sessionId, cause: cause)
    }

    private func reportRejectedEditorUpdateEnvelope(
        _ message: String,
        fallbackClassification: String
    ) {
        if let adapter = EditorV2Registry.adapter(forLegacyId: richTextView.editorId) {
            adapter.rejectExternalRenderEnvelope(message)
        } else {
            editorUpdateInternalRejections.append(
                "boundary/FFI_RESULT_INVALID/\(fallbackClassification)"
            )
        }
    }

    func applyRemoteCommitRefresh() {
        // Preparing an external update commits a live composition. The commit
        // re-bases the adapter itself, so leave the half-typed word alone.
        guard !richTextView.textView.hasPendingCompositionForExternalRefresh else { return }
        let boundEditorId = richTextView.editorId
        guard boundEditorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: boundEditorId),
              !adapter.isDestroyed
        else {
            return
        }
        let autonomousOwner = autonomousErrorBindingAdapter === adapter
            && autonomousErrorBindingToken.map { adapter.isNativeBindingOwner(token: $0) } == true
        guard autonomousOwner || richTextView.textView.ownsNativeBinding(adapter) else { return }
        let preflight = richTextView.textView.prepareForExternalEditorUpdateResult()
        guard preflight.ready else { return }
        guard let update = preflight.adoptedUpdateJSON
            ?? adapter.refreshFromRustState(mirrorSelection: nil)
        else {
            return
        }
        if !richTextView.textView.applyUpdateJSON(update),
           let recovery = adapter.recoverNativeRender() {
            richTextView.textView.applyUpdateJSON(recovery)
        }
    }

    private func applyEditorUpdateOutcome(
        _ updateJson: String,
        sourceEditorId: String?
    ) -> EditorUpdateApplyOutcome {
        let boundEditorId = richTextView.editorId
        guard boundEditorId != 0 else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update has no bound adapter",
                fallbackClassification: "missingAdapter"
            )
            return .rejected
        }
        guard let sourceEditorId else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update source id is missing or malformed",
                fallbackClassification: "malformedSourceEditorId"
            )
            return .rejected
        }
        guard sourceEditorId == String(boundEditorId) else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update source does not match the bound canonical editor id",
                fallbackClassification: "sourceEditorMismatch"
            )
            return .rejected
        }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: boundEditorId) else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update adapter is missing",
                fallbackClassification: "missingAdapter"
            )
            return .rejected
        }
        guard !adapter.isDestroyed else {
            adapter.rejectExternalRenderEnvelope("external editor update adapter is destroyed")
            return .rejected
        }
        // A malformed external envelope is permanent, even if UIKit is in a
        // transient composition that would otherwise make application
        // retryable. Classify it before entering composition preflight.
        guard adapter.validateExternalRender(updateJson) else {
            return .rejected
        }
        if isSupersededEditorUpdate(updateJson) {
            return .applied
        }
        let preflight = richTextView.textView.prepareForExternalEditorUpdateResult()
        guard preflight.ready else {
            return .retryableDeferred
        }
        if preflight.adoptedUpdateJSON != nil {
            return .applied
        }
        let adoptedUpdateJSON = adapter.adoptExternalRender(updateJson)
        guard let adoptedUpdateJSON else {
            // The adapter owns strict-parser and destroyed-race reporting.
            // Do not add a second view-side record for the same rejection.
            return .rejected
        }
        isApplyingJSUpdate = true
        defer { isApplyingJSUpdate = false }
        imageLoadOwner.withCurrent {
            // The adapter cache and the payload are paired by the same
            // editor-scoped call above; do not let the view display a render
            // whose revision has not already been adopted for native input.
            _ = richTextView.textView.applyUpdateJSON(adoptedUpdateJSON)
        }
        return .applied
    }

    /// Apply an editor update from JS. Sets the echo-suppression flag so the
    /// resulting delegate callback is NOT re-dispatched back to JS.
    @discardableResult
    func applyEditorUpdate(_ updateJson: String) -> Bool {
        let sourceEditorId = richTextView.editorId == 0 ? nil : String(richTextView.editorId)
        switch applyEditorUpdateOutcome(updateJson, sourceEditorId: sourceEditorId) {
        case .applied:
            return true
        case .retryableDeferred:
            scheduleViewCommandUpdateRetry(updateJson, sourceEditorId: sourceEditorId)
            return false
        case .rejected:
            return false
        }
    }

    private func scheduleViewCommandUpdateRetry(_ updateJson: String, sourceEditorId: String?) {
        pendingViewCommandUpdateJSON = updateJson
        pendingViewCommandUpdateEditorId = richTextView.editorId
        guard !pendingViewCommandUpdateRetryScheduled else { return }
        pendingViewCommandUpdateRetryScheduled = true
        pendingViewCommandUpdateRetryGeneration &+= 1
        let retryGeneration = pendingViewCommandUpdateRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingViewCommandUpdateRetryGeneration else {
                return
            }
            guard self.pendingViewCommandUpdateJSON != nil else {
                self.pendingViewCommandUpdateRetryScheduled = false
                return
            }
            guard self.pendingViewCommandUpdateEditorId == self.richTextView.editorId else {
                self.pendingViewCommandUpdateJSON = nil
                self.pendingViewCommandUpdateEditorId = nil
                self.pendingViewCommandUpdateRetryScheduled = false
                return
            }
            guard self.richTextView.editorId != 0 else {
                self.pendingViewCommandUpdateJSON = nil
                self.pendingViewCommandUpdateEditorId = nil
                self.pendingViewCommandUpdateRetryScheduled = false
                return
            }
            let updateJSON = self.pendingViewCommandUpdateJSON
            self.pendingViewCommandUpdateJSON = nil
            self.pendingViewCommandUpdateEditorId = nil
            self.pendingViewCommandUpdateRetryScheduled = false
            guard let updateJSON else { return }
            switch self.applyEditorUpdateOutcome(
                updateJSON,
                sourceEditorId: sourceEditorId
            ) {
            case .applied, .rejected:
                return
            case .retryableDeferred:
                self.scheduleViewCommandUpdateRetry(updateJSON, sourceEditorId: sourceEditorId)
            }
        }
    }

    func prepareForEditorCommandJSON() -> String {
        isApplyingJSUpdate = true
        defer { isApplyingJSUpdate = false }
        let preparation = richTextView.textView.prepareForExternalEditorCommand()
        return NativeEditorViewRegistry.commandPreparationJSON(
            ready: preparation.ready,
            updateJSON: preparation.updateJSON,
            blockedReason: preparation.blockedReason
        )
    }

    // MARK: - Focus Commands

    func focus() {
        _ = richTextView.textView.becomeFirstResponder()
    }

    func blur() {
        clearRecentToolbarTouch()
        _ = richTextView.textView.resignFirstResponder()
    }

    func getCaretRectJson() -> String? {
        layoutIfNeeded()
        richTextView.layoutIfNeeded()

        guard let caretRect = richTextView.currentCaretRect() else {
            return nil
        }
        let editorRect = richTextView.convert(caretRect, to: self)
        let payload: [String: Any] = [
            "x": editorRect.minX,
            "y": editorRect.minY,
            "width": editorRect.width,
            "height": editorRect.height,
            "editorWidth": bounds.width,
            "editorHeight": bounds.height,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return nil
        }
        return json
    }

    // MARK: - Focus Notifications

    @objc private func textViewDidBeginEditing(_ notification: Notification) {
        let originatingEditorId = richTextView.textView.editorId
        installOutsideTapRecognizerIfNeeded()
        richTextView.textView.refreshSelectionVisualState()
        refreshMentionQuery()
        guard let event = Self.editorScopedEventPayload(
            ["isFocused": true],
            originatingEditorId: originatingEditorId
        ) else { return }
        onFocusChange(event)
    }

    @objc private func textViewDidEndEditing(_ notification: Notification) {
        let originatingEditorId = richTextView.textView.editorId
        if consumeToolbarFocusPreservationForBlur() {
            DispatchQueue.main.async { [weak self] in
                _ = self?.richTextView.textView.becomeFirstResponder()
            }
            return
        }

        uninstallOutsideTapRecognizer()
        richTextView.textView.refreshSelectionVisualState()
        clearMentionQueryStateAndHidePopover()
        guard let event = Self.editorScopedEventPayload(
            ["isFocused": false],
            originatingEditorId: originatingEditorId
        ) else { return }
        onFocusChange(event)
    }

    @objc private func handleOutsideTap(_ recognizer: UITapGestureRecognizer) {
        guard recognizer.state == .ended else { return }
        guard richTextView.textView.isFirstResponder else { return }
        guard let tapWindow = gestureWindow ?? window else { return }
        let locationInWindow = recognizer.location(in: tapWindow)
        guard shouldHandleOutsideTap(locationInWindow: locationInWindow, touchedView: nil) else {
            return
        }
        clearRecentToolbarTouch()
        blur()
    }

    private func installOutsideTapRecognizerIfNeeded() {
        guard let window else { return }
        if gestureWindow === window, window.gestureRecognizers?.contains(outsideTapGestureRecognizer) == true {
            return
        }
        uninstallOutsideTapRecognizer()
        window.addGestureRecognizer(outsideTapGestureRecognizer)
        gestureWindow = window
    }

    private func uninstallOutsideTapRecognizer() {
        if let window = gestureWindow {
            window.removeGestureRecognizer(outsideTapGestureRecognizer)
        }
        gestureWindow = nil
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        guard gestureRecognizer === outsideTapGestureRecognizer else { return true }
        guard let tapWindow = gestureWindow ?? window else { return true }
        let locationInWindow = touch.location(in: tapWindow)
        return prepareOutsideTapForFocusHandling(
            locationInWindow: locationInWindow,
            touchedView: touch.view
        )
    }

    private func prepareOutsideTapForFocusHandling(
        locationInWindow: CGPoint,
        touchedView: UIView?
    ) -> Bool {
        if isLocationInStandaloneToolbarFrame(locationInWindow) {
            markRecentToolbarTouch()
        }
        let result = shouldHandleOutsideTap(
            locationInWindow: locationInWindow,
            touchedView: touchedView
        )
        if result {
            clearRecentToolbarTouch()
        }
        return result
    }

    private func markRecentToolbarTouch() {
        lastToolbarTouchUptime = ProcessInfo.processInfo.systemUptime
    }

    private func clearRecentToolbarTouch() {
        lastToolbarTouchUptime = -Double.infinity
    }

    private func shouldPreserveFocusAfterToolbarTouch() -> Bool {
        ProcessInfo.processInfo.systemUptime - lastToolbarTouchUptime <= 0.75
    }

    private func consumeToolbarFocusPreservationForBlur() -> Bool {
        guard shouldPreserveFocusAfterToolbarTouch() else { return false }
        clearRecentToolbarTouch()
        return true
    }

    private func isLocationInStandaloneToolbarFrame(_ locationInWindow: CGPoint) -> Bool {
        toolbarFramesInWindow.contains(where: { $0.contains(locationInWindow) })
    }

    private func shouldHandleOutsideTap(
        locationInWindow: CGPoint,
        touchedView: UIView?
    ) -> Bool {
        if let touchedView, touchedView.isDescendant(of: self) {
            return false
        }
        if let tapWindow = gestureWindow ?? window {
            let editorFrameInWindow = convert(bounds, to: tapWindow)
            if editorFrameInWindow.contains(locationInWindow) {
                return false
            }
        }
        if let touchedView, touchedView.isDescendant(of: accessoryToolbar) {
            return false
        }
        if isLocationInStandaloneToolbarFrame(locationInWindow) {
            return false
        }
        return true
    }

    // MARK: - EditorTextViewDelegate

    func editorTextView(
        _ textView: EditorTextView,
        didEndExternalTextComposition resultJSON: String
    ) {
        schedulePendingAtomsWakeIfNeeded()
        dispatchExternalTextCompositionEnd(resultJSON)
    }

    private func dispatchExternalTextCompositionEnd(_ resultJSON: String) {
        let payload: [String: Any] = [
            "editorId": String(richTextView.editorId),
            "resultJson": resultJSON,
        ]
        if let onExternalTextCompositionEndForTesting {
            onExternalTextCompositionEndForTesting(payload)
        } else {
            onExternalTextCompositionEnd(payload)
        }
    }

    func editorTextView(_ textView: EditorTextView, selectionDidChange anchor: UInt32, head: UInt32) {
        let originatingEditorId = textView.editorId
        let stateJSON = refreshToolbarStateFromEditorSelection()
        refreshSystemAssistantToolbarIfNeeded()
        refreshMentionQuery()
        richTextView.refreshRemoteSelections()
        var event: [String: Any] = ["anchor": Int(anchor), "head": Int(head)]
        if let stateJSON {
            event["stateJson"] = stateJSON
        }
        guard let scopedEvent = Self.editorScopedEventPayload(
            event,
            originatingEditorId: originatingEditorId
        ) else { return }
        onSelectionChange(scopedEvent)
    }

    func editorTextView(_ textView: EditorTextView, didReceiveUpdate updateJSON: String) {
        schedulePendingAtomsWakeIfNeeded()
        if let revision = renderRevision(fromUpdateJSON: updateJSON) {
            renderedRevision = revision
        }
        // Capture both fields from the same committed atomic update before
        // any view work can cause a rebind. The event must never relabel A's
        // update as B merely because the host changes editorId afterwards.
        let nativeCommitEvent = Self.nativeCommitEventPayload(
            originatingEditorId: String(textView.editorId),
            updateJSON: updateJSON
        )
        if let state = NativeToolbarState(updateJSON: updateJSON) {
            toolbarState = state
            accessoryToolbar.apply(state: state)
            refreshSystemAssistantToolbarIfNeeded()
        }
        refreshMentionQuery()
        richTextView.refreshRemoteSelections()
        guard !isApplyingJSUpdate else { return }
        guard let nativeCommitEvent else { return }
        onEditorUpdate(nativeCommitEvent)
    }

    /// The canonical JS commit contract. `originatingEditorId` is captured
    /// synchronously from the text view which applied `updateJSON`; it is not
    /// read from the host view after asynchronous rebind work.
    static func nativeCommitEventPayload(
        originatingEditorId: String,
        updateJSON: String
    ) -> [String: Any]? {
        guard let editorId = v2CanonicalUInt64String(originatingEditorId),
              editorId != "0",
              let nativeEditorId = UInt64(editorId),
              let data = updateJSON.data(using: .utf8),
              let update = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let rawRevision = update["documentVersion"] as? String,
              let documentRevision = v2CanonicalUInt64String(rawRevision),
              let revision = UInt64(documentRevision),
              let atomicUpdateJSON = EditorV2Registry.adapter(forLegacyId: nativeEditorId)?
                .atomicRenderJSON(matchingDocumentRevision: revision)
        else {
            return nil
        }
        return [
            "editorId": editorId,
            "documentRevision": documentRevision,
            "updateJson": atomicUpdateJSON,
        ]
    }

    /// Every non-commit view event is labelled with the editor that produced
    /// it, captured before refresh/rebind work can change the view binding.
    static func editorScopedEventPayload(
        _ payload: [String: Any],
        originatingEditorId: UInt64
    ) -> [String: Any]? {
        guard originatingEditorId != 0,
              let editorId = v2CanonicalUInt64String(String(originatingEditorId))
        else {
            return nil
        }
        var scopedPayload = payload
        scopedPayload["editorId"] = editorId
        return scopedPayload
    }

    @discardableResult
    private func refreshToolbarStateFromEditorSelection() -> String? {
        guard richTextView.editorId != 0 else { return nil }
        let stateJSON = EditorV2Shadow.getSelectionState(id: richTextView.editorId)
        guard let state = NativeToolbarState(updateJSON: stateJSON) else { return nil }
        toolbarState = state
        accessoryToolbar.apply(state: state)
        return stateJSON
    }

    private func configureAccessoryToolbar() {
        accessoryToolbar.onPressItem = { [weak self] item in
            self?.handleToolbarItemPress(item)
        }
        accessoryToolbar.onSelectMentionSuggestion = { [weak self] suggestion in
            self?.insertMentionSuggestion(suggestion)
        }
        accessoryToolbar.setItems(toolbarItems)
        accessoryToolbar.apply(state: toolbarState)
        updateAccessoryToolbarVisibility()
    }

    private func refreshMentionQuery() {
        guard richTextView.editorId != 0,
              richTextView.textView.isFirstResponder,
              let mentions = addons.mentions
        else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        guard prepareForInputAccessoryMutationOrRetry(.refreshMentionQuery) else { return }

        guard let queryState = currentMentionQueryState(trigger: mentions.trigger) else {
            emitMentionQueryChange(query: "", trigger: mentions.trigger, anchor: 0, head: 0, isActive: false)
            clearMentionQueryStateAndHidePopover()
            return
        }

        let suggestions = filteredMentionSuggestions(for: queryState, config: mentions)
        mentionQueryState = queryState
        accessoryToolbar.apply(mentionTheme: richTextView.textView.theme?.mentions ?? mentions.theme)
        let didChangeToolbarHeight = accessoryToolbar.setMentionSuggestions(
            suggestions,
            trigger: mentions.trigger
        )
        refreshSystemAssistantToolbarIfNeeded()
        if didChangeToolbarHeight,
           richTextView.textView.isFirstResponder,
           richTextView.textView.inputAccessoryView === accessoryToolbar
        {
            richTextView.textView.reloadInputViews()
        }
        markAccessoryMutationSucceeded(.refreshMentionQuery)
        emitMentionQueryChange(
            query: queryState.query,
            trigger: queryState.trigger,
            anchor: queryState.anchor,
            head: queryState.head,
            isActive: true
        )
    }

    private func clearMentionQueryStateAndHidePopover() {
        guard prepareForInputAccessoryMutationOrRetry(.clearMentionQueryState) else { return }
        mentionQueryState = nil
        let didChangeToolbarHeight = accessoryToolbar.setMentionSuggestions([])
        refreshSystemAssistantToolbarIfNeeded()
        if didChangeToolbarHeight,
           richTextView.textView.isFirstResponder,
           richTextView.textView.inputAccessoryView === accessoryToolbar
        {
            richTextView.textView.reloadInputViews()
        }
        markAccessoryMutationSucceeded(.clearMentionQueryState)
    }

    private func emitMentionQueryChange(
        query: String,
        trigger: String,
        anchor: UInt32,
        head: UInt32,
        isActive: Bool
    ) {
        let payload: [String: Any] = [
            "type": "mentionsQueryChange",
            "query": query,
            "trigger": trigger,
            "range": [
                "anchor": Int(anchor),
                "head": Int(head),
            ],
            "isActive": isActive,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }
        guard json != lastMentionEventJSON else { return }
        lastMentionEventJSON = json
        dispatchAddonEvent(json)
    }

    private func resolvedMentionAttrs(
        trigger: String,
        suggestion: NativeMentionSuggestion
    ) -> [String: Any] {
        var attrs = suggestion.attrs
        if attrs["label"] == nil {
            attrs["label"] = suggestion.label
        }
        if attrs["mentionSuggestionChar"] == nil {
            attrs["mentionSuggestionChar"] = trigger
        }
        return attrs
    }

    private func emitMentionSelect(
        trigger: String,
        suggestion: NativeMentionSuggestion,
        attrs: [String: Any]
    ) {
        let payload: [String: Any] = [
            "type": "mentionsSelect",
            "trigger": trigger,
            "suggestionKey": suggestion.key,
            "attrs": attrs,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }
        dispatchAddonEvent(json)
    }

    private func emitMentionSelectRequest(
        trigger: String,
        suggestion: NativeMentionSuggestion,
        attrs: [String: Any],
        range: MentionQueryState,
        preflightUpdateJSON: String? = nil
    ) {
        var payload: [String: Any] = [
            "type": "mentionsSelectRequest",
            "trigger": trigger,
            "suggestionKey": suggestion.key,
            "attrs": attrs,
            "range": [
                "anchor": Int(range.anchor),
                "head": Int(range.head),
            ],
        ]
        if let preflightUpdateJSON {
            payload["updateJson"] = preflightUpdateJSON
        }
        if let documentVersion = documentVersion(fromUpdateJSON: preflightUpdateJSON) {
            payload["documentVersion"] = documentVersion
        }
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }
        dispatchAddonEvent(json)
    }

    private func dispatchAddonEvent(_ json: String) {
        let originatingEditorId = richTextView.editorId
        lastAddonEventJSONForTestingValue = json
        guard let event = Self.editorScopedEventPayload(
            ["eventJson": json],
            originatingEditorId: originatingEditorId
        ) else { return }
        onAddonEvent(event)
    }

    private func isSupersededEditorUpdate(_ updateJSON: String) -> Bool {
        guard let rendered = renderedRevision,
              let incoming = renderRevision(fromUpdateJSON: updateJSON)
        else {
            return false
        }
        if incoming.document != rendered.document {
            return incoming.document < rendered.document
        }
        return incoming.state < rendered.state
    }

    private func renderRevision(
        fromUpdateJSON updateJSON: String
    ) -> (document: UInt64, state: UInt64)? {
        guard let data = updateJSON.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let document = (raw["documentVersion"] as? String)
                .flatMap(v2CanonicalUInt64String)
                .flatMap(UInt64.init),
              let state = (raw["stateRevision"] as? String)
                .flatMap(v2CanonicalUInt64String)
                .flatMap(UInt64.init)
        else {
            return nil
        }
        return (document: document, state: state)
    }

    private func documentVersion(fromUpdateJSON updateJSON: String?) -> String? {
        guard let updateJSON,
              let data = updateJSON.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }
        return (raw["documentVersion"] as? String).flatMap(v2CanonicalUInt64String)
    }

    private func filteredMentionSuggestions(
        for queryState: MentionQueryState,
        config: NativeMentionsAddonConfig
    ) -> [NativeMentionSuggestion] {
        let query = queryState.query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else {
            return config.suggestions
        }

        return config.suggestions.filter { suggestion in
            suggestion.title.lowercased().contains(query)
                || suggestion.label.lowercased().contains(query)
                || (suggestion.subtitle?.lowercased().contains(query) ?? false)
        }
    }

    private func currentMentionQueryState(trigger: String) -> MentionQueryState? {
        guard let selectedTextRange = richTextView.textView.selectedTextRange,
              selectedTextRange.isEmpty
        else {
            return nil
        }

        let currentText = richTextView.textView.text ?? ""
        let cursorUtf16Offset = richTextView.textView.offset(
            from: richTextView.textView.beginningOfDocument,
            to: selectedTextRange.start
        )
        let visibleCursorScalar = PositionBridge.utf16OffsetToScalar(
            cursorUtf16Offset,
            in: currentText
        )

        guard let visibleQueryState = resolveMentionQueryState(
            in: currentText,
            cursorScalar: visibleCursorScalar,
            trigger: trigger,
            isCaretInsideMention: isCaretInsideMention(
                cursorScalar: PositionBridge.textViewToScalar(
                    selectedTextRange.start,
                    in: richTextView.textView
                )
            )
        ) else {
            return nil
        }

        let anchorUtf16Offset = PositionBridge.scalarToUtf16Offset(
            visibleQueryState.anchor,
            in: currentText
        )
        let headUtf16Offset = PositionBridge.scalarToUtf16Offset(
            visibleQueryState.head,
            in: currentText
        )

        return MentionQueryState(
            query: visibleQueryState.query,
            trigger: visibleQueryState.trigger,
            anchor: PositionBridge.utf16OffsetToScalar(
                anchorUtf16Offset,
                in: richTextView.textView
            ),
            head: PositionBridge.utf16OffsetToScalar(
                headUtf16Offset,
                in: richTextView.textView
            )
        )
    }

    private func isCaretInsideMention(cursorScalar: UInt32) -> Bool {
        let utf16Offset = PositionBridge.scalarToUtf16Offset(
            cursorScalar,
            in: richTextView.textView.text ?? ""
        )
        let textStorage = richTextView.textView.textStorage
        guard textStorage.length > 0 else { return false }
        let candidateOffsets = [
            min(max(utf16Offset, 0), max(textStorage.length - 1, 0)),
            min(max(utf16Offset - 1, 0), max(textStorage.length - 1, 0)),
        ]

        for offset in candidateOffsets where offset >= 0 && offset < textStorage.length {
            if let nodeType = textStorage.attribute(RenderBridgeAttributes.voidNodeType, at: offset, effectiveRange: nil) as? String,
               nodeType == "mention" {
                return true
            }
        }
        return false
    }

    private func insertMentionSuggestion(
        _ suggestion: NativeMentionSuggestion
    ) {
        insertMentionSuggestion(suggestionKey: suggestion.key)
    }

    private func insertMentionSuggestion(
        retryScope: PendingMentionSuggestionRetry
    ) {
        insertMentionSuggestion(
            suggestionKey: retryScope.suggestionKey,
            retryScope: retryScope
        )
    }

    private func insertMentionSuggestion(
        suggestionKey: String,
        retryScope: PendingMentionSuggestionRetry? = nil
    ) {
        guard let mentions = addons.mentions,
              mentionQueryState != nil
        else {
            return
        }
        if let retryScope,
           !isMentionSuggestionRetryScopeCurrent(retryScope)
        {
            return
        }

        let scopedQueryState = currentMentionQueryState(trigger: mentions.trigger) ?? mentionQueryState
        guard let scopedQueryState else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        let preparation = richTextView.textView.prepareForExternalEditorCommand()
        guard preparation.ready else {
            scheduleMentionSuggestionRetry(
                PendingMentionSuggestionRetry(
                    suggestionKey: suggestionKey,
                    editorId: richTextView.editorId,
                    trigger: mentions.trigger,
                    query: scopedQueryState.query,
                    anchor: scopedQueryState.anchor,
                    head: scopedQueryState.head,
                    documentVersion: currentDocumentVersion(),
                    textSnapshot: richTextView.textView.text ?? ""
                )
            )
            return
        }
        let queryState = currentMentionQueryState(trigger: mentions.trigger)
            ?? (richTextView.textView.isFirstResponder ? nil : mentionQueryState)
        guard let queryState else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        if let retryScope,
           !doesMentionQueryState(
                queryState,
                match: retryScope,
                acceptingPreflightDocumentVersion: documentVersion(fromUpdateJSON: preparation.updateJSON),
                currentText: richTextView.textView.text ?? ""
           )
        {
            return
        }
        guard let currentSuggestion = filteredMentionSuggestions(
            for: queryState,
            config: mentions
        ).first(where: { $0.key == suggestionKey }) else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        mentionQueryState = queryState

        let attrs = resolvedMentionAttrs(trigger: mentions.trigger, suggestion: currentSuggestion)
        if mentions.resolveSelectionAttrs || mentions.resolveTheme {
            emitMentionSelectRequest(
                trigger: mentions.trigger,
                suggestion: currentSuggestion,
                attrs: attrs,
                range: queryState,
                preflightUpdateJSON: preparation.updateJSON
            )
            lastMentionEventJSON = nil
            clearMentionQueryStateAndHidePopover()
            return
        }
        let payload: [String: Any] = [
            "type": "doc",
            "content": [[
                "type": "mention",
                "attrs": attrs,
            ]],
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }

        let updateJSON = EditorV2Shadow.insertContentJsonAtSelectionScalar(
            id: richTextView.editorId,
            scalarAnchor: queryState.anchor,
            scalarHead: queryState.head,
            json: json
        )
        imageLoadOwner.withCurrent {
            _ = richTextView.textView.applyUpdateJSON(updateJSON)
        }
        emitMentionSelect(trigger: mentions.trigger, suggestion: currentSuggestion, attrs: attrs)
        lastMentionEventJSON = nil
        clearMentionQueryStateAndHidePopover()
    }

    private func scheduleMentionSuggestionRetry(_ retry: PendingMentionSuggestionRetry) {
        pendingMentionSuggestionRetry = retry
        guard !pendingMentionSuggestionRetryScheduled else { return }
        pendingMentionSuggestionRetryScheduled = true
        pendingMentionSuggestionRetryGeneration &+= 1
        let retryGeneration = pendingMentionSuggestionRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingMentionSuggestionRetryGeneration else { return }
            guard let retry = self.pendingMentionSuggestionRetry else {
                self.pendingMentionSuggestionRetryScheduled = false
                return
            }
            guard retry.editorId == self.richTextView.editorId else {
                self.clearPendingMentionSuggestionRetry()
                return
            }
            self.pendingMentionSuggestionRetry = nil
            self.pendingMentionSuggestionRetryScheduled = false
            self.insertMentionSuggestion(retryScope: retry)
        }
    }

    private func isMentionSuggestionRetryScopeCurrent(
        _ retry: PendingMentionSuggestionRetry
    ) -> Bool {
        guard retry.editorId == richTextView.editorId,
              addons.mentions?.trigger == retry.trigger
        else {
            return false
        }
        let queryState = currentMentionQueryState(trigger: retry.trigger) ?? mentionQueryState
        guard let queryState else { return false }
        guard doesMentionQueryStateMatchRetryIdentity(queryState, match: retry) else {
            return false
        }
        return isMentionSuggestionRetryDocumentVersionCurrent(retry)
    }

    private func doesMentionQueryState(
        _ queryState: MentionQueryState,
        match retry: PendingMentionSuggestionRetry,
        acceptingPreflightDocumentVersion preflightDocumentVersion: String? = nil,
        currentText: String? = nil
    ) -> Bool {
        guard doesMentionQueryStateMatchRetryIdentity(queryState, match: retry) else {
            return false
        }

        let currentVersion = currentDocumentVersion()
        var acceptedPreflightVersionChange = false
        if let retryVersion = retry.documentVersion,
           let currentVersion,
           currentVersion != retryVersion
        {
            guard let preflightDocumentVersion,
                  currentVersion == preflightDocumentVersion
            else {
                return false
            }
            acceptedPreflightVersionChange = true
        }

        if queryState.anchor == retry.anchor && queryState.head == retry.head {
            return true
        }

        guard acceptedPreflightVersionChange else {
            return false
        }

        guard let currentText,
              let diff = mentionRetryTextDiff(
                from: retry.textSnapshot,
                to: currentText
              ),
              let mappedRange = mappedMentionRetryRange(retry, through: diff)
        else {
            return false
        }

        return queryState.anchor == mappedRange.anchor && queryState.head == mappedRange.head
    }

    private func doesMentionQueryStateMatchRetryIdentity(
        _ queryState: MentionQueryState,
        match retry: PendingMentionSuggestionRetry
    ) -> Bool {
        queryState.trigger == retry.trigger && queryState.query == retry.query
    }

    private func isMentionSuggestionRetryDocumentVersionCurrent(
        _ retry: PendingMentionSuggestionRetry
    ) -> Bool {
        let currentVersion = currentDocumentVersion()
        if let retryVersion = retry.documentVersion,
           let currentVersion,
           currentVersion != retryVersion
        {
            return false
        }
        return true
    }

    private func mentionRetryTextDiff(
        from oldText: String,
        to newText: String
    ) -> MentionRetryTextDiff? {
        let oldScalars = Array(oldText.unicodeScalars)
        let newScalars = Array(newText.unicodeScalars)
        let sharedLength = min(oldScalars.count, newScalars.count)

        var prefix = 0
        while prefix < sharedLength,
              oldScalars[prefix] == newScalars[prefix]
        {
            prefix += 1
        }

        var oldEnd = oldScalars.count
        var newEnd = newScalars.count
        while oldEnd > prefix,
              newEnd > prefix,
              oldScalars[oldEnd - 1] == newScalars[newEnd - 1]
        {
            oldEnd -= 1
            newEnd -= 1
        }

        guard prefix != oldEnd || prefix != newEnd else {
            return nil
        }

        return MentionRetryTextDiff(
            start: prefix,
            oldEnd: oldEnd,
            newEnd: newEnd
        )
    }

    private func mappedMentionRetryRange(
        _ retry: PendingMentionSuggestionRetry,
        through diff: MentionRetryTextDiff
    ) -> (anchor: UInt32, head: UInt32)? {
        let anchor = Int(retry.anchor)
        let head = Int(retry.head)
        guard anchor <= head else { return nil }

        if head <= diff.start {
            return (retry.anchor, retry.head)
        }

        if anchor >= diff.oldEnd {
            let delta = diff.newEnd - diff.oldEnd
            let mappedAnchor = anchor + delta
            let mappedHead = head + delta
            guard mappedAnchor >= 0,
                  mappedHead >= mappedAnchor,
                  mappedHead <= Int(UInt32.max)
            else {
                return nil
            }
            return (UInt32(mappedAnchor), UInt32(mappedHead))
        }

        return nil
    }

    private func currentDocumentVersion() -> String? {
        guard richTextView.editorId != 0 else { return nil }
        return documentVersion(fromUpdateJSON: EditorV2Shadow.getCurrentState(id: richTextView.editorId))
    }

    func setMentionQueryStateForTesting(_ state: MentionQueryState?) {
        mentionQueryState = state
    }

    func currentMentionQueryStateForTesting(trigger: String) -> MentionQueryState? {
        currentMentionQueryState(trigger: trigger)
    }

    func setMentionSuggestionsForTesting(_ suggestions: [NativeMentionSuggestion]) {
        accessoryToolbar.setMentionSuggestions(
            suggestions,
            trigger: mentionQueryState?.trigger ?? "@"
        )
    }

    func isShowingMentionSuggestionsForTesting() -> Bool {
        accessoryToolbar.isShowingMentionSuggestions
    }

    func lastAddonEventJSONForTesting() -> String? {
        lastAddonEventJSONForTestingValue
    }

    func triggerMentionSuggestionTapForTesting(at index: Int) {
        accessoryToolbar.triggerMentionSuggestionTapForTesting(at: index)
    }

    func inputAccessoryViewForTesting() -> UIView? {
        richTextView.textView.inputAccessoryView
    }

    func isUsingAccessoryToolbarForTesting() -> Bool {
        richTextView.textView.inputAccessoryView === accessoryToolbar
    }

    func isUsingAccessoryPlaceholderForTesting() -> Bool {
        richTextView.textView.inputAccessoryView === accessoryPlaceholder
    }

    func markRecentToolbarTouchForTesting() {
        markRecentToolbarTouch()
    }

    func shouldPreserveFocusAfterToolbarTouchForTesting() -> Bool {
        shouldPreserveFocusAfterToolbarTouch()
    }

    func consumeToolbarFocusPreservationForTesting() -> Bool {
        consumeToolbarFocusPreservationForBlur()
    }

    func prepareOutsideTapForFocusHandlingForTesting(
        locationInWindow: CGPoint,
        touchedView: UIView? = nil
    ) -> Bool {
        prepareOutsideTapForFocusHandling(
            locationInWindow: locationInWindow,
            touchedView: touchedView
        )
    }

    private func updateAccessoryToolbarVisibility() {
        guard prepareForInputAccessoryMutationOrRetry(.updateAccessoryToolbarVisibility) else { return }
        refreshSystemAssistantToolbarIfNeeded()
        let nextAccessoryView: UIView?
        if showsToolbar &&
            toolbarPlacement == "keyboard" &&
            richTextView.textView.isEditable &&
            !shouldUseSystemAssistantToolbar
        {
            nextAccessoryView = accessoryToolbar
        } else if richTextView.textView.isEditable && !shouldUseSystemAssistantToolbar {
            nextAccessoryView = accessoryPlaceholder
        } else {
            nextAccessoryView = nil
        }
        if richTextView.textView.inputAccessoryView !== nextAccessoryView {
            richTextView.textView.inputAccessoryView = nextAccessoryView
            if richTextView.textView.isFirstResponder {
                richTextView.textView.reloadInputViews()
            }
        }
        markAccessoryMutationSucceeded(.updateAccessoryToolbarVisibility)
    }

    private var shouldUseSystemAssistantToolbar: Bool {
        false
    }

    private func refreshSystemAssistantToolbarIfNeeded() {
        guard #available(iOS 26.0, *) else { return }

        let assistantItem = richTextView.textView.inputAssistantItem
        assistantItem.allowsHidingShortcuts = false
        assistantItem.leadingBarButtonGroups = []
        assistantItem.trailingBarButtonGroups = []
    }

    private func handleListToggle(_ listType: String) {
        let isActive = toolbarState.nodes[listType] == true
        richTextView.textView.performToolbarToggleList(listType, isActive: isActive)
    }

    private func handleToolbarItemPress(_ item: NativeToolbarItem) {
        let originatingEditorId = richTextView.editorId
        switch item.type {
        case .mark:
            guard let mark = item.mark else { return }
            richTextView.textView.performToolbarToggleMark(mark)
        case .heading:
            guard let level = item.headingLevel else { return }
            richTextView.textView.performToolbarToggleHeading(level)
        case .blockquote:
            richTextView.textView.performToolbarToggleBlockquote()
        case .list:
            guard let listType = item.listType?.rawValue else { return }
            handleListToggle(listType)
        case .command:
            switch item.command {
            case .indentList:
                richTextView.textView.performToolbarIndentListItem()
            case .outdentList:
                richTextView.textView.performToolbarOutdentListItem()
            case .undo:
                richTextView.textView.performToolbarUndo()
            case .redo:
                richTextView.textView.performToolbarRedo()
            case .none:
                break
            }
        case .node:
            guard let nodeType = item.nodeType else { return }
            richTextView.textView.performToolbarInsertNode(nodeType)
        case .action:
            guard let key = item.key else { return }
            guard let event = Self.editorScopedEventPayload(
                ["key": key],
                originatingEditorId: originatingEditorId
            ) else { return }
            onToolbarAction(event)
        case .group:
            break
        case .separator:
            break
        }
    }
}
