import CoreText
import UIKit

private let preparedAtomAttribute = NSAttributedString.Key("PREPPreparedAtom")

private extension Int {
    func rendererSaturatingMultiply(_ other: Int) -> Int {
        let result = multipliedReportingOverflow(by: other)
        return result.overflow ? Int.max : result.partialValue
    }
}

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

    /// UIFont/UIColor bridge objects and the resolved heading dictionary are
    /// retained by each cached generation theme. Keep the LRU's accounting
    /// deliberately conservative; paint values themselves are immutable.
    var estimatedRetainedBytes: Int { 3_072 + headings.count * 384 }
}

private struct PreparedAtomAppearance {
    let attributes: [NSAttributedString.Key: Any]
    let background: UIColor
    let borderColor: UIColor?
    let borderWidth: CGFloat
    let radius: CGFloat
    let padding: UIEdgeInsets
}

private struct PreparedAtomSpec {
    let range: NSRange
    let nodeType: String
    let label: String
    let metrics: PreparedAtomMetrics
    let line: CTLine
    let appearance: PreparedAtomAppearance
}

private struct PreparedAttributedBlock {
    let string: NSAttributedString
    let atoms: [PreparedAtomSpec]
    let retainedBytes: Int
}

private struct PreparedListMarker {
    let line: CTLine?
    let label: String
    let width: CGFloat
    let ascent: CGFloat
    let descent: CGFloat
    let checked: Bool
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
        if block.nodeType == "horizontalRule" || block.nodeType == "horizontal_rule" {
            let y = cursorY + theme.ruleMargin
            let rule = CGRect(x: contentX, y: y, width: contentWidth, height: theme.ruleThickness)
            let bounds = CGRect(x: contentX, y: cursorY, width: contentWidth, height: theme.ruleMargin * 2 + theme.ruleThickness)
            let prepared = PreparedProseBlock(
                fragments: [.init(kind: .rule, bounds: rule, color: theme.ruleColor.cgColor, strokeWidth: theme.ruleThickness)],
                bounds: bounds
            )
            return (
                prepared,
                bounds.maxY,
                prepared.estimatedRetainedBytes
            )
        }

        let paint = theme.paint(for: block)
        let marker = block.listContext.map { makeListMarker($0, paint: paint, theme: theme) }
        let listDepth = block.listContext == nil ? 0 : max(0, Int(block.depth) - 1)
        // The marker gutter is an independently measured column. In particular,
        // baseIndentMultiplier == 0 must not permit text to overlap a scaled
        // ordered marker or task box.
        let listBaseIndent = block.listContext == nil ? 0 : max(0, theme.listIndent * theme.listBaseIndentMultiplier)
        let nestedListIndent = block.listContext == nil ? 0 : max(0, theme.listIndent * CGFloat(listDepth))
        let markerGutter = marker.map { max(6, $0.width + 6) } ?? 0
        let listInset = listBaseIndent + nestedListIndent + markerGutter
        let quoteInset = block.inBlockquote ? theme.quoteBorderWidth + theme.quoteMarkerGap + theme.quoteIndent : 0
        let codeInset = block.nodeType == "codeBlock" ? theme.codePaddingHorizontal : 0
        let textX = contentX + listInset + quoteInset + codeInset
        let availableWidth = max(1, contentWidth - listInset - quoteInset - codeInset * 2)
        let attributed = makeAttributedString(block.inlines, paint: paint, theme: theme)
        let typesetter = CTTypesetterCreateWithAttributedString(attributed.string)
        var location = 0
        var fragments: [PreparedProseFragment] = []
        let markerTopInset = marker.map { max(0, $0.ascent - paint.font.ascender) } ?? 0
        var textTop = cursorY + (block.nodeType == "codeBlock" ? theme.codePaddingVertical : 0) + markerTopInset
        let textStart = textTop
        var firstLineBaseline: CGFloat?
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
            if firstLineBaseline == nil { firstLineBaseline = baseline }
            let lineRange = NSRange(location: location, length: count)
            for atom in attributed.atoms where NSIntersectionRange(atom.range, lineRange).length > 0 {
                let offset = CGFloat(CTLineGetOffsetForStringIndex(line, atom.range.location, nil))
                let atomBounds = CGRect(
                    x: textX + offset,
                    y: baseline - atom.metrics.ascent,
                    width: atom.metrics.width,
                    height: atom.metrics.ascent + atom.metrics.descent
                )
                fragments.append(
                    .init(
                        kind: .atom,
                        line: atom.line,
                        origin: CGPoint(x: atomBounds.minX + atom.appearance.padding.left, y: baseline),
                        bounds: atomBounds,
                        color: atom.appearance.background.cgColor,
                        borderColor: atom.appearance.borderColor?.cgColor,
                        cornerRadius: atom.appearance.radius,
                        strokeWidth: atom.appearance.borderWidth,
                        padding: atom.appearance.padding,
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
            firstLineBaseline = textTop + paint.font.ascender
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
            fragments.append(.init(kind: .border, bounds: border, color: theme.quoteBorderColor.cgColor, strokeWidth: theme.quoteBorderWidth))
        }
        if let marker, let baseline = firstLineBaseline {
            let markerX = textX - markerGutter + (markerGutter - marker.width)
            let markerBounds = CGRect(
                x: markerX,
                y: baseline - marker.ascent,
                width: marker.width,
                height: marker.ascent + marker.descent
            )
            fragments.append(.init(kind: .marker, line: marker.line, origin: CGPoint(x: markerX, y: baseline), bounds: markerBounds, color: theme.listMarkerColor.cgColor, label: marker.label, checked: marker.checked))
        }
        let spacing = block.listContext == nil ? paint.spacingAfter : theme.listItemSpacing
        let seedBounds = CGRect(x: contentX, y: cursorY, width: contentWidth, height: max(0, totalEnd - cursorY))
        let bounds = fragments.reduce(seedBounds) { $0.union($1.bounds) }
        let prepared = PreparedProseBlock(fragments: fragments, bounds: bounds)
        return (
            prepared,
            max(totalEnd, bounds.maxY) + spacing,
            256 + attributed.retainedBytes + prepared.estimatedRetainedBytes
        )
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
            case let .atom(nodeType: nodeType, docPos: _, attrsJSON: attrsJSON, label: label):
                if nodeType == "hardBreak" || nodeType == "hard_break" {
                    result.append(NSAttributedString(string: "\n", attributes: baseAttributes(paint)))
                    continue
                }
                let appearance = atomAppearance(nodeType: nodeType, attrsJSON: attrsJSON, paint: paint, theme: theme)
                let displayLabel = label.isEmpty ? " " : label
                let labelLine = CTLineCreateWithAttributedString(
                    NSAttributedString(string: displayLabel, attributes: appearance.attributes)
                )
                var labelAscent: CGFloat = 0
                var labelDescent: CGFloat = 0
                var labelLeading: CGFloat = 0
                let labelWidth = CGFloat(CTLineGetTypographicBounds(labelLine, &labelAscent, &labelDescent, &labelLeading))
                let metrics = PreparedAtomMetrics(
                    width: max(paint.font.lineHeight, labelWidth + appearance.padding.left + appearance.padding.right),
                    ascent: labelAscent + appearance.padding.top,
                    descent: max(labelDescent, 2) + appearance.padding.bottom
                )
                let range = NSRange(location: result.length, length: 1)
                result.append(NSAttributedString(string: "\u{FFFC}", attributes: [
                    kCTRunDelegateAttributeName as NSAttributedString.Key: preparedAtomDelegate(metrics),
                    preparedAtomAttribute: nodeType,
                ]))
                atoms.append(
                    PreparedAtomSpec(
                        range: range,
                        nodeType: nodeType,
                        label: displayLabel,
                        metrics: metrics,
                        line: labelLine,
                        appearance: appearance
                    )
                )
            }
        }
        // NSMutableAttributedString retains UTF-16 storage, attribute runs,
        // atom delegates, and copied labels. This scales with every character,
        // even when a narrow width turns it into many CTLines.
        let stringBytes = result.length.rendererSaturatingMultiply(4)
        let attributeBytes = max(1, result.length).rendererSaturatingMultiply(48)
        let atomBytes = atoms.reduce(0) { partial, atom in
            partial + 256 + atom.label.utf8.count.rendererSaturatingMultiply(2)
        }
        return PreparedAttributedBlock(string: result, atoms: atoms, retainedBytes: 256 + stringBytes + attributeBytes + atomBytes)
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

    private func makeListMarker(
        _ context: ViewerListContext,
        paint: PreparedTextPaint,
        theme: PreparedProseTheme
    ) -> PreparedListMarker {
        let scale = max(0.01, theme.listMarkerScale)
        let font = paint.font.withSize(max(1, paint.font.pointSize * scale))
        let label: String
        if context.kind == "task" {
            label = ""
        } else if context.ordered {
            label = "\(context.index)."
        } else {
            label = "•"
        }
        guard !label.isEmpty else {
            let side = max(font.lineHeight, font.pointSize)
            return PreparedListMarker(line: nil, label: label, width: side, ascent: side * 0.75, descent: side * 0.25, checked: context.checked)
        }
        let line = CTLineCreateWithAttributedString(
            NSAttributedString(
                string: label,
                attributes: [
                    kCTFontAttributeName as NSAttributedString.Key: CTFontCreateWithName(font.fontName as CFString, font.pointSize, nil),
                    kCTForegroundColorAttributeName as NSAttributedString.Key: theme.listMarkerColor.cgColor,
                ]
            )
        )
        var ascent: CGFloat = 0
        var descent: CGFloat = 0
        var leading: CGFloat = 0
        let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
        return PreparedListMarker(line: line, label: label, width: max(1, width), ascent: ascent, descent: descent, checked: context.checked)
    }

    private func atomAppearance(
        nodeType: String,
        attrsJSON: String,
        paint: PreparedTextPaint,
        theme: PreparedProseTheme
    ) -> PreparedAtomAppearance {
        if nodeType == "mention" {
            let values = jsonDictionary(attrsJSON)
            let localMention = (values["mentionTheme"] as? [String: Any]).map(EditorMentionTheme.init(dictionary:))
            let mention = theme.mention?.merged(with: localMention) ?? localMention
            var attributes = baseAttributes(paint)
            if let weight = mention?.fontWeight {
                let font = UIFont.systemFont(ofSize: paint.font.pointSize, weight: EditorTheme.fontWeight(from: weight))
                attributes[kCTFontAttributeName as NSAttributedString.Key] = CTFontCreateWithName(font.fontName as CFString, font.pointSize, nil)
            }
            attributes[kCTForegroundColorAttributeName as NSAttributedString.Key] = (mention?.textColor ?? paint.color).cgColor
            return PreparedAtomAppearance(
                attributes: attributes,
                background: mention?.backgroundColor ?? UIColor.systemBlue.withAlphaComponent(0.12),
                borderColor: mention?.borderColor,
                borderWidth: max(0, mention?.borderWidth ?? 0),
                radius: max(0, mention?.borderRadius ?? 6),
                padding: UIEdgeInsets(top: 4, left: 6, bottom: 4, right: 6)
            )
        }
        return PreparedAtomAppearance(
            attributes: baseAttributes(paint),
            background: UIColor.systemGray5,
            borderColor: nil,
            borderWidth: 0,
            radius: 5,
            padding: UIEdgeInsets(top: 4, left: 6, bottom: 4, right: 6)
        )
    }

    private func jsonDictionary(_ json: String) -> [String: Any] {
        guard let data = json.data(using: .utf8), let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return value
    }
}
