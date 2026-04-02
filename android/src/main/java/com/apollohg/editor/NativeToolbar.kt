package com.apollohg.editor

import android.content.Context
import android.os.Build
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import androidx.appcompat.widget.AppCompatButton
import androidx.appcompat.widget.AppCompatTextView
import androidx.appcompat.content.res.AppCompatResources
import androidx.core.view.setPadding
import com.google.android.material.R as MaterialR
import com.google.android.material.color.DynamicColors
import org.json.JSONObject
import kotlin.math.roundToInt

internal data class NativeToolbarState(
    val marks: Map<String, Boolean>,
    val nodes: Map<String, Boolean>,
    val commands: Map<String, Boolean>,
    val allowedMarks: Set<String>,
    val insertableNodes: Set<String>,
    val canUndo: Boolean,
    val canRedo: Boolean
) {
    companion object {
        val empty = NativeToolbarState(
            marks = emptyMap(),
            nodes = emptyMap(),
            commands = emptyMap(),
            allowedMarks = emptySet(),
            insertableNodes = emptySet(),
            canUndo = false,
            canRedo = false
        )

        fun fromUpdateJson(updateJson: String): NativeToolbarState? {
            val root = try {
                JSONObject(updateJson)
            } catch (_: Exception) {
                return null
            }
            val activeState = root.optJSONObject("activeState") ?: JSONObject()
            val historyState = root.optJSONObject("historyState") ?: JSONObject()
            return NativeToolbarState(
                marks = boolMap(activeState.optJSONObject("marks")),
                nodes = boolMap(activeState.optJSONObject("nodes")),
                commands = boolMap(activeState.optJSONObject("commands")),
                allowedMarks = stringSet(activeState.optJSONArray("allowedMarks")),
                insertableNodes = stringSet(activeState.optJSONArray("insertableNodes")),
                canUndo = historyState.optBoolean("canUndo", false),
                canRedo = historyState.optBoolean("canRedo", false)
            )
        }

        private fun boolMap(json: JSONObject?): Map<String, Boolean> {
            json ?: return emptyMap()
            val result = mutableMapOf<String, Boolean>()
            val keys = json.keys()
            while (keys.hasNext()) {
                val key = keys.next()
                result[key] = json.optBoolean(key, false)
            }
            return result
        }

        private fun stringSet(array: org.json.JSONArray?): Set<String> {
            array ?: return emptySet()
            val result = linkedSetOf<String>()
            for (index in 0 until array.length()) {
                array.optString(index, null)?.let { result.add(it) }
            }
            return result
        }
    }
}

internal enum class ToolbarCommand {
    indentList,
    outdentList,
    undo,
    redo,
}

internal enum class ToolbarListType {
    bulletList,
    orderedList,
}

internal enum class ToolbarDefaultIconId {
    bold,
    italic,
    underline,
    strike,
    link,
    blockquote,
    bulletList,
    orderedList,
    indentList,
    outdentList,
    lineBreak,
    horizontalRule,
    undo,
    redo,
}

internal enum class ToolbarItemKind {
    mark,
    blockquote,
    list,
    command,
    node,
    action,
    separator,
}

internal data class NativeToolbarIcon(
    val defaultId: ToolbarDefaultIconId? = null,
    val glyphText: String? = null,
    val fallbackText: String? = null,
    val materialIconName: String? = null
) {
    companion object {
        private val defaultGlyphs = mapOf(
            ToolbarDefaultIconId.bold to "B",
            ToolbarDefaultIconId.italic to "I",
            ToolbarDefaultIconId.underline to "U",
            ToolbarDefaultIconId.strike to "S",
            ToolbarDefaultIconId.link to "🔗",
            ToolbarDefaultIconId.blockquote to "❝",
            ToolbarDefaultIconId.bulletList to "•≡",
            ToolbarDefaultIconId.orderedList to "1.",
            ToolbarDefaultIconId.indentList to "→",
            ToolbarDefaultIconId.outdentList to "←",
            ToolbarDefaultIconId.lineBreak to "↵",
            ToolbarDefaultIconId.horizontalRule to "—",
            ToolbarDefaultIconId.undo to "↩",
            ToolbarDefaultIconId.redo to "↪"
        )
        private val defaultMaterialIcons = mapOf(
            ToolbarDefaultIconId.bold to "format-bold",
            ToolbarDefaultIconId.italic to "format-italic",
            ToolbarDefaultIconId.underline to "format-underlined",
            ToolbarDefaultIconId.strike to "strikethrough-s",
            ToolbarDefaultIconId.link to "link",
            ToolbarDefaultIconId.blockquote to "format-quote",
            ToolbarDefaultIconId.bulletList to "format-list-bulleted",
            ToolbarDefaultIconId.orderedList to "format-list-numbered",
            ToolbarDefaultIconId.indentList to "format-indent-increase",
            ToolbarDefaultIconId.outdentList to "format-indent-decrease",
            ToolbarDefaultIconId.lineBreak to "keyboard-return",
            ToolbarDefaultIconId.horizontalRule to "horizontal-rule",
            ToolbarDefaultIconId.undo to "undo",
            ToolbarDefaultIconId.redo to "redo"
        )

        fun fromJson(raw: JSONObject?): NativeToolbarIcon? {
            raw ?: return null
            return when (raw.optString("type")) {
                "default" -> {
                    val id = runCatching {
                        ToolbarDefaultIconId.valueOf(raw.getString("id"))
                    }.getOrNull() ?: return null
                    NativeToolbarIcon(defaultId = id)
                }
                "glyph" -> {
                    val text = raw.optString("text")
                    if (text.isBlank()) null else NativeToolbarIcon(glyphText = text)
                }
                "platform" -> {
                    val materialName = raw.optJSONObject("android")
                        ?.takeIf { it.optString("type") == "material" }
                        ?.optNullableString("name")
                    val fallback = raw.optNullableString("fallbackText")
                    if (materialName.isNullOrBlank() && fallback.isNullOrBlank()) {
                        null
                    } else {
                        NativeToolbarIcon(
                            fallbackText = fallback,
                            materialIconName = materialName
                        )
                    }
                }
                else -> null
            }
        }

        fun defaultMaterialIconName(defaultId: ToolbarDefaultIconId?): String? =
            defaultId?.let { defaultMaterialIcons[it] }
    }

    fun resolvedGlyphText(): String =
        glyphText?.takeIf { it.isNotBlank() }
            ?: fallbackText?.takeIf { it.isNotBlank() }
            ?: defaultId?.let { defaultGlyphs[it] }
            ?: "?"

    fun resolvedMaterialIconName(): String? =
        materialIconName?.takeIf { it.isNotBlank() }
            ?: Companion.defaultMaterialIconName(defaultId)
}

internal object MaterialIconRegistry {
    private const val FONT_ASSET_PATH = "editor-icons/MaterialIcons.ttf"
    private const val GLYPHMAP_ASSET_PATH = "editor-icons/MaterialIcons.json"

    @Volatile
    private var typeface: Typeface? = null

    @Volatile
    private var glyphMap: Map<String, String>? = null

    fun typeface(context: Context): Typeface? {
        val cached = typeface
        if (cached != null) return cached
        return runCatching {
            Typeface.createFromAsset(context.assets, FONT_ASSET_PATH)
        }.getOrNull()?.also { loaded ->
            typeface = loaded
        }
    }

    fun glyphForName(context: Context, name: String?): String? {
        if (name.isNullOrBlank()) return null
        val map = glyphMap ?: loadGlyphMap(context).also { loaded ->
            glyphMap = loaded
        }
        return map[name]
    }

    private fun loadGlyphMap(context: Context): Map<String, String> {
        val assetText = runCatching {
            context.assets.open(GLYPHMAP_ASSET_PATH).bufferedReader().use { it.readText() }
        }.getOrNull() ?: return emptyMap()

        val json = runCatching { JSONObject(assetText) }.getOrNull() ?: return emptyMap()
        val result = linkedMapOf<String, String>()
        val keys = json.keys()
        while (keys.hasNext()) {
            val key = keys.next()
            val codePoint = json.optInt(key, -1)
            if (codePoint > 0) {
                result[key] = String(Character.toChars(codePoint))
            }
        }
        return result
    }
}

internal data class NativeToolbarResolvedIcon(
    val text: String,
    val typeface: Typeface? = null
)

private fun NativeToolbarIcon.resolveForAndroid(context: Context): NativeToolbarResolvedIcon {
    val materialName = resolvedMaterialIconName()
    val materialGlyph = MaterialIconRegistry.glyphForName(context, materialName)
    val materialTypeface = MaterialIconRegistry.typeface(context)
    if (materialGlyph != null && materialTypeface != null) {
        return NativeToolbarResolvedIcon(
            text = materialGlyph,
            typeface = materialTypeface
        )
    }

    return NativeToolbarResolvedIcon(
        text = resolvedGlyphText(),
        typeface = null
    )
}

internal data class NativeToolbarItem(
    val type: ToolbarItemKind,
    val key: String? = null,
    val label: String? = null,
    val icon: NativeToolbarIcon? = null,
    val mark: String? = null,
    val listType: ToolbarListType? = null,
    val command: ToolbarCommand? = null,
    val nodeType: String? = null,
    val isActive: Boolean = false,
    val isDisabled: Boolean = false
) {
    companion object {
        val defaults = listOf(
            NativeToolbarItem(ToolbarItemKind.mark, label = "Bold", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.bold), mark = "bold"),
            NativeToolbarItem(ToolbarItemKind.mark, label = "Italic", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.italic), mark = "italic"),
            NativeToolbarItem(ToolbarItemKind.mark, label = "Underline", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.underline), mark = "underline"),
            NativeToolbarItem(ToolbarItemKind.mark, label = "Strikethrough", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.strike), mark = "strike"),
            NativeToolbarItem(ToolbarItemKind.blockquote, label = "Blockquote", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.blockquote)),
            NativeToolbarItem(ToolbarItemKind.separator),
            NativeToolbarItem(ToolbarItemKind.list, label = "Bullet List", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.bulletList), listType = ToolbarListType.bulletList),
            NativeToolbarItem(ToolbarItemKind.list, label = "Ordered List", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.orderedList), listType = ToolbarListType.orderedList),
            NativeToolbarItem(ToolbarItemKind.command, label = "Indent List", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.indentList), command = ToolbarCommand.indentList),
            NativeToolbarItem(ToolbarItemKind.command, label = "Outdent List", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.outdentList), command = ToolbarCommand.outdentList),
            NativeToolbarItem(ToolbarItemKind.node, label = "Line Break", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.lineBreak), nodeType = "hardBreak"),
            NativeToolbarItem(ToolbarItemKind.node, label = "Horizontal Rule", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.horizontalRule), nodeType = "horizontalRule"),
            NativeToolbarItem(ToolbarItemKind.separator),
            NativeToolbarItem(ToolbarItemKind.command, label = "Undo", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.undo), command = ToolbarCommand.undo),
            NativeToolbarItem(ToolbarItemKind.command, label = "Redo", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.redo), command = ToolbarCommand.redo)
        )

        fun fromJson(json: String?): List<NativeToolbarItem> {
            if (json.isNullOrBlank()) return defaults
            val rawArray = try {
                org.json.JSONArray(json)
            } catch (_: Exception) {
                return defaults
            }
            val parsed = mutableListOf<NativeToolbarItem>()
            for (index in 0 until rawArray.length()) {
                val rawItem = rawArray.optJSONObject(index) ?: continue
                val type = runCatching {
                    ToolbarItemKind.valueOf(rawItem.getString("type"))
                }.getOrNull() ?: continue
                val key = rawItem.optNullableString("key")
                when (type) {
                    ToolbarItemKind.separator -> parsed.add(NativeToolbarItem(type = type, key = key))
                    ToolbarItemKind.mark -> {
                        val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: continue
                        val mark = rawItem.optNullableString("mark") ?: continue
                        val label = rawItem.optNullableString("label") ?: continue
                        parsed.add(NativeToolbarItem(type, key, label, icon, mark = mark))
                    }
                    ToolbarItemKind.blockquote -> {
                        val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: continue
                        val label = rawItem.optNullableString("label") ?: continue
                        parsed.add(NativeToolbarItem(type, key, label, icon))
                    }
                    ToolbarItemKind.list -> {
                        val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: continue
                        val listType = runCatching {
                            ToolbarListType.valueOf(rawItem.getString("listType"))
                        }.getOrNull() ?: continue
                        val label = rawItem.optNullableString("label") ?: continue
                        parsed.add(NativeToolbarItem(type, key, label, icon, listType = listType))
                    }
                    ToolbarItemKind.command -> {
                        val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: continue
                        val command = runCatching {
                            ToolbarCommand.valueOf(rawItem.getString("command"))
                        }.getOrNull() ?: continue
                        val label = rawItem.optNullableString("label") ?: continue
                        parsed.add(NativeToolbarItem(type, key, label, icon, command = command))
                    }
                    ToolbarItemKind.node -> {
                        val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: continue
                        val nodeType = rawItem.optNullableString("nodeType") ?: continue
                        val label = rawItem.optNullableString("label") ?: continue
                        parsed.add(NativeToolbarItem(type, key, label, icon, nodeType = nodeType))
                    }
                    ToolbarItemKind.action -> {
                        val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: continue
                        val keyValue = rawItem.optNullableString("key") ?: continue
                        val label = rawItem.optNullableString("label") ?: continue
                        parsed.add(
                            NativeToolbarItem(
                                type = type,
                                key = keyValue,
                                label = label,
                                icon = icon,
                                isActive = rawItem.optBoolean("isActive", false),
                                isDisabled = rawItem.optBoolean("isDisabled", false)
                            )
                        )
                    }
                }
            }
            return parsed.ifEmpty { defaults }
        }
    }
}

internal class EditorKeyboardToolbarView(context: Context) : HorizontalScrollView(context) {
    private companion object {
        private const val NATIVE_CONTAINER_HEIGHT_DP = 64
        private const val NATIVE_CONTAINER_HORIZONTAL_PADDING_DP = 16
        private const val NATIVE_CONTAINER_VERTICAL_PADDING_DP = 12
        private const val NATIVE_BUTTON_SIZE_DP = 40
        private const val NATIVE_BUTTON_ICON_SIZE_SP = 24f
        private const val NATIVE_ITEM_SPACING_DP = 8
        private const val NATIVE_GROUP_SPACING_DP = 12
    }

    private data class ButtonBinding(
        val item: NativeToolbarItem,
        val button: AppCompatButton
    )

    var onPressItem: ((NativeToolbarItem) -> Unit)? = null
    var onSelectMentionSuggestion: ((NativeMentionSuggestion) -> Unit)? = null

    private val themedContext: Context = DynamicColors.wrapContextIfAvailable(context)
    private val contentRow = LinearLayout(context)
    private var theme: EditorToolbarTheme? = null
    private var mentionTheme: EditorMentionTheme? = null
    private var state: NativeToolbarState = NativeToolbarState.empty
    private var items: List<NativeToolbarItem> = NativeToolbarItem.defaults
    private var mentionSuggestions: List<NativeMentionSuggestion> = emptyList()
    private val bindings = mutableListOf<ButtonBinding>()
    private val separators = mutableListOf<View>()
    private val mentionChips = mutableListOf<MentionSuggestionChipView>()
    private val buttonBackgroundColors = mutableMapOf<AppCompatButton, Int>()
    private val density = resources.displayMetrics.density
    internal var appliedAppearance: EditorToolbarAppearance = EditorToolbarAppearance.CUSTOM
        private set
    internal var appliedChromeCornerRadiusPx: Float = 0f
        private set
    internal var appliedChromeStrokeWidthPx: Int = 0
        private set
    internal var appliedChromeElevationPx: Float = 0f
        private set
    internal var appliedChromeColor: Int = Color.TRANSPARENT
        private set
    internal var appliedButtonCornerRadiusPx: Float = 0f
        private set
    val isShowingMentionSuggestions: Boolean
        get() = mentionSuggestions.isNotEmpty()

    init {
        isHorizontalScrollBarEnabled = false
        overScrollMode = OVER_SCROLL_NEVER
        setBackgroundColor(Color.TRANSPARENT)
        clipToPadding = false
        clipChildren = false
        isFillViewport = true

        contentRow.orientation = LinearLayout.HORIZONTAL
        contentRow.gravity = Gravity.START or Gravity.CENTER_VERTICAL
        contentRow.setPadding(dp(12))
        contentRow.clipToPadding = false
        contentRow.clipChildren = false
        addView(
            contentRow,
            LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT)
        )
        rebuildContent()
    }

    fun setItems(items: List<NativeToolbarItem>) {
        this.items = compactItems(items)
        if (!isShowingMentionSuggestions) {
            rebuildContent()
        }
    }

    fun applyTheme(theme: EditorToolbarTheme?) {
        this.theme = theme
        updateChrome()
        separators.forEach { separator ->
            separator.setBackgroundColor(resolveSeparatorColor())
        }
        bindings.forEach { binding ->
            updateButtonAppearance(
                binding.button,
                enabled = buttonState(binding.item, state).first,
                active = buttonState(binding.item, state).second
            )
        }
        mentionChips.forEach { chip ->
            chip.applyTheme(mentionTheme, theme?.appearance ?: EditorToolbarAppearance.CUSTOM)
        }
    }

    fun applyMentionTheme(theme: EditorMentionTheme?) {
        mentionTheme = theme
        mentionChips.forEach { chip ->
            chip.applyTheme(theme, this.theme?.appearance ?: EditorToolbarAppearance.CUSTOM)
        }
    }

    fun applyState(state: NativeToolbarState) {
        this.state = state
        bindings.forEach { binding ->
            val (enabled, active) = buttonState(binding.item, state)
            binding.button.isEnabled = enabled
            binding.button.isSelected = active
            updateButtonAppearance(binding.button, enabled, active)
        }
    }

    fun setMentionSuggestions(suggestions: List<NativeMentionSuggestion>): Boolean {
        val hadSuggestions = isShowingMentionSuggestions
        mentionSuggestions = suggestions.take(8)
        rebuildContent()
        return hadSuggestions != isShowingMentionSuggestions
    }

    fun triggerMentionSuggestionTapForTesting(index: Int) {
        mentionChips.getOrNull(index)?.performClick()
    }

    internal fun buttonAtForTesting(index: Int): AppCompatButton? =
        bindings.getOrNull(index)?.button

    internal fun buttonBackgroundColorAtForTesting(index: Int): Int? =
        bindings.getOrNull(index)?.button?.let { buttonBackgroundColors[it] }

    internal fun mentionChipAtForTesting(index: Int): MentionSuggestionChipView? =
        mentionChips.getOrNull(index)

    internal fun separatorAtForTesting(index: Int): View? =
        separators.getOrNull(index)

    private fun rebuildContent() {
        bindings.clear()
        separators.clear()
        mentionChips.clear()
        contentRow.removeAllViews()

        if (isShowingMentionSuggestions) {
            rebuildMentionSuggestions()
        } else {
            rebuildButtons()
        }

        updateChrome()
        applyState(state)
        scrollTo(0, 0)
    }

    private fun rebuildButtons() {
        for (item in compactItems(items)) {
            if (item.type == ToolbarItemKind.separator) {
                val separator = View(context)
                configureSeparator(separator)
                separators.add(separator)
                contentRow.addView(separator)
                continue
            }

            val button = AppCompatButton(themedContext).apply {
                val resolvedIcon = item.icon?.resolveForAndroid(themedContext)
                    ?: NativeToolbarResolvedIcon("?")
                text = resolvedIcon.text
                typeface = resolvedIcon.typeface ?: Typeface.DEFAULT
                gravity = Gravity.CENTER
                background = GradientDrawable()
                isAllCaps = false
                includeFontPadding = false
                contentDescription = item.label
                setOnClickListener { onPressItem?.invoke(item) }
                elevation = 0f
                translationZ = 0f
                stateListAnimator = null
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
                button.foreground = resolveDrawableAttr(android.R.attr.selectableItemBackgroundBorderless)
            }
            val params = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
            )
            button.layoutParams = params
            applyButtonLayout(button, appearance = theme?.appearance ?: EditorToolbarAppearance.CUSTOM)
            bindings.add(ButtonBinding(item, button))
            contentRow.addView(button)
        }
    }

    private fun rebuildMentionSuggestions() {
        for (suggestion in mentionSuggestions) {
            val chip = MentionSuggestionChipView(context, suggestion).apply {
                applyTheme(mentionTheme, theme?.appearance ?: EditorToolbarAppearance.CUSTOM)
                setOnClickListener { onSelectMentionSuggestion?.invoke(suggestion) }
            }
            val params = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
            )
            params.marginEnd = dp(8)
            chip.layoutParams = params
            mentionChips.add(chip)
            contentRow.addView(chip)
        }
    }

    private fun compactItems(items: List<NativeToolbarItem>): List<NativeToolbarItem> {
        return items.filterIndexed { index, item ->
            if (item.type != ToolbarItemKind.separator) return@filterIndexed true
            index > 0 &&
                index < items.lastIndex &&
                items[index - 1].type != ToolbarItemKind.separator &&
                items[index + 1].type != ToolbarItemKind.separator
        }
    }

    private fun updateChrome() {
        val appearance = theme?.appearance ?: EditorToolbarAppearance.CUSTOM
        val cornerRadiusPx = (theme?.resolvedBorderRadius() ?: 0f) * density
        val strokeWidthPx = if (appearance == EditorToolbarAppearance.NATIVE) {
            0
        } else {
            ((theme?.resolvedBorderWidth() ?: 1f) * density).roundToInt().coerceAtLeast(1)
        }
        val drawable = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = cornerRadiusPx
            setColor(
                if (appearance == EditorToolbarAppearance.NATIVE) {
                    resolveColorAttr(
                        MaterialR.attr.colorSurfaceContainer,
                        MaterialR.attr.colorSurfaceContainerLow,
                        MaterialR.attr.colorSurface,
                        android.R.attr.colorBackground
                    )
                } else {
                    theme?.backgroundColor ?: resolveColorAttr(
                        MaterialR.attr.colorSurface,
                        android.R.attr.colorBackground
                    )
                }
            )
            if (strokeWidthPx > 0) {
                setStroke(strokeWidthPx, theme?.borderColor ?: resolveSeparatorColor())
            }
        }
        appliedAppearance = appearance
        appliedChromeCornerRadiusPx = cornerRadiusPx
        appliedChromeStrokeWidthPx = strokeWidthPx
        appliedChromeElevationPx = 0f
        appliedChromeColor = if (appearance == EditorToolbarAppearance.NATIVE) {
            resolveColorAttr(
                MaterialR.attr.colorSurfaceContainer,
                MaterialR.attr.colorSurfaceContainerLow,
                MaterialR.attr.colorSurface,
                android.R.attr.colorBackground
            )
        } else {
            theme?.backgroundColor ?: resolveColorAttr(
                MaterialR.attr.colorSurface,
                android.R.attr.colorBackground
            )
        }
        background = drawable
        elevation = appliedChromeElevationPx
        updateContainerLayout(appearance)
        separators.forEach(::configureSeparator)
    }

    private fun updateButtonAppearance(button: AppCompatButton, enabled: Boolean, active: Boolean) {
        val appearance = theme?.appearance ?: EditorToolbarAppearance.CUSTOM
        applyButtonLayout(button, appearance)
        val textColor = if (appearance == EditorToolbarAppearance.NATIVE) {
            when {
                !enabled -> withAlpha(
                    resolveColorAttr(
                        MaterialR.attr.colorOnSurface,
                        android.R.attr.textColorPrimary
                    ),
                    0.38f
                )
                active -> resolveColorAttr(
                    MaterialR.attr.colorOnSecondaryContainer,
                    MaterialR.attr.colorOnPrimaryContainer,
                    MaterialR.attr.colorOnSurface,
                    android.R.attr.textColorPrimary
                )
                else -> resolveColorAttr(
                    MaterialR.attr.colorOnSurfaceVariant,
                    MaterialR.attr.colorOnSurface,
                    android.R.attr.textColorSecondary
                )
            }
        } else {
            when {
                !enabled -> theme?.buttonDisabledColor ?: withAlpha(
                    resolveColorAttr(MaterialR.attr.colorOnSurface, android.R.attr.textColorPrimary),
                    0.38f
                )
                active -> theme?.buttonActiveColor ?: resolveColorAttr(
                    MaterialR.attr.colorPrimary,
                    android.R.attr.textColorPrimary
                )
                else -> theme?.buttonColor ?: resolveColorAttr(
                    MaterialR.attr.colorOnSurfaceVariant,
                    MaterialR.attr.colorOnSurface,
                    android.R.attr.textColorSecondary
                )
            }
        }
        val backgroundColor = if (appearance == EditorToolbarAppearance.NATIVE) {
            if (active) {
                resolveColorAttr(
                    MaterialR.attr.colorSecondaryContainer,
                    MaterialR.attr.colorPrimaryContainer,
                    MaterialR.attr.colorSurfaceVariant,
                    android.R.attr.colorAccent
                )
            } else {
                Color.TRANSPARENT
            }
        } else if (active) {
            theme?.buttonActiveBackgroundColor ?: resolveColorAttr(
                MaterialR.attr.colorPrimaryContainer,
                MaterialR.attr.colorSecondaryContainer,
                MaterialR.attr.colorSurfaceVariant,
                android.R.attr.colorAccent
            )
        } else {
            Color.TRANSPARENT
        }
        val buttonCornerRadiusPx = (theme?.resolvedButtonBorderRadius() ?: 6f) * density
        val drawable = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = buttonCornerRadiusPx
            setColor(backgroundColor)
        }
        appliedButtonCornerRadiusPx = buttonCornerRadiusPx
        buttonBackgroundColors[button] = backgroundColor
        button.background = drawable
        button.setTextColor(textColor)
        button.alpha = if (enabled || appearance == EditorToolbarAppearance.NATIVE) 1f else 0.7f
        button.refreshDrawableState()
        button.invalidate()
    }

    private fun buttonState(
        item: NativeToolbarItem,
        state: NativeToolbarState
    ): Pair<Boolean, Boolean> {
        val isInList = state.nodes["bulletList"] == true || state.nodes["orderedList"] == true
        return when (item.type) {
            ToolbarItemKind.mark -> {
                val mark = item.mark.orEmpty()
                Pair(state.allowedMarks.contains(mark), state.marks[mark] == true)
            }
            ToolbarItemKind.blockquote -> Pair(
                state.commands["toggleBlockquote"] == true,
                state.nodes["blockquote"] == true
            )
            ToolbarItemKind.list -> when (item.listType) {
                ToolbarListType.bulletList -> Pair(
                    state.commands["wrapBulletList"] == true,
                    state.nodes["bulletList"] == true
                )
                ToolbarListType.orderedList -> Pair(
                    state.commands["wrapOrderedList"] == true,
                    state.nodes["orderedList"] == true
                )
                null -> Pair(false, false)
            }
            ToolbarItemKind.command -> when (item.command) {
                ToolbarCommand.indentList -> Pair(isInList && state.commands["indentList"] == true, false)
                ToolbarCommand.outdentList -> Pair(isInList && state.commands["outdentList"] == true, false)
                ToolbarCommand.undo -> Pair(state.canUndo, false)
                ToolbarCommand.redo -> Pair(state.canRedo, false)
                null -> Pair(false, false)
            }
            ToolbarItemKind.node -> {
                val nodeType = item.nodeType.orEmpty()
                Pair(state.insertableNodes.contains(nodeType), state.nodes[nodeType] == true)
            }
            ToolbarItemKind.action -> Pair(!item.isDisabled, item.isActive)
            ToolbarItemKind.separator -> Pair(false, false)
        }
    }

    private fun dp(value: Int): Int = (value * density).toInt()

    private fun resolveColorAttr(vararg attrs: Int): Int =
        resolveColorAttrOrNull(*attrs) ?: Color.TRANSPARENT

    private fun resolveColorAttrOrNull(vararg attrs: Int): Int? {
        val typedValue = TypedValue()
        for (attr in attrs) {
            if (!themedContext.theme.resolveAttribute(attr, typedValue, true)) {
                continue
            }
            if (typedValue.resourceId != 0) {
                AppCompatResources.getColorStateList(themedContext, typedValue.resourceId)
                    ?.defaultColor
                    ?.let { return it }
            } else if (typedValue.type in TypedValue.TYPE_FIRST_COLOR_INT..TypedValue.TYPE_LAST_COLOR_INT) {
                return typedValue.data
            }
        }
        return null
    }

    private fun resolveDrawableAttr(attr: Int) =
        TypedValue().let { typedValue ->
            if (!themedContext.theme.resolveAttribute(attr, typedValue, true) || typedValue.resourceId == 0) {
                null
            } else {
                AppCompatResources.getDrawable(themedContext, typedValue.resourceId)
            }
        }

    private fun resolveSeparatorColor(): Int =
        theme?.separatorColor
            ?: theme?.borderColor
            ?: resolveColorAttr(
                MaterialR.attr.colorOutlineVariant,
                MaterialR.attr.colorOutline,
                android.R.attr.textColorHint
            )

    private fun updateContainerLayout(appearance: EditorToolbarAppearance) {
        val isNative = appearance == EditorToolbarAppearance.NATIVE
        val horizontalPadding = dp(
            if (isNative) {
                NATIVE_CONTAINER_HORIZONTAL_PADDING_DP
            } else {
                12
            }
        )
        val verticalPadding = dp(
            if (isNative) {
                NATIVE_CONTAINER_VERTICAL_PADDING_DP
            } else {
                12
            }
        )
        contentRow.setPadding(horizontalPadding, verticalPadding, horizontalPadding, verticalPadding)
        contentRow.gravity = if (isNative) {
            Gravity.START or Gravity.CENTER_VERTICAL
        } else {
            Gravity.CENTER_VERTICAL
        }
        contentRow.minimumHeight = dp(
            if (isNative) {
                NATIVE_CONTAINER_HEIGHT_DP
            } else {
                0
            }
        )
    }

    private fun applyButtonLayout(button: AppCompatButton, appearance: EditorToolbarAppearance) {
        val isNative = appearance == EditorToolbarAppearance.NATIVE
        val sizePx = dp(if (isNative) NATIVE_BUTTON_SIZE_DP else 36)
        button.textSize = if (isNative) NATIVE_BUTTON_ICON_SIZE_SP else 16f
        button.minWidth = sizePx
        button.minimumWidth = sizePx
        button.minHeight = sizePx
        button.minimumHeight = sizePx
        button.setPadding(
            if (isNative) 0 else dp(10),
            if (isNative) 0 else dp(8),
            if (isNative) 0 else dp(10),
            if (isNative) 0 else dp(8)
        )
        (button.layoutParams as? LinearLayout.LayoutParams)?.let { params ->
            params.marginEnd = dp(if (isNative) NATIVE_ITEM_SPACING_DP else 6)
            button.layoutParams = params
        }
    }

    private fun configureSeparator(separator: View) {
        val appearance = theme?.appearance ?: EditorToolbarAppearance.CUSTOM
        val params = if (appearance == EditorToolbarAppearance.NATIVE) {
            LinearLayout.LayoutParams(dp(1), dp(24)).apply {
                marginStart = dp(NATIVE_GROUP_SPACING_DP / 2)
                marginEnd = dp(NATIVE_GROUP_SPACING_DP / 2)
            }
        } else {
            LinearLayout.LayoutParams(dp(1), dp(22)).apply {
                marginStart = dp(6)
                marginEnd = dp(6)
            }
        }
        separator.layoutParams = params
        separator.setBackgroundColor(
            if (appearance == EditorToolbarAppearance.NATIVE) {
                withAlpha(resolveSeparatorColor(), 0.6f)
            } else {
                resolveSeparatorColor()
            }
        )
    }
}

private fun withAlpha(color: Int, alphaFraction: Float): Int {
    val alpha = (alphaFraction.coerceIn(0f, 1f) * 255).roundToInt()
    return Color.argb(alpha, Color.red(color), Color.green(color), Color.blue(color))
}

internal class MentionSuggestionChipView(
    context: Context,
    val suggestion: NativeMentionSuggestion
) : LinearLayout(context) {
    private val titleView = AppCompatTextView(context)
    private val subtitleView = AppCompatTextView(context)
    private var theme: EditorMentionTheme? = null
    private var toolbarAppearance: EditorToolbarAppearance = EditorToolbarAppearance.CUSTOM
    private val density = resources.displayMetrics.density

    init {
        orientation = VERTICAL
        gravity = Gravity.CENTER_VERTICAL
        minimumHeight = dp(40)
        setPadding(dp(12), dp(8), dp(12), dp(8))
        isClickable = true
        isFocusable = true

        titleView.apply {
            text = suggestion.label
            setTypeface(typeface, Typeface.BOLD)
            textSize = 14f
            includeFontPadding = false
        }
        addView(
            titleView,
            LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT)
        )

        subtitleView.apply {
            text = suggestion.subtitle
            textSize = 12f
            includeFontPadding = false
            visibility = if (suggestion.subtitle.isNullOrBlank()) View.GONE else View.VISIBLE
        }
        addView(
            subtitleView,
            LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT)
        )

        setOnTouchListener { _, motionEvent ->
            when (motionEvent.actionMasked) {
                android.view.MotionEvent.ACTION_DOWN,
                android.view.MotionEvent.ACTION_MOVE -> updateAppearance(highlighted = true)
                android.view.MotionEvent.ACTION_CANCEL,
                android.view.MotionEvent.ACTION_UP -> updateAppearance(highlighted = false)
            }
            false
        }

        applyTheme(null)
    }

    fun applyTheme(
        theme: EditorMentionTheme?,
        toolbarAppearance: EditorToolbarAppearance = EditorToolbarAppearance.CUSTOM
    ) {
        this.theme = theme
        this.toolbarAppearance = toolbarAppearance
        val hasSubtitle = !suggestion.subtitle.isNullOrBlank()
        subtitleView.visibility = if (hasSubtitle) View.VISIBLE else View.GONE
        background = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = (if (toolbarAppearance == EditorToolbarAppearance.NATIVE) 20f else (theme?.borderRadius ?: 12f)) * density
            setColor(
                if (toolbarAppearance == EditorToolbarAppearance.NATIVE) {
                    Color.TRANSPARENT
                } else {
                    theme?.backgroundColor ?: resolveColorAttr(
                        MaterialR.attr.colorSurfaceContainerLow,
                        MaterialR.attr.colorSurfaceVariant,
                        MaterialR.attr.colorSurface,
                        android.R.attr.colorBackground
                    )
                }
            )
            val strokeWidth = if (toolbarAppearance == EditorToolbarAppearance.NATIVE) {
                0
            } else {
                ((theme?.borderWidth ?: 0f) * density).toInt()
            }
            if (strokeWidth > 0) {
                setStroke(strokeWidth, theme?.borderColor ?: Color.TRANSPARENT)
            }
        }
        updateAppearance(highlighted = false)
    }

    private fun updateAppearance(highlighted: Boolean) {
        val backgroundDrawable = background as? GradientDrawable
        val backgroundColor = if (toolbarAppearance == EditorToolbarAppearance.NATIVE) {
            if (highlighted) {
                resolveColorAttr(
                    MaterialR.attr.colorSecondaryContainer,
                    MaterialR.attr.colorPrimaryContainer,
                    MaterialR.attr.colorSurfaceVariant,
                    android.R.attr.colorAccent
                )
            } else {
                Color.TRANSPARENT
            }
        } else if (highlighted) {
            theme?.optionHighlightedBackgroundColor ?: resolveColorAttr(
                MaterialR.attr.colorSecondaryContainer,
                MaterialR.attr.colorPrimaryContainer,
                MaterialR.attr.colorSurfaceVariant,
                android.R.attr.colorAccent
            )
        } else {
            theme?.backgroundColor ?: resolveColorAttr(
                MaterialR.attr.colorSurfaceContainerLow,
                MaterialR.attr.colorSurfaceVariant,
                MaterialR.attr.colorSurface,
                android.R.attr.colorBackground
            )
        }
        backgroundDrawable?.setColor(backgroundColor)
        titleView.setTextColor(
            if (toolbarAppearance == EditorToolbarAppearance.NATIVE && !highlighted) {
                resolveColorAttr(
                    MaterialR.attr.colorOnSurface,
                    android.R.attr.textColorPrimary
                )
            } else if (highlighted) {
                theme?.optionHighlightedTextColor
                    ?: theme?.optionTextColor
                    ?: resolveColorAttr(
                        MaterialR.attr.colorOnSecondaryContainer,
                        MaterialR.attr.colorOnPrimaryContainer,
                        MaterialR.attr.colorOnSurface,
                        android.R.attr.textColorPrimary
                    )
            } else {
                theme?.optionTextColor
                    ?: theme?.textColor
                    ?: resolveColorAttr(
                        MaterialR.attr.colorOnSurface,
                        android.R.attr.textColorPrimary
                    )
            }
        )
        subtitleView.setTextColor(
            if (toolbarAppearance == EditorToolbarAppearance.NATIVE) {
                resolveColorAttr(
                    MaterialR.attr.colorOnSurfaceVariant,
                    android.R.attr.textColorSecondary
                )
            } else {
                theme?.optionSecondaryTextColor ?: resolveColorAttr(
                    MaterialR.attr.colorOnSurfaceVariant,
                    android.R.attr.textColorSecondary
                )
            }
        )
    }

    fun usesNativeAppearanceForTesting(): Boolean =
        toolbarAppearance == EditorToolbarAppearance.NATIVE

    private fun dp(value: Int): Int = (value * density).toInt()

    private fun resolveColorAttr(vararg attrs: Int): Int {
        val typedValue = TypedValue()
        for (attr in attrs) {
            if (!context.theme.resolveAttribute(attr, typedValue, true)) {
                continue
            }
            if (typedValue.resourceId != 0) {
                AppCompatResources.getColorStateList(context, typedValue.resourceId)
                    ?.defaultColor
                    ?.let { return it }
            } else if (typedValue.type in TypedValue.TYPE_FIRST_COLOR_INT..TypedValue.TYPE_LAST_COLOR_INT) {
                return typedValue.data
            }
        }
        return Color.TRANSPARENT
    }
}

private fun JSONObject.optNullableString(key: String): String? {
    if (!has(key) || isNull(key)) return null
    return optString(key).takeUnless { it == "null" }
}
