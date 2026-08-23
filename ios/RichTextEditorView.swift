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

    func editorTextView(_ textView: EditorTextView, didEndExternalTextComposition resultJSON: String)
}

extension EditorTextViewDelegate {
    func editorTextView(_ textView: EditorTextView, didEndExternalTextComposition resultJSON: String) {}
}

enum EditorHeightBehavior: String {
    case fixed
    case autoGrow
}

struct RemoteSelectionDecoration {
    let clientId: String
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
            guard let clientId = item["clientId"] as? String,
                  !clientId.isEmpty,
                  clientId.allSatisfy({ $0 >= "0" && $0 <= "9" }),
                  (clientId == "0" || clientId.first != "0"),
                  UInt64(clientId) != nil,
                  let anchor = v2ExactUInt32(item["anchor"] as? NSNumber),
                  let head = v2ExactUInt32(item["head"] as? NSNumber),
                  let colorRaw = item["color"] as? String,
                  let color = colorFromString(colorRaw)
            else {
                return nil
            }

            return RemoteSelectionDecoration(
                clientId: clientId,
                anchor: anchor,
                head: head,
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
        let startScalar = EditorV2Shadow.docToScalar(
            id: editorId,
            docPos: min(selection.anchor, selection.head)
        )
        let endScalar = EditorV2Shadow.docToScalar(
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

private final class TaskListMarkerTapOverlayView: UIView {
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
        return editorView.textView.hasTaskListMarker(at: pointInTextView)
    }

    @objc
    private func handleTap(_ recognizer: UITapGestureRecognizer) {
        guard recognizer.state == .ended, let editorView else { return }
        let pointInTextView = convert(recognizer.location(in: self), to: editorView.textView)
        _ = editorView.textView.toggleTaskListMarker(at: pointInTextView)
    }

    func interceptsPointForTesting(_ point: CGPoint) -> Bool {
        self.point(inside: point, with: nil)
    }

    @discardableResult
    func handleTapForTesting(_ point: CGPoint) -> Bool {
        guard let editorView else { return false }
        let pointInTextView = convert(point, to: editorView.textView)
        return editorView.textView.toggleTaskListMarker(at: pointInTextView)
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

/// Dedicated `UITextViewDelegate`, because the editor must not be its own.
///
/// Proxy keyboard integrations forward unimplemented selectors via
/// `forwardingTarget(for:)`, so UIKit's private `keyboardInputChangedSelection:`
/// bounces view -> proxy -> view until the stack overflows (APOLLO-REACT-56).
/// A plain NSObject does not respond to those private selectors.
final class EditorTextViewInternalDelegate: NSObject, UITextViewDelegate {
    private weak var editor: EditorTextView?

    init(editor: EditorTextView) {
        self.editor = editor
    }

    func textViewDidChangeSelection(_ textView: UITextView) {
        editor?.textViewDidChangeSelection(textView)
    }

    func textView(
        _ textView: UITextView,
        shouldInteractWith URL: URL,
        in characterRange: NSRange,
        interaction: UITextItemInteraction
    ) -> Bool {
        editor?.textView(textView, shouldInteractWith: URL, in: characterRange, interaction: interaction) ?? false
    }

    func textView(
        _ textView: UITextView,
        shouldInteractWith textAttachment: NSTextAttachment,
        in characterRange: NSRange,
        interaction: UITextItemInteraction
    ) -> Bool {
        editor?.textView(textView, shouldInteractWith: textAttachment, in: characterRange, interaction: interaction)
            ?? false
    }
}

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
final class EditorTextView: UITextView, UIGestureRecognizerDelegate {
    private static let emptyBlockPlaceholderScalar = UnicodeScalar(0x200B)!

    private lazy var internalTextViewDelegate = EditorTextViewInternalDelegate(editor: self)
    var imageLoadOwner: RenderImageLoadOwner?

    private func withImageLoadOwner<T>(_ body: () -> T) -> T {
        guard let imageLoadOwner else { return body() }
        return imageLoadOwner.withCurrent(body)
    }

    func imageLoadingPolicyDidChange() {
        renderAppearanceRevision &+= 1
    }

    override var undoManager: UndoManager? { nil }

    struct ApplyUpdateTrace {
        let attemptedPatch: Bool
        let patchStartIndex: Int?
        let patchDeleteCount: Int?
        let patchRenderBlockCount: Int?
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

    private struct NativeTextMutation {
        let from: UInt32
        let to: UInt32
        /// The replaced span as UTF-16 in the authorized storage. Kept so a
        /// caret can be converted against that storage patched with this
        /// replacement, which is the only ruler that still carries the
        /// document's structural attributes.
        let authorizedReplacementUtf16Range: NSRange
        let replacementText: String
        let resultingText: String
        let authorizedText: String
        let selectionAnchor: UInt32?
        let selectionHead: UInt32?
        let authorizedSelectionUtf16Range: NSRange?
        let rawSelectionUtf16Range: NSRange?
        let acceptedSpaceCaretUtf16Offset: Int?
        let selectionRevision: UInt64
        let capturedWhileFirstResponder: Bool
        let capturedWhileEditable: Bool
        let capturedAfterBlur: Bool
        let inputGeneration: UInt64
    }

    private enum NativeTextMutationCommitResult {
        case committed(adoptedUpdateJSON: String)
        case deferred
        case rejected
    }

    private struct NativeTextMutationDrainResult {
        let ready: Bool
        let adoptedUpdateJSON: String?
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
    private let nativeBindingToken = UUID()

    func ownsNativeBinding(_ adapter: EditorV2Adapter) -> Bool {
        adapter.isNativeBindingOwner(token: nativeBindingToken)
    }

    /// Guard flag to prevent re-entrant input interception while we're
    /// applying state from Rust (calling replaceCharacters on the text storage).
    var isApplyingRustState = false
    private var visibleSelectionTintColor: UIColor = .systemBlue
    private var hidesNativeSelectionChrome = false
    private var isPreviewingImageResize = false
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
    private var lastAuthorizedAttributedTextStorage = NSMutableAttributedString()
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
    var hasPendingCompositionForExternalRefresh: Bool { isComposing }
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
    private var nativeTextMutationCommitScheduled = false
    private var pendingNativeTextMutation: NativeTextMutation?
    private var nativeTextMutationGeneration: UInt64 = 0
    private var nativeTextMutationAfterBlurDeadline: TimeInterval?
    private var nativeTextMutationAfterBlurGeneration: UInt64?
    private let nativeTextMutationAfterBlurGraceInterval: TimeInterval = 1.0
    /// Last selection known to match `lastAuthorizedText`, stored in that text's UTF-16 coordinates.
    private var lastAuthorizedSelectedUtf16Range: NSRange?
    private var logicalSelectionScalarRange: (anchor: UInt32, head: UInt32)?
    /// The UIKit selection `logicalSelectionScalarRange` was resolved to, after
    /// any empty-block autocapitalization nudge. Lets a deliberate nudge be
    /// told apart from the user moving the caret.
    private var logicalSelectionUtf16Range: NSRange?
    private var selectionRevision: UInt64 = 0
    private var desiredInputTraitState = InputTraitState()
    private var appliedInputTraitState = InputTraitState()
    private var pendingInputTraitChange = PendingInputTraitChange()
    private var pendingInputTraitRetryScheduled = false
    private var pendingInputTraitRetryGeneration: UInt64 = 0

    /// Coalesces selection sync until UIKit has finished resolving the
    /// current tap/drag gesture's final caret position.
    private var pendingSelectionSyncGeneration: UInt64 = 0
    private var pendingDeferredImageSelectionRange: NSRange?
    private var pendingDeferredImageSelectionGeneration: UInt64 = 0

    /// Stores the Rust-authorized scalar range replaced by the active marked
    /// text session. UIKit mutates visible TextKit state during composition,
    /// so final commits must not infer their range from the transient cursor.
    private var markedTextReplacementScalarRange: (from: UInt32, to: UInt32)?
    private var markedTextReplacementUtf16Range: NSRange?
    private var markedTextCompositionText: String?
    private var markedTextCompositionIsExplicitlyEmpty = false

    private struct ExternalTextCompositionState {
        let sessionId: String
        let startingAuthorizedText: String
        let startingAuthorizedAttributedText: NSAttributedString
        let startingSelectedUtf16Range: NSRange
        var latestText: String
    }

    private struct ExternalTextCompositionFinish {
        let resultJSON: String
        let adoptedUpdateJSON: String?
        let succeeded: Bool
    }

    private var externalTextComposition: ExternalTextCompositionState?
    private var externalTextCompositionTerminalResults: [String: String] = [:]

    private let editorLayoutManager: EditorLayoutManager

    private struct InputTraitState {
        var autoCapitalize: String?
        var autoCorrect: Bool?
        var keyboardType: String?
    }

    private struct PendingInputTraitChange {
        var hasAutoCapitalize = false
        var autoCapitalize: String?
        var hasAutoCorrect = false
        var autoCorrect: Bool?
        var hasKeyboardType = false
        var keyboardType: String?

        var isEmpty: Bool {
            !hasAutoCapitalize && !hasAutoCorrect && !hasKeyboardType
        }
    }

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
        addGestureRecognizer(imageSelectionTapRecognizer)
        installImageSelectionTapDependencies()

        addSubview(placeholderLabel)
        refreshPlaceholderVisibility()
        refreshNativeSelectionChromeVisibility()
    }

    func setAutoCapitalize(_ autoCapitalize: String?) {
        desiredInputTraitState.autoCapitalize = autoCapitalize
        guard prepareForInputTraitChange() else {
            pendingInputTraitChange.hasAutoCapitalize = true
            pendingInputTraitChange.autoCapitalize = autoCapitalize
            scheduleInputTraitChangeRetry()
            return
        }
        applyAutoCapitalize(autoCapitalize)
        appliedInputTraitState.autoCapitalize = autoCapitalize
        clearPendingAutoCapitalize()
    }

    private func applyAutoCapitalize(_ autoCapitalize: String?) {
        switch autoCapitalize {
        case "none":
            autocapitalizationType = .none
        case "words":
            autocapitalizationType = .words
        case "characters":
            autocapitalizationType = .allCharacters
        default:
            autocapitalizationType = .sentences
        }
        if isFirstResponder {
            reloadInputViews()
        }
    }

    func setAutoCorrect(_ autoCorrect: Bool?) {
        desiredInputTraitState.autoCorrect = autoCorrect
        guard prepareForInputTraitChange() else {
            pendingInputTraitChange.hasAutoCorrect = true
            pendingInputTraitChange.autoCorrect = autoCorrect
            scheduleInputTraitChangeRetry()
            return
        }
        applyAutoCorrect(autoCorrect)
        appliedInputTraitState.autoCorrect = autoCorrect
        clearPendingAutoCorrect()
    }

    private func applyAutoCorrect(_ autoCorrect: Bool?) {
        let isEnabled = autoCorrect ?? false
        autocorrectionType = isEnabled ? .yes : .no
        spellCheckingType = isEnabled ? .default : .no
        if isFirstResponder {
            reloadInputViews()
        }
    }

    func setKeyboardType(_ keyboardType: String?) {
        desiredInputTraitState.keyboardType = keyboardType
        guard prepareForInputTraitChange() else {
            pendingInputTraitChange.hasKeyboardType = true
            pendingInputTraitChange.keyboardType = keyboardType
            scheduleInputTraitChangeRetry()
            return
        }
        applyKeyboardType(keyboardType)
        appliedInputTraitState.keyboardType = keyboardType
        clearPendingKeyboardType()
    }

    private func applyKeyboardType(_ keyboardType: String?) {
        self.keyboardType = Self.resolvedKeyboardType(from: keyboardType)
        if isFirstResponder {
            reloadInputViews()
        }
    }

    private func prepareForInputTraitChange() -> Bool {
        guard isFirstResponder, editorId != 0 else { return true }
        return prepareForExternalEditorUpdate()
    }

    private func scheduleInputTraitChangeRetry() {
        guard !pendingInputTraitRetryScheduled else { return }
        pendingInputTraitRetryScheduled = true
        pendingInputTraitRetryGeneration &+= 1
        let retryGeneration = pendingInputTraitRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingInputTraitRetryGeneration else { return }
            self.pendingInputTraitRetryScheduled = false
            let pending = self.pendingInputTraitChange
            self.pendingInputTraitChange = PendingInputTraitChange()
            if pending.hasAutoCapitalize,
               pending.autoCapitalize == self.desiredInputTraitState.autoCapitalize {
                self.setAutoCapitalize(pending.autoCapitalize)
            }
            if pending.hasAutoCorrect,
               pending.autoCorrect == self.desiredInputTraitState.autoCorrect {
                self.setAutoCorrect(pending.autoCorrect)
            }
            if pending.hasKeyboardType,
               pending.keyboardType == self.desiredInputTraitState.keyboardType {
                self.setKeyboardType(pending.keyboardType)
            }
        }
    }

    private func clearPendingAutoCapitalize() {
        pendingInputTraitChange.hasAutoCapitalize = false
        pendingInputTraitChange.autoCapitalize = nil
        cancelPendingInputTraitRetryIfEmpty()
    }

    private func clearPendingAutoCorrect() {
        pendingInputTraitChange.hasAutoCorrect = false
        pendingInputTraitChange.autoCorrect = nil
        cancelPendingInputTraitRetryIfEmpty()
    }

    private func clearPendingKeyboardType() {
        pendingInputTraitChange.hasKeyboardType = false
        pendingInputTraitChange.keyboardType = nil
        cancelPendingInputTraitRetryIfEmpty()
    }

    private func clearPendingInputTraitRetry() {
        pendingInputTraitChange = PendingInputTraitChange()
        guard pendingInputTraitRetryScheduled else { return }
        pendingInputTraitRetryScheduled = false
        pendingInputTraitRetryGeneration &+= 1
    }

    private func cancelPendingInputTraitRetryIfEmpty() {
        guard pendingInputTraitRetryScheduled, pendingInputTraitChange.isEmpty else { return }
        pendingInputTraitRetryScheduled = false
        pendingInputTraitRetryGeneration &+= 1
    }

    private func replayDesiredInputTraitsIfNeeded() {
        if desiredInputTraitState.autoCapitalize != appliedInputTraitState.autoCapitalize {
            setAutoCapitalize(desiredInputTraitState.autoCapitalize)
        }
        if desiredInputTraitState.autoCorrect != appliedInputTraitState.autoCorrect {
            setAutoCorrect(desiredInputTraitState.autoCorrect)
        }
        if desiredInputTraitState.keyboardType != appliedInputTraitState.keyboardType {
            setKeyboardType(desiredInputTraitState.keyboardType)
        }
    }

    private static func resolvedKeyboardType(from keyboardType: String?) -> UIKeyboardType {
        switch keyboardType {
        case "ascii-capable":
            return .asciiCapable
        case "numbers-and-punctuation":
            return .numbersAndPunctuation
        case "url":
            return .URL
        case "number-pad":
            return .numberPad
        case "phone-pad":
            return .phonePad
        case "name-phone-pad":
            return .namePhonePad
        case "email-address":
            return .emailAddress
        case "decimal-pad", "numeric":
            return .decimalPad
        case "twitter":
            return .twitter
        case "web-search":
            return .webSearch
        case "ascii-capable-number-pad":
            return .asciiCapableNumberPad
        case "visible-password":
            return .asciiCapable
        default:
            return .default
        }
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
        guard let attachment = notification.object as? NSTextAttachment else { return }
        guard textStorage.length > 0 else { return }
        var ownsAttachment = false
        textStorage.enumerateAttribute(
            .attachment,
            in: NSRange(location: 0, length: textStorage.length),
            options: [.longestEffectiveRangeNotRequired]
        ) { value, _, stop in
            guard let candidate = value as? NSTextAttachment,
                  candidate === attachment
            else {
                return
            }
            ownsAttachment = true
            stop.pointee = true
        }
        guard ownsAttachment else { return }

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

    /// Whether the document holds nothing the user authored.
    ///
    /// Taken verbatim from the core. Deriving it cannot work: an empty list
    /// item contributes no characters, so scanning the storage reports empty
    /// and leaves the placeholder over a visible bullet. The scan below is
    /// only the fallback for renders with no editor update (the viewer).
    private func isRenderedContentEmpty() -> Bool {
        if let coreReportedDocumentIsEmpty {
            return coreReportedDocumentIsEmpty
        }

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

    @discardableResult
    private func normalizeSelectionForEmptyBlockAutocapitalizationIfNeeded() -> Bool {
        guard textStorage.length == 1 else { return false }
        guard textStorage.string.unicodeScalars.elementsEqual([Self.emptyBlockPlaceholderScalar]) else {
            return false
        }

        let currentRange = selectedRange
        guard currentRange.location != NSNotFound, currentRange.length == 0 else { return false }
        guard currentRange.location == textStorage.length else { return false }

        let adjustedRange = NSRange(location: 0, length: 0)
        guard currentRange != adjustedRange else { return false }
        selectedRange = adjustedRange
        noteSelectionDidChange()
        return true
    }

    /// The core's `documentIsEmpty` from the most recent editor update, or nil
    /// when the current render arrived without one.
    private var coreReportedDocumentIsEmpty: Bool?

    private func refreshPlaceholderVisibility() {
        let hasProvisionalText = externalTextComposition?.latestText.isEmpty == false
        placeholderLabel.isHidden = placeholder.isEmpty
            || hasProvisionalText
            || !isRenderedContentEmpty()
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

    func hasTaskListMarker(at location: CGPoint) -> Bool {
        taskListMarkerParagraphStart(at: location) != nil
    }

    @discardableResult
    func toggleTaskListMarker(at location: CGPoint) -> Bool {
        guard editorId != 0 else { return false }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return false }
        guard prepareForExternalEditorUpdate() else { return false }
        guard let paragraphStart = taskListMarkerParagraphStart(at: location) else {
            return false
        }

        _ = becomeFirstResponder()
        let scalar = PositionBridge.utf16OffsetToScalar(paragraphStart, in: self)
        performInterceptedInput {
            toggleTaskItemCheckedAtSelectionScalarInRust(anchor: scalar, head: scalar)
        }
        return true
    }

    @discardableResult
    private func selectImageAttachment(range: NSRange) -> Bool {
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return false }
        guard isSelectable,
              let start = position(from: beginningOfDocument, offset: range.location),
              let end = position(from: start, offset: range.length),
              let textRange = textRange(from: start, to: end)
        else {
            return false
        }

        _ = becomeFirstResponder()
        selectedTextRange = textRange
        noteSelectionDidChange()
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

    private func noteSelectionDidChange() {
        selectionRevision &+= 1
    }

    private func recordAuthorizedSelectionIfPossible() {
        guard editorId != 0 else {
            lastAuthorizedSelectedUtf16Range = nil
            return
        }
        let currentText = textStorage.string
        guard currentText.utf16.count == lastAuthorizedTextStorage.length,
              currentText == lastAuthorizedText
        else {
            return
        }
        lastAuthorizedSelectedUtf16Range = selectedUtf16Range()
    }

    private func scalarRange(forUtf16Range range: NSRange) -> (from: UInt32, to: UInt32) {
        let start = PositionBridge.utf16OffsetToScalar(range.location, in: self)
        let end = PositionBridge.utf16OffsetToScalar(NSMaxRange(range), in: self)
        return (from: min(start, end), to: max(start, end))
    }

    private func scalarRange(
        forUtf16Range range: NSRange,
        in storage: NSAttributedString
    ) -> (from: UInt32, to: UInt32) {
        let start = PositionBridge.utf16OffsetToScalar(range.location, in: storage)
        let end = PositionBridge.utf16OffsetToScalar(NSMaxRange(range), in: storage)
        return (from: min(start, end), to: max(start, end))
    }

    /// The last Rust-authorized storage with one native replacement applied.
    ///
    /// A caret inside a natively mutated document cannot be converted against
    /// the live text view. A keyboard correction replaces the word with an
    /// unattributed string, which strips the structural attributes the
    /// utf16→scalar conversion reads to count a list's block openings, so the
    /// live view maps the caret as though the line were never wrapped. The
    /// authorized storage still carries those attributes; patching its text
    /// with the same replacement gives a ruler that matches the new text
    /// while keeping the structure. Falls back to the unpatched storage when
    /// the range no longer addresses it.
    private func authorizedStorageApplying(
        _ replacementText: String,
        inUtf16Range range: NSRange
    ) -> NSAttributedString {
        guard range.location >= 0,
              range.length >= 0,
              NSMaxRange(range) <= lastAuthorizedAttributedTextStorage.length
        else {
            return lastAuthorizedAttributedTextStorage
        }
        let patched = NSMutableAttributedString(
            attributedString: lastAuthorizedAttributedTextStorage
        )
        // The string overload inherits the replaced run's attributes, which is
        // exactly what keeps the structure the keyboard's replacement dropped.
        patched.replaceCharacters(in: range, with: replacementText)
        return patched
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

    func authorizedTextForTesting() -> String {
        lastAuthorizedText
    }

    func lastApplyUpdateTrace() -> ApplyUpdateTrace? {
        lastApplyUpdateTraceForTesting
    }

    func isUsingInternalTextViewDelegateForTesting() -> Bool {
        (delegate as AnyObject?) === internalTextViewDelegate
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

    func resetCodeBlockDrawPassesForTesting() {
        editorLayoutManager.resetCodeBlockDrawPassesForTesting()
    }

    func codeBlockDrawPassesForTesting() -> [[CGRect]] {
        editorLayoutManager.codeBlockDrawPassesForTesting
    }

    @discardableResult
    func selectImageAttachmentForTesting(at location: CGPoint) -> Bool {
        selectImageAttachmentIfNeeded(at: location)
    }

    private func taskListMarkerParagraphStart(at location: CGPoint) -> Int? {
        guard let layoutManager = layoutManager as? EditorLayoutManager else { return nil }
        let origin = CGPoint(
            x: textContainerInset.left - contentOffset.x,
            y: textContainerInset.top - contentOffset.y
        )
        return layoutManager.taskListMarkerParagraphStart(
            at: location,
            in: textStorage,
            textContainerOrigin: origin
        )
    }

    func imageSelectionTapWouldHandleForTesting(at location: CGPoint) -> Bool {
        imageAttachmentRange(at: location) != nil
    }

    func taskListMarkerParagraphStartForTesting(at location: CGPoint) -> Int? {
        taskListMarkerParagraphStart(at: location)
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
    func bindEditor(
        id: UInt64,
        initialHTML: String? = nil,
        initialUpdateJSON: String? = nil
    ) {
        ensureInternalTextViewDelegate()
        if editorId == id, initialHTML == nil {
            return
        }
        if editorId != id {
            EditorV2Registry.adapter(forLegacyId: editorId)?
                .releaseNativeBindingOwner(token: nativeBindingToken)
            discardTransientNativeInputForEditorRebind()
        }
        editorId = id
        EditorV2Registry.adapter(forLegacyId: id)?
            .claimNativeBindingIfUnowned(token: nativeBindingToken)

        if let initialUpdateJSON {
            applyUpdateJSON(initialUpdateJSON, notifyDelegate: false)
        } else if let html = initialHTML, !html.isEmpty {
            let updateJSON = EditorV2Shadow.setHtml(id: editorId, html: html)
            if !applyUpdateJSON(updateJSON, notifyDelegate: false) {
                let stateJSON = EditorV2Shadow.getCurrentState(id: editorId)
                applyUpdateJSON(stateJSON, notifyDelegate: false)
            }
        } else {
            // Pull current state from Rust (content may already be loaded via bridge).
            let stateJSON = EditorV2Shadow.getCurrentState(id: editorId)
            applyUpdateJSON(stateJSON, notifyDelegate: false)
        }
        replayDesiredInputTraitsIfNeeded()
    }

    /// Unbind from the current editor instance.
    func unbindEditor() {
        discardTransientNativeInputForEditorRebind()
        EditorV2Registry.adapter(forLegacyId: editorId)?
            .releaseNativeBindingOwner(token: nativeBindingToken)
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
        ensureInternalTextViewDelegate()
        guard !isApplyingRustState else {
            super.insertText(text)
            return
        }
        guard editorId != 0 else {
            super.insertText(text)
            return
        }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        guard flushPendingNativeTextMutationCommitIfNeeded() else { return }

        if text == "\n" {
            guard commitActiveMarkedTextBeforeReturn() else { return }
            performInterceptedInput {
                handleReturnKey()
            }
            return
        }

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

    @objc private func handleIndentKeyCommand() {
        handleListDepthKeyCommand(outdent: false)
    }

    @objc private func handleHardBreakKeyCommand() {
        guard !isApplyingRustState, editorId != 0, isEditable else { return }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
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
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
        guard isCaretInsideList() else { return }
        guard let selection = currentScalarSelection() else { return }

        performInterceptedInput {
            let updateJSON = outdent
                ? EditorV2Shadow.outdentListItemAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head
                )
                : EditorV2Shadow.indentListItemAtSelectionScalar(
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
            let data = EditorV2Shadow.getCurrentState(id: editorId).data(using: .utf8),
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

        if markedTextReplacementScalarRange != nil || markedTextRange != nil {
            let replacementRange = trackedMarkedTextReplacementRange()
            finishTransientMarkedTextMutation()
            _ = commitMarkedText(text, replacementRange: replacementRange)
            return
        }

        let scalarRange = PositionBridge.textRangeToScalarRange(range, in: self)
        Self.inputLog.debug(
            "[replace] text=\(self.preview(text), privacy: .public) scalarRange=\(scalarRange.from)-\(scalarRange.to) selection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )

        // Atomically replace the range with the new text via Rust.
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.replaceTextScalar(
                id: editorId,
                scalarFrom: scalarRange.from,
                scalarTo: scalarRange.to,
                text: text
            )
            applyUpdateJSON(updateJSON)
        }
    }

    // MARK: - External Text Composition

    func beginExternalTextComposition(sessionId: String) -> String {
        guard editorId != 0,
              isEditable,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId),
              !adapter.isDestroyed
        else {
            return externalCompositionErrorJSON(
                sessionId: sessionId,
                code: "EXTERNAL_COMPOSITION_UNAVAILABLE",
                message: "The native editor is unavailable or not editable"
            )
        }
        guard externalTextCompositionTerminalResults[sessionId] == nil else {
            return externalCompositionEndedErrorJSON(sessionId: sessionId)
        }
        guard let selectionJSON = adapter.selectionJSON(),
              let selectionData = selectionJSON.data(using: .utf8),
              let selection = try? JSONSerialization.jsonObject(with: selectionData) as? [String: Any],
              selection["type"] as? String == "text"
        else {
            return externalCompositionErrorJSON(
                sessionId: sessionId,
                code: "EXTERNAL_COMPOSITION_SELECTION_INCOMPATIBLE",
                message: "External composition requires a text selection"
            )
        }

        if externalTextComposition != nil {
            let finished = finishExternalTextComposition(
                cause: "consumer",
                finalText: nil,
                cancel: false
            )
            guard finished?.succeeded == true else {
                return externalCompositionErrorJSON(
                    sessionId: sessionId,
                    code: "EXTERNAL_COMPOSITION_COMMIT_FAILED",
                    message: "The previous external text composition could not be committed"
                )
            }
        } else if isComposing, !prepareForExternalEditorUpdate() {
            return externalCompositionErrorJSON(
                sessionId: sessionId,
                code: "EXTERNAL_COMPOSITION_UNAVAILABLE",
                message: "The active input composition could not be committed"
            )
        }
        guard flushPendingNativeTextMutationCommitIfNeeded() else {
            return externalCompositionErrorJSON(
                sessionId: sessionId,
                code: "EXTERNAL_COMPOSITION_UNAVAILABLE",
                message: "Pending native input could not be committed"
            )
        }

        let startingSelection = selectedRange.location == NSNotFound
            ? NSRange(location: 0, length: 0)
            : selectedRange
        captureMarkedTextReplacementRangeIfNeeded()
        externalTextComposition = ExternalTextCompositionState(
            sessionId: sessionId,
            startingAuthorizedText: lastAuthorizedText,
            startingAuthorizedAttributedText: NSAttributedString(
                attributedString: lastAuthorizedAttributedTextStorage
            ),
            startingSelectedUtf16Range: startingSelection,
            latestText: ""
        )
        isComposing = true
        return externalCompositionActiveJSON(sessionId: sessionId)
    }

    func updateExternalTextComposition(sessionId: String, text: String) -> String {
        guard var state = externalTextComposition, state.sessionId == sessionId else {
            return externalCompositionEndedErrorJSON(sessionId: sessionId)
        }
        performTransientTextMutation {
            super.setMarkedText(
                text,
                selectedRange: NSRange(location: (text as NSString).length, length: 0)
            )
        }
        state.latestText = text
        externalTextComposition = state
        refreshMarkedTextCompositionText(fallback: text)
        refreshPlaceholderVisibility()
        return externalCompositionActiveJSON(sessionId: sessionId)
    }

    func commitExternalTextComposition(sessionId: String, finalText: String) -> String {
        guard externalTextComposition?.sessionId == sessionId else {
            return externalTextCompositionTerminalResults[sessionId]
                ?? externalCompositionEndedErrorJSON(sessionId: sessionId)
        }
        return finishExternalTextComposition(
            cause: "consumer",
            finalText: finalText,
            cancel: false
        )?.resultJSON ?? externalCompositionEndedErrorJSON(sessionId: sessionId)
    }

    func cancelExternalTextComposition(sessionId: String, cause: String) -> String {
        guard ["consumer", "documentChange", "lifecycle"].contains(cause) else {
            return externalCompositionErrorJSON(
                sessionId: sessionId,
                code: "EXTERNAL_COMPOSITION_CANCEL_CAUSE_INVALID",
                message: "The external composition cancellation cause is invalid"
            )
        }
        guard externalTextComposition?.sessionId == sessionId else {
            return externalTextCompositionTerminalResults[sessionId]
                ?? externalCompositionEndedErrorJSON(sessionId: sessionId)
        }
        return finishExternalTextComposition(
            cause: cause,
            finalText: nil,
            cancel: true
        )?.resultJSON ?? externalCompositionEndedErrorJSON(sessionId: sessionId)
    }

    private func finishExternalTextCompositionBeforeInteractionIfNeeded() -> Bool {
        guard externalTextComposition != nil else { return true }
        return finishExternalTextComposition(
            cause: "interaction",
            finalText: nil,
            cancel: false
        )?.succeeded == true
    }

    private func finishExternalTextComposition(
        cause: String,
        finalText: String?,
        cancel: Bool
    ) -> ExternalTextCompositionFinish? {
        guard var state = externalTextComposition else { return nil }
        if let finalText {
            if finalText != state.latestText {
                performTransientTextMutation {
                    super.setMarkedText(
                        finalText,
                        selectedRange: NSRange(
                            location: (finalText as NSString).length,
                            length: 0
                        )
                    )
                }
                refreshMarkedTextCompositionText(fallback: finalText)
            }
            state.latestText = finalText
            externalTextComposition = state
        }

        let replacementRange = trackedMarkedTextReplacementRange()
        var authoritativeUpdateJSON: String?
        if cancel,
           editorId != 0,
           let adapter = EditorV2Registry.adapter(forLegacyId: editorId),
           !adapter.isDestroyed
        {
            authoritativeUpdateJSON = adapter.currentStateJSON()
        }
        externalTextComposition = nil
        finishTransientMarkedTextMutation()

        if cancel {
            restoreAuthorizedExternalComposition(
                state,
                authoritativeUpdateJSON: authoritativeUpdateJSON
            )
            let resultJSON = externalCompositionEndedJSON(
                sessionId: state.sessionId,
                outcome: "cancelled",
                cause: cause,
                text: state.latestText
            )
            emitExternalTextCompositionEnd(sessionId: state.sessionId, resultJSON: resultJSON)
            return ExternalTextCompositionFinish(
                resultJSON: resultJSON,
                adoptedUpdateJSON: nil,
                succeeded: true
            )
        }

        guard let commit = commitMarkedTextWithNativeOutcome(
            state.latestText,
            replacementRange: replacementRange
        ) else {
            let currentUpdateJSON: String?
            if editorId == 0 {
                currentUpdateJSON = nil
            } else {
                currentUpdateJSON = EditorV2Registry.adapter(forLegacyId: editorId)?
                    .recoverNativeRender()
            }
            restoreAuthorizedExternalComposition(
                state,
                authoritativeUpdateJSON: currentUpdateJSON
            )
            let error = externalCompositionErrorPayload(
                code: "EXTERNAL_COMPOSITION_COMMIT_FAILED",
                message: "The external text composition could not be committed"
            )
            let resultJSON = externalCompositionEndedJSON(
                sessionId: state.sessionId,
                outcome: "cancelled",
                cause: cause,
                text: state.latestText,
                error: error
            )
            emitExternalTextCompositionEnd(sessionId: state.sessionId, resultJSON: resultJSON)
            return ExternalTextCompositionFinish(
                resultJSON: resultJSON,
                adoptedUpdateJSON: nil,
                succeeded: false
            )
        }

        let resultJSON = externalCompositionEndedJSON(
            sessionId: state.sessionId,
            outcome: "committed",
            cause: cause,
            text: state.latestText
        )
        emitExternalTextCompositionEnd(sessionId: state.sessionId, resultJSON: resultJSON)
        return ExternalTextCompositionFinish(
            resultJSON: resultJSON,
            adoptedUpdateJSON: commit.updateJSON,
            succeeded: true
        )
    }

    private func restoreAuthorizedExternalComposition(
        _ state: ExternalTextCompositionState,
        authoritativeUpdateJSON: String? = nil
    ) {
        _ = applyAttributedRender(
            state.startingAuthorizedAttributedText,
            usedPatch: false,
            positionCacheUpdate: .scan
        )
        if let authoritativeUpdateJSON {
            _ = applyUpdateJSON(authoritativeUpdateJSON, notifyDelegate: false)
        }
        guard textStorage.string == state.startingAuthorizedText,
              state.startingSelectedUtf16Range.location >= 0,
              state.startingSelectedUtf16Range.length >= 0,
              state.startingSelectedUtf16Range.location
                + state.startingSelectedUtf16Range.length <= textStorage.length
        else {
            return
        }

        logicalSelectionScalarRange = nil
        logicalSelectionUtf16Range = nil
        performTransientTextMutation {
            selectedRange = state.startingSelectedUtf16Range
            noteSelectionDidChange()
        }
        recordAuthorizedSelectionIfPossible()
        refreshTypingAttributesForSelection()
    }

    private func emitExternalTextCompositionEnd(sessionId: String, resultJSON: String) {
        guard externalTextCompositionTerminalResults[sessionId] == nil else { return }
        externalTextCompositionTerminalResults[sessionId] = resultJSON
        editorDelegate?.editorTextView(self, didEndExternalTextComposition: resultJSON)
    }

    private func externalCompositionActiveJSON(sessionId: String) -> String {
        externalCompositionResultJSON([
            "version": 1,
            "type": "active",
            "sessionId": sessionId,
        ])
    }

    private func externalCompositionEndedJSON(
        sessionId: String,
        outcome: String,
        cause: String,
        text: String,
        error: [String: Any]? = nil
    ) -> String {
        var payload: [String: Any] = [
            "version": 1,
            "type": "ended",
            "sessionId": sessionId,
            "outcome": outcome,
            "cause": cause,
            "text": text,
        ]
        if let error {
            payload["error"] = error
        }
        return externalCompositionResultJSON(payload)
    }

    private func externalCompositionEndedErrorJSON(sessionId: String) -> String {
        externalCompositionErrorJSON(
            sessionId: sessionId,
            code: "EXTERNAL_COMPOSITION_ENDED",
            message: "The external text composition session has ended"
        )
    }

    private func externalCompositionErrorJSON(
        sessionId: String?,
        code: String,
        message: String
    ) -> String {
        externalCompositionResultJSON([
            "version": 1,
            "type": "error",
            "sessionId": sessionId.map { $0 as Any } ?? NSNull(),
            "error": externalCompositionErrorPayload(code: code, message: message),
        ])
    }

    private func externalCompositionErrorPayload(code: String, message: String) -> [String: Any] {
        [
            "domain": "lifecycle",
            "code": code,
            "message": message,
            "requestId": NSNull(),
            "operationIndex": NSNull(),
            "limit": NSNull(),
            "actual": NSNull(),
            "details": NSNull(),
        ]
    }

    private func externalCompositionResultJSON(_ payload: [String: Any]) -> String {
        guard JSONSerialization.isValidJSONObject(payload),
              let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return #"{"version":1,"type":"error","sessionId":null,"error":{"domain":"lifecycle","code":"EXTERNAL_COMPOSITION_RESULT_INVALID","message":"Could not serialize external composition result","requestId":null,"operationIndex":null,"limit":null,"actual":null,"details":null}}"#
        }
        return json
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

    private func captureMarkedTextReplacementRangeIfNeeded() {
        guard markedTextReplacementScalarRange == nil else { return }

        guard let selectedRange = selectedTextRange else {
            let scalarPos = PositionBridge.cursorScalarOffset(in: self)
            let utf16Pos = PositionBridge.scalarToUtf16Offset(scalarPos, in: lastAuthorizedText)
            markedTextReplacementScalarRange = (from: scalarPos, to: scalarPos)
            markedTextReplacementUtf16Range = NSRange(
                location: utf16Pos,
                length: 0
            )
            return
        }

        let scalarRange = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
        let startUtf16 = offset(from: beginningOfDocument, to: selectedRange.start)
        let endUtf16 = offset(from: beginningOfDocument, to: selectedRange.end)

        markedTextReplacementScalarRange = (from: scalarRange.from, to: scalarRange.to)
        markedTextReplacementUtf16Range = NSRange(
            location: min(startUtf16, endUtf16),
            length: abs(endUtf16 - startUtf16)
        )
    }

    private func trackedMarkedTextReplacementRange() -> (from: UInt32, to: UInt32)? {
        if let markedTextReplacementScalarRange {
            return markedTextReplacementScalarRange
        }
        guard let selectedRange = selectedTextRange else { return nil }
        let scalarRange = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
        return (from: scalarRange.from, to: scalarRange.to)
    }

    private func clearMarkedTextTracking() {
        markedTextReplacementScalarRange = nil
        markedTextReplacementUtf16Range = nil
        markedTextCompositionText = nil
        markedTextCompositionIsExplicitlyEmpty = false
        isComposing = false
    }

    private func finishTransientMarkedTextMutation() {
        performTransientTextMutation {
            super.unmarkText()
        }
        clearMarkedTextTracking()
    }

    private func performTransientTextMutation(_ action: () -> Void) {
        let wasApplyingRustState = isApplyingRustState
        isApplyingRustState = true
        action()
        isApplyingRustState = wasApplyingRustState
    }

    private func currentMarkedTextForCommit() -> String? {
        if markedTextCompositionIsExplicitlyEmpty { return "" }
        return transientMarkedTextFromAuthorizedDiff()
            ?? markedTextRange.flatMap { text(in: $0) }
            ?? markedTextCompositionText
    }

    private func validatedTrackedMarkedTextForCommit() -> String? {
        guard markedTextReplacementScalarRange != nil || markedTextReplacementUtf16Range != nil else {
            return nil
        }
        if markedTextCompositionIsExplicitlyEmpty { return "" }
        return transientMarkedTextFromAuthorizedDiff()
            ?? markedTextRange.flatMap { text(in: $0) }
    }

    private func refreshMarkedTextCompositionText(fallback: String? = nil) {
        if fallback?.isEmpty == true {
            markedTextCompositionText = ""
            markedTextCompositionIsExplicitlyEmpty = true
            return
        }
        markedTextCompositionIsExplicitlyEmpty = false
        markedTextCompositionText = transientMarkedTextFromAuthorizedDiff()
            ?? markedTextRange.flatMap { text(in: $0) }
            ?? fallback
    }

    private func transientMarkedTextFromAuthorizedDiff() -> String? {
        guard let replacementRange = markedTextReplacementUtf16Range else { return nil }

        let currentText = textStorage.string as NSString
        let authorizedText = lastAuthorizedText as NSString
        let replacementEnd = replacementRange.location + replacementRange.length
        guard replacementRange.location >= 0,
              replacementEnd <= authorizedText.length
        else {
            return nil
        }

        let insertedLength = currentText.length - (authorizedText.length - replacementRange.length)
        guard insertedLength >= 0,
              replacementRange.location + insertedLength <= currentText.length
        else {
            return nil
        }

        if replacementRange.location > 0 {
            let prefixRange = NSRange(location: 0, length: replacementRange.location)
            guard currentText.substring(with: prefixRange) == authorizedText.substring(with: prefixRange) else {
                return nil
            }
        }

        let insertedEnd = replacementRange.location + insertedLength
        let authorizedSuffixLength = authorizedText.length - replacementEnd
        let currentSuffixLength = currentText.length - insertedEnd
        guard authorizedSuffixLength == currentSuffixLength else { return nil }
        if authorizedSuffixLength > 0 {
            let authorizedSuffixRange = NSRange(location: replacementEnd, length: authorizedSuffixLength)
            let currentSuffixRange = NSRange(location: insertedEnd, length: currentSuffixLength)
            guard currentText.substring(with: currentSuffixRange)
                == authorizedText.substring(with: authorizedSuffixRange)
            else {
                return nil
            }
        }

        return currentText.substring(
            with: NSRange(location: replacementRange.location, length: insertedLength)
        )
    }

    private func commitMarkedText(
        _ text: String,
        replacementRange: (from: UInt32, to: UInt32)?
    ) -> String? {
        guard editorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId)
        else {
            return nil
        }
        var adoptedUpdateJSON: String?
        performInterceptedInput(flushPendingNativeTextMutation: false) {
            if let replacementRange {
                if replacementRange.from == replacementRange.to {
                    adoptedUpdateJSON = adapter.insertText(text, atScalar: replacementRange.from)
                } else {
                    adoptedUpdateJSON = adapter.replaceTextRange(
                        from: replacementRange.from,
                        to: replacementRange.to,
                        with: text
                    )
                }
            } else {
                adoptedUpdateJSON = adapter.insertText(
                    text,
                    atScalar: PositionBridge.cursorScalarOffset(in: self)
                )
            }
            guard let adoptedUpdateJSON else { return }
            applyUpdateJSON(adoptedUpdateJSON)
        }
        return adoptedUpdateJSON
    }

    private func commitMarkedTextWithNativeOutcome(
        _ text: String,
        replacementRange: (from: UInt32, to: UInt32)?
    ) -> EditorV2Adapter.NativeMutationRender? {
        guard editorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId)
        else {
            return nil
        }
        var commit: EditorV2Adapter.NativeMutationRender?
        performInterceptedInput(flushPendingNativeTextMutation: false) {
            if let replacementRange {
                if replacementRange.from == replacementRange.to {
                    commit = adapter.insertTextWithNativeOutcome(
                        text,
                        atScalar: replacementRange.from
                    )
                } else {
                    commit = adapter.replaceTextRangeWithNativeOutcome(
                        from: replacementRange.from,
                        to: replacementRange.to,
                        with: text
                    )
                }
            } else {
                let position = PositionBridge.cursorScalarOffset(in: self)
                commit = adapter.insertTextWithNativeOutcome(text, atScalar: position)
            }
            guard let commit else { return }
            applyUpdateJSON(commit.updateJSON, notifyDelegate: commit.documentChanged)
        }
        return commit
    }

    private func commitActiveMarkedTextBeforeReturn() -> Bool {
        guard markedTextReplacementScalarRange != nil || markedTextRange != nil else {
            return true
        }

        if markedTextReplacementScalarRange != nil || markedTextReplacementUtf16Range != nil {
            let composedText = currentMarkedTextForCommit()
            let replacementRange = trackedMarkedTextReplacementRange()
            finishTransientMarkedTextMutation()

            guard shouldCommitMarkedText(composedText, replacementRange: replacementRange) else {
                restoreAuthorizedTextAfterCancelledCompositionIfNeeded()
                return true
            }
            return commitMarkedText(composedText ?? "", replacementRange: replacementRange) != nil
        }

        let mutation = nativeTextMutationFromAuthorizedDiff(currentText: textStorage.string)
        finishTransientMarkedTextMutation()
        guard let mutation else {
            restoreAuthorizedTextAfterCancelledCompositionIfNeeded()
            return true
        }

        switch commitNativeTextMutationIfPossible(
            mutation,
            allowAfterBlur: false,
            allowWhileIntercepting: true
        ) {
        case .committed:
            return true
        case .deferred, .rejected:
            restoreAuthorizedTextAfterCancelledCompositionIfNeeded()
            return false
        }
    }

    private func shouldCommitMarkedText(
        _ text: String?,
        replacementRange: (from: UInt32, to: UInt32)?
    ) -> Bool {
        guard let text else { return false }
        if !text.isEmpty { return true }
        guard let replacementRange else { return false }
        return replacementRange.from != replacementRange.to
    }

    private func restoreAuthorizedTextAfterCancelledCompositionIfNeeded() {
        guard editorId != 0 else { return }
        guard textStorage.string != lastAuthorizedText else { return }

        let stateJSON = EditorV2Shadow.getCurrentState(id: editorId)
        applyUpdateJSON(stateJSON)
    }

    private func previewMarkedTextReplacementRange(
        _ range: (from: UInt32, to: UInt32)?
    ) -> String {
        guard let range else { return "none" }
        let utf16 = markedTextReplacementUtf16Range
            .map { "\($0.location)..<\($0.location + $0.length)" }
            ?? "none"
        return "scalar=\(range.from)..<\(range.to) utf16=\(utf16)"
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

    // MARK: - Selection Change Notification

    /// UITextViewDelegate hook for user-driven selection updates.
    ///
    /// Using the delegate callback is more reliable than observing
    /// `selectedTextRange` directly because UIKit can adjust selection
    /// internally during tap handling and word-boundary resolution.
    func textViewDidChangeSelection(_ textView: UITextView) {
        guard textView === self else { return }
        ensureInternalTextViewDelegate()
        noteSelectionDidChange()
        if externalTextComposition != nil {
            guard !isApplyingRustState else { return }
            let interactionSelection = selectedRange
            guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
            if interactionSelection.location != NSNotFound,
               interactionSelection.location >= 0,
               interactionSelection.length >= 0,
               interactionSelection.location + interactionSelection.length <= textStorage.length
            {
                logicalSelectionScalarRange = nil
                logicalSelectionUtf16Range = nil
                performTransientTextMutation {
                    selectedRange = interactionSelection
                    noteSelectionDidChange()
                }
            }
        }
        guard !isApplyingRustState,
              !isComposing,
              !nativeTextMutationCommitScheduled,
              pendingNativeTextMutation == nil
        else {
            return
        }
        if normalizeSelectionForEmptyBlockAutocapitalizationIfNeeded() {
            return
        }
        recordAuthorizedSelectionIfPossible()
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
        noteSelectionDidChange()
        refreshNativeSelectionChromeVisibility()
        onSelectionOrContentMayChange?()
        scheduleSelectionSync()
        return false
    }

    // MARK: - Private: Rust Integration

    private var isInterceptingInput: Bool {
        interceptedInputDepth > 0
    }

    private func ensureInternalTextViewDelegate() {
        // Some keyboard integrations replace UITextView's private delegate ivar
        // directly. The editor must own delegate callbacks so external observers
        // cannot inspect transient TextKit state during Rust-driven edits.
        // The delegate is a dedicated object rather than the text view itself;
        // see EditorTextViewInternalDelegate for why (APOLLO-REACT-56).
        guard (delegate as AnyObject?) !== internalTextViewDelegate else { return }
        delegate = internalTextViewDelegate
    }

    private func performInterceptedInput(
        flushPendingNativeTextMutation: Bool = true,
        _ action: () -> Void
    ) {
        if flushPendingNativeTextMutation, interceptedInputDepth == 0 {
            guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
        }
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
            if self.interceptedInputDepth == 0 {
                _ = self.drainPendingNativeTextMutation(
                    allowAfterBlur: false,
                    allowWhileIntercepting: false
                )
            }
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
        // The UTF-16 offsets are what UIKit actually holds; every scalar below
        // is derived through the conversion table. Logging both is what tells
        // a caret that moved apart from a table that changed under a caret
        // that did not.
        let anchorUtf16 = offset(from: beginningOfDocument, to: range.start)
        let headUtf16 = offset(from: beginningOfDocument, to: range.end)
        let anchorScalar = PositionBridge.textViewToScalar(range.start, in: self)
        let headScalar = PositionBridge.textViewToScalar(range.end, in: self)
        guard editorId != 0 else {
            return "utf16=\(anchorUtf16)-\(headUtf16) scalar=\(anchorScalar)-\(headScalar)"
        }
        let docAnchor = EditorV2Shadow.scalarToDoc(id: editorId, scalar: anchorScalar)
        let docHead = EditorV2Shadow.scalarToDoc(id: editorId, scalar: headScalar)
        return "utf16=\(anchorUtf16)-\(headUtf16) scalar=\(anchorScalar)-\(headScalar) doc=\(docAnchor)-\(docHead)"
    }

    private func selectionSummary(from selection: [String: Any]) -> String {
        guard let type = selection["type"] as? String else { return "unknown" }
        switch type {
        case "text":
            let anchor = v2ExactUInt32(selection["anchor"] as? NSNumber) ?? 0
            let head = v2ExactUInt32(selection["head"] as? NSNumber) ?? 0
            return "text doc=\(anchor)-\(head)"
        case "node":
            let pos = v2ExactUInt32(selection["pos"] as? NSNumber) ?? 0
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
        _ = normalizeSelectionForEmptyBlockAutocapitalizationIfNeeded()
        refreshNativeSelectionChromeVisibility()
        refreshTypingAttributesForSelection()
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
        guard !isApplyingRustState,
              !isComposing,
              !nativeTextMutationCommitScheduled,
              pendingNativeTextMutation == nil,
              editorId != 0
        else {
            return
        }
        guard let selection = currentLogicalScalarSelection() else { return }

        let anchor = selection.anchor
        let head = selection.head
        guard let sync = EditorV2Shadow.setSelectionScalar(
            id: editorId,
            scalarAnchor: anchor,
            scalarHead: head
        ) else {
            return
        }
        Self.selectionLog.debug(
            "[textViewDidChangeSelection] scalar=\(anchor)-\(head) doc=\(sync.docAnchor)-\(sync.docHead) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )
        if let refreshed = sync.refreshedUpdateJSON {
            applyUpdateJSON(refreshed, notifyDelegate: false)
        }
        recordAuthorizedSelectionIfPossible()
        refreshTypingAttributesForSelection()
        editorDelegate?.editorTextView(
            self,
            selectionDidChange: sync.docAnchor,
            head: sync.docHead
        )
    }

    @discardableResult
    func applyTheme(_ theme: EditorTheme?) -> Bool {
        if editorId != 0 {
            guard prepareForExternalEditorUpdate() else { return false }
            self.theme = theme
            let previousOffset = contentOffset
            let stateJSON = EditorV2Shadow.getCurrentState(id: editorId)
            applyUpdateJSON(stateJSON, notifyDelegate: false)
            if heightBehavior == .fixed {
                preserveScrollOffset(previousOffset)
            }
        } else {
            self.theme = theme
            refreshTypingAttributesForSelection()
        }
        if heightBehavior == .autoGrow {
            notifyHeightChangeIfNeeded(force: true)
        }
        return true
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
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.toggleMarkAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head,
                markName: markName
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarToggleList(_ listType: String, isActive: Bool) {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = isActive
                ? EditorV2Shadow.unwrapFromListAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head
                )
                : EditorV2Shadow.wrapInListAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head,
                    listType: listType
                )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarToggleBlockquote() {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.toggleBlockquoteAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarToggleHeading(_ level: Int) {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        guard let level = UInt8(exactly: level), (1...6).contains(level) else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.toggleHeadingAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head,
                level: level
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarIndentListItem() {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.indentListItemAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarOutdentListItem() {
        guard prepareForToolbarCommand() else { return }
        guard let selection = currentScalarSelection() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.outdentListItemAtSelectionScalar(
                id: editorId,
                scalarAnchor: selection.anchor,
                scalarHead: selection.head
            )
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarInsertNode(_ nodeType: String) {
        guard prepareForToolbarCommand() else { return }
        performInterceptedInput {
            insertNodeInRust(nodeType)
        }
    }

    func performToolbarUndo() {
        guard prepareForToolbarCommand() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.undo(id: editorId)
            applyUpdateJSON(updateJSON)
        }
    }

    func performToolbarRedo() {
        guard prepareForToolbarCommand() else { return }
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.redo(id: editorId)
            applyUpdateJSON(updateJSON)
        }
    }

    private func prepareForToolbarCommand() -> Bool {
        guard editorId != 0 else { return false }
        guard isEditable else { return false }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return false }
        return prepareForExternalEditorUpdate()
    }

    /// Insert text at a scalar position via the Rust editor.
    private func insertTextInRust(_ text: String, at scalarPos: UInt32) {
        Self.inputLog.debug(
            "[rust.insertTextScalar] text=\(self.preview(text), privacy: .public) scalarPos=\(scalarPos) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.insertTextScalar(id: editorId, scalarPos: scalarPos, text: text)
        applyUpdateJSON(updateJSON)
    }

    private func replaceTextRangeInRust(from: UInt32, to: UInt32, with text: String) {
        Self.inputLog.debug(
            "[rust.replaceTextScalar] text=\(self.preview(text), privacy: .public) scalar=\(from)-\(to) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.replaceTextScalar(
            id: editorId,
            scalarFrom: from,
            scalarTo: to,
            text: text
        )
        applyUpdateJSON(updateJSON)
    }

    private func nativeTextMutationFromAuthorizedDiff(
        currentText: String
    ) -> NativeTextMutation? {
        let authorizedText = lastAuthorizedText
        guard currentText != authorizedText else { return nil }

        let authorized = authorizedText as NSString
        let current = currentText as NSString
        let sharedLength = min(authorized.length, current.length)
        var prefix = 0
        while prefix < sharedLength,
              authorized.character(at: prefix) == current.character(at: prefix) {
            prefix += 1
        }
        prefix = sharedUtf16ScalarBoundary(atOrBefore: prefix, in: authorizedText, and: currentText)

        var authorizedEnd = authorized.length
        var currentEnd = current.length
        while authorizedEnd > prefix,
              currentEnd > prefix,
              authorized.character(at: authorizedEnd - 1) == current.character(at: currentEnd - 1) {
            authorizedEnd -= 1
            currentEnd -= 1
        }
        authorizedEnd = utf16ScalarBoundary(atOrAfter: authorizedEnd, in: authorizedText)
        currentEnd = utf16ScalarBoundary(atOrAfter: currentEnd, in: currentText)

        let replacementLength = currentEnd - prefix
        guard replacementLength >= 0 else { return nil }
        let rawReplacementText = current.substring(
            with: NSRange(location: prefix, length: replacementLength)
        )

        let rawSelectionUtf16Range = selectedUtf16Range()
        let authorizedSelectionUtf16Range = lastAuthorizedSelectedUtf16Range
        let preservesAcceptedSpace = shouldPreserveAcceptedAutocorrectSpace(
            authorizedText: authorized,
            replacementStartUtf16: prefix,
            authorizedEndUtf16: authorizedEnd,
            currentEndUtf16: currentEnd,
            replacementText: rawReplacementText,
            rawSelectionUtf16Range: rawSelectionUtf16Range,
            authorizedSelectionUtf16Range: authorizedSelectionUtf16Range
        )
        let replacementText = preservesAcceptedSpace
            ? rawReplacementText + " "
            : rawReplacementText
        let selectionRangeForMapping = preservesAcceptedSpace
            ? NSRange(location: currentEnd + 1, length: 0)
            : rawSelectionUtf16Range
        let targetSelectionUtf16Range = targetSelectionUtf16RangeForNativeTextMutation(
            rawSelectionUtf16Range: selectionRangeForMapping,
            authorizedSelectionUtf16Range: authorizedSelectionUtf16Range,
            replacementStartUtf16: prefix,
            authorizedEndUtf16: authorizedEnd,
            currentEndUtf16: currentEnd + (preservesAcceptedSpace ? 1 : 0),
            currentTextUtf16Length: current.length + (preservesAcceptedSpace ? 1 : 0)
        )
        let authorizedReplacementUtf16Range = NSRange(
            location: prefix,
            length: authorizedEnd - prefix
        )
        let selectedScalarRange = targetSelectionUtf16Range.map { range in
            scalarRange(
                forUtf16Range: range,
                in: authorizedStorageApplying(
                    replacementText,
                    inUtf16Range: authorizedReplacementUtf16Range
                )
            )
        }
        let capturedAfterBlur = canAdoptNativeTextMutationAfterBlur()

        return NativeTextMutation(
            from: PositionBridge.utf16OffsetToScalar(prefix, in: lastAuthorizedAttributedTextStorage),
            to: PositionBridge.utf16OffsetToScalar(authorizedEnd, in: lastAuthorizedAttributedTextStorage),
            authorizedReplacementUtf16Range: authorizedReplacementUtf16Range,
            replacementText: replacementText,
            resultingText: currentText,
            authorizedText: authorizedText,
            selectionAnchor: selectedScalarRange?.from,
            selectionHead: selectedScalarRange?.to,
            authorizedSelectionUtf16Range: authorizedSelectionUtf16Range,
            rawSelectionUtf16Range: rawSelectionUtf16Range,
            acceptedSpaceCaretUtf16Offset: preservesAcceptedSpace ? currentEnd : nil,
            selectionRevision: selectionRevision,
            capturedWhileFirstResponder: isFirstResponder || capturedAfterBlur,
            capturedWhileEditable: isEditable,
            capturedAfterBlur: capturedAfterBlur,
            inputGeneration: nativeTextMutationGeneration
        )
    }

    private func shouldPreserveAcceptedAutocorrectSpace(
        authorizedText: NSString,
        replacementStartUtf16: Int,
        authorizedEndUtf16: Int,
        currentEndUtf16: Int,
        replacementText: String,
        rawSelectionUtf16Range: NSRange?,
        authorizedSelectionUtf16Range: NSRange?
    ) -> Bool {
        guard authorizedEndUtf16 > replacementStartUtf16,
              authorizedText.character(at: authorizedEndUtf16 - 1) == 0x20,
              !replacementText.isEmpty,
              let replacementLastCharacter = replacementText.last,
              replacementLastCharacter.isLetter || replacementLastCharacter.isNumber,
              let rawSelectionUtf16Range,
              rawSelectionUtf16Range.length == 0,
              rawSelectionUtf16Range.location == currentEndUtf16,
              let authorizedSelectionUtf16Range,
              authorizedSelectionUtf16Range.length == 0,
              authorizedSelectionUtf16Range.location == authorizedEndUtf16
        else {
            return false
        }

        let correctedText = authorizedText.substring(
            with: NSRange(
                location: replacementStartUtf16,
                length: authorizedEndUtf16 - replacementStartUtf16 - 1
            )
        )
        guard !correctedText.isEmpty,
              correctedText != replacementText,
              let correctedLastCharacter = correctedText.last
        else {
            return false
        }
        return correctedLastCharacter.isLetter || correctedLastCharacter.isNumber
    }

    private func nativeTextMutationWithCurrentSelection(
        _ mutation: NativeTextMutation
    ) -> NativeTextMutation {
        let currentSelectionUtf16Range = selectedUtf16Range()
        let didSelectionChangeAfterCapture = selectionRevision != mutation.selectionRevision
        let didCurrentRangeMoveAfterCapture: Bool
        if let currentSelectionUtf16Range,
           let rawSelectionUtf16Range = mutation.rawSelectionUtf16Range {
            didCurrentRangeMoveAfterCapture = !NSEqualRanges(
                currentSelectionUtf16Range,
                rawSelectionUtf16Range
            )
        } else {
            didCurrentRangeMoveAfterCapture = false
        }
        let currentSelectionDiffersFromAuthorized: Bool
        if let currentSelectionUtf16Range,
           let authorizedSelectionUtf16Range = mutation.authorizedSelectionUtf16Range {
            currentSelectionDiffersFromAuthorized = !NSEqualRanges(
                currentSelectionUtf16Range,
                authorizedSelectionUtf16Range
            )
        } else {
            currentSelectionDiffersFromAuthorized = currentSelectionUtf16Range != nil
        }
        let shouldUseCurrentSelection = currentSelectionUtf16Range != nil
            && (
                (didSelectionChangeAfterCapture && currentSelectionDiffersFromAuthorized)
                    || didCurrentRangeMoveAfterCapture
                    || mutation.rawSelectionUtf16Range == nil
            )
        // Same ruler as the capture path: the live view has lost the
        // structural attributes wherever the keyboard replaced a run, so a
        // caret converted against it maps as though the block were never
        // wrapped.
        let selectionConversionStorage = authorizedStorageApplying(
            mutation.replacementText,
            inUtf16Range: mutation.authorizedReplacementUtf16Range
        )
        let selectionRangeForConversion: NSRange?
        if let currentSelectionUtf16Range,
           currentSelectionUtf16Range.length == 0,
           currentSelectionUtf16Range.location == mutation.acceptedSpaceCaretUtf16Offset {
            selectionRangeForConversion = NSRange(
                location: currentSelectionUtf16Range.location + 1,
                length: 0
            )
        } else {
            selectionRangeForConversion = currentSelectionUtf16Range
        }
        let selectedScalarRange = shouldUseCurrentSelection
            ? selectionRangeForConversion.map {
                scalarRange(forUtf16Range: $0, in: selectionConversionStorage)
            }
            : nil
        return NativeTextMutation(
            from: mutation.from,
            to: mutation.to,
            authorizedReplacementUtf16Range: mutation.authorizedReplacementUtf16Range,
            replacementText: mutation.replacementText,
            resultingText: mutation.resultingText,
            authorizedText: mutation.authorizedText,
            selectionAnchor: selectedScalarRange?.from ?? mutation.selectionAnchor,
            selectionHead: selectedScalarRange?.to ?? mutation.selectionHead,
            authorizedSelectionUtf16Range: mutation.authorizedSelectionUtf16Range,
            rawSelectionUtf16Range: shouldUseCurrentSelection
                ? currentSelectionUtf16Range
                : mutation.rawSelectionUtf16Range,
            acceptedSpaceCaretUtf16Offset: mutation.acceptedSpaceCaretUtf16Offset,
            selectionRevision: shouldUseCurrentSelection
                ? selectionRevision
                : mutation.selectionRevision,
            capturedWhileFirstResponder: mutation.capturedWhileFirstResponder,
            capturedWhileEditable: mutation.capturedWhileEditable,
            capturedAfterBlur: mutation.capturedAfterBlur,
            inputGeneration: mutation.inputGeneration
        )
    }

    private func targetSelectionUtf16RangeForNativeTextMutation(
        rawSelectionUtf16Range: NSRange?,
        authorizedSelectionUtf16Range: NSRange?,
        replacementStartUtf16: Int,
        authorizedEndUtf16: Int,
        currentEndUtf16: Int,
        currentTextUtf16Length: Int
    ) -> NSRange? {
        guard let authorizedSelection = authorizedSelectionUtf16Range else {
            return clampedUtf16Range(rawSelectionUtf16Range, length: currentTextUtf16Length)
        }
        guard authorizedSelection.location != NSNotFound else {
            return clampedUtf16Range(rawSelectionUtf16Range, length: currentTextUtf16Length)
        }

        if let rawSelection = rawSelectionUtf16Range,
           rawSelection.location != NSNotFound,
           !NSEqualRanges(rawSelection, authorizedSelection) {
            return clampedUtf16Range(rawSelection, length: currentTextUtf16Length)
        }

        if authorizedSelection.length == 0 {
            let mappedOffset = mapCollapsedAuthorizedSelectionOffsetThroughNativeTextMutation(
                authorizedSelection.location,
                replacementStartUtf16: replacementStartUtf16,
                authorizedEndUtf16: authorizedEndUtf16,
                currentEndUtf16: currentEndUtf16
            )
            let clampedOffset = min(max(mappedOffset, 0), currentTextUtf16Length)
            return NSRange(location: clampedOffset, length: 0)
        }

        let mappedStart = mapAuthorizedSelectionOffsetThroughNativeTextMutation(
            authorizedSelection.location,
            replacementStartUtf16: replacementStartUtf16,
            authorizedEndUtf16: authorizedEndUtf16,
            currentEndUtf16: currentEndUtf16,
            isRangeStart: true
        )
        let mappedEnd = mapAuthorizedSelectionOffsetThroughNativeTextMutation(
            NSMaxRange(authorizedSelection),
            replacementStartUtf16: replacementStartUtf16,
            authorizedEndUtf16: authorizedEndUtf16,
            currentEndUtf16: currentEndUtf16,
            isRangeStart: false
        )
        let start = min(mappedStart, mappedEnd)
        let end = max(mappedStart, mappedEnd)
        let clampedStart = min(max(start, 0), currentTextUtf16Length)
        let clampedEnd = min(max(end, 0), currentTextUtf16Length)
        return NSRange(location: clampedStart, length: max(0, clampedEnd - clampedStart))
    }

    private func clampedUtf16Range(_ range: NSRange?, length: Int) -> NSRange? {
        guard let range, range.location != NSNotFound else { return nil }
        let start = min(max(range.location, 0), length)
        let end = min(max(NSMaxRange(range), 0), length)
        return NSRange(location: min(start, end), length: abs(end - start))
    }

    private func mapCollapsedAuthorizedSelectionOffsetThroughNativeTextMutation(
        _ offset: Int,
        replacementStartUtf16: Int,
        authorizedEndUtf16: Int,
        currentEndUtf16: Int
    ) -> Int {
        // UIKit can leave a stale caret at the insertion point during autocomplete.
        // A collapsed authorized caret should stay collapsed after the inserted text.
        if replacementStartUtf16 == authorizedEndUtf16,
           offset == replacementStartUtf16,
           currentEndUtf16 > replacementStartUtf16 {
            return currentEndUtf16
        }
        if offset <= replacementStartUtf16 {
            return offset
        }
        if offset < authorizedEndUtf16 {
            return currentEndUtf16
        }
        return offset + currentEndUtf16 - authorizedEndUtf16
    }

    private func mapAuthorizedSelectionOffsetThroughNativeTextMutation(
        _ offset: Int,
        replacementStartUtf16: Int,
        authorizedEndUtf16: Int,
        currentEndUtf16: Int,
        isRangeStart: Bool
    ) -> Int {
        if offset <= replacementStartUtf16 {
            return offset
        }
        if offset >= authorizedEndUtf16 {
            return offset + currentEndUtf16 - authorizedEndUtf16
        }
        return isRangeStart ? replacementStartUtf16 : currentEndUtf16
    }

    private func isUtf16ScalarBoundary(_ offset: Int, in text: String) -> Bool {
        guard offset >= 0, offset <= text.utf16.count else { return false }
        let utf16Index = text.utf16.index(text.utf16.startIndex, offsetBy: offset)
        return String.Index(utf16Index, within: text) != nil
    }

    private func utf16ScalarBoundary(atOrBefore offset: Int, in text: String) -> Int {
        var candidate = min(max(offset, 0), text.utf16.count)
        while candidate > 0, !isUtf16ScalarBoundary(candidate, in: text) {
            candidate -= 1
        }
        return candidate
    }

    private func utf16ScalarBoundary(atOrAfter offset: Int, in text: String) -> Int {
        var candidate = min(max(offset, 0), text.utf16.count)
        while candidate < text.utf16.count, !isUtf16ScalarBoundary(candidate, in: text) {
            candidate += 1
        }
        return candidate
    }

    private func sharedUtf16ScalarBoundary(atOrBefore offset: Int, in lhs: String, and rhs: String) -> Int {
        var candidate = min(max(offset, 0), lhs.utf16.count, rhs.utf16.count)
        while candidate > 0,
              (!isUtf16ScalarBoundary(candidate, in: lhs) || !isUtf16ScalarBoundary(candidate, in: rhs)) {
            candidate -= 1
        }
        return candidate
    }

    private func shouldAdoptNativeTextStorageMutation(
        _ mutation: NativeTextMutation,
        allowAfterBlur: Bool = false
    ) -> Bool {
        if isFirstResponder && isEditable {
            return true
        }
        return allowAfterBlur
            && mutation.capturedAfterBlur
            && mutation.inputGeneration == nativeTextMutationGeneration
            && canAdoptNativeTextMutationAfterBlur()
            && mutation.capturedWhileFirstResponder
            && mutation.capturedWhileEditable
    }

    private func canAdoptNativeTextMutationAfterBlur() -> Bool {
        guard let deadline = nativeTextMutationAfterBlurDeadline else {
            return false
        }
        guard nativeTextMutationAfterBlurGeneration == nativeTextMutationGeneration else {
            clearNativeTextMutationAfterBlurWindow()
            return false
        }
        guard ProcessInfo.processInfo.systemUptime <= deadline else {
            clearNativeTextMutationAfterBlurWindow()
            return false
        }
        return true
    }

    private func clearNativeTextMutationAfterBlurWindow() {
        nativeTextMutationAfterBlurDeadline = nil
        nativeTextMutationAfterBlurGeneration = nil
    }

    private func advanceNativeTextMutationGeneration() {
        nativeTextMutationGeneration &+= 1
        clearNativeTextMutationAfterBlurWindow()
    }

    private func resetPendingNativeTextMutationState() {
        pendingNativeTextMutation = nil
        nativeTextMutationCommitScheduled = false
        advanceNativeTextMutationGeneration()
    }

    func expireNativeTextMutationAfterBlurDeadlineForTesting() {
        nativeTextMutationAfterBlurDeadline = ProcessInfo.processInfo.systemUptime - 0.001
    }

    @discardableResult
    func cancelExternalTextCompositionForLifecycleIfNeeded() -> String? {
        finishExternalTextComposition(
            cause: "lifecycle",
            finalText: nil,
            cancel: true
        )?.resultJSON
    }

    @discardableResult
    func discardTransientNativeInputForEditorRebind() -> String? {
        let externalCompositionResultJSON =
            cancelExternalTextCompositionForLifecycleIfNeeded()
        resetPendingNativeTextMutationState()
        lastAuthorizedSelectedUtf16Range = nil
        logicalSelectionScalarRange = nil
        logicalSelectionUtf16Range = nil
        clearPendingInputTraitRetry()
        markedTextReplacementScalarRange = nil
        markedTextReplacementUtf16Range = nil
        markedTextCompositionText = nil
        markedTextCompositionIsExplicitlyEmpty = false
        isComposing = false
        return externalCompositionResultJSON
    }

    @discardableResult
    private func flushPendingNativeTextMutationCommitIfNeeded() -> Bool {
        drainPendingNativeTextMutation(
            allowAfterBlur: false,
            allowWhileIntercepting: true
        ).ready
    }

    struct ExternalEditorUpdatePreparation {
        let ready: Bool
        /// When preflight committed local UIKit state, the adapter already
        /// rendered and adopted this current atomic update. Reuse it instead
        /// of reading Rust again for the same external update operation.
        let adoptedUpdateJSON: String?
    }

    private struct ActiveCompositionPreparation {
        let ready: Bool
        let adoptedUpdateJSON: String?
    }

    @discardableResult
    func prepareForExternalEditorUpdate() -> Bool {
        prepareForExternalEditorUpdateResult().ready
    }

    func prepareForExternalEditorUpdateResult() -> ExternalEditorUpdatePreparation {
        let composition = prepareActiveCompositionForExternalMutation()
        guard composition.ready else {
            return ExternalEditorUpdatePreparation(ready: false, adoptedUpdateJSON: nil)
        }
        let nativeMutation = drainPendingNativeTextMutation(
            allowAfterBlur: true,
            allowWhileIntercepting: true
        )
        return ExternalEditorUpdatePreparation(
            ready: nativeMutation.ready,
            adoptedUpdateJSON: composition.adoptedUpdateJSON ?? nativeMutation.adoptedUpdateJSON
        )
    }

    @discardableResult
    func prepareForExternalEditorCommand() -> (ready: Bool, updateJSON: String?, blockedReason: String?) {
        let previousEditorId = editorId
        let previousAuthorizedText = lastAuthorizedText
        let previousStateJSON = previousEditorId != 0 ? EditorV2Shadow.getCurrentState(id: previousEditorId) : nil
        guard prepareForExternalEditorUpdate() else {
            return (false, nil, "composition")
        }
        guard editorId != 0 else {
            return (true, nil, nil)
        }
        let currentStateJSON = EditorV2Shadow.getCurrentState(id: editorId)
        guard lastAuthorizedText != previousAuthorizedText
                || previousEditorId != editorId
                || previousStateJSON != currentStateJSON
        else {
            return (true, nil, nil)
        }
        return (true, currentStateJSON, nil)
    }

    private func prepareActiveCompositionForExternalMutation() -> ActiveCompositionPreparation {
        if externalTextComposition != nil {
            guard let finished = finishExternalTextComposition(
                cause: "documentChange",
                finalText: nil,
                cancel: false
            ) else {
                return ActiveCompositionPreparation(ready: false, adoptedUpdateJSON: nil)
            }
            return ActiveCompositionPreparation(
                ready: finished.succeeded,
                adoptedUpdateJSON: finished.adoptedUpdateJSON
            )
        }
        guard isComposing else {
            return ActiveCompositionPreparation(ready: true, adoptedUpdateJSON: nil)
        }

        let composedText = validatedTrackedMarkedTextForCommit()
        let replacementRange = trackedMarkedTextReplacementRange()
        finishTransientMarkedTextMutation()

        guard shouldCommitMarkedText(composedText, replacementRange: replacementRange) else {
            restoreAuthorizedTextAfterCancelledCompositionIfNeeded()
            return ActiveCompositionPreparation(ready: false, adoptedUpdateJSON: nil)
        }

        return ActiveCompositionPreparation(
            ready: true,
            adoptedUpdateJSON: commitMarkedText(composedText ?? "", replacementRange: replacementRange)
        )
    }

    @discardableResult
    private func drainPendingNativeTextMutation(
        allowAfterBlur: Bool,
        allowWhileIntercepting: Bool
    ) -> NativeTextMutationDrainResult {
        guard nativeTextMutationCommitScheduled
                || pendingNativeTextMutation != nil
                || (!isComposing && markedTextRange == nil && textStorage.string != lastAuthorizedText)
        else {
            return NativeTextMutationDrainResult(ready: true, adoptedUpdateJSON: nil)
        }

        nativeTextMutationCommitScheduled = false
        let currentText = textStorage.string
        let mutation: NativeTextMutation?
        if let pendingNativeTextMutation,
           pendingNativeTextMutation.resultingText == currentText,
           pendingNativeTextMutation.authorizedText == lastAuthorizedText
        {
            mutation = nativeTextMutationWithCurrentSelection(pendingNativeTextMutation)
        } else {
            mutation = nativeTextMutationFromAuthorizedDiff(currentText: currentText)
        }

        guard let mutation else {
            pendingNativeTextMutation = nil
            return NativeTextMutationDrainResult(ready: true, adoptedUpdateJSON: nil)
        }

        switch commitNativeTextMutationIfPossible(
            mutation,
            allowAfterBlur: allowAfterBlur,
            allowWhileIntercepting: allowWhileIntercepting
        ) {
        case .committed(let adoptedUpdateJSON):
            pendingNativeTextMutation = nil
            return NativeTextMutationDrainResult(ready: true, adoptedUpdateJSON: adoptedUpdateJSON)
        case .rejected:
            pendingNativeTextMutation = nil
            return NativeTextMutationDrainResult(ready: true, adoptedUpdateJSON: nil)
        case .deferred:
            pendingNativeTextMutation = mutation
            return NativeTextMutationDrainResult(ready: false, adoptedUpdateJSON: nil)
        }
    }

    private func scheduleNativeTextMutationCommit(_ mutation: NativeTextMutation) {
        pendingNativeTextMutation = mutation
        guard !nativeTextMutationCommitScheduled else { return }

        nativeTextMutationCommitScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            _ = self.drainPendingNativeTextMutation(
                allowAfterBlur: true,
                allowWhileIntercepting: true
            )
        }
    }

    @discardableResult
    private func commitNativeTextMutationIfPossible(
        _ mutation: NativeTextMutation,
        allowAfterBlur: Bool,
        allowWhileIntercepting: Bool
    ) -> NativeTextMutationCommitResult {
        guard editorId != 0 else {
            return .rejected
        }

        guard !isApplyingRustState,
              (!isInterceptingInput || allowWhileIntercepting),
              !isComposing
        else {
            return .deferred
        }

        guard shouldAdoptNativeTextStorageMutation(mutation, allowAfterBlur: allowAfterBlur) else {
            if textStorage.string != lastAuthorizedText {
                scheduleReconciliationFromRust()
            }
            return .rejected
        }

        guard textStorage.string == mutation.resultingText else {
            if let refreshedMutation = nativeTextMutationFromAuthorizedDiff(currentText: textStorage.string) {
                return commitNativeTextMutationIfPossible(
                    refreshedMutation,
                    allowAfterBlur: allowAfterBlur,
                    allowWhileIntercepting: allowWhileIntercepting
                )
            }
            return .rejected
        }

        guard mutation.authorizedText == lastAuthorizedText else {
            if let refreshedMutation = nativeTextMutationFromAuthorizedDiff(currentText: textStorage.string) {
                return commitNativeTextMutationIfPossible(
                    refreshedMutation,
                    allowAfterBlur: allowAfterBlur,
                    allowWhileIntercepting: allowWhileIntercepting
                )
            }
            return .rejected
        }

        var adoptedUpdateJSON: String?
        performInterceptedInput(flushPendingNativeTextMutation: false) {
            guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else { return }
            if mutation.from == mutation.to {
                guard !mutation.replacementText.isEmpty else { return }
                adoptedUpdateJSON = adapter.insertText(mutation.replacementText, atScalar: mutation.from)
            } else if mutation.replacementText.isEmpty {
                adoptedUpdateJSON = adapter.deleteScalarRange(from: mutation.from, to: mutation.to)
            } else {
                adoptedUpdateJSON = adapter.replaceTextRange(
                    from: mutation.from,
                    to: mutation.to,
                    with: mutation.replacementText
                )
            }
            guard let adoptedUpdateJSON else { return }
            applyUpdateJSON(adoptedUpdateJSON)
            restoreNativeTextMutationSelectionIfNeeded(mutation)
        }
        guard let adoptedUpdateJSON else { return .rejected }
        if mutation.capturedAfterBlur {
            clearNativeTextMutationAfterBlurWindow()
        }
        return .committed(adoptedUpdateJSON: adoptedUpdateJSON)
    }

    private func restoreNativeTextMutationSelectionIfNeeded(_ mutation: NativeTextMutation) {
        guard let anchor = mutation.selectionAnchor,
              let head = mutation.selectionHead,
              editorId != 0
        else {
            return
        }

        let startUtf16 = PositionBridge.scalarToUtf16Offset(min(anchor, head), in: self)
        let endUtf16 = PositionBridge.scalarToUtf16Offset(max(anchor, head), in: self)
        let targetRange = NSRange(location: startUtf16, length: max(0, endUtf16 - startUtf16))
        logicalSelectionScalarRange = (anchor: anchor, head: head)
        if selectedRange != targetRange {
            selectedRange = targetRange
            noteSelectionDidChange()
        }
        logicalSelectionUtf16Range = selectedRange
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: anchor, scalarHead: head)
        recordAuthorizedSelectionIfPossible()
        refreshTypingAttributesForSelection()
        let docAnchor = EditorV2Shadow.scalarToDoc(id: editorId, scalar: anchor)
        let docHead = EditorV2Shadow.scalarToDoc(id: editorId, scalar: head)
        editorDelegate?.editorTextView(self, selectionDidChange: docAnchor, head: docHead)
    }

    private func insertNodeInRust(_ nodeType: String) {
        guard let selection = currentScalarSelection() else { return }
        Self.inputLog.debug(
            "[rust.insertNode] nodeType=\(nodeType, privacy: .public) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.insertNodeAtSelectionScalar(
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
        let updateJSON = EditorV2Shadow.deleteScalarRange(id: editorId, scalarFrom: from, scalarTo: to)
        applyUpdateJSON(updateJSON)
    }

    private func deleteBackwardAtSelectionScalarInRust(anchor: UInt32, head: UInt32) {
        Self.inputLog.debug(
            "[rust.deleteBackwardAtSelectionScalar] scalar=\(anchor)-\(head) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.deleteBackwardAtSelectionScalar(
            id: editorId,
            scalarAnchor: anchor,
            scalarHead: head
        )
        applyUpdateJSON(updateJSON)
    }

    private func toggleTaskItemCheckedAtSelectionScalarInRust(anchor: UInt32, head: UInt32) {
        Self.inputLog.debug(
            "[rust.toggleTaskItemCheckedAtSelectionScalar] scalar=\(anchor)-\(head) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.toggleTaskItemCheckedAtSelectionScalar(
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
        let updateJSON = EditorV2Shadow.deleteRange(id: editorId, from: from, to: to)
        applyUpdateJSON(updateJSON)
    }

    func currentLogicalScalarSelection() -> (anchor: UInt32, head: UInt32)? {
        guard let range = selectedTextRange else { return nil }
        let scalarRange = PositionBridge.textRangeToScalarRange(range, in: self)
        if let logicalSelectionScalarRange,
           min(logicalSelectionScalarRange.anchor, logicalSelectionScalarRange.head) == scalarRange.from,
           max(logicalSelectionScalarRange.anchor, logicalSelectionScalarRange.head) == scalarRange.to
        {
            return logicalSelectionScalarRange
        }
        // A lone empty block parks the caret ahead of the placeholder so UIKit
        // offers autocapitalization, which makes the UIKit caret disagree with
        // the engine scalar by exactly the placeholder. Matching the range that
        // nudge produced keeps the engine authoritative.
        //
        // Confined to that block: a matching UTF-16 range is not evidence a
        // scalar is current. Wrapping a line in a list shifts every scalar
        // while leaving UTF-16 offsets untouched, so a stale pre-wrap caret
        // would match here and place the next character inside the word.
        if let logicalSelectionScalarRange,
           let logicalSelectionUtf16Range,
           logicalSelectionUtf16Range == selectedRange,
           isLoneEmptyPlaceholderBlock
        {
            return logicalSelectionScalarRange
        }
        logicalSelectionScalarRange = nil
        logicalSelectionUtf16Range = nil
        return (anchor: scalarRange.from, head: scalarRange.to)
    }

    private func currentScalarSelection() -> (anchor: UInt32, head: UInt32)? {
        currentLogicalScalarSelection()
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

        let docPos = v2ExactUInt32(attrs[RenderBridgeAttributes.docPos] as? NSNumber)
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
            let attributeDocPos = v2ExactUInt32(attrs[RenderBridgeAttributes.docPos] as? NSNumber)
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
            let updateJSON = EditorV2Shadow.resizeImageAtDocPos(
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
            let updateJSON = EditorV2Shadow.deleteAndSplitScalar(
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
        let updateJSON = EditorV2Shadow.splitBlockScalar(id: editorId, scalarPos: scalarPos)
        applyUpdateJSON(updateJSON)
    }

    /// Paste HTML content through Rust.
    @discardableResult
    private func pasteHTML(_ html: String, detectContentChange: Bool = false) -> Bool {
        let previousHTML = detectContentChange ? EditorV2Shadow.getHtml(id: editorId) : nil
        syncCurrentUIKitSelectionToRust()
        Self.inputLog.debug(
            "[rust.pasteHTML] html=\(self.preview(html), privacy: .public) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.insertContentHtml(id: editorId, html: html)
        applyUpdateJSON(updateJSON)
        guard let previousHTML else { return true }
        return EditorV2Shadow.getHtml(id: editorId) != previousHTML
    }

    private func syncCurrentUIKitSelectionToRust() {
        guard editorId != 0, let range = selectedTextRange else { return }
        let anchor = PositionBridge.textViewToScalar(range.start, in: self)
        let head = PositionBridge.textViewToScalar(range.end, in: self)
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: anchor, scalarHead: head)
    }

    /// Paste plain text through Rust.
    private func pastePlainText(_ text: String) {
        if let selectedRange = selectedTextRange, !selectedRange.isEmpty {
            // Atomically replace the selection with the pasted text.
            let range = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
            Self.inputLog.debug(
                "[rust.pastePlainText.replace] text=\(self.preview(text), privacy: .public) scalar=\(range.from)-\(range.to) selection=\(self.selectionSummary(), privacy: .public)"
            )
            let updateJSON = EditorV2Shadow.replaceTextScalar(
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
        guard lhs.count == rhs.count else { return false }
        for (lhsElement, rhsElement) in zip(lhs, rhs) {
            guard renderElementEquals(lhsElement, rhsElement) else { return false }
        }
        return true
    }

    private func voidNodeMetadata(in renderBlock: [[String: Any]]) -> [(type: String, docPos: NSNumber)]? {
        let nodes = renderBlock.compactMap { element -> (type: String, docPos: NSNumber)? in
            guard let elementType = element["type"] as? String,
                  elementType == "voidInline" || elementType == "voidBlock",
                  let nodeType = element["nodeType"] as? String,
                  let docPos = RenderBridge.jsonUInt32(element["docPos"])
            else {
                return nil
            }
            return (nodeType, NSNumber(value: docPos))
        }
        return nodes
    }

    private func refreshRetainedPositionalMetadata(
        startingAt retainedStart: Int,
        updatedRenderBlocks: [[[String: Any]]]
    ) -> Bool {
        guard let currentTopLevelChildMetadata,
              currentTopLevelChildMetadata.count == updatedRenderBlocks.count,
              retainedStart >= 0,
              retainedStart <= currentTopLevelChildMetadata.count
        else {
            return false
        }

        for index in retainedStart..<currentTopLevelChildMetadata.count {
            let start = currentTopLevelChildMetadata[index].startOffset
            let end = index + 1 < currentTopLevelChildMetadata.count
                ? currentTopLevelChildMetadata[index + 1].startOffset
                : textStorage.length
            guard start >= 0, end >= start, end <= textStorage.length else { return false }
            let range = NSRange(location: start, length: end - start)
            textStorage.addAttribute(
                RenderBridgeAttributes.topLevelChildIndex,
                value: NSNumber(value: index),
                range: range
            )

            guard let expectedVoidNodes = voidNodeMetadata(in: updatedRenderBlocks[index]) else {
                return false
            }
            var actualVoidNodes: [(type: String, range: NSRange)] = []
            textStorage.enumerateAttribute(
                RenderBridgeAttributes.voidNodeType,
                in: range,
                options: [.longestEffectiveRangeNotRequired]
            ) { value, nodeRange, _ in
                if let type = value as? String {
                    actualVoidNodes.append((type, nodeRange))
                }
            }
            guard actualVoidNodes.count == expectedVoidNodes.count,
                  zip(actualVoidNodes, expectedVoidNodes).allSatisfy({ $0.type == $1.type })
            else {
                return false
            }
            for (actual, expected) in zip(actualVoidNodes, expectedVoidNodes) {
                textStorage.addAttribute(
                    RenderBridgeAttributes.docPos,
                    value: expected.docPos,
                    range: actual.range
                )
            }
        }
        return true
    }

    private func renderElementEquals(_ lhs: [String: Any], _ rhs: [String: Any]) -> Bool {
        if (lhs as NSDictionary).isEqual(to: rhs) { return true }
        var comparableLhs = lhs
        var comparableRhs = rhs
        comparableLhs.removeValue(forKey: "docPos")
        comparableRhs.removeValue(forKey: "docPos")
        comparableLhs.removeValue(forKey: "topLevelChildIndex")
        comparableRhs.removeValue(forKey: "topLevelChildIndex")
        return (comparableLhs as NSDictionary).isEqual(to: comparableRhs)
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
            let patchEntries = renderedPatchMetadata.entries.prefix(patch.renderBlocks.count)
            guard patchEntries.count == patch.renderBlocks.count else {
                currentTopLevelChildMetadata = nil
                return
            }
            newEntries = patchEntries.map { entry in
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
        positionCacheUpdate: PositionCacheUpdate = .scan,
        authorizedReplaceRange: NSRange? = nil,
        authorizedReplacementText: String? = nil,
        authorizedReplacementAttributedText: NSAttributedString? = nil
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
        textStorage.delegate = nil
        delegate = nil
        defer {
            textStorage.delegate = previousTextStorageDelegate
            ensureInternalTextViewDelegate()
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
        let snapshotReplaceRange = authorizedReplaceRange ?? replaceRange
        let snapshotReplacementText = authorizedReplacementText ?? attrStr.string
        let snapshotReplacementAttributedText = authorizedReplacementAttributedText ?? attrStr
        if let snapshotReplaceRange,
           snapshotReplaceRange.location >= 0,
           snapshotReplaceRange.location + snapshotReplaceRange.length <= lastAuthorizedTextStorage.length
        {
            lastAuthorizedTextStorage.replaceCharacters(
                in: snapshotReplaceRange,
                with: snapshotReplacementText
            )
            lastAuthorizedAttributedTextStorage.replaceCharacters(
                in: snapshotReplaceRange,
                with: snapshotReplacementAttributedText
            )
        } else {
            lastAuthorizedTextStorage.setString(replaceRange == nil ? snapshotReplacementText : textStorage.string)
            let fallbackAttributedSnapshot = replaceRange == nil
                ? snapshotReplacementAttributedText
                : NSAttributedString(attributedString: textStorage)
            lastAuthorizedAttributedTextStorage.setAttributedString(fallbackAttributedSnapshot)
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
        // `docPos` and `topLevelChildIndex` are positional metadata consumed
        // by atom actions and future patch-range resolution. Trimming across
        // either change would preserve stale attributed metadata.
        NSDictionary(dictionary: lhs).isEqual(to: rhs)
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
        let existingRawString = existing.string
        let replacementRawString = replacement.string
        let existingString = existingRawString as NSString
        let replacementString = replacementRawString as NSString
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
        prefix = sharedUtf16ScalarBoundary(atOrBefore: prefix, in: existingRawString, and: replacementRawString)
        while suffix > 0 {
            let existingSuffixStart = existing.length - suffix
            let replacementSuffixStart = replacement.length - suffix
            if suffix <= sharedLength - prefix,
               isUtf16ScalarBoundary(existingSuffixStart, in: existingRawString),
               isUtf16ScalarBoundary(replacementSuffixStart, in: replacementRawString) {
                break
            }
            suffix -= 1
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
        let attrStr = withImageLoadOwner {
            RenderBridge.renderBlocks(
                fromArray: patch.renderBlocks,
                startIndex: patch.startIndex,
                includeLeadingInterBlockSeparator: patch.startIndex > 0,
                // Replacement ranges already extend to the following child
                // boundary, whose separator is rendered by that retained
                // child. A trailing separator is therefore needed only when
                // inserting before the first existing child.
                includeTrailingInterBlockSeparator: patch.startIndex == 0
                    && patch.deleteCount == 0
                    && !(currentTopLevelChildMetadata?.isEmpty ?? true),
                baseFont: baseFont,
                textColor: baseTextColor,
                theme: theme
            )
        }
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
            positionCacheUpdate: positionCacheUpdate,
            authorizedReplaceRange: fullReplaceRange,
            authorizedReplacementText: attrStr.string,
            authorizedReplacementAttributedText: attrStr
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
    @discardableResult
    func applyUpdateJSON(_ updateJSON: String, notifyDelegate: Bool = true) -> Bool {
        ensureInternalTextViewDelegate()
        let totalStartedAt = DispatchTime.now().uptimeNanoseconds
        let parseStartedAt = totalStartedAt
        guard let data = updateJSON.data(using: .utf8),
              let update = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return false }
        let parseNanos = DispatchTime.now().uptimeNanoseconds - parseStartedAt
        resetPendingNativeTextMutationState()

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
            if let currentRenderBlocks,
               let resolvedRenderBlocks
            {
                deriveRenderPatch(from: currentRenderBlocks, to: resolvedRenderBlocks)
            } else {
                nil
            }
        let renderPatch: ParsedRenderPatch? = if let explicitRenderPatch {
            explicitRenderPatch
        } else if case let .patch(derivedPatch)? = derivedRenderPatch {
            derivedPatch
        } else {
            nil
        }
        let shouldSkipRender = if case .unchanged? = derivedRenderPatch {
            textStorage.string == lastAuthorizedText
                && lastAppliedRenderAppearanceRevision == renderAppearanceRevision
        } else {
            false
        }

        let patchTrace: PatchApplyTrace? = if lastAppliedRenderAppearanceRevision == renderAppearanceRevision {
            renderPatch.map(applyRenderPatchIfPossible)
        } else {
            nil
        }
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
                attrStr = withImageLoadOwner {
                    RenderBridge.renderBlocks(
                        fromArray: resolvedRenderBlocks,
                        baseFont: baseFont,
                        textColor: baseTextColor,
                        theme: theme
                    )
                }
                currentRenderBlocks = resolvedRenderBlocks
            } else if let renderElements {
                attrStr = withImageLoadOwner {
                    RenderBridge.renderElements(
                        fromArray: renderElements,
                        baseFont: baseFont,
                        textColor: baseTextColor,
                        theme: theme
                    )
                }
                currentRenderBlocks = nil
            } else {
                return false
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
        if appliedPatch,
           let renderPatch,
           let resolvedRenderBlocks,
           !refreshRetainedPositionalMetadata(
               startingAt: renderPatch.startIndex + renderPatch.renderBlocks.count,
               updatedRenderBlocks: resolvedRenderBlocks
           )
        {
            // The preflight is deliberately conservative: a partial metadata
            // refresh must never leave a retained atom targeting stale state.
            // The next update will take the full safe path after cache reset.
            currentTopLevelChildMetadata = nil
        }

        // The core is the authority on empty state; adopt it before the
        // placeholder is reconsidered.
        coreReportedDocumentIsEmpty = update["documentIsEmpty"] as? Bool
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
                patchStartIndex: renderPatch?.startIndex,
                patchDeleteCount: renderPatch?.deleteCount,
                patchRenderBlockCount: renderPatch?.renderBlocks.count,
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
        recordAuthorizedSelectionIfPossible()
        Self.updateLog.debug(
            "[applyUpdateJSON.end] finalSelection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )

        // Notify the delegate.
        if notifyDelegate {
            editorDelegate?.editorTextView(self, didReceiveUpdate: updateJSON)
        }
        return true
    }

    /// Apply a render JSON string (just render elements, no update wrapper).
    ///
    /// Used for initial content loading (set_html / set_json return render
    /// elements directly, not wrapped in an EditorUpdate).
    func applyRenderJSON(_ renderJSON: String) {
        ensureInternalTextViewDelegate()
        resetPendingNativeTextMutationState()
        Self.updateLog.debug(
            "[applyRenderJSON.begin] before=\(self.textSnapshotSummary(), privacy: .public)"
        )
        let attrStr = withImageLoadOwner {
            RenderBridge.renderElements(
                fromJSON: renderJSON,
                baseFont: baseFont,
                textColor: baseTextColor,
                theme: theme
            )
        }
        _ = applyAttributedRender(attrStr, usedPatch: false)
        currentRenderBlocks = nil
        refreshTopLevelChildMetadata(from: attrStr)
        lastAppliedRenderAppearanceRevision = renderAppearanceRevision

        // A bare render carries no editor update, so there is no authoritative
        // empty state to adopt; fall back until the next update supplies one.
        coreReportedDocumentIsEmpty = nil
        refreshPlaceholderVisibility()
        _ = performPostApplyMaintenance()
        recordAuthorizedSelectionIfPossible()
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
        delegate = nil
        defer {
            ensureInternalTextViewDelegate()
            isApplyingRustState = false
        }

        switch type {
        case "text":
            let resolveStartedAt = DispatchTime.now().uptimeNanoseconds
            guard let anchor = v2ExactUInt32(selection["anchor"] as? NSNumber),
                  let head = v2ExactUInt32(selection["head"] as? NSNumber)
            else {
                return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
            }
            // anchor/head from Rust are document positions; convert to scalar offsets first.
            let anchorScalar: UInt32
            if let rawAnchorScalar = selection["anchorScalar"] {
                guard let exactAnchorScalar = v2ExactUInt32(rawAnchorScalar as? NSNumber) else {
                    return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
                }
                anchorScalar = exactAnchorScalar
            } else {
                anchorScalar = EditorV2Shadow.docToScalar(id: editorId, docPos: anchor)
            }
            let headScalar: UInt32
            if let rawHeadScalar = selection["headScalar"] {
                guard let exactHeadScalar = v2ExactUInt32(rawHeadScalar as? NSNumber) else {
                    return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
                }
                headScalar = exactHeadScalar
            } else {
                headScalar = EditorV2Shadow.docToScalar(id: editorId, docPos: head)
            }
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
            logicalSelectionScalarRange = (anchor: anchorScalar, head: headScalar)
            if anchorScalar == headScalar {
                let endPos = position(from: beginningOfDocument, offset: endUtf16) ?? endOfDocument
                if let adjustedPosition = autocapitalizationFriendlyEmptyBlockPosition(for: endPos) {
                    let adjustedOffset = offset(from: beginningOfDocument, to: adjustedPosition)
                    let adjustedRange = NSRange(location: adjustedOffset, length: 0)
                    if selectedRange != adjustedRange {
                        selectedRange = adjustedRange
                        noteSelectionDidChange()
                    }
                } else {
                    let targetRange = NSRange(location: endUtf16, length: 0)
                    if selectedRange != targetRange {
                        selectedRange = targetRange
                        noteSelectionDidChange()
                    }
                }
            } else {
                let targetRange = NSRange(location: startUtf16, length: endUtf16 - startUtf16)
                if selectedRange != targetRange {
                    selectedRange = targetRange
                    noteSelectionDidChange()
                }
            }
            logicalSelectionUtf16Range = selectedRange
            let assignmentNanos = DispatchTime.now().uptimeNanoseconds - assignmentStartedAt
            let chromeStartedAt = DispatchTime.now().uptimeNanoseconds
            showNativeSelectionChromeIfNeeded()
            let chromeNanos = DispatchTime.now().uptimeNanoseconds - chromeStartedAt
            Self.selectionLog.debug(
                "[applySelectionFromJSON.text] doc=\(anchor)-\(head) scalar=\(anchorScalar)-\(headScalar) final=\(self.selectionSummary(), privacy: .public)"
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
            guard let pos = v2ExactUInt32(selection["pos"] as? NSNumber) else {
                return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
            }
            // pos from Rust is a document position; convert to scalar offset.
            let posScalar: UInt32
            if let rawPosScalar = selection["posScalar"] {
                guard let exactPosScalar = v2ExactUInt32(rawPosScalar as? NSNumber) else {
                    return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
                }
                posScalar = exactPosScalar
            } else {
                posScalar = EditorV2Shadow.docToScalar(id: editorId, docPos: pos)
            }
            let startUtf16 = PositionBridge.scalarToUtf16Offset(posScalar, in: self)
            let targetRange = NSRange(location: startUtf16, length: 1)
            let resolveNanos = DispatchTime.now().uptimeNanoseconds - resolveStartedAt
            let assignmentStartedAt = DispatchTime.now().uptimeNanoseconds
            logicalSelectionScalarRange = nil
            logicalSelectionUtf16Range = nil
            if selectedRange != targetRange {
                selectedRange = targetRange
                noteSelectionDidChange()
            }
            let assignmentNanos = DispatchTime.now().uptimeNanoseconds - assignmentStartedAt
            let chromeStartedAt = DispatchTime.now().uptimeNanoseconds
            refreshNativeSelectionChromeVisibility()
            let chromeNanos = DispatchTime.now().uptimeNanoseconds - chromeStartedAt
            Self.selectionLog.debug(
                "[applySelectionFromJSON.node] doc=\(pos) scalar=\(posScalar) final=\(self.selectionSummary(), privacy: .public)"
            )
            return SelectionApplyTrace(
                totalNanos: DispatchTime.now().uptimeNanoseconds - totalStartedAt,
                resolveNanos: resolveNanos,
                assignmentNanos: assignmentNanos,
                chromeNanos: chromeNanos
            )

        case "all":
            let assignmentStartedAt = DispatchTime.now().uptimeNanoseconds
            logicalSelectionScalarRange = nil
            logicalSelectionUtf16Range = nil
            selectedTextRange = textRange(from: beginningOfDocument, to: endOfDocument)
            noteSelectionDidChange()
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

    /// Whether the document is the lone empty block whose caret the
    /// autocapitalization nudge parks ahead of the placeholder. That block
    /// holds exactly one caret position, which is what makes a cached scalar
    /// safe to trust there even though the UIKit offset disagrees with it.
    var isLoneEmptyPlaceholderBlock: Bool {
        textStorage.length == 1
            && textStorage.string.unicodeScalars.elementsEqual([Self.emptyBlockPlaceholderScalar])
    }

    private func autocapitalizationFriendlyEmptyBlockPosition(
        for position: UITextPosition
    ) -> UITextPosition? {
        guard isLoneEmptyPlaceholderBlock else { return nil }

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

        // Skip if this change came from our own Rust apply path, transient IME
        // composition, or an inline prediction. iOS inline predictions (iOS 17+)
        // mutate textStorage directly and set markedTextRange without calling
        // setMarkedText, so isComposing remains false — check markedTextRange too.
        guard !isApplyingRustState, !isComposing, markedTextRange == nil else { return }

        // Skip if no editor is bound yet (nothing to reconcile against).
        guard editorId != 0 else { return }

        PositionBridge.invalidateCache(for: self)

        // Compare current text storage content against last authorized snapshot.
        let currentText = textStorage.string
        guard currentText != lastAuthorizedText else { return }
        currentTopLevelChildMetadata = nil

        let allowAfterBlur = canAdoptNativeTextMutationAfterBlur()
        if let mutation = nativeTextMutationFromAuthorizedDiff(currentText: currentText),
           isInterceptingInput
                || shouldAdoptNativeTextStorageMutation(
                    mutation,
                    allowAfterBlur: allowAfterBlur
                )
        {
            scheduleNativeTextMutationCommit(mutation)
            return
        }

        let authorizedPreview = preview(lastAuthorizedText)
        let storagePreview = preview(currentText)

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

            guard !self.isApplyingRustState, !self.isInterceptingInput, !self.isComposing else { return }
            guard self.editorId != 0 else { return }
            guard self.textStorage.string != self.lastAuthorizedText else { return }

            // Reconcile by pulling the current editor state without rebuilding
            // the Rust backend or clearing history. This must run after the
            // current NSTextStorage edit transaction has finished.
            let stateJSON = EditorV2Shadow.getCurrentState(id: self.editorId)
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
    private let defaultImageLoadOwner = RenderImageLoadOwner(policy: .default)
    var imageLoadOwner: RenderImageLoadOwner {
        get { textView.imageLoadOwner ?? defaultImageLoadOwner }
        set {
            guard textView.imageLoadOwner !== newValue else { return }
            textView.imageLoadOwner?.cancelAll()
            textView.imageLoadOwner = newValue
            textView.imageLoadingPolicyDidChange()
        }
    }
    private let remoteSelectionOverlayView = RemoteSelectionOverlayView()
    private let taskListMarkerTapOverlayView = TaskListMarkerTapOverlayView()
    private let imageTapOverlayView = ImageTapOverlayView()
    private let imageResizeOverlayView = ImageResizeOverlayView()
    var onHeightMayChange: ((CGFloat) -> Void)?
    private var lastAutoGrowWidth: CGFloat = 0
    private var cachedAutoGrowMeasuredHeight: CGFloat = 0
    private var remoteSelections: [RemoteSelectionDecoration] = []
    private var initialUpdateJSONForNextEditorBind: String?
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
            guard oldValue != editorId else { return }
            textView.discardTransientNativeInputForEditorRebind()
            if editorId != 0 {
                let initialUpdateJSON = initialUpdateJSONForNextEditorBind
                initialUpdateJSONForNextEditorBind = nil
                textView.bindEditor(id: editorId, initialUpdateJSON: initialUpdateJSON)
            } else {
                initialUpdateJSONForNextEditorBind = nil
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

    func bindEditor(id: UInt64, initialUpdateJSON: String?) {
        guard editorId != id else { return }
        initialUpdateJSONForNextEditorBind = initialUpdateJSON
        editorId = id
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
        textView.imageLoadOwner = defaultImageLoadOwner
        remoteSelectionOverlayView.bind(textView: textView)
        taskListMarkerTapOverlayView.bind(editorView: self)
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
        addSubview(taskListMarkerTapOverlayView)
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

    @discardableResult
    func applyTheme(_ theme: EditorTheme?) -> Bool {
        guard textView.applyTheme(theme) else { return false }
        let cornerRadius = theme?.borderRadius ?? 0
        layer.cornerRadius = cornerRadius
        clipsToBounds = cornerRadius > 0
        refreshOverlays()
        return true
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

    func currentCaretRect() -> CGRect? {
        guard let selectedTextRange = textView.selectedTextRange else { return nil }
        let rect = textView.caretRect(for: selectedTextRange.end)
        guard rect.height > 0 else { return nil }
        return textView.convert(rect, to: self)
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

    func taskListMarkerTapOverlayInterceptsPointForTesting(_ point: CGPoint) -> Bool {
        taskListMarkerTapOverlayView.interceptsPointForTesting(
            convert(point, to: taskListMarkerTapOverlayView)
        )
    }

    @discardableResult
    func tapTaskListMarkerOverlayForTesting(at point: CGPoint) -> Bool {
        taskListMarkerTapOverlayView.handleTapForTesting(
            convert(point, to: taskListMarkerTapOverlayView)
        )
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
        _ = EditorV2Shadow.setHtml(id: editorId, html: html)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
    }

    /// Set initial content from ProseMirror JSON.
    ///
    /// - Parameter json: The JSON string to load.
    func setContent(json: String) {
        guard editorId != 0 else { return }
        _ = EditorV2Shadow.setJson(id: editorId, json: json)
        textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
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

    func remeasureAutoGrowHeight() -> CGFloat {
        guard heightBehavior == .autoGrow else { return 0 }
        cachedAutoGrowMeasuredHeight = 0
        let measuredHeight = measuredEditorHeight()
        invalidateIntrinsicContentSize()
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
        if taskListMarkerTapOverlayView.frame != managedFrame {
            taskListMarkerTapOverlayView.frame = managedFrame
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
        textView.imageLoadOwner?.cancelAll()
        if editorId != 0 {
            textView.unbindEditor()
        }
    }
}
