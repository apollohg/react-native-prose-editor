import ExpoModulesCore
import UIKit

extension NativeEditorExpoView {
    func handleEditorDestroyed(_ editorId: UInt64) {
        guard editorId != 0 else { return }
        guard richTextView.editorId == editorId || richTextView.textView.editorId == editorId else {
            NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: self)
            return
        }

        richTextView.textView.discardTransientNativeInputForEditorRebind()
        clearAutonomousErrorBinding()
        NativeEditorViewRegistry.shared.unregister(editorId: editorId, view: self)
        clearPendingEditorUpdateRetries()
        clearPendingViewCommandUpdateRetry()
        clearPendingEditableRetry()
        clearPendingThemeRetry()
        clearPendingAtomsRetry()
        clearPendingAccessoryRetry()
        clearPendingMentionSuggestionRetry()
        lastMentionEventJSON = nil
        _ = richTextView.textView.resignFirstResponder()
        richTextView.editorId = 0
        mentionQueryState = nil
        _ = accessoryToolbar.setMentionSuggestions([])
        toolbarState = .empty
        accessoryToolbar.apply(state: .empty)
        uninstallOutsideTapRecognizer()
        refreshSystemAssistantToolbarIfNeeded()
    }

    func setEditorId(_ id: UInt64) {
        let previousEditorId = richTextView.editorId
        if id != 0 && NativeEditorViewRegistry.shared.isDestroyed(editorId: id) {
            if previousEditorId == id {
                handleEditorDestroyed(id)
            } else {
                setEditorId(0)
            }
            return
        }
        guard previousEditorId != id else {
            if id != 0 {
                if !NativeEditorViewRegistry.shared.register(editorId: id, view: self) {
                    handleEditorDestroyed(id)
                } else {
                    ensureAutonomousErrorBinding()
                }
            }
            return
        }
        if previousEditorId != id {
            richTextView.textView.discardTransientNativeInputForEditorRebind()
            let releasedNativeOwner = ownsNativeBinding(editorId: previousEditorId)
            clearAutonomousErrorBinding()
            NativeEditorViewRegistry.shared.unregister(editorId: previousEditorId, view: self)
            if releasedNativeOwner {
                NativeEditorViewRegistry.shared.nativeOwnerReleased(
                    editorId: previousEditorId,
                    by: self
                )
            }
            clearPendingEditorUpdateRetries()
            clearPendingViewCommandUpdateRetry()
            clearPendingEditableRetry()
            clearPendingThemeRetry()
            clearPendingAtomsRetry()
            clearPendingAccessoryRetry()
            clearPendingMentionSuggestionRetry()
        }
        var initialBindUpdateJSON: String?
        if id != 0 {
            guard NativeEditorViewRegistry.shared.register(editorId: id, view: self) else {
                handleEditorDestroyed(id)
                return
            }
            bindAutonomousError(adapter: EditorV2Registry.adapter(forLegacyId: id), editorId: id)
            initialBindUpdateJSON = EditorV2Registry.adapter(forLegacyId: id)?.initialUpdateJSON()
        }
        // Bind the editor with the same adopted snapshot used for toolbar
        // state. The text view must not perform an independent state read.
        imageLoadOwner.withCurrent {
            richTextView.bindEditor(id: id, initialUpdateJSON: initialBindUpdateJSON)
        }
        if id != 0 {
            richTextView.emitAtomContentWidthIfAvailable(force: true)
        }
        if id != 0 {
            if let initialBindUpdateJSON,
               let state = NativeToolbarState(updateJSON: initialBindUpdateJSON)
            {
                toolbarState = state
                accessoryToolbar.apply(state: state)
            } else {
                toolbarState = .empty
                accessoryToolbar.apply(state: .empty)
            }
        } else {
            toolbarState = .empty
            accessoryToolbar.apply(state: .empty)
        }
        if desiredThemeJSON != lastThemeJSON {
            setThemeJson(desiredThemeJSON)
        }
        if desiredAtomsJSON != lastAtomsJSON {
            setAtomsJson(desiredAtomsJSON)
        }
        refreshSystemAssistantToolbarIfNeeded()
        refreshMentionQuery()
    }

    func ownsNativeBinding(editorId: UInt64) -> Bool {
        guard editorId != 0,
              richTextView.editorId == editorId,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId)
        else { return false }
        let autonomousOwner = autonomousErrorBindingAdapter === adapter
            && autonomousErrorBindingToken.map { adapter.isNativeBindingOwner(token: $0) } == true
        return autonomousOwner || richTextView.textView.ownsNativeBinding(adapter)
    }

    func claimNativeOwnershipAndCatchUp(editorId: UInt64) {
        guard window != nil, richTextView.editorId == editorId else { return }
        ensureAutonomousErrorBinding()
        applyRemoteCommitRefresh()
    }

    func ensureAutonomousErrorBinding() {
        let editorId = richTextView.editorId
        guard editorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId)
        else { return }
        guard autonomousErrorBindingAdapter !== adapter
            || autonomousErrorBindingEditorId != adapter.editorId
            || autonomousErrorBindingToken.map({ adapter.isAutonomousErrorOwner(token: $0) }) != true
        else { return }
        clearAutonomousErrorBinding()
        bindAutonomousError(adapter: adapter, editorId: editorId)
    }

    private func bindAutonomousError(adapter: EditorV2Adapter?, editorId: UInt64) {
        guard let adapter,
              let canonicalEditorId = v2CanonicalUInt64String(adapter.editorId),
              canonicalEditorId == String(editorId),
              !adapter.isDestroyed
        else { return }
        let token = UUID()
        let generation = autonomousErrorBindingGeneration
        autonomousErrorBindingAdapter = adapter
        autonomousErrorBindingEditorId = canonicalEditorId
        autonomousErrorBindingToken = token
        adapter.bindAutonomousErrorOwner(token: token) { [weak self, weak adapter] error in
            let enqueue = {
                guard let self, let adapter else { return }
                self.enqueueAutonomousError(
                    error,
                    from: adapter,
                    editorId: canonicalEditorId,
                    token: token,
                    generation: generation
                )
            }
            if Thread.isMainThread {
                enqueue()
            } else {
                DispatchQueue.main.async(execute: enqueue)
            }
        }
    }

    func clearAutonomousErrorBinding() {
        autonomousErrorBindingGeneration &+= 1
        pendingAutonomousErrors.removeAll()
        if let adapter = autonomousErrorBindingAdapter,
           let token = autonomousErrorBindingToken
        {
            adapter.clearAutonomousErrorOwner(token: token)
        }
        autonomousErrorBindingAdapter = nil
        autonomousErrorBindingEditorId = nil
        autonomousErrorBindingToken = nil
    }

    private func enqueueAutonomousError(
        _ error: FfiError,
        from adapter: EditorV2Adapter,
        editorId: String,
        token: UUID,
        generation: UInt64
    ) {
        guard isLiveAutonomousErrorBinding(
            adapter: adapter,
            editorId: editorId,
            token: token,
            generation: generation
        ) else { return }
        let dispatchId = UUID()
        pendingAutonomousErrors[dispatchId] = PendingAutonomousError(
            adapter: adapter,
            editorId: editorId,
            token: token,
            generation: generation,
            error: error
        )
        DispatchQueue.main.async { [weak self] in
            self?.dispatchAutonomousError(id: dispatchId)
        }
    }

    private func dispatchAutonomousError(id: UUID) {
        // Remove before invoking Expo/test code. Reentrant state changes or a
        // duplicate callback cannot deliver this particular failure twice.
        guard let pending = pendingAutonomousErrors.removeValue(forKey: id),
              isLiveAutonomousErrorBinding(
                adapter: pending.adapter,
                editorId: pending.editorId,
                token: pending.token,
                generation: pending.generation
              )
        else { return }
        let payload = NativeEditorExpoView.autonomousErrorEventPayload(
            editorId: pending.editorId,
            error: pending.error
        )
        if let onEditorErrorForTesting {
            onEditorErrorForTesting(payload)
        } else {
            onEditorError(payload)
        }
    }

    private func isLiveAutonomousErrorBinding(
        adapter: EditorV2Adapter,
        editorId: String,
        token: UUID,
        generation: UInt64
    ) -> Bool {
        guard autonomousErrorBindingGeneration == generation,
              autonomousErrorBindingAdapter === adapter,
              autonomousErrorBindingEditorId == editorId,
              autonomousErrorBindingToken == token,
              adapter.isAutonomousErrorOwner(token: token),
              !adapter.isDestroyed,
              let nativeEditorId = UInt64(editorId),
              !NativeEditorViewRegistry.shared.isDestroyed(editorId: nativeEditorId),
              EditorV2Registry.adapter(forLegacyId: nativeEditorId) === adapter
        else { return false }
        return true
    }

    private static func autonomousErrorEventPayload(editorId: String, error: FfiError) -> [String: Any] {
        let errorRecord: [String: Any] = [
            "domain": error.domain,
            "code": error.code,
            "message": error.message,
            "requestId": error.requestId ?? NSNull(),
            "operationIndex": error.operationIndex ?? NSNull(),
            "limit": error.limit ?? NSNull(),
            "actual": error.actual ?? NSNull(),
            "detailsJson": error.detailsJson ?? NSNull(),
        ]
        let payload: [String: Any] = [
            "editorId": editorId,
            "error": errorRecord,
        ]
        return payload
    }

}
