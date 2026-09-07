import UIKit

struct EditorStyleSheet {
    let styles: [String: [String: Any]]

    static func collapsedMargin(_ first: CGFloat, _ second: CGFloat) -> CGFloat {
        max(0, first, second) + min(0, first, second)
    }

    static func element(_ nodeType: String) -> String {
        switch nodeType {
        case "code_block": return "codeBlock"
        case "bullet_list": return "bulletList"
        case "ordered_list": return "orderedList"
        case "task_list": return "taskList"
        case "list_item": return "listItem"
        case "task_item": return "taskItem"
        case "horizontal_rule": return "horizontalRule"
        case "strong": return "bold"
        case "em": return "italic"
        case "code": return "inlineCode"
        case "strikethrough": return "strike"
        default: return nodeType
        }
    }

    subscript(_ element: String) -> [String: Any] { styles[Self.element(element)] ?? [:] }

    func box(_ element: String) -> EditorStyleBox {
        var values: [String: Any] = [:]
        switch Self.element(element) {
        case "codeBlock":
            values = ["backgroundColor": UIColor.secondarySystemBackground, "paddingLeft": 12, "paddingRight": 12,
                      "paddingTop": 8, "paddingBottom": 8, "borderRadius": 8]
        case "blockquote":
            values = ["borderLeftWidth": 3, "borderLeftColor": UIColor.systemGray3, "paddingLeft": 16]
        case "paragraph": values = ["marginBottom": 8]
        case "h1", "h2", "h3", "h4", "h5", "h6": values = ["marginBottom": 10]
        case "listItem", "taskItem": values = ["marginBottom": 4]
        case "horizontalRule": values = ["backgroundColor": UIColor.separator, "marginTop": 12, "marginBottom": 12]
        default: break
        }
        values.merge(self[element]) { _, new in new }
        return EditorStyleBox(values)
    }

    func textStyle(_ element: String, ancestors: [String] = [], semantic: EditorTextStyle? = nil) -> EditorTextStyle {
        var style = EditorTextStyle(dictionary: self["text"])
        let name = Self.element(element)
        if let level = Int(name.dropFirst()), name.first == "h", (1...6).contains(level) {
            style = style.merged(with: EditorTextStyle(fontSize: [32, 28, 24, 21, 19, 17][level - 1], fontWeight: "700"))
        }
        if name == "codeBlock" { style = style.merged(with: EditorTextStyle(fontFamily: "monospace")) }
        style = style.merged(with: semantic)
        for ancestor in ancestors { style = style.merged(with: EditorTextStyle(dictionary: self[ancestor])) }
        return style.merged(with: EditorTextStyle(dictionary: self[name]))
    }

    func textValues(_ element: String, ancestors: [String] = []) -> [String: Any] {
        let keys = ["letterSpacing", "textAlign", "textDecorationLine", "textDecorationColor", "textDecorationStyle"]
        var result: [String: Any] = [:]
        for layer in ["text"] + ancestors + [element] {
            for key in keys where self[layer][key] != nil { result[key] = self[layer][key] }
        }
        return result
    }

    func inlineAttributes(_ marks: [Any], base: [NSAttributedString.Key: Any], scale: CGFloat = 1) -> [NSAttributedString.Key: Any] {
        var attributes = base
        var byName: [String: [String: Any]] = [:]
        for mark in marks {
            if let name = mark as? String { byName[Self.element(name)] = [:] }
            if let object = mark as? [String: Any], let name = object["type"] as? String {
                byName[Self.element(name)] = object
            }
        }
        for name in ["inlineCode", "bold", "italic", "link", "underline", "strike"] where byName[name] != nil {
            var values: [String: Any] = [:]
            switch name {
            case "inlineCode": values = ["fontFamily": "monospace"]
            case "bold": values = ["fontWeight": "700"]
            case "italic": values = ["fontStyle": "italic"]
            case "link": values = ["color": "#007affff", "textDecorationLine": "underline"]
            case "underline": values = ["textDecorationLine": "underline"]
            case "strike": values = ["textDecorationLine": "line-through"]
            default: break
            }
            values.merge(self[name]) { _, new in new }
            Self.applyText(values, to: &attributes, scale: scale)
            if name == "link", let href = byName[name]?["href"] as? String {
                attributes[RenderBridgeAttributes.linkHref] = href
            }
        }
        return attributes
    }

    static func applyText(_ values: [String: Any], to attributes: inout [NSAttributedString.Key: Any], scale: CGFloat = 1) {
        var fallback = attributes[.font] as? UIFont ?? .systemFont(ofSize: 16)
        var traits = fallback.fontDescriptor.symbolicTraits
        if let weight = values["fontWeight"] as? String {
            traits.remove(.traitBold)
            if EditorTheme.shouldApplyBoldTrait(weight) { traits.insert(.traitBold) }
        }
        if let style = values["fontStyle"] as? String {
            traits.remove(.traitItalic)
            if style == "italic" { traits.insert(.traitItalic) }
        }
        var descriptor = fallback.fontDescriptor
        if let weight = values["fontWeight"] as? String {
            descriptor = descriptor.addingAttributes([.traits: [UIFontDescriptor.TraitKey.weight: EditorTheme.fontWeight(from: weight)]])
        }
        if let resolved = descriptor.withSymbolicTraits(traits) { fallback = UIFont(descriptor: resolved, size: fallback.pointSize) }
        attributes[.font] = ViewerFontEnvironment.shared.resolveFont(style: EditorTextStyle(dictionary: values), fallback: fallback, fontScale: scale, semanticGeneration: "editor-stylesheet")
        if let color = EditorTheme.color(from: values["color"]) { attributes[.foregroundColor] = color }
        if let color = EditorTheme.color(from: values["backgroundColor"]) { attributes[.backgroundColor] = color }
        if let spacing = EditorTheme.cgFloat(values["letterSpacing"]) { attributes[.kern] = spacing * scale }
        if let height = EditorTheme.cgFloat(values["lineHeight"]) { attributes[editorInlineLineHeightAttribute] = height * scale }
        var decoration = NSUnderlineStyle.single
        switch values["textDecorationStyle"] as? String {
        case "double": decoration = .double
        case "dashed": decoration = [.single, .patternDash]
        case "dotted": decoration = [.single, .patternDot]
        default: break
        }
        if let line = values["textDecorationLine"] as? String {
            if line == "none" {
                attributes.removeValue(forKey: .underlineStyle)
                attributes.removeValue(forKey: .strikethroughStyle)
            } else {
                if line.contains("underline") { attributes[.underlineStyle] = decoration.rawValue }
                if line.contains("line-through") { attributes[.strikethroughStyle] = decoration.rawValue }
            }
        }
        if let color = EditorTheme.color(from: values["textDecorationColor"]) {
            attributes[.underlineColor] = color
            attributes[.strikethroughColor] = color
        }
    }
}

struct EditorStyleBox {
    let values: [String: Any]
    init(_ values: [String: Any] = [:]) { self.values = values }
    func number(_ key: String, fallback: CGFloat = 0) -> CGFloat { EditorTheme.cgFloat(values[key]) ?? fallback }
    func color(_ key: String) -> UIColor? { (values[key] as? UIColor) ?? EditorTheme.color(from: values[key]) }
    var padding: UIEdgeInsets { insets("padding") }
    var margin: UIEdgeInsets { insets("margin") }
    var borders: UIEdgeInsets {
        UIEdgeInsets(top: number("borderTopWidth", fallback: number("borderWidth")), left: number("borderLeftWidth", fallback: number("borderWidth")), bottom: number("borderBottomWidth", fallback: number("borderWidth")), right: number("borderRightWidth", fallback: number("borderWidth")))
    }
    var inset: UIEdgeInsets { padding.adding(borders) }
    var outerInsets: UIEdgeInsets { inset.adding(margin) }
    private func insets(_ key: String) -> UIEdgeInsets {
        UIEdgeInsets(top: number(key + "Top", fallback: number(key)), left: number(key + "Left", fallback: number(key)), bottom: number(key + "Bottom", fallback: number(key)), right: number(key + "Right", fallback: number(key)))
    }
    var radii: [CGFloat] { ["TopLeft", "TopRight", "BottomRight", "BottomLeft"].map { number("border" + $0 + "Radius", fallback: number("borderRadius")) } }

    func path(in rect: CGRect, inner: Bool = false) -> UIBezierPath {
        let edges = borders
        let bounds = inner ? rect.inset(by: edges) : rect
        var corners = radii
        let sums = [corners[0] + corners[1], corners[3] + corners[2], corners[0] + corners[3], corners[1] + corners[2]]
        let factor = zip([rect.width, rect.width, rect.height, rect.height], sums).reduce(CGFloat(1)) { result, pair in pair.1 > 0 ? min(result, pair.0 / pair.1) : result }
        corners = corners.map { max(0, $0 * factor) }
        if inner {
            corners = zip(corners, [max(edges.top, edges.left), max(edges.top, edges.right), max(edges.bottom, edges.right), max(edges.bottom, edges.left)]).map { max(0, $0 - $1) }
        }
        let path = UIBezierPath()
        let points = [CGPoint(x: bounds.minX, y: bounds.minY), CGPoint(x: bounds.maxX, y: bounds.minY), CGPoint(x: bounds.maxX, y: bounds.maxY), CGPoint(x: bounds.minX, y: bounds.maxY)]
        path.move(to: CGPoint(x: bounds.minX + corners[0], y: bounds.minY))
        path.addLine(to: CGPoint(x: bounds.maxX - corners[1], y: bounds.minY))
        path.addQuadCurve(to: CGPoint(x: bounds.maxX, y: bounds.minY + corners[1]), controlPoint: points[1])
        path.addLine(to: CGPoint(x: bounds.maxX, y: bounds.maxY - corners[2]))
        path.addQuadCurve(to: CGPoint(x: bounds.maxX - corners[2], y: bounds.maxY), controlPoint: points[2])
        path.addLine(to: CGPoint(x: bounds.minX + corners[3], y: bounds.maxY))
        path.addQuadCurve(to: CGPoint(x: bounds.minX, y: bounds.maxY - corners[3]), controlPoint: points[3])
        path.addLine(to: CGPoint(x: bounds.minX, y: bounds.minY + corners[0]))
        path.addQuadCurve(to: CGPoint(x: bounds.minX + corners[0], y: bounds.minY), controlPoint: points[0])
        path.close()
        return path
    }

    func draw(in rect: CGRect, context: CGContext) {
        guard rect.width > 0, rect.height > 0 else { return }
        context.saveGState()
        defer { context.restoreGState() }
        context.addPath(path(in: rect).cgPath)
        context.clip()
        if let background = color("backgroundColor") { context.setFillColor(background.cgColor); context.fill(rect) }
        let edges = borders
        let outer = [CGPoint(x: rect.minX, y: rect.minY), CGPoint(x: rect.maxX, y: rect.minY), CGPoint(x: rect.maxX, y: rect.maxY), CGPoint(x: rect.minX, y: rect.maxY)]
        // Extend side clips through the rounded inner corners.
        let reach = min(edges.left + edges.right > 0 ? rect.width / (edges.left + edges.right) : .infinity,
                        edges.top + edges.bottom > 0 ? rect.height / (edges.top + edges.bottom) : .infinity)
        guard reach.isFinite else { return }
        let join = rect.inset(by: UIEdgeInsets(top: edges.top * reach, left: edges.left * reach, bottom: edges.bottom * reach, right: edges.right * reach))
        let inside = [CGPoint(x: join.minX, y: join.minY), CGPoint(x: join.maxX, y: join.minY), CGPoint(x: join.maxX, y: join.maxY), CGPoint(x: join.minX, y: join.maxY)]
        let ring = path(in: rect)
        ring.append(path(in: rect, inner: true))
        context.addPath(ring.cgPath)
        context.clip(using: .evenOdd)
        let borderColors = ["Top", "Right", "Bottom", "Left"].map { color("border" + $0 + "Color") ?? color("borderColor") ?? .black }
        if (values["borderStyle"] as? String ?? "solid") == "solid",
           borderColors.allSatisfy({ $0 == borderColors[0] }) {
            context.setFillColor(borderColors[0].cgColor)
            context.fill(rect)
            return
        }
        for (index, side) in ["Top", "Right", "Bottom", "Left"].enumerated() {
            let width = [edges.top, edges.right, edges.bottom, edges.left][index]
            guard width > 0 else { continue }
            context.saveGState()
            let next = (index + 1) % 4
            let wedge = UIBezierPath()
            wedge.move(to: outer[index]); wedge.addLine(to: outer[next]); wedge.addLine(to: inside[next]); wedge.addLine(to: inside[index]); wedge.close()
            context.addPath(wedge.cgPath); context.clip()
            let color = color("border" + side + "Color") ?? color("borderColor") ?? .black
            context.setFillColor(color.cgColor)
            if let style = values["borderStyle"] as? String, style != "solid" {
                context.setStrokeColor(color.cgColor)
                context.setLineWidth(width * 2)
                context.setLineCap(style == "dotted" ? .round : .butt)
                context.setLineDash(phase: 0, lengths: style == "dotted" ? [0, width * 2] : [width * 3, width * 2])
                context.addPath(path(in: rect).cgPath); context.strokePath()
            } else { context.fill(rect) }
            context.restoreGState()
        }
    }

    func imageRect(_ size: CGSize, in bounds: CGRect) -> CGRect {
        let content = bounds.inset(by: inset)
        guard size.width > 0, size.height > 0, (values["resizeMode"] as? String) != "stretch" else { return content }
        let ratios = [content.width / size.width, content.height / size.height]
        let scale = (values["resizeMode"] as? String) == "cover" ? ratios.max()! : ratios.min()!
        let target = CGSize(width: size.width * scale, height: size.height * scale)
        return CGRect(x: content.midX - target.width / 2, y: content.midY - target.height / 2, width: target.width, height: target.height)
    }
}

extension UIEdgeInsets {
    func adding(_ other: UIEdgeInsets) -> UIEdgeInsets {
        UIEdgeInsets(top: top + other.top, left: left + other.left, bottom: bottom + other.bottom, right: right + other.right)
    }
}

extension EditorTheme {
    static func legacyProjection(styles: [String: [String: Any]], root: [String: Any]) -> [String: Any] {
        let sheet = EditorStyleSheet(styles: styles)
        var result: [String: Any] = [:]
        result["text"] = sheet["text"]
        result["paragraph"] = sheet["paragraph"]
        result["headings"] = Dictionary(uniqueKeysWithValues: (1...6).map { ("h\($0)", sheet["h\($0)"]) })
        let code = sheet.box("codeBlock")
        result["codeBlock"] = ["text": sheet["codeBlock"], "backgroundColor": code.values["backgroundColor"] ?? "transparent", "paddingHorizontal": 0, "paddingVertical": 0, "borderRadius": 0]
        result["blockquote"] = ["text": sheet["blockquote"], "indent": 0, "borderWidth": 0, "markerGap": 0]
        let marker = sheet["listMarker"]
        var list = sheet["bulletList"]
        list["markerColor"] = marker["color"]
        list["markerScale"] = marker["scale"]
        list["markerGap"] = marker["gap"]
        list["orderedMarker"] = marker["ordered"]
        result["list"] = list
        let rule = sheet.box("horizontalRule")
        result["horizontalRule"] = ["color": rule.values["backgroundColor"] ?? "transparent", "thickness": sheet["horizontalRule"]["height"] ?? 1, "verticalMargin": 0]
        var link = sheet["link"]
        if let decoration = link["textDecorationLine"] as? String { link["underline"] = decoration.contains("underline") }
        result["links"] = link
        let content = sheet.box("content")
        result["backgroundColor"] = content.values["backgroundColor"]
        let inset = content.inset
        result["contentInsets"] = ["top": inset.top, "left": inset.left, "bottom": inset.bottom, "right": inset.right]
        result["placeholderColor"] = sheet["placeholder"]["color"]
        result["toolbar"] = root["toolbar"]
        var mention = sheet["mention"]
        mention["textColor"] = mention["color"]
        mention["style"] = sheet["mention"]
        result["mentions"] = root["mentions"]
        if !mention.isEmpty {
            var merged = root["mentions"] as? [String: Any] ?? [:]
            var node = mention
            let addon = merged["node"] as? [String: Any] ?? [:]
            node.merge(addon) { _, new in new }
            var rich = sheet["mention"]
            rich.merge(addon["style"] as? [String: Any] ?? [:]) { _, new in new }
            node["style"] = rich
            merged["node"] = node
            result["mentions"] = merged
        }
        return result
    }
}

let editorStyleBoxesAttribute = NSAttributedString.Key("com.apollohg.editor.styleBoxes")
let editorBlockSpacingBoxAttribute = NSAttributedString.Key("com.apollohg.editor.blockSpacingBox")

final class EditorRenderedBox: NSObject {
    let box: EditorStyleBox
    let depth: Int
    let leading: CGFloat
    let trailing: CGFloat
    var topInset: CGFloat = 0
    var bottomInset: CGFloat = 0
    init(box: EditorStyleBox, depth: Int, leading: CGFloat, trailing: CGFloat) {
        self.box = box
        self.depth = depth
        self.leading = leading
        self.trailing = trailing
    }
}

extension RenderBridge {
    static func closeStyledBlock(_ context: BlockContext, ancestors: [BlockContext], in result: NSMutableAttributedString, theme: EditorTheme?, baseFont: UIFont, textColor: UIColor, omitBottomMargin: Bool = false) {
        guard let sheet = theme?.styleSheet else { return }
        var start = min(context.styleStart, result.length)
        while start < result.length, result.attribute(RenderBridgeAttributes.blockBoundary, at: start, effectiveRange: nil) != nil { start += 1 }
        if start == result.length, !isTransparentContainer(context.nodeType), !isListItemNodeType(context.nodeType) {
            var attributes = applyBlockStyle(to: defaultAttributes(baseFont: resolvedFont(for: ancestors + [context], baseFont: baseFont, theme: theme), textColor: resolvedTextColor(for: ancestors + [context], textColor: textColor, theme: theme)), blockStack: ancestors + [context], theme: theme)
            attributes[RenderBridgeAttributes.syntheticPlaceholder] = true
            result.append(NSAttributedString(string: "\u{200B}", attributes: attributes))
        }
        guard start < result.length else { return }
        let range = NSRange(location: start, length: result.length - start)
        let outer = ancestors.reduce(UIEdgeInsets.zero) { $0.adding(sheet.box($1.nodeType).outerInsets) }
        var values = sheet.box(context.nodeType).values
        if omitBottomMargin { values["marginBottom"] = 0 }
        let box = EditorStyleBox(values)
        let descriptor = EditorRenderedBox(box: box, depth: ancestors.count, leading: outer.left + box.margin.left, trailing: outer.right + box.margin.right)
        result.enumerateAttribute(editorStyleBoxesAttribute, in: range) { value, subrange, _ in
            var boxes = value as? [EditorRenderedBox] ?? []
            boxes.insert(descriptor, at: 0)
            result.addAttribute(editorStyleBoxesAttribute, value: boxes, range: subrange)
        }
        let nsString = result.string as NSString
        let first = NSIntersectionRange(nsString.paragraphRange(for: NSRange(location: start, length: 0)), range)
        let last = NSIntersectionRange(nsString.paragraphRange(for: NSRange(location: result.length - 1, length: 0)), range)
        let isContainer = isTransparentContainer(context.nodeType) || isListItemNodeType(context.nodeType)
        for (target, leading) in [(first, true), (last, false)] {
            result.enumerateAttribute(.paragraphStyle, in: target) { value, subrange, _ in
                let style = (value as? NSParagraphStyle)?.mutableCopy() as? NSMutableParagraphStyle ?? NSMutableParagraphStyle()
                if leading {
                    style.paragraphSpacingBefore = (isContainer ? style.paragraphSpacingBefore : 0) + box.outerInsets.top
                } else {
                    style.paragraphSpacing = (isContainer ? style.paragraphSpacing : 0) + box.outerInsets.bottom
                }
                result.addAttribute(.paragraphStyle, value: style, range: subrange)
                if leading { descriptor.topInset = style.paragraphSpacingBefore - box.margin.top }
                else { descriptor.bottomInset = style.paragraphSpacing - box.margin.bottom }
            }
        }
    }

    static func collapseStyledSiblingMargins(in result: NSMutableAttributedString) {
        typealias Span = (box: EditorRenderedBox, parent: ObjectIdentifier?, range: NSRange)
        var spans: [ObjectIdentifier: Span] = [:]
        result.enumerateAttributes(in: NSRange(location: 0, length: result.length)) { attributes, range, _ in
            var boxes = attributes[editorStyleBoxesAttribute] as? [EditorRenderedBox] ?? []
            if let spacingBox = attributes[editorBlockSpacingBoxAttribute] as? EditorRenderedBox { boxes.append(spacingBox) }
            for (index, box) in boxes.enumerated() {
                let identity = ObjectIdentifier(box)
                let parent = index > 0 ? ObjectIdentifier(boxes[index - 1]) : nil
                spans[identity] = (box, parent, spans[identity].map { NSUnionRange($0.range, range) } ?? range)
            }
        }
        let text = result.string as NSString
        var leadingSpacing: [Int: (range: NSRange, value: CGFloat)] = [:]
        for opening in Dictionary(grouping: spans.values, by: { $0.range.location }).values {
            guard let outer = opening.min(by: { $0.box.depth < $1.box.depth }) else { continue }
            let paragraph = NSIntersectionRange(text.paragraphRange(for: NSRange(location: outer.range.location, length: 0)), outer.range)
            leadingSpacing[outer.range.location] = (paragraph, outer.box.topInset + outer.box.box.margin.top)
        }
        for siblings in Dictionary(grouping: spans.values, by: \.parent).values {
            let ordered = siblings.sorted { $0.range.location < $1.range.location }
            for (previous, next) in zip(ordered, ordered.dropFirst()) {
                let gap = NSRange(location: NSMaxRange(previous.range), length: max(0, next.range.location - NSMaxRange(previous.range)))
                var adjacent = NSMaxRange(previous.range) <= next.range.location
                result.enumerateAttribute(RenderBridgeAttributes.blockBoundary, in: gap) { value, _, _ in
                    if value == nil { adjacent = false }
                }
                guard adjacent else { continue }
                let bottom = previous.box.box.margin.bottom
                let top = next.box.box.margin.top
                let adjustment = bottom + top - EditorStyleSheet.collapsedMargin(bottom, top)
                leadingSpacing[next.range.location]?.value -= adjustment
            }
        }
        for spacing in leadingSpacing.values {
            result.enumerateAttribute(.paragraphStyle, in: spacing.range) { value, range, _ in
                guard let style = (value as? NSParagraphStyle)?.mutableCopy() as? NSMutableParagraphStyle,
                      style.paragraphSpacingBefore != spacing.value else { return }
                style.paragraphSpacingBefore = spacing.value
                result.addAttribute(.paragraphStyle, value: style, range: range)
            }
        }
        if result.length > 1 {
            result.enumerateAttribute(RenderBridgeAttributes.blockBoundary, in: NSRange(location: 1, length: result.length - 1)) { value, range, _ in
                guard value != nil, let style = result.attribute(.paragraphStyle, at: range.location - 1, effectiveRange: nil) else { return }
                result.addAttribute(.paragraphStyle, value: style, range: range)
            }
        }
    }
}

let editorMentionBoxAttribute = NSAttributedString.Key("com.apollohg.editor.mentionBox")
final class EditorMentionRenderedBox: NSObject {
    let box: EditorStyleBox
    let label: NSAttributedString?
    let size: CGSize
    let padding: UIEdgeInsets

    init(box: EditorStyleBox, label: NSAttributedString? = nil) {
        self.box = box
        self.label = label
        padding = UIEdgeInsets(top: 4, left: 6, bottom: 4, right: 6).adding(box.inset)
        let measured = label?.size() ?? .zero
        let lineHeight = box.number("lineHeight", fallback: measured.height)
        size = CGSize(width: ceil(measured.width) + padding.left + padding.right,
                      height: ceil(max(measured.height, lineHeight)) + padding.top + padding.bottom)
    }
}

final class EditorStyleBoxView: UIView {
    var box: EditorStyleBox? { didSet { setNeedsDisplay() } }
    override func draw(_ rect: CGRect) {
        guard let context = UIGraphicsGetCurrentContext() else { return }
        box?.draw(in: bounds, context: context)
    }
}

let editorTaskCheckboxAttribute = NSAttributedString.Key("com.apollohg.editor.taskCheckbox")

extension EditorStyleSheet {
    func checkbox(checked: Bool) -> EditorStyleBox {
        var values: [String: Any] = ["borderWidth": 1.8, "borderColor": "#8e8e93ff", "borderRadius": 5, "size": 24, "gap": 8, "checkColor": "#007affff"]
        values.merge(self["taskCheckbox"]) { _, new in new }
        if checked { values.merge(self["taskCheckbox"]["checked"] as? [String: Any] ?? [:]) { _, new in new } }
        return EditorStyleBox(values)
    }

    static func drawCheckbox(_ box: EditorStyleBox, in rect: CGRect, checked: Bool, context: CGContext) {
        box.draw(in: rect, context: context)
        guard checked else { return }
        let check = UIBezierPath()
        check.move(to: CGPoint(x: rect.minX + rect.width * 0.22, y: rect.midY))
        check.addLine(to: CGPoint(x: rect.minX + rect.width * 0.43, y: rect.maxY - rect.height * 0.24))
        check.addLine(to: CGPoint(x: rect.maxX - rect.width * 0.18, y: rect.minY + rect.height * 0.24))
        context.saveGState()
        context.setStrokeColor((box.color("checkColor") ?? .systemBlue).cgColor)
        context.setLineWidth(max(1.4, rect.width * 0.1))
        context.setLineCap(.round); context.setLineJoin(.round)
        context.addPath(check.cgPath); context.strokePath()
        context.restoreGState()
    }
}

let editorStyledContentAttribute = NSAttributedString.Key("com.apollohg.editor.styledContent")

let editorInlineLineHeightAttribute = NSAttributedString.Key("com.apollohg.editor.inlineLineHeight")
