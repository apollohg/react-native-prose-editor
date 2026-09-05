import UIKit
import os

/// UITextView that intercepts input and routes it through editor-core.
///
/// The text view is a rendering surface, not a text engine: intent (typing,
/// deleting, pasting, autocorrect) goes to Rust, which returns render elements
/// that RenderBridge converts back to NSAttributedString.
///
/// IME: UITextView owns marked text during composition so the user sees it;
/// `unmarkText` commits through Rust at the Rust-authorized range.
///
/// Every UITextView method runs on the main thread, and the UniFFI calls are
/// synchronous.
final class EditorTextView: UITextView, UIGestureRecognizerDelegate, UITextDragDelegate, UITextDropDelegate {
    static let emptyBlockPlaceholderScalar = UnicodeScalar(0x200B)!

    lazy var internalTextViewDelegate = EditorTextViewInternalDelegate(editor: self)
    var imageLoadOwner: RenderImageLoadOwner?

    override var undoManager: UndoManager? { nil }

    // MARK: - Properties

    /// The Rust editor instance ID (from editor_create / editor_create_with_max_length).
    /// Set to 0 when no editor is bound.
    var editorId: UInt64 = 0
    let nativeBindingToken = UUID()

    /// Guard flag to prevent re-entrant input interception while we're
    /// applying state from Rust (calling replaceCharacters on the text storage).
    var isApplyingRustState = false
    var localTextDragState = LocalTextDragState.idle
    var visibleSelectionTintColor: UIColor = .systemBlue
    var hidesNativeSelectionChrome = false
    var isPreviewingImageResize = false
    var allowImageResizing = true

    override var isEditable: Bool {
        didSet {
            if oldValue, !isEditable {
                _ = finishExternalTextComposition(
                    cause: "lifecycle",
                    finalText: nil,
                    cancel: true
                )
            }
        }
    }

    /// The base font used for unstyled text. Configurable from React props.
    var baseFont: UIFont = .systemFont(ofSize: 16) {
        didSet {
            placeholderLabel.font = resolvedDefaultFont()
            renderAppearanceRevision &+= 1
            invalidateAutoGrowHeightMeasurement()
        }
    }

    /// The base text color. Configurable from React props.
    var baseTextColor: UIColor = .label {
        didSet {
            renderAppearanceRevision &+= 1
        }
    }

    /// The base background color before theme overrides.
    var baseBackgroundColor: UIColor = .systemBackground
    var baseTextContainerInset: UIEdgeInsets = .zero
    var baseLineFragmentPadding: CGFloat = 0

    /// Optional render theme supplied by React.
    var theme: EditorTheme? {
        didSet {
            renderAppearanceRevision &+= 1
            placeholderLabel.font = resolvedDefaultFont()
            placeholderLabel.textColor = theme?.placeholderColor ?? .placeholderText
            backgroundColor = theme?.backgroundColor ?? baseBackgroundColor
            if let contentInsets = theme?.contentInsets {
                textContainerInset = UIEdgeInsets(
                    top: contentInsets.top ?? 0,
                    left: contentInsets.left ?? 0,
                    bottom: contentInsets.bottom ?? 0,
                    right: contentInsets.right ?? 0
                )
                textContainer.lineFragmentPadding = 0
            } else {
                textContainerInset = baseTextContainerInset
                textContainer.lineFragmentPadding = baseLineFragmentPadding
            }
            invalidateAutoGrowHeightMeasurement()
            setNeedsLayout()
        }
    }

    var atomRenderConfiguration: AtomRenderConfiguration? {
        didSet {
            guard oldValue != atomRenderConfiguration else { return }
            renderAppearanceRevision &+= 1
            invalidateAutoGrowHeightMeasurement()
        }
    }

    var heightBehavior: EditorHeightBehavior = .fixed {
        didSet {
            guard oldValue != heightBehavior else { return }
            isScrollEnabled = heightBehavior == .fixed
            invalidateAutoGrowHeightMeasurement()
            invalidateIntrinsicContentSize()
            notifyHeightChangeIfNeeded(force: true)
        }
    }

    var onHeightMayChange: ((CGFloat) -> Void)?
    var onViewportMayChange: (() -> Void)?
    var onSelectionOrContentMayChange: (() -> Void)?
    var onExternalUpdateReadinessMayChange: (() -> Void)?
    var lastAutoGrowMeasuredHeight: CGFloat = 0
    var lastAutoGrowMeasuredWidth: CGFloat = 0
    var autoGrowHostHeight: CGFloat = 0
    var autoGrowHeightCheckIsDirty = true
    var lastHeightNotifyMeasureNanosForTesting: UInt64 = 0
    var lastHeightNotifyCallbackNanosForTesting: UInt64 = 0
    var lastHeightNotifyEnsureLayoutNanosForTesting: UInt64 = 0
    var lastHeightNotifyUsedRectNanosForTesting: UInt64 = 0
    var lastHeightNotifyContentSizeNanosForTesting: UInt64 = 0
    var lastHeightNotifySizeThatFitsNanosForTesting: UInt64 = 0

    /// Delegate for editor events.
    weak var editorDelegate: EditorTextViewDelegate?

    /// The plain text from the last Rust render, used by the reconciliation
    /// fallback to detect unauthorized text storage mutations.
    var lastAuthorizedTextStorage = NSMutableString()
    var lastAuthorizedAttributedTextStorage = NSMutableAttributedString()
    var lastAuthorizedText: String {
        lastAuthorizedTextStorage as String
    }
    var lastRenderAppliedPatchForTesting: Bool = false
    var onApplyingRustTextForTesting: (() -> Void)?
    var captureApplyUpdateTraceForTesting = false
    var lastApplyUpdateTraceForTesting: ApplyUpdateTrace?
    var currentRenderBlocks: [[[String: Any]]]? = nil
    var currentRenderBlocksDocumentVersion: UInt64?
    var recoveringRenderPatchBaseMismatch = false
    var currentTopLevelChildMetadata: [TopLevelChildMetadata]? = nil
    var renderAppearanceRevision: UInt64 = 1
    var lastAppliedRenderAppearanceRevision: UInt64 = 0

    /// Number of times the reconciliation fallback has fired. Exposed for
    /// monitoring / kill-condition telemetry.
    var reconciliationCount: Int = 0

    /// Logger for reconciliation events (visible in Console.app / device logs).
    static let reconciliationLog = Logger(
        subsystem: "com.apollohg.prose-editor",
        category: "reconciliation"
    )
    static let inputLog = Logger(
        subsystem: "com.apollohg.prose-editor",
        category: "input"
    )
    static let updateLog = Logger(
        subsystem: "com.apollohg.prose-editor",
        category: "update"
    )
    static let selectionLog = Logger(
        subsystem: "com.apollohg.prose-editor",
        category: "selection"
    )

    /// Tracks whether we're in a composition session (CJK / IME input).
    var isComposing = false
    var hasPendingCompositionForExternalRefresh: Bool { isComposing }
    lazy var imageSelectionTapRecognizer: UITapGestureRecognizer = {
        let recognizer = UITapGestureRecognizer(target: self, action: #selector(handleImageSelectionTap(_:)))
        recognizer.cancelsTouchesInView = true
        recognizer.delaysTouchesBegan = false
        recognizer.delaysTouchesEnded = false
        recognizer.delegate = self
        return recognizer
    }()

    /// Guards against reconciliation firing while we're already intercepting
    /// and replaying a user input operation through Rust, including the
    /// trailing UIKit text-storage callbacks that arrive on the next run loop.
    var interceptedInputDepth = 0
    var deferredInsertTexts: [String] = []
    var deferredInsertDrainScheduled = false
    var isReplayingDeferredInsertText = false
    var reconciliationWorkScheduled = false
    var nativeTextMutationCommitScheduled = false
    var pendingNativeTextMutation: NativeTextMutation?
    var nativeTextMutationGeneration: UInt64 = 0
    var nativeTextMutationAfterBlurDeadline: TimeInterval?
    var nativeTextMutationAfterBlurGeneration: UInt64?
    private let nativeTextMutationAfterBlurGraceInterval: TimeInterval = 1.0
    /// Last selection known to match `lastAuthorizedText`, stored in that text's UTF-16 coordinates.
    var lastAuthorizedSelectedUtf16Range: NSRange?
    var lastAuthorizedSelectionIsBackward = false
    var logicalSelectionScalarRange: (anchor: UInt32, head: UInt32)?
    /// The UIKit selection `logicalSelectionScalarRange` was resolved to, after
    /// any empty-block autocapitalization nudge. Lets a deliberate nudge be
    /// told apart from the user moving the caret.
    var logicalSelectionUtf16Range: NSRange?
    var selectionRevision: UInt64 = 0
    var desiredInputTraitState = InputTraitState()
    var appliedInputTraitState = InputTraitState()
    var pendingInputTraitChange = PendingInputTraitChange()
    var pendingInputTraitRetryScheduled = false
    var pendingInputTraitRetryGeneration: UInt64 = 0

    /// Coalesces selection sync until UIKit has finished resolving the
    /// current tap/drag gesture's final caret position.
    var pendingSelectionSyncGeneration: UInt64 = 0
    var pendingDeferredImageSelectionRange: NSRange?
    var pendingDeferredImageSelectionGeneration: UInt64 = 0

    /// Stores the Rust-authorized scalar range replaced by the active marked
    /// text session. UIKit mutates visible TextKit state during composition,
    /// so final commits must not infer their range from the transient cursor.
    var markedTextReplacementScalarRange: (from: UInt32, to: UInt32)?
    var markedTextReplacementUtf16Range: NSRange?
    var markedTextCompositionText: String?
    var markedTextCompositionIsExplicitlyEmpty = false

    var externalTextComposition: ExternalTextCompositionState?
    var externalTextCompositionTerminalResults: [String: String] = [:]

    let editorLayoutManager: EditorLayoutManager

    // MARK: - Placeholder

    lazy var placeholderLabel: UILabel = {
        let label = UILabel()
        label.textColor = .placeholderText
        label.font = baseFont
        label.numberOfLines = 0
        label.isUserInteractionEnabled = false
        return label
    }()

    var placeholder: String = "" {
        didSet {
            placeholderLabel.text = placeholder
            refreshPlaceholderVisibility()
            setNeedsLayout()
        }
    }

    // MARK: - Initialization

    override init(frame: CGRect, textContainer: NSTextContainer?) {
        let layoutManager = EditorLayoutManager()
        let container = textContainer ?? NSTextContainer(
            size: CGSize(width: 0, height: CGFloat.greatestFiniteMagnitude)
        )
        let textStorage = NSTextStorage()
        layoutManager.addTextContainer(container)
        textStorage.addLayoutManager(layoutManager)
        editorLayoutManager = layoutManager
        super.init(frame: frame, textContainer: container)
        commonInit()
    }

    required init?(coder: NSCoder) {
        return nil
    }

    private func commonInit() {
        textContainer.widthTracksTextView = true
        // Large documents edit more smoothly when TextKit can invalidate and
        // relayout only the touched region instead of forcing contiguous layout.
        editorLayoutManager.allowsNonContiguousLayout = true
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(handleImageAttachmentDidLoad(_:)),
            name: .editorImageAttachmentDidLoad,
            object: nil
        )

        // Configure the text view as a Rust-controlled editor surface.
        // UIKit smart-edit features mutate text storage outside our transaction
        // pipeline and can race with stored-mark typing after toolbar actions.
        setAutoCorrect(nil)
        setAutoCapitalize(nil)
        setKeyboardType(nil)
        smartQuotesType = .no
        smartDashesType = .no
        smartInsertDeleteType = .no

        // Allow scrolling and text selection.
        isScrollEnabled = heightBehavior == .fixed
        isEditable = true
        isSelectable = true

        // Set a reasonable default font.
        font = baseFont
        textColor = baseTextColor
        backgroundColor = baseBackgroundColor
        baseTextContainerInset = textContainerInset
        baseLineFragmentPadding = textContainer.lineFragmentPadding
        visibleSelectionTintColor = tintColor

        // Register as the text storage delegate so we can detect unauthorized
        // mutations (reconciliation fallback).
        textStorage.delegate = self
        ensureInternalTextViewDelegate()
        textDragDelegate = self
        textDropDelegate = self
        addGestureRecognizer(imageSelectionTapRecognizer)
        installImageSelectionTapDependencies()

        addSubview(placeholderLabel)
        refreshPlaceholderVisibility()
        refreshNativeSelectionChromeVisibility()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        installImageSelectionTapDependencies()
    }

    override func didAddSubview(_ subview: UIView) {
        super.didAddSubview(subview)
        installImageSelectionTapDependencies()
    }

    override func tintColorDidChange() {
        super.tintColorDidChange()
        if !hidesNativeSelectionChrome, tintColor.cgColor.alpha > 0 {
            visibleSelectionTintColor = tintColor
        }
    }

    // MARK: - Layout

    override func layoutSubviews() {
        super.layoutSubviews()
        let placeholderX = textContainerInset.left + textContainer.lineFragmentPadding
        let placeholderY = textContainerInset.top
        let placeholderWidth = max(
            0,
            bounds.width - textContainerInset.left - textContainerInset.right - 2 * textContainer.lineFragmentPadding
        )
        if placeholderLabel.isHidden {
            placeholderLabel.frame = CGRect(
                x: placeholderX,
                y: placeholderY,
                width: placeholderWidth,
                height: 0
            )
        } else {
            let maxPlaceholderHeight = max(
                0,
                bounds.height - textContainerInset.top - textContainerInset.bottom
            )
            let fittedHeight = placeholderLabel.sizeThatFits(
                CGSize(width: placeholderWidth, height: CGFloat.greatestFiniteMagnitude)
            ).height
            placeholderLabel.frame = CGRect(
                x: placeholderX,
                y: placeholderY,
                width: placeholderWidth,
                height: min(maxPlaceholderHeight, ceil(fittedHeight))
            )
        }
        if heightBehavior == .autoGrow, !isPreviewingImageResize {
            let currentWidth = ceil(bounds.width)
            if abs(currentWidth - lastAutoGrowMeasuredWidth) > 0.5 {
                autoGrowHeightCheckIsDirty = true
                lastAutoGrowMeasuredWidth = currentWidth
            }
            if autoGrowHeightCheckIsDirty {
                notifyHeightChangeIfNeeded()
            }
        }
        if !isPreviewingImageResize {
            onViewportMayChange?()
        }
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    override var contentOffset: CGPoint {
        didSet {
            if !isPreviewingImageResize {
                onViewportMayChange?()
            }
        }
    }

    override func becomeFirstResponder() -> Bool {
        let didBecomeFirstResponder = super.becomeFirstResponder()
        if didBecomeFirstResponder {
            ensureInternalTextViewDelegate()
            clearNativeTextMutationAfterBlurWindow()
            DispatchQueue.main.async { [weak self] in
                self?.ensureInternalTextViewDelegate()
            }
            _ = normalizeSelectionForEmptyBlockAutocapitalizationIfNeeded()
            recordAuthorizedSelectionIfPossible()
            refreshTypingAttributesForSelection()
        }
        return didBecomeFirstResponder
    }

    override func resignFirstResponder() -> Bool {
        ensureInternalTextViewDelegate()
        _ = drainPendingNativeTextMutation(allowAfterBlur: false, allowWhileIntercepting: true)

        let wasFirstResponder = isFirstResponder
        if wasFirstResponder {
            nativeTextMutationAfterBlurGeneration = nativeTextMutationGeneration
            nativeTextMutationAfterBlurDeadline = ProcessInfo.processInfo.systemUptime
                + nativeTextMutationAfterBlurGraceInterval
        }

        let didResignFirstResponder = super.resignFirstResponder()
        if wasFirstResponder || didResignFirstResponder {
            _ = drainPendingNativeTextMutation(allowAfterBlur: true, allowWhileIntercepting: true)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                _ = self.drainPendingNativeTextMutation(
                    allowAfterBlur: true,
                    allowWhileIntercepting: true
                )
            }
        }
        return didResignFirstResponder
    }

    /// The core's `documentIsEmpty` from the most recent editor update, or nil
    /// when the current render arrived without one.
    var coreReportedDocumentIsEmpty: Bool?

    override func caretRect(for position: UITextPosition) -> CGRect {
        if hidesNativeSelectionChrome {
            return .zero
        }
        let utf16Offset = offset(from: beginningOfDocument, to: position)
        if isAtomBoundaryCaretOffset(utf16Offset) {
            return .zero
        }
        let rect = resolvedCaretReferenceRect(for: position)
        guard rect.height > 0 else { return rect }

        let caretFont = resolvedCaretFont(for: position)
        let screenScale = window?.screen.scale ?? UIScreen.main.scale
        let targetHeight = ceil(caretFont.lineHeight)
        guard targetHeight > 0, targetHeight < rect.height else { return rect }

        if let baselineY = caretBaselineY(for: position, referenceRect: rect) {
            return Self.adjustedCaretRect(
                from: rect,
                baselineY: baselineY,
                font: caretFont,
                screenScale: screenScale
            )
        }

        return Self.adjustedCaretRect(
            from: rect,
            font: caretFont,
            screenScale: screenScale
        )
    }

    override func closestPosition(to point: CGPoint) -> UITextPosition? {
        if atomAttachmentRange(at: point) != nil,
           let selectedTextRange,
           selectedTextRange.isEmpty
        {
            return selectedTextRange.start
        }
        return super.closestPosition(to: point)
    }

    override func closestPosition(
        to point: CGPoint,
        within range: UITextRange
    ) -> UITextPosition? {
        if atomAttachmentRange(at: point) != nil,
           let selectedTextRange,
           selectedTextRange.isEmpty
        {
            return selectedTextRange.start
        }
        return super.closestPosition(to: point, within: range)
    }

    // MARK: - Input Interception: Text Insertion

    /// Intercept text insertion. This is called for:
    /// - Single character typing (including autocomplete insertions)
    /// - Return/Enter key
    /// - Dictation results
    ///
    /// Instead of calling `super.insertText()` (which would modify the
    /// underlying text storage directly), we route through Rust.
    override func insertText(_ text: String) {
        ensureInternalTextViewDelegate()
        if isApplyingRustState
            || (!isReplayingDeferredInsertText && !deferredInsertTexts.isEmpty)
        {
            enqueueDeferredInsertText(text)
            return
        }
        guard editorId != 0 else {
            super.insertText(text)
            return
        }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
        if !isReplayingDeferredInsertText, !deferredInsertTexts.isEmpty {
            enqueueDeferredInsertText(text)
            return
        }
        guard !isCollapsedAtomBoundary(selectedUtf16Range()) else { return }

        if interceptReturnInput(text) { return }

        if markedTextReplacementScalarRange != nil || markedTextRange != nil {
            let replacementRange = trackedMarkedTextReplacementRange()
            finishTransientMarkedTextMutation()
            _ = commitMarkedText(text, replacementRange: replacementRange)
            return
        }

        // Get the current cursor position as a scalar offset.
        let scalarPos = PositionBridge.cursorScalarOffset(in: self)
        Self.inputLog.debug(
            "[insertText] text=\(self.preview(text), privacy: .public) scalarPos=\(scalarPos) selection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )

        // If there's a range selection, atomically replace it.
        if let selectedRange = selectedTextRange, !selectedRange.isEmpty {
            let range = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
            performInterceptedInput {
                let updateJSON = EditorV2Shadow.replaceTextScalar(
                    id: editorId,
                    scalarFrom: range.from,
                    scalarTo: range.to,
                    text: text
                )
                applyUpdateJSON(updateJSON)
            }
        } else {
            performInterceptedInput {
                insertTextInRust(text, at: scalarPos)
            }
        }
    }

    override var keyCommands: [UIKeyCommand]? {
        [
            UIKeyCommand(
                input: "\r",
                modifierFlags: [.shift],
                action: #selector(handleHardBreakKeyCommand)
            ),
            UIKeyCommand(
                input: "\t",
                modifierFlags: [],
                action: #selector(handleIndentKeyCommand)
            ),
            UIKeyCommand(
                input: "\t",
                modifierFlags: [.shift],
                action: #selector(handleOutdentKeyCommand)
            ),
        ]
    }

    // MARK: - Input Interception: Deletion

    /// Intercept backward deletion (Backspace key).
    ///
    /// If there's a range selection, delete the range. If it's a cursor,
    /// delete the character (grapheme cluster) before the cursor.
    override func deleteBackward() {
        ensureInternalTextViewDelegate()
        guard !isApplyingRustState else {
            super.deleteBackward()
            return
        }
        guard editorId != 0 else {
            super.deleteBackward()
            return
        }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
        guard !isCollapsedAtomBoundary(selectedUtf16Range()) else { return }

        if markedTextReplacementScalarRange != nil || markedTextRange != nil {
            performTransientTextMutation {
                super.deleteBackward()
            }
            refreshMarkedTextCompositionText()
            isComposing = markedTextRange != nil || markedTextReplacementScalarRange != nil
            return
        }

        guard let selectedRange = selectedTextRange else { return }
        Self.inputLog.debug(
            "[deleteBackward] selection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )

        if !selectedRange.isEmpty {
            // Range selection: delete the entire range.
            let range = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
            performInterceptedInput {
                deleteScalarRangeInRust(from: range.from, to: range.to)
            }
        } else {
            // Cursor: delete one grapheme cluster backward. The engine's caret
            // is the authority — in an empty block UIKit's own caret is parked
            // ahead of the block placeholder for autocapitalization and does
            // not address the same position.
            let cursorPos = currentLogicalScalarSelection()?.head
                ?? PositionBridge.cursorScalarOffset(in: self)
            if cursorPos == 0 {
                performInterceptedInput {
                    deleteBackwardAtSelectionScalarInRust(anchor: cursorPos, head: cursorPos)
                }
                return
            }

            let cursorUtf16Offset = offset(from: beginningOfDocument, to: selectedRange.start)
            if cursorUtf16Offset <= 0 {
                performInterceptedInput {
                    deleteBackwardAtSelectionScalarInRust(anchor: cursorPos, head: cursorPos)
                }
                return
            }
            if let marker = PositionBridge.virtualListMarker(
                atUtf16Offset: cursorUtf16Offset,
                in: self
            ), marker.paragraphStartUtf16 == cursorUtf16Offset {
                performInterceptedInput {
                    deleteScalarRangeInRust(from: cursorPos - 1, to: cursorPos)
                }
                return
            }

            if let deleteRange = trailingVoidBlockDeleteRangeForBackwardDelete(
                cursorUtf16Offset: cursorUtf16Offset
            ) {
                performInterceptedInput {
                    deleteScalarRangeInRust(from: deleteRange.from, to: deleteRange.to)
                }
                return
            }

            if let deleteRange = adjacentVoidBlockDeleteRangeForBackwardDelete(
                cursorUtf16Offset: cursorUtf16Offset,
                cursorScalar: cursorPos
            ) {
                performInterceptedInput {
                    deleteScalarRangeInRust(from: deleteRange.from, to: deleteRange.to)
                }
                return
            }

            if cursorUtf16Offset > 0,
               (textStorage.string as NSString).character(at: cursorUtf16Offset - 1) == 0x200B
            {
                performInterceptedInput {
                    deleteBackwardAtSelectionScalarInRust(anchor: cursorPos, head: cursorPos)
                }
                return
            }

            // Find the start of the previous grapheme cluster.
            // We need to figure out how many scalars the previous grapheme occupies.
            // Use UITextView's tokenizer to find the previous grapheme boundary.
            guard let prevPos = position(from: selectedRange.start, offset: -1) else { return }
            let prevScalar = PositionBridge.textViewToScalar(prevPos, in: self)

            performInterceptedInput {
                if prevScalar < cursorPos {
                    deleteScalarRangeInRust(from: prevScalar, to: cursorPos)
                } else {
                    deleteBackwardAtSelectionScalarInRust(anchor: cursorPos, head: cursorPos)
                }
            }
        }
    }

    // MARK: - Input Interception: Replace (Autocorrect)

    /// Intercept text replacement. This is called when:
    /// - Autocorrect replaces a word
    /// - User accepts a spelling suggestion
    /// - Programmatic text replacement
    ///
    /// We route the replacement through Rust to keep the document model in sync.
    override func replace(_ range: UITextRange, withText text: String) {
        ensureInternalTextViewDelegate()
        guard !isApplyingRustState else {
            super.replace(range, withText: text)
            return
        }
        guard editorId != 0 else {
            super.replace(range, withText: text)
            return
        }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
        let replacementUtf16Range = NSRange(
            location: offset(from: beginningOfDocument, to: range.start),
            length: offset(from: range.start, to: range.end)
        )
        guard !isCollapsedAtomBoundary(replacementUtf16Range) else { return }

        if interceptReturnInput(text, replacing: range) { return }

        if markedTextReplacementScalarRange != nil || markedTextRange != nil {
            let replacementRange = trackedMarkedTextReplacementRange()
            finishTransientMarkedTextMutation()
            _ = commitMarkedText(text, replacementRange: replacementRange)
            return
        }

        let scalarRange = PositionBridge.textRangeToScalarRange(range, in: self)
        let replacementStartUtf16 = replacementUtf16Range.location
        let replacementEndUtf16 = NSMaxRange(replacementUtf16Range)
        let preservesAcceptedSpace = textStorage.string == lastAuthorizedText
            && shouldPreserveAcceptedAutocorrectSpace(
                authorizedText: lastAuthorizedText as NSString,
                replacementStartUtf16: replacementStartUtf16,
                authorizedEndUtf16: replacementEndUtf16,
                replacementText: text,
                rawSelectionUtf16Range: selectedUtf16Range(),
                authorizedSelectionUtf16Range: lastAuthorizedSelectedUtf16Range,
                acceptedCaretUtf16Offset: replacementEndUtf16
            )
        let replacementText = preservesAcceptedSpace ? text + " " : text
        Self.inputLog.debug(
            "[replace] text=\(self.preview(replacementText), privacy: .public) scalarRange=\(scalarRange.from)-\(scalarRange.to) selection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )

        // Atomically replace the range with the new text via Rust.
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.replaceTextScalar(
                id: editorId,
                scalarFrom: scalarRange.from,
                scalarTo: scalarRange.to,
                text: replacementText
            )
            applyUpdateJSON(updateJSON)
        }
    }

    // MARK: - Composition Handling (CJK / IME)

    /// Called when the input method sets marked (composing) text.
    ///
    /// During CJK input, the user composes characters incrementally. We let
    /// UITextView display the composing text normally (with its underline
    /// decoration). The text is NOT sent to Rust during composition.
    override func setMarkedText(_ markedText: String?, selectedRange: NSRange) {
        ensureInternalTextViewDelegate()
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        if markedText != nil {
            guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
            captureMarkedTextReplacementRangeIfNeeded()
        } else if markedTextReplacementScalarRange == nil, markedTextRange == nil {
            guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
        }
        isComposing = markedText != nil || markedTextReplacementScalarRange != nil
        Self.inputLog.debug(
            "[setMarkedText] marked=\(self.preview(markedText ?? ""), privacy: .public) nsRange=\(selectedRange.location),\(selectedRange.length) selection=\(self.selectionSummary(), privacy: .public)"
        )
        if markedText == nil {
            // Some keyboard paths finalize composition by clearing marked text
            // instead of calling unmarkText().
            let composedText = validatedTrackedMarkedTextForCommit()
            let replacementRange = trackedMarkedTextReplacementRange()
            performTransientTextMutation {
                super.setMarkedText(nil, selectedRange: selectedRange)
            }
            clearMarkedTextTracking()
            if shouldCommitMarkedText(composedText, replacementRange: replacementRange) {
                _ = commitMarkedText(composedText ?? "", replacementRange: replacementRange)
            } else {
                restoreAuthorizedTextAfterCancelledCompositionIfNeeded()
            }
            return
        }

        performTransientTextMutation {
            super.setMarkedText(markedText, selectedRange: selectedRange)
        }
        refreshMarkedTextCompositionText(fallback: markedText)
    }

    /// Called when composition is finalized (user selects a candidate or
    /// presses space/enter to commit).
    ///
    /// At this point, the composed text is final. We capture it and commit it
    /// to Rust at the original replacement range captured before UIKit mutated
    /// the transient text storage.
    override func unmarkText() {
        ensureInternalTextViewDelegate()
        if externalTextComposition != nil {
            guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
            return
        }
        let composedText = currentMarkedTextForCommit()
        let replacementRange = trackedMarkedTextReplacementRange()

        finishTransientMarkedTextMutation()

        if let composed = composedText, !composed.isEmpty {
            Self.inputLog.debug(
                "[unmarkText] composed=\(self.preview(composed), privacy: .public) replacement=\(self.previewMarkedTextReplacementRange(replacementRange), privacy: .public) selection=\(self.selectionSummary(), privacy: .public)"
            )
            _ = commitMarkedText(composed, replacementRange: replacementRange)
        } else if shouldCommitMarkedText(composedText, replacementRange: replacementRange) {
            _ = commitMarkedText(composedText ?? "", replacementRange: replacementRange)
        } else {
            restoreAuthorizedTextAfterCancelledCompositionIfNeeded()
        }
    }

    // MARK: - Paste Handling

    /// Intercept paste operations to route content through Rust.
    ///
    /// Attempts to extract HTML from the pasteboard first (for rich text paste),
    /// falling back to plain text.
    override func paste(_ sender: Any?) {
        ensureInternalTextViewDelegate()
        guard editorId != 0 else {
            super.paste(sender)
            return
        }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        guard prepareForExternalEditorUpdate() else { return }

        Self.inputLog.debug(
            "[paste] selection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )

        let pasteboard = UIPasteboard.general

        // Try HTML first for rich paste.
        if let htmlData = pasteboard.data(forPasteboardType: "public.html"),
           let html = String(data: htmlData, encoding: .utf8) {
            performInterceptedInput {
                pasteHTML(html)
            }
            return
        }

        // Try attributed string (e.g. from Notes, Pages).
        if let rtfData = pasteboard.data(forPasteboardType: "public.rtf") {
            if let attrStr = try? NSAttributedString(
                data: rtfData,
                options: [.documentType: NSAttributedString.DocumentType.rtf],
                documentAttributes: nil
            ) {
                // Convert attributed string to HTML for Rust processing.
                if let htmlData = try? attrStr.data(
                    from: NSRange(location: 0, length: attrStr.length),
                    documentAttributes: [.documentType: NSAttributedString.DocumentType.html]
                ), let html = String(data: htmlData, encoding: .utf8) {
                    performInterceptedInput {
                        if !pasteHTML(html, detectContentChange: true),
                           !attrStr.string.isEmpty {
                            pastePlainText(attrStr.string)
                        }
                    }
                    return
                }
            }
        }

        // Fallback to plain text.
        if let text = pasteboard.string {
            performInterceptedInput {
                pastePlainText(text)
            }
            return
        }
    }

    // MARK: - Private: Rust Integration

    var isInterceptingInput: Bool {
        interceptedInputDepth > 0
    }

    // MARK: - Applying Rust State

    /// Whether the document is the lone empty block whose caret the
    /// autocapitalization nudge parks ahead of the placeholder. That block
    /// holds exactly one caret position, which is what makes a cached scalar
    /// safe to trust there even though the UIKit offset disagrees with it.
    var isLoneEmptyPlaceholderBlock: Bool {
        textStorage.length == 1
            && textStorage.string.unicodeScalars.elementsEqual([Self.emptyBlockPlaceholderScalar])
    }

}
