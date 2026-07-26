import CoreText
import UIKit

private let preparedAtomAttribute = NSAttributedString.Key("PREPPreparedAtom")

private final class PreparedAtomMetrics {
    let width: CGFloat
    let ascent: CGFloat
    let descent: CGFloat

    init(width: CGFloat, ascent: CGFloat, descent: CGFloat) {
        self.width = width
        self.ascent = ascent
        self.descent = descent
    }
}

private func preparedAtomDelegate(_ metrics: PreparedAtomMetrics) -> CTRunDelegate {
    var callbacks = CTRunDelegateCallbacks(
        version: kCTRunDelegateVersion1,
        dealloc: { refCon in
            guard let refCon else { return }
            Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).release()
        },
        getAscent: { refCon in
            guard let refCon else { return 0 }
            return Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).takeUnretainedValue().ascent
        },
        getDescent: { refCon in
            guard let refCon else { return 0 }
            return Unmanaged<PreparedAtomMetrics>.fromOpaque(refCon).takeUnretainedValue().descent
        },
        getWidth: { refCon in
            guard let refCon else { return 0 }
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
}

/// Theme parsing is deliberately outside the drawing path. A registry stores
/// this value once per generation and every width-specific artifact reuses it.
struct PreparedProseTheme {
    let text: PreparedTextPaint
    let paragraph: PreparedTextPaint
    let headings: [String: PreparedTextPaint]
    let blockquote: PreparedTextPaint
    let code: PreparedTextPaint
    let contentInsets: UIEdgeInsets
    let listIndent: CGFloat
    let listBaseIndentMultiplier: CGFloat
    let listItemSpacing: CGFloat
    let listMarkerColor: UIColor
    let listMarkerScale: CGFloat
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

    static func resolve(themeJSON: String?) -> PreparedProseTheme {
        let theme = EditorTheme.from(json: themeJSON) ?? EditorTheme(dictionary: [:])
        let baseFont = UIFont.systemFont(ofSize: 17)
        func paint(_ style: EditorTextStyle?, fallback: PreparedTextPaint? = nil) -> PreparedTextPaint {
            let fallback = fallback ?? PreparedTextPaint(font: baseFont, color: .label, lineHeight: nil, spacingAfter: 0)
            guard let style else { return fallback }
            return PreparedTextPaint(
                font: style.resolvedFont(fallback: fallback.font),
                color: style.color ?? fallback.color,
                lineHeight: style.lineHeight ?? fallback.lineHeight,
                spacingAfter: style.spacingAfter ?? fallback.spacingAfter
            )
        }
        let text = paint(theme.text)
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
            headings[name] = paint(defaultHeading.merged(with: theme.headings[name]), fallback: paragraph)
        }
        return PreparedProseTheme(
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
            listItemSpacing: theme.list?.itemSpacing ?? 4,
            listMarkerColor: theme.list?.markerColor ?? text.color,
            listMarkerScale: theme.list?.markerScale ?? 1,
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
        if block.nodeType == "codeBlock" { return code }
        if let heading = headings[block.nodeType] { return heading }
        if block.inBlockquote { return blockquote }
        return paragraph
    }
}

private struct PreparedAtomSpec {
    let range: NSRange
    let nodeType: String
    let label: String
    let metrics: PreparedAtomMetrics
}

private struct PreparedAttributedBlock {
    let string: NSAttributedString
    let atoms: [PreparedAtomSpec]
}

/// Performs the width-dependent, immutable Core Text preparation step.
final class CoreTextProseLayoutEngine {
    func prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        widthPoints: CGFloat,
        displayScale: CGFloat
    ) throws -> PreparedProseLayout {
        guard let widthPixels = ProseLayoutMetrics.widthPixels(widthPoints: widthPoints, scale: displayScale) else {
            return .error(key: key, width: 0, error: .hostContract(message: "A finite positive width is required for prose measurement."))
        }
        let canonicalWidth = ProseLayoutMetrics.canonicalWidth(widthPixels: widthPixels, scale: displayScale)
        if document.isEmpty {
            return PreparedProseLayout(key: key, size: CGSize(width: canonicalWidth, height: 0), blocks: [], retainedBytes: document.retainedBytes)
        }

        let theme = document.preparedTheme ?? PreparedProseTheme.resolve(themeJSON: nil)
        var cursorY = theme.contentInsets.top
        var blocks: [PreparedProseBlock] = []
        var retainedBytes = document.retainedBytes
        for block in document.blocks {
            let prepared = prepareBlock(block, theme: theme, width: canonicalWidth, cursorY: cursorY)
            blocks.append(prepared.block)
            cursorY = prepared.nextY
            retainedBytes += prepared.retainedBytes
        }
        cursorY += theme.contentInsets.bottom
        let pixelHeight = ceil(cursorY * displayScale)
        return PreparedProseLayout(
            key: key,
            size: CGSize(width: canonicalWidth, height: pixelHeight / displayScale),
            blocks: blocks,
            retainedBytes: retainedBytes
        )
    }

    private func prepareBlock(
        _ block: ViewerBlock,
        theme: PreparedProseTheme,
        width: CGFloat,
        cursorY: CGFloat
    ) -> (block: PreparedProseBlock, nextY: CGFloat, retainedBytes: Int) {
        let contentX = theme.contentInsets.left
        let contentWidth = max(1, width - theme.contentInsets.left - theme.contentInsets.right)
        if block.nodeType == "horizontalRule" {
            let y = cursorY + theme.ruleMargin
            let rule = CGRect(x: contentX, y: y, width: contentWidth, height: theme.ruleThickness)
            let bounds = CGRect(x: contentX, y: cursorY, width: contentWidth, height: theme.ruleMargin * 2 + theme.ruleThickness)
            return (
                PreparedProseBlock(fragments: [.init(kind: .rule, bounds: rule, color: theme.ruleColor.cgColor, strokeWidth: theme.ruleThickness)], bounds: bounds),
                bounds.maxY,
                96
            )
        }

        let paint = theme.paint(for: block)
        let listDepth = block.listContext == nil ? 0 : max(0, Int(block.depth) - 1)
        let listInset = block.listContext == nil ? 0 : theme.listIndent * (CGFloat(listDepth) + theme.listBaseIndentMultiplier)
        let quoteInset = block.inBlockquote ? theme.quoteBorderWidth + theme.quoteMarkerGap + theme.quoteIndent : 0
        let codeInset = block.nodeType == "codeBlock" ? theme.codePaddingHorizontal : 0
        let textX = contentX + listInset + quoteInset + codeInset
        let availableWidth = max(1, contentWidth - listInset - quoteInset - codeInset * 2)
        let attributed = makeAttributedString(block.inlines, paint: paint, theme: theme)
        let typesetter = CTTypesetterCreateWithAttributedString(attributed.string)
        var location = 0
        var fragments: [PreparedProseFragment] = []
        var textTop = cursorY + (block.nodeType == "codeBlock" ? theme.codePaddingVertical : 0)
        let textStart = textTop
        while location < attributed.string.length {
            let suggested = CTTypesetterSuggestLineBreak(typesetter, location, availableWidth)
            let count = max(1, suggested)
            let line = CTTypesetterCreateLine(typesetter, CFRange(location: location, length: count))
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            var leading: CGFloat = 0
            let lineWidth = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
            let naturalHeight = ascent + descent + leading
            let lineHeight = max(naturalHeight, paint.lineHeight ?? 0)
            let baseline = textTop + (lineHeight - naturalHeight) / 2 + ascent
            let lineBounds = CGRect(x: textX, y: textTop, width: min(availableWidth, max(0, lineWidth)), height: lineHeight)
            fragments.append(.init(kind: .text, line: line, origin: CGPoint(x: textX, y: baseline), bounds: lineBounds))
            let lineRange = NSRange(location: location, length: count)
            for atom in attributed.atoms where NSIntersectionRange(atom.range, lineRange).length > 0 {
                let offset = CGFloat(CTLineGetOffsetForStringIndex(line, atom.range.location, nil))
                let atomBounds = CGRect(
                    x: textX + offset,
                    y: baseline - atom.metrics.ascent,
                    width: atom.metrics.width,
                    height: atom.metrics.ascent + atom.metrics.descent
                )
                let atomPaint = atomAppearance(nodeType: atom.nodeType, paint: paint, theme: theme)
                let atomLine = CTLineCreateWithAttributedString(NSAttributedString(string: atom.label, attributes: atomPaint.attributes))
                fragments.append(
                    .init(
                        kind: .atom,
                        line: atomLine,
                        origin: CGPoint(x: atomBounds.minX + 6, y: baseline),
                        bounds: atomBounds,
                        color: atomPaint.background.cgColor,
                        cornerRadius: atomPaint.radius,
                        strokeWidth: atomPaint.borderWidth,
                        label: atom.label
                    )
                )
            }
            location += count
            textTop += lineHeight
        }
        if fragments.isEmpty {
            let fallbackHeight = paint.lineHeight ?? paint.font.lineHeight
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: "\u{200B}", attributes: baseAttributes(paint)))
            fragments.append(.init(kind: .text, line: line, origin: CGPoint(x: textX, y: textTop + paint.font.ascender), bounds: CGRect(x: textX, y: textTop, width: 0, height: fallbackHeight)))
            textTop += fallbackHeight
        }
        let textEnd = textTop
        let totalEnd = textEnd + (block.nodeType == "codeBlock" ? theme.codePaddingVertical : 0)
        let blockRect = CGRect(x: contentX, y: cursorY, width: contentWidth, height: max(0, totalEnd - cursorY))
        if block.nodeType == "codeBlock" {
            fragments.insert(.init(kind: .background, bounds: blockRect, color: theme.codeBackground.cgColor, cornerRadius: theme.codeRadius), at: 0)
        }
        if block.inBlockquote {
            let border = CGRect(x: contentX, y: cursorY, width: theme.quoteBorderWidth, height: max(0, totalEnd - cursorY))
            fragments.insert(.init(kind: .border, bounds: border, color: theme.quoteBorderColor.cgColor, strokeWidth: theme.quoteBorderWidth), at: 0)
        }
        if let context = block.listContext {
            let markerWidth = max(theme.listIndent - 6, paint.font.pointSize * 1.25)
            let markerBounds = CGRect(x: contentX + listInset - markerWidth, y: textStart, width: markerWidth, height: paint.font.lineHeight)
            let label: String
            if context.kind == "task" {
                label = ""
            } else if context.ordered {
                label = "\(context.index)."
            } else {
                label = "•"
            }
            let markerLine = label.isEmpty ? nil : CTLineCreateWithAttributedString(
                NSAttributedString(
                    string: label,
                    attributes: [
                        kCTFontAttributeName as NSAttributedString.Key: CTFontCreateWithName(
                            paint.font.withSize(paint.font.pointSize * theme.listMarkerScale).fontName as CFString,
                            paint.font.pointSize * theme.listMarkerScale,
                            nil
                        ),
                        kCTForegroundColorAttributeName as NSAttributedString.Key: theme.listMarkerColor.cgColor,
                    ]
                )
            )
            fragments.append(.init(kind: .marker, line: markerLine, origin: CGPoint(x: markerBounds.minX, y: textStart + paint.font.ascender), bounds: markerBounds, color: theme.listMarkerColor.cgColor, label: label, checked: context.checked))
        }
        let spacing = paint.spacingAfter + (block.listContext == nil ? 0 : theme.listItemSpacing)
        let seedBounds = CGRect(x: contentX, y: cursorY, width: contentWidth, height: max(0, totalEnd - cursorY))
        let bounds = fragments.reduce(seedBounds) { $0.union($1.bounds) }
        return (PreparedProseBlock(fragments: fragments, bounds: bounds), max(totalEnd, bounds.maxY) + spacing, 256 + attributed.string.length * MemoryLayout<UInt16>.size)
    }

    private func makeAttributedString(
        _ inlines: [ViewerInline],
        paint: PreparedTextPaint,
        theme: PreparedProseTheme
    ) -> PreparedAttributedBlock {
        let result = NSMutableAttributedString()
        var atoms: [PreparedAtomSpec] = []
        for inline in inlines {
            switch inline {
            case let .text(text: text, marks: marks):
                result.append(NSAttributedString(string: text, attributes: attributes(for: marks, paint: paint, theme: theme)))
            case let .atom(nodeType: nodeType, docPos: _, attrsJSON: _, label: label):
                if nodeType == "hardBreak" || nodeType == "hard_break" {
                    result.append(NSAttributedString(string: "\n", attributes: baseAttributes(paint)))
                    continue
                }
                let displayLabel = label.isEmpty ? " " : label
                let labelWidth = (displayLabel as NSString).size(withAttributes: [.font: paint.font]).width
                let metrics = PreparedAtomMetrics(width: max(paint.font.lineHeight, labelWidth + 12), ascent: paint.font.ascender + 4, descent: max(-paint.font.descender, 2) + 4)
                let range = NSRange(location: result.length, length: 1)
                result.append(NSAttributedString(string: "\u{FFFC}", attributes: [
                    kCTRunDelegateAttributeName as NSAttributedString.Key: preparedAtomDelegate(metrics),
                    preparedAtomAttribute: nodeType,
                ]))
                atoms.append(PreparedAtomSpec(range: range, nodeType: nodeType, label: displayLabel, metrics: metrics))
            }
        }
        return PreparedAttributedBlock(string: result, atoms: atoms)
    }

    private func attributes(for marks: [FfiViewerMark], paint: PreparedTextPaint, theme: PreparedProseTheme) -> [NSAttributedString.Key: Any] {
        var attributes = baseAttributes(paint)
        var font = paint.font
        var traits = font.fontDescriptor.symbolicTraits
        var underline = false
        var useMonospace = false
        for mark in marks {
            let values = jsonDictionary(mark.attrsJson)
            switch mark.markType {
            case "bold", "strong": traits.insert(.traitBold)
            case "italic", "em": traits.insert(.traitItalic)
            case "underline": underline = true
            case "strike", "strikethrough": attributes[.strikethroughStyle] = NSUnderlineStyle.single.rawValue
            case "code": useMonospace = true
            case "link":
                let link = theme.link
                font = link?.resolvedFont(fallback: font) ?? font
                attributes[.foregroundColor] = link?.color ?? UIColor.systemBlue
                if let background = link?.backgroundColor { attributes[.backgroundColor] = background }
                underline = link?.underline ?? true
            case "textColor", "color", "foregroundColor":
                if let color = EditorTheme.color(from: values["color"] ?? values["textColor"]) { attributes[.foregroundColor] = color }
            case "highlight", "backgroundColor":
                if let color = EditorTheme.color(from: values["color"] ?? values["backgroundColor"]) { attributes[.backgroundColor] = color }
            case "textStyle", "font":
                let markedSize = (values["fontSize"] as? NSNumber).map { CGFloat(truncating: $0) }
                if let family = values["fontFamily"] as? String, let resolved = UIFont(name: family, size: markedSize ?? font.pointSize) { font = resolved }
                if let markedSize { font = font.withSize(markedSize) }
            default: break
            }
        }
        if useMonospace {
            font = UIFont.monospacedSystemFont(ofSize: font.pointSize, weight: traits.contains(.traitBold) ? .bold : .regular)
        } else if let descriptor = font.fontDescriptor.withSymbolicTraits(traits) {
            font = UIFont(descriptor: descriptor, size: font.pointSize)
        }
        attributes[kCTFontAttributeName as NSAttributedString.Key] = CTFontCreateWithName(font.fontName as CFString, font.pointSize, nil)
        if underline { attributes[.underlineStyle] = NSUnderlineStyle.single.rawValue }
        return attributes
    }

    private func baseAttributes(_ paint: PreparedTextPaint) -> [NSAttributedString.Key: Any] {
        [
            kCTFontAttributeName as NSAttributedString.Key: CTFontCreateWithName(paint.font.fontName as CFString, paint.font.pointSize, nil),
            kCTForegroundColorAttributeName as NSAttributedString.Key: paint.color.cgColor,
        ]
    }

    private func atomAppearance(nodeType: String, paint: PreparedTextPaint, theme: PreparedProseTheme) -> (attributes: [NSAttributedString.Key: Any], background: UIColor, radius: CGFloat, borderWidth: CGFloat) {
        if nodeType == "mention" {
            let mention = theme.mention
            var attributes = baseAttributes(paint)
            if let weight = mention?.fontWeight {
                let font = UIFont.systemFont(ofSize: paint.font.pointSize, weight: EditorTheme.fontWeight(from: weight))
                attributes[kCTFontAttributeName as NSAttributedString.Key] = CTFontCreateWithName(font.fontName as CFString, font.pointSize, nil)
            }
            attributes[kCTForegroundColorAttributeName as NSAttributedString.Key] = (mention?.textColor ?? paint.color).cgColor
            return (attributes, mention?.backgroundColor ?? UIColor.systemBlue.withAlphaComponent(0.12), mention?.borderRadius ?? 6, mention?.borderWidth ?? 0)
        }
        return (baseAttributes(paint), UIColor.systemGray5, 5, 0)
    }

    private func jsonDictionary(_ json: String) -> [String: Any] {
        guard let data = json.data(using: .utf8), let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return value
    }
}
