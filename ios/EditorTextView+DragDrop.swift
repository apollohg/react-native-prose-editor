import UIKit
import os

extension EditorTextView {
    enum LocalTextDragState {
        case idle
        case dragging(
            session: ObjectIdentifier,
            editorId: UInt64,
            documentRevision: UInt64,
            supported: Bool,
            range: (from: UInt32, to: UInt32)
        )
        case awaitingUIKitCleanup(
            session: ObjectIdentifier,
            editorId: UInt64,
            cleanupRanges: [NSRange]
        )
    }

    func textDraggableView(
        _ textDraggableView: UIView & UITextDraggable,
        itemsForDrag dragRequest: UITextDragRequest
    ) -> [UIDragItem] {
        let requestedRange = PositionBridge.textRangeToScalarRange(
            dragRequest.dragRange,
            in: self
        )
        let range = scalarRange(for: dragRequest)
        if editorId != 0,
           range.from < range.to,
           let documentRevision = EditorV2Shadow.documentRevision(id: editorId)
        {
            let narrowedToVoidAttachment = !dragRequest.isSelected
                && (range.from != requestedRange.from || range.to != requestedRange.to)
            localTextDragState = .dragging(
                session: ObjectIdentifier(dragRequest.dragSession as AnyObject),
                editorId: editorId,
                documentRevision: documentRevision,
                supported: narrowedToVoidAttachment
                    || containsNoBlockBoundary(dragRequest.dragRange),
                range: range
            )
        }
        return dragRequest.suggestedItems
    }

    private func containsNoBlockBoundary(_ range: UITextRange) -> Bool {
        let start = offset(from: beginningOfDocument, to: range.start)
        let end = offset(from: beginningOfDocument, to: range.end)
        guard start >= 0, end > start, end <= textStorage.length else { return false }
        var containsBoundary = false
        textStorage.enumerateAttribute(
            RenderBridgeAttributes.blockBoundary,
            in: NSRange(location: start, length: end - start)
        ) { value, _, stop in
            guard value != nil else { return }
            containsBoundary = true
            stop.pointee = true
        }
        return !containsBoundary
    }

    func scalarRange(for dragRequest: UITextDragRequest) -> (from: UInt32, to: UInt32) {
        let range = PositionBridge.textRangeToScalarRange(dragRequest.dragRange, in: self)
        guard !dragRequest.isSelected else { return range }

        let start = offset(from: beginningOfDocument, to: dragRequest.dragRange.start)
        let end = offset(from: beginningOfDocument, to: dragRequest.dragRange.end)
        guard start >= 0, end > start, start < textStorage.length else { return range }
        let length = min(end, textStorage.length) - start
        var atomRange: (from: UInt32, to: UInt32)?
        textStorage.enumerateAttribute(
            .attachment,
            in: NSRange(location: start, length: length)
        ) { value, characterRange, stop in
            guard value is NSTextAttachment,
                  textStorage.attribute(
                    RenderBridgeAttributes.voidNodeType,
                    at: characterRange.location,
                    effectiveRange: nil
                  ) is String
            else {
                return
            }
            let from = PositionBridge.utf16OffsetToScalar(characterRange.location, in: self)
            let to = PositionBridge.utf16OffsetToScalar(NSMaxRange(characterRange), in: self)
            atomRange = (from, to)
            stop.pointee = true
        }
        return atomRange ?? range
    }

    func textDroppableView(
        _ textDroppableView: UIView & UITextDroppable,
        proposalForDrop drop: UITextDropRequest
    ) -> UITextDropProposal {
        guard let drag = matchingLocalTextDrag(for: drop) else {
            return drop.suggestedProposal
        }
        let destination = PositionBridge.textViewToScalar(drop.dropPosition, in: self)
        guard drag.supported,
              drag.documentRevision == EditorV2Shadow.documentRevision(id: editorId),
              canMove(drag.range, to: destination)
        else {
            return UITextDropProposal(operation: .forbidden)
        }

        let proposal = UITextDropProposal(operation: .move)
        proposal.dropAction = .insert
        proposal.dropPerformer = .delegate
        proposal.dropProgressMode = .custom
        proposal.useFastSameViewOperations = false
        return proposal
    }

    func textDroppableView(
        _ textDroppableView: UIView & UITextDroppable,
        willPerformDrop drop: UITextDropRequest
    ) {
        guard let drag = matchingLocalTextDrag(for: drop) else { return }
        let destination = PositionBridge.textViewToScalar(drop.dropPosition, in: self)
        guard drag.supported,
              drag.documentRevision == EditorV2Shadow.documentRevision(id: editorId),
              canMove(drag.range, to: destination)
        else {
            localTextDragState = .idle
            return
        }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
        guard drag.documentRevision == EditorV2Shadow.documentRevision(id: editorId) else {
            localTextDragState = .idle
            return
        }

        let sourceStartUtf16 = PositionBridge.scalarToUtf16Offset(drag.range.from, in: self)
        let sourceEndUtf16 = PositionBridge.scalarToUtf16Offset(drag.range.to, in: self)
        let sourceUtf16Range = NSRange(
            location: sourceStartUtf16,
            length: max(0, sourceEndUtf16 - sourceStartUtf16)
        )

        var applied = false
        performInterceptedInput {
            let updateJSON = EditorV2Shadow.moveSelectionAtScalar(
                id: editorId,
                scalarAnchor: drag.range.from,
                scalarHead: drag.range.to,
                destination: destination
            )
            applied = applyUpdateJSON(updateJSON)
        }
        if applied {
            localTextDragState = .awaitingUIKitCleanup(
                session: drag.session,
                editorId: drag.editorId,
                cleanupRanges: localTextDragCleanupRanges(
                    sourceUtf16Range: sourceUtf16Range,
                    sourceScalarRange: drag.range,
                    destination: destination
                )
            )
        } else {
            localTextDragState = .idle
        }
    }

    func textDroppableView(
        _ textDroppableView: UIView & UITextDroppable,
        previewForDroppingAllItemsWithDefault defaultPreview: UITargetedDragPreview
    ) -> UITargetedDragPreview? {
        defaultPreview
    }

    func textDraggableView(
        _ textDraggableView: UIView & UITextDraggable,
        dragSessionDidEnd session: UIDragSession,
        with operation: UIDropOperation
    ) {
        finishLocalTextDrag(for: session)
    }

    private func matchingLocalTextDrag(
        for drop: UITextDropRequest
    ) -> (
        session: ObjectIdentifier,
        editorId: UInt64,
        documentRevision: UInt64,
        supported: Bool,
        range: (from: UInt32, to: UInt32)
    )? {
        guard drop.isSameView,
              let localSession = drop.dropSession.localDragSession,
              case let .dragging(
                  session,
                  sourceEditorId,
                  documentRevision,
                  supported,
                  range
              ) = localTextDragState,
              sourceEditorId == editorId,
              session == ObjectIdentifier(localSession as AnyObject)
        else {
            return nil
        }
        return (session, sourceEditorId, documentRevision, supported, range)
    }

    private func canMove(
        _ range: (from: UInt32, to: UInt32),
        to destination: UInt32
    ) -> Bool {
        destination < range.from || destination > range.to
    }

    private func finishLocalTextDrag(for session: UIDragSession) {
        let sessionID = ObjectIdentifier(session as AnyObject)
        guard case let .awaitingUIKitCleanup(completedSession, sourceEditorId, _) = localTextDragState,
              completedSession == sessionID,
              sourceEditorId == editorId
        else {
            if case let .dragging(activeSession, _, _, _, _) = localTextDragState,
               activeSession == sessionID {
                localTextDragState = .idle
            }
            return
        }
        scheduleLocalTextDragCleanup(session: sessionID, sourceEditorId: sourceEditorId)
    }

    private func scheduleLocalTextDragCleanup(
        session: ObjectIdentifier,
        sourceEditorId: UInt64
    ) {
        DispatchQueue.main.async { [weak self] in
            guard let self,
                  case let .awaitingUIKitCleanup(activeSession, activeEditorId, _) = self.localTextDragState,
                  activeSession == session,
                  activeEditorId == sourceEditorId,
                  activeEditorId == self.editorId
            else {
                return
            }
            if self.pendingNativeTextMutation != nil || self.nativeTextMutationCommitScheduled {
                _ = self.drainPendingNativeTextMutation(
                    allowAfterBlur: self.canAdoptNativeTextMutationAfterBlur(),
                    allowWhileIntercepting: true
                )
            }
            self.localTextDragState = .idle
            self.restoreAfterLocalTextDragCleanup()
        }
    }

    func restoreAfterLocalTextDragCleanup() {
        if textStorage.string != lastAuthorizedText {
            _ = applyAttributedRender(
                NSAttributedString(attributedString: lastAuthorizedAttributedTextStorage),
                usedPatch: false,
                positionCacheUpdate: .invalidate
            )
        }
        applyUpdateJSON(
            EditorV2Shadow.getCurrentState(id: editorId),
            notifyDelegate: false
        )
    }

    private func localTextDragCleanupRanges(
        sourceUtf16Range: NSRange,
        sourceScalarRange: (from: UInt32, to: UInt32),
        destination: UInt32
    ) -> [NSRange] {
        var ranges = [sourceUtf16Range]
        if destination < sourceScalarRange.from {
            ranges.append(NSRange(
                location: sourceUtf16Range.location + sourceUtf16Range.length,
                length: sourceUtf16Range.length
            ))
        }
        return ranges
    }

}
