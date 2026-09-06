import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeFocus,
    mockNativeBlur,
    V2_INITIAL_DOC,
    createV2LocalHandle,
} from './helpers/NativeRichTextEditorFixture';
import { createRef } from 'react';
import { View } from 'react-native';
import { render, act } from '@testing-library/react-native';
import { NativeRichTextEditor, type NativeRichTextEditorRef } from '../NativeRichTextEditor';

import { NativeEditorV2BoundaryError } from '../NativeEditorBoundaryError';
import * as EditorUpdateRevision from '../EditorUpdateRevision';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('applies a JS-driven render snapshot locally without parsing its native handoff JSON', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onActiveStateChange = jest.fn();
        const snapshot = Object.freeze({
            renderBlocks: Object.freeze([]),
            renderPatch: null,
            selection: Object.freeze({ type: 'text' as const, anchor: 1, head: 1 }),
            activeState: Object.freeze({
                marks: Object.freeze({ bold: true }),
                markAttrs: Object.freeze({}),
                nodes: Object.freeze({}),
                commands: Object.freeze({}),
                allowedMarks: Object.freeze([]),
                insertableNodes: Object.freeze([]),
            }),
            historyState: Object.freeze({ canUndo: true, canRedo: false }),
            documentVersion: '42',
            stateRevision: '7',
            scalarLength: 5,
            documentIsEmpty: false,
        }) as ReturnType<typeof handle.bridge.renderUpdate>;
        const snapshotJson = JSON.stringify(snapshot);
        const renderUpdate = jest.spyOn(handle.bridge, 'renderUpdate').mockReturnValue(snapshot);
        const parse = jest.spyOn(JSON, 'parse');
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onActiveStateChange={onActiveStateChange}
            />
        );

        act(() => {
            ref.current!.toggleMark('bold');
        });

        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBe(snapshotJson);
        expect(onActiveStateChange.mock.calls.at(-1)![0]).toBe(snapshot.activeState);
        expect(parse).not.toHaveBeenCalledWith(snapshotJson);
        parse.mockRestore();
        renderUpdate.mockRestore();
        handle.destroy();
    });

    it('maps astral scalars, block and atom extents, and selected active state through the fake snapshot', () => {
        const handle = createV2LocalHandle({
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [
                        {
                            type: 'text',
                            text: 'A😀',
                            marks: [{ type: 'bold' }],
                        },
                    ],
                },
                {
                    type: 'paragraph',
                    content: [{ type: 'text', text: 'I', marks: [{ type: 'italic' }] }],
                },
                { type: 'image', attrs: { src: 'https://example.test/image.png' } },
                {
                    type: 'paragraph',
                    content: [{ type: 'text', text: 'Z' }],
                },
            ],
        });

        handle.bridge.setSelection({
            baseDocumentRevision: '1',
            selection: {
                type: 'text',
                anchor: { offset: 3, kind: 'scalar' },
                head: { offset: 3, kind: 'scalar' },
            },
        });
        const authoritative = handle.bridge.renderUpdate();
        expect(authoritative.scalarLength).toBe(8);
        expect(authoritative.selection).toEqual({
            type: 'text',
            anchor: 5,
            head: 5,
            anchorScalar: 3,
            headScalar: 3,
        });
        expect(authoritative.activeState.marks).toEqual({ italic: true });

        const selections = [0, 2, 3, 4, 5, 6, 7, 8, 99].map(
            (scalar) => handle.bridge.renderUpdate({ anchor: scalar, head: scalar }).selection
        );
        expect(selections).toEqual([
            { type: 'text', anchor: 1, head: 1, anchorScalar: 0, headScalar: 0 },
            { type: 'text', anchor: 3, head: 3, anchorScalar: 2, headScalar: 2 },
            { type: 'text', anchor: 5, head: 5, anchorScalar: 3, headScalar: 3 },
            { type: 'text', anchor: 6, head: 6, anchorScalar: 4, headScalar: 4 },
            { type: 'text', anchor: 7, head: 7, anchorScalar: 5, headScalar: 5 },
            { type: 'text', anchor: 8, head: 8, anchorScalar: 6, headScalar: 6 },
            { type: 'text', anchor: 9, head: 9, anchorScalar: 7, headScalar: 7 },
            { type: 'text', anchor: 10, head: 10, anchorScalar: 8, headScalar: 8 },
            { type: 'text', anchor: 10, head: 10, anchorScalar: 8, headScalar: 8 },
        ]);

        const astral = handle.bridge.renderUpdate({ anchor: 1, head: 1 });
        expect(astral.selection).toEqual({
            type: 'text',
            anchor: 2,
            head: 2,
            anchorScalar: 1,
            headScalar: 1,
        });
        expect(astral.activeState.marks).toEqual({ bold: true });

        const atom = handle.bridge.renderUpdate({ anchor: 5, head: 5 });
        expect(atom.selection).toEqual({
            type: 'text',
            anchor: 7,
            head: 7,
            anchorScalar: 5,
            headScalar: 5,
        });
        expect(atom.activeState.marks).toEqual({});
        handle.destroy();
    });

    it('maps empty placeholders plus supported inline and block atoms through the fake snapshot', () => {
        const handle = createV2LocalHandle({
            type: 'doc',
            content: [
                { type: 'paragraph' },
                {
                    type: 'paragraph',
                    content: [
                        { type: 'hardBreak' },
                        {
                            type: 'mention',
                            atom: true,
                            attrs: { label: 'Ada', mentionSuggestionChar: '@' },
                        },
                    ],
                },
                { type: 'callout', atom: true, attrs: { label: 'X' } },
            ],
        });

        expect(handle.bridge.renderUpdate().scalarLength).toBe(11);
        expect(
            [0, 1, 2, 3, 6, 7, 8, 9, 11, 99].map(
                (scalar) => handle.bridge.renderUpdate({ anchor: scalar, head: scalar }).selection
            )
        ).toEqual([
            { type: 'text', anchor: 1, head: 1, anchorScalar: 1, headScalar: 1 },
            { type: 'text', anchor: 1, head: 1, anchorScalar: 1, headScalar: 1 },
            { type: 'text', anchor: 3, head: 3, anchorScalar: 2, headScalar: 2 },
            { type: 'text', anchor: 4, head: 4, anchorScalar: 3, headScalar: 3 },
            { type: 'text', anchor: 4, head: 4, anchorScalar: 3, headScalar: 3 },
            { type: 'text', anchor: 5, head: 5, anchorScalar: 7, headScalar: 7 },
            { type: 'text', anchor: 6, head: 6, anchorScalar: 8, headScalar: 8 },
            { type: 'text', anchor: 6, head: 6, anchorScalar: 8, headScalar: 8 },
            { type: 'text', anchor: 7, head: 7, anchorScalar: 11, headScalar: 11 },
            { type: 'text', anchor: 7, head: 7, anchorScalar: 11, headScalar: 11 },
        ]);
        handle.destroy();
    });

    it('suppresses the view update after the editor update revision is exhausted', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onActiveStateChange = jest.fn();
        const errors: unknown[] = [];
        const originalAllocateEditorUpdateRevision =
            EditorUpdateRevision.allocateEditorUpdateRevision;
        const allocateEditorUpdateRevision = jest
            .spyOn(EditorUpdateRevision, 'allocateEditorUpdateRevision')
            .mockImplementation((currentRevision) =>
                currentRevision === 0
                    ? { revision: 0xffff_ffff }
                    : originalAllocateEditorUpdateRevision(currentRevision)
            );
        const renderUpdate = jest.spyOn(handle.bridge, 'renderUpdate');
        handle.addErrorListener((error) => errors.push(error));
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onActiveStateChange={onActiveStateChange}
            />
        );

        act(() => {
            ref.current!.toggleMark('bold');
        });
        const view = getByTestId('native-editor-view');
        const pushedUpdateJson = view.props.editorUpdateJson;
        const pushedUpdateRevision = view.props.editorUpdateRevision;
        expect(pushedUpdateRevision).toBe(0xffff_ffff);

        renderUpdate.mockClear();
        onActiveStateChange.mockClear();
        act(() => {
            ref.current!.toggleMark('bold');
        });

        expect(renderUpdate).not.toHaveBeenCalled();
        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBe(pushedUpdateJson);
        expect(getByTestId('native-editor-view').props.editorUpdateRevision).toBe(
            pushedUpdateRevision
        );
        expect(onActiveStateChange).not.toHaveBeenCalled();
        expect(errors.at(-1)).toBeInstanceOf(NativeEditorV2BoundaryError);
        expect((errors.at(-1) as NativeEditorV2BoundaryError).code).toBe('CONFIG_INVALID');
        allocateEditorUpdateRevision.mockRestore();
        handle.destroy();
    });

    it('renders the inline JS toolbar only in inline placement', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { queryByTestId, rerender } = render(
            <NativeRichTextEditor documentHandle={handle} toolbarPlacement='keyboard' />
        );
        expect(queryByTestId('native-editor-js-toolbar')).toBeNull();
        rerender(<NativeRichTextEditor documentHandle={handle} toolbarPlacement='inline' />);
        expect(queryByTestId('native-editor-js-toolbar')).not.toBeNull();
        handle.destroy();
    });

    it('forwards focused toolbar frames to native without racing a refocus after blur', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const onBlur = jest.fn();
        const sendRef = createRef<View>();
        const viewPrototype = (
            View as unknown as {
                prototype: {
                    measureInWindow: (
                        callback: (x: number, y: number, width: number, height: number) => void
                    ) => void;
                };
            }
        ).prototype;
        const measureInWindow = jest
            .spyOn(viewPrototype, 'measureInWindow')
            .mockImplementation(function (callback) {
                const testID = (this as unknown as { props?: { testID?: string } }).props?.testID;
                if (testID === 'editor-toolbar-root') {
                    callback(12, 24, 320, 48);
                }
                if (testID === 'send') {
                    callback(300, 700, 44, 44);
                }
            });

        try {
            const { getByTestId } = render(
                <>
                    <NativeRichTextEditor
                        documentHandle={handle}
                        toolbarPlacement='inline'
                        focusPreservingRefs={sendRef}
                        onBlur={onBlur}
                    />
                    <View ref={sendRef} testID='send' />
                </>
            );
            const nativeView = getByTestId('native-editor-view');

            act(() => {
                nativeView.props.onFocusChange({
                    nativeEvent: { isFocused: true, editorId: handle.editorId },
                });
                jest.runOnlyPendingTimers();
            });

            expect(JSON.parse(getByTestId('native-editor-view').props.toolbarFrameJson)).toEqual({
                frames: [
                    { x: 12, y: 24, width: 320, height: 48 },
                    { x: 300, y: 700, width: 44, height: 44 },
                ],
            });
            expect(mockNativeFocus).not.toHaveBeenCalled();

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: false, editorId: handle.editorId },
                });
            });

            expect(getByTestId('native-editor-view').props.toolbarFrameJson).toBeUndefined();
            expect(onBlur).toHaveBeenCalledTimes(1);
            expect(mockNativeFocus).not.toHaveBeenCalled();
        } finally {
            measureInWindow.mockRestore();
            handle.destroy();
        }
    });

    it('forwards single and multiple focus-preserving element frames only while focused', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const sendRef = createRef<View>();
        const attachmentRef = createRef<View>();
        const viewPrototype = (
            View as unknown as {
                prototype: {
                    measureInWindow: (
                        callback: (x: number, y: number, width: number, height: number) => void
                    ) => void;
                };
            }
        ).prototype;
        const measureInWindow = jest
            .spyOn(viewPrototype, 'measureInWindow')
            .mockImplementation(function (callback) {
                const testID = (this as unknown as { props?: { testID?: string } }).props?.testID;
                if (testID === 'send') {
                    callback(300, 700, 44, 44);
                }
                if (testID === 'attachment') {
                    callback(244, 700, 44, 44);
                }
            });

        try {
            const { getByTestId, rerender } = render(
                <>
                    <NativeRichTextEditor documentHandle={handle} focusPreservingRefs={sendRef} />
                    <View ref={sendRef} testID='send' />
                    <View ref={attachmentRef} testID='attachment' />
                </>
            );

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: true, editorId: handle.editorId },
                });
            });

            expect(JSON.parse(getByTestId('native-editor-view').props.toolbarFrameJson)).toEqual({
                x: 300,
                y: 700,
                width: 44,
                height: 44,
            });
            expect(mockNativeFocus).not.toHaveBeenCalled();
            expect(mockNativeBlur).not.toHaveBeenCalled();

            rerender(
                <>
                    <NativeRichTextEditor
                        documentHandle={handle}
                        focusPreservingRefs={[sendRef, attachmentRef]}
                    />
                    <View ref={sendRef} testID='send' />
                    <View ref={attachmentRef} testID='attachment' />
                </>
            );

            expect(JSON.parse(getByTestId('native-editor-view').props.toolbarFrameJson)).toEqual({
                frames: [
                    { x: 300, y: 700, width: 44, height: 44 },
                    { x: 244, y: 700, width: 44, height: 44 },
                ],
            });

            act(() => {
                getByTestId('native-editor-view').props.onFocusChange({
                    nativeEvent: { isFocused: false, editorId: handle.editorId },
                });
            });

            expect(getByTestId('native-editor-view').props.toolbarFrameJson).toBeUndefined();
        } finally {
            measureInWindow.mockRestore();
            handle.destroy();
        }
    });
});
