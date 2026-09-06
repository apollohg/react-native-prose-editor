import CoreText
import UIKit

enum PreparedProseInteractionGeometry {
    static func visualOrder(_ left: CGRect, _ right: CGRect) -> Bool {
        if left.minY != right.minY { return left.minY < right.minY }
        if left.minX != right.minX { return left.minX < right.minX }
        if left.maxY != right.maxY { return left.maxY < right.maxY }
        return left.maxX < right.maxX
    }

    static func appendSameLinePiece(
        _ rect: CGRect,
        to rects: inout [CGRect],
        mayMergeWithPrior: Bool
    ) {
        guard mayMergeWithPrior,
              let prior = rects.last,
              prior.minY == rect.minY,
              // A semantic hit region may include only real glyph geometry:
              // overlapping pieces and exact edge contact are contiguous; any
              // positive gap must remain separately hittable/accessibility-visible.
              prior.maxX >= rect.minX
        else {
            rects.append(rect)
            return
        }
        rects[rects.count - 1] = prior.union(rect)
    }
}

final class PreparedAtomMetrics {
    let width: CGFloat
    let ascent: CGFloat
    let descent: CGFloat

    init(width: CGFloat, ascent: CGFloat, descent: CGFloat) {
        self.width = width
        self.ascent = ascent
        self.descent = descent
    }
}

func preparedAtomDelegate(_ metrics: PreparedAtomMetrics) -> CTRunDelegate {
    var callbacks = CTRunDelegateCallbacks(
        version: kCTRunDelegateVersion1,
        dealloc: { refCon in
            Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).release()
        },
        getAscent: { refCon in
            return Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).takeUnretainedValue().ascent
        },
        getDescent: { refCon in
            return Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).takeUnretainedValue().descent
        },
        getWidth: { refCon in
            return Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).takeUnretainedValue().width
        }
    )
    return CTRunDelegateCreate(&callbacks, Unmanaged.passRetained(metrics).toOpaque())!
}

struct PreparedTextPaint {
    let font: UIFont
    let color: UIColor
    let lineHeight: CGFloat?
    let spacingAfter: CGFloat
    var textValues: [String: Any] = [:]
}

struct PreparedViewerAtoms {
    let generation: String
    let revision: String
    let nodeTypes: Set<String>
    let estimatedHeights: [String: Double]
    let measurements: [String: [String: Double]]
    let retainedBytes: Int

    static func resolve(_ themeJSON: String?) -> PreparedViewerAtoms? {
        guard let data = themeJSON?.data(using: .utf8),
              let root = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let value = root["viewerAtoms"] as? [String: Any]
        else { return nil }
        return PreparedViewerAtoms(
            generation: value["generation"] as? String ?? "",
            revision: value["revision"] as? String ?? "",
            nodeTypes: Set(value["nodeTypes"] as? [String] ?? []),
            estimatedHeights: value["estimatedHeights"] as? [String: Double] ?? [:],
            measurements: value["measurements"] as? [String: [String: Double]] ?? [:],
            retainedBytes: data.count * 2
        )
    }

    func height(nodeType: String, docPos: UInt32, width: CGFloat) -> CGFloat {
        if let measurement = measurements[String(docPos)],
           let measuredWidth = measurement["width"], measuredWidth.isFinite,
           abs(measuredWidth - Double(width)) < 0.01,
           let height = measurement["height"], height.isFinite, height >= 0 {
            return CGFloat(height)
        }
        let estimate = estimatedHeights[nodeType] ?? 32
        return estimate.isFinite && estimate >= 0 ? CGFloat(estimate) : 32
    }
}

/// Theme parsing is deliberately outside the drawing path. A registry stores
/// this value once per generation and every width-specific artifact reuses it.
struct PreparedProseTheme {
    var codeHighlighting: NativeCodeHighlightConfiguration? = nil
    let styleSheet: EditorStyleSheet?
    let viewerAtoms: PreparedViewerAtoms?
    let fontScale: CGFloat
    let text: PreparedTextPaint
    let paragraph: PreparedTextPaint
    let headings: [String: PreparedTextPaint]
    let blockquote: PreparedTextPaint
    let code: PreparedTextPaint
    let contentInsets: UIEdgeInsets
    let listIndent: CGFloat
    let listBaseIndentMultiplier: CGFloat
    let listItemSpacing: CGFloat
    let listSpacingAfter: CGFloat
    let listMarkerColor: UIColor
    let listMarkerScale: CGFloat
    let listMarkerGap: CGFloat
    let orderedListMarker: EditorOrderedListMarkerTheme?
    static let defaultListMarkerGap: CGFloat = 6
    let quoteIndent: CGFloat
    let quoteBorderColor: UIColor
    let quoteBorderWidth: CGFloat
    let quoteMarkerGap: CGFloat
    let codeBackground: UIColor
    let codeRadius: CGFloat
    let codePaddingHorizontal: CGFloat
    let codePaddingVertical: CGFloat
    let ruleColor: UIColor
    let ruleThickness: CGFloat
    let ruleMargin: CGFloat
    let link: EditorLinkTheme?
    let mention: EditorMentionTheme?

    static func resolve(
        themeJSON: String?,
        fontScale: CGFloat = 1,
        semanticGeneration: String = "standalone-theme"
    ) -> PreparedProseTheme {
        let theme = EditorTheme.from(json: themeJSON) ?? EditorTheme(dictionary: [:])
        let resolvedScale = fontScale.isFinite && fontScale > 0 ? fontScale : 1
        let baseFont = UIFont.systemFont(ofSize: 17 * resolvedScale)
        func paint(_ style: EditorTextStyle?, fallback: PreparedTextPaint? = nil) -> PreparedTextPaint {
            let fallback = fallback ?? PreparedTextPaint(font: baseFont, color: .label, lineHeight: nil, spacingAfter: 0)
            guard let style else { return fallback }
            let resolvedFont = ViewerFontEnvironment.shared.resolveFont(
                style: style,
                fallback: fallback.font,
                fontScale: resolvedScale,
                semanticGeneration: semanticGeneration
            )
            return PreparedTextPaint(
                font: resolvedFont,
                color: style.color ?? fallback.color,
                lineHeight: style.lineHeight.map { $0 * resolvedScale } ?? fallback.lineHeight,
                spacingAfter: style.spacingAfter.map { $0 * resolvedScale } ?? fallback.spacingAfter
            )
        }
        let text = paint(theme.text)
        if let link = theme.links {
            _ = ViewerFontEnvironment.shared.resolveFont(
                style: EditorTextStyle(
                    fontFamily: link.fontFamily,
                    fontSize: link.fontSize,
                    fontWeight: link.fontWeight,
                    fontStyle: link.fontStyle
                ),
                fallback: text.font,
                fontScale: resolvedScale,
                semanticGeneration: semanticGeneration
            )
        }
        let paragraph = paint(theme.effectiveTextStyle(for: "paragraph"), fallback: text)
        let quote = paint(theme.effectiveTextStyle(for: "paragraph", inBlockquote: true), fallback: paragraph)
        let codeStyle = theme.effectiveTextStyle(for: "codeBlock")
        let codeFallback = PreparedTextPaint(
            font: UIFont.monospacedSystemFont(ofSize: text.font.pointSize, weight: .regular),
            color: text.color,
            lineHeight: text.lineHeight,
            spacingAfter: text.spacingAfter
        )
        var headings: [String: PreparedTextPaint] = [:]
        let defaults: [(String, CGFloat)] = [("h1", 32), ("h2", 28), ("h3", 24), ("h4", 21), ("h5", 19), ("h6", 17)]
        for (name, size) in defaults {
            let defaultHeading = EditorTextStyle(fontSize: size, fontWeight: "700", spacingAfter: 10)
            headings[name] = paint(
                theme.effectiveTextStyle(for: name, defaultStyle: defaultHeading),
                fallback: paragraph
            )
        }
        let listItemSpacing = theme.list?.itemSpacing ?? 4
        return PreparedProseTheme(
            styleSheet: theme.styleSheet,
            viewerAtoms: PreparedViewerAtoms.resolve(themeJSON),
            fontScale: resolvedScale,
            text: text,
            paragraph: paragraph,
            headings: headings,
            blockquote: quote,
            code: paint(codeStyle, fallback: codeFallback),
            contentInsets: UIEdgeInsets(
                top: theme.contentInsets?.top ?? 0,
                left: theme.contentInsets?.left ?? 0,
                bottom: theme.contentInsets?.bottom ?? 0,
                right: theme.contentInsets?.right ?? 0
            ),
            listIndent: theme.list?.indent ?? 28,
            listBaseIndentMultiplier: theme.list?.baseIndentMultiplier ?? 1,
            listItemSpacing: listItemSpacing,
            listSpacingAfter: theme.list?.spacingAfter ?? listItemSpacing,
            listMarkerColor: theme.list?.markerColor ?? text.color,
            listMarkerScale: theme.list?.markerScale ?? 1,
            listMarkerGap: theme.list?.markerGap ?? PreparedProseTheme.defaultListMarkerGap,
            orderedListMarker: theme.list?.orderedMarker,
            quoteIndent: theme.blockquote?.indent ?? 16,
            quoteBorderColor: theme.blockquote?.borderColor ?? UIColor.systemGray3,
            quoteBorderWidth: theme.blockquote?.borderWidth ?? 3,
            quoteMarkerGap: theme.blockquote?.markerGap ?? 10,
            codeBackground: theme.codeBlock?.backgroundColor ?? UIColor.secondarySystemBackground,
            codeRadius: theme.codeBlock?.borderRadius ?? 8,
            codePaddingHorizontal: theme.codeBlock?.paddingHorizontal ?? 12,
            codePaddingVertical: theme.codeBlock?.paddingVertical ?? 8,
            ruleColor: theme.horizontalRule?.color ?? UIColor.separator,
            ruleThickness: theme.horizontalRule?.thickness ?? 1,
            ruleMargin: theme.horizontalRule?.verticalMargin ?? 12,
            link: theme.links,
            mention: theme.mentions
        )
    }

    func paint(for block: ViewerBlock) -> PreparedTextPaint {
        if let styleSheet {
            let ancestors = block.styleAncestors.map(\.nodeType)
            let style = styleSheet.textStyle(block.nodeType, ancestors: ancestors)
            return PreparedTextPaint(
                font: ViewerFontEnvironment.shared.resolveFont(style: style, fallback: text.font, fontScale: fontScale, semanticGeneration: "editor-stylesheet"),
                color: style.color ?? text.color,
                lineHeight: style.lineHeight.map { $0 * fontScale },
                spacingAfter: 0,
                textValues: styleSheet.textValues(block.nodeType, ancestors: ancestors)
            )
        }
        if block.nodeType == "codeBlock" { return code }
        if let heading = headings[block.nodeType] { return heading }
        if block.inBlockquote { return blockquote }
        return paragraph
    }

    /// UIFont/UIColor bridge objects and the resolved heading dictionary are
    /// retained by each cached generation theme. Keep the LRU's accounting
    /// deliberately conservative; paint values themselves are immutable.
    var estimatedRetainedBytes: Int { 3_072 + headings.count * 384 + (viewerAtoms?.retainedBytes ?? 0) }
}

struct PreparedAtomAppearance {
    var styleBox: EditorStyleBox? = nil
    let attributes: [NSAttributedString.Key: Any]
    let background: UIColor
    let borderColor: UIColor?
    let borderWidth: CGFloat
    let radius: CGFloat
    let padding: UIEdgeInsets
}

struct PreparedAtomSpec {
    let range: NSRange
    let nodeType: String
    let docPos: UInt32
    let label: String
    let metrics: PreparedAtomMetrics
    let line: CTLine
    let appearance: PreparedAtomAppearance
}

struct PreparedAttributedBlock {
    let string: NSAttributedString
    let atoms: [PreparedAtomSpec]
    let semanticRanges: [PreparedSemanticRange]
    let accessibilityRanges: [PreparedAccessibilityRange]
    let retainedBytes: Int
}

enum PreparedSemanticRange {
    case link(range: NSRange, href: String, text: String)
    case mention(range: NSRange, docPos: UInt32, label: String, attrsJSON: String)

    var range: NSRange {
        switch self {
        case let .link(range, _, _), let .mention(range, _, _, _): range
        }
    }
}

struct PreparedAccessibilityRange {
    enum Role: Equatable {
        case text
        case link(semanticIndex: Int)
        case mention(semanticIndex: Int)
    }

    let range: NSRange
    let label: String
    let role: Role
}

struct PreparedListMarker {
    let line: CTLine?
    let label: String
    let width: CGFloat
    let ascent: CGFloat
    let descent: CGFloat
    let checked: Bool
}

/// Performs the width-dependent, immutable Core Text preparation step.
