package com.apollohg.editor

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import androidx.appcompat.widget.AppCompatButton
import androidx.appcompat.widget.AppCompatTextView
import androidx.core.view.setPadding
import org.json.JSONObject

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
    private data class ButtonBinding(
        val item: NativeToolbarItem,
        val button: AppCompatButton
    )

    var onPressItem: ((NativeToolbarItem) -> Unit)? = null
    var onSelectMentionSuggestion: ((NativeMentionSuggestion) -> Unit)? = null

    private val contentRow = LinearLayout(context)
    private var theme: EditorToolbarTheme? = null
    private var mentionTheme: EditorMentionTheme? = null
    private var state: NativeToolbarState = NativeToolbarState.empty
    private var items: List<NativeToolbarItem> = NativeToolbarItem.defaults
    private var mentionSuggestions: List<NativeMentionSuggestion> = emptyList()
    private val bindings = mutableListOf<ButtonBinding>()
    private val separators = mutableListOf<View>()
    private val mentionChips = mutableListOf<MentionSuggestionChipView>()
    private val density = resources.displayMetrics.density
    internal var appliedChromeCornerRadiusPx: Float = 0f
        private set
    internal var appliedChromeStrokeWidthPx: Int = 0
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

        contentRow.orientation = LinearLayout.HORIZONTAL
        contentRow.gravity = Gravity.CENTER_VERTICAL
        contentRow.setPadding(dp(12))
        addView(
            contentRow,
            LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT)
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
            separator.setBackgroundColor(theme?.separatorColor ?: Color.parseColor("#E5E5EA"))
        }
        bindings.forEach { binding ->
            updateButtonAppearance(
                binding.button,
                enabled = buttonState(binding.item, state).first,
                active = buttonState(binding.item, state).second
            )
        }
        mentionChips.forEach { chip ->
            chip.applyTheme(mentionTheme)
        }
    }

    fun applyMentionTheme(theme: EditorMentionTheme?) {
        mentionTheme = theme
        mentionChips.forEach { chip ->
            chip.applyTheme(theme)
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
                val params = LinearLayout.LayoutParams(dp(1), dp(22))
                params.marginStart = dp(6)
                params.marginEnd = dp(6)
                separator.layoutParams = params
                separator.setBackgroundColor(theme?.separatorColor ?: Color.parseColor("#E5E5EA"))
                separators.add(separator)
                contentRow.addView(separator)
                continue
            }

            val button = AppCompatButton(context).apply {
                val resolvedIcon = item.icon?.resolveForAndroid(context)
                    ?: NativeToolbarResolvedIcon("?")
                text = resolvedIcon.text
                typeface = resolvedIcon.typeface ?: Typeface.DEFAULT
                textSize = 16f
                minWidth = dp(36)
                minimumWidth = dp(36)
                minHeight = dp(36)
                minimumHeight = dp(36)
                gravity = Gravity.CENTER
                setPadding(dp(10), dp(8), dp(10), dp(8))
                background = GradientDrawable()
                isAllCaps = false
                includeFontPadding = false
                contentDescription = item.label
                setOnClickListener { onPressItem?.invoke(item) }
            }
            val params = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
            )
            params.marginEnd = dp(6)
            button.layoutParams = params
            bindings.add(ButtonBinding(item, button))
            contentRow.addView(button)
        }
    }

    private fun rebuildMentionSuggestions() {
        for (suggestion in mentionSuggestions) {
            val chip = MentionSuggestionChipView(context, suggestion).apply {
                applyTheme(mentionTheme)
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
        val cornerRadiusPx = (theme?.borderRadius ?: 0f) * density
        val strokeWidthPx = ((theme?.borderWidth ?: 1f) * density).toInt().coerceAtLeast(1)
        val drawable = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = cornerRadiusPx
            setColor(theme?.backgroundColor ?: Color.WHITE)
            setStroke(strokeWidthPx, theme?.borderColor ?: Color.parseColor("#E5E5EA"))
        }
        appliedChromeCornerRadiusPx = cornerRadiusPx
        appliedChromeStrokeWidthPx = strokeWidthPx
        background = drawable
        elevation = 0f
    }

    private fun updateButtonAppearance(button: AppCompatButton, enabled: Boolean, active: Boolean) {
        val textColor = when {
            !enabled -> theme?.buttonDisabledColor ?: Color.parseColor("#C7C7CC")
            active -> theme?.buttonActiveColor ?: Color.parseColor("#007AFF")
            else -> theme?.buttonColor ?: Color.parseColor("#666666")
        }
        val backgroundColor = if (active) {
            theme?.buttonActiveBackgroundColor ?: Color.parseColor("#1F007AFF")
        } else {
            Color.TRANSPARENT
        }
        val buttonCornerRadiusPx = (theme?.buttonBorderRadius ?: 6f) * density
        val drawable = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = buttonCornerRadiusPx
            setColor(backgroundColor)
        }
        appliedButtonCornerRadiusPx = buttonCornerRadiusPx
        button.background = drawable
        button.setTextColor(textColor)
        button.alpha = if (enabled) 1f else 0.7f
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
}

private class MentionSuggestionChipView(
    context: Context,
    val suggestion: NativeMentionSuggestion
) : LinearLayout(context) {
    private val titleView = AppCompatTextView(context)
    private val subtitleView = AppCompatTextView(context)
    private var theme: EditorMentionTheme? = null
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

    fun applyTheme(theme: EditorMentionTheme?) {
        this.theme = theme
        val hasSubtitle = !suggestion.subtitle.isNullOrBlank()
        subtitleView.visibility = if (hasSubtitle) View.VISIBLE else View.GONE
        background = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = (theme?.borderRadius ?: 12f) * density
            setColor(theme?.backgroundColor ?: Color.parseColor("#F2F2F7"))
            val strokeWidth = ((theme?.borderWidth ?: 0f) * density).toInt()
            if (strokeWidth > 0) {
                setStroke(strokeWidth, theme?.borderColor ?: Color.TRANSPARENT)
            }
        }
        updateAppearance(highlighted = false)
    }

    private fun updateAppearance(highlighted: Boolean) {
        val backgroundDrawable = background as? GradientDrawable
        val backgroundColor = if (highlighted) {
            theme?.optionHighlightedBackgroundColor ?: Color.parseColor("#1F007AFF")
        } else {
            theme?.backgroundColor ?: Color.parseColor("#F2F2F7")
        }
        backgroundDrawable?.setColor(backgroundColor)
        titleView.setTextColor(
            if (highlighted) {
                theme?.optionHighlightedTextColor ?: theme?.optionTextColor ?: Color.BLACK
            } else {
                theme?.optionTextColor ?: theme?.textColor ?: Color.BLACK
            }
        )
        subtitleView.setTextColor(theme?.optionSecondaryTextColor ?: Color.DKGRAY)
    }

    private fun dp(value: Int): Int = (value * density).toInt()
}

private fun JSONObject.optNullableString(key: String): String? {
    if (!has(key) || isNull(key)) return null
    return optString(key).takeUnless { it == "null" }
}
