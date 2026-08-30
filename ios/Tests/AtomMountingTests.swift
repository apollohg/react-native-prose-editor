import XCTest

final class AtomMountingTests: XCTestCase {
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

    func testAtomContainerUsesAttachmentLayoutRect() throws {
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
            y: attachmentRect.minY,
            width: editor.textView.textContainer.size.width - (padding * 2),
            height: attachment.reservedHeight
        )

        XCTAssertEqual(try XCTUnwrap(editor.atomHostContainer(for: "counter:1")).frame, expected)
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
