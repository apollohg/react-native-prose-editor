import UIKit
import os

extension EditorTextView {
    struct InputTraitState {
        var autoCapitalize: String?
        var autoCorrect: Bool?
        var keyboardType: String?
    }

    struct PendingInputTraitChange {
        var hasAutoCapitalize = false
        var autoCapitalize: String?
        var hasAutoCorrect = false
        var autoCorrect: Bool?
        var hasKeyboardType = false
        var keyboardType: String?

        var isEmpty: Bool {
            !hasAutoCapitalize && !hasAutoCorrect && !hasKeyboardType
        }
    }

    func setAutoCapitalize(_ autoCapitalize: String?) {
        desiredInputTraitState.autoCapitalize = autoCapitalize
        guard prepareForInputTraitChange() else {
            pendingInputTraitChange.hasAutoCapitalize = true
            pendingInputTraitChange.autoCapitalize = autoCapitalize
            scheduleInputTraitChangeRetry()
            return
        }
        applyAutoCapitalize(autoCapitalize)
        appliedInputTraitState.autoCapitalize = autoCapitalize
        clearPendingAutoCapitalize()
    }

    private func applyAutoCapitalize(_ autoCapitalize: String?) {
        switch autoCapitalize {
        case "none":
            autocapitalizationType = .none
        case "words":
            autocapitalizationType = .words
        case "characters":
            autocapitalizationType = .allCharacters
        default:
            autocapitalizationType = .sentences
        }
        if isFirstResponder {
            reloadInputViews()
        }
    }

    func setAutoCorrect(_ autoCorrect: Bool?) {
        desiredInputTraitState.autoCorrect = autoCorrect
        guard prepareForInputTraitChange() else {
            pendingInputTraitChange.hasAutoCorrect = true
            pendingInputTraitChange.autoCorrect = autoCorrect
            scheduleInputTraitChangeRetry()
            return
        }
        applyAutoCorrect(autoCorrect)
        appliedInputTraitState.autoCorrect = autoCorrect
        clearPendingAutoCorrect()
    }

    private func applyAutoCorrect(_ autoCorrect: Bool?) {
        let isEnabled = autoCorrect ?? false
        autocorrectionType = isEnabled ? .yes : .no
        spellCheckingType = isEnabled ? .default : .no
        if isFirstResponder {
            reloadInputViews()
        }
    }

    func setKeyboardType(_ keyboardType: String?) {
        desiredInputTraitState.keyboardType = keyboardType
        guard prepareForInputTraitChange() else {
            pendingInputTraitChange.hasKeyboardType = true
            pendingInputTraitChange.keyboardType = keyboardType
            scheduleInputTraitChangeRetry()
            return
        }
        applyKeyboardType(keyboardType)
        appliedInputTraitState.keyboardType = keyboardType
        clearPendingKeyboardType()
    }

    private func applyKeyboardType(_ keyboardType: String?) {
        self.keyboardType = Self.resolvedKeyboardType(from: keyboardType)
        if isFirstResponder {
            reloadInputViews()
        }
    }

    private func prepareForInputTraitChange() -> Bool {
        guard isFirstResponder, editorId != 0 else { return true }
        return prepareForExternalEditorUpdate()
    }

    private func scheduleInputTraitChangeRetry() {
        guard !pendingInputTraitRetryScheduled else { return }
        pendingInputTraitRetryScheduled = true
        pendingInputTraitRetryGeneration &+= 1
        let retryGeneration = pendingInputTraitRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingInputTraitRetryGeneration else { return }
            self.pendingInputTraitRetryScheduled = false
            let pending = self.pendingInputTraitChange
            self.pendingInputTraitChange = PendingInputTraitChange()
            if pending.hasAutoCapitalize,
               pending.autoCapitalize == self.desiredInputTraitState.autoCapitalize {
                self.setAutoCapitalize(pending.autoCapitalize)
            }
            if pending.hasAutoCorrect,
               pending.autoCorrect == self.desiredInputTraitState.autoCorrect {
                self.setAutoCorrect(pending.autoCorrect)
            }
            if pending.hasKeyboardType,
               pending.keyboardType == self.desiredInputTraitState.keyboardType {
                self.setKeyboardType(pending.keyboardType)
            }
        }
    }

    private func clearPendingAutoCapitalize() {
        pendingInputTraitChange.hasAutoCapitalize = false
        pendingInputTraitChange.autoCapitalize = nil
        cancelPendingInputTraitRetryIfEmpty()
    }

    private func clearPendingAutoCorrect() {
        pendingInputTraitChange.hasAutoCorrect = false
        pendingInputTraitChange.autoCorrect = nil
        cancelPendingInputTraitRetryIfEmpty()
    }

    private func clearPendingKeyboardType() {
        pendingInputTraitChange.hasKeyboardType = false
        pendingInputTraitChange.keyboardType = nil
        cancelPendingInputTraitRetryIfEmpty()
    }

    func clearPendingInputTraitRetry() {
        pendingInputTraitChange = PendingInputTraitChange()
        guard pendingInputTraitRetryScheduled else { return }
        pendingInputTraitRetryScheduled = false
        pendingInputTraitRetryGeneration &+= 1
    }

    private func cancelPendingInputTraitRetryIfEmpty() {
        guard pendingInputTraitRetryScheduled, pendingInputTraitChange.isEmpty else { return }
        pendingInputTraitRetryScheduled = false
        pendingInputTraitRetryGeneration &+= 1
    }

    func replayDesiredInputTraitsIfNeeded() {
        if desiredInputTraitState.autoCapitalize != appliedInputTraitState.autoCapitalize {
            setAutoCapitalize(desiredInputTraitState.autoCapitalize)
        }
        if desiredInputTraitState.autoCorrect != appliedInputTraitState.autoCorrect {
            setAutoCorrect(desiredInputTraitState.autoCorrect)
        }
        if desiredInputTraitState.keyboardType != appliedInputTraitState.keyboardType {
            setKeyboardType(desiredInputTraitState.keyboardType)
        }
    }

    private static func resolvedKeyboardType(from keyboardType: String?) -> UIKeyboardType {
        switch keyboardType {
        case "ascii-capable":
            return .asciiCapable
        case "numbers-and-punctuation":
            return .numbersAndPunctuation
        case "url":
            return .URL
        case "number-pad":
            return .numberPad
        case "phone-pad":
            return .phonePad
        case "name-phone-pad":
            return .namePhonePad
        case "email-address":
            return .emailAddress
        case "decimal-pad", "numeric":
            return .decimalPad
        case "twitter":
            return .twitter
        case "web-search":
            return .webSearch
        case "ascii-capable-number-pad":
            return .asciiCapableNumberPad
        case "visible-password":
            return .asciiCapable
        default:
            return .default
        }
    }

}
