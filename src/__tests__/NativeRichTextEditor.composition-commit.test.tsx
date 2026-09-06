import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeGetCaretRect,
    mockNativeBeginExternalComposition,
    mockNativeCancelExternalComposition,
    mockNativeModule,
    deferred,
    v2Runtime,
    V2_INITIAL_DOC,
    V2_DOC_B,
    createV2LocalHandle,
    renderUpdateValue,
} from './helpers/NativeRichTextEditorFixture';
import { createRef } from 'react';

import { render, act } from '@testing-library/react-native';
import { NativeRichTextEditor, type NativeRichTextEditorRef } from '../NativeRichTextEditor';

import { NativeEditorV2LifecycleError } from '../NativeEditorBoundaryError';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('waits for composition cancellation before a controlled value reset is pushed', async () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const cancellation = deferred<string>();
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='reset'
            />
        );
        await ref.current!.beginExternalTextComposition();
        const sessionId = mockNativeBeginExternalComposition.mock.calls.at(-1)![0];
        mockNativeCancelExternalComposition.mockReturnValueOnce(cancellation.promise);
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_DOC_B}
                valueJSONUpdateMode='reset'
            />
        );

        expect(mockNativeCancelExternalComposition).toHaveBeenCalledWith(
            sessionId,
            'documentChange'
        );
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();

        await act(async () => {
            cancellation.resolve(
                JSON.stringify({
                    version: 1,
                    type: 'ended',
                    sessionId,
                    outcome: 'cancelled',
                    cause: 'documentChange',
                    text: '',
                })
            );
            await cancellation.promise;
        });

        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        expect(mockNativeCancelExternalComposition.mock.invocationCallOrder.at(-1)).toBeLessThan(
            mockNativeModule.editorV2ApplyLocalApi.mock.invocationCallOrder[0]
        );
        handle.destroy();
    });

    it('does not reset after cancellation rejection and emits the typed failure', async () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const received: unknown[] = [];
        const cancellationError = new NativeEditorV2LifecycleError({
            domain: 'lifecycle',
            code: 'EXTERNAL_COMPOSITION_CANCEL_FAILED',
            message: 'Could not cancel external composition',
            requestId: null,
            operationIndex: null,
            limit: null,
            actual: null,
            details: null,
        });
        handle.addErrorListener((error) => received.push(error));
        const { rerender } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_INITIAL_DOC}
                valueJSONUpdateMode='reset'
            />
        );
        await ref.current!.beginExternalTextComposition();
        mockNativeCancelExternalComposition.mockRejectedValueOnce(cancellationError);
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                valueJSON={V2_DOC_B}
                valueJSONUpdateMode='reset'
            />
        );
        await act(async () => Promise.resolve());

        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeCancelExternalComposition).toHaveBeenCalledTimes(1);
        expect(received).toHaveLength(1);
        expect(received[0]).toBeInstanceOf(NativeEditorV2LifecycleError);
        expect(received[0]).toMatchObject({ code: 'EXTERNAL_COMPOSITION_CANCEL_FAILED' });
        handle.destroy();
    });

    it('keeps provisional composition out of the document and uses the normal commit path once', async () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onContentChange = jest.fn();
        const onContentChangeJSON = jest.fn();
        const onLocalCommit = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onContentChange={onContentChange}
                onContentChangeJSON={onContentChangeJSON}
                onLocalCommit={onLocalCommit}
            />
        );
        const session = await ref.current!.beginExternalTextComposition();
        mockNativeModule.editorV2ApplyInput.mockClear();
        mockNativeModule.editorV2ApplyCommand.mockClear();
        mockNativeModule.editorV2ApplyLocalApi.mockClear();
        mockNativeModule.editorV2ReplaceDocument.mockClear();

        await session.update('provisional');

        expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyCommand).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ReplaceDocument).not.toHaveBeenCalled();
        expect(onContentChange).not.toHaveBeenCalled();
        expect(onContentChangeJSON).not.toHaveBeenCalled();
        expect(onLocalCommit).not.toHaveBeenCalled();

        v2Runtime.module.editorV2ApplyInput(
            handle.editorId,
            JSON.stringify({
                version: 1,
                requestId: '1',
                baseDocumentRevision: handle.bridge.getState().documentRevision,
                text: '!',
            })
        );
        act(() => {
            getByTestId('native-editor-view').props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            });
        });

        expect(onLocalCommit).toHaveBeenCalledTimes(1);
        expect(onContentChange).toHaveBeenCalledTimes(1);
        expect(onContentChangeJSON).toHaveBeenCalledTimes(1);

        const secondSession = await ref.current!.beginExternalTextComposition();
        const secondSessionId = mockNativeBeginExternalComposition.mock.calls.at(-1)![0];
        act(() => {
            getByTestId('native-editor-view').props.onExternalTextCompositionEnd({
                nativeEvent: {
                    editorId: handle.editorId,
                    resultJson: JSON.stringify({
                        version: 1,
                        type: 'ended',
                        sessionId: secondSessionId,
                        outcome: 'cancelled',
                        cause: 'interaction',
                        text: '',
                    }),
                },
            });
        });
        await expect(secondSession.cancel()).resolves.toBeUndefined();
        expect(onLocalCommit).toHaveBeenCalledTimes(1);
        expect(onContentChange).toHaveBeenCalledTimes(1);
        expect(onContentChangeJSON).toHaveBeenCalledTimes(1);
        handle.destroy();
    });

    it('parses getCaretRect JSON from the native view', async () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);

        mockNativeGetCaretRect.mockReturnValue(
            JSON.stringify({ x: 1, y: 2, width: 3, height: 4, editorWidth: 100, editorHeight: 50 })
        );
        let rect: unknown;
        await act(async () => {
            rect = await ref.current!.getCaretRect();
        });
        expect(rect).toEqual({
            x: 1,
            y: 2,
            width: 3,
            height: 4,
            editorWidth: 100,
            editorHeight: 50,
        });

        mockNativeGetCaretRect.mockReturnValue(null);
        await act(async () => {
            rect = await ref.current!.getCaretRect();
        });
        expect(rect).toBeNull();

        mockNativeGetCaretRect.mockReturnValue('{"x":1}');
        await act(async () => {
            rect = await ref.current!.getCaretRect();
        });
        expect(rect).toBeNull();
        handle.destroy();
    });

    it('passes placeholder, accessibility props, and theme through to the native view', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                placeholder='Write something…'
                accessibilityLabel='Body'
                accessibilityHint='Message body editor'
                theme={{ paragraph: { textSize: 17 } }}
            />
        );
        const view = getByTestId('native-editor-view');
        expect(view.props.placeholder).toBe('Write something…');
        expect(view.props.accessibilityLabel).toBe('Body');
        expect(view.props.accessibilityHint).toBe('Message body editor');
        expect(JSON.parse(view.props.themeJson as string)).toEqual({
            paragraph: { textSize: 17 },
        });
        handle.destroy();
    });
});
