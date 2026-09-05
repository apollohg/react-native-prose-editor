import UIKit
import os

extension EditorTextView {
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
    func normalizeSelectionForEmptyBlockAutocapitalizationIfNeeded() -> Bool {
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

    func refreshPlaceholderVisibility() {
        let hasProvisionalText = externalTextComposition?.latestText.isEmpty == false
        placeholderLabel.isHidden = placeholder.isEmpty
            || hasProvisionalText
            || !isRenderedContentEmpty()
    }

    func preserveScrollOffset(_ previousOffset: CGPoint) {
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

    func defaultTypingAttributes() -> [NSAttributedString.Key: Any] {
        [
            .font: resolvedDefaultFont(),
            .foregroundColor: resolvedDefaultTextColor(),
        ]
    }

    func resolvedDefaultFont() -> UIFont {
        theme?.effectiveTextStyle(for: "paragraph").resolvedFont(fallback: baseFont)
            ?? baseFont
    }

    func resolvedDefaultTextColor() -> UIColor {
        theme?.effectiveTextStyle(for: "paragraph").color ?? baseTextColor
    }

    func notifyHeightChangeIfNeeded(force: Bool = false) {
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

    func measuredAutoGrowHeight(forWidth width: CGFloat) -> CGFloat {
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
            let layoutHeight = ceil(
                usedRect.height
                    + textContainerInset.top
                    + textContainerInset.bottom
            )

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

    func invalidateAutoGrowHeightMeasurement() {
        autoGrowHeightCheckIsDirty = true
        lastAutoGrowMeasuredWidth = 0
    }

}
