import UIKit
import os

extension EditorTextView {
    struct TopLevelChildMetadata {
        var startOffset: Int
        var containsAttachment: Bool
        var containsPositionAdjustments: Bool
    }

    private struct TopLevelChildMetadataSlice {
        let startIndex: Int
        let entries: [TopLevelChildMetadata]
    }

    struct ParsedRenderPatch {
        let baseDocumentVersion: UInt64?
        let startIndex: Int
        let deleteCount: Int
        let renderBlocks: [[[String: Any]]]
    }

    enum DerivedRenderPatch {
        case unchanged
        case patch(ParsedRenderPatch)
    }

    func parseRenderBlocks(_ value: Any?) -> [[[String: Any]]]? {
        value as? [[[String: Any]]]
    }

    func parseRenderPatch(_ value: Any?) -> ParsedRenderPatch? {
        guard let raw = value as? [String: Any],
              let startIndex = RenderBridge.jsonInt(raw["startIndex"]),
              let deleteCount = RenderBridge.jsonInt(raw["deleteCount"]),
              let renderBlocks = parseRenderBlocks(raw["renderBlocks"])
        else {
            return nil
        }
        let baseDocumentVersion: UInt64?
        if raw.keys.contains("baseDocumentVersion") {
            guard let parsed = canonicalDocumentVersion(raw["baseDocumentVersion"])
            else { return nil }
            baseDocumentVersion = parsed
        } else {
            baseDocumentVersion = nil
        }

        return ParsedRenderPatch(
            baseDocumentVersion: baseDocumentVersion,
            startIndex: startIndex,
            deleteCount: deleteCount,
            renderBlocks: renderBlocks
        )
    }

    func canonicalDocumentVersion(_ value: Any?) -> UInt64? {
        guard let text = value as? String,
              let version = UInt64(text),
              String(version) == text
        else { return nil }
        return version
    }

    func patchMatchesCurrentRenderBlocks(
        _ patch: ParsedRenderPatch,
        updateDocumentVersion: UInt64?
    ) -> Bool {
        if let baseDocumentVersion = patch.baseDocumentVersion {
            return baseDocumentVersion == currentRenderBlocksDocumentVersion
        }
        return updateDocumentVersion == nil && currentRenderBlocksDocumentVersion == nil
    }

    func retainCurrentRenderBlocks(
        _ blocks: [[[String: Any]]]?,
        documentVersion: UInt64?
    ) {
        currentRenderBlocks = blocks
        currentRenderBlocksDocumentVersion = blocks == nil ? nil : documentVersion
    }

    func invalidateCurrentRenderBlocks() {
        currentRenderBlocks = nil
        currentRenderBlocksDocumentVersion = nil
    }

    func recoverRenderPatchBaseMismatch(notifyDelegate: Bool) -> Bool {
        invalidateCurrentRenderBlocks()
        guard !recoveringRenderPatchBaseMismatch,
              editorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId),
              let recovery = adapter.recoverNativeRender()
        else { return false }
        recoveringRenderPatchBaseMismatch = true
        defer { recoveringRenderPatchBaseMismatch = false }
        return applyUpdateJSON(recovery, notifyDelegate: notifyDelegate)
    }

    func mergeRenderBlocks(
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

    func refreshRetainedPositionalMetadata(
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

    func deriveRenderPatch(
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
                baseDocumentVersion: currentRenderBlocksDocumentVersion,
                startIndex: startIndex,
                deleteCount: deleteCount,
                renderBlocks: replacementBlocks
            )
        )
    }

    func topLevelChildIndex(from value: Any?) -> Int? {
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

    func refreshTopLevelChildMetadata(
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

    func applyRenderPatchIfPossible(_ patch: ParsedRenderPatch) -> PatchApplyTrace {
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
                theme: theme,
                atomConfiguration: atomRenderConfiguration
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

}
