import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeModule,
    v2Runtime,
    V2_INITIAL_DOC,
    V2_DOC_B,
    V2_DOC_C,
    createV2RoomHandle,
    createV2LocalHandle,
    setupV2Controller,
} from './helpers/NativeRichTextEditorFixture';
import { createRef } from 'react';

import { render, act } from '@testing-library/react-native';
import { NativeRichTextEditor, type NativeRichTextEditorRef } from '../NativeRichTextEditor';

import {
    NativeEditorNonRetryableError,
    NativeEditorOperationError,
} from '../NativeEditorBoundaryError';

import { V2_FAKE_STEP2_FRAME } from './helpers/nativeEditorV2Fake';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('routes every typing/command ref method through the v2 bridge with the frozen payload', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);

        const lastCommand = (): Record<string, unknown> => {
            const calls = mockNativeModule.editorV2ApplyCommand.mock.calls;
            const request = JSON.parse(calls[calls.length - 1][1] as string) as Record<
                string,
                unknown
            >;
            return request.command as Record<string, unknown>;
        };
        const commandCount = () => mockNativeModule.editorV2ApplyCommand.mock.calls.length;

        act(() => ref.current!.toggleMark('bold'));
        expect(lastCommand()).toEqual({ type: 'toggleMark', markType: 'bold' });

        act(() => ref.current!.setLink('https://example.com'));
        expect(lastCommand()).toEqual({
            type: 'setMark',
            markType: 'link',
            attrs: { href: 'https://example.com' },
        });

        act(() => ref.current!.unsetLink());
        expect(lastCommand()).toEqual({ type: 'unsetMark', markType: 'link' });

        act(() => ref.current!.toggleBlockquote());
        expect(lastCommand()).toEqual({ type: 'toggleBlockquote' });

        act(() => ref.current!.toggleHeading(2));
        expect(lastCommand()).toEqual({ type: 'toggleHeading', level: 2 });

        act(() => ref.current!.toggleList('bulletList'));
        expect(lastCommand()).toEqual({
            type: 'wrapInList',
            listType: 'bulletList',
            itemType: 'listItem',
        });
        // The engine now reports the list active: toggling unwraps.
        act(() => ref.current!.toggleList('bulletList'));
        expect(lastCommand()).toEqual({ type: 'unwrapFromList' });

        act(() => ref.current!.toggleList('bullet_list'));
        expect(lastCommand()).toEqual({
            type: 'wrapInList',
            listType: 'bullet_list',
            itemType: 'list_item',
        });

        act(() => ref.current!.indentListItem());
        expect(lastCommand()).toEqual({ type: 'indentListItem' });

        act(() => ref.current!.outdentListItem());
        expect(lastCommand()).toEqual({ type: 'outdentListItem' });

        act(() => ref.current!.insertNode('horizontalRule'));
        expect(lastCommand()).toEqual({ type: 'insertNode', nodeType: 'horizontalRule' });

        act(() => ref.current!.insertImage('https://example.com/a.png'));
        expect(lastCommand()).toEqual({
            type: 'insertContentJson',
            json: {
                type: 'doc',
                content: [{ type: 'image', attrs: { src: 'https://example.com/a.png' } }],
            },
        });

        act(() => ref.current!.insertContentHtml('<p>hi</p>'));
        expect(lastCommand()).toEqual({ type: 'insertContentHtml', html: '<p>hi</p>' });

        const fragment = { type: 'doc', content: [{ type: 'paragraph' }] };
        act(() => ref.current!.insertContentJson(fragment));
        expect(lastCommand()).toEqual({ type: 'insertContentJson', json: fragment });

        // Every command carried the rendered engine revision as its base.
        const bases = mockNativeModule.editorV2ApplyCommand.mock.calls.map((call) =>
            String((JSON.parse(call[1] as string) as Record<string, unknown>).baseDocumentRevision)
        );
        expect(bases.every((base) => base !== 'undefined' && base !== 'null')).toBe(true);

        const commandsBefore = commandCount();
        act(() => ref.current!.insertText('abc'));
        expect(commandCount()).toBe(commandsBefore);
        const inputCalls = mockNativeModule.editorV2ApplyInput.mock.calls;
        const inputRequest = JSON.parse(inputCalls[inputCalls.length - 1][1] as string) as Record<
            string,
            unknown
        >;
        expect(inputRequest.text).toBe('abc');
        handle.destroy();
    });

    it('propagates structured error envelopes from ref commands', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        v2Runtime.injectNextApplyCommandError(handle.editorId, {
            domain: 'operation',
            code: 'POSITION_INVALID',
            message: 'selection is outside the document',
            requestId: null,
            operationIndex: null,
            limit: null,
            actual: null,
            details: null,
        });
        let thrown: unknown;
        act(() => {
            try {
                ref.current!.toggleMark('bold');
            } catch (error) {
                thrown = error;
            }
        });
        expect(thrown).toBeInstanceOf(NativeEditorOperationError);
        expect((thrown as NativeEditorOperationError).code).toBe('POSITION_INVALID');
        handle.destroy();
    });

    it('refreshes from the engine on REVISION_MISMATCH and never retries the command', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        mockNativeModule.editorV2ApplyInput.mockClear();

        // The engine advances behind the component's back (rev 1 -> 2).
        handle.bridge.replaceDocument({ setJson: V2_DOC_B, history: 'undoableBoundary' });

        act(() => {
            ref.current!.insertText('abc');
        });
        // Stale base rejected; the component refreshed and did not retry.
        expect(mockNativeModule.editorV2ApplyInput).toHaveBeenCalledTimes(1);
        expect(ref.current!.getContentJson()).toEqual(V2_DOC_B);
        handle.destroy();
    });

    it('rejects controlled valueJSON replace while the collaboration transport is connected', () => {
        const handle = createV2RoomHandle({ withSnapshot: true });
        const { controller } = setupV2Controller(handle);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='replace'
            />
        );
        act(() => {
            controller.connect();
        });
        act(() => {
            v2Runtime.transportOpen(handle.editorId);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        expect(controller.state.status).toBe('synchronized');

        let thrown: unknown;
        try {
            rerender(
                <NativeRichTextEditor
                    ref={ref}
                    documentHandle={handle}
                    documentRevision={controller.state.documentRevision}
                    valueJSON={V2_DOC_B}
                    valueJSONUpdateMode='replace'
                />
            );
        } catch (error) {
            thrown = error;
        }
        expect((thrown as { code?: string })?.code).toBe('WHOLE_DOCUMENT_REPLACEMENT_CONNECTED');
        handle.destroy();
    });

    it('rejects controlled valueJSON reset while the collaboration transport is connected', async () => {
        const handle = createV2RoomHandle({ withSnapshot: true });
        const { controller } = setupV2Controller(handle);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                documentRevision={controller.state.documentRevision}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='reset'
            />
        );
        act(() => {
            controller.connect();
        });
        act(() => {
            v2Runtime.transportOpen(handle.editorId);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        expect(controller.state.status).toBe('synchronized');

        let thrown: unknown;
        try {
            await act(async () => {
                rerender(
                    <NativeRichTextEditor
                        ref={ref}
                        documentHandle={handle}
                        documentRevision={controller.state.documentRevision}
                        valueJSON={V2_DOC_C}
                        valueJSONUpdateMode='reset'
                    />
                );
                await Promise.resolve();
            });
        } catch (error) {
            thrown = error;
        }
        expect((thrown as { code?: string })?.code).toBe('WHOLE_DOCUMENT_REPLACEMENT_CONNECTED');
        handle.destroy();
    });

    it('read-only mode blocks every mutation ref method but keeps selection and controlled content', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onSelectionChange = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                editable={false}
                onSelectionChange={onSelectionChange}
            />
        );

        const mutations: Array<() => void> = [
            () => ref.current!.toggleMark('bold'),
            () => ref.current!.setLink('https://example.com'),
            () => ref.current!.unsetLink(),
            () => ref.current!.toggleBlockquote(),
            () => ref.current!.toggleHeading(1),
            () => ref.current!.toggleList('bulletList'),
            () => ref.current!.indentListItem(),
            () => ref.current!.outdentListItem(),
            () => ref.current!.insertNode('horizontalRule'),
            () => ref.current!.insertImage('https://example.com/a.png'),
            () => ref.current!.insertText('x'),
            () => ref.current!.insertContentHtml('<p>x</p>'),
            () => ref.current!.insertContentJson({ type: 'doc', content: [] }),
        ];
        for (const mutation of mutations) {
            let thrown: unknown;
            act(() => {
                try {
                    mutation();
                } catch (error) {
                    thrown = error;
                }
            });
            expect(thrown).toBeInstanceOf(NativeEditorOperationError);
            expect((thrown as NativeEditorOperationError).code).toBe('MUTATION_REJECTED');
        }
        expect(mockNativeModule.editorV2ApplyCommand).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ReplaceDocument).not.toHaveBeenCalled();

        // Selection still flows.
        act(() => {
            getByTestId('native-editor-view').props.onSelectionChange({
                nativeEvent: { anchor: 0, head: 2, editorId: handle.editorId },
            });
        });
        expect(onSelectionChange).toHaveBeenCalledWith({ type: 'text', anchor: 0, head: 2 });

        // Controlled API content still passes under read-only (native parity).
        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                editable={false}
                valueJSON={V2_DOC_B}
            />
        );
        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        handle.destroy();
    });

    it('never destroys the shared handle on unmount and drops everything after destroy', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onContentChange = jest.fn();
        const { unmount, getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onContentChange={onContentChange}
            />
        );
        const view = getByTestId('native-editor-view');

        // Destroying is the consumer's call. While mounted, every surface
        // degrades to the structured destroyed contract.
        handle.destroy();
        expect(mockNativeModule.editorV2Destroy).toHaveBeenCalledTimes(1);
        expect(ref.current!.getContent()).toBe('');
        let thrown: unknown;
        try {
            ref.current!.toggleMark('bold');
        } catch (error) {
            thrown = error;
        }
        expect(thrown).toBeInstanceOf(NativeEditorNonRetryableError);
        expect((thrown as NativeEditorNonRetryableError).code).toBe('ENGINE_DESTROYED');

        // Events arriving for the destroyed session are dropped.
        onContentChange.mockClear();
        act(() => {
            view.props.onEditorUpdate({
                nativeEvent: { editorId: handle.editorId, updateJson: '{}' },
            });
        });
        expect(onContentChange).not.toHaveBeenCalled();

        // Unmount never destroys the shared handle a second time.
        unmount();
        expect(mockNativeModule.editorV2Destroy).toHaveBeenCalledTimes(1);
    });

    it('routes toolbar link/image actions through the request props and custom keys to onToolbarAction', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onRequestLink = jest.fn();
        const onRequestImage = jest.fn();
        const onToolbarAction = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onRequestLink={onRequestLink}
                onRequestImage={onRequestImage}
                onToolbarAction={onToolbarAction}
            />
        );

        act(() => {
            getByTestId('native-editor-view').props.onToolbarAction({
                nativeEvent: { key: '__native-editor-link__', editorId: handle.editorId },
            });
        });
        expect(onRequestLink).toHaveBeenCalledTimes(1);
        const linkContext = onRequestLink.mock.calls[0][0] as {
            isActive: boolean;
            setLink: (href: string) => void;
            unsetLink: () => void;
        };
        expect(linkContext.isActive).toBe(false);
        act(() => {
            linkContext.setLink('https://example.com');
        });
        let calls = mockNativeModule.editorV2ApplyCommand.mock.calls;
        expect(
            (JSON.parse(calls[calls.length - 1][1] as string) as Record<string, unknown>).command
        ).toEqual({
            type: 'setMark',
            markType: 'link',
            attrs: { href: 'https://example.com' },
        });

        act(() => {
            getByTestId('native-editor-view').props.onToolbarAction({
                nativeEvent: { key: '__native-editor-image__', editorId: handle.editorId },
            });
        });
        expect(onRequestImage).toHaveBeenCalledTimes(1);
        const imageContext = onRequestImage.mock.calls[0][0] as {
            insertImage: (src: string) => void;
        };
        act(() => {
            imageContext.insertImage('https://example.com/b.png');
        });
        calls = mockNativeModule.editorV2ApplyCommand.mock.calls;
        expect(
            (JSON.parse(calls[calls.length - 1][1] as string) as Record<string, unknown>).command
        ).toEqual({
            type: 'insertContentJson',
            json: {
                type: 'doc',
                content: [{ type: 'image', attrs: { src: 'https://example.com/b.png' } }],
            },
        });

        act(() => {
            getByTestId('native-editor-view').props.onToolbarAction({
                nativeEvent: {
                    key: 'action:custom:0',
                    editorId: handle.editorId,
                    documentRevision: '01',
                    updateJson: '{}',
                },
            });
        });
        expect(onToolbarAction).toHaveBeenCalledWith('action:custom:0');
        handle.destroy();
    });
});
