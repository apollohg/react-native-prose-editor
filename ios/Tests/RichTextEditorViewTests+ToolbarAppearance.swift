import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testToolbarThemeParsesNativeAppearance() {
        let theme = EditorTheme(dictionary: [
            "toolbar": [
                "appearance": "native",
                "height": 44,
            ],
        ])

        XCTAssertEqual(theme.toolbar?.appearance, .native)
        XCTAssertEqual(theme.toolbar?.height ?? 0, 44, accuracy: 0.1)
        XCTAssertEqual(theme.toolbar?.resolvedKeyboardOffset ?? 0, 6, accuracy: 0.1)
        XCTAssertEqual(theme.toolbar?.resolvedHorizontalInset ?? -1, 10, accuracy: 0.1)
        XCTAssertEqual(theme.toolbar?.resolvedBorderRadius ?? -1, 20, accuracy: 0.1)
    }

    func testToolbarThemeHonorsExplicitInsetAndBorderRadius() {
        let theme = EditorTheme(dictionary: [
            "toolbar": [
                "appearance": "native",
                "horizontalInset": 10,
                "borderRadius": 22,
            ],
        ])

        XCTAssertEqual(theme.toolbar?.resolvedHorizontalInset ?? -1, 10, accuracy: 0.1)
        XCTAssertEqual(theme.toolbar?.resolvedBorderRadius ?? -1, 22, accuracy: 0.1)
    }

    func testAccessoryToolbarAppliesNativeAppearanceChrome() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
            "height": 44,
        ]))
        XCTAssertTrue(toolbar.usesNativeAppearanceForTesting)
        if #available(iOS 26.0, *) {
#if compiler(>=6.2)
            XCTAssertTrue(toolbar.usesUIGlassEffectForTesting)
#else
            XCTAssertFalse(toolbar.usesUIGlassEffectForTesting)
#endif
            XCTAssertEqual(toolbar.chromeBorderWidthForTesting, 1 / UIScreen.main.scale, accuracy: 0.1)
        } else {
            XCTAssertEqual(toolbar.chromeBorderWidthForTesting, 1 / UIScreen.main.scale, accuracy: 0.1)
        }
        XCTAssertEqual(toolbar.intrinsicContentSize.height, 50, accuracy: 0.1)
    }

    func testAccessoryToolbarAppliesSelectedStateForActiveNativeButton() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)

        toolbar.apply(theme: EditorToolbarTheme(dictionary: [
            "appearance": "native",
        ]))
        toolbar.applyBoldStateForTesting(active: true, enabled: true)

        XCTAssertEqual(toolbar.selectedButtonCountForTesting, 1)
    }

    func testDefaultAccessoryToolbarUsesProseMirrorNodeNames() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.applyStateJSONForTesting("""
        {
          "activeState": {
            "marks": {},
            "nodes": { "bullet_list": true, "list_item": true },
            "commands": { "wrapBulletList": true, "wrapOrderedList": true },
            "allowedMarks": [],
            "insertableNodes": ["hard_break", "horizontal_rule"]
          },
          "historyState": { "canUndo": false, "canRedo": false }
        }
        """)

        XCTAssertEqual(toolbar.buttonLabelForTesting(5), "Bullet List")
        XCTAssertEqual(toolbar.selectedButtonCountForTesting, 1)
        XCTAssertEqual(toolbar.buttonIsEnabledForTesting(9), true)
        XCTAssertEqual(toolbar.buttonIsEnabledForTesting(10), true)
    }

    func testNativeToolbarCascadesGlobalAndPerButtonStyles() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "action",
            "key": "global-idle",
            "label": "Global Idle",
            "icon": { "type": "glyph", "text": "G" }
          },
          {
            "type": "action",
            "key": "idle",
            "label": "Idle",
            "icon": { "type": "glyph", "text": "I" },
            "buttonStyle": { "backgroundColor": "#121212" }
          },
          {
            "type": "action",
            "key": "global-disabled",
            "label": "Global Disabled",
            "icon": { "type": "glyph", "text": "E" },
            "isActive": true,
            "isDisabled": true
          },
          {
            "type": "action",
            "key": "disabled",
            "label": "Disabled",
            "icon": { "type": "glyph", "text": "D" },
            "isActive": true,
            "isDisabled": true,
            "buttonStyle": {
              "disabledColor": "#444444",
              "disabledBackgroundColor": "#555555"
            }
          },
          {
            "type": "action",
            "key": "global-active",
            "label": "Global Active",
            "icon": { "type": "glyph", "text": "T" },
            "isActive": true
          },
          {
            "type": "action",
            "key": "active",
            "label": "Active",
            "icon": { "type": "glyph", "text": "A" },
            "isActive": true,
            "buttonStyle": {
              "iconSize": 26,
              "activeColor": "#555555",
              "activeBackgroundColor": "#666666",
              "borderRadius": 12
            }
          }
        ]
        """)
        let theme = EditorToolbarTheme(dictionary: [
            "appearance": "native",
            "buttonIconSize": 18,
            "buttonColor": "#111111",
            "buttonBackgroundColor": "#050505",
            "buttonActiveColor": "#222222",
            "buttonDisabledColor": "#333333",
            "buttonActiveBackgroundColor": "#777777",
            "buttonDisabledBackgroundColor": "#888888",
            "buttonBorderRadius": 9,
        ])
        let buttonStyle = EditorToolbarButtonStyle(dictionary: [
            "backgroundColor": "#121212",
            "disabledBackgroundColor": "#555555",
        ])

        XCTAssertEqual(theme.buttonBackgroundColor, EditorTheme.color(from: "#050505"))
        XCTAssertEqual(theme.buttonDisabledBackgroundColor, EditorTheme.color(from: "#888888"))
        XCTAssertEqual(buttonStyle.backgroundColor, EditorTheme.color(from: "#121212"))
        XCTAssertEqual(buttonStyle.disabledBackgroundColor, EditorTheme.color(from: "#555555"))

        toolbar.apply(theme: theme)

        XCTAssertEqual(toolbar.buttonTintColorForTesting(0), EditorTheme.color(from: "#111111"))
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(0),
            EditorTheme.color(from: "#050505")
        )
        XCTAssertEqual(toolbar.buttonTintColorForTesting(1), EditorTheme.color(from: "#111111"))
        XCTAssertEqual(toolbar.buttonFontSizeForTesting(1) ?? -1, 18, accuracy: 0.1)
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(1),
            EditorTheme.color(from: "#121212")
        )
        XCTAssertEqual(toolbar.buttonCornerRadiusForTesting(1) ?? -1, 9, accuracy: 0.1)
        XCTAssertEqual(toolbar.buttonTintColorForTesting(2), EditorTheme.color(from: "#333333"))
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(2),
            EditorTheme.color(from: "#888888")
        )
        XCTAssertEqual(toolbar.buttonTintColorForTesting(3), EditorTheme.color(from: "#444444"))
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(3),
            EditorTheme.color(from: "#555555")
        )
        XCTAssertEqual(toolbar.buttonTintColorForTesting(4), EditorTheme.color(from: "#222222"))
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(4),
            EditorTheme.color(from: "#777777")
        )
        XCTAssertEqual(toolbar.buttonTintColorForTesting(5), EditorTheme.color(from: "#555555"))
        XCTAssertEqual(toolbar.buttonFontSizeForTesting(5) ?? -1, 26, accuracy: 0.1)
        XCTAssertEqual(
            toolbar.buttonBackgroundColorForTesting(5),
            EditorTheme.color(from: "#666666")
        )
        XCTAssertEqual(toolbar.buttonCornerRadiusForTesting(5) ?? -1, 12, accuracy: 0.1)
    }

    /// A configured `UIButton` resolves its own selected-state background, so
    /// filling `backgroundColor` as well stacks two shapes into a double halo.
    func testActiveButtonPaintsExactlyOneBackground() {
        for appearance in ["native", "custom"] {
            let toolbar = EditorAccessoryToolbarView(frame: .zero)
            toolbar.apply(theme: EditorToolbarTheme(dictionary: [
                "appearance": appearance,
            ]))

            toolbar.applyBoldStateForTesting(active: true, enabled: true)
            XCTAssertEqual(
                toolbar.buttonBackgroundSourceCountForTesting(0),
                1,
                "an active \(appearance) button must paint one background, not stack two"
            )

            toolbar.applyBoldStateForTesting(active: false, enabled: true)
            XCTAssertEqual(
                toolbar.buttonBackgroundSourceCountForTesting(0),
                0,
                "an inactive \(appearance) button must paint no background at all"
            )
        }
    }

}
