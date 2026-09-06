import UIKit
import os

extension EditorTextView {
    @objc
    func handleImageAttachmentDidLoad(_ notification: Notification) {
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

    @objc
    func handleImageSelectionTap(_ gesture: UITapGestureRecognizer) {
        guard gesture.state == .ended, gesture.numberOfTouches == 1 else { return }
        let location = gesture.location(in: self)
        guard let range = imageAttachmentRange(at: location) else { return }
        scheduleDeferredImageSelection(for: range)
        _ = selectImageAttachment(range: range)
    }

    @discardableResult
    func selectImageAttachmentIfNeeded(at location: CGPoint) -> Bool {
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

    func installImageSelectionTapDependencies() {
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

    func imageAttachmentRange(at location: CGPoint) -> NSRange? {
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
            rect.origin.x += textContainerInset.left
            rect.origin.y += textContainerInset.top

            if rect.insetBy(dx: -8, dy: -8).contains(location) {
                resolvedRange = range
                stop.pointee = true
            }
        }

        return resolvedRange
    }

    func atomAttachmentRange(at location: CGPoint) -> NSRange? {
        guard textStorage.length > 0 else { return nil }
        var resolvedRange: NSRange?
        textStorage.enumerateAttribute(
            .attachment,
            in: NSRange(location: 0, length: textStorage.length),
            options: [.longestEffectiveRangeNotRequired]
        ) { value, range, stop in
            guard value is AtomBlockAttachment, range.length > 0 else { return }
            let glyphRange = layoutManager.glyphRange(
                forCharacterRange: range,
                actualCharacterRange: nil
            )
            guard glyphRange.length > 0 else { return }
            var rect = layoutManager.boundingRect(forGlyphRange: glyphRange, in: textContainer)
            rect.origin.x += textContainerInset.left - contentOffset.x
            rect.origin.y += textContainerInset.top - contentOffset.y
            if rect.contains(location) {
                resolvedRange = range
                stop.pointee = true
            }
        }
        return resolvedRange
    }

    func taskListMarkerParagraphStart(at location: CGPoint) -> Int? {
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

    func selectedImageSelectionState() -> (docPos: UInt32, utf16Offset: Int)? {
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

}
