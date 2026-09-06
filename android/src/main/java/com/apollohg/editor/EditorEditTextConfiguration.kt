package com.apollohg.editor

import android.os.Build

import android.util.TypedValue

/**
     * Bind this EditText to a Rust editor instance and optionally apply initial content.
     *
     * @param id The editor session public ID.
     * @param initialHTML Optional HTML to set as initial content.
     */
internal fun EditorEditText.bindEditorImpl(id: Long, initialHTML: String? = null, notifyListener: Boolean = true) {
    if (id != 0L && NativeEditorViewRegistry.isDestroyed(id)) {
        discardTransientNativeInputForEditorRebind()
        editorId = 0L
        return
    }
    if (editorId != id) {
        discardTransientNativeInputForEditorRebind()
    }
    editorId = id
    v2Driver = EditorV2Registry.adapterForViewToken(id)
    val driver = v2Driver
    if (driver != null) {
        if (!initialHTML.isNullOrEmpty()) {
            driver.setContentHtml(initialHTML)?.let { applyUpdateJSON(it, notifyListener = false) }
        } else {
            driver.currentStateJson()?.let { applyUpdateJSON(it, notifyListener = notifyListener) }
        }
        return
    }
}

    /**
     * Unbind from the current editor instance.
     */
internal fun EditorEditText.unbindEditorImpl() {
    if (editorId != 0L) {
        discardTransientNativeInputForEditorRebind()
    }
    editorId = 0
    v2Driver = null
}

internal fun EditorEditText.handleEditorDestroyedFromRegistryImpl(destroyedEditorId: Long) {
    if (editorId != destroyedEditorId) return
    unbindEditor()
}

internal fun EditorEditText.setBaseStyleImpl(fontSizePx: Float, textColor: Int, backgroundColor: Int) {
    if (baseFontSize != fontSizePx || baseTextColor != textColor) {
        renderAppearanceRevision += 1L
    }
    baseFontSize = fontSizePx
    baseTextColor = textColor
    baseBackgroundColor = backgroundColor
    setTextSize(TypedValue.COMPLEX_UNIT_PX, fontSizePx)
    setTextColor(textColor)
    setBackgroundColor(theme?.backgroundColor ?: backgroundColor)
}

internal fun EditorEditText.applyThemeImpl(theme: EditorTheme?) {
    this.theme = theme
    renderAppearanceRevision += 1L
    setBackgroundColor(theme?.backgroundColor ?: baseBackgroundColor)
    theme?.styleSheet?.let { background = EditorBoxDrawable(it.box("content").scaled(resources.displayMetrics.density)) }
    applyContentInsets(theme?.contentInsets)
    if (hasLiveEditor()) {
        val previousScrollX = scrollX
        val previousScrollY = scrollY
        val stateJSON = v2Driver?.currentStateJson() ?: return
        reuseImagesDuringThemeUpdate = true
        try { applyUpdateJSON(stateJSON, notifyListener = false) } finally { reuseImagesDuringThemeUpdate = false }
        if (heightBehavior == EditorHeightBehavior.FIXED) {
            preserveScrollPosition(previousScrollX, previousScrollY)
        } else {
            requestLayout()
        }
    } else {
        standaloneRenderJSON?.let { json ->
            reuseImagesDuringThemeUpdate = true
            try {
                val rendered = RenderBridge.buildSpannable(json, baseFontSize, baseTextColor, theme, resources.displayMetrics.density, this, atomRenderConfiguration)
                applyFullRenderPreservingEditorState(rendered)
            } finally { reuseImagesDuringThemeUpdate = false }
        }
        requestLayout()
        invalidate()
    }
}

internal fun EditorEditText.applyAtomRenderConfigurationImpl(configuration: AtomRenderConfiguration?): Boolean {
    if (atomRenderConfiguration == configuration) return true
    val stateJson = if (hasLiveEditor()) {
        v2Driver?.currentStateJson() ?: return false
    } else {
        null
    }
    atomRenderConfiguration = configuration
    renderAppearanceRevision += 1L

    if (stateJson != null) {
        val previousScrollX = scrollX
        val previousScrollY = scrollY
        applyUpdateJSON(stateJson, notifyListener = false)
        if (heightBehavior == EditorHeightBehavior.FIXED) {
            preserveScrollPosition(previousScrollX, previousScrollY)
        } else {
            requestLayout()
        }
        return true
    }

    val renderBlocks = currentRenderBlocksJson ?: return true
    val spannable = RenderBridge.buildSpannableFromBlocks(
        renderBlocks,
        baseFontSize = baseFontSize,
        textColor = baseTextColor,
        theme = theme,
        density = resources.displayMetrics.density,
        hostView = this,
        atomConfiguration = atomRenderConfiguration
    )
    applyFullRenderPreservingEditorState(spannable)
    lastAppliedRenderAppearanceRevision = renderAppearanceRevision
    onSelectionOrContentMayChange?.invoke()
    requestLayout()
    return true
}

internal fun EditorEditText.applyAtomHeightImpl(
    atomKey: String,
    heightPx: Int,
    configuration: AtomRenderConfiguration?
): Boolean {
    atomRenderConfiguration = configuration
    val content = text ?: return false
    val span = content.getSpans(0, content.length, AtomBlockSpan::class.java)
        .firstOrNull { it.atomKey == atomKey }
        ?: return false
    if (span.reservedHeightPx == heightPx) return false

    val start = content.getSpanStart(span)
    val end = content.getSpanEnd(span)
    val flags = content.getSpanFlags(span)
    if (start < 0 || end <= start) return false
    span.reservedHeightPx = heightPx
    content.removeSpan(span)
    content.setSpan(span, start, end, flags)
    atomHeightRenderApplyCount += 1
    requestLayout()
    invalidate()
    onSelectionOrContentMayChange?.invoke()
    return true
}

internal fun EditorEditText.atomHeightRenderApplyCountForTestingImpl(): Int = atomHeightRenderApplyCount

internal fun EditorEditText.setHeightBehaviorImpl(heightBehavior: EditorHeightBehavior) {
    if (this.heightBehavior == heightBehavior) return
    this.heightBehavior = heightBehavior
    isVerticalScrollBarEnabled = heightBehavior == EditorHeightBehavior.FIXED
    overScrollMode = if (heightBehavior == EditorHeightBehavior.FIXED) {
        android.view.View.OVER_SCROLL_IF_CONTENT_SCROLLS
    } else {
        android.view.View.OVER_SCROLL_NEVER
    }
    updateEffectivePadding()
    ensureSelectionVisible()
    requestLayout()
}

internal fun EditorEditText.applyContentInsets(contentInsets: EditorContentInsets?) {
    this.contentInsets = contentInsets
    updateEffectivePadding()
}

internal fun EditorEditText.setViewportBottomInsetPxImpl(bottomInsetPx: Int) {
    val clampedInset = bottomInsetPx.coerceAtLeast(0)
    if (viewportBottomInsetPx == clampedInset) return
    viewportBottomInsetPx = clampedInset
    updateEffectivePadding()
    ensureSelectionVisible()
}

internal fun EditorEditText.setViewportBottomOcclusionTopOnScreenPxImpl(topPx: Int?) {
    if (viewportBottomOcclusionTopOnScreenPx == topPx) return
    viewportBottomOcclusionTopOnScreenPx = topPx
    ensureSelectionVisible()
}

internal fun EditorEditText.updateEffectivePadding() {
    val density = resources.displayMetrics.density
    val left = ((contentInsets?.left ?: 0f) * density).toInt()
    val top = ((contentInsets?.top ?: 0f) * density).toInt()
    val right = ((contentInsets?.right ?: 0f) * density).toInt()
    val bottom = ((contentInsets?.bottom ?: 0f) * density).toInt()

    if (heightBehavior == EditorHeightBehavior.FIXED && theme?.styleSheet == null) {
        setPadding(left, 0, right, 0)
        setCompoundDrawablesRelativeWithIntrinsicBounds(null, null, null, null)
    } else {
        setPadding(left, top, right, bottom)
        setCompoundDrawablesRelativeWithIntrinsicBounds(null, null, null, null)
    }
}

internal fun EditorEditText.setImageResizingEnabledImpl(enabled: Boolean) {
    if (imageResizingEnabled == enabled) return
    imageResizingEnabled = enabled
    if (!enabled) {
        clearExplicitSelectedImageRange()
    } else {
        onSelectionOrContentMayChange?.invoke()
    }
    updateImageSelectionHighlightAppearance()
}

internal fun EditorEditText.setImageLoadingPolicyJsonImpl(policyJson: String?) {
    val nextPolicy = ImageLoadingPolicy.fromJson(policyJson)
    if (nextPolicy == imageLoadingPolicy) return
    imageLoadingPolicy = nextPolicy
    if (!rebuildLatestRenderForImages()) cancelPendingImageLoads()
}

internal fun EditorEditText.currentImageLoadGenerationImpl(): Long = imageLoadGeneration

internal fun EditorEditText.registerImageLoadImpl(handle: RenderImageLoader.LoadHandle) {
    synchronized(imageLoadHandles) {
        imageLoadHandles += handle
    }
    handle.onFinished {
        synchronized(imageLoadHandles) {
            imageLoadHandles.remove(handle)
        }
    }
}

internal fun EditorEditText.activeImageLoadHandleCountForTestingImpl(): Int = synchronized(imageLoadHandles) {
    imageLoadHandles.size
}

internal fun EditorEditText.onImageSpanSizeMayChangeImpl(span: BlockImageSpan) {
    val content = text ?: return
    val start = content.getSpanStart(span)
    val end = content.getSpanEnd(span)
    val flags = content.getSpanFlags(span)
    if (start < 0 || end <= start) return
    content.removeSpan(span)
    content.setSpan(span, start, end, flags)
    requestLayout()
    invalidate()
    onContentSizeMayChange?.invoke()
    onSelectionOrContentMayChange?.invoke()
}

internal fun EditorEditText.initializeEditorView() {
    DecodedBitmapBudget.shared(context)
    // Configure for rich text editing.
    inputType = resolvedInputType()

    // Disable built-in spell checking to avoid conflicts with Rust state.
    // The Rust editor is the source of truth for text content.
    isSaveEnabled = false

    // Watch for unauthorized text mutations (IME, accessibility, etc.)
    // and reconcile back to Rust's authoritative state.
    addTextChangedListener(EditorReconciliationWatcher(this))
    baseBackgroundColor = android.graphics.Color.WHITE
    isVerticalScrollBarEnabled = true
    overScrollMode = android.view.View.OVER_SCROLL_IF_CONTENT_SCROLLS

    // Pin content to top-start to prevent theme-dependent vertical centering.
    gravity = android.view.Gravity.TOP or android.view.Gravity.START

    // Strip the default EditText theme drawable which carries implicit padding.
    // Background color is applied in setBaseStyle() / applyTheme().
    background = null
    linksClickable = false

    isCursorVisible = true
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        textCursorDrawable = EditorGlyphHeightCursorDrawable(this)
    }

    updateEffectivePadding()
}
