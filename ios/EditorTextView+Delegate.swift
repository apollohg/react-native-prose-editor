import UIKit

// MARK: - EditorTextViewDelegate

/// Delegate protocol for EditorTextView to communicate state changes
/// back to the hosting view (Fabric component or UIKit container).
protocol EditorTextViewDelegate: AnyObject {
    /// Called when the editor's selection changes.
    /// - Parameters:
    ///   - textView: The editor text view.
    ///   - anchor: Scalar offset of the selection anchor.
    ///   - head: Scalar offset of the selection head.
    func editorTextView(_ textView: EditorTextView, selectionDidChange anchor: UInt32, head: UInt32)

    /// Called when the editor content is updated after a Rust operation.
    /// - Parameters:
    ///   - textView: The editor text view.
    ///   - updateJSON: The full EditorUpdate JSON string from Rust.
    func editorTextView(_ textView: EditorTextView, didReceiveUpdate updateJSON: String)

    func editorTextView(_ textView: EditorTextView, didEndExternalTextComposition resultJSON: String)
}

extension EditorTextViewDelegate {
    func editorTextView(_ textView: EditorTextView, didEndExternalTextComposition resultJSON: String) {}
}

// MARK: - EditorTextView

/// Dedicated `UITextViewDelegate`, because the editor must not be its own.
///
/// Proxy keyboard integrations forward unimplemented selectors via
/// `forwardingTarget(for:)`, so UIKit's private `keyboardInputChangedSelection:`
/// bounces view -> proxy -> view until the stack overflows (APOLLO-REACT-56).
/// A plain NSObject does not respond to those private selectors.
final class EditorTextViewInternalDelegate: NSObject, UITextViewDelegate {
    private weak var editor: EditorTextView?

    init(editor: EditorTextView) {
        self.editor = editor
    }

    func textViewDidChangeSelection(_ textView: UITextView) {
        editor?.textViewDidChangeSelection(textView)
    }

    func textView(
        _ textView: UITextView,
        shouldInteractWith URL: URL,
        in characterRange: NSRange,
        interaction: UITextItemInteraction
    ) -> Bool {
        editor?.textView(textView, shouldInteractWith: URL, in: characterRange, interaction: interaction) ?? false
    }

    func textView(
        _ textView: UITextView,
        shouldInteractWith textAttachment: NSTextAttachment,
        in characterRange: NSRange,
        interaction: UITextItemInteraction
    ) -> Bool {
        editor?.textView(textView, shouldInteractWith: textAttachment, in: characterRange, interaction: interaction)
            ?? false
    }
}
