import ExpoModulesCore
import UIKit

extension NativeEditorExpoView {
    func setThemeJson(_ themeJson: String?) {
        desiredThemeJSON = themeJson
        guard lastThemeJSON != themeJson else {
            clearPendingThemeRetry()
            return
        }
        let theme = EditorTheme.from(json: themeJson)
        guard imageLoadOwner.withCurrent({ richTextView.applyTheme(theme) }) else {
            scheduleThemeRetry(themeJson)
            return
        }
        lastThemeJSON = themeJson
        clearPendingThemeRetry()
        accessoryToolbar.apply(theme: theme?.toolbar)
        accessoryToolbar.apply(mentionTheme: theme?.mentions ?? addons.mentions?.theme)
        refreshSystemAssistantToolbarIfNeeded()
        if richTextView.textView.isFirstResponder,
           (richTextView.textView.inputAccessoryView === accessoryToolbar || shouldUseSystemAssistantToolbar)
        {
            reloadInputViewsAfterPreparingOrRetry()
        }
    }

    func hasActiveMentionQueryForCurrentAddons() -> Bool {
        guard richTextView.editorId != 0,
              richTextView.textView.isFirstResponder,
              let mentions = addons.mentions
        else {
            return false
        }
        return currentMentionQueryState(trigger: mentions.trigger) != nil
    }

    func setAddonsJson(_ addonsJson: String?) {
        guard lastAddonsJSON != addonsJson else { return }
        lastAddonsJSON = addonsJson
        addons = NativeEditorAddons.from(json: addonsJson)
        accessoryToolbar.apply(mentionTheme: richTextView.textView.theme?.mentions ?? addons.mentions?.theme)
        refreshMentionQuery()
    }

    func setAtomsJson(_ atomsJson: String?) {
        if desiredAtomsJSON != atomsJson {
            clearPendingAtomsRetry()
        }
        desiredAtomsJSON = atomsJson
        guard lastAtomsJSON != atomsJson else {
            clearPendingAtomsRetry()
            return
        }
        let configuration = AtomRenderConfiguration.from(json: atomsJson)
        guard !blockAtomConfigurationApplyForTesting,
              richTextView.applyAtomRenderConfiguration(configuration)
        else {
            scheduleAtomsRetry(atomsJson)
            return
        }
        lastAtomsJSON = atomsJson
        clearPendingAtomsRetry()
    }

    func setImageLoadingPolicyJson(_ json: String?) {
        let policy = ImageLoadingPolicy.from(json: json)
        guard policy != imageLoadOwner.policy else { return }
        imageLoadOwner.updatePolicy(policy)
        richTextView.textView.imageLoadingPolicyDidChange()
        guard richTextView.editorId != 0 else { return }
        imageLoadOwner.withCurrent {
            _ = richTextView.textView.applyUpdateJSON(
                EditorV2Shadow.getCurrentState(id: richTextView.editorId),
                notifyDelegate: false
            )
        }
    }

    func setRemoteSelectionsJson(_ remoteSelectionsJson: String?) {
        guard lastRemoteSelectionsJSON != remoteSelectionsJson else { return }
        lastRemoteSelectionsJSON = remoteSelectionsJson
        richTextView.setRemoteSelections(RemoteSelectionDecoration.from(json: remoteSelectionsJson))
    }

    func setEditable(_ editable: Bool) {
        if !editable, richTextView.textView.isEditable {
            richTextView.textView.cancelExternalTextCompositionForLifecycleIfNeeded()
        }
        if !editable,
           richTextView.textView.isEditable,
           richTextView.editorId != 0,
           !richTextView.textView.prepareForExternalEditorUpdate()
        {
            scheduleEditableRetry(editable)
            return
        }
        pendingEditableRetryValue = nil
        pendingEditableRetryEditorId = nil
        pendingEditableRetryScheduled = false
        richTextView.textView.isEditable = editable
        updateAccessoryToolbarVisibility()
    }

    func setAccessibilityLabel(_ label: String?) {
        richTextView.textView.accessibilityLabel = label
    }

    func setAccessibilityHint(_ hint: String?) {
        richTextView.textView.accessibilityHint = hint
    }

    private func scheduleEditableRetry(_ editable: Bool) {
        pendingEditableRetryValue = editable
        pendingEditableRetryEditorId = richTextView.editorId
        guard !pendingEditableRetryScheduled else { return }
        pendingEditableRetryScheduled = true
        pendingEditableRetryGeneration &+= 1
        let retryGeneration = pendingEditableRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingEditableRetryGeneration else { return }
            guard let pendingEditable = self.pendingEditableRetryValue else {
                self.pendingEditableRetryScheduled = false
                return
            }
            guard self.pendingEditableRetryEditorId == self.richTextView.editorId else {
                self.clearPendingEditableRetry()
                return
            }
            self.pendingEditableRetryValue = nil
            self.pendingEditableRetryEditorId = nil
            self.pendingEditableRetryScheduled = false
            self.setEditable(pendingEditable)
        }
    }

    func setAutoFocus(_ autoFocus: Bool) {
        guard autoFocus, !didApplyAutoFocus else { return }
        didApplyAutoFocus = true
        focus()
    }

    func setAutoCapitalize(_ autoCapitalize: String?) {
        richTextView.textView.setAutoCapitalize(autoCapitalize)
    }

    func setAutoCorrect(_ autoCorrect: Bool?) {
        richTextView.textView.setAutoCorrect(autoCorrect)
    }

    func setKeyboardType(_ keyboardType: String?) {
        richTextView.textView.setKeyboardType(keyboardType)
    }

    func setShowToolbar(_ showToolbar: Bool) {
        showsToolbar = showToolbar
        updateAccessoryToolbarVisibility()
    }

    func setToolbarPlacement(_ toolbarPlacement: String?) {
        self.toolbarPlacement = toolbarPlacement == "inline" ? "inline" : "keyboard"
        updateAccessoryToolbarVisibility()
    }

    func setHeightBehavior(_ rawHeightBehavior: String) {
        let nextBehavior = EditorHeightBehavior(rawValue: rawHeightBehavior) ?? .fixed
        guard nextBehavior != heightBehavior else { return }
        heightBehavior = nextBehavior
        if nextBehavior != .autoGrow {
            cachedAutoGrowContentHeight = 0
            publishAutoGrowStyleHeight(nil)
        }
        richTextView.heightBehavior = nextBehavior
        invalidateIntrinsicContentSize()
        setNeedsLayout()
        if nextBehavior == .autoGrow {
            emitContentHeightIfNeeded(force: true)
            DispatchQueue.main.async { [weak self] in
                guard let self, self.heightBehavior == .autoGrow else { return }
                self.setNeedsLayout()
                self.layoutIfNeeded()
                let measuredHeight = self.richTextView.remeasureAutoGrowHeight()
                guard measuredHeight > 0 else { return }
                self.cachedAutoGrowContentHeight = measuredHeight
                self.invalidateIntrinsicContentSize()
                self.emitContentHeightIfNeeded(force: true, measuredHeight: measuredHeight)
            }
        }
    }

    func setAllowImageResizing(_ allowImageResizing: Bool) {
        richTextView.allowImageResizing = allowImageResizing
    }

    func emitContentHeightIfNeeded(force: Bool = false, measuredHeight: CGFloat? = nil) {
        let originatingEditorId = richTextView.editorId
        guard heightBehavior == .autoGrow else { return }
        let resolvedHeight = measuredHeight
            ?? (cachedAutoGrowContentHeight > 0 ? cachedAutoGrowContentHeight : richTextView.intrinsicContentSize.height)
        let contentHeight = ceil(resolvedHeight)
        guard contentHeight > 0 else { return }
        publishAutoGrowStyleHeight(contentHeight)
        guard force || abs(contentHeight - lastEmittedContentHeight) > 0.5 else { return }
        cachedAutoGrowContentHeight = contentHeight
        lastEmittedContentHeight = contentHeight
        guard let event = Self.editorScopedEventPayload(
            ["contentHeight": contentHeight],
            originatingEditorId: originatingEditorId
        ) else { return }
        onContentHeightChange(event)
    }

    func emitAtomLayout(width: CGFloat) {
        guard let event = Self.editorScopedEventPayload(
            [
                "width": Double(width),
                "positions": richTextView.atomLayoutPositions(),
                "viewport": [
                    "y": Double(richTextView.textView.contentOffset.y),
                    "height": Double(richTextView.textView.bounds.height),
                ],
            ],
            originatingEditorId: richTextView.editorId
        ) else { return }
        onAtomLayout(event)
    }

    static func atomKey(for view: UIView) -> String? {
        let selector = NSSelectorFromString("nativeId")
        let nativeId = view.responds(to: selector) ? view.value(forKey: "nativeId") as? String : nil
        let identifier = nativeId ?? view.accessibilityIdentifier
        let prefix = "prose-atom:"
        guard let identifier,
              identifier.hasPrefix(prefix),
              identifier.count > prefix.count
        else { return nil }
        return String(identifier.dropFirst(prefix.count))
    }

    private func publishAutoGrowStyleHeight(_ height: CGFloat?) {
        if let height {
            if let lastPublishedAutoGrowHeight,
               abs(height - lastPublishedAutoGrowHeight) <= Self.layoutEpsilon
            {
                return
            }
            lastPublishedAutoGrowHeight = height
        } else {
            guard lastPublishedAutoGrowHeight != nil else { return }
            lastPublishedAutoGrowHeight = nil
        }
        let selector = NSSelectorFromString("setStyleSize:height:")
        guard responds(to: selector) else { return }
        _ = perform(selector, with: nil, with: height.map { NSNumber(value: Double($0)) })
    }

    func setToolbarButtonsJson(_ toolbarButtonsJson: String?) {
        guard lastToolbarItemsJSON != toolbarButtonsJson else { return }
        lastToolbarItemsJSON = toolbarButtonsJson
        toolbarItems = NativeToolbarItem.from(json: toolbarButtonsJson)
        accessoryToolbar.setItems(toolbarItems)
        refreshSystemAssistantToolbarIfNeeded()
    }

    func setToolbarFrameJson(_ toolbarFrameJson: String?) {
        guard lastToolbarFrameJSON != toolbarFrameJson else { return }
        lastToolbarFrameJSON = toolbarFrameJson
        guard let toolbarFrameJson,
              let data = toolbarFrameJson.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            toolbarFramesInWindow = []
            return
        }

        if let frameDictionaries = raw["frames"] as? [[String: Any]] {
            toolbarFramesInWindow = frameDictionaries.compactMap(Self.toolbarFrame(from:))
            return
        }

        toolbarFramesInWindow = Self.toolbarFrame(from: raw).map { [$0] } ?? []
    }

    private static func toolbarFrame(from raw: [String: Any]) -> CGRect? {
        guard let x = (raw["x"] as? NSNumber)?.doubleValue,
              let y = (raw["y"] as? NSNumber)?.doubleValue,
              let width = (raw["width"] as? NSNumber)?.doubleValue,
              let height = (raw["height"] as? NSNumber)?.doubleValue,
              width > 0,
              height > 0
        else {
            return nil
        }

        return CGRect(x: x, y: y, width: width, height: height)
    }

}
