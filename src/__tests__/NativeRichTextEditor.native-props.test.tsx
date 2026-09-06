import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeModule,
    v2Runtime,
    V2_INITIAL_DOC,
    V2_SERVER_DOC,
    V2_SERVER_UPDATE_DOC,
    createV2RoomHandle,
    createV2LocalHandle,
    setupV2Controller,
} from './helpers/NativeRichTextEditorFixture';
import { createRef } from 'react';
import { Platform } from 'react-native';
import { render, act } from '@testing-library/react-native';
import { NativeRichTextEditor, type NativeRichTextEditorRef } from '../NativeRichTextEditor';

import { V2_FAKE_STEP2_FRAME, V2_FAKE_UPDATE_FRAME } from './helpers/nativeEditorV2Fake';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('renders nothing for a room editor without snapshot until an accepted server Step 2, and emits no client state', () => {
        const handle = createV2RoomHandle();
        const { controller } = setupV2Controller(handle);
        const ref = createRef<NativeRichTextEditorRef>();
        const { queryByTestId, rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
            />
        );

        // Loading: the engine is AwaitRemote, so nothing renders.
        expect(queryByTestId('native-editor-view')).toBeNull();

        act(() => {
            controller.connect();
        });
        act(() => {
            v2Runtime.transportOpen(handle.editorId);
        });
        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
            />
        );
        // Handshaking is not synchronized: still no client-side document.
        expect(queryByTestId('native-editor-view')).toBeNull();
        expect(controller.state.status).toBe('handshaking');

        act(() => {
            v2Runtime.pushRemoteDoc(handle.editorId, V2_SERVER_DOC);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
            />
        );

        // The first render comes only from the accepted server Step 2.
        expect(queryByTestId('native-editor-view')).not.toBeNull();
        expect(controller.state.status).toBe('synchronized');
        expect(ref.current!.getContentJson()).toEqual(V2_SERVER_DOC);

        // Server-owned room init: the editor emitted no client state at
        // any point (no local mutations, no seeding).
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ReplaceDocument).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
        handle.destroy();
    });

    it('re-renders remote commits from the shared engine with no TypeScript document push (no callback reset loop)', () => {
        const handle = createV2RoomHandle({ withSnapshot: true });
        const { controller } = setupV2Controller(handle);
        const ref = createRef<NativeRichTextEditorRef>();
        const onContentChangeJSON = jest.fn();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
                onContentChangeJSON={onContentChangeJSON}
            />
        );
        expect(ref.current!.getContentJson()).toEqual(V2_INITIAL_DOC);

        act(() => {
            controller.connect();
        });
        act(() => {
            v2Runtime.transportOpen(handle.editorId);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        expect(controller.state.status).toBe('synchronized');
        mockNativeModule.editorV2ApplyLocalApi.mockClear();
        mockNativeModule.editorV2ReplaceDocument.mockClear();
        mockNativeModule.editorV2ApplyInput.mockClear();

        act(() => {
            v2Runtime.pushRemoteDoc(handle.editorId, V2_SERVER_UPDATE_DOC);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_UPDATE_FRAME);
        });
        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
                onContentChangeJSON={onContentChangeJSON}
            />
        );

        // Document updates flow through the returned Rust state only.
        expect(ref.current!.getContentJson()).toEqual(V2_SERVER_UPDATE_DOC);
        expect(controller.state.documentJson).toEqual(V2_SERVER_UPDATE_DOC);
        expect(onContentChangeJSON).toHaveBeenCalledWith(V2_SERVER_UPDATE_DOC);
        // No full-document reset (or any local mutation) was pushed back
        // into the engine from TypeScript.
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ReplaceDocument).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
        handle.destroy();
    });

    it('still serializes remoteSelections for the native view in v2 mode', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const remoteSelections = [
            {
                clientId: '42',
                anchor: 4,
                head: 9,
                color: '#00f',
                name: 'Bob',
                isFocused: true,
            },
        ];
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} remoteSelections={remoteSelections} />
        );
        expect(getByTestId('native-editor-view').props.remoteSelectionsJson).toBe(
            JSON.stringify(remoteSelections)
        );
        handle.destroy();
    });

    it('preserves buttonStyle when link items are mapped to native actions', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const buttonStyle = {
            iconSize: 22,
            color: '#111111',
            backgroundColor: '#121212',
            activeColor: '#222222',
            disabledColor: '#333333',
            activeBackgroundColor: '#444444',
            disabledBackgroundColor: '#555555',
            borderRadius: 9,
        };
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                toolbarItems={[
                    {
                        type: 'link',
                        label: 'Link',
                        icon: { type: 'default', id: 'link' },
                        buttonStyle,
                    },
                ]}
                onRequestLink={jest.fn()}
            />
        );

        const items = JSON.parse(getByTestId('native-editor-view').props.toolbarItemsJson);
        expect(items[0]).toMatchObject({
            type: 'action',
            key: '__native-editor-link__',
            buttonStyle,
        });
        handle.destroy();
    });

    it('serializes Android input options for the native view', () => {
        const platformSpy = jest.replaceProperty(Platform, 'OS', 'android');
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        try {
            const { getByTestId, rerender } = render(
                <NativeRichTextEditor
                    documentHandle={handle}
                    androidInputOptions={{ privateImeOptions: 'nm' }}
                />
            );

            expect(getByTestId('native-editor-view').props.androidInputOptionsJson).toBe(
                JSON.stringify({ privateImeOptions: 'nm' })
            );

            rerender(
                <NativeRichTextEditor
                    documentHandle={handle}
                    androidInputOptions={{ privateImeOptions: 'com.example.option' }}
                />
            );
            expect(getByTestId('native-editor-view').props.androidInputOptionsJson).toBe(
                JSON.stringify({ privateImeOptions: 'com.example.option' })
            );

            rerender(<NativeRichTextEditor documentHandle={handle} />);
            expect(getByTestId('native-editor-view').props.androidInputOptionsJson).toBeUndefined();
        } finally {
            platformSpy.restore();
            handle.destroy();
        }
    });

    it('does not forward Android input options on iOS', () => {
        const platformSpy = jest.replaceProperty(Platform, 'OS', 'ios');
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        try {
            const { getByTestId } = render(
                <NativeRichTextEditor
                    documentHandle={handle}
                    androidInputOptions={{ privateImeOptions: 'nm' }}
                />
            );

            expect(getByTestId('native-editor-view').props).not.toHaveProperty(
                'androidInputOptionsJson'
            );
        } finally {
            platformSpy.restore();
            handle.destroy();
        }
    });
});
