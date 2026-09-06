import UIKit
import os

extension EditorTextView {
    func withImageLoadOwner<T>(_ body: () -> T) -> T {
        guard let imageLoadOwner else { return body() }
        return imageLoadOwner.withCurrent(body)
    }

    func imageLoadingPolicyDidChange() {
        renderAppearanceRevision &+= 1
    }

    enum PositionCacheUpdate {
        case scan
        case invalidate
        case plainText
        case attributed
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

    @discardableResult
    func applyAtomRenderConfiguration(_ configuration: AtomRenderConfiguration?) -> Bool {
        if editorId != 0 {
            guard prepareForExternalEditorUpdate() else { return false }
            atomRenderConfiguration = configuration
            let previousOffset = contentOffset
            applyUpdateJSON(EditorV2Shadow.getCurrentState(id: editorId), notifyDelegate: false)
            if heightBehavior == .fixed {
                preserveScrollOffset(previousOffset)
            }
        } else {
            atomRenderConfiguration = configuration
        }
        if heightBehavior == .autoGrow {
            notifyHeightChangeIfNeeded(force: true)
        }
        return true
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

    func applyAttributedRender(
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
        onApplyingRustTextForTesting?()
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
        scheduleCodeHighlighting()
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
        let updateDocumentVersion = canonicalDocumentVersion(update["documentVersion"])
        let renderBlocks = parseRenderBlocks(update["renderBlocks"])
        let explicitRenderPatch = parseRenderPatch(update["renderPatch"])
        let resolvedRenderBlocks = renderBlocks
            ?? explicitRenderPatch.flatMap { patch in
                guard patchMatchesCurrentRenderBlocks(
                    patch,
                    updateDocumentVersion: updateDocumentVersion
                ) else { return nil }
                return currentRenderBlocks.flatMap { mergeRenderBlocks(applying: patch, to: $0) }
            }
        let resolveRenderBlocksNanos =
            DispatchTime.now().uptimeNanoseconds - resolveRenderBlocksStartedAt
        if renderBlocks == nil,
           renderElements == nil,
           explicitRenderPatch != nil,
           resolvedRenderBlocks == nil
        {
            return recoverRenderPatchBaseMismatch(notifyDelegate: notifyDelegate)
        }

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

        let patchTrace: PatchApplyTrace? = if !shouldSkipRender
            && textStorage.string == lastAuthorizedText
            && lastAppliedRenderAppearanceRevision == renderAppearanceRevision
        {
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
                retainCurrentRenderBlocks(
                    resolvedRenderBlocks,
                    documentVersion: updateDocumentVersion
                )
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
                        theme: theme,
                        atomConfiguration: atomRenderConfiguration
                    )
                }
                retainCurrentRenderBlocks(
                    resolvedRenderBlocks,
                    documentVersion: updateDocumentVersion
                )
            } else if let renderElements {
                attrStr = withImageLoadOwner {
                    RenderBridge.renderElements(
                        fromArray: renderElements,
                        baseFont: baseFont,
                        textColor: baseTextColor,
                        theme: theme,
                        atomConfiguration: atomRenderConfiguration
                    )
                }
                invalidateCurrentRenderBlocks()
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
            retainCurrentRenderBlocks(
                resolvedRenderBlocks,
                documentVersion: updateDocumentVersion
            )
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
                theme: theme,
                atomConfiguration: atomRenderConfiguration
            )
        }
        _ = applyAttributedRender(attrStr, usedPatch: false)
        invalidateCurrentRenderBlocks()
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

}
