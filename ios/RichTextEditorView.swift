import UIKit

enum EditorHeightBehavior: String {
    case fixed
    case autoGrow
}

// MARK: - RichTextEditorView (Fabric Host)

final class AtomHostContainerView: UIView {
    let atomKey: String
    private(set) var hostedView: UIView?
    weak var editorView: RichTextEditorView?

    private var boundsObservation: NSKeyValueObservation?
    private var isDetachingHostedView = false
    private var lastReportedHeight: CGFloat?

    init(atomKey: String) {
        self.atomKey = atomKey
        super.init(frame: .zero)
        clipsToBounds = false
        isUserInteractionEnabled = true
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func host(_ view: UIView) {
        if hostedView !== view {
            _ = detachHostedView()
        }
        view.removeFromSuperview()
        hostedView = view
        addSubview(view)
        boundsObservation = view.observe(\.bounds, options: [.new]) { [weak self] _, _ in
            self?.reportHeightIfNeeded()
        }
        setNeedsLayout()
        reportHeightIfNeeded()
    }

    @discardableResult
    func detachHostedView() -> UIView? {
        boundsObservation = nil
        guard let hostedView else { return nil }
        self.hostedView = nil
        lastReportedHeight = nil
        isDetachingHostedView = true
        hostedView.removeFromSuperview()
        isDetachingHostedView = false
        return hostedView
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        reportHeightIfNeeded()
        guard let hostedView, lastReportedHeight != nil else { return }
        let height = hostedView.bounds.height
        let frame = CGRect(x: 0, y: 0, width: bounds.width, height: height)
        if hostedView.frame != frame {
            hostedView.frame = frame
        }
        reportHeightIfNeeded()
    }

    override func willRemoveSubview(_ subview: UIView) {
        super.willRemoveSubview(subview)
        guard !isDetachingHostedView, subview === hostedView else { return }
        boundsObservation = nil
        hostedView = nil
        lastReportedHeight = nil
        editorView?.atomHostContainerDidLoseHostedView(self)
    }

    private func reportHeightIfNeeded() {
        guard let height = hostedView?.bounds.height,
              height.isFinite,
              height >= 0,
              height > 0 || (hostedView?.bounds.width ?? 0) > 0,
              lastReportedHeight.map({ abs($0 - height) > 0.5 }) ?? true
        else { return }
        lastReportedHeight = height
        editorView?.atomHostContainer(self, didMeasureHeight: height)
    }
}

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
    var onAtomContentWidthChange: ((CGFloat) -> Void)?
    private var lastAutoGrowWidth: CGFloat = 0
    private var cachedAutoGrowMeasuredHeight: CGFloat = 0
    private var atomRenderConfiguration: AtomRenderConfiguration?
    private var atomHostContainers: [String: AtomHostContainerView] = [:]
    private var measuredAtomHeights: [String: CGFloat] = [:]
    private var fallbackAtomHeights: [String: CGFloat] = [:]
    private var lastAtomContentWidth: CGFloat = 0
    private var lastAtomPositions: [[String: AnyHashable]] = []
    private var lastAtomViewport = CGRect.zero
    private(set) var atomLayoutInvalidationCountForTesting = 0
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
            self?.emitAtomContentWidthIfAvailable()
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
        layoutAtomHostContainers()
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

    @discardableResult
    func applyAtomRenderConfiguration(_ configuration: AtomRenderConfiguration?) -> Bool {
        let previousConfiguration = atomRenderConfiguration
        atomRenderConfiguration = configuration
        guard textView.applyAtomRenderConfiguration(resolvedAtomRenderConfiguration()) else {
            atomRenderConfiguration = previousConfiguration
            return false
        }
        layoutAtomHostContainers()
        refreshOverlays()
        return true
    }

    func mountAtomChild(_ child: UIView, atomKey: String) {
        if let existing = atomHostContainers[atomKey] {
            if existing.hostedView === child {
                if child.superview !== existing {
                    existing.host(child)
                }
                layoutAtomHostContainers()
                return
            }
            _ = existing.detachHostedView()
            existing.removeFromSuperview()
        }

        let container = AtomHostContainerView(atomKey: atomKey)
        container.editorView = self
        atomHostContainers[atomKey] = container
        textView.addSubview(container)
        container.host(child)
        layoutAtomHostContainers()
    }

    @discardableResult
    func unmountAtomChild(_ child: UIView) -> Bool {
        guard let entry = atomHostContainers.first(where: { $0.value.hostedView === child }) else {
            return false
        }
        removeAtomHostContainer(for: entry.key, removeChild: true)
        return true
    }

    func atomHostContainer(for atomKey: String) -> AtomHostContainerView? {
        atomHostContainers[atomKey]
    }

    func measuredAtomHeight(for atomKey: String) -> CGFloat? {
        measuredAtomHeights[atomKey]
    }

    func emitAtomContentWidthIfAvailable(force: Bool = false) {
        let width = atomContentWidth()
        let positions = atomLayoutPositions()
        let viewport = textView.bounds
        guard width > 0,
              force || abs(width - lastAtomContentWidth) > 0.5
                || positions != lastAtomPositions || viewport != lastAtomViewport
        else { return }
        lastAtomContentWidth = width
        lastAtomPositions = positions
        lastAtomViewport = viewport
        onAtomContentWidthChange?(width)
    }

    func atomLayoutPositions() -> [[String: AnyHashable]] {
        let padding = textView.textContainer.lineFragmentPadding
        return atomAttachmentEntries().sorted { $0.value.1.location < $1.value.1.location }.map { key, entry in
            let (attachment, range) = entry
            textView.layoutManager.ensureLayout(forCharacterRange: range)
            let glyphs = textView.layoutManager.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            let rect = textView.layoutManager.boundingRect(forGlyphRange: glyphs, in: textView.textContainer)
            return [
                "key": key,
                "x": Double(textView.textContainerInset.left + padding),
                "y": Double(textView.textContainerInset.top + rect.minY),
                "height": Double(attachment.reservedHeight),
            ]
        }
    }

    fileprivate func atomHostContainer(
        _ container: AtomHostContainerView,
        didMeasureHeight height: CGFloat
    ) {
        setAtomHeight(key: container.atomKey, height: height)
    }

    fileprivate func atomHostContainerDidLoseHostedView(_ container: AtomHostContainerView) {
        guard atomHostContainers[container.atomKey] === container else { return }
        removeAtomHostContainer(for: container.atomKey, removeChild: false)
    }

    func setAtomHeight(key atomKey: String, height: CGFloat) {
        let previousMeasuredHeight = measuredAtomHeights[atomKey]
        guard height.isFinite,
              height >= 0,
              measuredAtomHeights[atomKey].map({ abs($0 - height) > 0.5 }) ?? true
        else { return }

        measuredAtomHeights[atomKey] = height
        textView.atomRenderConfiguration = resolvedAtomRenderConfiguration()
        guard let entry = atomAttachmentEntry(for: atomKey) else { return }
        let fallbackHeight = atomRenderConfiguration?.estimatedHeights[entry.attachment.nodeType]
            ?? (previousMeasuredHeight == nil && entry.attachment.reservedHeight > 0
                ? entry.attachment.reservedHeight
                : nil)
        if let fallbackHeight {
            fallbackAtomHeights[atomKey] = fallbackAtomHeights[atomKey] ?? fallbackHeight
        }
        updateAtomAttachment(entry, height: height)
    }

    private func removeAtomHostContainer(for atomKey: String, removeChild: Bool) {
        guard let container = atomHostContainers.removeValue(forKey: atomKey) else { return }
        container.editorView = nil
        let child = container.detachHostedView()
        container.removeFromSuperview()
        if removeChild {
            child?.removeFromSuperview()
        }

        measuredAtomHeights.removeValue(forKey: atomKey)
        textView.atomRenderConfiguration = resolvedAtomRenderConfiguration()
        if let entry = atomAttachmentEntry(for: atomKey) {
            let fallbackHeight = fallbackAtomHeights.removeValue(forKey: atomKey)
                ?? atomRenderConfiguration?.estimatedHeights[entry.attachment.nodeType]
            if let fallbackHeight {
                updateAtomAttachment(entry, height: fallbackHeight)
            }
        }
    }

    private func resolvedAtomRenderConfiguration() -> AtomRenderConfiguration? {
        guard let atomRenderConfiguration else { return nil }
        var heights = atomRenderConfiguration.measuredHeights
        heights.merge(measuredAtomHeights) { _, measured in measured }
        return AtomRenderConfiguration(
            registeredNodeTypes: atomRenderConfiguration.registeredNodeTypes,
            estimatedHeights: atomRenderConfiguration.estimatedHeights,
            measuredHeights: heights
        )
    }

    private func updateAtomAttachment(
        _ entry: (attachment: AtomBlockAttachment, range: NSRange),
        height: CGFloat
    ) {
        guard abs(entry.attachment.reservedHeight - height) > 0.5 else { return }
        entry.attachment.reservedHeight = height
        atomLayoutInvalidationCountForTesting += 1
        textView.layoutManager.invalidateLayout(
            forCharacterRange: entry.range,
            actualCharacterRange: nil
        )
        textView.layoutManager.invalidateDisplay(forCharacterRange: entry.range)
        textView.setNeedsLayout()
        setNeedsLayout()
        layoutAtomHostContainers()
    }

    private func atomAttachmentEntry(
        for atomKey: String
    ) -> (attachment: AtomBlockAttachment, range: NSRange)? {
        guard textView.textStorage.length > 0 else { return nil }
        var match: (attachment: AtomBlockAttachment, range: NSRange)?
        textView.textStorage.enumerateAttribute(
            .attachment,
            in: NSRange(location: 0, length: textView.textStorage.length),
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, stop in
            guard let attachment = value as? AtomBlockAttachment,
                  attachment.atomKey == atomKey
            else { return }
            match = (attachment, range)
            stop.pointee = true
        }
        return match
    }

    private func atomAttachmentEntries() -> [String: (AtomBlockAttachment, NSRange)] {
        guard textView.textStorage.length > 0 else { return [:] }
        var entries: [String: (AtomBlockAttachment, NSRange)] = [:]
        textView.textStorage.enumerateAttribute(
            .attachment,
            in: NSRange(location: 0, length: textView.textStorage.length),
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, _ in
            guard let attachment = value as? AtomBlockAttachment else { return }
            entries[attachment.atomKey] = (attachment, range)
        }
        return entries
    }

    private func atomContentWidth() -> CGFloat {
        let padding = textView.textContainer.lineFragmentPadding
        return max(0, textView.textContainer.size.width - (padding * 2))
    }

    private func layoutAtomHostContainers() {
        emitAtomContentWidthIfAvailable()
        guard !atomHostContainers.isEmpty else { return }
        let entries = atomAttachmentEntries()
        let padding = textView.textContainer.lineFragmentPadding
        let width = atomContentWidth()

        for (atomKey, container) in atomHostContainers {
            guard let (attachment, characterRange) = entries[atomKey] else {
                container.isHidden = true
                continue
            }
            textView.layoutManager.ensureLayout(forCharacterRange: characterRange)
            let glyphRange = textView.layoutManager.glyphRange(
                forCharacterRange: characterRange,
                actualCharacterRange: nil
            )
            let attachmentRect = textView.layoutManager.boundingRect(
                forGlyphRange: glyphRange,
                in: textView.textContainer
            )
            let frame = CGRect(
                x: textView.textContainerInset.left + padding,
                y: textView.textContainerInset.top + attachmentRect.minY,
                width: width,
                height: attachment.reservedHeight
            )
            if container.frame != frame {
                container.frame = frame
                container.setNeedsLayout()
            }
            textView.bringSubviewToFront(container)
            container.isHidden = false
        }
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
        let updateJSON = EditorV2Shadow.setHtml(id: editorId, html: html)
        if !textView.applyUpdateJSON(updateJSON, notifyDelegate: false) {
            textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        }
    }

    /// Set initial content from ProseMirror JSON.
    ///
    /// - Parameter json: The JSON string to load.
    func setContent(json: String) {
        guard editorId != 0 else { return }
        let updateJSON = EditorV2Shadow.setJson(id: editorId, json: json)
        if !textView.applyUpdateJSON(updateJSON, notifyDelegate: false) {
            textView.applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
        }
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

    func selectedImageGeometry() -> (docPos: UInt32, rect: CGRect)? {
        guard let geometry = textView.selectedImageGeometry() else { return nil }
        return (
            docPos: geometry.docPos,
            rect: textView.convert(geometry.rect, to: imageResizeOverlayView)
        )
    }

    func setImageResizePreviewActive(_ active: Bool) {
        textView.setImageResizePreviewActive(active)
    }

    func imagePreviewForResize(docPos: UInt32) -> UIImage? {
        textView.imagePreviewForDocPos(docPos)
    }

    func imageResizePreviewBackgroundColor() -> UIColor {
        textView.backgroundColor ?? .systemBackground
    }

    func maximumImageWidthForResizeGesture() -> CGFloat {
        textView.maximumRenderableImageWidth()
    }

    func clampedImageSize(_ size: CGSize, maximumWidth: CGFloat? = nil) -> CGSize {
        let aspectRatio = max(size.width / max(size.height, 1), 0.1)
        let maxWidth = max(48, maximumWidth ?? textView.maximumRenderableImageWidth())
        let clampedWidth = min(maxWidth, max(48, size.width))
        let clampedHeight = max(48, clampedWidth / aspectRatio)
        return CGSize(width: clampedWidth, height: clampedHeight)
    }

    func resizeImage(docPos: UInt32, size: CGSize) {
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
        layoutAtomHostContainers()
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
        if !atomHostContainers.isEmpty {
            return true
        }
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
