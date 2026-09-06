import ExpoModulesCore
import UIKit

extension NativeEditorExpoView {
    func setMentionQueryStateForTesting(_ state: MentionQueryState?) {
        mentionQueryState = state
    }

    func currentMentionQueryStateForTesting(trigger: String) -> MentionQueryState? {
        currentMentionQueryState(trigger: trigger)
    }

    func setMentionSuggestionsForTesting(_ suggestions: [NativeMentionSuggestion]) {
        accessoryToolbar.setMentionSuggestions(
            suggestions,
            trigger: mentionQueryState?.trigger ?? "@"
        )
    }

    func isShowingMentionSuggestionsForTesting() -> Bool {
        accessoryToolbar.isShowingMentionSuggestions
    }

    func lastAddonEventJSONForTesting() -> String? {
        lastAddonEventJSONForTestingValue
    }

    func triggerMentionSuggestionTapForTesting(at index: Int) {
        accessoryToolbar.triggerMentionSuggestionTapForTesting(at: index)
    }

    func inputAccessoryViewForTesting() -> UIView? {
        richTextView.textView.inputAccessoryView
    }

    func isUsingAccessoryToolbarForTesting() -> Bool {
        richTextView.textView.inputAccessoryView === accessoryToolbar
    }

    func isUsingAccessoryPlaceholderForTesting() -> Bool {
        richTextView.textView.inputAccessoryView === accessoryPlaceholder
    }

    func markRecentToolbarTouchForTesting() {
        markRecentToolbarTouch()
    }

    func shouldPreserveFocusAfterToolbarTouchForTesting() -> Bool {
        shouldPreserveFocusAfterToolbarTouch()
    }

    func consumeToolbarFocusPreservationForTesting() -> Bool {
        consumeToolbarFocusPreservationForBlur()
    }

    func prepareOutsideTapForFocusHandlingForTesting(
        locationInWindow: CGPoint,
        touchedView: UIView? = nil
    ) -> Bool {
        prepareOutsideTapForFocusHandling(
            locationInWindow: locationInWindow,
            touchedView: touchedView
        )
    }

}
