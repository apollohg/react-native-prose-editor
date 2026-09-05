import UIKit
import os

extension EditorTextView {
    func shouldUseSmallPatchTextMutation(
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

    func applyAttributes(from attributedString: NSAttributedString, to destinationRange: NSRange) {
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

    func attributedStringContainsAttachment(_ attributedString: NSAttributedString) -> Bool {
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

    func attributedStringContainsPositionAdjustments(_ attributedString: NSAttributedString) -> Bool {
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

    func attributedStringContainsListMarkerContext(_ attributedString: NSAttributedString) -> Bool {
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

    func textStorageRangeContainsPositionAdjustments(_ range: NSRange) -> Bool {
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

    func textStorageRangeContainsListMarkerContext(_ range: NSRange) -> Bool {
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

    func textStorageRangeContainsAttachment(_ range: NSRange) -> Bool {
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

    func topLevelChildrenContainAttachment(
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

    func topLevelChildrenContainPositionAdjustments(
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

    func trimmedAttributedPatch(
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

}
