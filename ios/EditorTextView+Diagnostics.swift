import UIKit
import os

extension EditorTextView {
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

    struct PatchApplyTrace {
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

    struct ApplyRenderTrace {
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

    struct SelectionApplyTrace {
        let totalNanos: UInt64
        let resolveNanos: UInt64
        let assignmentNanos: UInt64
        let chromeNanos: UInt64
    }

    struct PostApplyTrace {
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

    func measuredAutoGrowHeightForTesting(width: CGFloat) -> CGFloat {
        measuredAutoGrowHeight(forWidth: width)
    }

    func preview(_ text: String, limit: Int = 32) -> String {
        let normalized = text.replacingOccurrences(of: "\n", with: "\\n")
        if normalized.count <= limit {
            return normalized
        }
        return "\(normalized.prefix(limit))…"
    }

    func textSnapshotSummary() -> String {
        let text = textStorage.string
        return "len=\(text.count) preview=\"\(preview(text))\""
    }

    func selectionSummary() -> String {
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

    func selectionSummary(from selection: [String: Any]) -> String {
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

    func expireNativeTextMutationAfterBlurDeadlineForTesting() {
        nativeTextMutationAfterBlurDeadline = ProcessInfo.processInfo.systemUptime - 0.001
    }

}
