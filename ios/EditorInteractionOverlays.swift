import UIKit

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

final class RemoteSelectionOverlayView: UIView {
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

final class ImageTapOverlayView: UIView {
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

final class TaskListMarkerTapOverlayView: UIView {
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

final class ImageResizeOverlayView: UIView {
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
