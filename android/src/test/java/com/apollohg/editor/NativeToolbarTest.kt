package com.apollohg.editor

import android.view.View
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
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
}
