package com.apollohg.editor

import android.graphics.Color
import android.graphics.drawable.ColorDrawable
import android.os.Looper
import android.view.View
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativeToolbarTest {

    @Test
    fun `toolbar items parse platform material icons and action state`() {
        val items = NativeToolbarItem.fromJson(
            """
            [
              {
                "type": "action",
                "key": "mention",
                "label": "Mention",
                "icon": {
                  "type": "platform",
                  "android": { "type": "material", "name": "alternate-email" },
                  "fallbackText": "@"
                },
                "isActive": true,
                "isDisabled": false
              }
            ]
            """.trimIndent()
        )

        assertEquals(1, items.size)
        assertEquals(ToolbarItemKind.action, items[0].type)
        assertEquals("alternate-email", items[0].icon?.resolvedMaterialIconName())
        assertTrue(items[0].isActive)
        assertFalse(items[0].isDisabled)
    }

    @Test
    fun `toolbar items parse heading buttons`() {
        val items = NativeToolbarItem.fromJson(
            """
            [
              {
                "type": "heading",
                "level": 3,
                "label": "Heading 3",
                "icon": { "type": "default", "id": "h3" }
              }
            ]
            """.trimIndent()
        )

        assertEquals(1, items.size)
        assertEquals(ToolbarItemKind.heading, items[0].type)
        assertEquals(3, items[0].headingLevel)
        assertEquals("H3", items[0].icon?.resolvedGlyphText())
    }

    @Test
    fun `toolbar items parse grouped buttons`() {
        val items = NativeToolbarItem.fromJson(
            """
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
            """.trimIndent()
        )

        assertEquals(1, items.size)
        assertEquals(ToolbarItemKind.group, items[0].type)
        assertEquals(ToolbarGroupPresentation.menu, items[0].presentation)
        assertEquals(2, items[0].items.size)
        assertEquals(ToolbarItemKind.heading, items[0].items[0].type)
    }

    @Test
    fun `material icon registry resolves glyph and typeface`() {
        val context = RuntimeEnvironment.getApplication()
        val glyph = MaterialIconRegistry.glyphForName(context, "alternate-email")
        val typeface = MaterialIconRegistry.typeface(context)

        assertNotNull(glyph)
        assertTrue(glyph!!.isNotEmpty())
        assertNotNull(typeface)
    }

    @Test
    fun `toolbar state parses allowed marks insertable nodes and history`() {
        val state = NativeToolbarState.fromUpdateJson(
            """
            {
              "activeState": {
                "marks": { "bold": true },
                "nodes": { "paragraph": true },
                "commands": { "wrapBulletList": true },
                "allowedMarks": ["bold", "italic"],
                "insertableNodes": ["horizontalRule", "hardBreak"]
              },
              "historyState": {
                "canUndo": true,
                "canRedo": false
              }
            }
            """.trimIndent()
        )

        requireNotNull(state)
        assertTrue(state.marks["bold"] == true)
        assertTrue(state.allowedMarks.contains("italic"))
        assertTrue(state.insertableNodes.contains("hardBreak"))
        assertTrue(state.commands["wrapBulletList"] == true)
        assertTrue(state.canUndo)
        assertFalse(state.canRedo)
    }

    @Test
    fun `native toolbar heading button uses command and node state`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)
        toolbar.setItems(
            listOf(
                NativeToolbarItem(
                    type = ToolbarItemKind.heading,
                    label = "Heading 2",
                    icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.h2),
                    headingLevel = 2
                )
            )
        )
        toolbar.applyState(
            NativeToolbarState(
                marks = emptyMap(),
                nodes = mapOf("h2" to true),
                commands = mapOf("toggleHeading2" to true),
                allowedMarks = emptySet(),
                insertableNodes = emptySet(),
                canUndo = false,
                canRedo = false
            )
        )

        val headingButton = requireNotNull(toolbar.buttonAtForTesting(0))
        assertTrue(headingButton.isEnabled)
        assertNotNull(headingButton.background)
    }

    @Test
    fun `native toolbar expands grouped buttons inline`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)
        toolbar.setItems(
            listOf(
                NativeToolbarItem(
                    type = ToolbarItemKind.group,
                    key = "headings",
                    label = "Headings",
                    icon = NativeToolbarIcon(glyphText = "H"),
                    presentation = ToolbarGroupPresentation.expand,
                    items = listOf(
                        NativeToolbarItem(
                            type = ToolbarItemKind.heading,
                            label = "Heading 1",
                            icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.h1),
                            headingLevel = 1
                        ),
                        NativeToolbarItem(
                            type = ToolbarItemKind.heading,
                            label = "Heading 2",
                            icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.h2),
                            headingLevel = 2
                        )
                    )
                )
            )
        )
        toolbar.applyState(
            NativeToolbarState(
                marks = emptyMap(),
                nodes = emptyMap(),
                commands = mapOf("toggleHeading1" to true, "toggleHeading2" to true),
                allowedMarks = emptySet(),
                insertableNodes = emptySet(),
                canUndo = false,
                canRedo = false
            )
        )

        assertEquals(1, toolbar.buttonCountForTesting())

        requireNotNull(toolbar.buttonAtForTesting(0)).performClick()

        assertEquals(3, toolbar.buttonCountForTesting())
        assertEquals("Heading 1", toolbar.buttonAtForTesting(1)?.contentDescription)
        assertEquals("Heading 2", toolbar.buttonAtForTesting(2)?.contentDescription)
    }

    @Test
    fun `native toolbar preserves horizontal scroll offset when expanding grouped buttons`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)
        fun actionItem(key: String, label: String) = NativeToolbarItem(
            type = ToolbarItemKind.action,
            key = key,
            label = label,
            icon = NativeToolbarIcon(glyphText = label)
        )
        toolbar.setItems(
            listOf(
                actionItem("bold", "B"),
                actionItem("italic", "I"),
                actionItem("underline", "U"),
                NativeToolbarItem(
                    type = ToolbarItemKind.group,
                    key = "headings",
                    label = "Headings",
                    icon = NativeToolbarIcon(glyphText = "H"),
                    presentation = ToolbarGroupPresentation.expand,
                    items = listOf(
                        actionItem("h1", "H1"),
                        actionItem("h2", "H2")
                    )
                ),
                actionItem("redo", "R"),
                actionItem("undo", "U2")
            )
        )

        val widthSpec = View.MeasureSpec.makeMeasureSpec(140, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(64, View.MeasureSpec.EXACTLY)
        toolbar.measure(widthSpec, heightSpec)
        toolbar.layout(0, 0, 140, 64)

        toolbar.scrollTo(48, 0)
        assertEquals(48, toolbar.scrollX)

        requireNotNull(toolbar.buttonAtForTesting(3)).performClick()
        shadowOf(Looper.getMainLooper()).idle()
        toolbar.measure(widthSpec, heightSpec)
        toolbar.layout(0, 0, 140, 64)

        assertEquals(48, toolbar.scrollX)
    }

    @Test
    fun `toolbar theme parses native appearance`() {
        val theme = EditorToolbarTheme.fromJson(
            org.json.JSONObject(
                """
                {
                  "appearance": "native"
                }
                """.trimIndent()
            )
        )

        assertEquals(EditorToolbarAppearance.NATIVE, theme?.appearance)
        assertEquals(8f, theme?.resolvedKeyboardOffset())
        assertEquals(0f, theme?.resolvedHorizontalInset())
    }

    @Test
    fun `toolbar switches to mention suggestion mode`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)

        toolbar.applyMentionTheme(
            EditorMentionTheme.fromJson(
                org.json.JSONObject(
                    """
                    {
                      "backgroundColor": "#d7e4ff",
                      "optionTextColor": "#1a2c48"
                    }
                    """.trimIndent()
                )
            )
        )

        val didChange = toolbar.setMentionSuggestions(
            listOf(
                NativeMentionSuggestion(
                    key = "alice",
                    title = "Alice Chen",
                    subtitle = "Design",
                    label = "@alice",
                    attrs = org.json.JSONObject().put("id", "user_alice")
                )
            )
        )

        assertTrue(didChange)
        assertTrue(toolbar.isShowingMentionSuggestions)
    }

    @Test
    fun `toolbar mention suggestion tap invokes callback and clears back to button mode`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)
        val suggestion = NativeMentionSuggestion(
            key = "alice",
            title = "Alice Chen",
            subtitle = "Design",
            label = "@alice",
            attrs = org.json.JSONObject().put("id", "user_alice")
        )
        var selectedKey: String? = null
        toolbar.onSelectMentionSuggestion = { selected ->
            selectedKey = selected.key
        }
        toolbar.setMentionSuggestions(listOf(suggestion))

        val widthSpec = View.MeasureSpec.makeMeasureSpec(480, View.MeasureSpec.AT_MOST)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(120, View.MeasureSpec.AT_MOST)
        toolbar.measure(widthSpec, heightSpec)
        toolbar.layout(0, 0, toolbar.measuredWidth, toolbar.measuredHeight)
        toolbar.triggerMentionSuggestionTapForTesting(0)

        assertEquals("alice", selectedKey)

        val didChange = toolbar.setMentionSuggestions(emptyList())

        assertTrue(didChange)
        assertFalse(toolbar.isShowingMentionSuggestions)
    }

    @Test
    fun `native toolbar applies native appearance to mention suggestions`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)
        toolbar.applyTheme(
            EditorToolbarTheme(
                appearance = EditorToolbarAppearance.NATIVE
            )
        )
        toolbar.setMentionSuggestions(
            listOf(
                NativeMentionSuggestion(
                    key = "alice",
                    title = "Alice Chen",
                    subtitle = "Design",
                    label = "@alice",
                    attrs = org.json.JSONObject().put("id", "user_alice")
                )
            )
        )

        assertTrue(toolbar.mentionChipAtForTesting(0)?.usesNativeAppearanceForTesting() == true)
    }

    @Test
    fun `toolbar theme dimensions are applied in density scaled pixels without elevation`() {
        val context = RuntimeEnvironment.getApplication()
        val density = context.resources.displayMetrics.density
        val toolbar = EditorKeyboardToolbarView(context)

        toolbar.applyTheme(
            EditorToolbarTheme(
                borderWidth = 2f,
                borderRadius = 20f,
                buttonBorderRadius = 14f
            )
        )

        assertEquals(0f, toolbar.elevation)
        assertEquals(20f * density, toolbar.appliedChromeCornerRadiusPx)
        assertEquals((2f * density).toInt().coerceAtLeast(1), toolbar.appliedChromeStrokeWidthPx)
        assertEquals(14f * density, toolbar.appliedButtonCornerRadiusPx)
    }

    @Test
    fun `native toolbar appearance uses docked material chrome defaults`() {
        val context = RuntimeEnvironment.getApplication()
        val density = context.resources.displayMetrics.density
        val toolbar = EditorKeyboardToolbarView(context)

        toolbar.applyTheme(
            EditorToolbarTheme(
                appearance = EditorToolbarAppearance.NATIVE
            )
        )

        assertEquals(EditorToolbarAppearance.NATIVE, toolbar.appliedAppearance)
        assertEquals(0, toolbar.appliedChromeStrokeWidthPx)
        assertEquals(32f * density, toolbar.appliedChromeCornerRadiusPx)
        assertEquals(20f * density, toolbar.appliedButtonCornerRadiusPx)
        assertEquals(0f, toolbar.appliedChromeElevationPx)
        assertTrue(toolbar.clipToOutline)
    }

    @Test
    fun `native toolbar separators remain visible`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)

        toolbar.applyTheme(
            EditorToolbarTheme(
                appearance = EditorToolbarAppearance.NATIVE
            )
        )

        val separator = requireNotNull(toolbar.separatorAtForTesting(0))
        val separatorDrawable = separator.background as? ColorDrawable

        assertEquals(1, separator.layoutParams.width)
        assertNotNull(separatorDrawable)
        assertNotEquals(Color.TRANSPARENT, separatorDrawable?.color)
    }

    @Test
    fun `native toolbar updates button selected and disabled colors from state`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)
        toolbar.applyTheme(
            EditorToolbarTheme(
                appearance = EditorToolbarAppearance.NATIVE
            )
        )

        toolbar.applyState(
            NativeToolbarState(
                marks = emptyMap(),
                nodes = emptyMap(),
                commands = emptyMap(),
                allowedMarks = setOf("bold"),
                insertableNodes = emptySet(),
                canUndo = false,
                canRedo = false
            )
        )

        val boldButton = requireNotNull(toolbar.buttonAtForTesting(0))
        val inactiveColor = boldButton.currentTextColor

        toolbar.applyState(
            NativeToolbarState(
                marks = mapOf("bold" to true),
                nodes = emptyMap(),
                commands = emptyMap(),
                allowedMarks = setOf("bold"),
                insertableNodes = emptySet(),
                canUndo = false,
                canRedo = false
            )
        )

        assertTrue(boldButton.isSelected)
        assertNotEquals(inactiveColor, boldButton.currentTextColor)
        assertEquals(1f, boldButton.alpha)

        toolbar.applyState(
            NativeToolbarState(
                marks = emptyMap(),
                nodes = emptyMap(),
                commands = emptyMap(),
                allowedMarks = emptySet(),
                insertableNodes = emptySet(),
                canUndo = false,
                canRedo = false
            )
        )

        assertFalse(boldButton.isEnabled)
        assertEquals(1f, boldButton.alpha)
    }

    @Test
    @Config(sdk = [34], qualifiers = "night")
    fun `native toolbar resolves non-transparent colors in dark mode`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)

        toolbar.applyTheme(
            EditorToolbarTheme(
                appearance = EditorToolbarAppearance.NATIVE
            )
        )
        toolbar.applyState(
            NativeToolbarState(
                marks = emptyMap(),
                nodes = emptyMap(),
                commands = emptyMap(),
                allowedMarks = setOf("bold"),
                insertableNodes = emptySet(),
                canUndo = false,
                canRedo = false
            )
        )

        val boldButton = requireNotNull(toolbar.buttonAtForTesting(0))
        val inactiveColor = boldButton.currentTextColor
        assertNotEquals(Color.TRANSPARENT, toolbar.appliedChromeColor)
        assertNotEquals(Color.TRANSPARENT, inactiveColor)

        toolbar.applyState(
            NativeToolbarState(
                marks = mapOf("bold" to true),
                nodes = emptyMap(),
                commands = emptyMap(),
                allowedMarks = setOf("bold"),
                insertableNodes = emptySet(),
                canUndo = false,
                canRedo = false
            )
        )

        assertNotEquals(inactiveColor, boldButton.currentTextColor)
        assertNotEquals(Color.TRANSPARENT, toolbar.buttonBackgroundColorAtForTesting(0))
    }
}
