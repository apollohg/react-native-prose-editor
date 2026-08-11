import Foundation

enum EditorOrderedListNumberingScheme: String {
    case decimal
    case lowerAlpha
    case upperAlpha
    case lowerRoman
    case upperRoman
}

struct EditorOrderedListMarkerTheme {
    let schemes: [EditorOrderedListNumberingScheme]
    let suffix: String

    init(dictionary: [String: Any]) {
        let parsed = (dictionary["schemes"] as? [String])?
            .compactMap(EditorOrderedListNumberingScheme.init(rawValue:)) ?? []
        schemes = parsed.isEmpty ? [.decimal] : parsed
        let parsedSuffix = dictionary["suffix"] as? String
        suffix = parsedSuffix == ")" ? ")" : "."
    }
}

enum OrderedListMarkerFormatter {
    static func label(
        index: UInt32,
        nestingDepth: Int,
        theme: EditorOrderedListMarkerTheme?
    ) -> String {
        let resolved = theme ?? EditorOrderedListMarkerTheme(dictionary: [:])
        let depth = max(0, nestingDepth)
        let scheme = resolved.schemes[depth % resolved.schemes.count]
        return formattedIndex(index, scheme: scheme) + resolved.suffix
    }

    private static func formattedIndex(
        _ index: UInt32,
        scheme: EditorOrderedListNumberingScheme
    ) -> String {
        switch scheme {
        case .decimal:
            return String(index)
        case .lowerAlpha:
            return alphabeticIndex(index) ?? String(index)
        case .upperAlpha:
            return alphabeticIndex(index)?.uppercased() ?? String(index)
        case .lowerRoman:
            return romanIndex(index) ?? String(index)
        case .upperRoman:
            return romanIndex(index)?.uppercased() ?? String(index)
        }
    }

    private static func alphabeticIndex(_ index: UInt32) -> String? {
        guard index > 0 else { return nil }

        var value = index
        var characters: [String] = []
        while value > 0 {
            let offset = (value - 1) % 26
            guard let scalar = UnicodeScalar(97 + offset) else { return nil }
            characters.append(String(scalar))
            value = (value - 1) / 26
        }
        return characters.reversed().joined()
    }

    private static func romanIndex(_ index: UInt32) -> String? {
        guard index > 0, index <= 3_999 else { return nil }

        let table: [(value: UInt32, symbol: String)] = [
            (1_000, "m"),
            (900, "cm"),
            (500, "d"),
            (400, "cd"),
            (100, "c"),
            (90, "xc"),
            (50, "l"),
            (40, "xl"),
            (10, "x"),
            (9, "ix"),
            (5, "v"),
            (4, "iv"),
            (1, "i"),
        ]
        var value = index
        var result = ""
        for entry in table {
            while value >= entry.value {
                result += entry.symbol
                value -= entry.value
            }
        }
        return result
    }
}
