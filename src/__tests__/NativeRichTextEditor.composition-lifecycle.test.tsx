import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeFocus,
    mockNativeBlur,
    mockNativeBeginExternalComposition,
    mockNativeUpdateExternalComposition,
    mockNativeCommitExternalComposition,
    V2_INITIAL_DOC,
    V2_DOC_B,
    createV2LocalHandle,
    disableExternalCompositionSupport,
} from './helpers/NativeRichTextEditorFixture';
import { createRef, StrictMode } from 'react';

import { render, act } from '@testing-library/react-native';
import { NativeRichTextEditor, type NativeRichTextEditorRef } from '../NativeRichTextEditor';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('forwards focus and blur ref calls to the native view', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);

        act(() => {
            ref.current!.focus();
        });
        expect(mockNativeFocus).toHaveBeenCalledTimes(1);
        act(() => {
            ref.current!.blur();
        });
        expect(mockNativeBlur).toHaveBeenCalledTimes(1);
        handle.destroy();
    });

    it('drives external composition through the native view ref', async () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);

        expect(ref.current!.supportsExternalTextComposition()).toBe(true);
        const session = await ref.current!.beginExternalTextComposition();
        const sessionId = mockNativeBeginExternalComposition.mock.calls.at(-1)![0];
        await session.update('on arrival');
        await session.commit('O/A');

        expect(mockNativeUpdateExternalComposition).toHaveBeenCalledWith(sessionId, 'on arrival');
        expect(mockNativeCommitExternalComposition).toHaveBeenCalledWith(sessionId, 'O/A');
        handle.destroy();
    });

    it('keeps the manager usable through Strict Mode replay and ends rebound ownership once', async () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const ref = createRef<NativeRichTextEditorRef>();
        const onEndA = jest.fn();
        const onEndB = jest.fn();
        const { rerender, unmount } = render(
            <StrictMode>
                <NativeRichTextEditor ref={ref} documentHandle={handleA} />
            </StrictMode>
        );

        expect(ref.current!.supportsExternalTextComposition()).toBe(true);
        await expect(
            ref.current!.beginExternalTextComposition({ onEnd: onEndA })
        ).resolves.toBeDefined();

        rerender(
            <StrictMode>
                <NativeRichTextEditor ref={ref} documentHandle={handleB} />
            </StrictMode>
        );
        await act(async () => Promise.resolve());

        expect(onEndA).toHaveBeenCalledTimes(1);
        expect(onEndA).toHaveBeenCalledWith({
            outcome: 'cancelled',
            cause: 'lifecycle',
            text: '',
        });
        expect(ref.current!.supportsExternalTextComposition()).toBe(true);
        await expect(
            ref.current!.beginExternalTextComposition({ onEnd: onEndB })
        ).resolves.toBeDefined();

        unmount();
        await act(async () => Promise.resolve());
        expect(onEndB).toHaveBeenCalledTimes(1);
        handleA.destroy();
        handleB.destroy();
    });

    it('routes composition-end and editor-error events after Strict Mode replay', async () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const onEnd = jest.fn();
        const errors: unknown[] = [];
        handle.addErrorListener((error) => errors.push(error));
        const { getByTestId } = render(
            <StrictMode>
                <NativeRichTextEditor ref={ref} documentHandle={handle} />
            </StrictMode>
        );
        await ref.current!.beginExternalTextComposition({ onEnd });
        const sessionId = mockNativeBeginExternalComposition.mock.calls.at(-1)![0];
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onExternalTextCompositionEnd({
                nativeEvent: {
                    editorId: handle.editorId,
                    resultJson: JSON.stringify({
                        version: 1,
                        type: 'ended',
                        sessionId,
                        outcome: 'committed',
                        cause: 'interaction',
                        text: 'O/A',
                    }),
                },
            });
            view.props.onExternalTextCompositionEnd({
                nativeEvent: {
                    editorId: handle.editorId,
                    resultJson: JSON.stringify({
                        version: 1,
                        type: 'ended',
                        sessionId,
                        outcome: 'committed',
                        cause: 'interaction',
                        text: 'O/A',
                    }),
                },
            });
            view.props.onEditorError({
                nativeEvent: {
                    editorId: handle.editorId,
                    error: {
                        domain: 'operation',
                        code: 'POSITION_INVALID',
                        message: 'strict replay error',
                    },
                },
            });
        });

        expect(onEnd).toHaveBeenCalledTimes(1);
        expect(onEnd).toHaveBeenCalledWith({
            outcome: 'committed',
            cause: 'interaction',
            text: 'O/A',
        });
        expect(errors).toHaveLength(1);
        expect(errors[0]).toMatchObject({ code: 'POSITION_INVALID' });
        handle.destroy();
    });

    it('routes a canonical automatic native end event to the owning session once', async () => {
        const onEnd = jest.fn();
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const { getByTestId } = render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        await ref.current!.beginExternalTextComposition({ onEnd });
        const sessionId = mockNativeBeginExternalComposition.mock.calls.at(-1)![0];
        const endEvent = {
            nativeEvent: {
                editorId: handle.editorId,
                resultJson: JSON.stringify({
                    version: 1,
                    type: 'ended',
                    sessionId,
                    outcome: 'committed',
                    cause: 'interaction',
                    text: 'O/A',
                }),
            },
        };

        act(() => {
            getByTestId('native-editor-view').props.onExternalTextCompositionEnd({
                ...endEvent,
                nativeEvent: { ...endEvent.nativeEvent, editorId: `0${handle.editorId}` },
            });
            getByTestId('native-editor-view').props.onExternalTextCompositionEnd(endEvent);
            getByTestId('native-editor-view').props.onExternalTextCompositionEnd(endEvent);
        });

        expect(onEnd).toHaveBeenCalledTimes(1);
        expect(onEnd).toHaveBeenCalledWith({
            outcome: 'committed',
            cause: 'interaction',
            text: 'O/A',
        });
        handle.destroy();
    });

    it('drops a stale canonical composition end event after handle rebind', async () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const ref = createRef<NativeRichTextEditorRef>();
        const onEndB = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor ref={ref} documentHandle={handleA} />
        );
        const staleEndHandler =
            getByTestId('native-editor-view').props.onExternalTextCompositionEnd;

        rerender(<NativeRichTextEditor ref={ref} documentHandle={handleB} />);
        await ref.current!.beginExternalTextComposition({ onEnd: onEndB });
        const sessionId = mockNativeBeginExternalComposition.mock.calls.at(-1)![0];
        const resultJson = JSON.stringify({
            version: 1,
            type: 'ended',
            sessionId,
            outcome: 'committed',
            cause: 'interaction',
            text: 'B',
        });

        act(() => {
            staleEndHandler({
                nativeEvent: { editorId: handleA.editorId, resultJson },
            });
        });
        expect(onEndB).not.toHaveBeenCalled();

        act(() => {
            getByTestId('native-editor-view').props.onExternalTextCompositionEnd({
                nativeEvent: { editorId: handleB.editorId, resultJson },
            });
        });
        expect(onEndB).toHaveBeenCalledTimes(1);
        handleA.destroy();
        handleB.destroy();
    });

    it('routes malformed native composition results through the handle error channel', async () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const received: unknown[] = [];
        handle.addErrorListener((error) => received.push(error));
        const { getByTestId } = render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        await ref.current!.beginExternalTextComposition();

        expect(() => {
            act(() => {
                getByTestId('native-editor-view').props.onExternalTextCompositionEnd({
                    nativeEvent: { editorId: handle.editorId, resultJson: '{' },
                });
            });
        }).not.toThrow();
        expect(received).toHaveLength(1);
        expect(received[0]).toMatchObject({
            domain: 'boundary',
            code: 'EXTERNAL_COMPOSITION_RESULT_INVALID',
        });
        handle.destroy();
    });

    it('disposes the bound composition manager on handle rebind and unmount', async () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const ref = createRef<NativeRichTextEditorRef>();
        const onEndA = jest.fn();
        const onEndB = jest.fn();
        const { rerender, unmount } = render(
            <NativeRichTextEditor ref={ref} documentHandle={handleA} />
        );
        await ref.current!.beginExternalTextComposition({ onEnd: onEndA });

        rerender(<NativeRichTextEditor ref={ref} documentHandle={handleB} />);
        await act(async () => Promise.resolve());

        expect(onEndA).toHaveBeenCalledWith({
            outcome: 'cancelled',
            cause: 'lifecycle',
            text: '',
        });

        await ref.current!.beginExternalTextComposition({ onEnd: onEndB });
        unmount();
        await act(async () => Promise.resolve());

        expect(onEndB).toHaveBeenCalledWith({
            outcome: 'cancelled',
            cause: 'lifecycle',
            text: '',
        });
        handleA.destroy();
        handleB.destroy();
    });

    it('requires all four native composition methods and an editable view', async () => {
        disableExternalCompositionSupport();
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const ref = createRef<NativeRichTextEditorRef>();
        const { rerender } = render(<NativeRichTextEditor ref={ref} documentHandle={handle} />);
        expect(ref.current!.supportsExternalTextComposition()).toBe(false);

        rerender(<NativeRichTextEditor ref={ref} documentHandle={handle} editable={false} />);
        await expect(ref.current!.beginExternalTextComposition()).rejects.toMatchObject({
            domain: 'lifecycle',
            code: 'EXTERNAL_COMPOSITION_UNAVAILABLE',
        });
        handle.destroy();
    });
});
