package com.apollohg.editor

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.LinearLayout
import androidx.appcompat.widget.AppCompatTextView
import androidx.appcompat.content.res.AppCompatResources
import androidx.core.view.setPadding
import com.google.android.material.R as MaterialR
import org.json.JSONObject
import kotlin.math.roundToInt

internal fun withAlpha(color: Int, alphaFraction: Float): Int {
    val alpha = (alphaFraction.coerceIn(0f, 1f) * 255).roundToInt()
    return Color.argb(alpha, Color.red(color), Color.green(color), Color.blue(color))
}

internal class MentionSuggestionChipView(
    context: Context,
    suggestion: NativeMentionSuggestion,
    trigger: String = "@"
) : LinearLayout(context) {
    private val titleView = AppCompatTextView(context)
    private val subtitleView = AppCompatTextView(context)
    private var theme: EditorMentionTheme? = null
    private var toolbarAppearance: EditorToolbarAppearance = EditorToolbarAppearance.CUSTOM
    private val density = resources.displayMetrics.density
    var suggestion: NativeMentionSuggestion = suggestion
        private set
    private var trigger: String = trigger

    init {
        orientation = VERTICAL
        gravity = Gravity.CENTER_VERTICAL
        minimumHeight = dp(40)
        setPadding(dp(12), dp(8), dp(12), dp(8))
        isClickable = true
        isFocusable = true

        titleView.apply {
            text = suggestion.displayLabel(trigger)
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

    fun updateSuggestion(suggestion: NativeMentionSuggestion, trigger: String) {
        this.suggestion = suggestion
        this.trigger = trigger
        titleView.text = suggestion.displayLabel(trigger)
        subtitleView.text = suggestion.subtitle
        subtitleView.visibility = if (suggestion.subtitle.isNullOrBlank()) View.GONE else View.VISIBLE
    }

    fun applyTheme(
        theme: EditorMentionTheme?,
        toolbarAppearance: EditorToolbarAppearance = EditorToolbarAppearance.CUSTOM
    ) {
        this.theme = theme
        this.toolbarAppearance = toolbarAppearance
        val option = theme?.suggestions?.option
        val hasSubtitle = !suggestion.subtitle.isNullOrBlank()
        subtitleView.visibility = if (hasSubtitle) View.VISIBLE else View.GONE
        background = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = (if (toolbarAppearance == EditorToolbarAppearance.NATIVE) 20f else (option?.borderRadius ?: 12f)) * density
            setColor(
                if (toolbarAppearance == EditorToolbarAppearance.NATIVE) {
                    Color.TRANSPARENT
                } else {
                    option?.backgroundColor ?: resolveColorAttr(
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
                ((option?.borderWidth ?: 0f) * density).toInt()
            }
            if (strokeWidth > 0) {
                setStroke(strokeWidth, option?.borderColor ?: Color.TRANSPARENT)
            }
        }
        updateAppearance(highlighted = false)
    }

    private fun updateAppearance(highlighted: Boolean) {
        val optionTheme = theme?.suggestions?.option
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
            optionTheme?.highlightedBackgroundColor ?: resolveColorAttr(
                MaterialR.attr.colorSecondaryContainer,
                MaterialR.attr.colorPrimaryContainer,
                MaterialR.attr.colorSurfaceVariant,
                android.R.attr.colorAccent
            )
        } else {
            optionTheme?.backgroundColor ?: resolveColorAttr(
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
                optionTheme?.highlightedTextColor
                    ?: optionTheme?.textColor
                    ?: resolveColorAttr(
                        MaterialR.attr.colorOnSecondaryContainer,
                        MaterialR.attr.colorOnPrimaryContainer,
                        MaterialR.attr.colorOnSurface,
                        android.R.attr.textColorPrimary
                    )
            } else {
                optionTheme?.textColor
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
                optionTheme?.secondaryTextColor ?: resolveColorAttr(
                    MaterialR.attr.colorOnSurfaceVariant,
                    android.R.attr.textColorSecondary
                )
            }
        )
    }

    fun usesNativeAppearanceForTesting(): Boolean =
        toolbarAppearance == EditorToolbarAppearance.NATIVE

    fun titleTextForTesting(): String = titleView.text.toString()

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

internal fun NativeMentionSuggestion.displayLabel(trigger: String): String {
    val label = this.label.trim()
    return if (trigger.isNotEmpty() && !label.startsWith(trigger)) "$trigger$label" else label
}
