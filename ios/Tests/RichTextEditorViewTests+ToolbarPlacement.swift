import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    /// Every placement renders as a custom button, with scroll items in the
    /// scrolling middle and pinned items in the stacks on either side.
    func testNativeAppearanceRendersEveryPlacementAsCustomButtons() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.placementToolbarFixtureJSON)
        host.layoutIfNeeded()

        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("start"),
            ["Start"],
            "start-pinned items belong in the start pinned stack"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("end"),
            ["End"],
            "end-pinned items belong in the end pinned stack"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("scroll"),
            ["Scroll One", "Scroll Two"],
            "scroll-placement items belong in the scrolling middle"
        )
    }

    func testPinnedPlacementsPreserveOuterHorizontalInsets() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.placementToolbarFixtureJSON)
        host.layoutIfNeeded()

        func descendant(withLabel label: String, in view: UIView) -> UIView? {
            if view.accessibilityLabel == label {
                return view
            }
            return view.subviews.lazy.compactMap {
                descendant(withLabel: label, in: $0)
            }.first
        }

        guard let startButton = descendant(withLabel: "Start", in: toolbar),
              let endButton = descendant(withLabel: "End", in: toolbar),
              let startSection = startButton.superview,
              let endSection = endButton.superview
        else {
            XCTFail("expected both pinned toolbar buttons")
            return
        }

        XCTAssertEqual(
            startButton.frame.minX,
            startSection.bounds.minX + 12,
            accuracy: 0.1,
            "start-pinned items should include the standard 12-point leading inset"
        )
        XCTAssertEqual(
            endButton.frame.maxX,
            endSection.bounds.maxX - 12,
            accuracy: 0.1,
            "end-pinned items should include the standard 12-point trailing inset"
        )
    }

    /// `bodyStackView` lays the scrolling middle out between the two pinned
    /// stacks. Arranged subviews cannot overlap by construction, but the middle
    /// can still be starved to zero width if a pinned stack claims the row (see
    /// `updatePinnedStackParticipation`), so assert it actually claims width
    /// rather than merely avoiding overlap by being empty.
    func testContentStackClaimsMiddleSlotWithoutOverlappingPinnedStacks() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.placementToolbarFixtureJSON)
        host.setNeedsLayout()
        host.layoutIfNeeded()

        let contentFrame = toolbar.contentStackViewFrameForTesting
        let startFrame = toolbar.startPinnedStackViewFrameForTesting
        let endFrame = toolbar.endPinnedStackViewFrameForTesting

        XCTAssertGreaterThan(
            contentFrame.width,
            0,
            "the content column must claim the middle slot's width, not collapse to zero"
        )
        XCTAssertFalse(
            contentFrame.intersects(startFrame),
            "content stack frame \(contentFrame) must not overlap the start pinned stack frame \(startFrame)"
        )
        XCTAssertFalse(
            contentFrame.intersects(endFrame),
            "content stack frame \(contentFrame) must not overlap the end pinned stack frame \(endFrame)"
        )
    }

    /// The mention row and the button row share `contentStackView`, so showing
    /// suggestions swaps the middle slot's contents without disturbing either
    /// pinned stack or letting the middle overlap them.
    func testMentionSuggestionsSwapTheMiddleSlotWithoutDisturbingPinnedStacks() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        let host = Self.attachToFixedWidthHost(toolbar, width: 320)
        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.setItemsJSONForTesting(Self.placementToolbarFixtureJSON)
        host.setNeedsLayout()
        host.layoutIfNeeded()

        let didChange = toolbar.setMentionSuggestions([
            NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "alice",
                "attrs": ["label": "alice"],
            ])!,
        ], trigger: "@")
        host.setNeedsLayout()
        host.layoutIfNeeded()

        XCTAssertTrue(didChange, "setMentionSuggestions should report a mode change from empty to non-empty")
        XCTAssertEqual(
            toolbar.mentionButtonAtForTesting(0)?.titleTextForTesting(),
            "@alice",
            "the mention suggestion chip should render inside the content stack"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("start"),
            ["Start"],
            "start-pinned items should keep rendering while mention suggestions are shown"
        )
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("end"),
            ["End"],
            "end-pinned items should keep rendering while mention suggestions are shown"
        )

        let contentFrame = toolbar.contentStackViewFrameForTesting
        let startFrame = toolbar.startPinnedStackViewFrameForTesting
        let endFrame = toolbar.endPinnedStackViewFrameForTesting
        XCTAssertGreaterThan(
            contentFrame.width,
            0,
            "the content stack must claim the middle slot's width while showing mentions"
        )
        XCTAssertFalse(
            contentFrame.intersects(startFrame),
            "content stack frame \(contentFrame) must not overlap the start pinned stack frame \(startFrame) while mentions are shown"
        )
        XCTAssertFalse(
            contentFrame.intersects(endFrame),
            "content stack frame \(contentFrame) must not overlap the end pinned stack frame \(endFrame) while mentions are shown"
        )

        let didChangeBack = toolbar.setMentionSuggestions([], trigger: "@")
        host.setNeedsLayout()
        host.layoutIfNeeded()

        XCTAssertTrue(didChangeBack, "setMentionSuggestions should report a mode change back to empty")
        XCTAssertEqual(
            toolbar.buttonLabelsForPlacementForTesting("scroll"),
            ["Scroll One", "Scroll Two"],
            "clearing mention suggestions should restore the scrolling button row"
        )
    }

    func testMentionSuggestionChipContentViewsAllowTouchPassthrough() {
        let chip = MentionSuggestionChipButton(
            suggestion: NativeMentionSuggestion(dictionary: [
                "key": "alice",
                "title": "Alice Chen",
                "subtitle": "Design",
                "label": "@alice",
                "attrs": ["label": "@alice"],
            ])!,
            theme: nil
        )
        chip.frame = CGRect(x: 0, y: 0, width: 160, height: 44)
        chip.layoutIfNeeded()

        XCTAssertTrue(
            chip.contentViewsAllowTouchPassthroughForTesting(),
            "mention chip content views should not intercept taps from the button"
        )
    }

}
