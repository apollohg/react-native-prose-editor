package com.apollohg.editor

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewOutlineProvider
import android.widget.FrameLayout
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import androidx.appcompat.R as AppCompatR
import androidx.appcompat.view.ContextThemeWrapper
import androidx.appcompat.widget.AppCompatButton
import androidx.appcompat.widget.PopupMenu
import androidx.appcompat.content.res.AppCompatResources
import androidx.core.view.setPadding
import com.google.android.material.R as MaterialR
import com.google.android.material.color.DynamicColors
import kotlin.math.roundToInt

internal class EditorKeyboardToolbarView(context: Context) : FrameLayout(context) {
    private companion object {
        private const val NATIVE_CONTAINER_HEIGHT_DP = 68
        private const val NATIVE_CONTAINER_HORIZONTAL_PADDING_DP = 16
        private const val NATIVE_CONTAINER_VERTICAL_PADDING_DP = 14
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

    private val customThemedContext: Context = DynamicColors.wrapContextIfAvailable(context)
    private val nativeThemedContext: Context = DynamicColors.wrapContextIfAvailable(
        ContextThemeWrapper(context, MaterialR.style.Theme_Material3_DayNight)
    )
    private val rootRow = LinearLayout(context)
    private val startRow = LinearLayout(context)
    private val centerScrollView = HorizontalScrollView(context)
    private val contentRow = LinearLayout(context)
    private val endRow = LinearLayout(context)
    private var theme: EditorToolbarTheme? = null
    private var mentionTheme: EditorMentionTheme? = null
    private var state: NativeToolbarState = NativeToolbarState.empty
    private var items: List<NativeToolbarItem> = NativeToolbarItem.defaults
    private var mentionTrigger: String = "@"
    private var mentionSuggestions: List<NativeMentionSuggestion> = emptyList()
    private var expandedGroupKey: String? = null
    private var rebuildGeneration: Int = 0
    private val bindings = mutableListOf<ButtonBinding>()
    private val separators = mutableListOf<View>()
    private val mentionChips = mutableListOf<MentionSuggestionChipView>()
    private val buttonBackgroundColors = mutableMapOf<AppCompatButton, Int>()
    private val buttonCornerRadii = mutableMapOf<AppCompatButton, Float>()
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
        setBackgroundColor(Color.TRANSPARENT)
        clipToPadding = false
        clipChildren = false

        rootRow.orientation = LinearLayout.HORIZONTAL
        rootRow.gravity = Gravity.CENTER_VERTICAL
        rootRow.clipToPadding = false
        rootRow.clipChildren = false
        startRow.orientation = LinearLayout.HORIZONTAL
        startRow.gravity = Gravity.START or Gravity.CENTER_VERTICAL
        endRow.orientation = LinearLayout.HORIZONTAL
        endRow.gravity = Gravity.END or Gravity.CENTER_VERTICAL
        centerScrollView.isHorizontalScrollBarEnabled = false
        centerScrollView.overScrollMode = OVER_SCROLL_NEVER
        centerScrollView.clipToPadding = false
        centerScrollView.clipChildren = true
        contentRow.orientation = LinearLayout.HORIZONTAL
        contentRow.gravity = Gravity.START or Gravity.CENTER_VERTICAL
        contentRow.clipToPadding = false
        contentRow.clipChildren = false
        centerScrollView.addView(
            contentRow,
            LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT)
        )
        rootRow.addView(
            startRow,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
            )
        )
        rootRow.addView(
            centerScrollView,
            LinearLayout.LayoutParams(
                0,
                LinearLayout.LayoutParams.WRAP_CONTENT,
                1f
            )
        )
        rootRow.addView(
            endRow,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
            )
        )
        addView(
            rootRow,
            LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT)
        )
        rebuildContent(preserveScrollPosition = false)
    }

    fun setItems(items: List<NativeToolbarItem>) {
        this.items = compactItems(items)
        if (expandedGroupKey != null && !containsExpandableGroup(this.items, expandedGroupKey)) {
            expandedGroupKey = null
        }
        if (!isShowingMentionSuggestions) {
            rebuildContent()
        }
    }

    fun applyTheme(theme: EditorToolbarTheme?) {
        val previousAppearance = this.theme?.appearance ?: EditorToolbarAppearance.CUSTOM
        val nextAppearance = theme?.appearance ?: EditorToolbarAppearance.CUSTOM
        this.theme = theme
        if (previousAppearance != nextAppearance) {
            rebuildContent()
            return
        }
        updateChrome()
        separators.forEach { separator ->
            separator.setBackgroundColor(resolveSeparatorColor())
        }
        bindings.forEach { binding ->
            updateButtonAppearance(
                binding,
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
            updateButtonAppearance(binding, enabled, active)
        }
    }

    fun setMentionSuggestions(
        suggestions: List<NativeMentionSuggestion>,
        trigger: String = "@"
    ): Boolean {
        val hadSuggestions = isShowingMentionSuggestions
        mentionTrigger = trigger
        mentionSuggestions = suggestions.take(8)
        if (hadSuggestions && isShowingMentionSuggestions) {
            updateMentionSuggestionsInPlace()
            updateChrome()
            applyState(state)
        } else {
            rebuildContent(preserveScrollPosition = hadSuggestions == isShowingMentionSuggestions)
        }
        return hadSuggestions != isShowingMentionSuggestions
    }

    fun triggerMentionSuggestionTapForTesting(index: Int) {
        mentionChips.getOrNull(index)?.performClick()
    }

    internal fun buttonAtForTesting(index: Int): AppCompatButton? =
        bindings.getOrNull(index)?.button

    internal fun buttonCountForTesting(): Int = bindings.size

    internal fun buttonLabelsForPlacementForTesting(placement: ToolbarItemPlacement): List<String> {
        val row = when (placement) {
            ToolbarItemPlacement.start -> startRow
            ToolbarItemPlacement.scroll -> contentRow
            ToolbarItemPlacement.end -> endRow
        }
        return (0 until row.childCount).mapNotNull { index ->
            (row.getChildAt(index) as? AppCompatButton)?.contentDescription?.toString()
        }
    }

    internal fun buttonBackgroundColorAtForTesting(index: Int): Int? =
        bindings.getOrNull(index)?.button?.let { buttonBackgroundColors[it] }

    internal fun buttonCornerRadiusAtForTesting(index: Int): Float? =
        bindings.getOrNull(index)?.button?.let { buttonCornerRadii[it] }

    internal fun mentionChipAtForTesting(index: Int): MentionSuggestionChipView? =
        mentionChips.getOrNull(index)

    internal fun separatorAtForTesting(index: Int): View? =
        separators.getOrNull(index)

    private fun rebuildContent(preserveScrollPosition: Boolean = true) {
        val targetScrollX = if (preserveScrollPosition) centerScrollView.scrollX else 0
        val generation = ++rebuildGeneration
        bindings.clear()
        buttonBackgroundColors.clear()
        buttonCornerRadii.clear()
        separators.clear()
        mentionChips.clear()
        contentRow.removeAllViews()
        startRow.removeAllViews()
        endRow.removeAllViews()

        if (isShowingMentionSuggestions) {
            val visibleItems = visibleItemsByPlacement()
            rebuildButtonPlacement(visibleItems.start, startRow)
            rebuildMentionSuggestions()
            rebuildButtonPlacement(visibleItems.end, endRow)
        } else {
            rebuildButtons()
        }

        updateChrome()
        applyState(state)
        post {
            if (generation != rebuildGeneration) return@post
            val contentWidth = contentRow.width
            val viewportWidth = (centerScrollView.width - centerScrollView.paddingLeft - centerScrollView.paddingRight).coerceAtLeast(0)
            val maxScrollX = (contentWidth - viewportWidth).coerceAtLeast(0)
            centerScrollView.scrollTo(targetScrollX.coerceIn(0, maxScrollX), 0)
        }
    }

    private fun rebuildButtons() {
        val visibleItems = visibleItemsByPlacement()
        rebuildButtonPlacement(visibleItems.start, startRow)
        rebuildButtonPlacement(visibleItems.scroll, contentRow)
        rebuildButtonPlacement(visibleItems.end, endRow)
    }

    private fun rebuildButtonPlacement(items: List<NativeToolbarItem>, container: LinearLayout) {
        val themedContext = currentThemedContext()
        for (item in items) {
            if (item.type == ToolbarItemKind.separator) {
                val separator = View(context)
                configureSeparator(separator)
                separators.add(separator)
                container.addView(separator)
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
                setOnClickListener {
                    when (item.type) {
                        ToolbarItemKind.group -> handleGroupButtonPress(this, item)
                        else -> {
                            onPressItem?.invoke(item.copy(parentGroupKey = null))
                            if (item.parentGroupKey != null && expandedGroupKey == item.parentGroupKey) {
                                expandedGroupKey = null
                                rebuildContent()
                            }
                        }
                    }
                }
                elevation = 0f
                translationZ = 0f
                stateListAnimator = null
            }
            val params = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT,
                LinearLayout.LayoutParams.WRAP_CONTENT
            )
            button.layoutParams = params
            val binding = ButtonBinding(item, button)
            applyButtonLayout(binding, appearance = theme?.appearance ?: EditorToolbarAppearance.CUSTOM)
            bindings.add(binding)
            container.addView(button)
        }
    }

    private fun rebuildMentionSuggestions() {
        val themedContext = currentThemedContext()
        for (suggestion in mentionSuggestions) {
            val chip = MentionSuggestionChipView(themedContext, suggestion, mentionTrigger).apply {
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

    private fun updateMentionSuggestionsInPlace() {
        val themedContext = currentThemedContext()
        val existingByKey = mentionChips.associateBy { it.suggestion.key }.toMutableMap()
        val nextChips = mentionSuggestions.map { suggestion ->
            existingByKey.remove(suggestion.key)?.also {
                it.updateSuggestion(suggestion, mentionTrigger)
            } ?: MentionSuggestionChipView(themedContext, suggestion, mentionTrigger)
        }

        mentionChips.filterNot { existing -> nextChips.any { it === existing } }.forEach {
            contentRow.removeView(it)
        }
        nextChips.forEachIndexed { index, chip ->
            if (chip.layoutParams !is LinearLayout.LayoutParams) {
                chip.layoutParams = LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
                ).apply {
                    marginEnd = dp(8)
                }
            }
            chip.applyTheme(mentionTheme, theme?.appearance ?: EditorToolbarAppearance.CUSTOM)
            chip.setOnClickListener { onSelectMentionSuggestion?.invoke(chip.suggestion) }
            if (chip.parent == null) {
                contentRow.addView(chip, index)
            } else if (contentRow.indexOfChild(chip) != index) {
                contentRow.removeView(chip)
                contentRow.addView(chip, index)
            }
        }
        mentionChips.clear()
        mentionChips.addAll(nextChips)
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

    private fun visibleItems(): List<NativeToolbarItem> {
        val visible = mutableListOf<NativeToolbarItem>()
        for (item in compactItems(items)) {
            visible += item
            if (
                item.type == ToolbarItemKind.group &&
                    (item.presentation ?: ToolbarGroupPresentation.expand) == ToolbarGroupPresentation.expand &&
                    expandedGroupKey == item.key
            ) {
                visible += item.items.map { child ->
                    child.copy(parentGroupKey = item.key, placement = child.placement ?: item.placement)
                }
            }
        }
        return compactItems(visible)
    }

    private data class VisibleToolbarItemsByPlacement(
        val start: List<NativeToolbarItem>,
        val scroll: List<NativeToolbarItem>,
        val end: List<NativeToolbarItem>
    )

    private fun visibleItemsByPlacement(): VisibleToolbarItemsByPlacement {
        val start = mutableListOf<NativeToolbarItem>()
        val scroll = mutableListOf<NativeToolbarItem>()
        val end = mutableListOf<NativeToolbarItem>()
        for (item in visibleItems()) {
            when (item.placement ?: ToolbarItemPlacement.scroll) {
                ToolbarItemPlacement.start -> start += item
                ToolbarItemPlacement.end -> end += item
                ToolbarItemPlacement.scroll -> scroll += item
            }
        }
        return VisibleToolbarItemsByPlacement(
            start = compactItems(start),
            scroll = compactItems(scroll),
            end = compactItems(end)
        )
    }

    private fun containsExpandableGroup(items: List<NativeToolbarItem>, key: String?): Boolean {
        key ?: return false
        return items.any {
            it.type == ToolbarItemKind.group &&
                it.key == key &&
                (it.presentation ?: ToolbarGroupPresentation.expand) == ToolbarGroupPresentation.expand
        }
    }

    private fun handleGroupButtonPress(anchor: View, item: NativeToolbarItem) {
        if (item.items.isEmpty()) return
        when (item.presentation ?: ToolbarGroupPresentation.expand) {
            ToolbarGroupPresentation.expand -> {
                val key = item.key ?: return
                expandedGroupKey = if (expandedGroupKey == key) null else key
                rebuildContent()
            }
            ToolbarGroupPresentation.menu -> showGroupMenu(anchor, item)
        }
    }

    private fun showGroupMenu(anchor: View, item: NativeToolbarItem) {
        val popupMenu = PopupMenu(currentThemedContext(), anchor)
        item.items.forEachIndexed { index, child ->
            val (enabled, active) = buttonState(child, state)
            val menuItem = popupMenu.menu.add(0, index, index, child.label ?: child.key ?: "Item")
            menuItem.isEnabled = enabled
            menuItem.isCheckable = true
            menuItem.isChecked = active
        }
        popupMenu.setOnMenuItemClickListener { menuItem ->
            val child = item.items.getOrNull(menuItem.itemId) ?: return@setOnMenuItemClickListener false
            onPressItem?.invoke(child)
            true
        }
        popupMenu.show()
    }

    private fun updateChrome() {
        val appearance = theme?.appearance ?: EditorToolbarAppearance.CUSTOM
        val cornerRadiusPx = (theme?.resolvedBorderRadius() ?: 0f) * density
        val strokeWidthPx = if (appearance == EditorToolbarAppearance.NATIVE) {
            0
        } else {
            physicalToolbarBorderWidth(theme?.resolvedBorderWidth() ?: 1f, density)
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
        outlineProvider = ViewOutlineProvider.BACKGROUND
        clipToOutline = cornerRadiusPx > 0f
        elevation = appliedChromeElevationPx
        updateContainerLayout(appearance)
        separators.forEach(::configureSeparator)
    }

    private fun updateButtonAppearance(binding: ButtonBinding, enabled: Boolean, active: Boolean) {
        val button = binding.button
        val buttonStyle = binding.item.buttonStyle
        val appearance = theme?.appearance ?: EditorToolbarAppearance.CUSTOM
        applyButtonLayout(binding, appearance)
        val textColor = when {
            !enabled -> buttonStyle?.disabledColor
                ?: theme?.buttonDisabledColor
                ?: withAlpha(
                    resolveColorAttr(
                        MaterialR.attr.colorOnSurface,
                        android.R.attr.textColorPrimary
                    ),
                    0.38f
                )
            active -> buttonStyle?.activeColor
                ?: theme?.buttonActiveColor
                ?: if (appearance == EditorToolbarAppearance.NATIVE) {
                    resolveColorAttr(
                        MaterialR.attr.colorOnSecondaryContainer,
                        MaterialR.attr.colorOnPrimaryContainer,
                        MaterialR.attr.colorOnSurface,
                        android.R.attr.textColorPrimary
                    )
                } else {
                    resolveColorAttr(
                        AppCompatR.attr.colorPrimary,
                        android.R.attr.textColorPrimary
                    )
                }
            else -> buttonStyle?.color
                ?: theme?.buttonColor
                ?: resolveColorAttr(
                    MaterialR.attr.colorOnSurfaceVariant,
                    MaterialR.attr.colorOnSurface,
                    android.R.attr.textColorSecondary
                )
        }
        val inactiveBackgroundColor = buttonStyle?.backgroundColor
            ?: theme?.buttonBackgroundColor
            ?: Color.TRANSPARENT
        val activeBackgroundColor = buttonStyle?.activeBackgroundColor
            ?: theme?.buttonActiveBackgroundColor
            ?: if (appearance == EditorToolbarAppearance.NATIVE) {
                resolveColorAttr(
                    MaterialR.attr.colorSecondaryContainer,
                    MaterialR.attr.colorPrimaryContainer,
                    MaterialR.attr.colorSurfaceVariant,
                    android.R.attr.colorAccent
                )
            } else {
                resolveColorAttr(
                    MaterialR.attr.colorPrimaryContainer,
                    MaterialR.attr.colorSecondaryContainer,
                    MaterialR.attr.colorSurfaceVariant,
                    android.R.attr.colorAccent
                )
            }
        val disabledBackgroundColor = buttonStyle?.disabledBackgroundColor
            ?: theme?.buttonDisabledBackgroundColor
            ?: if (active) activeBackgroundColor else inactiveBackgroundColor
        val backgroundColor = when {
            !enabled -> disabledBackgroundColor
            active -> activeBackgroundColor
            else -> inactiveBackgroundColor
        }
        val defaultCornerRadius = theme?.resolvedButtonBorderRadius() ?: 6f
        val buttonCornerRadiusDp = (buttonStyle?.borderRadius ?: defaultCornerRadius)
            .takeIf { it.isFinite() }
            ?.coerceAtLeast(0f)
            ?: defaultCornerRadius
        val buttonCornerRadiusPx = buttonCornerRadiusDp * density
        val drawable = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = buttonCornerRadiusPx
            setColor(backgroundColor)
        }
        appliedButtonCornerRadiusPx = buttonCornerRadiusPx
        buttonBackgroundColors[button] = backgroundColor
        buttonCornerRadii[button] = buttonCornerRadiusPx
        button.background = drawable
        ensureButtonRipple(button, buttonCornerRadiusPx)
        button.setTextColor(textColor)
        button.alpha = if (enabled || appearance == EditorToolbarAppearance.NATIVE) 1f else 0.7f
        button.refreshDrawableState()
        button.invalidate()
    }

    private fun buttonState(
        item: NativeToolbarItem,
        state: NativeToolbarState
    ): Pair<Boolean, Boolean> {
        return when (item.type) {
            ToolbarItemKind.mark -> {
                val mark = item.mark.orEmpty()
                Pair(state.allowedMarks.contains(mark), state.marks[mark] == true)
            }
            ToolbarItemKind.heading -> {
                val level = item.headingLevel ?: return Pair(false, false)
                Pair(
                    state.commands["toggleHeading$level"] == true,
                    state.nodes["h$level"] == true
                )
            }
            ToolbarItemKind.blockquote -> Pair(
                state.commands["toggleBlockquote"] == true,
                state.nodes["blockquote"] == true
            )
            ToolbarItemKind.list -> when (item.listType) {
                ToolbarListType.bulletList,
                ToolbarListType.bullet_list -> Pair(
                    state.commands["wrapBulletList"] == true,
                    state.nodes[item.listType.name] == true
                )
                ToolbarListType.orderedList,
                ToolbarListType.ordered_list -> Pair(
                    state.commands["wrapOrderedList"] == true,
                    state.nodes[item.listType.name] == true
                )
                null -> Pair(false, false)
            }
            ToolbarItemKind.command -> when (item.command) {
                ToolbarCommand.indentList -> Pair(state.commands["indentList"] == true, false)
                ToolbarCommand.outdentList -> Pair(state.commands["outdentList"] == true, false)
                ToolbarCommand.undo -> Pair(state.canUndo, false)
                ToolbarCommand.redo -> Pair(state.canRedo, false)
                null -> Pair(false, false)
            }
            ToolbarItemKind.node -> {
                val nodeType = item.nodeType.orEmpty()
                Pair(state.insertableNodes.contains(nodeType), state.nodes[nodeType] == true)
            }
            ToolbarItemKind.action -> Pair(!item.isDisabled, item.isActive)
            ToolbarItemKind.group -> Pair(
                item.items.any { child -> buttonState(child, state).first },
                item.items.any { child -> buttonState(child, state).second } ||
                    (
                        (item.presentation ?: ToolbarGroupPresentation.expand) ==
                            ToolbarGroupPresentation.expand &&
                            expandedGroupKey == item.key
                        )
            )
            ToolbarItemKind.separator -> Pair(false, false)
        }
    }

    private fun dp(value: Int): Int = (value * density).toInt()

    private fun currentThemedContext(): Context =
        if (theme?.appearance == EditorToolbarAppearance.NATIVE) {
            nativeThemedContext
        } else {
            customThemedContext
        }

    private fun resolveColorAttr(vararg attrs: Int): Int =
        resolveColorAttrOrNull(*attrs) ?: Color.TRANSPARENT

    private fun resolveColorAttrOrNull(vararg attrs: Int): Int? {
        val themedContext = currentThemedContext()
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

    private fun ensureButtonRipple(button: AppCompatButton, cornerRadiusPx: Float) {
        val existingMask = (button.foreground as? RippleDrawable)
            ?.findDrawableByLayerId(android.R.id.mask) as? GradientDrawable
        if (existingMask != null) {
            existingMask.cornerRadius = cornerRadiusPx
            return
        }
        val mask = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = cornerRadiusPx
            setColor(Color.WHITE)
        }
        button.foreground = RippleDrawable(
            ColorStateList.valueOf(resolveColorAttr(android.R.attr.colorControlHighlight)),
            null,
            mask
        )
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
        val toolbarHeightDp = resolvedToolbarHeightDp(isNative)
        val buttonSizeDp = resolvedButtonSizeDp(isNative, toolbarHeightDp)
        val horizontalPadding = dp(
            if (isNative) {
                NATIVE_CONTAINER_HORIZONTAL_PADDING_DP
            } else {
                12
            }
        )
        val verticalPadding = dp(resolvedVerticalPaddingDp(isNative, toolbarHeightDp, buttonSizeDp).roundToInt())
        rootRow.setPadding(horizontalPadding, verticalPadding, horizontalPadding, verticalPadding)
        rootRow.minimumHeight = dp(toolbarHeightDp.roundToInt())
        startRow.gravity = Gravity.START or Gravity.CENTER_VERTICAL
        contentRow.gravity = Gravity.START or Gravity.CENTER_VERTICAL
        endRow.gravity = Gravity.END or Gravity.CENTER_VERTICAL
    }

    private fun applyButtonLayout(binding: ButtonBinding, appearance: EditorToolbarAppearance) {
        val button = binding.button
        val isNative = appearance == EditorToolbarAppearance.NATIVE
        val toolbarHeightDp = resolvedToolbarHeightDp(isNative)
        val buttonSizeDp = resolvedButtonSizeDp(isNative, toolbarHeightDp)
        val sizePx = dp(buttonSizeDp.roundToInt())
        val requestedIconSize = binding.item.buttonStyle?.iconSize ?: theme?.buttonIconSize
        button.textSize = requestedIconSize
            ?.takeIf { it.isFinite() && it > 0f }
            ?.coerceAtMost(buttonSizeDp)
            ?: if (isNative) NATIVE_BUTTON_ICON_SIZE_SP else 16f
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

    private fun resolvedToolbarHeightDp(isNative: Boolean): Float =
        if (isNative) NATIVE_CONTAINER_HEIGHT_DP.toFloat() else (theme?.height ?: 60f)

    private fun resolvedButtonSizeDp(isNative: Boolean, toolbarHeightDp: Float): Float {
        if (isNative) return NATIVE_BUTTON_SIZE_DP.toFloat()
        if (theme?.height == null) {
            return 36f
        }
        // Sizing contract shared with src/EditorToolbar.tsx (resolvedButtonHeight)
        // and ios/NativeEditorExpoView.swift (resolvedButtonSize): an explicit
        // theme height caps buttons at NATIVE_BUTTON_SIZE_DP regardless of
        // appearance, not the smaller non-native default.
        return maxOf(1f, minOf(NATIVE_BUTTON_SIZE_DP.toFloat(), toolbarHeightDp - 4f))
    }

    private fun resolvedVerticalPaddingDp(
        isNative: Boolean,
        toolbarHeightDp: Float,
        buttonSizeDp: Float
    ): Float {
        if (isNative) return NATIVE_CONTAINER_VERTICAL_PADDING_DP.toFloat()
        if (theme?.height == null) {
            return 12f
        }
        return maxOf(0f, (toolbarHeightDp - buttonSizeDp) / 2f)
    }
}
