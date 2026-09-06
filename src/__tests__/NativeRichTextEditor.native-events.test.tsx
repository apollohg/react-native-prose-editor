import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeModule,
    v2Runtime,
    V2_INITIAL_DOC,
    V2_DOC_B,
    createV2LocalHandle,
    renderUpdateValue,
} from './helpers/NativeRichTextEditorFixture';
import { createRef } from 'react';

import { render, act } from '@testing-library/react-native';
import { NativeRichTextEditor, type NativeRichTextEditorRef } from '../NativeRichTextEditor';

import { NativeEditorNonRetryableError } from '../NativeEditorBoundaryError';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('binds the native view to the session editor id and is editable by default', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId } = render(<NativeRichTextEditor documentHandle={handle} />);
        const view = getByTestId('native-editor-view');
        expect(view.props.editorId).toBe(handle.editorId);
        expect(view.props.editable).toBe(true);
        expect(view.props.showToolbar).toBe(true);
        expect(view.props.autoFocus).toBe(false);
        expect(typeof view.props.onEditorUpdate).toBe('function');
        expect(typeof view.props.onEditorError).toBe('function');
        // The native side marked the session live for view binding at create.
        expect(v2Runtime.liveEditorIds()).toContain(handle.editorId);
        handle.destroy();
    });

    it.each(['iOS', 'Android'])(
        'routes each valid %s autonomous native error once through the bound handle',
        (platform) => {
            const handle = createV2LocalHandle(V2_INITIAL_DOC);
            const primaryListener = jest.fn();
            const secondaryListener = jest.fn();
            handle.addErrorListener(primaryListener);
            handle.addErrorListener(secondaryListener);
            const { getByTestId } = render(<NativeRichTextEditor documentHandle={handle} />);
            const view = getByTestId('native-editor-view');
            const error = {
                domain: 'operation',
                code: 'POSITION_INVALID',
                message: 'native selection is invalid',
                requestId: '7',
            };
            const emit = () =>
                view.props.onEditorError({
                    // iOS constructs the map identity-first while Android
                    // supplies the error field first.
                    nativeEvent:
                        platform === 'iOS'
                            ? { editorId: handle.editorId, error }
                            : { error, editorId: handle.editorId },
                });

            act(() => emit());
            act(() => emit());

            // Equal native failures are separate emissions, never value-deduped.
            expect(primaryListener).toHaveBeenCalledTimes(2);
            expect(secondaryListener).toHaveBeenCalledTimes(2);
            expect(primaryListener).toHaveBeenNthCalledWith(
                1,
                expect.objectContaining({ code: 'POSITION_INVALID', requestId: '7' })
            );
            handle.destroy();
        }
    );

    it('accepts a queued native commit after the engine advances before event delivery', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const onContentChange = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} onContentChange={onContentChange} />
        );
        const baseDocumentVersion = handle.bridge.getState().documentRevision;
        v2Runtime.module.editorV2ApplyInput(
            handle.editorId,
            JSON.stringify({
                version: 1,
                requestId: '1',
                baseDocumentRevision: baseDocumentVersion,
                text: '!',
            })
        );
        const queuedRevision = handle.bridge.getState().documentRevision;
        const queuedUpdateJson = renderUpdateValue(handle.editorId);

        handle.bridge.replaceDocument({ setJson: V2_DOC_B, history: 'undoableBoundary' });

        act(() => {
            getByTestId('native-editor-view').props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    documentRevision: queuedRevision,
                    updateJson: queuedUpdateJson,
                },
            });
        });

        expect(onContentChange).toHaveBeenCalledTimes(1);
        expect(handle.bridge.getContentSnapshot().json).toEqual(V2_DOC_B);
        handle.destroy();
    });

    it('suppresses a native-origin revision even when collaboration reports it before the native event', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId, rerender } = render(<NativeRichTextEditor documentHandle={handle} />);
        v2Runtime.module.editorV2ApplyInput(
            handle.editorId,
            JSON.stringify({
                version: 1,
                requestId: '1',
                baseDocumentRevision: handle.bridge.getState().documentRevision,
                text: '!',
            })
        );
        v2Runtime.session(handle.editorId).documentOrigin = 'nativeView';
        const revision = handle.bridge.getState().documentRevision;

        rerender(<NativeRichTextEditor documentHandle={handle} documentRevision={revision} />);

        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();
        handle.destroy();
    });

    it('rejects invalid error identities and turns one malformed current error into FFI_RESULT_INVALID', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const received: unknown[] = [];
        handle.addErrorListener((error) => received.push(error));
        const { getByTestId } = render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        const view = getByTestId('native-editor-view');
        const validError = {
            domain: 'operation',
            code: 'POSITION_INVALID',
            message: 'native selection is invalid',
        };

        for (const editorId of [undefined, 1, '01', '18446744073709551616', '999']) {
            act(() => {
                view.props.onEditorError({ nativeEvent: { editorId, error: validError } });
            });
        }
        expect(received).toHaveLength(0);

        act(() => {
            view.props.onEditorError({
                nativeEvent: { editorId: handle.editorId, error: { code: 42 } },
            });
        });
        expect(received).toHaveLength(1);
        expect(received[0]).toBeInstanceOf(NativeEditorNonRetryableError);
        expect((received[0] as NativeEditorNonRetryableError).code).toBe('FFI_RESULT_INVALID');

        // The boundary event is non-terminal: a normal interaction remains usable.
        act(() => ref.current!.toggleMark('bold'));
        expect(mockNativeModule.editorV2ApplyCommand).toHaveBeenCalledTimes(1);
        handle.destroy();
    });

    it('drops late autonomous errors after unsubscribe, rebind, unmount, and destroy', () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const receivedA: unknown[] = [];
        const receivedB: unknown[] = [];
        const unsubscribeA = handleA.addErrorListener((error) => receivedA.push(error));
        handleB.addErrorListener((error) => receivedB.push(error));
        const { getByTestId, rerender, unmount } = render(
            <NativeRichTextEditor documentHandle={handleA} />
        );
        const firstABinding = getByTestId('native-editor-view').props.onEditorError;
        const errorA = {
            nativeEvent: {
                editorId: handleA.editorId,
                error: { domain: 'operation', code: 'POSITION_INVALID', message: 'A error' },
            },
        };

        act(() => firstABinding(errorA));
        expect(receivedA).toHaveLength(1);
        unsubscribeA();
        act(() => firstABinding(errorA));
        expect(receivedA).toHaveLength(1);

        rerender(<NativeRichTextEditor documentHandle={handleB} />);
        const bBinding = getByTestId('native-editor-view').props.onEditorError;
        rerender(<NativeRichTextEditor documentHandle={handleA} />);
        const reboundABinding = getByTestId('native-editor-view').props.onEditorError;
        handleA.addErrorListener((error) => receivedA.push(error));

        act(() => firstABinding(errorA));
        act(() =>
            bBinding({
                nativeEvent: {
                    editorId: handleB.editorId,
                    error: { domain: 'operation', code: 'POSITION_INVALID', message: 'B error' },
                },
            })
        );
        expect(receivedA).toHaveLength(1);
        expect(receivedB).toHaveLength(0);

        act(() => reboundABinding(errorA));
        expect(receivedA).toHaveLength(2);

        unmount();
        act(() => reboundABinding(errorA));
        expect(receivedA).toHaveLength(2);

        handleA.destroy();
        act(() => reboundABinding(errorA));
        expect(receivedA).toHaveLength(2);
        handleB.destroy();
    });

    it('respects editable={false} on the native view', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} editable={false} />
        );
        expect(getByTestId('native-editor-view').props.editable).toBe(false);
        handle.destroy();
    });

    it('routes a native typing commit through the engine exactly once and never echoes it back', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onContentChange = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                ref={ref}
                documentHandle={handle}
                onContentChange={onContentChange}
            />
        );
        mockNativeModule.editorV2ApplyInput.mockClear();
        mockNativeModule.editorV2ApplyCommand.mockClear();
        mockNativeModule.editorV2ApplyLocalApi.mockClear();
        mockNativeModule.editorV2ReplaceDocument.mockClear();

        // While the IME is composing there is no native event and no engine
        // traffic at all — transient composing text never crosses JS.
        expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();

        // The native adapter commits the final text as ONE typed transaction,
        // then the view emits the resulting update.
        const commitRequest = JSON.stringify({
            version: 1,
            requestId: '1',
            baseDocumentRevision: handle.bridge.getState().documentRevision,
            text: '!',
        });
        let commitOutcome: { value: string } | undefined;
        act(() => {
            commitOutcome = v2Runtime.module.editorV2ApplyInput(handle.editorId, commitRequest) as {
                value: string;
            };
        });
        expect(JSON.parse(commitOutcome!.value)).toMatchObject({ type: 'transaction' });

        act(() => {
            getByTestId('native-editor-view').props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            });
        });

        // The component issued no mutation of its own: exactly one typed
        // transaction reached the engine (the adapter's).
        expect(mockNativeModule.editorV2ApplyInput).toHaveBeenCalledTimes(1);
        expect(mockNativeModule.editorV2ApplyCommand).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(mockNativeModule.editorV2ReplaceDocument).not.toHaveBeenCalled();
        // The content callback fired exactly once for the one commit.
        expect(onContentChange).toHaveBeenCalledTimes(1);
        expect(onContentChange).toHaveBeenCalledWith('<p>hello!</p>');
        // No echo: the view already applied the adapter's update natively.
        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();
        expect(ref.current!.getContent()).toBe('<p>hello!</p>');
        handle.destroy();
    });
});
