import XCTest
import UIKit

struct ApplyUpdateTraceStats {
    let name: String
    let traces: [EditorTextView.ApplyUpdateTrace]

    private func average(_ selector: (EditorTextView.ApplyUpdateTrace) -> UInt64) -> Double {
        guard !traces.isEmpty else { return 0 }
        return Double(traces.map(selector).reduce(0, +)) / Double(traces.count) / 1_000_000.0
    }

    func summaryString(tag: String = "NativePerformanceTests") -> String {
        let averageReplaceUtf16 = traces.isEmpty
            ? 0
            : traces.map(\.applyRenderReplaceUtf16Length).reduce(0, +) / traces.count
        let averageReplacementUtf16 = traces.isEmpty
            ? 0
            : traces.map(\.applyRenderReplacementUtf16Length).reduce(0, +) / traces.count
        return "[\(tag)] \(name) avgMs={parse=\(String(format: "%.3f", average { $0.parseNanos })), resolveBlocks=\(String(format: "%.3f", average { $0.resolveRenderBlocksNanos })), patchEligibility=\(String(format: "%.3f", average { $0.patchEligibilityNanos })), patchTrim=\(String(format: "%.3f", average { $0.patchTrimNanos })), patchMetadata=\(String(format: "%.3f", average { $0.patchMetadataNanos })), buildRender=\(String(format: "%.3f", average { $0.buildRenderNanos })), applyRender=\(String(format: "%.3f", average { $0.applyRenderNanos })), applyRenderTextMutation=\(String(format: "%.3f", average { $0.applyRenderTextMutationNanos })), applyRenderBeginEditing=\(String(format: "%.3f", average { $0.applyRenderBeginEditingNanos })), applyRenderEndEditing=\(String(format: "%.3f", average { $0.applyRenderEndEditingNanos })), applyRenderStringMutation=\(String(format: "%.3f", average { $0.applyRenderStringMutationNanos })), applyRenderAttributeMutation=\(String(format: "%.3f", average { $0.applyRenderAttributeMutationNanos })), applyRenderAuthorizedText=\(String(format: "%.3f", average { $0.applyRenderAuthorizedTextNanos })), applyRenderCacheInvalidation=\(String(format: "%.3f", average { $0.applyRenderCacheInvalidationNanos })), selection=\(String(format: "%.3f", average { $0.selectionNanos })), selectionResolve=\(String(format: "%.3f", average { $0.selectionResolveNanos })), selectionAssignment=\(String(format: "%.3f", average { $0.selectionAssignmentNanos })), selectionChrome=\(String(format: "%.3f", average { $0.selectionChromeNanos })), postApply=\(String(format: "%.3f", average { $0.postApplyNanos })), postApplyTypingAttributes=\(String(format: "%.3f", average { $0.postApplyTypingAttributesNanos })), postApplyHeightNotify=\(String(format: "%.3f", average { $0.postApplyHeightNotifyNanos })), postApplyHeightNotifyMeasure=\(String(format: "%.3f", average { $0.postApplyHeightNotifyMeasureNanos })), postApplyHeightNotifyCallback=\(String(format: "%.3f", average { $0.postApplyHeightNotifyCallbackNanos })), postApplyHeightNotifyEnsureLayout=\(String(format: "%.3f", average { $0.postApplyHeightNotifyEnsureLayoutNanos })), postApplyHeightNotifyUsedRect=\(String(format: "%.3f", average { $0.postApplyHeightNotifyUsedRectNanos })), postApplyHeightNotifyContentSize=\(String(format: "%.3f", average { $0.postApplyHeightNotifyContentSizeNanos })), postApplyHeightNotifySizeThatFits=\(String(format: "%.3f", average { $0.postApplyHeightNotifySizeThatFitsNanos })), postApplySelectionOrContent=\(String(format: "%.3f", average { $0.postApplySelectionOrContentCallbackNanos })), total=\(String(format: "%.3f", average { $0.totalNanos }))} patchUsage=\(traces.filter { $0.usedPatch }.count)/\(traces.count) smallPatchMutationUsage=\(traces.filter { $0.usedSmallPatchTextMutation }.count)/\(traces.count) avgPatchUtf16={replace=\(averageReplaceUtf16), replacement=\(averageReplacementUtf16)}"
    }
}

struct HostedLayoutTraceStats {
    let name: String
    let traces: [RichTextEditorView.HostedLayoutTrace]

    private func average(
        _ selector: (RichTextEditorView.HostedLayoutTrace) -> UInt64
    ) -> Double {
        guard !traces.isEmpty else { return 0 }
        return Double(traces.map(selector).reduce(0, +)) / Double(traces.count) / 1_000_000.0
    }

    private func averageCount(
        _ selector: (RichTextEditorView.HostedLayoutTrace) -> Int
    ) -> Double {
        guard !traces.isEmpty else { return 0 }
        return Double(traces.map(selector).reduce(0, +)) / Double(traces.count)
    }

    func summaryString(tag: String = "NativePerformanceTests") -> String {
        "[\(tag)] \(name) avgMs={intrinsicContentSize=\(String(format: "%.3f", average { $0.intrinsicContentSizeNanos })), measuredEditorHeight=\(String(format: "%.3f", average { $0.measuredEditorHeightNanos })), layoutSubviews=\(String(format: "%.3f", average { $0.layoutSubviewsNanos })), refreshOverlays=\(String(format: "%.3f", average { $0.refreshOverlaysNanos })), onHeightMayChange=\(String(format: "%.3f", average { $0.onHeightMayChangeNanos }))} avgCount={intrinsicContentSize=\(String(format: "%.2f", averageCount { $0.intrinsicContentSizeCount })), measuredEditorHeight=\(String(format: "%.2f", averageCount { $0.measuredEditorHeightCount })), layoutSubviews=\(String(format: "%.2f", averageCount { $0.layoutSubviewsCount })), refreshOverlays=\(String(format: "%.2f", averageCount { $0.refreshOverlaysCount })), overlayScheduleRequest=\(String(format: "%.2f", averageCount { $0.overlayScheduleRequestCount })), overlayScheduleExecute=\(String(format: "%.2f", averageCount { $0.overlayScheduleExecuteCount })), overlayScheduleSkip=\(String(format: "%.2f", averageCount { $0.overlayScheduleSkipCount })), onHeightMayChange=\(String(format: "%.2f", averageCount { $0.onHeightMayChangeCount }))}"
    }
}

@MainActor
final class NativePerformanceTests: XCTestCase {
    let baseFont = UIFont.systemFont(ofSize: 16)
    let textColor = UIColor.black
    static let preparedProseWindowTraversalTimeout: TimeInterval = 10

    @objc func displayLinkProbe(_ displayLink: CADisplayLink) {}

    func traversePreparedProseWindows(
        _ harness: PreparedProseCollectionHarness,
        windows: [PreparedProseBenchmarkCorpus.WarmWindow],
        phase: PreparedProseInstrumentation.TraversalPhase,
        imagesEnabled: Bool
    ) throws -> [PreparedProseCollectionHarness.WindowTraversalResult] {
        let completion = expectation(description: "prepared prose \(phase.rawValue) traversal")
        var result: Result<[PreparedProseCollectionHarness.WindowTraversalResult], Error>?
        harness.traverseWindows(windows, phase: phase, imagesEnabled: imagesEnabled) {
            result = $0
            completion.fulfill()
        }
        wait(for: [completion], timeout: preparedProseTraversalTimeout(forWindowCount: windows.count))
        return try XCTUnwrap(result).get()
    }

    func preparedProseTraversalTimeout(forWindowCount windowCount: Int) -> TimeInterval {
        TimeInterval(max(1, windowCount)) * Self.preparedProseWindowTraversalTimeout
    }

    func attachedPreparedProseHeight(
        _ harness: PreparedProseCollectionHarness,
        entryID: String
    ) throws -> CGFloat {
        let window = PreparedProseBenchmarkCorpus.WarmWindow(
            id: "fixture-\(entryID)",
            primeIds: [entryID],
            warmIds: [entryID]
        )
        return try XCTUnwrap(
            traversePreparedProseWindows(harness, windows: [window], phase: .cold, imagesEnabled: true).first
        ).renderedHeight
    }

    func measureOptions() -> XCTMeasureOptions {
        let options = XCTMeasureOptions()
        options.iterationCount = 5
        return options
    }
}
