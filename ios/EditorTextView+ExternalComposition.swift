import UIKit
import os

extension EditorTextView {
    struct ExternalTextCompositionState {
        let sessionId: String
        let startingAuthorizedText: String
        let startingAuthorizedAttributedText: NSAttributedString
        let startingSelectedUtf16Range: NSRange
        var latestText: String
    }

    struct ExternalTextCompositionFinish {
        let resultJSON: String
        let adoptedUpdateJSON: String?
        let succeeded: Bool
    }

    func beginExternalTextComposition(sessionId: String) -> String {
        guard editorId != 0,
              isEditable,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId),
              !adapter.isDestroyed
        else {
            return externalCompositionErrorJSON(
                sessionId: sessionId,
                code: "EXTERNAL_COMPOSITION_UNAVAILABLE",
                message: "The native editor is unavailable or not editable"
            )
        }
        guard externalTextCompositionTerminalResults[sessionId] == nil else {
            return externalCompositionEndedErrorJSON(sessionId: sessionId)
        }
        guard let selectionJSON = adapter.selectionJSON(),
              let selectionData = selectionJSON.data(using: .utf8),
              let selection = try? JSONSerialization.jsonObject(with: selectionData) as? [String: Any],
              selection["type"] as? String == "text"
        else {
            return externalCompositionErrorJSON(
                sessionId: sessionId,
                code: "EXTERNAL_COMPOSITION_SELECTION_INCOMPATIBLE",
                message: "External composition requires a text selection"
            )
        }

        if externalTextComposition != nil {
            let finished = finishExternalTextComposition(
                cause: "consumer",
                finalText: nil,
                cancel: false
            )
            guard finished?.succeeded == true else {
                return externalCompositionErrorJSON(
                    sessionId: sessionId,
                    code: "EXTERNAL_COMPOSITION_COMMIT_FAILED",
                    message: "The previous external text composition could not be committed"
                )
            }
        } else if isComposing, !prepareForExternalEditorUpdate() {
            return externalCompositionErrorJSON(
                sessionId: sessionId,
                code: "EXTERNAL_COMPOSITION_UNAVAILABLE",
                message: "The active input composition could not be committed"
            )
        }
        guard flushPendingNativeTextMutationCommitIfNeeded() else {
            return externalCompositionErrorJSON(
                sessionId: sessionId,
                code: "EXTERNAL_COMPOSITION_UNAVAILABLE",
                message: "Pending native input could not be committed"
            )
        }

        let startingSelection = selectedRange.location == NSNotFound
            ? NSRange(location: 0, length: 0)
            : selectedRange
        captureMarkedTextReplacementRangeIfNeeded()
        externalTextComposition = ExternalTextCompositionState(
            sessionId: sessionId,
            startingAuthorizedText: lastAuthorizedText,
            startingAuthorizedAttributedText: NSAttributedString(
                attributedString: lastAuthorizedAttributedTextStorage
            ),
            startingSelectedUtf16Range: startingSelection,
            latestText: ""
        )
        isComposing = true
        return externalCompositionActiveJSON(sessionId: sessionId)
    }

    func updateExternalTextComposition(sessionId: String, text: String) -> String {
        guard var state = externalTextComposition, state.sessionId == sessionId else {
            return externalCompositionEndedErrorJSON(sessionId: sessionId)
        }
        performTransientTextMutation {
            super.setMarkedText(
                text,
                selectedRange: NSRange(location: (text as NSString).length, length: 0)
            )
        }
        state.latestText = text
        externalTextComposition = state
        refreshMarkedTextCompositionText(fallback: text)
        refreshPlaceholderVisibility()
        return externalCompositionActiveJSON(sessionId: sessionId)
    }

    func commitExternalTextComposition(sessionId: String, finalText: String) -> String {
        guard externalTextComposition?.sessionId == sessionId else {
            return externalTextCompositionTerminalResults[sessionId]
                ?? externalCompositionEndedErrorJSON(sessionId: sessionId)
        }
        return finishExternalTextComposition(
            cause: "consumer",
            finalText: finalText,
            cancel: false
        )?.resultJSON ?? externalCompositionEndedErrorJSON(sessionId: sessionId)
    }

    func cancelExternalTextComposition(sessionId: String, cause: String) -> String {
        guard ["consumer", "documentChange", "lifecycle"].contains(cause) else {
            return externalCompositionErrorJSON(
                sessionId: sessionId,
                code: "EXTERNAL_COMPOSITION_CANCEL_CAUSE_INVALID",
                message: "The external composition cancellation cause is invalid"
            )
        }
        guard externalTextComposition?.sessionId == sessionId else {
            return externalTextCompositionTerminalResults[sessionId]
                ?? externalCompositionEndedErrorJSON(sessionId: sessionId)
        }
        return finishExternalTextComposition(
            cause: cause,
            finalText: nil,
            cancel: true
        )?.resultJSON ?? externalCompositionEndedErrorJSON(sessionId: sessionId)
    }

    func finishExternalTextCompositionBeforeInteractionIfNeeded() -> Bool {
        guard externalTextComposition != nil else { return true }
        return finishExternalTextComposition(
            cause: "interaction",
            finalText: nil,
            cancel: false
        )?.succeeded == true
    }

    func finishExternalTextComposition(
        cause: String,
        finalText: String?,
        cancel: Bool
    ) -> ExternalTextCompositionFinish? {
        guard var state = externalTextComposition else { return nil }
        if let finalText {
            if finalText != state.latestText {
                performTransientTextMutation {
                    super.setMarkedText(
                        finalText,
                        selectedRange: NSRange(
                            location: (finalText as NSString).length,
                            length: 0
                        )
                    )
                }
                refreshMarkedTextCompositionText(fallback: finalText)
            }
            state.latestText = finalText
            externalTextComposition = state
        }

        let replacementRange = trackedMarkedTextReplacementRange()
        var authoritativeUpdateJSON: String?
        if cancel,
           editorId != 0,
           let adapter = EditorV2Registry.adapter(forLegacyId: editorId),
           !adapter.isDestroyed
        {
            authoritativeUpdateJSON = adapter.currentStateJSON()
        }
        externalTextComposition = nil
        finishTransientMarkedTextMutation()

        if cancel {
            restoreAuthorizedExternalComposition(
                state,
                authoritativeUpdateJSON: authoritativeUpdateJSON
            )
            let resultJSON = externalCompositionEndedJSON(
                sessionId: state.sessionId,
                outcome: "cancelled",
                cause: cause,
                text: state.latestText
            )
            emitExternalTextCompositionEnd(sessionId: state.sessionId, resultJSON: resultJSON)
            return ExternalTextCompositionFinish(
                resultJSON: resultJSON,
                adoptedUpdateJSON: nil,
                succeeded: true
            )
        }

        _ = applyAttributedRender(
            state.startingAuthorizedAttributedText,
            usedPatch: false,
            positionCacheUpdate: .scan
        )
        guard let commit = commitMarkedTextWithNativeOutcome(
            state.latestText,
            replacementRange: replacementRange
        ) else {
            let currentUpdateJSON: String?
            if editorId == 0 {
                currentUpdateJSON = nil
            } else {
                currentUpdateJSON = EditorV2Registry.adapter(forLegacyId: editorId)?
                    .recoverNativeRender()
            }
            restoreAuthorizedExternalComposition(
                state,
                authoritativeUpdateJSON: currentUpdateJSON
            )
            let error = externalCompositionErrorPayload(
                code: "EXTERNAL_COMPOSITION_COMMIT_FAILED",
                message: "The external text composition could not be committed"
            )
            let resultJSON = externalCompositionEndedJSON(
                sessionId: state.sessionId,
                outcome: "cancelled",
                cause: cause,
                text: state.latestText,
                error: error
            )
            emitExternalTextCompositionEnd(sessionId: state.sessionId, resultJSON: resultJSON)
            return ExternalTextCompositionFinish(
                resultJSON: resultJSON,
                adoptedUpdateJSON: nil,
                succeeded: false
            )
        }

        let resultJSON = externalCompositionEndedJSON(
            sessionId: state.sessionId,
            outcome: "committed",
            cause: cause,
            text: state.latestText
        )
        emitExternalTextCompositionEnd(sessionId: state.sessionId, resultJSON: resultJSON)
        return ExternalTextCompositionFinish(
            resultJSON: resultJSON,
            adoptedUpdateJSON: commit.updateJSON,
            succeeded: true
        )
    }

    private func restoreAuthorizedExternalComposition(
        _ state: ExternalTextCompositionState,
        authoritativeUpdateJSON: String? = nil
    ) {
        _ = applyAttributedRender(
            state.startingAuthorizedAttributedText,
            usedPatch: false,
            positionCacheUpdate: .scan
        )
        if let authoritativeUpdateJSON {
            _ = applyUpdateJSON(authoritativeUpdateJSON, notifyDelegate: false)
        }
        guard textStorage.string == state.startingAuthorizedText,
              state.startingSelectedUtf16Range.location >= 0,
              state.startingSelectedUtf16Range.length >= 0,
              state.startingSelectedUtf16Range.location
                + state.startingSelectedUtf16Range.length <= textStorage.length
        else {
            return
        }

        logicalSelectionScalarRange = nil
        logicalSelectionUtf16Range = nil
        performTransientTextMutation {
            selectedRange = state.startingSelectedUtf16Range
            noteSelectionDidChange()
        }
        recordAuthorizedSelectionIfPossible()
        refreshTypingAttributesForSelection()
    }

    private func emitExternalTextCompositionEnd(sessionId: String, resultJSON: String) {
        guard externalTextCompositionTerminalResults[sessionId] == nil else { return }
        externalTextCompositionTerminalResults[sessionId] = resultJSON
        editorDelegate?.editorTextView(self, didEndExternalTextComposition: resultJSON)
    }

    private func externalCompositionActiveJSON(sessionId: String) -> String {
        externalCompositionResultJSON([
            "version": 1,
            "type": "active",
            "sessionId": sessionId,
        ])
    }

    private func externalCompositionEndedJSON(
        sessionId: String,
        outcome: String,
        cause: String,
        text: String,
        error: [String: Any]? = nil
    ) -> String {
        var payload: [String: Any] = [
            "version": 1,
            "type": "ended",
            "sessionId": sessionId,
            "outcome": outcome,
            "cause": cause,
            "text": text,
        ]
        if let error {
            payload["error"] = error
        }
        return externalCompositionResultJSON(payload)
    }

    private func externalCompositionEndedErrorJSON(sessionId: String) -> String {
        externalCompositionErrorJSON(
            sessionId: sessionId,
            code: "EXTERNAL_COMPOSITION_ENDED",
            message: "The external text composition session has ended"
        )
    }

    private func externalCompositionErrorJSON(
        sessionId: String?,
        code: String,
        message: String
    ) -> String {
        externalCompositionResultJSON([
            "version": 1,
            "type": "error",
            "sessionId": sessionId.map { $0 as Any } ?? NSNull(),
            "error": externalCompositionErrorPayload(code: code, message: message),
        ])
    }

    private func externalCompositionErrorPayload(code: String, message: String) -> [String: Any] {
        [
            "domain": "lifecycle",
            "code": code,
            "message": message,
            "requestId": NSNull(),
            "operationIndex": NSNull(),
            "limit": NSNull(),
            "actual": NSNull(),
            "details": NSNull(),
        ]
    }

    func externalCompositionResultJSON(_ payload: [String: Any]) -> String {
        guard JSONSerialization.isValidJSONObject(payload),
              let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return #"{"version":1,"type":"error","sessionId":null,"error":{"domain":"lifecycle","code":"EXTERNAL_COMPOSITION_RESULT_INVALID","message":"Could not serialize external composition result","requestId":null,"operationIndex":null,"limit":null,"actual":null,"details":null}}"#
        }
        return json
    }

}
