import UIKit
import os

// MARK: - EditorTextViewDelegate

/// Delegate protocol for EditorTextView to communicate state changes
/// back to the hosting view (Fabric component or UIKit container).
protocol EditorTextViewDelegate: AnyObject {
    /// Called when the editor's selection changes.
    /// - Parameters:
    ///   - textView: The editor text view.
    ///   - anchor: Scalar offset of the selection anchor.
    ///   - head: Scalar offset of the selection head.
    func editorTextView(_ textView: EditorTextView, selectionDidChange anchor: UInt32, head: UInt32)

    /// Called when the editor content is updated after a Rust operation.
    /// - Parameters:
    ///   - textView: The editor text view.
    ///   - updateJSON: The full EditorUpdate JSON string from Rust.
    func editorTextView(_ textView: EditorTextView, didReceiveUpdate updateJSON: String)
}

enum EditorHeightBehavior: String {
    case fixed
    case autoGrow
}

struct RemoteSelectionDecoration {
    let clientId: Int
    let anchor: UInt32
    let head: UInt32
    let color: UIColor
    let name: String?
    let isFocused: Bool

    static func from(json: String?) -> [RemoteSelectionDecoration] {
        guard let json,
              let data = json.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]]
        else {
            return []
        }

        return raw.compactMap { item in
            guard let clientId = item["clientId"] as? NSNumber,
                  let anchor = item["anchor"] as? NSNumber,
                  let head = item["head"] as? NSNumber,
                  let colorRaw = item["color"] as? String,
                  let color = colorFromString(colorRaw)
            else {
                return nil
            }

            return RemoteSelectionDecoration(
                clientId: clientId.intValue,
                anchor: anchor.uint32Value,
                head: head.uint32Value,
                color: color,
                name: item["name"] as? String,
                isFocused: (item["isFocused"] as? Bool) ?? false
            )
        }
    }

    private static func colorFromString(_ raw: String) -> UIColor? {
        let value = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard value.hasPrefix("#") else { return nil }
        let hex = String(value.dropFirst())

        switch hex.count {
        case 3:
            let chars = Array(hex)
            return UIColor(
                red: component(String(repeating: String(chars[0]), count: 2)),
                green: component(String(repeating: String(chars[1]), count: 2)),
                blue: component(String(repeating: String(chars[2]), count: 2)),
                alpha: 1
            )
        case 4:
            let chars = Array(hex)
            return UIColor(
                red: component(String(repeating: String(chars[0]), count: 2)),
                green: component(String(repeating: String(chars[1]), count: 2)),
                blue: component(String(repeating: String(chars[2]), count: 2)),
                alpha: component(String(repeating: String(chars[3]), count: 2))
            )
        case 6:
            return UIColor(
                red: component(String(hex.prefix(2))),
                green: component(String(hex.dropFirst(2).prefix(2))),
                blue: component(String(hex.dropFirst(4).prefix(2))),
                alpha: 1
            )
        case 8:
            return UIColor(
                red: component(String(hex.prefix(2))),
                green: component(String(hex.dropFirst(2).prefix(2))),
                blue: component(String(hex.dropFirst(4).prefix(2))),
                alpha: component(String(hex.dropFirst(6).prefix(2)))
            )
        default:
            return nil
        }
    }

    private static func component(_ hex: String) -> CGFloat {
        CGFloat(Int(hex, radix: 16) ?? 0) / 255
    }
}

private final class RemoteSelectionBadgeLabel: UILabel {
    override func drawText(in rect: CGRect) {
        super.drawText(in: rect.inset(by: UIEdgeInsets(top: 0, left: 8, bottom: 0, right: 8)))
    }

    override var intrinsicContentSize: CGSize {
        let size = super.intrinsicContentSize
        return CGSize(width: size.width + 16, height: max(size.height + 8, 22))
    }
}

private final class RemoteSelectionOverlayView: UIView {
    weak var textView: EditorTextView?
    private var editorId: UInt64 = 0
    private var selections: [RemoteSelectionDecoration] = []

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        isUserInteractionEnabled = false
        clipsToBounds = true
    }

    required init?(coder: NSCoder) {
        return nil
    }

    func bind(textView: EditorTextView) {
        self.textView = textView
    }

    func update(selections: [RemoteSelectionDecoration], editorId: UInt64) {
        self.selections = selections
        self.editorId = editorId
        refresh()
    }

    func refresh() {
        subviews.forEach { $0.removeFromSuperview() }
        guard editorId != 0,
              let textView
        else {
            return
        }

        for selection in selections {
            let geometry = geometry(for: selection, in: textView)
            for rect in geometry.selectionRects {
                let selectionView = UIView(frame: rect.integral)
                selectionView.backgroundColor = selection.color.withAlphaComponent(0.18)
                selectionView.layer.cornerRadius = 3
                addSubview(selectionView)
            }

            guard selection.isFocused,
                  let caretRect = geometry.caretRect
            else {
                continue
            }

            let caretView = UIView(frame: CGRect(
                x: round(caretRect.minX),
                y: round(caretRect.minY),
                width: max(2, round(caretRect.width)),
                height: round(caretRect.height)
            ))
            caretView.backgroundColor = selection.color
            caretView.layer.cornerRadius = caretView.bounds.width / 2
            addSubview(caretView)
        }
    }

    private func geometry(
        for selection: RemoteSelectionDecoration,
        in textView: EditorTextView
    ) -> (selectionRects: [CGRect], caretRect: CGRect?) {
        let startScalar = editorDocToScalar(
            id: editorId,
            docPos: min(selection.anchor, selection.head)
        )
        let endScalar = editorDocToScalar(
            id: editorId,
            docPos: max(selection.anchor, selection.head)
        )

        let startPosition = PositionBridge.scalarToTextView(startScalar, in: textView)
        let endPosition = PositionBridge.scalarToTextView(endScalar, in: textView)
        let caretRect = resolvedCaretRect(
            for: endPosition,
            in: textView
        )

        if startScalar == endScalar {
            return ([], caretRect)
        }

        guard let range = textView.textRange(from: startPosition, to: endPosition) else {
            return ([], caretRect)
        }

        let selectionRects = textView.selectionRects(for: range)
            .map(\.rect)
            .filter { !$0.isEmpty && $0.width > 0 && $0.height > 0 }
            .map { textView.convert($0, to: self) }

        return (selectionRects, caretRect)
    }

    private func resolvedCaretRect(
        for position: UITextPosition,
        in textView: EditorTextView
    ) -> CGRect? {
        let directRect = textView.convert(textView.caretRect(for: position), to: self)
        if directRect.height > 0, directRect.width >= 0 {
            return directRect
        }

        if let previousPosition = textView.position(from: position, offset: -1),
           let previousRange = textView.textRange(from: previousPosition, to: position),
           let previousRect = textView.selectionRects(for: previousRange)
               .map(\.rect)
               .last(where: { !$0.isEmpty && $0.height > 0 })
        {
            let rect = textView.convert(previousRect, to: self)
            return CGRect(x: rect.maxX, y: rect.minY, width: 2, height: rect.height)
        }

        if let nextPosition = textView.position(from: position, offset: 1),
           let nextRange = textView.textRange(from: position, to: nextPosition),
           let nextRect = textView.selectionRects(for: nextRange)
               .map(\.rect)
               .first(where: { !$0.isEmpty && $0.height > 0 })
        {
            let rect = textView.convert(nextRect, to: self)
            return CGRect(x: rect.minX, y: rect.minY, width: 2, height: rect.height)
        }

        if directRect.isEmpty {
            return nil
        }

        return directRect
    }
}

// MARK: - EditorTextView

/// UITextView subclass that intercepts all text input and routes it through
/// the Rust editor-core engine via UniFFI bindings.
///
/// Instead of letting UITextView's internal text storage handle insertions
/// and deletions, this class captures the user's intent (typing, deleting,
/// pasting, autocorrect) and sends it to the Rust editor. The Rust editor
/// returns render elements, which are converted to NSAttributedString via
/// RenderBridge and applied back to the text view.
///
/// This is the "input interception" pattern: the UITextView is effectively
/// a rendering surface, not a text editing engine.
///
/// ## Composition (IME) Handling
///
/// For CJK input methods, `setMarkedText` / `unmarkText` are used. During
/// composition (marked text), we let UITextView handle it normally so the
/// user sees their composing text. When composition finalizes (`unmarkText`),
/// we capture the result and route it through Rust.
///
/// ## Thread Safety
///
/// All UITextView methods are called on the main thread. The UniFFI calls
/// (`editor_insert_text`, `editor_delete_range`, etc.) are synchronous and
/// fast enough for main-thread use. If profiling shows otherwise, we can
/// dispatch to a serial queue and batch updates.
final class EditorTextView: UITextView, UITextViewDelegate {
    private static let emptyBlockPlaceholderScalar = UnicodeScalar(0x200B)

    // MARK: - Properties

    /// The Rust editor instance ID (from editor_create / editor_create_with_max_length).
    /// Set to 0 when no editor is bound.
    var editorId: UInt64 = 0

    /// Guard flag to prevent re-entrant input interception while we're
    /// applying state from Rust (calling replaceCharacters on the text storage).
    var isApplyingRustState = false

    /// The base font used for unstyled text. Configurable from React props.
    var baseFont: UIFont = .systemFont(ofSize: 16)

    /// The base text color. Configurable from React props.
    var baseTextColor: UIColor = .label

    /// The base background color before theme overrides.
    var baseBackgroundColor: UIColor = .systemBackground
    var baseTextContainerInset: UIEdgeInsets = .zero

    /// Optional render theme supplied by React.
    var theme: EditorTheme? {
        didSet {
            placeholderLabel.font = resolvedDefaultFont()
            backgroundColor = theme?.backgroundColor ?? baseBackgroundColor
            if let contentInsets = theme?.contentInsets {
                textContainerInset = UIEdgeInsets(
                    top: contentInsets.top ?? 0,
                    left: contentInsets.left ?? 0,
                    bottom: contentInsets.bottom ?? 0,
                    right: contentInsets.right ?? 0
                )
            } else {
                textContainerInset = baseTextContainerInset
            }
            setNeedsLayout()
        }
    }

    var heightBehavior: EditorHeightBehavior = .fixed {
        didSet {
            guard oldValue != heightBehavior else { return }
            isScrollEnabled = heightBehavior == .fixed
            invalidateIntrinsicContentSize()
            notifyHeightChangeIfNeeded(force: true)
        }
    }

    var onHeightMayChange: (() -> Void)?
    var onViewportMayChange: (() -> Void)?
    private var lastAutoGrowMeasuredHeight: CGFloat = 0

    /// Delegate for editor events.
    weak var editorDelegate: EditorTextViewDelegate?

    /// The plain text from the last Rust render, used by the reconciliation
    /// fallback to detect unauthorized text storage mutations.
    private(set) var lastAuthorizedText: String = ""

    /// Number of times the reconciliation fallback has fired. Exposed for
    /// monitoring / kill-condition telemetry.
    private(set) var reconciliationCount: Int = 0

    /// Logger for reconciliation events (visible in Console.app / device logs).
    private static let reconciliationLog = Logger(
        subsystem: "com.apollohg.prose-editor",
        category: "reconciliation"
    )
    private static let inputLog = Logger(
        subsystem: "com.apollohg.prose-editor",
        category: "input"
    )
    private static let updateLog = Logger(
        subsystem: "com.apollohg.prose-editor",
        category: "update"
    )
    private static let selectionLog = Logger(
        subsystem: "com.apollohg.prose-editor",
        category: "selection"
    )

    /// Tracks whether we're in a composition session (CJK / IME input).
    private var isComposing = false

    /// Guards against reconciliation firing while we're already intercepting
    /// and replaying a user input operation through Rust, including the
    /// trailing UIKit text-storage callbacks that arrive on the next run loop.
    private var interceptedInputDepth = 0
    private var reconciliationWorkScheduled = false

    /// Coalesces selection sync until UIKit has finished resolving the
    /// current tap/drag gesture's final caret position.
    private var pendingSelectionSyncGeneration: UInt64 = 0

    /// Stores the text that was composed during a marked text session,
    /// captured when `unmarkText` is called.
    private var composedText: String?

    private let editorLayoutManager: EditorLayoutManager

    // MARK: - Placeholder

    private lazy var placeholderLabel: UILabel = {
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
        let container = textContainer ?? NSTextContainer(size: .zero)
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
        editorLayoutManager.allowsNonContiguousLayout = false

        // Configure the text view as a Rust-controlled editor surface.
        // UIKit smart-edit features mutate text storage outside our transaction
        // pipeline and can race with stored-mark typing after toolbar actions.
        autocorrectionType = .no
        autocapitalizationType = .sentences
        spellCheckingType = .no
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

        // Register as the text storage delegate so we can detect unauthorized
        // mutations (reconciliation fallback).
        textStorage.delegate = self
        delegate = self

        addSubview(placeholderLabel)
        refreshPlaceholderVisibility()
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
        if heightBehavior == .autoGrow {
            notifyHeightChangeIfNeeded()
        }
        onViewportMayChange?()
    }

    override var contentOffset: CGPoint {
        didSet {
            onViewportMayChange?()
        }
    }

    private func isRenderedContentEmpty() -> Bool {
        let renderedText = textStorage.string
        guard !renderedText.isEmpty else { return true }

        for scalar in renderedText.unicodeScalars {
            switch scalar {
            case Self.emptyBlockPlaceholderScalar, "\n", "\r":
                continue
            default:
                return false
            }
        }

        return true
    }

    private func refreshPlaceholderVisibility() {
        placeholderLabel.isHidden = placeholder.isEmpty || !isRenderedContentEmpty()
    }

    func isPlaceholderVisibleForTesting() -> Bool {
        !placeholderLabel.isHidden
    }

    func placeholderFrameForTesting() -> CGRect {
        placeholderLabel.frame
    }

    func blockquoteStripeRectsForTesting() -> [CGRect] {
        editorLayoutManager.blockquoteStripeRectsForTesting(in: textStorage)
    }

    func resetBlockquoteStripeDrawPassesForTesting() {
        editorLayoutManager.resetBlockquoteStripeDrawPassesForTesting()
    }

    func blockquoteStripeDrawPassesForTesting() -> [[CGRect]] {
        editorLayoutManager.blockquoteStripeDrawPassesForTesting
    }

    override func caretRect(for position: UITextPosition) -> CGRect {
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

    private func resolvedCaretReferenceRect(for position: UITextPosition) -> CGRect {
        let directRect = super.caretRect(for: position)
        guard directRect.height <= 0 || directRect.isEmpty else {
            return directRect
        }

        let caretWidth = max(directRect.width, 2)

        if let nextPosition = self.position(from: position, offset: 1),
           let nextRange = textRange(from: position, to: nextPosition),
           let nextRect = selectionRects(for: nextRange)
               .map(\.rect)
               .first(where: { !$0.isEmpty && $0.width > 0 && $0.height > 0 })
        {
            return CGRect(
                x: nextRect.minX,
                y: nextRect.minY,
                width: caretWidth,
                height: max(directRect.height, nextRect.height)
            )
        }

        if let previousPosition = self.position(from: position, offset: -1),
           let previousRange = textRange(from: previousPosition, to: position),
           let previousRect = selectionRects(for: previousRange)
               .map(\.rect)
               .last(where: { !$0.isEmpty && $0.width > 0 && $0.height > 0 })
        {
            return CGRect(
                x: previousRect.maxX,
                y: previousRect.minY,
                width: caretWidth,
                height: max(directRect.height, previousRect.height)
            )
        }

        return directRect
    }

    // MARK: - Editor Binding

    /// Bind this text view to a Rust editor instance and apply initial content.
    ///
    /// - Parameters:
    ///   - id: The editor ID from `editor_create()`.
    ///   - initialHTML: Optional HTML to set as initial content.
    func bindEditor(id: UInt64, initialHTML: String? = nil) {
        editorId = id

        if let html = initialHTML, !html.isEmpty {
            let renderJSON = editorSetHtml(id: editorId, html: html)
            applyRenderJSON(renderJSON)
        } else {
            // Pull current state from Rust (content may already be loaded via bridge).
            let stateJSON = editorGetCurrentState(id: editorId)
            applyUpdateJSON(stateJSON)
        }
    }

    /// Unbind from the current editor instance.
    func unbindEditor() {
        editorId = 0
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
        guard !isApplyingRustState else {
            super.insertText(text)
            return
        }
        guard editorId != 0 else {
            super.insertText(text)
            return
        }

        // Handle Enter/Return as a block split operation.
        if text == "\n" {
            performInterceptedInput {
                handleReturnKey()
            }
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
                let updateJSON = editorReplaceTextScalar(
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

    @objc private func handleIndentKeyCommand() {
        handleListDepthKeyCommand(outdent: false)
    }

    @objc private func handleHardBreakKeyCommand() {
        performInterceptedInput {
            insertNodeInRust("hardBreak")
        }
    }

    @objc private func handleOutdentKeyCommand() {
        handleListDepthKeyCommand(outdent: true)
    }

    // MARK: - Input Interception: Deletion

    /// Intercept backward deletion (Backspace key).
    ///
    /// If there's a range selection, delete the range. If it's a cursor,
    /// delete the character (grapheme cluster) before the cursor.
    override func deleteBackward() {
        guard !isApplyingRustState else {
            super.deleteBackward()
            return
        }
        guard editorId != 0 else {
            super.deleteBackward()
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
            // Cursor: delete one grapheme cluster backward.
            let cursorPos = PositionBridge.textViewToScalar(selectedRange.start, in: self)
            guard cursorPos > 0 else { return }

            let cursorUtf16Offset = offset(from: beginningOfDocument, to: selectedRange.start)
            if let marker = PositionBridge.virtualListMarker(
                atUtf16Offset: cursorUtf16Offset,
                in: self
            ), marker.paragraphStartUtf16 == cursorUtf16Offset {
                performInterceptedInput {
                    deleteScalarRangeInRust(from: cursorPos - 1, to: cursorPos)
                }
                return
            }

            if let deleteRange = trailingHorizontalRuleDeleteRangeForBackwardDelete(
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

            // Find the start of the previous grapheme cluster.
            // We need to figure out how many scalars the previous grapheme occupies.
            let utf16Offset = offset(from: beginningOfDocument, to: selectedRange.start)
            if utf16Offset <= 0 { return }

            // Use UITextView's tokenizer to find the previous grapheme boundary.
            guard let prevPos = position(from: selectedRange.start, offset: -1) else { return }
            let prevScalar = PositionBridge.textViewToScalar(prevPos, in: self)

            performInterceptedInput {
                deleteScalarRangeInRust(from: prevScalar, to: cursorPos)
            }
        }
    }

    private func adjacentVoidBlockDeleteRangeForBackwardDelete(
        cursorUtf16Offset: Int,
        cursorScalar: UInt32
    ) -> (from: UInt32, to: UInt32)? {
        guard cursorUtf16Offset >= 0, cursorUtf16Offset < textStorage.length else {
            return nil
        }
        let attrs = textStorage.attributes(at: cursorUtf16Offset, effectiveRange: nil)
        guard attrs[.attachment] is NSTextAttachment,
              attrs[RenderBridgeAttributes.voidNodeType] as? String != nil,
              cursorScalar < UInt32.max
        else {
            return nil
        }
        return (from: cursorScalar, to: cursorScalar + 1)
    }

    private func trailingHorizontalRuleDeleteRangeForBackwardDelete(
        cursorUtf16Offset: Int
    ) -> (from: UInt32, to: UInt32)? {
        let text = textStorage.string as NSString
        guard text.length > 0 else { return nil }

        let clampedCursor = min(max(cursorUtf16Offset, 0), text.length)
        let paragraphProbe = min(max(clampedCursor - 1, 0), text.length - 1)
        let paragraphRange = text.paragraphRange(for: NSRange(location: paragraphProbe, length: 0))

        let placeholderRange = NSRange(location: paragraphRange.location, length: 1)
        guard placeholderRange.location + placeholderRange.length <= text.length else {
            return nil
        }

        let paragraphText = text.substring(with: placeholderRange)
        guard paragraphText == "\u{200B}" else { return nil }
        guard paragraphRange.location >= 2 else { return nil }
        guard text.character(at: paragraphRange.location - 1) == 0x000A else { return nil }

        let attachmentIndex = paragraphRange.location - 2
        let attrs = textStorage.attributes(at: attachmentIndex, effectiveRange: nil)
        guard attrs[.attachment] is NSTextAttachment,
              attrs[RenderBridgeAttributes.voidNodeType] as? String == "horizontalRule"
        else {
            return nil
        }

        let placeholderEndScalar = PositionBridge.utf16OffsetToScalar(
            placeholderRange.location + placeholderRange.length,
            in: self
        )
        guard placeholderEndScalar > 0 else { return nil }
        return (from: placeholderEndScalar - 1, to: placeholderEndScalar)
    }

    private func handleListDepthKeyCommand(outdent: Bool) {
        guard !isApplyingRustState else { return }
        guard editorId != 0 else { return }
        guard isEditable else { return }
        guard isCaretInsideList() else { return }
        guard let selection = currentScalarSelection() else { return }

        performInterceptedInput {
            let updateJSON = outdent
                ? editorOutdentListItemAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head
                )
                : editorIndentListItemAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head
                )
            applyUpdateJSON(updateJSON)
        }
    }

    private func isCaretInsideList() -> Bool {
        guard editorId != 0 else { return false }
        guard
            let data = editorGetCurrentState(id: editorId).data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let activeState = object["activeState"] as? [String: Any],
            let nodes = activeState["nodes"] as? [String: Any]
        else {
            return false
        }

        return nodes["bulletList"] as? Bool == true || nodes["orderedList"] as? Bool == true
    }

    // MARK: - Input Interception: Replace (Autocorrect)

    /// Intercept text replacement. This is called when:
    /// - Autocorrect replaces a word
    /// - User accepts a spelling suggestion
    /// - Programmatic text replacement
    ///
    /// We route the replacement through Rust to keep the document model in sync.
    override func replace(_ range: UITextRange, withText text: String) {
        guard !isApplyingRustState else {
            super.replace(range, withText: text)
            return
        }
        guard editorId != 0 else {
            super.replace(range, withText: text)
            return
        }

        let scalarRange = PositionBridge.textRangeToScalarRange(range, in: self)
        Self.inputLog.debug(
            "[replace] text=\(self.preview(text), privacy: .public) scalarRange=\(scalarRange.from)-\(scalarRange.to) selection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )

        // Atomically replace the range with the new text via Rust.
        performInterceptedInput {
            let updateJSON = editorReplaceTextScalar(
                id: editorId,
                scalarFrom: scalarRange.from,
                scalarTo: scalarRange.to,
                text: text
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
        isComposing = true
        Self.inputLog.debug(
            "[setMarkedText] marked=\(self.preview(markedText ?? ""), privacy: .public) nsRange=\(selectedRange.location),\(selectedRange.length) selection=\(self.selectionSummary(), privacy: .public)"
        )
        super.setMarkedText(markedText, selectedRange: selectedRange)
    }

    /// Called when composition is finalized (user selects a candidate or
    /// presses space/enter to commit).
    ///
    /// At this point, the composed text is final. We capture it and send
    /// it to Rust as a single insertion. `unmarkText` in UITextView will
    /// replace the marked text with the final text in the text storage,
    /// but we intercept at a higher level.
    override func unmarkText() {
        // Capture the finalized composed text before UIKit clears it.
        composedText = markedTextRange.flatMap { text(in: $0) }

        // Prevent selection sync while UIKit commits the marked text, since
        // the Rust document doesn't have the composed text yet.
        isApplyingRustState = true
        super.unmarkText()
        isApplyingRustState = false
        isComposing = false

        // Now route the composed text through Rust. The cursor is at the end
        // of the composed text, so the insert position is cursor - length.
        if let composed = composedText, !composed.isEmpty, editorId != 0 {
            let cursorPos = PositionBridge.cursorScalarOffset(in: self)
            let composedScalars = UInt32(composed.unicodeScalars.count)
            let insertPos = cursorPos >= composedScalars ? cursorPos - composedScalars : 0
            Self.inputLog.debug(
                "[unmarkText] composed=\(self.preview(composed), privacy: .public) cursorPos=\(cursorPos) insertPos=\(insertPos) selection=\(self.selectionSummary(), privacy: .public)"
            )
            performInterceptedInput {
                insertTextInRust(composed, at: insertPos)
            }
        }
        composedText = nil
    }

    // MARK: - Paste Handling

    /// Intercept paste operations to route content through Rust.
    ///
    /// Attempts to extract HTML from the pasteboard first (for rich text paste),
    /// falling back to plain text.
    override func paste(_ sender: Any?) {
        guard editorId != 0 else {
            super.paste(sender)
            return
        }

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
                        pasteHTML(html)
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

    // MARK: - Selection Change Notification

    /// UITextViewDelegate hook for user-driven selection updates.
    ///
    /// Using the delegate callback is more reliable than observing
    /// `selectedTextRange` directly because UIKit can adjust selection
    /// internally during tap handling and word-boundary resolution.
    func textViewDidChangeSelection(_ textView: UITextView) {
        guard textView === self else { return }
        scheduleSelectionSync()
    }

    func textView(
        _ textView: UITextView,
        shouldInteractWith URL: URL,
        in characterRange: NSRange,
        interaction: UITextItemInteraction
    ) -> Bool {
        return false
    }

    func textView(
        _ textView: UITextView,
        shouldInteractWith textAttachment: NSTextAttachment,
        in characterRange: NSRange,
        interaction: UITextItemInteraction
    ) -> Bool {
        return false
    }

    // MARK: - Private: Rust Integration

    private var isInterceptingInput: Bool {
        interceptedInputDepth > 0
    }

    private func performInterceptedInput(_ action: () -> Void) {
        interceptedInputDepth += 1
        Self.inputLog.debug(
            "[intercept.begin] depth=\(self.interceptedInputDepth) selection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )
        action()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.interceptedInputDepth = max(0, self.interceptedInputDepth - 1)
            Self.inputLog.debug(
                "[intercept.end] depth=\(self.interceptedInputDepth) selection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
            )
        }
    }

    private func preview(_ text: String, limit: Int = 32) -> String {
        let normalized = text.replacingOccurrences(of: "\n", with: "\\n")
        if normalized.count <= limit {
            return normalized
        }
        return "\(normalized.prefix(limit))…"
    }

    private func textSnapshotSummary() -> String {
        let text = textStorage.string
        return "len=\(text.count) preview=\"\(preview(text))\""
    }

    private func selectionSummary() -> String {
        guard let range = selectedTextRange else { return "none" }
        let anchorScalar = PositionBridge.textViewToScalar(range.start, in: self)
        let headScalar = PositionBridge.textViewToScalar(range.end, in: self)
        guard editorId != 0 else {
            return "scalar=\(anchorScalar)-\(headScalar)"
        }
        let docAnchor = editorScalarToDoc(id: editorId, scalar: anchorScalar)
        let docHead = editorScalarToDoc(id: editorId, scalar: headScalar)
        return "scalar=\(anchorScalar)-\(headScalar) doc=\(docAnchor)-\(docHead)"
    }

    private func selectionSummary(from selection: [String: Any]) -> String {
        guard let type = selection["type"] as? String else { return "unknown" }
        switch type {
        case "text":
            let anchor = (selection["anchor"] as? NSNumber)?.uint32Value ?? 0
            let head = (selection["head"] as? NSNumber)?.uint32Value ?? 0
            return "text doc=\(anchor)-\(head)"
        case "node":
            let pos = (selection["pos"] as? NSNumber)?.uint32Value ?? 0
            return "node doc=\(pos)"
        case "all":
            return "all"
        default:
            return type
        }
    }

    private func refreshTypingAttributesForSelection() {
        guard let range = selectedTextRange else {
            typingAttributes = defaultTypingAttributes()
            return
        }

        if textStorage.length == 0 {
            typingAttributes = defaultTypingAttributes()
            return
        }

        let startOffset = offset(from: beginningOfDocument, to: range.start)
        let attributeIndex: Int
        if startOffset < textStorage.length {
            attributeIndex = max(0, startOffset)
        } else {
            attributeIndex = textStorage.length - 1
        }

        var attrs = textStorage.attributes(at: attributeIndex, effectiveRange: nil)
        attrs[.font] = attrs[.font] ?? resolvedDefaultFont()
        attrs[.foregroundColor] = attrs[.foregroundColor] ?? resolvedDefaultTextColor()
        typingAttributes = attrs
    }

    private func scheduleSelectionSync() {
        pendingSelectionSyncGeneration &+= 1
        let generation = pendingSelectionSyncGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard self.pendingSelectionSyncGeneration == generation else { return }
            self.syncSelectionToRustAndNotifyDelegate()
        }
    }

    private func syncSelectionToRustAndNotifyDelegate() {
        guard !isApplyingRustState, editorId != 0 else { return }
        guard let range = selectedTextRange else { return }

        let anchor = PositionBridge.textViewToScalar(range.start, in: self)
        let head = PositionBridge.textViewToScalar(range.end, in: self)
        let docAnchor = editorScalarToDoc(id: editorId, scalar: anchor)
        let docHead = editorScalarToDoc(id: editorId, scalar: head)
        Self.selectionLog.debug(
            "[textViewDidChangeSelection] scalar=\(anchor)-\(head) doc=\(docAnchor)-\(docHead) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )

        editorSetSelectionScalar(id: editorId, scalarAnchor: anchor, scalarHead: head)
        refreshTypingAttributesForSelection()
        editorDelegate?.editorTextView(self, selectionDidChange: docAnchor, head: docHead)
    }

    func applyTheme(_ theme: EditorTheme?) {
        self.theme = theme
        if editorId != 0 {
            let previousOffset = contentOffset
            let stateJSON = editorGetCurrentState(id: editorId)
            applyUpdateJSON(stateJSON, notifyDelegate: false)
            if heightBehavior == .fixed {
                preserveScrollOffset(previousOffset)
            }
        } else {
            refreshTypingAttributesForSelection()
        }
        if heightBehavior == .autoGrow {
            notifyHeightChangeIfNeeded(force: true)
        }
    }

    private func preserveScrollOffset(_ previousOffset: CGPoint) {
        let restore = { [weak self] in
            guard let self else { return }
            self.layoutIfNeeded()

            let maxOffsetX = max(
                -self.adjustedContentInset.left,
                self.contentSize.width - self.bounds.width + self.adjustedContentInset.right
            )
            let maxOffsetY = max(
                -self.adjustedContentInset.top,
                self.contentSize.height - self.bounds.height + self.adjustedContentInset.bottom
            )

            let clampedOffset = CGPoint(
                x: min(max(previousOffset.x, -self.adjustedContentInset.left), maxOffsetX),
                y: min(max(previousOffset.y, -self.adjustedContentInset.top), maxOffsetY)
            )
            self.setContentOffset(clampedOffset, animated: false)
        }

        restore()
        DispatchQueue.main.async(execute: restore)
    }

    private func defaultTypingAttributes() -> [NSAttributedString.Key: Any] {
        [
            .font: resolvedDefaultFont(),
            .foregroundColor: resolvedDefaultTextColor(),
        ]
    }

    private func resolvedDefaultFont() -> UIFont {
        theme?.effectiveTextStyle(for: "paragraph").resolvedFont(fallback: baseFont)
            ?? baseFont
    }

    private func resolvedDefaultTextColor() -> UIColor {
        theme?.effectiveTextStyle(for: "paragraph").color ?? baseTextColor
    }

    private func notifyHeightChangeIfNeeded(force: Bool = false) {
        guard heightBehavior == .autoGrow else { return }
        let width = bounds.width > 0 ? bounds.width : UIScreen.main.bounds.width
        guard width > 0 else { return }
        let measuredHeight = ceil(
            sizeThatFits(CGSize(width: width, height: CGFloat.greatestFiniteMagnitude)).height
        )
        guard force || abs(measuredHeight - lastAutoGrowMeasuredHeight) > 0.5 else { return }
        lastAutoGrowMeasuredHeight = measuredHeight
        onHeightMayChange?()
    }

    static func adjustedCaretRect(
        from rect: CGRect,
        targetHeight: CGFloat,
        screenScale: CGFloat
    ) -> CGRect {
        guard rect.height > 0, targetHeight > 0, targetHeight < rect.height else {
            return rect
        }

        let scale = max(screenScale, 1)
        let alignedHeight = ceil(targetHeight * scale) / scale
        let centeredY = rect.minY + ((rect.height - alignedHeight) / 2.0)
        let alignedY = (centeredY * scale).rounded() / scale

        var adjusted = rect
        adjusted.origin.y = alignedY
        adjusted.size.height = alignedHeight
        return adjusted
    }

    static func adjustedCaretRect(
        from rect: CGRect,
        font: UIFont,
        screenScale: CGFloat
    ) -> CGRect {
        let scale = max(screenScale, 1)
        let lineHeight = max(font.lineHeight, 0)
        let alignedHeight = ceil(lineHeight * scale) / scale
        let alignedY = ((rect.maxY - alignedHeight) * scale).rounded() / scale

        var adjusted = rect
        adjusted.origin.y = alignedY
        adjusted.size.height = alignedHeight
        return adjusted
    }

    static func adjustedCaretRect(
        from rect: CGRect,
        baselineY: CGFloat,
        font: UIFont,
        screenScale: CGFloat
    ) -> CGRect {
        let scale = max(screenScale, 1)
        let lineHeight = max(font.lineHeight, 0)
        let alignedHeight = ceil(lineHeight * scale) / scale
        let typographicHeight = font.ascender - font.descender
        let leading = max(lineHeight - typographicHeight, 0)
        let topY = baselineY - font.ascender - (leading / 2.0)
        let alignedY = (topY * scale).rounded() / scale

        var adjusted = rect
        adjusted.origin.y = alignedY
        adjusted.size.height = alignedHeight
        return adjusted
    }

    private func caretBaselineY(for position: UITextPosition, referenceRect: CGRect) -> CGFloat? {
        guard textStorage.length > 0 else { return nil }

        let rawOffset = offset(from: beginningOfDocument, to: position)
        let clampedOffset = min(max(rawOffset, 0), textStorage.length)

        if let hardBreakBaselineY = hardBreakBaselineY(after: clampedOffset) {
            return hardBreakBaselineY
        }

        var candidateCharacters = Set<Int>()

        if clampedOffset < textStorage.length {
            candidateCharacters.insert(clampedOffset)
        }
        if clampedOffset > 0 {
            candidateCharacters.insert(clampedOffset - 1)
        }
        if clampedOffset + 1 < textStorage.length {
            candidateCharacters.insert(clampedOffset + 1)
        }

        guard !candidateCharacters.isEmpty else { return nil }

        let referenceMidY = referenceRect.midY
        let referenceMinY = referenceRect.minY
        var bestMatch: (score: CGFloat, baselineY: CGFloat)?

        for characterIndex in candidateCharacters.sorted() {
            let glyphIndex = layoutManager.glyphIndexForCharacter(at: characterIndex)
            guard glyphIndex < layoutManager.numberOfGlyphs else { continue }

            let lineFragmentRect = layoutManager.lineFragmentRect(
                forGlyphAt: glyphIndex,
                effectiveRange: nil
            )
            let lineRectInView = lineFragmentRect.offsetBy(dx: 0, dy: textContainerInset.top)
            let score = abs(lineRectInView.midY - referenceMidY) * 10
                + abs(lineRectInView.minY - referenceMinY)
            let glyphLocation = layoutManager.location(forGlyphAt: glyphIndex)
            let baselineY = textContainerInset.top + lineFragmentRect.minY + glyphLocation.y

            if let currentBest = bestMatch, currentBest.score <= score {
                continue
            }
            bestMatch = (score, baselineY)
        }

        return bestMatch?.baselineY
    }

    private func hardBreakBaselineY(after utf16Offset: Int) -> CGFloat? {
        guard utf16Offset > 0, utf16Offset <= textStorage.length else { return nil }
        let previousVoidType = textStorage.attribute(
            RenderBridgeAttributes.voidNodeType,
            at: utf16Offset - 1,
            effectiveRange: nil
        ) as? String
        guard previousVoidType == "hardBreak" else { return nil }

        let previousGlyphIndex = layoutManager.glyphIndexForCharacter(at: utf16Offset - 1)
        guard previousGlyphIndex < layoutManager.numberOfGlyphs else { return nil }

        let lineFragmentRect = layoutManager.lineFragmentRect(
            forGlyphAt: previousGlyphIndex,
            effectiveRange: nil
        )
        let glyphLocation = layoutManager.location(forGlyphAt: previousGlyphIndex)
        let previousBaselineY = textContainerInset.top + lineFragmentRect.minY + glyphLocation.y

        let paragraphStyle = textStorage.attribute(
            .paragraphStyle,
            at: utf16Offset - 1,
            effectiveRange: nil
        ) as? NSParagraphStyle
        let configuredLineHeight = max(
            paragraphStyle?.minimumLineHeight ?? 0,
            paragraphStyle?.maximumLineHeight ?? 0
        )
        let lineAdvance = configuredLineHeight > 0
            ? configuredLineHeight
            : lineFragmentRect.height

        return previousBaselineY + lineAdvance
    }

    private func resolvedCaretFont(for position: UITextPosition) -> UIFont {
        guard textStorage.length > 0 else { return resolvedDefaultFont() }

        let offset = offset(from: beginningOfDocument, to: position)
        let attributeIndex: Int
        if offset <= 0 {
            attributeIndex = 0
        } else if offset < textStorage.length {
            attributeIndex = offset
        } else {
            attributeIndex = textStorage.length - 1
        }

        return (textStorage.attribute(.font, at: attributeIndex, effectiveRange: nil) as? UIFont)
            ?? resolvedDefaultFont()
    }

    func performToolbarToggleMark(_ markName: String) {
        guard editorId != 0 else { return }
        guard isEditable else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = editorToggleMarkAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head,
                markName: markName
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarToggleList(_ listType: String, isActive: Bool) {
        guard editorId != 0 else { return }
        guard isEditable else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = isActive
                ? editorUnwrapFromListAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head
                )
                : editorWrapInListAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head,
                    listType: listType
                )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarToggleBlockquote() {
        guard editorId != 0 else { return }
        guard isEditable else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = editorToggleBlockquoteAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarIndentListItem() {
        guard editorId != 0 else { return }
        guard isEditable else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = editorIndentListItemAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarOutdentListItem() {
        guard editorId != 0 else { return }
        guard isEditable else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = editorOutdentListItemAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarInsertNode(_ nodeType: String) {
        guard editorId != 0 else { return }
        guard isEditable else { return }
        performInterceptedInput {
            insertNodeInRust(nodeType)
        }
    }

    func performToolbarUndo() {
        guard editorId != 0 else { return }
        guard isEditable else { return }
        performInterceptedInput {
            let updateJSON = editorUndo(id: editorId)
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarRedo() {
        guard editorId != 0 else { return }
        guard isEditable else { return }
        performInterceptedInput {
            let updateJSON = editorRedo(id: editorId)
            applyUpdateJSON(updateJSON)
        }
    }

    /// Insert text at a scalar position via the Rust editor.
    private func insertTextInRust(_ text: String, at scalarPos: UInt32) {
        Self.inputLog.debug(
            "[rust.insertTextScalar] text=\(self.preview(text), privacy: .public) scalarPos=\(scalarPos) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = editorInsertTextScalar(id: editorId, scalarPos: scalarPos, text: text)
        applyUpdateJSON(updateJSON)
    }

    private func insertNodeInRust(_ nodeType: String) {
        guard let selection = currentScalarSelection() else { return }
        Self.inputLog.debug(
            "[rust.insertNode] nodeType=\(nodeType, privacy: .public) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = editorInsertNodeAtSelectionScalar(
            id: editorId,
            scalarAnchor: selection.anchor,
            scalarHead: selection.head,
            nodeType: nodeType
        )
        applyUpdateJSON(updateJSON)
    }

    /// Delete a scalar range via the Rust editor.
    private func deleteScalarRangeInRust(from: UInt32, to: UInt32) {
        guard from < to else { return }
        Self.inputLog.debug(
            "[rust.deleteScalarRange] scalar=\(from)-\(to) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = editorDeleteScalarRange(id: editorId, scalarFrom: from, scalarTo: to)
        applyUpdateJSON(updateJSON)
    }

    /// Delete a document-position range via the Rust editor.
    private func deleteRangeInRust(from: UInt32, to: UInt32) {
        guard from < to else { return }
        Self.inputLog.debug(
            "[rust.deleteRange] doc=\(from)-\(to) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = editorDeleteRange(id: editorId, from: from, to: to)
        applyUpdateJSON(updateJSON)
    }

    private func currentScalarSelection() -> (anchor: UInt32, head: UInt32)? {
        guard let range = selectedTextRange else { return nil }
        let scalarRange = PositionBridge.textRangeToScalarRange(range, in: self)
        return (anchor: scalarRange.from, head: scalarRange.to)
    }

    /// Handle return key press as a block split operation.
    private func handleReturnKey() {
        // If there's a range selection, atomically delete and split.
        if let selectedRange = selectedTextRange, !selectedRange.isEmpty {
            let range = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
            let updateJSON = editorDeleteAndSplitScalar(
                id: editorId,
                scalarFrom: range.from,
                scalarTo: range.to
            )
            applyUpdateJSON(updateJSON)
        } else {
            let scalarPos = PositionBridge.cursorScalarOffset(in: self)
            splitBlockInRust(at: scalarPos)
        }
    }

    /// Split a block at a scalar position via the Rust editor.
    private func splitBlockInRust(at scalarPos: UInt32) {
        Self.inputLog.debug(
            "[rust.splitBlockScalar] scalarPos=\(scalarPos) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = editorSplitBlockScalar(id: editorId, scalarPos: scalarPos)
        applyUpdateJSON(updateJSON)
    }

    /// Paste HTML content through Rust.
    private func pasteHTML(_ html: String) {
        Self.inputLog.debug(
            "[rust.pasteHTML] html=\(self.preview(html), privacy: .public) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = editorInsertContentHtml(id: editorId, html: html)
        applyUpdateJSON(updateJSON)
    }

    /// Paste plain text through Rust.
    private func pastePlainText(_ text: String) {
        if let selectedRange = selectedTextRange, !selectedRange.isEmpty {
            // Atomically replace the selection with the pasted text.
            let range = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
            Self.inputLog.debug(
                "[rust.pastePlainText.replace] text=\(self.preview(text), privacy: .public) scalar=\(range.from)-\(range.to) selection=\(self.selectionSummary(), privacy: .public)"
            )
            let updateJSON = editorReplaceTextScalar(
                id: editorId,
                scalarFrom: range.from,
                scalarTo: range.to,
                text: text
            )
            applyUpdateJSON(updateJSON)
        } else {
            Self.inputLog.debug(
                "[rust.pastePlainText.insert] text=\(self.preview(text), privacy: .public) selection=\(self.selectionSummary(), privacy: .public)"
            )
            insertTextInRust(text, at: PositionBridge.cursorScalarOffset(in: self))
        }
    }

    // MARK: - Applying Rust State

    /// Apply a full render update from Rust to the text view.
    ///
    /// Parses the update JSON, converts render elements to NSAttributedString
    /// via RenderBridge, and replaces the text view's content.
    ///
    /// - Parameter updateJSON: The JSON string from editor_insert_text, etc.
    func applyUpdateJSON(_ updateJSON: String, notifyDelegate: Bool = true) {
        guard let data = updateJSON.data(using: .utf8),
              let update = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }

        // Extract render elements.
        guard let renderElements = update["renderElements"] as? [[String: Any]] else { return }
        let selectionFromUpdate = (update["selection"] as? [String: Any])
            .map(self.selectionSummary(from:)) ?? "none"
        Self.updateLog.debug(
            "[applyUpdateJSON.begin] renderCount=\(renderElements.count) updateSelection=\(selectionFromUpdate, privacy: .public) before=\(self.textSnapshotSummary(), privacy: .public)"
        )

        let attrStr = RenderBridge.renderElements(
            fromArray: renderElements,
            baseFont: baseFont,
            textColor: baseTextColor,
            theme: theme
        )

        // Apply the attributed string without triggering input interception.
        isApplyingRustState = true
        textStorage.beginEditing()
        textStorage.setAttributedString(attrStr)
        textStorage.endEditing()
        lastAuthorizedText = textStorage.string
        isApplyingRustState = false

        refreshPlaceholderVisibility()
        Self.updateLog.debug(
            "[applyUpdateJSON.rendered] after=\(self.textSnapshotSummary(), privacy: .public)"
        )

        // Apply the selection from the update.
        if let selection = update["selection"] as? [String: Any] {
            applySelectionFromJSON(selection)
        }
        refreshTypingAttributesForSelection()
        if heightBehavior == .autoGrow {
            notifyHeightChangeIfNeeded(force: true)
        }

        Self.updateLog.debug(
            "[applyUpdateJSON.end] finalSelection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )

        // Notify the delegate.
        if notifyDelegate {
            editorDelegate?.editorTextView(self, didReceiveUpdate: updateJSON)
        }
    }

    /// Apply a render JSON string (just render elements, no update wrapper).
    ///
    /// Used for initial content loading (set_html / set_json return render
    /// elements directly, not wrapped in an EditorUpdate).
    func applyRenderJSON(_ renderJSON: String) {
        Self.updateLog.debug(
            "[applyRenderJSON.begin] before=\(self.textSnapshotSummary(), privacy: .public)"
        )
        let attrStr = RenderBridge.renderElements(
            fromJSON: renderJSON,
            baseFont: baseFont,
            textColor: baseTextColor,
            theme: theme
        )

        isApplyingRustState = true
        textStorage.beginEditing()
        textStorage.setAttributedString(attrStr)
        textStorage.endEditing()
        lastAuthorizedText = textStorage.string
        isApplyingRustState = false

        refreshPlaceholderVisibility()
        refreshTypingAttributesForSelection()
        if heightBehavior == .autoGrow {
            notifyHeightChangeIfNeeded(force: true)
        }
        Self.updateLog.debug(
            "[applyRenderJSON.end] after=\(self.textSnapshotSummary(), privacy: .public)"
        )
    }

    /// Apply a selection from a parsed JSON selection object.
    ///
    /// The selection JSON matches the format from `serialize_editor_update`:
    /// ```json
    /// {"type": "text", "anchor": 5, "head": 5}
    /// {"type": "node", "pos": 10}
    /// {"type": "all"}
    /// ```
    private func applySelectionFromJSON(_ selection: [String: Any]) {
        guard let type = selection["type"] as? String else { return }

        isApplyingRustState = true
        defer { isApplyingRustState = false }

        switch type {
        case "text":
            guard let anchorNum = selection["anchor"] as? NSNumber,
                  let headNum = selection["head"] as? NSNumber
            else { return }
            // anchor/head from Rust are document positions; convert to scalar offsets first.
            let anchorScalar = editorDocToScalar(id: editorId, docPos: anchorNum.uint32Value)
            let headScalar = editorDocToScalar(id: editorId, docPos: headNum.uint32Value)

            let startPos = PositionBridge.scalarToTextView(min(anchorScalar, headScalar), in: self)
            let endPos = PositionBridge.scalarToTextView(max(anchorScalar, headScalar), in: self)
            selectedTextRange = textRange(from: startPos, to: endPos)
            Self.selectionLog.debug(
                "[applySelectionFromJSON.text] doc=\(anchorNum.uint32Value)-\(headNum.uint32Value) scalar=\(anchorScalar)-\(headScalar) final=\(self.selectionSummary(), privacy: .public)"
            )

        case "node":
            // Node selection: select the object replacement character at that position.
            guard let posNum = selection["pos"] as? NSNumber else { return }
            // pos from Rust is a document position; convert to scalar offset.
            let posScalar = editorDocToScalar(id: editorId, docPos: posNum.uint32Value)
            let startPos = PositionBridge.scalarToTextView(posScalar, in: self)
            // Select one character (the void node placeholder).
            if let endPos = position(from: startPos, offset: 1) {
                selectedTextRange = textRange(from: startPos, to: endPos)
            }
            Self.selectionLog.debug(
                "[applySelectionFromJSON.node] doc=\(posNum.uint32Value) scalar=\(posScalar) final=\(self.selectionSummary(), privacy: .public)"
            )

        case "all":
            selectedTextRange = textRange(from: beginningOfDocument, to: endOfDocument)
            Self.selectionLog.debug(
                "[applySelectionFromJSON.all] final=\(self.selectionSummary(), privacy: .public)"
            )

        default:
            break
        }
    }

}

// MARK: - EditorTextView + NSTextStorageDelegate (Reconciliation Fallback)

extension EditorTextView: NSTextStorageDelegate {

    /// Detect unauthorized text storage mutations after UIKit finishes
    /// processing an editing operation. If the text storage diverges from
    /// the last Rust-authorized content and the change was NOT initiated by
    /// our Rust apply path, re-render from Rust ("Rust wins").
    func textStorage(
        _ textStorage: NSTextStorage,
        didProcessEditing editedMask: NSTextStorage.EditActions,
        range editedRange: NSRange,
        changeInLength delta: Int
    ) {
        // Only care about actual character edits, not attribute-only changes.
        guard editedMask.contains(.editedCharacters) else { return }

        // Skip if this change came from our own Rust apply path.
        guard !isApplyingRustState, !isInterceptingInput else { return }

        // Skip if no editor is bound yet (nothing to reconcile against).
        guard editorId != 0 else { return }

        // Compare current text storage content against last authorized snapshot.
        let currentText = textStorage.string
        guard currentText != lastAuthorizedText else { return }
        let authorizedPreview = preview(lastAuthorizedText)
        let storagePreview = preview(currentText)

        // --- Divergence detected ---
        reconciliationCount += 1

        Self.reconciliationLog.warning(
            """
            [NativeEditor:reconciliation] Text storage diverged from Rust state \
            (count: \(self.reconciliationCount), \
            delta: \(delta), \
            editedRange: \(editedRange.location)..<\(editedRange.location + editedRange.length), \
            authorizedLen: \(self.lastAuthorizedText.count), \
            storageLen: \(currentText.count), \
            selection: \(self.selectionSummary(), privacy: .public), \
            interceptedDepth: \(self.interceptedInputDepth), \
            composing: \(self.isComposing), \
            authorizedPreview: \(authorizedPreview, privacy: .public), \
            storagePreview: \(storagePreview, privacy: .public))
            """
        )

        scheduleReconciliationFromRust()
    }

    private func scheduleReconciliationFromRust() {
        guard !reconciliationWorkScheduled else { return }
        reconciliationWorkScheduled = true

        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.reconciliationWorkScheduled = false

            guard !self.isApplyingRustState, !self.isInterceptingInput else { return }
            guard self.editorId != 0 else { return }
            guard self.textStorage.string != self.lastAuthorizedText else { return }

            // Reconcile by pulling the current editor state without rebuilding
            // the Rust backend or clearing history. This must run after the
            // current NSTextStorage edit transaction has finished.
            let stateJSON = editorGetCurrentState(id: self.editorId)
            self.applyUpdateJSON(stateJSON)
        }
    }
}

// MARK: - RichTextEditorView (Fabric Host)

/// The top-level container view that a Fabric component would own.
///
/// Hosts the EditorTextView. In a full Fabric integration, this would be
/// a `RCTViewComponentView` subclass registered via the component descriptor.
///
/// For now, this is a plain UIView that can be used in a UIKit context
/// and serves as the integration point for the future Fabric component.
final class RichTextEditorView: UIView {

    // MARK: - Properties

    /// The editor text view that handles input interception.
    let textView: EditorTextView
    private let remoteSelectionOverlayView = RemoteSelectionOverlayView()
    var onHeightMayChange: (() -> Void)?
    private var lastAutoGrowWidth: CGFloat = 0
    private var remoteSelections: [RemoteSelectionDecoration] = []

    var heightBehavior: EditorHeightBehavior = .fixed {
        didSet {
            guard oldValue != heightBehavior else { return }
            textView.heightBehavior = heightBehavior
            invalidateIntrinsicContentSize()
            setNeedsLayout()
            onHeightMayChange?()
            remoteSelectionOverlayView.refresh()
        }
    }

    /// The Rust editor instance ID. Setting this binds/unbinds the editor.
    var editorId: UInt64 = 0 {
        didSet {
            if editorId != 0 {
                textView.bindEditor(id: editorId)
            } else {
                textView.unbindEditor()
            }
            remoteSelectionOverlayView.update(
                selections: remoteSelections,
                editorId: editorId
            )
        }
    }

    // MARK: - Initialization

    override init(frame: CGRect) {
        textView = EditorTextView(frame: .zero, textContainer: nil)
        super.init(frame: frame)
        setupView()
    }

    required init?(coder: NSCoder) {
        textView = EditorTextView(frame: .zero, textContainer: nil)
        super.init(coder: coder)
        setupView()
    }

    private func setupView() {
        // Add the text view as a subview.
        textView.translatesAutoresizingMaskIntoConstraints = false
        remoteSelectionOverlayView.translatesAutoresizingMaskIntoConstraints = false
        remoteSelectionOverlayView.bind(textView: textView)
        textView.onHeightMayChange = { [weak self] in
            guard let self, self.heightBehavior == .autoGrow else { return }
            self.invalidateIntrinsicContentSize()
            self.superview?.setNeedsLayout()
            self.onHeightMayChange?()
        }
        textView.onViewportMayChange = { [weak self] in
            self?.remoteSelectionOverlayView.refresh()
        }
        addSubview(textView)
        addSubview(remoteSelectionOverlayView)

        NSLayoutConstraint.activate([
            textView.topAnchor.constraint(equalTo: topAnchor),
            textView.leadingAnchor.constraint(equalTo: leadingAnchor),
            textView.trailingAnchor.constraint(equalTo: trailingAnchor),
            textView.bottomAnchor.constraint(equalTo: bottomAnchor),
            remoteSelectionOverlayView.topAnchor.constraint(equalTo: topAnchor),
            remoteSelectionOverlayView.leadingAnchor.constraint(equalTo: leadingAnchor),
            remoteSelectionOverlayView.trailingAnchor.constraint(equalTo: trailingAnchor),
            remoteSelectionOverlayView.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    override var intrinsicContentSize: CGSize {
        guard heightBehavior == .autoGrow else {
            return CGSize(width: UIView.noIntrinsicMetric, height: UIView.noIntrinsicMetric)
        }

        let measuredHeight = measuredEditorHeight()
        guard measuredHeight > 0 else {
            return CGSize(width: UIView.noIntrinsicMetric, height: UIView.noIntrinsicMetric)
        }
        return CGSize(width: UIView.noIntrinsicMetric, height: measuredHeight)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        remoteSelectionOverlayView.refresh()
        guard heightBehavior == .autoGrow else { return }
        let currentWidth = bounds.width.rounded(.towardZero)
        guard currentWidth != lastAutoGrowWidth else { return }
        lastAutoGrowWidth = currentWidth
        invalidateIntrinsicContentSize()
    }

    // MARK: - Configuration

    /// Configure the editor's appearance.
    ///
    /// - Parameters:
    ///   - font: Base font for unstyled text.
    ///   - textColor: Default text color.
    ///   - backgroundColor: Background color for the text view.
    func configure(
        font: UIFont = .systemFont(ofSize: 16),
        textColor: UIColor = .label,
        backgroundColor: UIColor = .systemBackground
    ) {
        textView.baseFont = font
        textView.baseTextColor = textColor
        textView.baseBackgroundColor = backgroundColor
        textView.font = font
        textView.textColor = textColor
        textView.backgroundColor = backgroundColor
    }

    func applyTheme(_ theme: EditorTheme?) {
        textView.applyTheme(theme)
        let cornerRadius = theme?.borderRadius ?? 0
        layer.cornerRadius = cornerRadius
        clipsToBounds = cornerRadius > 0
        remoteSelectionOverlayView.refresh()
    }

    func setRemoteSelections(_ selections: [RemoteSelectionDecoration]) {
        remoteSelections = selections
        remoteSelectionOverlayView.update(
            selections: selections,
            editorId: editorId
        )
    }

    func refreshRemoteSelections() {
        remoteSelectionOverlayView.refresh()
    }

    func remoteSelectionOverlaySubviewsForTesting() -> [UIView] {
        remoteSelectionOverlayView.subviews
    }

    /// Set initial content from HTML.
    ///
    /// - Parameter html: The HTML string to load.
    func setContent(html: String) {
        guard editorId != 0 else { return }
        let renderJSON = editorSetHtml(id: editorId, html: html)
        textView.applyRenderJSON(renderJSON)
    }

    /// Set initial content from ProseMirror JSON.
    ///
    /// - Parameter json: The JSON string to load.
    func setContent(json: String) {
        guard editorId != 0 else { return }
        let renderJSON = editorSetJson(id: editorId, json: json)
        textView.applyRenderJSON(renderJSON)
    }

    private func measuredEditorHeight() -> CGFloat {
        let width = resolvedMeasurementWidth()
        guard width > 0 else { return 0 }
        return ceil(
            textView.sizeThatFits(
                CGSize(width: width, height: CGFloat.greatestFiniteMagnitude)
            ).height
        )
    }

    private func resolvedMeasurementWidth() -> CGFloat {
        if bounds.width > 0 {
            return bounds.width
        }
        if superview?.bounds.width ?? 0 > 0 {
            return superview?.bounds.width ?? 0
        }
        return UIScreen.main.bounds.width
    }

    // MARK: - Cleanup

    deinit {
        if editorId != 0 {
            textView.unbindEditor()
        }
    }
}
