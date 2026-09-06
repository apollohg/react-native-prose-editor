import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeModule,
    v2Runtime,
    V2_INITIAL_DOC,
    V2_DOC_B,
    V2_DOC_C,
    V2_SERVER_DOC,
    createV2RoomHandle,
    createV2LocalHandle,
    setupV2Controller,
    renderUpdateValue,
} from './helpers/NativeRichTextEditorFixture';
import { createRef } from 'react';

import { render, act } from '@testing-library/react-native';
import {
    NativeRichTextEditor,
    type NativeRichTextEditorProps,
    type NativeRichTextEditorRef,
} from '../NativeRichTextEditor';

import { fakeDocForText, V2_FAKE_STEP2_FRAME } from './helpers/nativeEditorV2Fake';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('drives the toolbar from collapsed stored state without leaking it into explicit mirrors', () => {
        const handle = createV2LocalHandle({
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [{ type: 'text', text: 'bold', marks: [{ type: 'bold' }] }],
                },
            ],
        });
        const ref = createRef<NativeRichTextEditorRef>();
        const onActiveStateChange = jest.fn();
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
        let view = getByTestId('native-editor-view');
        expect(typeof view.props.editorUpdateJson).toBe('string');
        let pushed = JSON.parse(view.props.editorUpdateJson as string) as {
            activeState: { marks: Record<string, boolean> };
        };
        expect(pushed.activeState.marks.bold).toBe(false);
        expect(view.props.editorUpdateRevision).toBeGreaterThan(0);
        expect(view.props.editorUpdateEditorId).toBe(handle.editorId);
        expect(onActiveStateChange).toHaveBeenCalled();
        expect(
            (onActiveStateChange.mock.calls.at(-1)![0] as { marks: Record<string, boolean> }).marks
                .bold
        ).toBe(false);

        const mirror = handle.bridge.renderUpdate({ anchor: 0, head: 0 });
        expect(mirror.activeState).toMatchObject({
            marks: { bold: true },
            markAttrs: {},
            nodes: {},
        });

        act(() => {
            ref.current!.toggleMark('bold');
        });
        view = getByTestId('native-editor-view');
        pushed = JSON.parse(view.props.editorUpdateJson as string);
        expect(pushed.activeState.marks.bold).toBe(true);

        act(() => {
            ref.current!.setLink('https://example.test/stored');
            ref.current!.toggleHeading(2);
        });
        pushed = JSON.parse(getByTestId('native-editor-view').props.editorUpdateJson as string);
        expect(pushed.activeState).toMatchObject({
            marks: { bold: true, link: true },
            markAttrs: { link: { href: 'https://example.test/stored' } },
            nodes: { 'heading:2': true },
        });
        expect(handle.bridge.renderUpdate({ anchor: 0, head: 0 }).activeState).toMatchObject({
            marks: { bold: true },
            markAttrs: {},
            nodes: {},
        });
        handle.destroy();
    });

    it('keeps the focused native view mounted when rebinding to another ready handle', () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const ref = createRef<NativeRichTextEditorRef>();
        const onReboundFocus = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor ref={ref} documentHandle={handleA} />
        );

        const nativeView = getByTestId('native-editor-view');

        act(() => {
            nativeView.props.onFocusChange({
                nativeEvent: { editorId: handleA.editorId, isFocused: true },
            });
        });

        act(() => {
            ref.current!.toggleMark('bold');
            nativeView.props.onSelectionChange({
                nativeEvent: { editorId: handleA.editorId, anchor: 4, head: 4 },
            });
        });
        expect(getByTestId('native-editor-view').props.editorUpdateEditorId).toBe(handleA.editorId);

        rerender(
            <NativeRichTextEditor ref={ref} documentHandle={handleB} onFocus={onReboundFocus} />
        );

        const view = getByTestId('native-editor-view');
        expect(view).toBe(nativeView);
        expect(onReboundFocus).toHaveBeenCalledTimes(1);
        expect(view.props.editorId).toBe(handleB.editorId);
        const selectionRequest = JSON.parse(
            mockNativeModule.editorV2SetSelection.mock.calls.at(-1)![1] as string
        );
        expect(selectionRequest).toMatchObject({
            baseDocumentRevision: handleB.bridge.getState().documentRevision,
            selection: {
                type: 'text',
                anchor: { offset: 4, kind: 'scalar', affinity: 'after' },
                head: { offset: 4, kind: 'scalar', affinity: 'after' },
            },
        });
        expect(view.props.editorUpdateEditorId).toBe(handleB.editorId);
        expect(JSON.parse(view.props.editorUpdateJson as string).selection).toEqual({
            type: 'text',
            anchor: 5,
            anchorScalar: 4,
            head: 5,
            headScalar: 4,
        });
        handleA.destroy();
        handleB.destroy();
    });

    it('does not carry focus through a rebind while the room is awaiting remote state', () => {
        const localHandle = createV2LocalHandle(V2_INITIAL_DOC);
        const roomHandle = createV2RoomHandle();
        const { controller } = setupV2Controller(roomHandle);
        const onRoomFocus = jest.fn();
        const { getByTestId, queryByTestId, rerender } = render(
            <NativeRichTextEditor documentHandle={localHandle} />
        );

        act(() => {
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { editorId: localHandle.editorId, isFocused: true },
            });
        });
        rerender(<NativeRichTextEditor documentHandle={roomHandle} onFocus={onRoomFocus} />);
        expect(queryByTestId('native-editor-view')).toBeNull();

        act(() => {
            controller.connect();
            v2Runtime.transportOpen(roomHandle.editorId);
            v2Runtime.pushRemoteDoc(roomHandle.editorId, V2_SERVER_DOC);
            v2Runtime.transportReceive(roomHandle.editorId, V2_FAKE_STEP2_FRAME);
        });
        rerender(
            <NativeRichTextEditor
                documentHandle={roomHandle}
                documentRevision={controller.state.documentRevision}
                onFocus={onRoomFocus}
            />
        );

        expect(queryByTestId('native-editor-view')).not.toBeNull();
        expect(onRoomFocus).not.toHaveBeenCalled();
        localHandle.destroy();
        roomHandle.destroy();
    });

    it('clears A formatting state while carrying its focused selection into B', () => {
        const handleA = createV2LocalHandle(fakeDocForText('alpha'));
        const handleB = createV2LocalHandle({
            type: 'doc',
            content: [
                {
                    type: 'paragraph',
                    content: [{ type: 'text', text: 'beta', marks: [{ type: 'italic' }] }],
                },
            ],
        });
        const ref = createRef<NativeRichTextEditorRef>();
        const onFocus = jest.fn();
        const onRequestLink = jest.fn();
        const toolbarItems: NonNullable<NativeRichTextEditorProps['toolbarItems']> = [
            { type: 'mark', mark: 'bold', label: 'Bold', icon: 'bold' },
            { type: 'mark', mark: 'italic', label: 'Italic', icon: 'italic' },
        ];
        const { getByLabelText, getByTestId, rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handleA}
                toolbarItems={toolbarItems}
                toolbarPlacement='inline'
                onFocus={onFocus}
                onRequestLink={onRequestLink}
            />
        );

        act(() => {
            ref.current!.toggleMark('bold');
            getByTestId('native-editor-view').props.onSelectionChange({
                nativeEvent: {
                    anchor: 2,
                    head: 2,
                    editorId: handleA.editorId,
                },
            });
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handleA.editorId },
            });
        });
        expect(getByLabelText('Bold').props.accessibilityState.selected).toBe(true);
        expect(onFocus).toHaveBeenCalledTimes(1);

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handleB}
                toolbarItems={toolbarItems}
                toolbarPlacement='inline'
                onFocus={onFocus}
                onRequestLink={onRequestLink}
            />
        );

        const view = getByTestId('native-editor-view');
        expect(getByLabelText('Bold').props.accessibilityState.selected).toBe(false);
        act(() => {
            view.props.onToolbarAction({
                nativeEvent: { key: '__native-editor-link__', editorId: handleB.editorId },
            });
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handleB.editorId },
            });
        });
        expect(onRequestLink.mock.calls.at(-1)![0].selection).toEqual({
            type: 'text',
            anchor: 3,
            anchorScalar: 2,
            head: 3,
            headScalar: 2,
        });
        expect(onFocus).toHaveBeenCalledTimes(2);

        act(() => {
            view.props.onEditorUpdate({
                nativeEvent: {
                    editorId: handleB.editorId,
                    updateJson: renderUpdateValue(handleB.editorId, 2, 2),
                    documentRevision: handleB.bridge.getState().documentRevision,
                },
            });
        });

        expect(getByLabelText('Bold').props.accessibilityState.selected).toBe(false);
        expect(getByLabelText('Italic').props.accessibilityState.selected).toBe(true);
        act(() => ref.current!.toggleMark('bold'));
        const request = JSON.parse(
            mockNativeModule.editorV2ApplyCommand.mock.calls.at(-1)![1] as string
        ) as { baseDocumentRevision: string };
        expect(request.baseDocumentRevision).toBe(handleB.bridge.getState().documentRevision);
        handleA.destroy();
        handleB.destroy();
    });

    it('treats B revisions as new observations after rebinding from A', () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const ref = createRef<NativeRichTextEditorRef>();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor ref={ref} documentHandle={handleA} />
        );

        const nativeCommit = JSON.stringify({
            version: 1,
            requestId: '1',
            baseDocumentRevision: handleA.bridge.getState().documentRevision,
            text: '!',
        });
        act(() => {
            v2Runtime.module.editorV2ApplyInput(handleA.editorId, nativeCommit);
            getByTestId('native-editor-view').props.onEditorUpdate({
                nativeEvent: {
                    editorId: handleA.editorId,
                    updateJson: renderUpdateValue(handleA.editorId),
                    documentRevision: handleA.bridge.getState().documentRevision,
                },
            });
        });
        const nativeRevision = handleA.bridge.getState().documentRevision;

        rerender(<NativeRichTextEditor ref={ref} documentHandle={handleB} />);

        // B's initial revision is pulled natively, rather than being treated
        // as an external update under A's observation state.
        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();

        act(() => {
            handleB.bridge.replaceDocument({ setJson: V2_DOC_C, history: 'undoableBoundary' });
        });
        const externalRevision = handleB.bridge.getState().documentRevision;
        expect(externalRevision).toBe(nativeRevision);

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handleB}
                documentRevision={externalRevision}
            />
        );

        const view = getByTestId('native-editor-view');
        expect(view.props.editorUpdateEditorId).toBe(handleB.editorId);
        expect(JSON.parse(view.props.editorUpdateJson as string)).toMatchObject({
            documentVersion: externalRevision,
        });
        handleA.destroy();
        handleB.destroy();
    });

    it('discards a re-entrant A render entirely after the same editor component rebinds to B', () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const ref = createRef<NativeRichTextEditorRef>();
        const onActiveStateChange = jest.fn();
        const onRequestLink = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handleA}
                onActiveStateChange={onActiveStateChange}
                onRequestLink={onRequestLink}
            />
        );
        act(() => {});
        const renderAUpdate = handleA.bridge.renderUpdate.bind(handleA.bridge);
        const renderUpdate = jest.spyOn(handleA.bridge, 'renderUpdate').mockImplementation(() => {
            act(() => {
                rerender(
                    <NativeRichTextEditor
                        ref={ref}
                        documentHandle={handleB}
                        onActiveStateChange={onActiveStateChange}
                        onRequestLink={onRequestLink}
                    />
                );
            });
            return renderAUpdate();
        });

        onActiveStateChange.mockClear();
        ref.current!.toggleMark('bold');

        const view = getByTestId('native-editor-view');
        expect(view.props.editorId).toBe(handleB.editorId);
        expect(view.props.editorUpdateJson).toBeUndefined();
        expect(view.props.editorUpdateEditorId).toBeUndefined();
        expect(renderUpdate).toHaveBeenCalledTimes(1);
        expect(onActiveStateChange).not.toHaveBeenCalled();

        act(() => {
            view.props.onToolbarAction({
                nativeEvent: { key: '__native-editor-link__', editorId: handleB.editorId },
            });
        });
        expect(onRequestLink).toHaveBeenCalledWith(
            expect.objectContaining({ selection: { type: 'text', anchor: 0, head: 0 } })
        );
        renderUpdate.mockRestore();
        handleA.destroy();
        handleB.destroy();
    });
});
