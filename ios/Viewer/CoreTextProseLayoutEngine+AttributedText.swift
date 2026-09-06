import CoreText
import UIKit

private extension Int {
    func rendererSaturatingMultiply(_ other: Int) -> Int {
        let result = multipliedReportingOverflow(by: other)
        return result.overflow ? Int.max : result.partialValue
    }
}

extension CoreTextProseLayoutEngine {
    func makeAttributedString(
        _ inlines: [ViewerInline],
        paint: PreparedTextPaint,
        theme: PreparedProseTheme,
        warningSemanticGeneration: String
    ) -> PreparedAttributedBlock {
        let result = NSMutableAttributedString()
        var atoms: [PreparedAtomSpec] = []
        var semanticRanges: [PreparedSemanticRange] = []
        var accessibilityRanges: [PreparedAccessibilityRange] = []

        func appendAccessibilityRange(_ range: NSRange, label: String, role: PreparedAccessibilityRange.Role) {
            guard range.length > 0 else { return }
            if let previous = accessibilityRanges.last,
               previous.role == role,
               previous.range.upperBound == range.location
            {
                accessibilityRanges[accessibilityRanges.count - 1] = PreparedAccessibilityRange(
                    range: NSRange(location: previous.range.location, length: previous.range.length + range.length),
                    label: previous.label + label,
                    role: role
                )
            } else {
                accessibilityRanges.append(PreparedAccessibilityRange(range: range, label: label, role: role))
            }
        }

        for inline in inlines {
            switch inline {
            case let .text(text: text, marks: marks):
                let start = result.length
                result.append(NSAttributedString(string: text, attributes: attributes(for: marks, paint: paint, theme: theme, warningSemanticGeneration: warningSemanticGeneration)))
                let range = NSRange(location: start, length: (text as NSString).length)
                if let href = href(in: marks), !text.isEmpty {
                    let semanticIndex: Int
                    if case let .link(previous, previousHref, previousText)? = semanticRanges.last,
                       previousHref == href, previous.upperBound == range.location {
                        semanticRanges[semanticRanges.count - 1] = .link(range: NSRange(location: previous.location, length: previous.length + range.length), href: href, text: previousText + text)
                        semanticIndex = semanticRanges.count - 1
                    } else {
                        semanticRanges.append(.link(range: range, href: href, text: text))
                        semanticIndex = semanticRanges.count - 1
                    }
                    appendAccessibilityRange(range, label: text, role: .link(semanticIndex: semanticIndex))
                } else {
                    appendAccessibilityRange(range, label: text, role: .text)
                }
            case let .atom(nodeType: nodeType, docPos: docPos, attrsJSON: attrsJSON, label: label):
                if nodeType == "hardBreak" || nodeType == "hard_break" {
                    let range = NSRange(location: result.length, length: 1)
                    result.append(NSAttributedString(string: "\n", attributes: baseAttributes(paint)))
                    appendAccessibilityRange(range, label: "\n", role: .text)
                    continue
                }
                let appearance = atomAppearance(
                    nodeType: nodeType,
                    attrsJSON: attrsJSON,
                    paint: paint,
                    theme: theme,
                    warningSemanticGeneration: warningSemanticGeneration
                )
                let displayLabel = label.isEmpty ? " " : label
                let labelLine = CTLineCreateWithAttributedString(
                    NSAttributedString(string: displayLabel, attributes: appearance.attributes)
                )
                var labelAscent: CGFloat = 0
                var labelDescent: CGFloat = 0
                var labelLeading: CGFloat = 0
                let labelWidth = CGFloat(CTLineGetTypographicBounds(labelLine, &labelAscent, &labelDescent, &labelLeading))
                let requestedHeight = EditorTheme.cgFloat(appearance.attributes[editorInlineLineHeightAttribute]) ?? 0
                let extra = max(0, requestedHeight - labelAscent - max(labelDescent, 2)) / 2
                let metrics = PreparedAtomMetrics(
                    width: max(paint.font.lineHeight, labelWidth + appearance.padding.left + appearance.padding.right),
                    ascent: labelAscent + extra + appearance.padding.top,
                    descent: max(labelDescent, 2) + extra + appearance.padding.bottom
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
                        docPos: docPos,
                        label: displayLabel,
                        metrics: metrics,
                        line: labelLine,
                        appearance: appearance
                    )
                )
                if nodeType == "mention" {
                    semanticRanges.append(.mention(range: range, docPos: docPos, label: displayLabel, attrsJSON: attrsJSON))
                    appendAccessibilityRange(
                        range,
                        label: displayLabel,
                        role: .mention(semanticIndex: semanticRanges.count - 1)
                    )
                } else {
                    appendAccessibilityRange(range, label: displayLabel, role: .text)
                }
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
        return PreparedAttributedBlock(
            string: result,
            atoms: atoms,
            semanticRanges: semanticRanges,
            accessibilityRanges: accessibilityRanges,
            retainedBytes: 256 + stringBytes + attributeBytes + atomBytes
        )
    }

    func href(in marks: [FfiViewerMark]) -> String? {
        for mark in marks where mark.markType == "link" {
            if let href = jsonDictionary(mark.attrsJson)["href"] as? String, !href.isEmpty { return href }
        }
        return nil
    }

    func attributes(for marks: [FfiViewerMark], paint: PreparedTextPaint, theme: PreparedProseTheme, warningSemanticGeneration: String) -> [NSAttributedString.Key: Any] {
        if let sheet = theme.styleSheet {
            var base: [NSAttributedString.Key: Any] = [.font: paint.font, .foregroundColor: paint.color]
            EditorStyleSheet.applyText(paint.textValues, to: &base, scale: theme.fontScale)
            let markValues: [Any] = marks.map { mark in
                var values = jsonDictionary(mark.attrsJson)
                values["type"] = mark.markType
                return values
            }
            var resolved = sheet.inlineAttributes(markValues, base: base, scale: theme.fontScale)
            for mark in marks {
                let values = jsonDictionary(mark.attrsJson)
                switch mark.markType {
                case "textColor", "color", "foregroundColor":
                    if let color = EditorTheme.color(from: values["color"] ?? values["textColor"]) { resolved[.foregroundColor] = color }
                case "highlight", "backgroundColor":
                    if let color = EditorTheme.color(from: values["color"] ?? values["backgroundColor"]) { resolved[.backgroundColor] = color }
                case "textStyle", "font": EditorStyleSheet.applyText(values, to: &resolved, scale: theme.fontScale)
                default: break
                }
            }
            return coreTextAttributes(resolved)
        }
        var linkTheme: EditorLinkTheme?
        var explicitForeground: UIColor?
        var background: UIColor?
        var fontFamily: String?
        var fontSize: CGFloat?
        var underline = false
        var useMonospace = false
        var strike = false
        var wantsBold = false
        var wantsItalic = false
        var hasLink = false
        for mark in marks {
            let values = jsonDictionary(mark.attrsJson)
            switch mark.markType {
            case "bold", "strong": wantsBold = true
            case "italic", "em": wantsItalic = true
            case "underline": underline = true
            case "strike", "strikethrough": strike = true
            case "code": useMonospace = true
            case "link":
                hasLink = true
                linkTheme = theme.link
                if let linkBackground = theme.link?.backgroundColor { background = linkBackground }
                underline = underline || (theme.link?.underline ?? true)
            case "textColor", "color", "foregroundColor":
                explicitForeground = EditorTheme.color(from: values["color"] ?? values["textColor"]) ?? explicitForeground
            case "highlight", "backgroundColor":
                background = EditorTheme.color(from: values["color"] ?? values["backgroundColor"]) ?? background
            case "textStyle", "font":
                if let family = values["fontFamily"] as? String, !family.isEmpty { fontFamily = family }
                if let markedSize = (values["fontSize"] as? NSNumber).map({ CGFloat(truncating: $0) }), markedSize.isFinite, markedSize > 0 {
                    fontSize = markedSize
                }
            default: break
            }
        }
        let linkStyle = linkTheme.map {
            EditorTextStyle(fontFamily: $0.fontFamily, fontSize: $0.fontSize, fontWeight: $0.fontWeight, fontStyle: $0.fontStyle)
        }
        var font = ViewerFontEnvironment.shared.resolveFont(
            style: linkStyle,
            fallback: paint.font,
            fontScale: theme.fontScale,
            semanticGeneration: warningSemanticGeneration
        )
        let scaledMarkSize = fontSize.map { $0 * theme.fontScale }
        if let fontFamily {
            font = ViewerFontEnvironment.shared.resolveFont(
                family: fontFamily,
                size: scaledMarkSize ?? font.pointSize,
                fallback: font,
                semanticGeneration: warningSemanticGeneration
            )
        }
        if let scaledMarkSize { font = font.withSize(scaledMarkSize) }
        var markTraits: UIFontDescriptor.SymbolicTraits = []
        if wantsBold { markTraits.insert(.traitBold) }
        if wantsItalic { markTraits.insert(.traitItalic) }
        if useMonospace {
            font = ViewerFontEnvironment.shared.resolveFont(
                family: "monospace",
                size: font.pointSize,
                fallback: font,
                additionalTraits: markTraits,
                semanticGeneration: warningSemanticGeneration
            )
        } else {
            font = ViewerFontEnvironment.shared.resolveFont(
                family: nil,
                size: font.pointSize,
                fallback: font,
                additionalTraits: markTraits,
                semanticGeneration: warningSemanticGeneration
            )
        }
        var attributes = baseAttributes(paint)
        // An explicit text-color mark is document content and therefore wins
        // over link-theme paint regardless of compiler mark ordering.
        let foreground = explicitForeground ?? (hasLink ? linkTheme?.color ?? UIColor.systemBlue : paint.color)
        attributes[kCTFontAttributeName as NSAttributedString.Key] = Self.coreTextFont(from: font)
        attributes[kCTForegroundColorAttributeName as NSAttributedString.Key] = foreground.cgColor
        if let background { attributes[kCTBackgroundColorAttributeName as NSAttributedString.Key] = background.cgColor }
        if underline { attributes[kCTUnderlineStyleAttributeName as NSAttributedString.Key] = NSNumber(value: CTUnderlineStyle.single.rawValue) }
        if strike { attributes[preparedStrikeAttribute] = NSNumber(value: true) }
        return attributes
    }

    func strikeFragments(
        for line: CTLine,
        lineOrigin: CGPoint,
        displayScale: CGFloat
    ) -> [PreparedProseFragment] {
        let unit = displayScale.isFinite && displayScale > 0 ? 1 / displayScale : 1
        return (CTLineGetGlyphRuns(line) as? [CTRun] ?? []).flatMap { run -> [PreparedProseFragment] in
            let attributes = CTRunGetAttributes(run) as? [NSAttributedString.Key: Any] ?? [:]
            guard (attributes[preparedStrikeAttribute] as? NSNumber)?.boolValue == true,
                  let colorValue = attributes[kCTForegroundColorAttributeName as NSAttributedString.Key]
            else { return [] }
            let color = (attributes[.strikethroughColor] as? UIColor)?.cgColor ?? (colorValue as! CGColor)
            var ascent: CGFloat = 0
            let width = CGFloat(CTRunGetTypographicBounds(run, CFRange(location: 0, length: 0), &ascent, nil, nil))
            let stringRange = CTRunGetStringRange(run)
            let start = CGFloat(CTLineGetOffsetForStringIndex(line, stringRange.location, nil))
            let end = CGFloat(CTLineGetOffsetForStringIndex(line, stringRange.location + stringRange.length, nil))
            let extent = max(width, abs(end - start))
            guard extent.isFinite, extent > 0, ascent.isFinite, ascent > 0 else { return [] }
            let thickness = max(unit, min(2, ascent * 0.08))
            let centerY = lineOrigin.y - ascent * 0.35
            let style = NSUnderlineStyle(rawValue: attributes[.strikethroughStyle] as? Int ?? NSUnderlineStyle.single.rawValue)
            let dotted = style.contains(.patternDot)
            let dashed = style.contains(.patternDash)
            let doubleLine = style.rawValue & 0xff == NSUnderlineStyle.double.rawValue
            var result: [PreparedProseFragment] = []
            for offset: CGFloat in doubleLine ? [-thickness, thickness] : [0] {
                var x: CGFloat = 0
                while x < extent {
                    let length = min(extent - x, dotted ? thickness : (dashed ? thickness * 4 : extent))
                    result.append(PreparedProseFragment(kind: .strike,
                        bounds: CGRect(x: lineOrigin.x + min(start, end) + x, y: centerY + offset - thickness / 2, width: length, height: thickness),
                        color: color, cornerRadius: dotted ? thickness / 2 : 0, strokeWidth: thickness))
                    x += length + thickness * 2
                }
            }
            return result
        }
    }

    private func coreTextAttributes(_ attributes: [NSAttributedString.Key: Any]) -> [NSAttributedString.Key: Any] {
        var result = attributes
        if let font = attributes[.font] as? UIFont { result[kCTFontAttributeName as NSAttributedString.Key] = Self.coreTextFont(from: font) }
        if let color = attributes[.foregroundColor] as? UIColor { result[kCTForegroundColorAttributeName as NSAttributedString.Key] = color.cgColor }
        if let color = attributes[.backgroundColor] as? UIColor { result[kCTBackgroundColorAttributeName as NSAttributedString.Key] = color.cgColor }
        if let color = attributes[.underlineColor] as? UIColor { result[kCTUnderlineColorAttributeName as NSAttributedString.Key] = color.cgColor }
        if let decoration = attributes[.underlineStyle] { result[kCTUnderlineStyleAttributeName as NSAttributedString.Key] = decoration }
        if let spacing = attributes[.kern] { result[kCTKernAttributeName as NSAttributedString.Key] = spacing }
        if let decoration = attributes[.strikethroughStyle] as? Int, decoration != 0 { result[preparedStrikeAttribute] = NSNumber(value: true) }
        return result
    }

    func baseAttributes(_ paint: PreparedTextPaint) -> [NSAttributedString.Key: Any] {
        var attributes: [NSAttributedString.Key: Any] = [.font: paint.font, .foregroundColor: paint.color]
        EditorStyleSheet.applyText(paint.textValues, to: &attributes)
        return coreTextAttributes(attributes)
    }

    func makeListMarker(
        _ context: ViewerListContext,
        nestingDepth: Int,
        paint: PreparedTextPaint,
        theme: PreparedProseTheme
    ) -> PreparedListMarker {
        let scale: CGFloat
        if let sheet = theme.styleSheet {
            scale = EditorTheme.cgFloat(sheet["listMarker"]["scale"]) ?? (context.ordered ? 1 : LayoutConstants.unorderedListMarkerFontScale)
            if !context.ordered, context.kind != "task" {
                let diameter = EditorLayoutManager.unorderedBulletDrawingRect(usedRect: .zero, lineFragmentRect: .zero, markerWidth: 0, baselineY: 0, baseFont: paint.font, markerScale: scale, origin: .zero).width
                return PreparedListMarker(line: nil, label: "•", width: diameter, ascent: diameter / 2, descent: diameter / 2, checked: false)
            }
        } else {
            scale = !context.ordered && context.kind != "task" ? max(0.01, theme.listMarkerScale) : 1
        }
        let font = paint.font.withSize(max(1, paint.font.pointSize * scale))
        let label: String
        if context.kind == "task" {
            label = ""
        } else if context.ordered {
            label = OrderedListMarkerFormatter.label(
                index: UInt32(exactly: context.index) ?? 0,
                nestingDepth: nestingDepth,
                theme: theme.orderedListMarker
            )
        } else {
            label = "•"
        }
        guard !label.isEmpty else {
            let side = theme.styleSheet?.checkbox(checked: context.checked).number("size", fallback: 24) ?? max(font.lineHeight, font.pointSize)
            return PreparedListMarker(line: nil, label: label, width: side, ascent: side * 0.75, descent: side * 0.25, checked: context.checked)
        }
        let line = CTLineCreateWithAttributedString(
            NSAttributedString(
                string: label,
                attributes: [
                    kCTFontAttributeName as NSAttributedString.Key: Self.coreTextFont(from: font),
                    kCTForegroundColorAttributeName as NSAttributedString.Key: theme.listMarkerColor.cgColor,
                ]
            )
        )
        var ascent: CGFloat = 0
        var descent: CGFloat = 0
        var leading: CGFloat = 0
        let width = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, &leading))
        let imageBounds = CTLineGetImageBounds(line, nil)
        let visualAscent = imageBounds.isNull || imageBounds.isEmpty ? ascent : imageBounds.maxY
        let visualDescent = imageBounds.isNull || imageBounds.isEmpty ? descent : -imageBounds.minY
        return PreparedListMarker(line: line, label: label, width: max(1, width), ascent: visualAscent, descent: visualDescent, checked: context.checked)
    }

    private func atomAppearance(
        nodeType: String,
        attrsJSON: String,
        paint: PreparedTextPaint,
        theme: PreparedProseTheme,
        warningSemanticGeneration: String
    ) -> PreparedAtomAppearance {
        if nodeType == "mention" {
            let values = jsonDictionary(attrsJSON)
            let localMention = (values["mentionTheme"] as? [String: Any]).map(EditorMentionTheme.init(dictionary:))
            let mention = (theme.mention?.merged(with: localMention) ?? localMention)?.node
            var attributes = baseAttributes(paint)
            if let weight = mention?.fontWeight {
                let font = ViewerFontEnvironment.shared.resolveFont(
                    style: EditorTextStyle(fontWeight: weight),
                    fallback: paint.font,
                    fontScale: 1,
                    semanticGeneration: warningSemanticGeneration
                )
                attributes[kCTFontAttributeName as NSAttributedString.Key] = Self.coreTextFont(from: font)
            }
            attributes[kCTForegroundColorAttributeName as NSAttributedString.Key] = (mention?.textColor ?? paint.color).cgColor
            if let values = mention?.style, !values.isEmpty {
                var styled: [NSAttributedString.Key: Any] = [.font: paint.font, .foregroundColor: mention?.textColor ?? paint.color]
                EditorStyleSheet.applyText(values, to: &styled, scale: theme.fontScale)
                var boxValues = values
                boxValues["backgroundColor"] = values["backgroundColor"] ?? "#007aff1f"
                let box = EditorStyleBox(boxValues)
                return PreparedAtomAppearance(styleBox: box, attributes: coreTextAttributes(styled), background: box.color("backgroundColor") ?? .clear, borderColor: nil, borderWidth: 0, radius: 0, padding: UIEdgeInsets(top: 4, left: 6, bottom: 4, right: 6).adding(box.inset))
            }
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

    func jsonDictionary(_ json: String) -> [String: Any] {
        guard let data = json.data(using: .utf8), let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return value
    }

}

extension CoreTextProseLayoutEngine {
    func inlineBackgroundFragments(for line: CTLine, bounds: CGRect) -> [PreparedProseFragment] {
        (CTLineGetGlyphRuns(line) as? [CTRun] ?? []).compactMap { run in
            let attributes = CTRunGetAttributes(run) as? [NSAttributedString.Key: Any] ?? [:]
            guard let value = attributes[kCTBackgroundColorAttributeName as NSAttributedString.Key] else { return nil }
            let color = value as! CGColor
            let range = CTRunGetStringRange(run)
            let start = CGFloat(CTLineGetOffsetForStringIndex(line, range.location, nil))
            let end = CGFloat(CTLineGetOffsetForStringIndex(line, range.location + range.length, nil))
            return PreparedProseFragment(kind: .background, bounds: CGRect(x: bounds.minX + min(start, end), y: bounds.minY, width: abs(end - start), height: bounds.height), color: color)
        }
    }
}
