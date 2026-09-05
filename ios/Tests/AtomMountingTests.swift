import XCTest

final class AtomMountingTests: XCTestCase {
    func testFabricMountIndexesIgnoreAtomAndInternalSubviews() {
        let editor = NativeEditorExpoView()
        let first = UIView()
        let atom = atomChild(key: "counter:1", height: 40)
        let second = UIView()

        editor.mountChildComponentView(first, index: 0)
        editor.mountChildComponentView(atom, index: 1)
        editor.mountChildComponentView(second, index: 2)

        XCTAssertEqual(editor.subviews.count, 3)
        XCTAssertTrue(editor.subviews[0] === editor.richTextView)
        XCTAssertTrue(editor.subviews[1] === first)
        XCTAssertTrue(editor.subviews[2] === second)

        editor.unmountChildComponentView(first, index: 0)
        XCTAssertTrue(editor.subviews[0] === editor.richTextView)
        XCTAssertTrue(editor.subviews[1] === second)
    }

    func testPrefixedReactChildIsReparentedIntoTextView() throws {
        let editor = NativeEditorExpoView()
        editor.frame = CGRect(x: 0, y: 0, width: 320, height: 240)
        installAtom(key: "counter:1", height: 80, in: editor.richTextView)
        editor.layoutIfNeeded()

        let child = atomChild(key: "counter:1", height: 80)
        editor.mountChildComponentView(child, index: 0)

        let container = try XCTUnwrap(child.superview as? AtomHostContainerView)
        XCTAssertTrue(container.superview === editor.richTextView.textView)
        XCTAssertEqual(container.atomKey, "counter:1")
    }

    func testFabricNativeIdMountsReactChildIntoAtomContainer() throws {
        let editor = NativeEditorExpoView()
        editor.frame = CGRect(x: 0, y: 0, width: 320, height: 240)
        installAtom(key: "counter:1", height: 80, in: editor.richTextView)
        editor.layoutIfNeeded()

        let componentViewClass = try XCTUnwrap(
            NSClassFromString("RCTViewComponentView") as? NSObject.Type
        )
        let child = try XCTUnwrap(componentViewClass.init() as? UIView)
        child.frame = CGRect(x: 0, y: 0, width: 100, height: 80)
        child.setValue("prose-atom:counter:1", forKey: "nativeId")
        editor.mountChildComponentView(child, index: 0)

        let container = try XCTUnwrap(child.superview as? AtomHostContainerView)
        XCTAssertTrue(container.superview === editor.richTextView.textView)
    }

    func testChildrenBindToAttachmentsByKeyInsteadOfMountOrder() throws {
        let editor = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 300))
        installAtoms(
            [
                AtomBlockAttachment(
                    atomKey: "counter:1",
                    nodeType: "counter",
                    docPos: 1,
                    reservedHeight: 60
                ),
                AtomBlockAttachment(
                    atomKey: "counter:2",
                    nodeType: "counter",
                    docPos: 2,
                    reservedHeight: 90
                ),
            ],
            in: editor
        )

        let second = atomChild(key: "counter:2", height: 90)
        let first = atomChild(key: "counter:1", height: 60)
        editor.mountAtomChild(second, atomKey: "counter:2")
        editor.mountAtomChild(first, atomKey: "counter:1")

        XCTAssertTrue(try XCTUnwrap(editor.atomHostContainer(for: "counter:1")).hostedView === first)
        XCTAssertTrue(try XCTUnwrap(editor.atomHostContainer(for: "counter:2")).hostedView === second)

        let unmatched = atomChild(key: "missing", height: 50)
        editor.mountAtomChild(unmatched, atomKey: "missing")
        XCTAssertTrue(try XCTUnwrap(editor.atomHostContainer(for: "missing")).isHidden)
    }

    func testChildHeightChangeUpdatesSpacerAndInvalidatesLayoutOnce() throws {
        let editor = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let attachment = installAtom(key: "counter:1", height: 40, in: editor)
        let child = atomChild(key: "counter:1", height: 96)

        editor.mountAtomChild(child, atomKey: "counter:1")
        let invalidations = editor.atomLayoutInvalidationCountForTesting
        try XCTUnwrap(editor.atomHostContainer(for: "counter:1")).layoutIfNeeded()

        XCTAssertEqual(attachment.reservedHeight, 96)
        XCTAssertEqual(editor.measuredAtomHeight(for: "counter:1"), 96)
        XCTAssertEqual(invalidations, 1)
        XCTAssertEqual(editor.atomLayoutInvalidationCountForTesting, invalidations)
    }

    func testUnmeasuredChildKeepsEstimateUntilReactSuppliesBounds() throws {
        let editor = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let attachment = installAtom(key: "counter:1", height: 40, in: editor)
        let child = UIView(frame: .zero)
        editor.mountAtomChild(child, atomKey: "counter:1")
        let container = try XCTUnwrap(editor.atomHostContainer(for: "counter:1"))
        container.layoutIfNeeded()

        XCTAssertNil(editor.measuredAtomHeight(for: "counter:1"))
        XCTAssertEqual(attachment.reservedHeight, 40)

        child.frame = CGRect(x: 0, y: 0, width: 280, height: 0)
        container.setNeedsLayout()
        container.layoutIfNeeded()

        XCTAssertEqual(editor.measuredAtomHeight(for: "counter:1"), 0)
        XCTAssertEqual(attachment.reservedHeight, 0)
    }

    func testChildCanCollapseToZeroAndExpandAgain() throws {
        let editor = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let attachment = installAtom(key: "counter:1", height: 40, in: editor)
        let child = atomChild(key: "counter:1", height: 96)
        editor.mountAtomChild(child, atomKey: "counter:1")
        let container = try XCTUnwrap(editor.atomHostContainer(for: "counter:1"))

        for height in [CGFloat(0), 72] {
            child.frame.size.height = height
            container.setNeedsLayout()
            container.layoutIfNeeded()

            XCTAssertEqual(editor.measuredAtomHeight(for: "counter:1"), height)
            XCTAssertEqual(attachment.reservedHeight, height)
        }
    }

    func testChildMountedBeforeAttachmentSuppliesMeasuredHeightToLaterRender() throws {
        let editor = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        XCTAssertTrue(editor.applyAtomRenderConfiguration(AtomRenderConfiguration(
            registeredNodeTypes: ["counter"],
            estimatedHeights: ["counter": 40],
            measuredHeights: [:]
        )))

        let child = atomChild(key: "counter:0", height: 96)
        editor.mountAtomChild(child, atomKey: "counter:0")

        XCTAssertEqual(editor.measuredAtomHeight(for: "counter:0"), 96)
        let rendered = RenderBridge.renderElements(
            fromJSON: """
            [{"type":"voidBlock","nodeType":"counter","docPos":1}]
            """,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label,
            theme: nil,
            atomConfiguration: editor.textView.atomRenderConfiguration
        )
        let attachment = try XCTUnwrap(
            rendered.attribute(.attachment, at: 0, effectiveRange: nil) as? AtomBlockAttachment
        )

        XCTAssertEqual(attachment.reservedHeight, 96)
        editor.textView.attributedText = rendered
        XCTAssertTrue(editor.unmountAtomChild(child))
        XCTAssertEqual(attachment.reservedHeight, 40)
    }

    func testAtomContainerUsesAttachmentLayoutRectIncludingTextContainerInset() throws {
        let editor = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let attachment = installAtom(key: "counter:1", height: 72, in: editor)
        let child = atomChild(key: "counter:1", height: 72)
        editor.mountAtomChild(child, atomKey: "counter:1")
        editor.layoutIfNeeded()

        let characterRange = NSRange(location: 0, length: 1)
        editor.textView.layoutManager.ensureLayout(for: editor.textView.textContainer)
        let glyphRange = editor.textView.layoutManager.glyphRange(
            forCharacterRange: characterRange,
            actualCharacterRange: nil
        )
        let attachmentRect = editor.textView.layoutManager.boundingRect(
            forGlyphRange: glyphRange,
            in: editor.textView.textContainer
        )
        let padding = editor.textView.textContainer.lineFragmentPadding
        let expected = CGRect(
            x: editor.textView.textContainerInset.left + padding,
            y: editor.textView.textContainerInset.top + attachmentRect.minY,
            width: editor.textView.textContainer.size.width - (padding * 2),
            height: attachment.reservedHeight
        )

        XCTAssertEqual(try XCTUnwrap(editor.atomHostContainer(for: "counter:1")).frame, expected)
    }

    func testAtomContainerPreservesRenderedParagraphSpacing() throws {
        let editor = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        let value = RenderBridge.renderElements(
            fromJSON: """
            [
                {"type":"blockStart","nodeType":"paragraph","depth":0},
                {"type":"textRun","text":"Above","marks":[]},
                {"type":"blockEnd"},
                {"type":"voidBlock","nodeType":"counterCard","docPos":7}
            ]
            """,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label,
            theme: EditorTheme(dictionary: [
                "paragraph": ["spacingAfter": 18],
            ]),
            atomConfiguration: AtomRenderConfiguration(
                registeredNodeTypes: ["counterCard"],
                estimatedHeights: ["counterCard": 72],
                measuredHeights: [:]
            )
        )
        editor.textView.textStorage.setAttributedString(value)
        let attachmentRange = (value.string as NSString).range(of: "\u{FFFC}")
        let attachment = try XCTUnwrap(
            value.attribute(.attachment, at: attachmentRange.location, effectiveRange: nil)
                as? AtomBlockAttachment
        )
        let child = atomChild(key: attachment.atomKey, height: 72)
        editor.mountAtomChild(child, atomKey: attachment.atomKey)
        editor.layoutIfNeeded()

        let layoutManager = editor.textView.layoutManager
        layoutManager.ensureLayout(for: editor.textView.textContainer)
        let precedingGlyph = layoutManager.glyphIndexForCharacter(at: 0)
        let precedingLineRect = layoutManager.lineFragmentUsedRect(
            forGlyphAt: precedingGlyph,
            effectiveRange: nil
        )
        let precedingLineBottom = editor.textView.textContainerInset.top
            + precedingLineRect.maxY
        let atomTop = try XCTUnwrap(editor.atomHostContainer(for: attachment.atomKey)).frame.minY

        XCTAssertEqual(atomTop - precedingLineBottom, 18, accuracy: 0.5)
    }

    func testAtomContainerUsesAtomThemeSpacingBeforeFollowingParagraph() throws {
        let editor = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 320))
        let value = RenderBridge.renderElements(
            fromJSON: """
            [
                {"type":"voidBlock","nodeType":"counterCard","docPos":1},
                {"type":"blockStart","nodeType":"paragraph","depth":0},
                {"type":"textRun","text":"Below","marks":[]},
                {"type":"blockEnd"}
            ]
            """,
            baseFont: .systemFont(ofSize: 16),
            textColor: .label,
            theme: EditorTheme(dictionary: [
                "text": ["spacingAfter": 11],
                "paragraph": ["spacingAfter": 29],
            ]),
            atomConfiguration: AtomRenderConfiguration(
                registeredNodeTypes: ["counterCard"],
                estimatedHeights: ["counterCard": 72],
                measuredHeights: [:]
            )
        )
        editor.textView.textStorage.setAttributedString(value)
        let attachmentRange = (value.string as NSString).range(of: "\u{FFFC}")
        let attachment = try XCTUnwrap(
            value.attribute(.attachment, at: attachmentRange.location, effectiveRange: nil)
                as? AtomBlockAttachment
        )
        editor.mountAtomChild(
            atomChild(key: attachment.atomKey, height: 72),
            atomKey: attachment.atomKey
        )
        editor.layoutIfNeeded()

        let layoutManager = editor.textView.layoutManager
        layoutManager.ensureLayout(for: editor.textView.textContainer)
        let atomBottom = try XCTUnwrap(
            editor.atomHostContainer(for: attachment.atomKey)
        ).frame.maxY
        let followingCharacter = (value.string as NSString).range(of: "Below").location
        let followingGlyph = layoutManager.glyphIndexForCharacter(at: followingCharacter)
        let followingLineRect = layoutManager.lineFragmentUsedRect(
            forGlyphAt: followingGlyph,
            effectiveRange: nil
        )
        let followingLineTop = editor.textView.textContainerInset.top + followingLineRect.minY

        XCTAssertEqual(followingLineTop - atomBottom, 11, accuracy: 0.5)
    }

    func testUnmountClearsHostAndMeasuredHeight() {
        let editor = NativeEditorExpoView()
        editor.frame = CGRect(x: 0, y: 0, width: 320, height: 240)
        let attachment = installAtom(key: "counter:1", height: 40, in: editor.richTextView)
        let child = atomChild(key: "counter:1", height: 90)
        editor.mountChildComponentView(child, index: 0)

        editor.unmountChildComponentView(child, index: 0)

        XCTAssertNil(editor.richTextView.atomHostContainer(for: "counter:1"))
        XCTAssertNil(editor.richTextView.measuredAtomHeight(for: "counter:1"))
        XCTAssertNil(child.superview)
        XCTAssertEqual(attachment.reservedHeight, 40)
    }

    func testUnmountWithoutEstimateKeepsMeasuredHeightWhenNoFallbackExists() {
        let editor = NativeEditorExpoView()
        editor.frame = CGRect(x: 0, y: 0, width: 320, height: 240)
        let attachment = installAtom(key: "counter:1", height: 0, in: editor.richTextView)
        XCTAssertTrue(editor.richTextView.applyAtomRenderConfiguration(
            AtomRenderConfiguration(
                registeredNodeTypes: ["counter"],
                estimatedHeights: [:],
                measuredHeights: [:]
            )
        ))
        let child = atomChild(key: "counter:1", height: 240)
        editor.mountChildComponentView(child, index: 0)
        XCTAssertEqual(attachment.reservedHeight, 240)

        editor.unmountChildComponentView(child, index: 0)

        XCTAssertEqual(attachment.reservedHeight, 240)
    }

    func testAtomContentWidthEventOnlyFiresWhenWidthChanges() {
        let editor = RichTextEditorView(frame: CGRect(x: 0, y: 0, width: 320, height: 240))
        var widths: [CGFloat] = []
        editor.onAtomContentWidthChange = { widths.append($0) }

        editor.layoutIfNeeded()
        editor.layoutIfNeeded()
        editor.frame.size.width = 400
        editor.layoutIfNeeded()

        XCTAssertEqual(widths.count, 2)
        XCTAssertNotEqual(widths[0], widths[1])
    }

    @discardableResult
    private func installAtom(
        key: String,
        height: CGFloat,
        in editor: RichTextEditorView
    ) -> AtomBlockAttachment {
        let attachment = AtomBlockAttachment(
            atomKey: key,
            nodeType: "counter",
            docPos: 1,
            reservedHeight: height
        )
        installAtoms([attachment], in: editor)
        return attachment
    }

    private func installAtoms(
        _ attachments: [AtomBlockAttachment],
        in editor: RichTextEditorView
    ) {
        let value = NSMutableAttributedString()
        for attachment in attachments {
            value.append(NSAttributedString(attachment: attachment))
        }
        editor.textView.textStorage.setAttributedString(value)
        editor.setNeedsLayout()
    }

    private func atomChild(key: String, height: CGFloat) -> UIView {
        let child = UIView(frame: CGRect(x: 0, y: 0, width: 100, height: height))
        child.accessibilityIdentifier = "prose-atom:\(key)"
        return child
    }
}
