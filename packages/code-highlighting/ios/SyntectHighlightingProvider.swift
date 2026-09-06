import ReactNativeProseEditor

final class SyntectHighlightingProvider: NativeCodeHighlightingProvider {
    let id = "syntect"
    let version = 1

    func highlight(text: String, language: String?, theme: String) throws -> [NativeCodeHighlightRange] {
        try highlightCode(text: text, language: language, theme: theme).map {
            NativeCodeHighlightRange(start: Int($0.start), length: Int($0.length), color: $0.color, fontStyle: $0.fontStyle)
        }
    }
}
