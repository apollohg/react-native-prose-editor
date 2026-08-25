package com.apollohg.editor

import android.graphics.Color
import android.graphics.Rect
import android.graphics.drawable.ColorDrawable
import android.os.Looper
import android.view.View
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import androidx.appcompat.R as AppCompatR
import androidx.appcompat.view.ContextThemeWrapper
import com.google.android.material.R as MaterialR
import com.google.android.material.color.MaterialColors
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
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
                "placement": "start",
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
                    "icon": { "type": "default", "id": "h2" },
                    "placement": "end"
                  }
                ]
              }
            ]
            """.trimIndent()
        )

        assertEquals(1, items.size)
        assertEquals(ToolbarItemKind.group, items[0].type)
        assertEquals(ToolbarGroupPresentation.menu, items[0].presentation)
        assertEquals(ToolbarItemPlacement.start, items[0].placement)
        assertEquals(2, items[0].items.size)
        assertEquals(ToolbarItemKind.heading, items[0].items[0].type)
        assertEquals(ToolbarItemPlacement.end, items[0].items[1].placement)
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
    fun `native toolbar enables list depth commands for task lists`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)
        toolbar.setItems(
            listOf(
                NativeToolbarItem(
                    type = ToolbarItemKind.command,
                    label = "Indent",
                    icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.indentList),
                    command = ToolbarCommand.indentList
                ),
                NativeToolbarItem(
                    type = ToolbarItemKind.command,
                    label = "Outdent",
                    icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.outdentList),
                    command = ToolbarCommand.outdentList
                )
            )
        )
        toolbar.applyState(
            NativeToolbarState(
                marks = emptyMap(),
                nodes = mapOf("taskList" to true, "taskItem" to true),
                commands = mapOf("indentList" to true, "outdentList" to true),
                allowedMarks = emptySet(),
                insertableNodes = emptySet(),
                canUndo = false,
                canRedo = false
            )
        )

        assertTrue(requireNotNull(toolbar.buttonAtForTesting(0)).isEnabled)
        assertTrue(requireNotNull(toolbar.buttonAtForTesting(1)).isEnabled)
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
    fun `native toolbar lets grouped children override parent placement`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)
        toolbar.setItems(
            listOf(
                NativeToolbarItem(
                    type = ToolbarItemKind.group,
                    key = "headings",
                    label = "Headings",
                    icon = NativeToolbarIcon(glyphText = "H"),
                    placement = ToolbarItemPlacement.start,
                    presentation = ToolbarGroupPresentation.expand,
                    items = listOf(
                        NativeToolbarItem(
                            type = ToolbarItemKind.action,
                            key = "inherited",
                            label = "Inherited",
                            icon = NativeToolbarIcon(glyphText = "I")
                        ),
                        NativeToolbarItem(
                            type = ToolbarItemKind.action,
                            key = "pinned",
                            label = "Pinned",
                            icon = NativeToolbarIcon(glyphText = "P"),
                            placement = ToolbarItemPlacement.end
                        )
                    )
                )
            )
        )

        assertEquals(listOf("Headings"), toolbar.buttonLabelsForPlacementForTesting(ToolbarItemPlacement.start))
        assertEquals(emptyList<String>(), toolbar.buttonLabelsForPlacementForTesting(ToolbarItemPlacement.end))

        requireNotNull(toolbar.buttonAtForTesting(0)).performClick()

        assertEquals(
            listOf("Headings", "Inherited"),
            toolbar.buttonLabelsForPlacementForTesting(ToolbarItemPlacement.start)
        )
        assertEquals(listOf("Pinned"), toolbar.buttonLabelsForPlacementForTesting(ToolbarItemPlacement.end))
    }

    @Test
    fun `native toolbar pins end placement inside the viewport when middle content overflows`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)
        fun actionItem(key: String, placement: ToolbarItemPlacement? = null) = NativeToolbarItem(
            type = ToolbarItemKind.action,
            key = key,
            label = key,
            icon = NativeToolbarIcon(glyphText = key),
            placement = placement
        )
        toolbar.setItems(
            listOf(
                actionItem("one"),
                actionItem("two"),
                actionItem("three"),
                actionItem("four"),
                actionItem("five"),
                actionItem("six"),
                actionItem("end", ToolbarItemPlacement.end)
            )
        )

        val width = 240
        val widthSpec = View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(64, View.MeasureSpec.EXACTLY)
        toolbar.measure(widthSpec, heightSpec)
        toolbar.layout(0, 0, width, 64)

        val endButton = requireNotNull(toolbar.buttonAtForTesting(6))
        val endBounds = Rect(0, 0, endButton.width, endButton.height)
        toolbar.offsetDescendantRectToMyCoords(endButton, endBounds)
        val rootRow = toolbar.getChildAt(0) as LinearLayout
        val centerScrollView = rootRow.getChildAt(1) as HorizontalScrollView
        val endMargin = (endButton.layoutParams as LinearLayout.LayoutParams).marginEnd

        assertTrue(
            "middle content must overflow its viewport for this regression",
            centerScrollView.getChildAt(0).width > centerScrollView.width
        )
        assertTrue(
            "middle content must be clipped before the transparent pinned region",
            centerScrollView.clipChildren
        )
        assertEquals(Color.TRANSPARENT, toolbar.buttonBackgroundColorAtForTesting(6))
        assertEquals(width - rootRow.paddingRight - endMargin, endBounds.right)
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

        val rootRow = toolbar.getChildAt(0) as LinearLayout
        val centerScrollView = rootRow.getChildAt(1) as HorizontalScrollView
        centerScrollView.scrollTo(48, 0)
        assertEquals(48, centerScrollView.scrollX)

        requireNotNull(toolbar.buttonAtForTesting(3)).performClick()
        shadowOf(Looper.getMainLooper()).idle()
        toolbar.measure(widthSpec, heightSpec)
        toolbar.layout(0, 0, 140, 64)

        assertEquals(48, centerScrollView.scrollX)
    }

    @Test
    fun `toolbar theme parses native appearance`() {
        val theme = EditorToolbarTheme.fromJson(
            org.json.JSONObject(
                """
                {
                  "appearance": "native",
                  "height": 44
                }
                """.trimIndent()
            )
        )

        assertEquals(EditorToolbarAppearance.NATIVE, theme?.appearance)
        assertEquals(44f, theme?.height)
        assertEquals(8f, theme?.resolvedKeyboardOffset())
        assertEquals(0f, theme?.resolvedHorizontalInset())
        assertEquals(0f, theme?.resolvedBorderRadius())
    }

    @Test
    fun `native toolbar cascades global and per-button icon and state styles`() {
        val context = RuntimeEnvironment.getApplication()
        val density = context.resources.displayMetrics.density
        val scaledDensity = density * context.resources.configuration.fontScale
        val toolbar = EditorKeyboardToolbarView(context)
        val items = NativeToolbarItem.fromJson(
            """
            [
              {
                "type":"action","key":"global-idle","label":"Global Idle",
                "icon":{"type":"glyph","text":"G"}
              },
              {
                "type":"action","key":"idle","label":"Idle",
                "icon":{"type":"glyph","text":"I"},
                "buttonStyle":{"backgroundColor":"#121212"}
              },
              {
                "type":"action","key":"global-disabled","label":"Global Disabled",
                "icon":{"type":"glyph","text":"E"},"isActive":true,"isDisabled":true
              },
              {
                "type":"action","key":"disabled","label":"Disabled",
                "icon":{"type":"glyph","text":"D"},"isActive":true,"isDisabled":true,
                "buttonStyle":{
                  "disabledColor":"#444444",
                  "disabledBackgroundColor":"#555555"
                }
              },
              {
                "type":"action","key":"global-active","label":"Global Active",
                "icon":{"type":"glyph","text":"T"},"isActive":true
              },
              {
                "type":"action","key":"active","label":"Active",
                "icon":{"type":"glyph","text":"A"},"isActive":true,
                "buttonStyle":{
                  "iconSize":26,
                  "activeColor":"#555555",
                  "activeBackgroundColor":"#666666",
                  "borderRadius":12
                }
              }
            ]
            """.trimIndent()
        )
        val theme = EditorToolbarTheme.fromJson(
            org.json.JSONObject(
                """
                {
                  "appearance":"native",
                  "buttonIconSize":18,
                  "buttonColor":"#111111",
                  "buttonBackgroundColor":"#050505",
                  "buttonActiveColor":"#222222",
                  "buttonDisabledColor":"#333333",
                  "buttonActiveBackgroundColor":"#777777",
                  "buttonDisabledBackgroundColor":"#888888",
                  "buttonBorderRadius":9
                }
                """.trimIndent()
            )
        )

        toolbar.setItems(items)
        toolbar.applyTheme(theme)
        toolbar.applyState(NativeToolbarState.empty)

        val globalIdle = requireNotNull(toolbar.buttonAtForTesting(0))
        val idle = requireNotNull(toolbar.buttonAtForTesting(1))
        val globalDisabled = requireNotNull(toolbar.buttonAtForTesting(2))
        val disabled = requireNotNull(toolbar.buttonAtForTesting(3))
        val globalActive = requireNotNull(toolbar.buttonAtForTesting(4))
        val active = requireNotNull(toolbar.buttonAtForTesting(5))
        assertEquals(Color.parseColor("#111111"), globalIdle.currentTextColor)
        assertEquals(
            Color.parseColor("#050505"),
            toolbar.buttonBackgroundColorAtForTesting(0)
        )
        assertEquals(Color.parseColor("#111111"), idle.currentTextColor)
        assertEquals(18f * scaledDensity, idle.textSize, 0.01f)
        assertEquals(Color.parseColor("#121212"), toolbar.buttonBackgroundColorAtForTesting(1))
        assertEquals(9f * density, toolbar.buttonCornerRadiusAtForTesting(1) ?: -1f, 0.01f)
        assertEquals(Color.parseColor("#333333"), globalDisabled.currentTextColor)
        assertEquals(
            Color.parseColor("#888888"),
            toolbar.buttonBackgroundColorAtForTesting(2)
        )
        assertEquals(Color.parseColor("#444444"), disabled.currentTextColor)
        assertEquals(Color.parseColor("#555555"), toolbar.buttonBackgroundColorAtForTesting(3))
        assertEquals(Color.parseColor("#222222"), globalActive.currentTextColor)
        assertEquals(
            Color.parseColor("#777777"),
            toolbar.buttonBackgroundColorAtForTesting(4)
        )
        assertEquals(Color.parseColor("#555555"), active.currentTextColor)
        assertEquals(26f * scaledDensity, active.textSize, 0.01f)
        assertEquals(Color.parseColor("#666666"), toolbar.buttonBackgroundColorAtForTesting(5))
        assertEquals(12f * density, toolbar.buttonCornerRadiusAtForTesting(5) ?: -1f, 0.01f)
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
                      "suggestions": {
                        "option": {
                          "backgroundColor": "#d7e4ff",
                          "textColor": "#1a2c48"
                        }
                      }
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
                    label = "alice",
                    attrs = org.json.JSONObject().put("id", "user_alice")
                )
            ),
            trigger = "@"
        )

        assertTrue(didChange)
        assertTrue(toolbar.isShowingMentionSuggestions)
        assertEquals("@alice", toolbar.mentionChipAtForTesting(0)?.titleTextForTesting())
    }

    @Test
    fun `toolbar keeps retained mention chips mounted while query narrows`() {
        val context = RuntimeEnvironment.getApplication()
        val toolbar = EditorKeyboardToolbarView(context)
        val alice = NativeMentionSuggestion(
            key = "alice",
            title = "Alice Chen",
            subtitle = "Design",
            label = "alice",
            attrs = org.json.JSONObject().put("id", "user_alice")
        )
        val ben = NativeMentionSuggestion(
            key = "ben",
            title = "Ben Ortiz",
            subtitle = "Engineering",
            label = "ben",
            attrs = org.json.JSONObject().put("id", "user_ben")
        )

        toolbar.setMentionSuggestions(listOf(alice, ben), trigger = "@")
        val retainedChip = toolbar.mentionChipAtForTesting(0)

        toolbar.setMentionSuggestions(listOf(alice), trigger = "@")

        assertSame(retainedChip, toolbar.mentionChipAtForTesting(0))
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
    fun `themed button size caps at the native max even in custom appearance`() {
        val context = RuntimeEnvironment.getApplication()
        val density = context.resources.displayMetrics.density
        val toolbar = EditorKeyboardToolbarView(context)

        toolbar.applyTheme(
            EditorToolbarTheme(
                appearance = EditorToolbarAppearance.CUSTOM,
                height = 60f
            )
        )

        val button = requireNotNull(toolbar.buttonAtForTesting(0))

        // Sizing contract shared with src/EditorToolbar.tsx (resolvedButtonHeight) and
        // ios/NativeEditorExpoView.swift (resolvedButtonSize):
        // max(1, min(40, 60 - 4)) = 40 — previously capped at 36dp for non-native appearance.
        assertEquals((40f * density).toInt(), button.minimumWidth)
        assertEquals((40f * density).toInt(), button.minimumHeight)
    }

    @Test
    fun `themed button size honors heights below the native default in custom appearance`() {
        val context = RuntimeEnvironment.getApplication()
        val density = context.resources.displayMetrics.density
        val toolbar = EditorKeyboardToolbarView(context)

        toolbar.applyTheme(
            EditorToolbarTheme(
                appearance = EditorToolbarAppearance.CUSTOM,
                height = 32f
            )
        )

        val button = requireNotNull(toolbar.buttonAtForTesting(0))

        // max(1, min(40, 32 - 4)) = 28 — matches the JS/iOS formula exactly.
        assertEquals((28f * density).toInt(), button.minimumWidth)
        assertEquals((28f * density).toInt(), button.minimumHeight)
    }

    @Test
    fun `native toolbar appearance ignores themed height and integrates with the keyboard`() {
        val context = RuntimeEnvironment.getApplication()
        val density = context.resources.displayMetrics.density
        val toolbar = EditorKeyboardToolbarView(context)

        toolbar.applyTheme(
            EditorToolbarTheme(
                appearance = EditorToolbarAppearance.NATIVE,
                height = 42f
            )
        )

        assertEquals(EditorToolbarAppearance.NATIVE, toolbar.appliedAppearance)
        assertEquals(0, toolbar.appliedChromeStrokeWidthPx)
        assertEquals(0f, toolbar.appliedChromeCornerRadiusPx)
        assertEquals(20f * density, toolbar.appliedButtonCornerRadiusPx)
        assertEquals(0f, toolbar.appliedChromeElevationPx)
        assertFalse(toolbar.clipToOutline)

        val rootRow = toolbar.getChildAt(0)
        assertEquals((14f * density).toInt(), rootRow.paddingTop)
        assertEquals((14f * density).toInt(), rootRow.paddingBottom)

        toolbar.measure(
            View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        )
        assertEquals((68f * density).toInt(), toolbar.measuredHeight)
    }

    @Test
    @Config(sdk = [30])
    fun `native toolbar uses material 3 colors when dynamic colors are unavailable`() {
        val application = RuntimeEnvironment.getApplication()
        val appCompatContext = ContextThemeWrapper(
            application,
            AppCompatR.style.Theme_AppCompat_Light_NoActionBar
        )
        val material3Context = ContextThemeWrapper(
            application,
            MaterialR.style.Theme_Material3_DayNight
        )
        val expectedSurface = MaterialColors.getColor(
            material3Context,
            MaterialR.attr.colorSurfaceContainer,
            Color.TRANSPARENT
        )
        val toolbar = EditorKeyboardToolbarView(appCompatContext)

        toolbar.applyTheme(
            EditorToolbarTheme(
                appearance = EditorToolbarAppearance.NATIVE
            )
        )

        assertNotEquals(Color.TRANSPARENT, expectedSurface)
        assertEquals(expectedSurface, toolbar.appliedChromeColor)
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
