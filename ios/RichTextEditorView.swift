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
    private struct ColoredRect {
        let frame: CGRect
        let color: UIColor
    }

    weak var textView: EditorTextView?
    private var editorId: UInt64 = 0
    private var selections: [RemoteSelectionDecoration] = []
    private var selectionViews: [UIView] = []
    private var caretViews: [UIView] = []

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
        guard editorId != 0,
              let textView
        else {
            syncSelectionViews(with: [])
            syncCaretViews(with: [])
            return
        }

        var selectionRects: [ColoredRect] = []
        var caretRects: [ColoredRect] = []

        for selection in selections {
            let geometry = geometry(for: selection, in: textView)
            for rect in geometry.selectionRects {
                selectionRects.append(
                    ColoredRect(
                        frame: rect.integral,
                        color: selection.color.withAlphaComponent(0.18)
                    )
                )
            }

            guard selection.isFocused,
                  let caretRect = geometry.caretRect
            else {
                continue
            }

            caretRects.append(
                ColoredRect(
                    frame: CGRect(
                        x: round(caretRect.minX),
                        y: round(caretRect.minY),
                        width: max(2, round(caretRect.width)),
                        height: round(caretRect.height)
                    ),
                    color: selection.color
                )
            )
        }

        syncSelectionViews(with: selectionRects)
        syncCaretViews(with: caretRects)
    }

    var hasVisibleDecorations: Bool {
        selectionViews.contains { !$0.isHidden } || caretViews.contains { !$0.isHidden }
    }

    var hasSelectionsOrVisibleDecorations: Bool {
        !selections.isEmpty || hasVisibleDecorations
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

    private func syncSelectionViews(with rects: [ColoredRect]) {
        syncViews(rects, existingViews: &selectionViews) { view, rect in
            view.frame = rect.frame
            view.backgroundColor = rect.color
            view.layer.cornerRadius = 3
        }
    }

    private func syncCaretViews(with rects: [ColoredRect]) {
        syncViews(rects, existingViews: &caretViews) { view, rect in
            view.frame = rect.frame
            view.backgroundColor = rect.color
            view.layer.cornerRadius = view.bounds.width / 2
            bringSubviewToFront(view)
        }
    }

    private func syncViews(
        _ rects: [ColoredRect],
        existingViews: inout [UIView],
        configure: (UIView, ColoredRect) -> Void
    ) {
        while existingViews.count < rects.count {
            let view = UIView(frame: .zero)
            view.isUserInteractionEnabled = false
            addSubview(view)
            existingViews.append(view)
        }

        for (index, rect) in rects.enumerated() {
            let view = existingViews[index]
            view.isHidden = false
            configure(view, rect)
        }

        if existingViews.count > rects.count {
            for view in existingViews[rects.count...] {
                view.isHidden = true
                view.frame = .zero
            }
        }
    }
}

private final class ImageTapOverlayView: UIView {
    private weak var editorView: RichTextEditorView?
    private lazy var tapRecognizer: UITapGestureRecognizer = {
        let recognizer = UITapGestureRecognizer(target: self, action: #selector(handleTap(_:)))
        recognizer.cancelsTouchesInView = true
        return recognizer
    }()

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        addGestureRecognizer(tapRecognizer)
    }

    required init?(coder: NSCoder) {
        return nil
    }

    func bind(editorView: RichTextEditorView) {
        self.editorView = editorView
    }

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        guard let editorView else { return false }
        let pointInTextView = convert(point, to: editorView.textView)
        return editorView.textView.hasImageAttachment(at: pointInTextView)
    }

    @objc
    private func handleTap(_ recognizer: UITapGestureRecognizer) {
        guard recognizer.state == .ended, let editorView else { return }
        let pointInTextView = convert(recognizer.location(in: self), to: editorView.textView)
        _ = editorView.textView.selectImageAttachment(at: pointInTextView)
    }

    func interceptsPointForTesting(_ point: CGPoint) -> Bool {
        self.point(inside: point, with: nil)
    }

    @discardableResult
    func handleTapForTesting(_ point: CGPoint) -> Bool {
        guard let editorView else { return false }
        let pointInTextView = convert(point, to: editorView.textView)
        return editorView.textView.selectImageAttachment(at: pointInTextView)
    }
}

private final class ImageResizeHandleView: UIView {
    let corner: ImageResizeOverlayView.Corner

    init(corner: ImageResizeOverlayView.Corner) {
        self.corner = corner
        super.init(frame: .zero)
        isUserInteractionEnabled = true
        backgroundColor = .systemBackground
        layer.borderColor = UIColor.systemBlue.cgColor
        layer.borderWidth = 2
        layer.cornerRadius = 10
    }

    required init?(coder: NSCoder) {
        return nil
    }
}

private final class ImageResizeOverlayView: UIView {
    enum Corner: CaseIterable {
        case topLeft
        case topRight
        case bottomLeft
        case bottomRight
    }

    private struct DragState {
        let corner: Corner
        let originalRect: CGRect
        let docPos: UInt32
        let maximumWidth: CGFloat
    }

    private weak var editorView: RichTextEditorView?
    private let selectionLayer = CAShapeLayer()
    private let previewBackdropView = UIView()
    private let previewImageView = UIImageView()
    private var handleViews: [Corner: ImageResizeHandleView] = [:]
    private var currentRect: CGRect?
    private var currentDocPos: UInt32?
    private var dragState: DragState?
    private let handleSize: CGFloat = 20
    private let minimumImageSize: CGFloat = 48

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        clipsToBounds = true

        previewBackdropView.isUserInteractionEnabled = false
        previewBackdropView.isHidden = true
        previewBackdropView.layer.zPosition = 1
        addSubview(previewBackdropView)

        previewImageView.isUserInteractionEnabled = false
        previewImageView.isHidden = true
        previewImageView.contentMode = .scaleToFill
        previewImageView.layer.zPosition = 2
        addSubview(previewImageView)

        selectionLayer.strokeColor = UIColor.systemBlue.cgColor
        selectionLayer.fillColor = UIColor.clear.cgColor
        selectionLayer.lineWidth = 2
        selectionLayer.zPosition = 10
        layer.addSublayer(selectionLayer)

        for corner in Corner.allCases {
            let handleView = ImageResizeHandleView(corner: corner)
            let panGesture = UIPanGestureRecognizer(target: self, action: #selector(handlePan(_:)))
            handleView.addGestureRecognizer(panGesture)
            handleView.layer.zPosition = 20
            addSubview(handleView)
            handleViews[corner] = handleView
        }

        isHidden = true
    }

    required init?(coder: NSCoder) {
        return nil
    }

    func bind(editorView: RichTextEditorView) {
        self.editorView = editorView
    }

    func refresh() {
        if dragState != nil {
            return
        }

        guard let editorView,
              let geometry = editorView.selectedImageGeometry()
        else {
            hideOverlay()
            return
        }

        hidePreviewLayers()
        applyGeometry(rect: geometry.rect, docPos: geometry.docPos)
    }

    func simulateResizeForTesting(width: CGFloat, height: CGFloat) {
        guard let docPos = currentDocPos else { return }
        editorView?.resizeImage(docPos: docPos, size: CGSize(width: width, height: height))
    }

    func simulatePreviewResizeForTesting(width: CGFloat, height: CGFloat) {
        guard beginPreviewResize(from: .bottomRight) else { return }
        let nextRect = CGRect(
            origin: dragState?.originalRect.origin ?? .zero,
            size: editorView?.clampedImageSize(
                CGSize(width: width, height: height),
                maximumWidth: dragState?.maximumWidth
            ) ?? CGSize(width: width, height: height)
        )
        updatePreviewRect(nextRect)
    }

    func commitPreviewResizeForTesting() {
        finishPreviewResize(commit: true)
    }

    var visibleRectForTesting: CGRect? {
        isHidden ? nil : currentRect
    }

    var isOverlayVisible: Bool {
        !isHidden
    }

    var previewHasImageForTesting: Bool {
        !previewImageView.isHidden && previewImageView.image != nil
    }

    func interceptsPointForTesting(_ location: CGPoint) -> Bool {
        self.point(inside: location, with: nil)
    }

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        guard !isHidden else { return false }
        for handleView in handleViews.values where !handleView.isHidden {
            if handleView.frame.insetBy(dx: -12, dy: -12).contains(point) {
                return true
            }
        }
        return false
    }

    private func hideOverlay() {
        hidePreviewLayers()
        dragState = nil
        currentRect = nil
        currentDocPos = nil
        selectionLayer.path = nil
        isHidden = true
    }

    private func applyGeometry(rect: CGRect, docPos: UInt32) {
        let integralRect = rect.integral
        currentRect = integralRect
        currentDocPos = docPos
        selectionLayer.path = UIBezierPath(roundedRect: integralRect, cornerRadius: 8).cgPath
        isHidden = false
        layoutHandleViews(for: integralRect)
    }

    private func hidePreviewLayers() {
        previewBackdropView.isHidden = true
        previewImageView.isHidden = true
        previewImageView.image = nil
    }

    private func showPreview(docPos: UInt32, originalRect: CGRect) {
        previewBackdropView.backgroundColor = editorView?.imageResizePreviewBackgroundColor() ?? .systemBackground
        previewBackdropView.frame = originalRect
        previewBackdropView.isHidden = false

        previewImageView.image = editorView?.imagePreviewForResize(docPos: docPos)
        previewImageView.frame = originalRect
        previewImageView.isHidden = previewImageView.image == nil
    }

    @discardableResult
    private func beginPreviewResize(from corner: Corner) -> Bool {
        guard let currentRect, let currentDocPos else { return false }
        editorView?.setImageResizePreviewActive(true)
        let maximumWidth = editorView?.maximumImageWidthForResizeGesture() ?? currentRect.width
        dragState = DragState(
            corner: corner,
            originalRect: currentRect,
            docPos: currentDocPos,
            maximumWidth: maximumWidth
        )
        showPreview(docPos: currentDocPos, originalRect: currentRect)
        return true
    }

    private func updatePreviewRect(_ rect: CGRect) {
        guard let currentDocPos else { return }
        applyGeometry(rect: rect, docPos: currentDocPos)
        previewImageView.frame = currentRect ?? rect.integral
    }

    private func finishPreviewResize(commit: Bool) {
        guard let dragState else { return }
        let finalSize = currentRect?.size ?? dragState.originalRect.size
        self.dragState = nil
        editorView?.setImageResizePreviewActive(false)
        if commit {
            editorView?.resizeImage(docPos: dragState.docPos, size: finalSize)
        } else {
            hidePreviewLayers()
        }
        DispatchQueue.main.async { [weak self] in
            self?.refresh()
        }
    }

    private func layoutHandleViews(for rect: CGRect) {
        for (corner, handleView) in handleViews {
            let center = handleCenter(for: corner, in: rect)
            handleView.frame = CGRect(
                x: center.x - (handleSize / 2),
                y: center.y - (handleSize / 2),
                width: handleSize,
                height: handleSize
            )
        }
    }

    private func handleCenter(for corner: Corner, in rect: CGRect) -> CGPoint {
        switch corner {
        case .topLeft:
            return CGPoint(x: rect.minX, y: rect.minY)
        case .topRight:
            return CGPoint(x: rect.maxX, y: rect.minY)
        case .bottomLeft:
            return CGPoint(x: rect.minX, y: rect.maxY)
        case .bottomRight:
            return CGPoint(x: rect.maxX, y: rect.maxY)
        }
    }

    private func anchorPoint(for corner: Corner, in rect: CGRect) -> CGPoint {
        switch corner {
        case .topLeft:
            return CGPoint(x: rect.maxX, y: rect.maxY)
        case .topRight:
            return CGPoint(x: rect.minX, y: rect.maxY)
        case .bottomLeft:
            return CGPoint(x: rect.maxX, y: rect.minY)
        case .bottomRight:
            return CGPoint(x: rect.minX, y: rect.minY)
        }
    }

    private func resizedRect(
        from originalRect: CGRect,
        corner: Corner,
        translation: CGPoint,
        maximumWidth: CGFloat?
    ) -> CGRect {
        let aspectRatio = max(originalRect.width / max(originalRect.height, 1), 0.1)
        let signedDx = (corner == .topRight || corner == .bottomRight) ? translation.x : -translation.x
        let signedDy = (corner == .bottomLeft || corner == .bottomRight) ? translation.y : -translation.y
        let widthScale = (originalRect.width + signedDx) / max(originalRect.width, 1)
        let heightScale = (originalRect.height + signedDy) / max(originalRect.height, 1)
        let scale = max(minimumImageSize / max(originalRect.width, 1), widthScale, heightScale)
        let unclampedSize = CGSize(
            width: max(minimumImageSize, originalRect.width * scale),
            height: max(minimumImageSize / aspectRatio, (max(minimumImageSize, originalRect.width * scale) / aspectRatio))
        )
        let clampedSize = editorView?.clampedImageSize(unclampedSize, maximumWidth: maximumWidth) ?? unclampedSize
        let width = clampedSize.width
        let height = clampedSize.height
        let anchor = anchorPoint(for: corner, in: originalRect)

        switch corner {
        case .topLeft:
            return CGRect(x: anchor.x - width, y: anchor.y - height, width: width, height: height)
        case .topRight:
            return CGRect(x: anchor.x, y: anchor.y - height, width: width, height: height)
        case .bottomLeft:
            return CGRect(x: anchor.x - width, y: anchor.y, width: width, height: height)
        case .bottomRight:
            return CGRect(x: anchor.x, y: anchor.y, width: width, height: height)
        }
    }

    @objc
    private func handlePan(_ gesture: UIPanGestureRecognizer) {
        guard let handleView = gesture.view as? ImageResizeHandleView else { return }

        switch gesture.state {
        case .began:
            _ = beginPreviewResize(from: handleView.corner)
        case .changed:
            guard let dragState else { return }
            let nextRect = resizedRect(
                from: dragState.originalRect,
                corner: dragState.corner,
                translation: gesture.translation(in: self),
                maximumWidth: dragState.maximumWidth
            )
            updatePreviewRect(nextRect)
        case .ended:
            finishPreviewResize(commit: true)
        case .cancelled, .failed:
            finishPreviewResize(commit: false)
        default:
            finishPreviewResize(commit: false)
        }
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
final class EditorTextView: UITextView, UITextViewDelegate, UIGestureRecognizerDelegate {
    private static let emptyBlockPlaceholderScalar = UnicodeScalar(0x200B)!

    override var undoManager: UndoManager? { nil }

    struct ApplyUpdateTrace {
        let attemptedPatch: Bool
        let usedPatch: Bool
        let usedSmallPatchTextMutation: Bool
        let applyRenderReplaceUtf16Length: Int
        let applyRenderReplacementUtf16Length: Int
        let parseNanos: UInt64
        let resolveRenderBlocksNanos: UInt64
        let patchEligibilityNanos: UInt64
        let patchTrimNanos: UInt64
        let patchMetadataNanos: UInt64
        let buildRenderNanos: UInt64
        let applyRenderNanos: UInt64
        let selectionNanos: UInt64
        let postApplyNanos: UInt64
        let totalNanos: UInt64
        let applyRenderTextMutationNanos: UInt64
        let applyRenderBeginEditingNanos: UInt64
        let applyRenderEndEditingNanos: UInt64
        let applyRenderStringMutationNanos: UInt64
        let applyRenderAttributeMutationNanos: UInt64
        let applyRenderAuthorizedTextNanos: UInt64
        let applyRenderCacheInvalidationNanos: UInt64
        let selectionResolveNanos: UInt64
        let selectionAssignmentNanos: UInt64
        let selectionChromeNanos: UInt64
        let postApplyTypingAttributesNanos: UInt64
        let postApplyHeightNotifyNanos: UInt64
        let postApplyHeightNotifyMeasureNanos: UInt64
        let postApplyHeightNotifyCallbackNanos: UInt64
        let postApplyHeightNotifyEnsureLayoutNanos: UInt64
        let postApplyHeightNotifyUsedRectNanos: UInt64
        let postApplyHeightNotifyContentSizeNanos: UInt64
        let postApplyHeightNotifySizeThatFitsNanos: UInt64
        let postApplySelectionOrContentCallbackNanos: UInt64
    }

    private struct PatchApplyTrace {
        let applied: Bool
        let eligibilityNanos: UInt64
        let trimNanos: UInt64
        let metadataNanos: UInt64
        let buildRenderNanos: UInt64
        let applyRenderNanos: UInt64
        let applyRenderReplaceUtf16Length: Int
        let applyRenderReplacementUtf16Length: Int
        let applyRenderTextMutationNanos: UInt64
        let applyRenderBeginEditingNanos: UInt64
        let applyRenderEndEditingNanos: UInt64
        let applyRenderStringMutationNanos: UInt64
        let applyRenderAttributeMutationNanos: UInt64
        let applyRenderAuthorizedTextNanos: UInt64
        let applyRenderCacheInvalidationNanos: UInt64
        let usedSmallPatchTextMutation: Bool
    }

    private struct ApplyRenderTrace {
        let totalNanos: UInt64
        let replaceUtf16Length: Int
        let replacementUtf16Length: Int
        let textMutationNanos: UInt64
        let beginEditingNanos: UInt64
        let endEditingNanos: UInt64
        let stringMutationNanos: UInt64
        let attributeMutationNanos: UInt64
        let authorizedTextNanos: UInt64
        let cacheInvalidationNanos: UInt64
        let usedSmallPatchTextMutation: Bool
    }

    private struct SelectionApplyTrace {
        let totalNanos: UInt64
        let resolveNanos: UInt64
        let assignmentNanos: UInt64
        let chromeNanos: UInt64
    }

    private struct PostApplyTrace {
        let totalNanos: UInt64
        let typingAttributesNanos: UInt64
        let heightNotifyNanos: UInt64
        let heightNotifyMeasureNanos: UInt64
        let heightNotifyCallbackNanos: UInt64
        let heightNotifyEnsureLayoutNanos: UInt64
        let heightNotifyUsedRectNanos: UInt64
        let heightNotifyContentSizeNanos: UInt64
        let heightNotifySizeThatFitsNanos: UInt64
        let selectionOrContentCallbackNanos: UInt64
    }

    private struct TopLevelChildMetadata {
        var startOffset: Int
        var containsAttachment: Bool
        var containsPositionAdjustments: Bool
    }

    private struct TopLevelChildMetadataSlice {
        let startIndex: Int
        let entries: [TopLevelChildMetadata]
    }

    private enum PositionCacheUpdate {
        case scan
        case invalidate
        case plainText
        case attributed
    }

    // MARK: - Properties

    /// The Rust editor instance ID (from editor_create / editor_create_with_max_length).
    /// Set to 0 when no editor is bound.
    var editorId: UInt64 = 0

    /// Guard flag to prevent re-entrant input interception while we're
    /// applying state from Rust (calling replaceCharacters on the text storage).
    var isApplyingRustState = false
    private var visibleSelectionTintColor: UIColor = .systemBlue
    private var hidesNativeSelectionChrome = false
    private var isPreviewingImageResize = false
    var allowImageResizing = true

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

    /// Optional render theme supplied by React.
    var theme: EditorTheme? {
        didSet {
            renderAppearanceRevision &+= 1
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
            invalidateAutoGrowHeightMeasurement()
            setNeedsLayout()
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
    private var lastAutoGrowMeasuredHeight: CGFloat = 0
    private var lastAutoGrowMeasuredWidth: CGFloat = 0
    private var autoGrowHostHeight: CGFloat = 0
    private var autoGrowHeightCheckIsDirty = true
    private var lastHeightNotifyMeasureNanosForTesting: UInt64 = 0
    private var lastHeightNotifyCallbackNanosForTesting: UInt64 = 0
    private var lastHeightNotifyEnsureLayoutNanosForTesting: UInt64 = 0
    private var lastHeightNotifyUsedRectNanosForTesting: UInt64 = 0
    private var lastHeightNotifyContentSizeNanosForTesting: UInt64 = 0
    private var lastHeightNotifySizeThatFitsNanosForTesting: UInt64 = 0

    /// Delegate for editor events.
    weak var editorDelegate: EditorTextViewDelegate?

    /// The plain text from the last Rust render, used by the reconciliation
    /// fallback to detect unauthorized text storage mutations.
    private var lastAuthorizedTextStorage = NSMutableString()
    private var lastAuthorizedText: String {
        lastAuthorizedTextStorage as String
    }
    private(set) var lastRenderAppliedPatchForTesting: Bool = false
    var captureApplyUpdateTraceForTesting = false
    private(set) var lastApplyUpdateTraceForTesting: ApplyUpdateTrace?
    private var currentRenderBlocks: [[[String: Any]]]? = nil
    private var currentTopLevelChildMetadata: [TopLevelChildMetadata]? = nil
    private var renderAppearanceRevision: UInt64 = 1
    private var lastAppliedRenderAppearanceRevision: UInt64 = 0

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
    private lazy var imageSelectionTapRecognizer: UITapGestureRecognizer = {
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
    private var interceptedInputDepth = 0
    private var reconciliationWorkScheduled = false

    /// Coalesces selection sync until UIKit has finished resolving the
    /// current tap/drag gesture's final caret position.
    private var pendingSelectionSyncGeneration: UInt64 = 0
    private var pendingDeferredImageSelectionRange: NSRange?
    private var pendingDeferredImageSelectionGeneration: UInt64 = 0

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
        visibleSelectionTintColor = tintColor

        // Register as the text storage delegate so we can detect unauthorized
        // mutations (reconciliation fallback).
        textStorage.delegate = self
        delegate = self
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

    @objc
    private func handleImageAttachmentDidLoad(_ notification: Notification) {
        guard notification.object is NSTextAttachment else { return }
        guard textStorage.length > 0 else { return }

        textStorage.beginEditing()
        textStorage.edited(.editedAttributes, range: NSRange(location: 0, length: textStorage.length), changeInLength: 0)
        textStorage.endEditing()
        invalidateAutoGrowHeightMeasurement()
        setNeedsLayout()
        invalidateIntrinsicContentSize()
        onSelectionOrContentMayChange?()
    }

    override func tintColorDidChange() {
        super.tintColorDidChange()
        if !hidesNativeSelectionChrome, tintColor.cgColor.alpha > 0 {
            visibleSelectionTintColor = tintColor
        }
    }

    @objc
    private func handleImageSelectionTap(_ gesture: UITapGestureRecognizer) {
        guard gesture.state == .ended, gesture.numberOfTouches == 1 else { return }
        let location = gesture.location(in: self)
        guard let range = imageAttachmentRange(at: location) else { return }
        scheduleDeferredImageSelection(for: range)
        _ = selectImageAttachment(range: range)
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

    @discardableResult
    private func selectImageAttachmentIfNeeded(at location: CGPoint) -> Bool {
        guard let range = imageAttachmentRange(at: location) else { return false }
        scheduleDeferredImageSelection(for: range)
        return selectImageAttachment(range: range)
    }

    @discardableResult
    func selectImageAttachment(at location: CGPoint) -> Bool {
        selectImageAttachmentIfNeeded(at: location)
    }

    func hasImageAttachment(at location: CGPoint) -> Bool {
        imageAttachmentRange(at: location) != nil
    }

    @discardableResult
    private func selectImageAttachment(range: NSRange) -> Bool {
        guard isSelectable,
              let start = position(from: beginningOfDocument, offset: range.location),
              let end = position(from: start, offset: range.length),
              let textRange = textRange(from: start, to: end)
        else {
            return false
        }

        _ = becomeFirstResponder()
        selectedTextRange = textRange
        refreshNativeSelectionChromeVisibility()
        onSelectionOrContentMayChange?()
        scheduleSelectionSync()
        return true
    }

    private func selectedUtf16Range() -> NSRange? {
        guard let range = selectedTextRange else { return nil }
        let location = offset(from: beginningOfDocument, to: range.start)
        let length = offset(from: range.start, to: range.end)
        guard location >= 0, length >= 0 else { return nil }
        return NSRange(location: location, length: length)
    }

    private func scheduleDeferredImageSelection(for range: NSRange) {
        pendingDeferredImageSelectionRange = range
        pendingDeferredImageSelectionGeneration &+= 1
        let generation = pendingDeferredImageSelectionGeneration
        DispatchQueue.main.async { [weak self] in
            self?.applyDeferredImageSelectionIfNeeded(generation: generation)
        }
    }

    private func applyDeferredImageSelectionIfNeeded(generation: UInt64) {
        guard pendingDeferredImageSelectionGeneration == generation,
              let pendingRange = pendingDeferredImageSelectionRange
        else {
            return
        }
        pendingDeferredImageSelectionRange = nil
        guard selectedUtf16Range() != pendingRange else { return }
        _ = selectImageAttachment(range: pendingRange)
    }

    private func installImageSelectionTapDependencies() {
        for view in gestureDependencyViews(startingAt: self) {
            guard let recognizers = view.gestureRecognizers else { continue }
            for recognizer in recognizers {
                guard recognizer !== imageSelectionTapRecognizer,
                      let tapRecognizer = recognizer as? UITapGestureRecognizer
                else {
                    continue
                }
                tapRecognizer.require(toFail: imageSelectionTapRecognizer)
            }
        }
    }

    private func gestureDependencyViews(startingAt rootView: UIView) -> [UIView] {
        var views: [UIView] = [rootView]
        for subview in rootView.subviews {
            views.append(contentsOf: gestureDependencyViews(startingAt: subview))
        }
        return views
    }

    private func imageAttachmentRange(at location: CGPoint) -> NSRange? {
        guard allowImageResizing else { return nil }
        guard textStorage.length > 0 else { return nil }

        let fullRange = NSRange(location: 0, length: textStorage.length)
        var resolvedRange: NSRange?

        textStorage.enumerateAttribute(
            .attachment,
            in: fullRange,
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, stop in
            guard value is NSTextAttachment, range.length > 0 else { return }

            let attrs = textStorage.attributes(at: range.location, effectiveRange: nil)
            guard (attrs[RenderBridgeAttributes.voidNodeType] as? String) == "image" else { return }

            let glyphRange = layoutManager.glyphRange(
                forCharacterRange: range,
                actualCharacterRange: nil
            )
            guard glyphRange.length > 0 else { return }

            var rect = layoutManager.boundingRect(forGlyphRange: glyphRange, in: textContainer)
            rect.origin.x += textContainerInset.left - contentOffset.x
            rect.origin.y += textContainerInset.top - contentOffset.y

            if rect.insetBy(dx: -8, dy: -8).contains(location) {
                resolvedRange = range
                stop.pointee = true
            }
        }

        return resolvedRange
    }

    func isPlaceholderVisibleForTesting() -> Bool {
        !placeholderLabel.isHidden
    }

    func placeholderFrameForTesting() -> CGRect {
        placeholderLabel.frame
    }

    func lastRenderAppliedPatch() -> Bool {
        lastRenderAppliedPatchForTesting
    }

    func lastApplyUpdateTrace() -> ApplyUpdateTrace? {
        lastApplyUpdateTraceForTesting
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

    @discardableResult
    func selectImageAttachmentForTesting(at location: CGPoint) -> Bool {
        selectImageAttachmentIfNeeded(at: location)
    }

    func imageSelectionTapWouldHandleForTesting(at location: CGPoint) -> Bool {
        imageAttachmentRange(at: location) != nil
    }

    func imageSelectionTapCancelsTouchesForTesting() -> Bool {
        imageSelectionTapRecognizer.cancelsTouchesInView
    }

    func imageSelectionTapYieldsToDefaultTapForTesting() -> Bool {
        gestureRecognizer(
            imageSelectionTapRecognizer,
            shouldBeRequiredToFailBy: UITapGestureRecognizer()
        ) || gestureRecognizer(
            imageSelectionTapRecognizer,
            shouldRequireFailureOf: UITapGestureRecognizer()
        )
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        guard gestureRecognizer === imageSelectionTapRecognizer,
              touch.tapCount == 1
        else {
            return true
        }

        return imageAttachmentRange(at: touch.location(in: self)) != nil
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRecognizeSimultaneouslyWith otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        false
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldRequireFailureOf otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        false
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer,
        shouldBeRequiredToFailBy otherGestureRecognizer: UIGestureRecognizer
    ) -> Bool {
        false
    }

    override func caretRect(for position: UITextPosition) -> CGRect {
        if hidesNativeSelectionChrome {
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

    func measuredAutoGrowHeightForTesting(width: CGFloat) -> CGFloat {
        measuredAutoGrowHeight(forWidth: width)
    }

    private func resolvedCaretReferenceRect(for position: UITextPosition) -> CGRect {
        let directRect = super.caretRect(for: position)
        if let horizontalRuleRect = resolvedHorizontalRuleAdjacentCaretRect(
            for: position,
            directRect: directRect
        ) {
            return horizontalRuleRect
        }
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

    private func resolvedHorizontalRuleAdjacentCaretRect(
        for position: UITextPosition,
        directRect: CGRect
    ) -> CGRect? {
        guard textStorage.length > 0 else { return nil }

        let utf16Offset = offset(from: beginningOfDocument, to: position)
        let caretWidth = max(directRect.width, 2)

        if isHorizontalRuleAttachment(at: utf16Offset),
           let previousCharacterIndex = nearestVisibleCharacterIndex(
               from: utf16Offset - 1,
               direction: -1
           ),
           let previousRect = visibleSelectionRect(forCharacterAt: previousCharacterIndex)
        {
            return CGRect(
                x: previousRect.maxX,
                y: previousRect.minY,
                width: caretWidth,
                height: max(directRect.height, previousRect.height)
            )
        }

        if isHorizontalRuleAttachment(at: utf16Offset - 1),
           let nextCharacterIndex = nearestVisibleCharacterIndex(
               from: utf16Offset,
               direction: 1
           ),
           let nextRect = visibleSelectionRect(forCharacterAt: nextCharacterIndex)
        {
            return CGRect(
                x: nextRect.minX,
                y: nextRect.minY,
                width: caretWidth,
                height: max(directRect.height, nextRect.height)
            )
        }

        return nil
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
            _ = editorSetHtml(id: editorId, html: html)
            let stateJSON = editorGetCurrentState(id: editorId)
            applyUpdateJSON(stateJSON, notifyDelegate: false)
        } else {
            // Pull current state from Rust (content may already be loaded via bridge).
            let stateJSON = editorGetCurrentState(id: editorId)
            applyUpdateJSON(stateJSON, notifyDelegate: false)
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
            if cursorPos == 0 {
                performInterceptedInput {
                    deleteBackwardAtSelectionScalarInRust(anchor: cursorPos, head: cursorPos)
                }
                return
            }

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

            // Find the start of the previous grapheme cluster.
            // We need to figure out how many scalars the previous grapheme occupies.
            let utf16Offset = offset(from: beginningOfDocument, to: selectedRange.start)
            if utf16Offset <= 0 { return }

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

    private func trailingVoidBlockDeleteRangeForBackwardDelete(
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
        guard
            let deleteRange = scalarDeleteRangeForVoidAttachment(at: attachmentIndex)
        else {
            return nil
        }

        return deleteRange
    }

    private func scalarDeleteRangeForVoidAttachment(
        at utf16Offset: Int
    ) -> (from: UInt32, to: UInt32)? {
        guard utf16Offset >= 0, utf16Offset < textStorage.length else {
            return nil
        }
        let attrs = textStorage.attributes(at: utf16Offset, effectiveRange: nil)
        guard attrs[.attachment] is NSTextAttachment,
              attrs[RenderBridgeAttributes.voidNodeType] as? String != nil
        else {
            return nil
        }

        let attachmentEndScalar = PositionBridge.utf16OffsetToScalar(
            utf16Offset + 1,
            in: self
        )
        guard attachmentEndScalar > 0 else { return nil }
        return (from: attachmentEndScalar - 1, to: attachmentEndScalar)
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
        guard !isApplyingRustState else { return }
        refreshNativeSelectionChromeVisibility()
        onSelectionOrContentMayChange?()
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
        guard textView === self,
              characterRange.location >= 0,
              characterRange.location < textStorage.length
        else {
            return false
        }

        let attrs = textStorage.attributes(at: characterRange.location, effectiveRange: nil)
        guard (attrs[RenderBridgeAttributes.voidNodeType] as? String) == "image",
              let start = position(from: beginningOfDocument, offset: characterRange.location),
              let end = position(from: start, offset: characterRange.length)
        else {
            return false
        }

        selectedTextRange = textRange(from: start, to: end)
        refreshNativeSelectionChromeVisibility()
        onSelectionOrContentMayChange?()
        scheduleSelectionSync()
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

    private func setNativeSelectionChromeHidden(_ hidden: Bool) {
        guard hidesNativeSelectionChrome != hidden else { return }
        hidesNativeSelectionChrome = hidden
        super.tintColor = hidden ? .clear : visibleSelectionTintColor
    }

    private func refreshNativeSelectionChromeVisibility() {
        let hidden = selectedImageSelectionState() != nil
        if !hidden, tintColor.cgColor.alpha > 0 {
            visibleSelectionTintColor = tintColor
        }
        setNativeSelectionChromeHidden(hidden)
    }

    private func showNativeSelectionChromeIfNeeded() {
        if tintColor.cgColor.alpha > 0 {
            visibleSelectionTintColor = tintColor
        }
        setNativeSelectionChromeHidden(false)
    }

    func refreshSelectionVisualState() {
        refreshNativeSelectionChromeVisibility()
        onSelectionOrContentMayChange?()
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
        if !force {
            let measuredWidth = ceil(width)
            if !autoGrowHeightCheckIsDirty && abs(measuredWidth - lastAutoGrowMeasuredWidth) <= 0.5 {
                return
            }
        }
        lastHeightNotifyEnsureLayoutNanosForTesting = 0
        lastHeightNotifyUsedRectNanosForTesting = 0
        lastHeightNotifyContentSizeNanosForTesting = 0
        lastHeightNotifySizeThatFitsNanosForTesting = 0
        let measurementStartedAt = DispatchTime.now().uptimeNanoseconds
        let measuredHeight = measuredAutoGrowHeight(forWidth: width)
        lastHeightNotifyMeasureNanosForTesting =
            DispatchTime.now().uptimeNanoseconds - measurementStartedAt
        autoGrowHeightCheckIsDirty = false
        lastAutoGrowMeasuredWidth = ceil(width)
        guard force || abs(measuredHeight - lastAutoGrowMeasuredHeight) > 0.5 else { return }
        lastAutoGrowMeasuredHeight = measuredHeight
        let callbackStartedAt = DispatchTime.now().uptimeNanoseconds
        onHeightMayChange?(measuredHeight)
        lastHeightNotifyCallbackNanosForTesting =
            DispatchTime.now().uptimeNanoseconds - callbackStartedAt
    }

    private func measuredAutoGrowHeight(forWidth width: CGFloat) -> CGFloat {
        guard width > 0 else { return 0 }

        if abs(bounds.width - width) <= 0.5 {
            let currentHeight = ceil(bounds.height)
            let ensureLayoutStartedAt = DispatchTime.now().uptimeNanoseconds
            editorLayoutManager.ensureLayout(for: textContainer)
            lastHeightNotifyEnsureLayoutNanosForTesting =
                DispatchTime.now().uptimeNanoseconds - ensureLayoutStartedAt

            let usedRectStartedAt = DispatchTime.now().uptimeNanoseconds
            var usedRect = editorLayoutManager.usedRect(for: textContainer)
            let extraLineFragmentRect = editorLayoutManager.extraLineFragmentRect
            if !extraLineFragmentRect.isEmpty {
                usedRect = usedRect.union(extraLineFragmentRect)
            }
            lastHeightNotifyUsedRectNanosForTesting =
                DispatchTime.now().uptimeNanoseconds - usedRectStartedAt
            let layoutHeight = ceil(usedRect.height + textContainerInset.top + textContainerInset.bottom)

            let contentSizeStartedAt = DispatchTime.now().uptimeNanoseconds
            let contentHeight = ceil(contentSize.height)
            lastHeightNotifyContentSizeNanosForTesting =
                DispatchTime.now().uptimeNanoseconds - contentSizeStartedAt
            if currentHeight > 0 {
                if layoutHeight > currentHeight + 0.5 {
                    return layoutHeight
                }
                let hostIsTrackingMeasuredHeight =
                    autoGrowHostHeight > 0
                    && abs(currentHeight - ceil(autoGrowHostHeight)) <= 1.0
                guard hostIsTrackingMeasuredHeight else {
                    return layoutHeight
                }
                let measuredFromLayout = max(layoutHeight, contentHeight)
                if measuredFromLayout > currentHeight + 0.5 {
                    return measuredFromLayout
                }
                let sizeThatFitsStartedAt = DispatchTime.now().uptimeNanoseconds
                let fittedHeight = ceil(
                    sizeThatFits(
                        CGSize(width: width, height: CGFloat.greatestFiniteMagnitude)
                    ).height
                )
                lastHeightNotifySizeThatFitsNanosForTesting =
                    DispatchTime.now().uptimeNanoseconds - sizeThatFitsStartedAt
                if fittedHeight > currentHeight + 0.5 {
                    return max(measuredFromLayout, fittedHeight)
                }
                return layoutHeight
            }
            return max(layoutHeight, contentHeight)
        }

        let sizeThatFitsStartedAt = DispatchTime.now().uptimeNanoseconds
        let fittedHeight = ceil(
            sizeThatFits(CGSize(width: width, height: CGFloat.greatestFiniteMagnitude)).height
        )
        lastHeightNotifySizeThatFitsNanosForTesting =
            DispatchTime.now().uptimeNanoseconds - sizeThatFitsStartedAt
        return fittedHeight
    }

    func updateAutoGrowHostHeight(_ height: CGFloat) {
        autoGrowHostHeight = max(0, ceil(height))
    }

    private func invalidateAutoGrowHeightMeasurement() {
        autoGrowHeightCheckIsDirty = true
        lastAutoGrowMeasuredWidth = 0
    }

    private func performPostApplyMaintenance(forceHeightNotify: Bool = false) -> PostApplyTrace {
        let totalStartedAt = DispatchTime.now().uptimeNanoseconds

        let typingAttributesStartedAt = totalStartedAt
        refreshTypingAttributesForSelection()
        let typingAttributesNanos = DispatchTime.now().uptimeNanoseconds - typingAttributesStartedAt

        let heightNotifyStartedAt = DispatchTime.now().uptimeNanoseconds
        lastHeightNotifyMeasureNanosForTesting = 0
        lastHeightNotifyCallbackNanosForTesting = 0
        lastHeightNotifyEnsureLayoutNanosForTesting = 0
        lastHeightNotifyUsedRectNanosForTesting = 0
        lastHeightNotifyContentSizeNanosForTesting = 0
        lastHeightNotifySizeThatFitsNanosForTesting = 0
        if heightBehavior == .autoGrow {
            invalidateAutoGrowHeightMeasurement()
            if forceHeightNotify || window == nil {
                notifyHeightChangeIfNeeded(force: forceHeightNotify)
            } else {
                setNeedsLayout()
            }
        }
        let heightNotifyNanos = DispatchTime.now().uptimeNanoseconds - heightNotifyStartedAt

        let selectionOrContentStartedAt = DispatchTime.now().uptimeNanoseconds
        onSelectionOrContentMayChange?()
        let selectionOrContentCallbackNanos =
            DispatchTime.now().uptimeNanoseconds - selectionOrContentStartedAt

        return PostApplyTrace(
            totalNanos: DispatchTime.now().uptimeNanoseconds - totalStartedAt,
            typingAttributesNanos: typingAttributesNanos,
            heightNotifyNanos: heightNotifyNanos,
            heightNotifyMeasureNanos: lastHeightNotifyMeasureNanosForTesting,
            heightNotifyCallbackNanos: lastHeightNotifyCallbackNanosForTesting,
            heightNotifyEnsureLayoutNanos: lastHeightNotifyEnsureLayoutNanosForTesting,
            heightNotifyUsedRectNanos: lastHeightNotifyUsedRectNanosForTesting,
            heightNotifyContentSizeNanos: lastHeightNotifyContentSizeNanosForTesting,
            heightNotifySizeThatFitsNanos: lastHeightNotifySizeThatFitsNanosForTesting,
            selectionOrContentCallbackNanos: selectionOrContentCallbackNanos
        )
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

        if let horizontalRuleBaselineY = horizontalRuleAdjacentBaselineY(at: clampedOffset) {
            return horizontalRuleBaselineY
        }

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

    private func horizontalRuleAdjacentBaselineY(at utf16Offset: Int) -> CGFloat? {
        guard textStorage.length > 0 else { return nil }

        if isHorizontalRuleAttachment(at: utf16Offset),
           let previousCharacterIndex = nearestVisibleCharacterIndex(
               from: utf16Offset - 1,
               direction: -1
           )
        {
            return baselineY(forCharacterAt: previousCharacterIndex)
        }

        if isHorizontalRuleAttachment(at: utf16Offset - 1),
           let nextCharacterIndex = nearestVisibleCharacterIndex(
               from: utf16Offset,
               direction: 1
           )
        {
            return baselineY(forCharacterAt: nextCharacterIndex)
        }

        return nil
    }

    private func baselineY(forCharacterAt characterIndex: Int) -> CGFloat? {
        guard characterIndex >= 0, characterIndex < textStorage.length else { return nil }

        let glyphIndex = layoutManager.glyphIndexForCharacter(at: characterIndex)
        guard glyphIndex < layoutManager.numberOfGlyphs else { return nil }

        let lineFragmentRect = layoutManager.lineFragmentRect(
            forGlyphAt: glyphIndex,
            effectiveRange: nil
        )
        let glyphLocation = layoutManager.location(forGlyphAt: glyphIndex)
        return textContainerInset.top + lineFragmentRect.minY + glyphLocation.y
    }

    private func visibleSelectionRect(forCharacterAt characterIndex: Int) -> CGRect? {
        guard characterIndex >= 0, characterIndex < textStorage.length else { return nil }
        guard let start = position(from: beginningOfDocument, offset: characterIndex),
              let end = position(from: start, offset: 1),
              let range = textRange(from: start, to: end)
        else {
            return nil
        }

        return selectionRects(for: range)
            .map(\.rect)
            .first(where: { !$0.isEmpty && $0.width > 0 && $0.height > 0 })
    }

    private func nearestVisibleCharacterIndex(from startIndex: Int, direction: Int) -> Int? {
        guard direction == -1 || direction == 1 else { return nil }
        guard textStorage.length > 0 else { return nil }

        let text = textStorage.string as NSString
        var index = startIndex

        while index >= 0, index < text.length {
            let attrs = textStorage.attributes(at: index, effectiveRange: nil)
            let character = text.substring(with: NSRange(location: index, length: 1))

            if attrs[.attachment] == nil,
               character != "\n",
               character != "\r",
               visibleSelectionRect(forCharacterAt: index) != nil
            {
                return index
            }

            index += direction
        }

        return nil
    }

    private func isHorizontalRuleAttachment(at utf16Offset: Int) -> Bool {
        guard utf16Offset >= 0, utf16Offset < textStorage.length else { return false }

        let attrs = textStorage.attributes(at: utf16Offset, effectiveRange: nil)
        return attrs[.attachment] is NSTextAttachment
            && (attrs[RenderBridgeAttributes.voidNodeType] as? String) == "horizontalRule"
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

    func performToolbarToggleHeading(_ level: Int) {
        guard editorId != 0 else { return }
        guard isEditable else { return }
        guard let selection = currentScalarSelection() else { return }
        guard let level = UInt8(exactly: level), (1...6).contains(level) else { return }
        performInterceptedInput {
            let updateJSON = editorToggleHeadingAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head,
                level: level
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

    private func deleteBackwardAtSelectionScalarInRust(anchor: UInt32, head: UInt32) {
        Self.inputLog.debug(
            "[rust.deleteBackwardAtSelectionScalar] scalar=\(anchor)-\(head) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = editorDeleteBackwardAtSelectionScalar(
            id: editorId,
            scalarAnchor: anchor,
            scalarHead: head
        )
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

    private func selectedImageSelectionState() -> (docPos: UInt32, utf16Offset: Int)? {
        guard allowImageResizing else { return nil }
        guard isFirstResponder else { return nil }
        guard let selectedRange = selectedUtf16Range(),
              selectedRange.length == 1,
              selectedRange.location >= 0,
              selectedRange.location < textStorage.length
        else {
            return nil
        }

        let attrs = textStorage.attributes(at: selectedRange.location, effectiveRange: nil)
        guard (attrs[RenderBridgeAttributes.voidNodeType] as? String) == "image",
              attrs[.attachment] is NSTextAttachment
        else {
            return nil
        }

        let docPos = (attrs[RenderBridgeAttributes.docPos] as? NSNumber)?.uint32Value
            ?? (attrs[RenderBridgeAttributes.docPos] as? UInt32)
        guard let docPos else { return nil }
        return (docPos, selectedRange.location)
    }

    func selectedImageGeometry() -> (docPos: UInt32, rect: CGRect)? {
        guard let selectionState = selectedImageSelectionState() else { return nil }

        let glyphRange = layoutManager.glyphRange(
            forCharacterRange: NSRange(location: selectionState.utf16Offset, length: 1),
            actualCharacterRange: nil
        )
        guard glyphRange.length > 0 else { return nil }

        var rect = layoutManager.boundingRect(forGlyphRange: glyphRange, in: textContainer)
        rect.origin.x += textContainerInset.left
        rect.origin.y += textContainerInset.top
        guard rect.width > 0, rect.height > 0 else { return nil }
        return (selectionState.docPos, rect)
    }

    private func blockImageAttachment(docPos: UInt32) -> (range: NSRange, attachment: BlockImageAttachment)? {
        let fullRange = NSRange(location: 0, length: textStorage.length)
        var resolved: (range: NSRange, attachment: BlockImageAttachment)?
        textStorage.enumerateAttribute(
            .attachment,
            in: fullRange,
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, stop in
            guard let attachment = value as? BlockImageAttachment, range.length > 0 else { return }
            let attrs = textStorage.attributes(at: range.location, effectiveRange: nil)
            guard (attrs[RenderBridgeAttributes.voidNodeType] as? String) == "image" else { return }
            let attributeDocPos = (attrs[RenderBridgeAttributes.docPos] as? NSNumber)?.uint32Value
                ?? (attrs[RenderBridgeAttributes.docPos] as? UInt32)
            guard attributeDocPos == docPos else { return }
            resolved = (range, attachment)
            stop.pointee = true
        }
        return resolved
    }

    func imagePreviewForDocPos(_ docPos: UInt32) -> UIImage? {
        blockImageAttachment(docPos: docPos)?.attachment.previewImage()
    }

    func maximumRenderableImageWidth() -> CGFloat {
        let containerWidth: CGFloat
        if bounds.width > 0 {
            containerWidth = bounds.width - textContainerInset.left - textContainerInset.right
        } else {
            containerWidth = textContainer.size.width
        }
        let linePadding = textContainer.lineFragmentPadding * 2
        return max(48, containerWidth - linePadding)
    }

    func resizeImageAtDocPos(_ docPos: UInt32, width: UInt32, height: UInt32) {
        guard editorId != 0 else { return }
        performInterceptedInput {
            let updateJSON = editorResizeImageAtDocPos(
                id: editorId,
                docPos: docPos,
                width: width,
                height: height
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func previewResizeImageAtDocPos(_ docPos: UInt32, width: CGFloat, height: CGFloat) {
        guard let attachmentState = blockImageAttachment(docPos: docPos) else { return }
        attachmentState.attachment.setPreferredSize(width: width, height: height)
        layoutManager.invalidateLayout(forCharacterRange: attachmentState.range, actualCharacterRange: nil)
        layoutManager.invalidateDisplay(forCharacterRange: attachmentState.range)
        textStorage.beginEditing()
        textStorage.edited(.editedAttributes, range: attachmentState.range, changeInLength: 0)
        textStorage.endEditing()
    }

    func setImageResizePreviewActive(_ active: Bool) {
        isPreviewingImageResize = active
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

    private struct ParsedRenderPatch {
        let startIndex: Int
        let deleteCount: Int
        let renderBlocks: [[[String: Any]]]
    }

    private enum DerivedRenderPatch {
        case unchanged
        case patch(ParsedRenderPatch)
    }

    private func parseRenderBlocks(_ value: Any?) -> [[[String: Any]]]? {
        value as? [[[String: Any]]]
    }

    private func parseRenderPatch(_ value: Any?) -> ParsedRenderPatch? {
        guard let raw = value as? [String: Any],
              let startIndex = RenderBridge.jsonInt(raw["startIndex"]),
              let deleteCount = RenderBridge.jsonInt(raw["deleteCount"]),
              let renderBlocks = parseRenderBlocks(raw["renderBlocks"])
        else {
            return nil
        }

        return ParsedRenderPatch(
            startIndex: startIndex,
            deleteCount: deleteCount,
            renderBlocks: renderBlocks
        )
    }

    private func mergeRenderBlocks(
        applying patch: ParsedRenderPatch,
        to current: [[[String: Any]]]
    ) -> [[[String: Any]]]? {
        guard patch.startIndex >= 0,
              patch.deleteCount >= 0,
              patch.startIndex <= current.count,
              patch.startIndex + patch.deleteCount <= current.count
        else {
            return nil
        }

        var merged = current
        merged.replaceSubrange(
            patch.startIndex..<(patch.startIndex + patch.deleteCount),
            with: patch.renderBlocks
        )
        return merged
    }

    private func renderBlockEquals(
        _ lhs: [[String: Any]],
        _ rhs: [[String: Any]]
    ) -> Bool {
        (lhs as NSArray).isEqual(rhs)
    }

    private func deriveRenderPatch(
        from current: [[[String: Any]]],
        to updated: [[[String: Any]]]
    ) -> DerivedRenderPatch {
        let sharedCount = min(current.count, updated.count)

        var prefix = 0
        while prefix < sharedCount, renderBlockEquals(current[prefix], updated[prefix]) {
            prefix += 1
        }

        if prefix == current.count, prefix == updated.count {
            return .unchanged
        }

        var suffix = 0
        while suffix < (sharedCount - prefix),
              renderBlockEquals(
                  current[current.count - suffix - 1],
                  updated[updated.count - suffix - 1]
              )
        {
            suffix += 1
        }

        let startIndex = prefix
        let deleteCount = current.count - prefix - suffix
        let endIndex = updated.count - suffix
        let replacementBlocks = Array(updated[startIndex..<endIndex])

        return .patch(
            ParsedRenderPatch(
                startIndex: startIndex,
                deleteCount: deleteCount,
                renderBlocks: replacementBlocks
            )
        )
    }

    private func topLevelChildIndex(from value: Any?) -> Int? {
        if let number = value as? NSNumber {
            return number.intValue
        }
        return value as? Int
    }

    private func topLevelChildMetadataSlice(
        from attributedString: NSAttributedString
    ) -> TopLevelChildMetadataSlice? {
        guard attributedString.length > 0 else {
            return TopLevelChildMetadataSlice(startIndex: 0, entries: [])
        }

        var entriesByIndex: [Int: TopLevelChildMetadata] = [:]
        var orderedIndexes: [Int] = []

        attributedString.enumerateAttributes(
            in: NSRange(location: 0, length: attributedString.length),
            options: []
        ) { attrs, range, _ in
            guard let index = topLevelChildIndex(from: attrs[RenderBridgeAttributes.topLevelChildIndex]) else {
                return
            }
            if entriesByIndex[index] == nil {
                entriesByIndex[index] = TopLevelChildMetadata(
                    startOffset: range.location,
                    containsAttachment: false,
                    containsPositionAdjustments: false
                )
                orderedIndexes.append(index)
            }
            if attrs[.attachment] != nil {
                entriesByIndex[index]?.containsAttachment = true
            }
            if attrs[RenderBridgeAttributes.syntheticPlaceholder] as? Bool == true
                || attrs[RenderBridgeAttributes.listMarkerContext] != nil
            {
                entriesByIndex[index]?.containsPositionAdjustments = true
            }
        }

        guard !orderedIndexes.isEmpty else { return nil }
        orderedIndexes.sort()
        guard let startIndex = orderedIndexes.first else { return nil }

        var entries: [TopLevelChildMetadata] = []
        entries.reserveCapacity(orderedIndexes.count)
        for (offset, index) in orderedIndexes.enumerated() {
            guard index == startIndex + offset,
                  let entry = entriesByIndex[index]
            else {
                return nil
            }
            entries.append(entry)
        }

        return TopLevelChildMetadataSlice(startIndex: startIndex, entries: entries)
    }

    private func refreshTopLevelChildMetadata(
        from attributedString: NSAttributedString
    ) {
        guard let slice = topLevelChildMetadataSlice(from: attributedString),
              slice.startIndex == 0
        else {
            currentTopLevelChildMetadata = nil
            return
        }
        currentTopLevelChildMetadata = slice.entries
    }

    private func applyTopLevelChildMetadataPatch(
        _ patch: ParsedRenderPatch,
        replaceRange: NSRange,
        renderedPatchMetadata: TopLevelChildMetadataSlice?,
        renderedPatchLength: Int
    ) {
        guard var currentMetadata = currentTopLevelChildMetadata else {
            currentTopLevelChildMetadata = nil
            return
        }

        let newEntries: [TopLevelChildMetadata]
        if let renderedPatchMetadata,
           renderedPatchMetadata.entries.isEmpty
        {
            newEntries = []
        } else if let renderedPatchMetadata,
                  renderedPatchMetadata.startIndex == patch.startIndex
        {
            newEntries = renderedPatchMetadata.entries.map { entry in
                TopLevelChildMetadata(
                    startOffset: replaceRange.location + entry.startOffset,
                    containsAttachment: entry.containsAttachment,
                    containsPositionAdjustments: entry.containsPositionAdjustments
                )
            }
        } else {
            currentTopLevelChildMetadata = nil
            return
        }

        guard patch.startIndex >= 0,
              patch.deleteCount >= 0,
              patch.startIndex <= currentMetadata.count,
              patch.startIndex + patch.deleteCount <= currentMetadata.count
        else {
            currentTopLevelChildMetadata = nil
            return
        }

        currentMetadata.replaceSubrange(
            patch.startIndex..<(patch.startIndex + patch.deleteCount),
            with: newEntries
        )

        let delta = renderedPatchLength - replaceRange.length
        if delta != 0 {
            let shiftStart = patch.startIndex + newEntries.count
            for index in shiftStart..<currentMetadata.count {
                currentMetadata[index].startOffset += delta
            }
        }

        currentTopLevelChildMetadata = currentMetadata
    }

    private func hasTopLevelChildMetadata() -> Bool {
        currentTopLevelChildMetadata != nil
    }

    private func firstCharacterOffset(forTopLevelChildIndex index: Int) -> Int? {
        guard let currentTopLevelChildMetadata,
              index >= 0,
              index < currentTopLevelChildMetadata.count
        else {
            return nil
        }
        return currentTopLevelChildMetadata[index].startOffset
    }

    private func replacementRangeForRenderPatch(
        startIndex: Int,
        deleteCount: Int
    ) -> NSRange? {
        let startLocation: Int
        if let resolvedStart = firstCharacterOffset(forTopLevelChildIndex: startIndex) {
            startLocation = resolvedStart
        } else if deleteCount == 0 {
            startLocation = textStorage.length
        } else {
            return nil
        }

        let endIndexExclusive = startIndex + deleteCount
        let endLocation = firstCharacterOffset(forTopLevelChildIndex: endIndexExclusive)
            ?? textStorage.length
        guard startLocation <= endLocation else { return nil }
        return NSRange(location: startLocation, length: endLocation - startLocation)
    }

    private func applyAttributedRender(
        _ attrStr: NSAttributedString,
        replaceRange: NSRange? = nil,
        usedPatch: Bool,
        positionCacheUpdate: PositionCacheUpdate = .scan
    ) -> ApplyRenderTrace {
        let totalStartedAt = DispatchTime.now().uptimeNanoseconds
        let replaceUtf16Length = replaceRange?.length ?? textStorage.length
        let replacementUtf16Length = attrStr.length
        let shouldUseSmallPatchTextMutation =
            replaceRange != nil && shouldUseSmallPatchTextMutation(for: attrStr, replaceRange: replaceRange)
        isApplyingRustState = true
        let textMutationStartedAt = DispatchTime.now().uptimeNanoseconds
        let beginEditingStartedAt = DispatchTime.now().uptimeNanoseconds
        textStorage.beginEditing()
        let beginEditingNanos = DispatchTime.now().uptimeNanoseconds - beginEditingStartedAt
        var stringMutationNanos: UInt64 = 0
        var attributeMutationNanos: UInt64 = 0
        let previousTextStorageDelegate = textStorage.delegate
        let previousTextViewDelegate = delegate
        textStorage.delegate = nil
        delegate = nil
        defer {
            textStorage.delegate = previousTextStorageDelegate
            delegate = previousTextViewDelegate
        }
        if let replaceRange {
            if shouldUseSmallPatchTextMutation {
                let stringMutationStartedAt = DispatchTime.now().uptimeNanoseconds
                textStorage.replaceCharacters(in: replaceRange, with: attrStr.string)
                stringMutationNanos =
                    DispatchTime.now().uptimeNanoseconds - stringMutationStartedAt
                let destinationRange = NSRange(location: replaceRange.location, length: attrStr.length)
                let attributeMutationStartedAt = DispatchTime.now().uptimeNanoseconds
                applyAttributes(from: attrStr, to: destinationRange)
                attributeMutationNanos =
                    DispatchTime.now().uptimeNanoseconds - attributeMutationStartedAt
            } else {
                let stringMutationStartedAt = DispatchTime.now().uptimeNanoseconds
                textStorage.replaceCharacters(in: replaceRange, with: attrStr)
                stringMutationNanos =
                    DispatchTime.now().uptimeNanoseconds - stringMutationStartedAt
            }
        } else {
            let stringMutationStartedAt = DispatchTime.now().uptimeNanoseconds
            textStorage.setAttributedString(attrStr)
            stringMutationNanos =
                DispatchTime.now().uptimeNanoseconds - stringMutationStartedAt
        }
        let endEditingStartedAt = DispatchTime.now().uptimeNanoseconds
        textStorage.endEditing()
        let endEditingNanos = DispatchTime.now().uptimeNanoseconds - endEditingStartedAt
        let textMutationNanos = DispatchTime.now().uptimeNanoseconds - textMutationStartedAt
        let authorizedTextStartedAt = DispatchTime.now().uptimeNanoseconds
        if let replaceRange,
           replaceRange.location >= 0,
           replaceRange.location + replaceRange.length <= lastAuthorizedTextStorage.length
        {
            lastAuthorizedTextStorage.replaceCharacters(in: replaceRange, with: attrStr.string)
        } else {
            lastAuthorizedTextStorage.setString(attrStr.string)
        }
        let authorizedTextNanos = DispatchTime.now().uptimeNanoseconds - authorizedTextStartedAt
        let cacheInvalidationStartedAt = DispatchTime.now().uptimeNanoseconds
        lastRenderAppliedPatchForTesting = usedPatch
        switch positionCacheUpdate {
        case .plainText:
            guard let replaceRange else {
                PositionBridge.invalidateCache(for: self)
                break
            }
            let patchedPositionCache = PositionBridge.applyPlainTextPatchIfPossible(
                for: self,
                replaceRange: replaceRange,
                replacementText: attrStr.string
            )
            if !patchedPositionCache {
                PositionBridge.invalidateCache(for: self)
            }
        case .attributed:
            guard let replaceRange else {
                PositionBridge.invalidateCache(for: self)
                break
            }
            let patchedPositionCache = PositionBridge.applyAttributedPatchIfPossible(
                for: self,
                replaceRange: replaceRange,
                replacement: attrStr
            )
            if !patchedPositionCache {
                PositionBridge.invalidateCache(for: self)
            }
        case .invalidate:
            PositionBridge.invalidateCache(for: self)
        case .scan:
            let canPatchPositionCache = if let replaceRange {
                replaceRange.location >= 0
                    && !textStorageRangeContainsAttachment(replaceRange)
                    && !attributedStringContainsAttachment(attrStr)
            } else {
                false
            }
            if let replaceRange, canPatchPositionCache {
                let patchedPositionCache: Bool
                if !textStorageRangeContainsPositionAdjustments(replaceRange),
                   !attributedStringContainsPositionAdjustments(attrStr)
                {
                    patchedPositionCache = PositionBridge.applyPlainTextPatchIfPossible(
                        for: self,
                        replaceRange: replaceRange,
                        replacementText: attrStr.string
                    )
                } else {
                    patchedPositionCache = PositionBridge.applyAttributedPatchIfPossible(
                        for: self,
                        replaceRange: replaceRange,
                        replacement: attrStr
                    )
                }

                if !patchedPositionCache {
                    PositionBridge.invalidateCache(for: self)
                }
            } else {
                PositionBridge.invalidateCache(for: self)
            }
        }
        let cacheInvalidationNanos = DispatchTime.now().uptimeNanoseconds - cacheInvalidationStartedAt
        isApplyingRustState = false
        return ApplyRenderTrace(
            totalNanos: DispatchTime.now().uptimeNanoseconds - totalStartedAt,
            replaceUtf16Length: replaceUtf16Length,
            replacementUtf16Length: replacementUtf16Length,
            textMutationNanos: textMutationNanos,
            beginEditingNanos: beginEditingNanos,
            endEditingNanos: endEditingNanos,
            stringMutationNanos: stringMutationNanos,
            attributeMutationNanos: attributeMutationNanos,
            authorizedTextNanos: authorizedTextNanos,
            cacheInvalidationNanos: cacheInvalidationNanos,
            usedSmallPatchTextMutation: shouldUseSmallPatchTextMutation
        )
    }

    private func shouldUseSmallPatchTextMutation(
        for attributedString: NSAttributedString,
        replaceRange: NSRange?
    ) -> Bool {
        attributedString.length > 0
            && attributedString.length <= 512
            && (replaceRange?.length ?? 0) <= 512
            && !attributedStringContainsAttachment(attributedString)
    }

    private func attributesEqualForPatchTrimming(
        _ lhs: [NSAttributedString.Key: Any],
        _ rhs: [NSAttributedString.Key: Any]
    ) -> Bool {
        if let lhsValue = lhs[RenderBridgeAttributes.topLevelChildIndex] as? NSNumber,
           let rhsValue = rhs[RenderBridgeAttributes.topLevelChildIndex] as? NSNumber,
           lhsValue == rhsValue
        {
            return NSDictionary(dictionary: lhs).isEqual(to: rhs)
        }

        var lhsComparable = lhs
        var rhsComparable = rhs
        lhsComparable.removeValue(forKey: RenderBridgeAttributes.topLevelChildIndex)
        rhsComparable.removeValue(forKey: RenderBridgeAttributes.topLevelChildIndex)
        return NSDictionary(dictionary: lhsComparable).isEqual(to: rhsComparable)
    }

    private func applyAttributes(from attributedString: NSAttributedString, to destinationRange: NSRange) {
        guard attributedString.length == destinationRange.length else { return }
        if let uniformAttributes = uniformAttributes(in: attributedString) {
            textStorage.setAttributes(uniformAttributes, range: destinationRange)
            return
        }
        let sourceRange = NSRange(location: 0, length: attributedString.length)
        attributedString.enumerateAttributes(
            in: sourceRange,
            options: [.longestEffectiveRangeNotRequired]
        ) { attrs, range, _ in
            let targetRange = NSRange(location: destinationRange.location + range.location, length: range.length)
            textStorage.setAttributes(attrs, range: targetRange)
        }
    }

    private func uniformAttributes(in attributedString: NSAttributedString) -> [NSAttributedString.Key: Any]? {
        guard attributedString.length > 0 else { return [:] }
        let firstAttributes = attributedString.attributes(at: 0, effectiveRange: nil)
        var isUniform = true
        attributedString.enumerateAttributes(
            in: NSRange(location: 0, length: attributedString.length),
            options: [.longestEffectiveRangeNotRequired]
        ) { attrs, _, stop in
            guard (attrs as NSDictionary).isEqual(firstAttributes) else {
                isUniform = false
                stop.pointee = true
                return
            }
        }
        return isUniform ? firstAttributes : nil
    }

    private func attributedStringContainsAttachment(_ attributedString: NSAttributedString) -> Bool {
        guard attributedString.length > 0 else { return false }
        var hasAttachment = false
        attributedString.enumerateAttribute(
            .attachment,
            in: NSRange(location: 0, length: attributedString.length),
            options: [.longestEffectiveRangeNotRequired]
        ) { value, _, stop in
            if value != nil {
                hasAttachment = true
                stop.pointee = true
            }
        }
        return hasAttachment
    }

    private func attributedStringContainsPositionAdjustments(_ attributedString: NSAttributedString) -> Bool {
        guard attributedString.length > 0 else { return false }
        var hasAdjustments = false
        attributedString.enumerateAttributes(
            in: NSRange(location: 0, length: attributedString.length),
            options: [.longestEffectiveRangeNotRequired]
        ) { attrs, _, stop in
            if attrs[RenderBridgeAttributes.syntheticPlaceholder] as? Bool == true
                || attrs[RenderBridgeAttributes.listMarkerContext] != nil
            {
                hasAdjustments = true
                stop.pointee = true
            }
        }
        return hasAdjustments
    }

    private func attributedStringContainsListMarkerContext(_ attributedString: NSAttributedString) -> Bool {
        guard attributedString.length > 0 else { return false }
        var hasListMarkerContext = false
        attributedString.enumerateAttribute(
            RenderBridgeAttributes.listMarkerContext,
            in: NSRange(location: 0, length: attributedString.length),
            options: [.longestEffectiveRangeNotRequired]
        ) { value, _, stop in
            if value != nil {
                hasListMarkerContext = true
                stop.pointee = true
            }
        }
        return hasListMarkerContext
    }

    private func textStorageRangeContainsPositionAdjustments(_ range: NSRange) -> Bool {
        guard range.length > 0,
              range.location >= 0,
              range.location + range.length <= textStorage.length
        else {
            return false
        }

        var hasAdjustments = false
        textStorage.enumerateAttributes(
            in: range,
            options: [.longestEffectiveRangeNotRequired]
        ) { attrs, _, stop in
            if attrs[RenderBridgeAttributes.syntheticPlaceholder] as? Bool == true
                || attrs[RenderBridgeAttributes.listMarkerContext] != nil
            {
                hasAdjustments = true
                stop.pointee = true
            }
        }
        return hasAdjustments
    }

    private func textStorageRangeContainsListMarkerContext(_ range: NSRange) -> Bool {
        guard range.length > 0,
              range.location >= 0,
              range.location + range.length <= textStorage.length
        else {
            return false
        }

        var hasListMarkerContext = false
        textStorage.enumerateAttribute(
            RenderBridgeAttributes.listMarkerContext,
            in: range,
            options: [.longestEffectiveRangeNotRequired]
        ) { value, _, stop in
            if value != nil {
                hasListMarkerContext = true
                stop.pointee = true
            }
        }
        return hasListMarkerContext
    }

    private func textStorageRangeContainsAttachment(_ range: NSRange) -> Bool {
        guard range.length > 0,
              range.location >= 0,
              range.location + range.length <= textStorage.length
        else {
            return false
        }

        var hasAttachment = false
        textStorage.enumerateAttribute(
            .attachment,
            in: range,
            options: [.longestEffectiveRangeNotRequired]
        ) { value, _, stop in
            if value != nil {
                hasAttachment = true
                stop.pointee = true
            }
        }
        return hasAttachment
    }

    private func topLevelChildrenContainAttachment(
        startIndex: Int,
        deleteCount: Int
    ) -> Bool {
        guard deleteCount > 0,
              let currentTopLevelChildMetadata,
              startIndex >= 0,
              startIndex + deleteCount <= currentTopLevelChildMetadata.count
        else {
            return false
        }
        return currentTopLevelChildMetadata[startIndex..<(startIndex + deleteCount)]
            .contains(where: \.containsAttachment)
    }

    private func topLevelChildrenContainPositionAdjustments(
        startIndex: Int,
        deleteCount: Int
    ) -> Bool {
        guard deleteCount > 0,
              let currentTopLevelChildMetadata,
              startIndex >= 0,
              startIndex + deleteCount <= currentTopLevelChildMetadata.count
        else {
            return false
        }
        return currentTopLevelChildMetadata[startIndex..<(startIndex + deleteCount)]
            .contains(where: \.containsPositionAdjustments)
    }

    private func trimmedAttributedPatch(
        replacing fullReplaceRange: NSRange,
        with replacement: NSAttributedString
    ) -> (replaceRange: NSRange, replacement: NSAttributedString) {
        guard fullReplaceRange.length > 0 else {
            return (fullReplaceRange, replacement)
        }

        let existing = textStorage.attributedSubstring(from: fullReplaceRange)
        let existingString = existing.string as NSString
        let replacementString = replacement.string as NSString
        let sharedLength = min(existing.length, replacement.length)

        var prefix = 0
        while prefix < sharedLength {
            var existingRange = NSRange()
            let existingAttrs = existing.attributes(
                at: prefix,
                longestEffectiveRange: &existingRange,
                in: NSRange(location: prefix, length: sharedLength - prefix)
            )
            var replacementRange = NSRange()
            let replacementAttrs = replacement.attributes(
                at: prefix,
                longestEffectiveRange: &replacementRange,
                in: NSRange(location: prefix, length: sharedLength - prefix)
            )
            guard attributesEqualForPatchTrimming(existingAttrs, replacementAttrs) else { break }
            let runEnd = min(NSMaxRange(existingRange), NSMaxRange(replacementRange), sharedLength)
            while prefix < runEnd,
                  existingString.character(at: prefix) == replacementString.character(at: prefix)
            {
                prefix += 1
            }
            if prefix < runEnd {
                break
            }
        }

        var suffix = 0
        while suffix < (sharedLength - prefix) {
            let existingIndex = existing.length - suffix - 1
            let replacementIndex = replacement.length - suffix - 1
            var existingRange = NSRange()
            let existingAttrs = existing.attributes(
                at: existingIndex,
                longestEffectiveRange: &existingRange,
                in: NSRange(location: prefix, length: existingIndex - prefix + 1)
            )
            var replacementRange = NSRange()
            let replacementAttrs = replacement.attributes(
                at: replacementIndex,
                longestEffectiveRange: &replacementRange,
                in: NSRange(location: prefix, length: replacementIndex - prefix + 1)
            )
            guard attributesEqualForPatchTrimming(existingAttrs, replacementAttrs) else { break }
            let maxComparableLength = min(
                existingIndex - max(existingRange.location, prefix) + 1,
                replacementIndex - max(replacementRange.location, prefix) + 1,
                sharedLength - prefix - suffix
            )
            var matchedLength = 0
            while matchedLength < maxComparableLength,
                  existingString.character(at: existingIndex - matchedLength)
                      == replacementString.character(at: replacementIndex - matchedLength)
            {
                matchedLength += 1
            }
            suffix += matchedLength
            if matchedLength < maxComparableLength {
                break
            }
        }

        guard prefix > 0 || suffix > 0 else {
            return (fullReplaceRange, replacement)
        }

        let trimmedReplaceRange = NSRange(
            location: fullReplaceRange.location + prefix,
            length: fullReplaceRange.length - prefix - suffix
        )
        let trimmedReplacementRange = NSRange(
            location: prefix,
            length: replacement.length - prefix - suffix
        )
        return (
            trimmedReplaceRange,
            replacement.attributedSubstring(from: trimmedReplacementRange)
        )
    }

    private func applyRenderPatchIfPossible(_ patch: ParsedRenderPatch) -> PatchApplyTrace {
        let eligibilityStartedAt = DispatchTime.now().uptimeNanoseconds
        guard hasTopLevelChildMetadata(),
              let fullReplaceRange = replacementRangeForRenderPatch(
                  startIndex: patch.startIndex,
                  deleteCount: patch.deleteCount
              )
        else {
            return PatchApplyTrace(
                applied: false,
                eligibilityNanos: DispatchTime.now().uptimeNanoseconds - eligibilityStartedAt,
                trimNanos: 0,
                metadataNanos: 0,
                buildRenderNanos: 0,
                applyRenderNanos: 0,
                applyRenderReplaceUtf16Length: 0,
                applyRenderReplacementUtf16Length: 0,
                applyRenderTextMutationNanos: 0,
                applyRenderBeginEditingNanos: 0,
                applyRenderEndEditingNanos: 0,
                applyRenderStringMutationNanos: 0,
                applyRenderAttributeMutationNanos: 0,
                applyRenderAuthorizedTextNanos: 0,
                applyRenderCacheInvalidationNanos: 0,
                usedSmallPatchTextMutation: false
            )
        }

        let buildStartedAt = DispatchTime.now().uptimeNanoseconds
        let attrStr = RenderBridge.renderBlocks(
            fromArray: patch.renderBlocks,
            startIndex: patch.startIndex,
            includeLeadingInterBlockSeparator: patch.startIndex > 0,
            baseFont: baseFont,
            textColor: baseTextColor,
            theme: theme
        )
        let buildRenderNanos = DispatchTime.now().uptimeNanoseconds - buildStartedAt
        let renderedPatchMetadata = topLevelChildMetadataSlice(from: attrStr)
        let renderedPatchContainsAttachment =
            renderedPatchMetadata?.entries.contains(where: \.containsAttachment)
            ?? attributedStringContainsAttachment(attrStr)
        let renderedPatchContainsListMarkerContext =
            attributedStringContainsListMarkerContext(attrStr)
        let renderedPatchContainsPositionAdjustments =
            renderedPatchMetadata?.entries.contains(where: \.containsPositionAdjustments)
            ?? attributedStringContainsPositionAdjustments(attrStr)
        guard !topLevelChildrenContainAttachment(
                  startIndex: patch.startIndex,
                  deleteCount: patch.deleteCount
              ),
              !renderedPatchContainsAttachment
        else {
            return PatchApplyTrace(
                applied: false,
                eligibilityNanos: DispatchTime.now().uptimeNanoseconds - eligibilityStartedAt,
                trimNanos: 0,
                metadataNanos: 0,
                buildRenderNanos: buildRenderNanos,
                applyRenderNanos: 0,
                applyRenderReplaceUtf16Length: 0,
                applyRenderReplacementUtf16Length: 0,
                applyRenderTextMutationNanos: 0,
                applyRenderBeginEditingNanos: 0,
                applyRenderEndEditingNanos: 0,
                applyRenderStringMutationNanos: 0,
                applyRenderAttributeMutationNanos: 0,
                applyRenderAuthorizedTextNanos: 0,
                applyRenderCacheInvalidationNanos: 0,
                usedSmallPatchTextMutation: false
            )
        }
        guard !textStorageRangeContainsListMarkerContext(fullReplaceRange),
              !renderedPatchContainsListMarkerContext
        else {
            return PatchApplyTrace(
                applied: false,
                eligibilityNanos: DispatchTime.now().uptimeNanoseconds - eligibilityStartedAt,
                trimNanos: 0,
                metadataNanos: 0,
                buildRenderNanos: buildRenderNanos,
                applyRenderNanos: 0,
                applyRenderReplaceUtf16Length: 0,
                applyRenderReplacementUtf16Length: 0,
                applyRenderTextMutationNanos: 0,
                applyRenderBeginEditingNanos: 0,
                applyRenderEndEditingNanos: 0,
                applyRenderStringMutationNanos: 0,
                applyRenderAttributeMutationNanos: 0,
                applyRenderAuthorizedTextNanos: 0,
                applyRenderCacheInvalidationNanos: 0,
                usedSmallPatchTextMutation: false
            )
        }
        let eligibilityNanos =
            DispatchTime.now().uptimeNanoseconds - eligibilityStartedAt - buildRenderNanos
        let positionCacheUpdate: PositionCacheUpdate =
            if topLevelChildrenContainPositionAdjustments(
                startIndex: patch.startIndex,
                deleteCount: patch.deleteCount
            ) || renderedPatchContainsPositionAdjustments
            {
                .attributed
            } else {
                .plainText
            }
        let trimStartedAt = DispatchTime.now().uptimeNanoseconds
        let patchToApply = trimmedAttributedPatch(replacing: fullReplaceRange, with: attrStr)
        let trimNanos = DispatchTime.now().uptimeNanoseconds - trimStartedAt
        let applyTrace = applyAttributedRender(
            patchToApply.replacement,
            replaceRange: patchToApply.replaceRange,
            usedPatch: true,
            positionCacheUpdate: positionCacheUpdate
        )
        let metadataStartedAt = DispatchTime.now().uptimeNanoseconds
        applyTopLevelChildMetadataPatch(
            patch,
            replaceRange: fullReplaceRange,
            renderedPatchMetadata: renderedPatchMetadata,
            renderedPatchLength: attrStr.length
        )
        let metadataNanos = DispatchTime.now().uptimeNanoseconds - metadataStartedAt
        return PatchApplyTrace(
            applied: true,
            eligibilityNanos: eligibilityNanos,
            trimNanos: trimNanos,
            metadataNanos: metadataNanos,
            buildRenderNanos: buildRenderNanos,
            applyRenderNanos: applyTrace.totalNanos,
            applyRenderReplaceUtf16Length: applyTrace.replaceUtf16Length,
            applyRenderReplacementUtf16Length: applyTrace.replacementUtf16Length,
            applyRenderTextMutationNanos: applyTrace.textMutationNanos,
            applyRenderBeginEditingNanos: applyTrace.beginEditingNanos,
            applyRenderEndEditingNanos: applyTrace.endEditingNanos,
            applyRenderStringMutationNanos: applyTrace.stringMutationNanos,
            applyRenderAttributeMutationNanos: applyTrace.attributeMutationNanos,
            applyRenderAuthorizedTextNanos: applyTrace.authorizedTextNanos,
            applyRenderCacheInvalidationNanos: applyTrace.cacheInvalidationNanos,
            usedSmallPatchTextMutation: applyTrace.usedSmallPatchTextMutation
        )
    }

    /// Apply a full render update from Rust to the text view.
    ///
    /// Parses the update JSON, converts render elements to NSAttributedString
    /// via RenderBridge, and replaces the text view's content.
    ///
    /// - Parameter updateJSON: The JSON string from editor_insert_text, etc.
    func applyUpdateJSON(_ updateJSON: String, notifyDelegate: Bool = true) {
        let totalStartedAt = DispatchTime.now().uptimeNanoseconds
        let parseStartedAt = totalStartedAt
        guard let data = updateJSON.data(using: .utf8),
              let update = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        let parseNanos = DispatchTime.now().uptimeNanoseconds - parseStartedAt

        let renderElements = update["renderElements"] as? [[String: Any]]
        let selectionFromUpdate = (update["selection"] as? [String: Any])
            .map(self.selectionSummary(from:)) ?? "none"
        Self.updateLog.debug(
            "[applyUpdateJSON.begin] renderCount=\(renderElements?.count ?? 0) updateSelection=\(selectionFromUpdate, privacy: .public) before=\(self.textSnapshotSummary(), privacy: .public)"
        )
        let resolveRenderBlocksStartedAt = DispatchTime.now().uptimeNanoseconds
        let renderBlocks = parseRenderBlocks(update["renderBlocks"])
        let explicitRenderPatch = parseRenderPatch(update["renderPatch"])
        let resolvedRenderBlocks = renderBlocks
            ?? explicitRenderPatch.flatMap { patch in
                currentRenderBlocks.flatMap { mergeRenderBlocks(applying: patch, to: $0) }
            }
        let resolveRenderBlocksNanos =
            DispatchTime.now().uptimeNanoseconds - resolveRenderBlocksStartedAt

        let derivedRenderPatch: DerivedRenderPatch? =
            if explicitRenderPatch == nil,
               let currentRenderBlocks,
               let resolvedRenderBlocks
            {
                deriveRenderPatch(from: currentRenderBlocks, to: resolvedRenderBlocks)
            } else {
                nil
            }
        let renderPatch = explicitRenderPatch ?? {
            if case let .patch(patch)? = derivedRenderPatch {
                return patch
            }
            return nil
        }()
        let shouldSkipRender = if case .unchanged? = derivedRenderPatch {
            textStorage.string == lastAuthorizedText
                && lastAppliedRenderAppearanceRevision == renderAppearanceRevision
        } else {
            false
        }

        let patchTrace = renderPatch.map(applyRenderPatchIfPossible)
        let appliedPatch = patchTrace?.applied == true
        var usedSmallPatchTextMutation = patchTrace?.usedSmallPatchTextMutation ?? false
        var applyRenderReplaceUtf16Length = patchTrace?.applyRenderReplaceUtf16Length ?? 0
        var applyRenderReplacementUtf16Length =
            patchTrace?.applyRenderReplacementUtf16Length ?? 0
        var buildRenderNanos = patchTrace?.buildRenderNanos ?? 0
        var applyRenderNanos = patchTrace?.applyRenderNanos ?? 0
        var applyRenderTextMutationNanos = patchTrace?.applyRenderTextMutationNanos ?? 0
        var applyRenderBeginEditingNanos = patchTrace?.applyRenderBeginEditingNanos ?? 0
        var applyRenderEndEditingNanos = patchTrace?.applyRenderEndEditingNanos ?? 0
        var applyRenderStringMutationNanos = patchTrace?.applyRenderStringMutationNanos ?? 0
        var applyRenderAttributeMutationNanos =
            patchTrace?.applyRenderAttributeMutationNanos ?? 0
        var applyRenderAuthorizedTextNanos = patchTrace?.applyRenderAuthorizedTextNanos ?? 0
        var applyRenderCacheInvalidationNanos = patchTrace?.applyRenderCacheInvalidationNanos ?? 0
        if shouldSkipRender {
            lastRenderAppliedPatchForTesting = false
            if let resolvedRenderBlocks {
                currentRenderBlocks = resolvedRenderBlocks
            }
        } else if !appliedPatch {
            let buildStartedAt = DispatchTime.now().uptimeNanoseconds
            let attrStr: NSAttributedString
            if let resolvedRenderBlocks {
                attrStr = RenderBridge.renderBlocks(
                    fromArray: resolvedRenderBlocks,
                    baseFont: baseFont,
                    textColor: baseTextColor,
                    theme: theme
                )
                currentRenderBlocks = resolvedRenderBlocks
            } else if let renderElements {
                attrStr = RenderBridge.renderElements(
                    fromArray: renderElements,
                    baseFont: baseFont,
                    textColor: baseTextColor,
                    theme: theme
                )
                currentRenderBlocks = nil
            } else {
                return
            }
            buildRenderNanos = DispatchTime.now().uptimeNanoseconds - buildStartedAt
            let applyTrace = applyAttributedRender(
                attrStr,
                usedPatch: false,
                positionCacheUpdate: .invalidate
            )
            refreshTopLevelChildMetadata(from: attrStr)
            applyRenderReplaceUtf16Length = applyTrace.replaceUtf16Length
            applyRenderReplacementUtf16Length = applyTrace.replacementUtf16Length
            applyRenderNanos = applyTrace.totalNanos
            applyRenderTextMutationNanos = applyTrace.textMutationNanos
            applyRenderBeginEditingNanos = applyTrace.beginEditingNanos
            applyRenderEndEditingNanos = applyTrace.endEditingNanos
            applyRenderStringMutationNanos = applyTrace.stringMutationNanos
            applyRenderAttributeMutationNanos = applyTrace.attributeMutationNanos
            applyRenderAuthorizedTextNanos = applyTrace.authorizedTextNanos
            applyRenderCacheInvalidationNanos = applyTrace.cacheInvalidationNanos
            usedSmallPatchTextMutation = applyTrace.usedSmallPatchTextMutation
            lastAppliedRenderAppearanceRevision = renderAppearanceRevision
        } else if let resolvedRenderBlocks {
            currentRenderBlocks = resolvedRenderBlocks
            lastAppliedRenderAppearanceRevision = renderAppearanceRevision
        }

        refreshPlaceholderVisibility()
        Self.updateLog.debug(
            "[applyUpdateJSON.rendered] mode=\(appliedPatch ? "patch" : "full", privacy: .public) after=\(self.textSnapshotSummary(), privacy: .public)"
        )

        // Apply the selection from the update.
        let selectionTrace: SelectionApplyTrace
        if let selection = update["selection"] as? [String: Any] {
            selectionTrace = applySelectionFromJSON(selection)
        } else {
            selectionTrace = SelectionApplyTrace(
                totalNanos: 0,
                resolveNanos: 0,
                assignmentNanos: 0,
                chromeNanos: 0
            )
        }
        let postApplyTrace = performPostApplyMaintenance()
        let postApplyNanos = postApplyTrace.totalNanos

        if captureApplyUpdateTraceForTesting {
            lastApplyUpdateTraceForTesting = ApplyUpdateTrace(
                attemptedPatch: renderPatch != nil,
                usedPatch: appliedPatch,
                usedSmallPatchTextMutation: usedSmallPatchTextMutation,
                applyRenderReplaceUtf16Length: applyRenderReplaceUtf16Length,
                applyRenderReplacementUtf16Length: applyRenderReplacementUtf16Length,
                parseNanos: parseNanos,
                resolveRenderBlocksNanos: resolveRenderBlocksNanos,
                patchEligibilityNanos: patchTrace?.eligibilityNanos ?? 0,
                patchTrimNanos: patchTrace?.trimNanos ?? 0,
                patchMetadataNanos: patchTrace?.metadataNanos ?? 0,
                buildRenderNanos: buildRenderNanos,
                applyRenderNanos: applyRenderNanos,
                selectionNanos: selectionTrace.totalNanos,
                postApplyNanos: postApplyNanos,
                totalNanos: DispatchTime.now().uptimeNanoseconds - totalStartedAt,
                applyRenderTextMutationNanos: applyRenderTextMutationNanos,
                applyRenderBeginEditingNanos: applyRenderBeginEditingNanos,
                applyRenderEndEditingNanos: applyRenderEndEditingNanos,
                applyRenderStringMutationNanos: applyRenderStringMutationNanos,
                applyRenderAttributeMutationNanos: applyRenderAttributeMutationNanos,
                applyRenderAuthorizedTextNanos: applyRenderAuthorizedTextNanos,
                applyRenderCacheInvalidationNanos: applyRenderCacheInvalidationNanos,
                selectionResolveNanos: selectionTrace.resolveNanos,
                selectionAssignmentNanos: selectionTrace.assignmentNanos,
                selectionChromeNanos: selectionTrace.chromeNanos,
                postApplyTypingAttributesNanos: postApplyTrace.typingAttributesNanos,
                postApplyHeightNotifyNanos: postApplyTrace.heightNotifyNanos,
                postApplyHeightNotifyMeasureNanos: postApplyTrace.heightNotifyMeasureNanos,
                postApplyHeightNotifyCallbackNanos: postApplyTrace.heightNotifyCallbackNanos,
                postApplyHeightNotifyEnsureLayoutNanos: postApplyTrace.heightNotifyEnsureLayoutNanos,
                postApplyHeightNotifyUsedRectNanos: postApplyTrace.heightNotifyUsedRectNanos,
                postApplyHeightNotifyContentSizeNanos: postApplyTrace.heightNotifyContentSizeNanos,
                postApplyHeightNotifySizeThatFitsNanos: postApplyTrace.heightNotifySizeThatFitsNanos,
                postApplySelectionOrContentCallbackNanos:
                    postApplyTrace.selectionOrContentCallbackNanos
            )
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
        _ = applyAttributedRender(attrStr, usedPatch: false)
        currentRenderBlocks = nil
        lastAppliedRenderAppearanceRevision = renderAppearanceRevision

        refreshPlaceholderVisibility()
        _ = performPostApplyMaintenance()
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
    private func applySelectionFromJSON(_ selection: [String: Any]) -> SelectionApplyTrace {
        guard let type = selection["type"] as? String else {
            return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
        }

        let totalStartedAt = DispatchTime.now().uptimeNanoseconds
        isApplyingRustState = true
        defer { isApplyingRustState = false }

        switch type {
        case "text":
            let resolveStartedAt = DispatchTime.now().uptimeNanoseconds
            guard let anchorNum = selection["anchor"] as? NSNumber,
                  let headNum = selection["head"] as? NSNumber
            else {
                return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
            }
            // anchor/head from Rust are document positions; convert to scalar offsets first.
            let anchorScalar = (selection["anchorScalar"] as? NSNumber)?.uint32Value
                ?? editorDocToScalar(id: editorId, docPos: anchorNum.uint32Value)
            let headScalar = (selection["headScalar"] as? NSNumber)?.uint32Value
                ?? editorDocToScalar(id: editorId, docPos: headNum.uint32Value)
            let startUtf16 = PositionBridge.scalarToUtf16Offset(
                min(anchorScalar, headScalar),
                in: self
            )
            let endUtf16 = PositionBridge.scalarToUtf16Offset(
                max(anchorScalar, headScalar),
                in: self
            )
            let resolveNanos = DispatchTime.now().uptimeNanoseconds - resolveStartedAt

            let assignmentStartedAt = DispatchTime.now().uptimeNanoseconds
            if anchorScalar == headScalar {
                let endPos = position(from: beginningOfDocument, offset: endUtf16) ?? endOfDocument
                if let adjustedPosition = autocapitalizationFriendlyEmptyBlockPosition(for: endPos) {
                    let adjustedOffset = offset(from: beginningOfDocument, to: adjustedPosition)
                    let adjustedRange = NSRange(location: adjustedOffset, length: 0)
                    if selectedRange != adjustedRange {
                        selectedRange = adjustedRange
                    }
                } else {
                    let targetRange = NSRange(location: endUtf16, length: 0)
                    if selectedRange != targetRange {
                        selectedRange = targetRange
                    }
                }
            } else {
                let targetRange = NSRange(location: startUtf16, length: endUtf16 - startUtf16)
                if selectedRange != targetRange {
                    selectedRange = targetRange
                }
            }
            let assignmentNanos = DispatchTime.now().uptimeNanoseconds - assignmentStartedAt
            let chromeStartedAt = DispatchTime.now().uptimeNanoseconds
            showNativeSelectionChromeIfNeeded()
            let chromeNanos = DispatchTime.now().uptimeNanoseconds - chromeStartedAt
            Self.selectionLog.debug(
                "[applySelectionFromJSON.text] doc=\(anchorNum.uint32Value)-\(headNum.uint32Value) scalar=\(anchorScalar)-\(headScalar) final=\(self.selectionSummary(), privacy: .public)"
            )
            return SelectionApplyTrace(
                totalNanos: DispatchTime.now().uptimeNanoseconds - totalStartedAt,
                resolveNanos: resolveNanos,
                assignmentNanos: assignmentNanos,
                chromeNanos: chromeNanos
            )

        case "node":
            // Node selection: select the object replacement character at that position.
            let resolveStartedAt = DispatchTime.now().uptimeNanoseconds
            guard let posNum = selection["pos"] as? NSNumber else {
                return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
            }
            // pos from Rust is a document position; convert to scalar offset.
            let posScalar = (selection["posScalar"] as? NSNumber)?.uint32Value
                ?? editorDocToScalar(id: editorId, docPos: posNum.uint32Value)
            let startUtf16 = PositionBridge.scalarToUtf16Offset(posScalar, in: self)
            let targetRange = NSRange(location: startUtf16, length: 1)
            let resolveNanos = DispatchTime.now().uptimeNanoseconds - resolveStartedAt
            let assignmentStartedAt = DispatchTime.now().uptimeNanoseconds
            if selectedRange != targetRange {
                selectedRange = targetRange
            }
            let assignmentNanos = DispatchTime.now().uptimeNanoseconds - assignmentStartedAt
            let chromeStartedAt = DispatchTime.now().uptimeNanoseconds
            refreshNativeSelectionChromeVisibility()
            let chromeNanos = DispatchTime.now().uptimeNanoseconds - chromeStartedAt
            Self.selectionLog.debug(
                "[applySelectionFromJSON.node] doc=\(posNum.uint32Value) scalar=\(posScalar) final=\(self.selectionSummary(), privacy: .public)"
            )
            return SelectionApplyTrace(
                totalNanos: DispatchTime.now().uptimeNanoseconds - totalStartedAt,
                resolveNanos: resolveNanos,
                assignmentNanos: assignmentNanos,
                chromeNanos: chromeNanos
            )

        case "all":
            let assignmentStartedAt = DispatchTime.now().uptimeNanoseconds
            selectedTextRange = textRange(from: beginningOfDocument, to: endOfDocument)
            let assignmentNanos = DispatchTime.now().uptimeNanoseconds - assignmentStartedAt
            let chromeStartedAt = DispatchTime.now().uptimeNanoseconds
            showNativeSelectionChromeIfNeeded()
            let chromeNanos = DispatchTime.now().uptimeNanoseconds - chromeStartedAt
            Self.selectionLog.debug(
                "[applySelectionFromJSON.all] final=\(self.selectionSummary(), privacy: .public)"
            )
            return SelectionApplyTrace(
                totalNanos: DispatchTime.now().uptimeNanoseconds - totalStartedAt,
                resolveNanos: 0,
                assignmentNanos: assignmentNanos,
                chromeNanos: chromeNanos
            )

        default:
            return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
        }
    }

    private func autocapitalizationFriendlyEmptyBlockPosition(
        for position: UITextPosition
    ) -> UITextPosition? {
        guard textStorage.length == 1 else { return nil }
        guard textStorage.string.unicodeScalars.elementsEqual([Self.emptyBlockPlaceholderScalar]) else {
            return nil
        }

        let utf16Offset = offset(from: beginningOfDocument, to: position)
        guard utf16Offset == textStorage.length else { return nil }
        return beginningOfDocument
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
        currentTopLevelChildMetadata = nil
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

    struct HostedLayoutTrace {
        let intrinsicContentSizeNanos: UInt64
        let intrinsicContentSizeCount: Int
        let measuredEditorHeightNanos: UInt64
        let measuredEditorHeightCount: Int
        let layoutSubviewsNanos: UInt64
        let layoutSubviewsCount: Int
        let refreshOverlaysNanos: UInt64
        let refreshOverlaysCount: Int
        let overlayScheduleRequestCount: Int
        let overlayScheduleExecuteCount: Int
        let overlayScheduleSkipCount: Int
        let onHeightMayChangeNanos: UInt64
        let onHeightMayChangeCount: Int
    }

    // MARK: - Properties

    /// The editor text view that handles input interception.
    let textView: EditorTextView
    private let remoteSelectionOverlayView = RemoteSelectionOverlayView()
    private let imageTapOverlayView = ImageTapOverlayView()
    private let imageResizeOverlayView = ImageResizeOverlayView()
    var onHeightMayChange: ((CGFloat) -> Void)?
    private var lastAutoGrowWidth: CGFloat = 0
    private var cachedAutoGrowMeasuredHeight: CGFloat = 0
    private var remoteSelections: [RemoteSelectionDecoration] = []
    private var overlayRefreshScheduled = false
    var captureHostedLayoutTraceForTesting = false
    private var hostedLayoutTraceNanos = (
        intrinsicContentSize: UInt64(0),
        measuredEditorHeight: UInt64(0),
        layoutSubviews: UInt64(0),
        refreshOverlays: UInt64(0),
        onHeightMayChange: UInt64(0)
    )
    private var hostedLayoutTraceCounts = (
        intrinsicContentSize: 0,
        measuredEditorHeight: 0,
        layoutSubviews: 0,
        refreshOverlays: 0,
        overlayScheduleRequest: 0,
        overlayScheduleExecute: 0,
        overlayScheduleSkip: 0,
        onHeightMayChange: 0
    )
    var allowImageResizing = true {
        didSet {
            guard oldValue != allowImageResizing else { return }
            textView.allowImageResizing = allowImageResizing
            textView.refreshSelectionVisualState()
            imageTapOverlayView.isHidden = editorId == 0 || !allowImageResizing
            imageResizeOverlayView.refresh()
        }
    }

    var heightBehavior: EditorHeightBehavior = .fixed {
        didSet {
            guard oldValue != heightBehavior else { return }
            textView.heightBehavior = heightBehavior
            textView.updateAutoGrowHostHeight(heightBehavior == .autoGrow ? bounds.height : 0)
            if heightBehavior != .autoGrow {
                cachedAutoGrowMeasuredHeight = 0
            }
            invalidateIntrinsicContentSize()
            setNeedsLayout()
            if heightBehavior == .autoGrow {
                let measuredHeight = measuredEditorHeight()
                if measuredHeight > 0 {
                    cachedAutoGrowMeasuredHeight = measuredHeight
                    onHeightMayChange?(measuredHeight)
                } else {
                    onHeightMayChange?(0)
                }
            } else {
                onHeightMayChange?(0)
            }
            remoteSelectionOverlayView.refresh()
            imageResizeOverlayView.refresh()
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
            imageTapOverlayView.isHidden = editorId == 0 || !allowImageResizing
            imageResizeOverlayView.refresh()
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
        // Add the text view as a subview. These views always track the host bounds,
        // so manual layout is cheaper than driving them through Auto Layout.
        remoteSelectionOverlayView.bind(textView: textView)
        imageTapOverlayView.bind(editorView: self)
        imageResizeOverlayView.bind(editorView: self)
        textView.allowImageResizing = allowImageResizing
        imageTapOverlayView.isHidden = editorId == 0 || !allowImageResizing
        textView.onHeightMayChange = { [weak self] measuredHeight in
            guard let self, self.heightBehavior == .autoGrow else { return }
            let startedAt = DispatchTime.now().uptimeNanoseconds
            self.cachedAutoGrowMeasuredHeight = measuredHeight
            self.invalidateIntrinsicContentSize()
            self.onHeightMayChange?(measuredHeight)
            self.recordHostedLayoutTrace(
                durationNanos: DispatchTime.now().uptimeNanoseconds - startedAt,
                keyPath: .onHeightMayChange
            )
        }
        textView.onViewportMayChange = { [weak self] in
            self?.refreshOverlaysIfNeeded()
        }
        textView.onSelectionOrContentMayChange = { [weak self] in
            self?.scheduleRefreshOverlaysIfNeeded()
        }
        addSubview(textView)
        addSubview(remoteSelectionOverlayView)
        addSubview(imageTapOverlayView)
        addSubview(imageResizeOverlayView)
        layoutManagedSubviews()
    }

    override var intrinsicContentSize: CGSize {
        let startedAt = DispatchTime.now().uptimeNanoseconds
        defer {
            recordHostedLayoutTrace(
                durationNanos: DispatchTime.now().uptimeNanoseconds - startedAt,
                keyPath: .intrinsicContentSize
            )
        }
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
        let startedAt = DispatchTime.now().uptimeNanoseconds
        defer {
            recordHostedLayoutTrace(
                durationNanos: DispatchTime.now().uptimeNanoseconds - startedAt,
                keyPath: .layoutSubviews
            )
        }
        super.layoutSubviews()
        layoutManagedSubviews()
        refreshOverlaysIfNeeded()
        guard heightBehavior == .autoGrow else { return }
        textView.updateAutoGrowHostHeight(bounds.height)
        let currentWidth = bounds.width.rounded(.towardZero)
        guard currentWidth != lastAutoGrowWidth else { return }
        lastAutoGrowWidth = currentWidth
        cachedAutoGrowMeasuredHeight = 0
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
        refreshOverlays()
    }

    func setRemoteSelections(_ selections: [RemoteSelectionDecoration]) {
        remoteSelections = selections
        remoteSelectionOverlayView.update(
            selections: selections,
            editorId: editorId
        )
    }

    func refreshRemoteSelections() {
        guard remoteSelectionOverlayView.hasSelectionsOrVisibleDecorations else { return }
        remoteSelectionOverlayView.refresh()
    }

    func remoteSelectionOverlaySubviewsForTesting() -> [UIView] {
        remoteSelectionOverlayView.subviews.filter { !$0.isHidden }
    }

    func resetHostedLayoutTraceForTesting() {
        hostedLayoutTraceNanos = (
            intrinsicContentSize: 0,
            measuredEditorHeight: 0,
            layoutSubviews: 0,
            refreshOverlays: 0,
            onHeightMayChange: 0
        )
        hostedLayoutTraceCounts = (
            intrinsicContentSize: 0,
            measuredEditorHeight: 0,
            layoutSubviews: 0,
            refreshOverlays: 0,
            overlayScheduleRequest: 0,
            overlayScheduleExecute: 0,
            overlayScheduleSkip: 0,
            onHeightMayChange: 0
        )
    }

    func lastHostedLayoutTraceForTesting() -> HostedLayoutTrace {
        HostedLayoutTrace(
            intrinsicContentSizeNanos: hostedLayoutTraceNanos.intrinsicContentSize,
            intrinsicContentSizeCount: hostedLayoutTraceCounts.intrinsicContentSize,
            measuredEditorHeightNanos: hostedLayoutTraceNanos.measuredEditorHeight,
            measuredEditorHeightCount: hostedLayoutTraceCounts.measuredEditorHeight,
            layoutSubviewsNanos: hostedLayoutTraceNanos.layoutSubviews,
            layoutSubviewsCount: hostedLayoutTraceCounts.layoutSubviews,
            refreshOverlaysNanos: hostedLayoutTraceNanos.refreshOverlays,
            refreshOverlaysCount: hostedLayoutTraceCounts.refreshOverlays,
            overlayScheduleRequestCount: hostedLayoutTraceCounts.overlayScheduleRequest,
            overlayScheduleExecuteCount: hostedLayoutTraceCounts.overlayScheduleExecute,
            overlayScheduleSkipCount: hostedLayoutTraceCounts.overlayScheduleSkip,
            onHeightMayChangeNanos: hostedLayoutTraceNanos.onHeightMayChange,
            onHeightMayChangeCount: hostedLayoutTraceCounts.onHeightMayChange
        )
    }

    func imageResizeOverlayRectForTesting() -> CGRect? {
        imageResizeOverlayView.visibleRectForTesting
    }

    func imageTapOverlayInterceptsPointForTesting(_ point: CGPoint) -> Bool {
        imageTapOverlayView.interceptsPointForTesting(convert(point, to: imageTapOverlayView))
    }

    @discardableResult
    func tapImageOverlayForTesting(at point: CGPoint) -> Bool {
        imageTapOverlayView.handleTapForTesting(convert(point, to: imageTapOverlayView))
    }

    func imageResizePreviewHasImageForTesting() -> Bool {
        imageResizeOverlayView.previewHasImageForTesting
    }

    func refreshSelectionVisualStateForTesting() {
        textView.refreshSelectionVisualState()
    }

    func imageResizeOverlayInterceptsPointForTesting(_ point: CGPoint) -> Bool {
        imageResizeOverlayView.interceptsPointForTesting(convert(point, to: imageResizeOverlayView))
    }

    func maximumImageWidthForTesting() -> CGFloat {
        textView.maximumRenderableImageWidth()
    }

    func resizeSelectedImageForTesting(width: CGFloat, height: CGFloat) {
        imageResizeOverlayView.simulateResizeForTesting(width: width, height: height)
    }

    func previewResizeSelectedImageForTesting(width: CGFloat, height: CGFloat) {
        imageResizeOverlayView.simulatePreviewResizeForTesting(width: width, height: height)
    }

    func commitPreviewResizeForTesting() {
        imageResizeOverlayView.commitPreviewResizeForTesting()
    }

    /// Set initial content from HTML.
    ///
    /// - Parameter html: The HTML string to load.
    func setContent(html: String) {
        guard editorId != 0 else { return }
        _ = editorSetHtml(id: editorId, html: html)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
    }

    /// Set initial content from ProseMirror JSON.
    ///
    /// - Parameter json: The JSON string to load.
    func setContent(json: String) {
        guard editorId != 0 else { return }
        _ = editorSetJson(id: editorId, json: json)
        textView.applyUpdateJSON(editorGetCurrentState(id: editorId), notifyDelegate: false)
    }

    private func measuredEditorHeight() -> CGFloat {
        let startedAt = DispatchTime.now().uptimeNanoseconds
        defer {
            recordHostedLayoutTrace(
                durationNanos: DispatchTime.now().uptimeNanoseconds - startedAt,
                keyPath: .measuredEditorHeight
            )
        }
        if cachedAutoGrowMeasuredHeight > 0 {
            return cachedAutoGrowMeasuredHeight
        }
        let width = resolvedMeasurementWidth()
        guard width > 0 else { return 0 }
        let measuredHeight = textView.measuredAutoGrowHeightForTesting(width: width)
        if measuredHeight > 0 {
            cachedAutoGrowMeasuredHeight = measuredHeight
        }
        return measuredHeight
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

    private func layoutManagedSubviews() {
        let managedFrame = bounds
        if textView.frame != managedFrame {
            textView.frame = managedFrame
        }
        if remoteSelectionOverlayView.frame != managedFrame {
            remoteSelectionOverlayView.frame = managedFrame
        }
        if imageTapOverlayView.frame != managedFrame {
            imageTapOverlayView.frame = managedFrame
        }
        if imageResizeOverlayView.frame != managedFrame {
            imageResizeOverlayView.frame = managedFrame
        }
    }

    fileprivate func selectedImageGeometry() -> (docPos: UInt32, rect: CGRect)? {
        guard let geometry = textView.selectedImageGeometry() else { return nil }
        return (
            docPos: geometry.docPos,
            rect: textView.convert(geometry.rect, to: imageResizeOverlayView)
        )
    }

    fileprivate func setImageResizePreviewActive(_ active: Bool) {
        textView.setImageResizePreviewActive(active)
    }

    fileprivate func imagePreviewForResize(docPos: UInt32) -> UIImage? {
        textView.imagePreviewForDocPos(docPos)
    }

    fileprivate func imageResizePreviewBackgroundColor() -> UIColor {
        textView.backgroundColor ?? .systemBackground
    }

    fileprivate func maximumImageWidthForResizeGesture() -> CGFloat {
        textView.maximumRenderableImageWidth()
    }

    fileprivate func clampedImageSize(_ size: CGSize, maximumWidth: CGFloat? = nil) -> CGSize {
        let aspectRatio = max(size.width / max(size.height, 1), 0.1)
        let maxWidth = max(48, maximumWidth ?? textView.maximumRenderableImageWidth())
        let clampedWidth = min(maxWidth, max(48, size.width))
        let clampedHeight = max(48, clampedWidth / aspectRatio)
        return CGSize(width: clampedWidth, height: clampedHeight)
    }

    fileprivate func resizeImage(docPos: UInt32, size: CGSize) {
        let clampedSize = clampedImageSize(size)
        let width = max(48, Int(clampedSize.width.rounded()))
        let height = max(48, Int(clampedSize.height.rounded()))
        textView.resizeImageAtDocPos(docPos, width: UInt32(width), height: UInt32(height))
    }

    private func refreshOverlays() {
        let startedAt = DispatchTime.now().uptimeNanoseconds
        defer {
            recordHostedLayoutTrace(
                durationNanos: DispatchTime.now().uptimeNanoseconds - startedAt,
                keyPath: .refreshOverlays
            )
        }
        remoteSelectionOverlayView.refresh()
        imageResizeOverlayView.refresh()
    }

    private func refreshOverlaysIfNeeded() {
        guard shouldRefreshOverlays() else { return }
        refreshOverlays()
    }

    private func scheduleRefreshOverlaysIfNeeded() {
        if !shouldRefreshOverlays() {
            if captureHostedLayoutTraceForTesting {
                hostedLayoutTraceCounts.overlayScheduleSkip += 1
            }
            return
        }
        scheduleRefreshOverlays()
    }

    private func scheduleRefreshOverlays() {
        if captureHostedLayoutTraceForTesting {
            hostedLayoutTraceCounts.overlayScheduleRequest += 1
        }
        guard !overlayRefreshScheduled else { return }
        overlayRefreshScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.overlayRefreshScheduled = false
            if self.captureHostedLayoutTraceForTesting {
                self.hostedLayoutTraceCounts.overlayScheduleExecute += 1
            }
            self.refreshOverlays()
        }
    }

    private func shouldRefreshOverlays() -> Bool {
        if !remoteSelections.isEmpty || remoteSelectionOverlayView.hasVisibleDecorations {
            return true
        }
        if imageResizeOverlayView.isOverlayVisible {
            return true
        }
        if textView.selectedImageGeometry() != nil {
            return true
        }
        return false
    }

    private enum HostedLayoutTraceKey {
        case intrinsicContentSize
        case measuredEditorHeight
        case layoutSubviews
        case refreshOverlays
        case onHeightMayChange
    }

    private func recordHostedLayoutTrace(durationNanos: UInt64, keyPath: HostedLayoutTraceKey) {
        guard captureHostedLayoutTraceForTesting else { return }
        switch keyPath {
        case .intrinsicContentSize:
            hostedLayoutTraceNanos.intrinsicContentSize += durationNanos
            hostedLayoutTraceCounts.intrinsicContentSize += 1
        case .measuredEditorHeight:
            hostedLayoutTraceNanos.measuredEditorHeight += durationNanos
            hostedLayoutTraceCounts.measuredEditorHeight += 1
        case .layoutSubviews:
            hostedLayoutTraceNanos.layoutSubviews += durationNanos
            hostedLayoutTraceCounts.layoutSubviews += 1
        case .refreshOverlays:
            hostedLayoutTraceNanos.refreshOverlays += durationNanos
            hostedLayoutTraceCounts.refreshOverlays += 1
        case .onHeightMayChange:
            hostedLayoutTraceNanos.onHeightMayChange += durationNanos
            hostedLayoutTraceCounts.onHeightMayChange += 1
        }
    }

    // MARK: - Cleanup

    deinit {
        if editorId != 0 {
            textView.unbindEditor()
        }
    }
}
