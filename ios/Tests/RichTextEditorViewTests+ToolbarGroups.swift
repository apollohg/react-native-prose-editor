import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {
    func testAccessoryToolbarExpandsGroupedButtonsInline() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "group",
            "key": "headings",
            "label": "Headings",
            "icon": { "type": "glyph", "text": "H" },
            "presentation": "expand",
            "items": [
              {
                "type": "heading",
                "level": 1,
                "label": "Heading 1",
                "icon": { "type": "default", "id": "h1" }
              },
              {
                "type": "heading",
                "level": 2,
                "label": "Heading 2",
                "icon": { "type": "default", "id": "h2" }
              }
            ]
          }
        ]
        """)
        toolbar.applyStateJSONForTesting("""
        {
          "activeState": {
            "marks": {},
            "nodes": {},
            "commands": {
              "toggleHeading1": true,
              "toggleHeading2": true
            },
            "allowedMarks": [],
            "insertableNodes": []
          },
          "historyState": {
            "canUndo": false,
            "canRedo": false
          }
        }
        """)

        XCTAssertEqual(toolbar.buttonCountForTesting(), 1)

        toolbar.triggerButtonTapForTesting(0)

        XCTAssertEqual(toolbar.buttonCountForTesting(), 3)
        XCTAssertEqual(toolbar.buttonLabelForTesting(1), "Heading 1")
        XCTAssertEqual(toolbar.buttonLabelForTesting(2), "Heading 2")
    }

    func testAccessoryToolbarMenuGroupUsesEditMenuWithoutAttachingMenuToVisibleButton() {
        let toolbar = EditorAccessoryToolbarView(frame: CGRect(x: 0, y: 0, width: 320, height: 56))
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 160))
        let viewController = UIViewController()
        window.rootViewController = viewController
        window.makeKeyAndVisible()
        viewController.view.addSubview(toolbar)
        defer {
            toolbar.removeFromSuperview()
            window.isHidden = true
        }
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "group",
            "key": "headings",
            "label": "Headings",
            "icon": { "type": "glyph", "text": "H" },
            "presentation": "menu",
            "items": [
              {
                "type": "heading",
                "level": 1,
                "label": "Heading 1",
                "icon": { "type": "default", "id": "h1" }
              }
            ]
          },
          {
            "type": "group",
            "key": "insert",
            "label": "Insert",
            "icon": { "type": "glyph", "text": "+" },
            "presentation": "menu",
            "items": [
              {
                "type": "action",
                "key": "custom",
                "label": "Custom",
                "icon": { "type": "glyph", "text": "+" }
              }
            ]
          }
        ]
        """)
        toolbar.applyStateJSONForTesting("""
        {
          "activeState": {
            "marks": {},
            "nodes": { "h1": true },
            "commands": { "toggleHeading1": true },
            "allowedMarks": [],
            "insertableNodes": []
          },
          "historyState": {
            "canUndo": false,
            "canRedo": false
          }
        }
        """)
        toolbar.layoutIfNeeded()

        var descendants = toolbar.subviews
        var descendantIndex = 0
        while descendantIndex < descendants.count {
            descendants.append(contentsOf: descendants[descendantIndex].subviews)
            descendantIndex += 1
        }
        let visibleButton = descendants
            .compactMap { $0 as? UIButton }
            .first { $0.accessibilityLabel == "Headings" }

        XCTAssertNotNil(visibleButton)
        XCTAssertNil(visibleButton?.menu, "the visible parent button must not become UIKit's hidden menu source")
        XCTAssertEqual(visibleButton?.accessibilityHint, "Shows menu")

        guard let editMenuInteraction = toolbar.interactions.first(where: { $0 is UIEditMenuInteraction }) as? UIEditMenuInteraction else {
            return XCTFail("the toolbar should own the edit-menu presentation interaction")
        }
        defer {
            editMenuInteraction.dismissMenu()
            RunLoop.main.run(until: Date().addingTimeInterval(0.35))
        }
        toolbar.triggerButtonTapForTesting(0)
        RunLoop.main.run(until: Date().addingTimeInterval(0.35))
        let configuration = UIEditMenuConfiguration(identifier: nil, sourcePoint: .zero)
        let menu = editMenuInteraction.delegate?.editMenuInteraction?(
            editMenuInteraction,
            menuFor: configuration,
            suggestedActions: []
        )
        let headingAction = menu?.children.first as? UIAction

        XCTAssertEqual(toolbar.editMenuPresentationRequestCountForTesting, 1)
        XCTAssertEqual(menu?.title, "Headings")
        XCTAssertEqual(menu?.preferredElementSize, .large)
        XCTAssertEqual(headingAction?.title, "Heading 1")
        XCTAssertEqual(headingAction?.state, .on)
        XCTAssertFalse(headingAction?.attributes.contains(.disabled) ?? true)

        toolbar.triggerButtonTapForTesting(1)
        XCTAssertEqual(
            toolbar.editMenuPresentationRequestCountForTesting,
            2,
            "tapping a different source should immediately request its menu"
        )
        RunLoop.main.run(until: Date().addingTimeInterval(0.35))

        toolbar.triggerButtonTapForTesting(1)
        XCTAssertEqual(
            toolbar.editMenuPresentationRequestCountForTesting,
            2,
            "tapping the active source should dismiss without requesting another presentation"
        )
    }

    func testAccessoryToolbarGroupedChildrenCanOverrideParentPlacement() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "group",
            "key": "headings",
            "label": "Headings",
            "icon": { "type": "glyph", "text": "H" },
            "presentation": "expand",
            "placement": "start",
            "items": [
              {
                "type": "action",
                "key": "inherited",
                "label": "Inherited",
                "icon": { "type": "glyph", "text": "I" }
              },
              {
                "type": "action",
                "key": "pinned",
                "label": "Pinned",
                "icon": { "type": "glyph", "text": "P" },
                "placement": "end"
              }
            ]
          }
        ]
        """)

        XCTAssertEqual(toolbar.buttonLabelsForPlacementForTesting("start"), ["Headings"])
        XCTAssertEqual(toolbar.buttonLabelsForPlacementForTesting("end"), [])

        toolbar.triggerButtonTapForTesting(0)

        XCTAssertEqual(toolbar.buttonLabelsForPlacementForTesting("start"), ["Headings", "Inherited"])
        XCTAssertEqual(toolbar.buttonLabelsForPlacementForTesting("end"), ["Pinned"])
    }

    func testAccessoryToolbarEnablesListDepthCommandsForTaskLists() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "command",
            "command": "indentList",
            "label": "Indent",
            "icon": { "type": "default", "id": "indentList" }
          },
          {
            "type": "command",
            "command": "outdentList",
            "label": "Outdent",
            "icon": { "type": "default", "id": "outdentList" }
          }
        ]
        """)
        toolbar.applyStateJSONForTesting("""
        {
          "activeState": {
            "marks": {},
            "nodes": {
              "taskList": true,
              "taskItem": true
            },
            "commands": {
              "indentList": true,
              "outdentList": true
            },
            "allowedMarks": [],
            "insertableNodes": []
          },
          "historyState": {
            "canUndo": false,
            "canRedo": false
          }
        }
        """)

        XCTAssertEqual(toolbar.buttonIsEnabledForTesting(0), true)
        XCTAssertEqual(toolbar.buttonIsEnabledForTesting(1), true)
    }

    func testAccessoryToolbarGroupReflectsActiveChildState() {
        let toolbar = EditorAccessoryToolbarView(frame: .zero)
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "group",
            "key": "headings",
            "label": "Headings",
            "icon": { "type": "glyph", "text": "H" },
            "items": [
              {
                "type": "heading",
                "level": 2,
                "label": "Heading 2",
                "icon": { "type": "default", "id": "h2" }
              }
            ]
          }
        ]
        """)
        toolbar.applyStateJSONForTesting("""
        {
          "activeState": {
            "marks": {},
            "nodes": {
              "h2": true
            },
            "commands": {
              "toggleHeading2": true
            },
            "allowedMarks": [],
            "insertableNodes": []
          },
          "historyState": {
            "canUndo": false,
            "canRedo": false
          }
        }
        """)

        XCTAssertEqual(toolbar.selectedButtonCountForTesting, 1)
    }

    func testAccessoryToolbarPreservesScrolledOffsetWhenExpandingGroupedButtons() {
        let toolbar = EditorAccessoryToolbarView(frame: CGRect(x: 0, y: 0, width: 180, height: 56))
        toolbar.setItemsJSONForTesting("""
        [
          {
            "type": "action",
            "key": "bold",
            "label": "Bold",
            "icon": { "type": "default", "id": "bold" }
          },
          {
            "type": "action",
            "key": "italic",
            "label": "Italic",
            "icon": { "type": "default", "id": "italic" }
          },
          {
            "type": "action",
            "key": "underline",
            "label": "Underline",
            "icon": { "type": "default", "id": "underline" }
          },
          {
            "type": "group",
            "key": "headings",
            "label": "Headings",
            "icon": { "type": "glyph", "text": "H" },
            "presentation": "expand",
            "items": [
              {
                "type": "action",
                "key": "h1",
                "label": "Heading 1",
                "icon": { "type": "default", "id": "h1" }
              },
              {
                "type": "action",
                "key": "h2",
                "label": "Heading 2",
                "icon": { "type": "default", "id": "h2" }
              }
            ]
          },
          {
            "type": "action",
            "key": "undo",
            "label": "Undo",
            "icon": { "type": "default", "id": "undo" }
          },
          {
            "type": "action",
            "key": "redo",
            "label": "Redo",
            "icon": { "type": "default", "id": "redo" }
          }
        ]
        """)
        toolbar.layoutIfNeeded()

        let targetOffset = min(
            40,
            toolbar.nativeToolbarContentWidthForTesting - toolbar.nativeToolbarVisibleWidthForTesting
        )
        XCTAssertGreaterThan(targetOffset, 0)

        toolbar.setNativeToolbarContentOffsetXForTesting(targetOffset)
        toolbar.triggerButtonTapForTesting(3)
        toolbar.layoutIfNeeded()

        XCTAssertEqual(toolbar.nativeToolbarContentOffsetXForTesting, targetOffset, accuracy: 0.1)
    }

}
