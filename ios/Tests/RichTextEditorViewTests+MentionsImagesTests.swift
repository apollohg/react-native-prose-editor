import XCTest
import ExpoModulesCore

extension RichTextEditorViewTests {

    func aliceMentionSuggestion() -> NativeMentionSuggestion {
        NativeMentionSuggestion(dictionary: [
            "key": "alice",
            "title": "Alice Chen",
            "subtitle": "Design",
            "label": "@alice",
            "attrs": ["id": "user_alice", "label": "@alice"],
        ])!
    }

    func jsonInt(_ value: Any?) -> Int? {
        if let value = value as? Int {
            return value
        }
        if let value = value as? NSNumber {
            return value.intValue
        }
        return nil
    }

}
