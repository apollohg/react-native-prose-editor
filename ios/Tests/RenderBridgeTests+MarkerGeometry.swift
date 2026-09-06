import XCTest
import CoreText

extension RenderBridgeTests {
    func testRender_themeOverridesHorizontalRuleMetrics() {
        let json = """
        [
            {"type": "voidBlock", "nodeType": "horizontalRule", "docPos": 0}
        ]
        """
        let theme = EditorTheme(dictionary: [
            "horizontalRule": [
                "color": "#445566",
                "thickness": 3,
                "verticalMargin": 12,
            ],
        ])

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attachment = result.attribute(.attachment, at: 0, effectiveRange: nil)
            as? HorizontalRuleAttachment
        XCTAssertEqual(attachment?.lineColor, EditorTheme.color(from: "#445566"))
        XCTAssertEqual(attachment?.lineHeight ?? 0, 3, accuracy: 0.1)
        XCTAssertEqual(attachment?.verticalPadding ?? 0, 12, accuracy: 0.1)
    }

    func testListMarkerDrawingRectUsesParagraphLineBox() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 10, width: 160, height: 28)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let baselineY: CGFloat = 28.140625
        let rect = EditorLayoutManager.markerDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            origin: CGPoint(x: 0, y: 0)
        )
        let typographicHeight = markerFont.ascender - markerFont.descender
        let leading = max(markerFont.lineHeight - typographicHeight, 0)
        let expectedY = baselineY - markerFont.ascender - (leading / 2.0)

        XCTAssertEqual(rect.origin.x, 4, accuracy: 0.1)
        XCTAssertEqual(rect.origin.y, expectedY, accuracy: 0.1)
        XCTAssertEqual(rect.height, markerFont.lineHeight, accuracy: 0.1)
    }

    func testListMarkerDrawingRectUsesFullLineFragmentWhenGlyphsUseShorterRect() {
        let markerFont = baseFont.withSize(18)
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 17)
        let baselineY: CGFloat = 30.140625
        let rect = EditorLayoutManager.markerDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            origin: CGPoint(x: 0, y: 0)
        )
        let typographicHeight = markerFont.ascender - markerFont.descender
        let leading = max(markerFont.lineHeight - typographicHeight, 0)
        let expectedY = baselineY - markerFont.ascender - (leading / 2.0)

        XCTAssertEqual(rect.origin.x, 4, accuracy: 0.1)
        XCTAssertEqual(rect.origin.y, expectedY, accuracy: 0.1)
        XCTAssertEqual(rect.height, markerFont.lineHeight, accuracy: 0.1)
    }

    func testListMarkerDrawingRectFallsBackToLineFragmentWhenUsedRectIsEmpty() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 10, width: 160, height: 28)
        let rect = EditorLayoutManager.markerDrawingRect(
            usedRect: CGRect(x: 24, y: 10, width: 160, height: 0),
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: 28.140625,
            markerFont: markerFont,
            origin: CGPoint(x: 0, y: 0)
        )
        let typographicHeight = markerFont.ascender - markerFont.descender
        let leading = max(markerFont.lineHeight - typographicHeight, 0)
        let expectedY = 28.140625 - markerFont.ascender - (leading / 2.0)

        XCTAssertEqual(rect.origin.x, 4, accuracy: 0.1)
        XCTAssertEqual(rect.origin.y, expectedY, accuracy: 0.1)
    }

    func testOrderedMarkerDrawingOriginAlignsToBaselineWithoutParagraphLineHeight() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let baselineY: CGFloat = 30.140625
        let markerText = "12. "

        let point = EditorLayoutManager.orderedMarkerDrawingOrigin(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            markerText: markerText,
            origin: .zero
        )
        let markerWidth = ceil(("12." as NSString).size(withAttributes: [
            .font: markerFont,
        ]).width)

        XCTAssertEqual(
            point.x,
            usedRect.minX - LayoutConstants.listMarkerTextGap - markerWidth,
            accuracy: 0.1
        )
        XCTAssertEqual(point.y, baselineY - markerFont.ascender, accuracy: 0.1)
    }

    func testOrderedMarkerDrawingOriginIgnoresTrailingSpaceForHorizontalAlignment() {
        let markerFont = baseFont
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let baselineY: CGFloat = 30.140625
        let markerText = "12. "

        let point = EditorLayoutManager.orderedMarkerDrawingOrigin(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            markerFont: markerFont,
            markerText: markerText,
            origin: .zero
        )
        let visibleWidth = ceil(("12." as NSString).size(withAttributes: [
            .font: markerFont,
        ]).width)
        let fullWidth = ceil((markerText as NSString).size(withAttributes: [
            .font: markerFont,
        ]).width)

        XCTAssertEqual(
            point.x,
            usedRect.minX - LayoutConstants.listMarkerTextGap - visibleWidth,
            accuracy: 0.1
        )
        XCTAssertNotEqual(
            point.x,
            usedRect.minX - LayoutConstants.listMarkerTextGap - fullWidth,
            accuracy: 0.1
        )
    }

    func testListMarkerBaseFontUsesParagraphFontInsteadOfLeadingBoldRun() {
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Bold", "marks": ["bold"]},
            {"type": "textRun", "text": " start", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """

        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let textFont = attrs[.font] as? UIFont
        let markerBaseFont = attrs[RenderBridgeAttributes.listMarkerBaseFont] as? UIFont

        XCTAssertTrue(
            textFont?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "First text run should still be bold"
        )
        XCTAssertNotNil(markerBaseFont, "List marker should carry its paragraph base font")
        XCTAssertFalse(
            markerBaseFont?.fontDescriptor.symbolicTraits.contains(.traitBold) ?? false,
            "Marker base font should ignore inline bold marks on the first run"
        )
        XCTAssertEqual(markerBaseFont?.pointSize ?? 0, baseFont.pointSize, accuracy: 0.1)
    }

    func testListMarkerParagraphStylePreservesThemedLineHeight() {
        let sourceStyle = NSMutableParagraphStyle()
        sourceStyle.minimumLineHeight = 28
        sourceStyle.maximumLineHeight = 28

        let markerStyle = EditorLayoutManager.markerParagraphStyle(from: [
            .paragraphStyle: sourceStyle,
        ])

        XCTAssertEqual(markerStyle.minimumLineHeight, 28, accuracy: 0.1)
        XCTAssertEqual(markerStyle.maximumLineHeight, 28, accuracy: 0.1)
        XCTAssertEqual(markerStyle.alignment, .right)
        XCTAssertEqual(markerStyle.lineBreakMode, .byClipping)
        XCTAssertEqual(markerStyle.firstLineHeadIndent, 0, accuracy: 0.1)
        XCTAssertEqual(markerStyle.headIndent, 0, accuracy: 0.1)
        XCTAssertEqual(markerStyle.tailIndent, 0, accuracy: 0.1)
    }

    func testUnorderedBulletDrawingRectCentersBulletOnTextMidline() {
        let rect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: CGRect(x: 24, y: 14, width: 160, height: 19),
            lineFragmentRect: CGRect(x: 24, y: 8, width: 160, height: 32),
            markerWidth: 20,
            baselineY: 28.140625,
            baseFont: baseFont,
            markerScale: 2,
            origin: .zero
        )
        let targetMidline = 28.140625 - ((baseFont.xHeight > 0 ? baseFont.xHeight : baseFont.capHeight) / 2.0)

        XCTAssertEqual(rect.midY, targetMidline, accuracy: 0.1)
        XCTAssertGreaterThan(rect.width, 0)
        XCTAssertGreaterThan(rect.height, 0)
    }

    func testUnorderedBulletDrawingRectPreservesTextSideGapAcrossMarkerScales() {
        let usedRect = CGRect(x: 24, y: 14, width: 160, height: 19)
        let lineFragmentRect = CGRect(x: 24, y: 8, width: 160, height: 32)
        let baselineY: CGFloat = 28.140625

        let normalRect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            baseFont: baseFont,
            markerScale: 1,
            origin: .zero
        )
        let scaledRect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: usedRect,
            lineFragmentRect: lineFragmentRect,
            markerWidth: 20,
            baselineY: baselineY,
            baseFont: baseFont,
            markerScale: 2,
            origin: .zero
        )

        XCTAssertEqual(usedRect.minX - normalRect.maxX, usedRect.minX - scaledRect.maxX, accuracy: 0.1)
        XCTAssertEqual(usedRect.minX - scaledRect.maxX, LayoutConstants.listMarkerTextGap, accuracy: 0.1)
    }

    func testUnorderedBulletDrawingRectReproducesTallLineHeightListItem() {
        let theme = EditorTheme(dictionary: [
            "paragraph": [
                "lineHeight": 32,
            ],
            "list": [
                "markerScale": 2,
            ],
        ])
        let json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Bullet item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """
        let result = RenderBridge.renderElements(
            fromJSON: json,
            baseFont: baseFont,
            textColor: textColor,
            theme: theme
        )

        let attrs = result.attributes(at: 0, effectiveRange: nil)
        let textFont = attrs[.font] as? UIFont ?? baseFont
        let paragraphStyle = attrs[.paragraphStyle] as? NSParagraphStyle
        let markerScale = (attrs[RenderBridgeAttributes.listMarkerScale] as? NSNumber)
            .map { CGFloat(truncating: $0) }
            ?? 1
        let bulletRect = EditorLayoutManager.unorderedBulletDrawingRect(
            usedRect: CGRect(x: 24, y: 14, width: 160, height: 19),
            lineFragmentRect: CGRect(x: 24, y: 8, width: 160, height: 32),
            markerWidth: 20,
            baselineY: 28.140625,
            baseFont: textFont,
            markerScale: markerScale,
            origin: .zero
        )
        let expectedCenterY = 28.140625 - ((textFont.xHeight > 0 ? textFont.xHeight : textFont.capHeight) / 2.0)

        XCTAssertNotNil(attrs[RenderBridgeAttributes.listMarkerContext])
        XCTAssertEqual(paragraphStyle?.minimumLineHeight ?? 0, 32, accuracy: 0.1)
        XCTAssertEqual(paragraphStyle?.maximumLineHeight ?? 0, 32, accuracy: 0.1)
        XCTAssertEqual(bulletRect.midY, expectedCenterY, accuracy: 0.1)
        XCTAssertGreaterThan(bulletRect.width, 0)
        XCTAssertGreaterThan(bulletRect.height, 0)
        XCTAssertEqual(bulletRect.width, bulletRect.height, accuracy: 0.1)
    }

    func testOrderedListMarkerBaselineOffsetIsNeutral() {
        let orderedContext: [String: Any] = ["ordered": true]

        let offset = EditorLayoutManager.markerBaselineOffset(
            for: orderedContext,
            baseFont: baseFont,
            markerFont: baseFont
        )

        XCTAssertEqual(offset, 0, accuracy: 0.1)
    }

    func testMarkerBaseFontPrefersStoredParagraphFont() {
        let boldDescriptor = baseFont.fontDescriptor.withSymbolicTraits(.traitBold)
            ?? baseFont.fontDescriptor
        let boldFont = UIFont(descriptor: boldDescriptor, size: baseFont.pointSize)
        let resolved = EditorLayoutManager.markerBaseFont(from: [
            .font: boldFont,
            RenderBridgeAttributes.listMarkerBaseFont: baseFont,
        ])

        XCTAssertFalse(
            resolved.fontDescriptor.symbolicTraits.contains(.traitBold),
            "Stored paragraph font should win over the inline bold run font"
        )
        XCTAssertEqual(resolved.pointSize, baseFont.pointSize, accuracy: 0.1)
    }

}
