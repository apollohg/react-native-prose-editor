import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeModule,
    V2_TRANSPORT_URL,
    v2Runtime,
    V2_INITIAL_DOC,
    V2_DOC_B,
    V2_DOC_C,
    createV2RoomHandle,
    createV2LocalHandle,
    renderUpdateValue,
} from './helpers/NativeRichTextEditorFixture';

import { render, act } from '@testing-library/react-native';
import { NativeRichTextEditor } from '../NativeRichTextEditor';
import { type DocumentJSON } from '../NativeEditorBridge';

import { useYjsCollaboration } from '../YjsCollaboration';
import { V2_FAKE_STEP2_FRAME } from './helpers/nativeEditorV2Fake';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('does not roll back a newer native commit when a prior controlled echo renders late', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const onContentChange = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                value='<p>hello</p>'
                onContentChange={onContentChange}
            />
        );
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        const commit = (html: string) => {
            handle.bridge.replaceDocument({ setHtml: html, history: 'undoableBoundary' });
            getByTestId('native-editor-view').props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            });
        };

        act(() => commit('<p>hello!</p>'));
        expect(onContentChange).toHaveBeenLastCalledWith('<p>hello!</p>');
        act(() => commit('<p>hello! </p>'));
        expect(onContentChange).toHaveBeenLastCalledWith('<p>hello! </p>');
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        rerender(
            <NativeRichTextEditor
                documentHandle={handle}
                value='<p>hello!</p>'
                onContentChange={onContentChange}
            />
        );

        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(handle.bridge.getContentSnapshot().html).toBe('<p>hello! </p>');
        expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();

        rerender(
            <NativeRichTextEditor
                documentHandle={handle}
                value='<p>hello! </p>'
                onContentChange={onContentChange}
            />
        );
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();

        rerender(
            <NativeRichTextEditor
                documentHandle={handle}
                value='<p>hello!</p>'
                onContentChange={onContentChange}
            />
        );
        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        expect(handle.bridge.getContentSnapshot().html).toBe('<p>hello!</p>');
        handle.destroy();
    });

    it('does not roll back a newer native commit when a prior controlled JSON echo renders late', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const onContentChangeJSON = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                valueJSON={V2_INITIAL_DOC}
                onContentChangeJSON={onContentChangeJSON}
            />
        );
        mockNativeModule.editorV2ApplyLocalApi.mockClear();

        const commit = (json: DocumentJSON) => {
            handle.bridge.replaceDocument({ setJson: json, history: 'undoableBoundary' });
            getByTestId('native-editor-view').props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            });
        };

        act(() => commit(V2_DOC_B));
        act(() => commit(V2_DOC_C));

        rerender(
            <NativeRichTextEditor
                documentHandle={handle}
                valueJSON={V2_DOC_B}
                onContentChangeJSON={onContentChangeJSON}
            />
        );
        expect(mockNativeModule.editorV2ApplyLocalApi).not.toHaveBeenCalled();
        expect(handle.bridge.getContentSnapshot().json).toEqual(V2_DOC_C);

        rerender(
            <NativeRichTextEditor
                documentHandle={handle}
                valueJSON={V2_DOC_C}
                onContentChangeJSON={onContentChangeJSON}
            />
        );
        rerender(
            <NativeRichTextEditor
                documentHandle={handle}
                valueJSON={V2_DOC_B}
                onContentChangeJSON={onContentChangeJSON}
            />
        );
        expect(mockNativeModule.editorV2ApplyLocalApi).toHaveBeenCalledTimes(1);
        expect(handle.bridge.getContentSnapshot().json).toEqual(V2_DOC_B);
        handle.destroy();
    });

    it('emits content changes for incremental native commit snapshots', () => {
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
        const atomicUpdate = JSON.parse(renderUpdateValue(handle.editorId)) as Record<
            string,
            unknown
        >;
        const renderBlocks = atomicUpdate.renderBlocks as unknown[];
        atomicUpdate.renderBlocks = null;
        atomicUpdate.renderPatch = {
            baseDocumentVersion,
            startIndex: 0,
            deleteCount: renderBlocks.length,
            renderBlocks,
        };

        act(() => {
            getByTestId('native-editor-view').props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: JSON.stringify(atomicUpdate),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            });
        });

        expect(onContentChange).toHaveBeenCalledWith('<p>hello!</p>');
        handle.destroy();
    });

    it.each(['iOS', 'Android'])(
        'accepts only an authentic canonical %s native commit payload and suppresses its exact echo',
        (platform) => {
            const handle = createV2LocalHandle(V2_INITIAL_DOC);
            const onContentChange = jest.fn();
            const { getByTestId, rerender } = render(
                <NativeRichTextEditor documentHandle={handle} onContentChange={onContentChange} />
            );
            const view = getByTestId('native-editor-view');
            const commit = (text: string) => {
                v2Runtime.module.editorV2ApplyInput(
                    handle.editorId,
                    JSON.stringify({
                        version: 1,
                        requestId: text === '!' ? '1' : '2',
                        baseDocumentRevision: handle.bridge.getState().documentRevision,
                        text,
                    })
                );
                const documentRevision = handle.bridge.getState().documentRevision;
                return {
                    editorId: handle.editorId,
                    documentRevision,
                    updateJson: renderUpdateValue(handle.editorId),
                };
            };
            const first = commit('!');
            const emit = (payload: Record<string, unknown>) =>
                view.props.onEditorUpdate({
                    // Keep both native property orderings covered: iOS emits
                    // identity first while Android builds its map update-first.
                    nativeEvent:
                        platform === 'iOS'
                            ? payload
                            : {
                                  updateJson: payload.updateJson,
                                  documentRevision: payload.documentRevision,
                                  editorId: payload.editorId,
                              },
                });

            const mismatchedSnapshot = JSON.parse(first.updateJson) as Record<string, unknown>;
            mismatchedSnapshot.documentVersion = '0';
            const rejectedPayloads: Record<string, unknown>[] = [
                { ...first, editorId: '999' },
                { ...first, documentRevision: '01' },
                { ...first, documentRevision: '18446744073709551616' },
                { ...first, updateJson: '{}' },
                { ...first, updateJson: JSON.stringify(mismatchedSnapshot) },
            ];
            for (const rejected of rejectedPayloads) {
                act(() => emit(rejected));
            }
            expect(onContentChange).not.toHaveBeenCalled();

            act(() => emit(first));
            expect(onContentChange).toHaveBeenCalledTimes(1);

            // Duplicate native delivery must not refresh, notify, or replace
            // the one-shot echo token.
            act(() => emit(first));
            expect(onContentChange).toHaveBeenCalledTimes(1);

            // The identical revision signal is the sole suppressed echo.
            rerender(
                <NativeRichTextEditor
                    documentHandle={handle}
                    documentRevision={first.documentRevision}
                    onContentChange={onContentChange}
                />
            );
            expect(getByTestId('native-editor-view').props.editorUpdateJson).toBeUndefined();

            // A different/newer external revision clears that token and is
            // pushed, rather than being hidden behind native revision N.
            handle.bridge.replaceDocument({ setJson: V2_DOC_B, history: 'undoableBoundary' });
            const externalRevision = handle.bridge.getState().documentRevision;
            rerender(
                <NativeRichTextEditor
                    documentHandle={handle}
                    documentRevision={externalRevision}
                    onContentChange={onContentChange}
                />
            );
            expect(
                JSON.parse(getByTestId('native-editor-view').props.editorUpdateJson as string)
                    .documentVersion
            ).toBe(externalRevision);

            // A stale native commit after N+1 is deterministically rejected:
            // it produces no further content notification of its own.
            const notificationsBeforeStaleCommit = onContentChange.mock.calls.length;
            act(() => emit(first));
            expect(onContentChange).toHaveBeenCalledTimes(notificationsBeforeStaleCommit);
            handle.destroy();
        }
    );

    it('forwards native selection changes with the engine selection and fires focus/blur edges', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const onSelectionChange = jest.fn();
        const onFocus = jest.fn();
        const onBlur = jest.fn();
        const { getByTestId } = render(
            <NativeRichTextEditor
                documentHandle={handle}
                onSelectionChange={onSelectionChange}
                onFocus={onFocus}
                onBlur={onBlur}
            />
        );

        act(() => {
            getByTestId('native-editor-view').props.onSelectionChange({
                nativeEvent: {
                    anchor: 1,
                    head: 3,
                    editorId: handle.editorId,
                    stateJson: renderUpdateValue(handle.editorId, 1, 3),
                },
            });
        });
        expect(onSelectionChange).toHaveBeenCalledTimes(1);
        // The native mirror offsets are scalar coordinates; the render
        // payload resolves them to the engine's document coordinates.
        expect(onSelectionChange).toHaveBeenCalledWith({ type: 'text', anchor: 2, head: 4 });

        // Without a state payload the raw scalar range is forwarded.
        act(() => {
            getByTestId('native-editor-view').props.onSelectionChange({
                nativeEvent: { anchor: 2, head: 4, editorId: handle.editorId },
            });
        });
        expect(onSelectionChange).toHaveBeenLastCalledWith({ type: 'text', anchor: 2, head: 4 });

        act(() => {
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });
        expect(onFocus).toHaveBeenCalledTimes(1);
        expect(onBlur).not.toHaveBeenCalled();
        act(() => {
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });
        expect(onFocus).toHaveBeenCalledTimes(1);
        act(() => {
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { isFocused: false, editorId: handle.editorId },
            });
        });
        expect(onBlur).toHaveBeenCalledTimes(1);
        handle.destroy();
    });

    it('discards a stale auto-grow height before returning from fixed mode', () => {
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor documentHandle={handle} heightBehavior='autoGrow' />
        );

        act(() => {
            getByTestId('native-editor-view').props.onContentHeightChange({
                nativeEvent: { contentHeight: 420, editorId: handle.editorId },
            });
        });
        expect(getByTestId('native-editor-view').props.style).toEqual(
            expect.objectContaining({ height: 420 })
        );

        rerender(<NativeRichTextEditor documentHandle={handle} heightBehavior='fixed' />);
        rerender(<NativeRichTextEditor documentHandle={handle} heightBehavior='autoGrow' />);

        expect(getByTestId('native-editor-view').props.style).not.toEqual(
            expect.objectContaining({ height: 420 })
        );
        handle.destroy();
    });

    it('ignores missing and late editor-scoped interaction events after rebinding to B', () => {
        const handleA = createV2LocalHandle(V2_INITIAL_DOC);
        const handleB = createV2LocalHandle(V2_DOC_B);
        const onSelectionChange = jest.fn();
        const onFocus = jest.fn();
        const onBlur = jest.fn();
        const onToolbarAction = jest.fn();
        const { getByTestId, rerender } = render(
            <NativeRichTextEditor
                documentHandle={handleA}
                heightBehavior='autoGrow'
                onSelectionChange={onSelectionChange}
                onFocus={onFocus}
                onBlur={onBlur}
                onToolbarAction={onToolbarAction}
            />
        );

        rerender(
            <NativeRichTextEditor
                documentHandle={handleB}
                heightBehavior='autoGrow'
                onSelectionChange={onSelectionChange}
                onFocus={onFocus}
                onBlur={onBlur}
                onToolbarAction={onToolbarAction}
            />
        );
        const view = getByTestId('native-editor-view');

        act(() => {
            view.props.onSelectionChange({ nativeEvent: { anchor: 9, head: 9 } });
            view.props.onFocusChange({ nativeEvent: { isFocused: true } });
            view.props.onContentHeightChange({ nativeEvent: { contentHeight: 240 } });
            view.props.onToolbarAction({ nativeEvent: { key: 'late-action' } });
            view.props.onSelectionChange({
                nativeEvent: { anchor: 9, head: 9, editorId: handleA.editorId },
            });
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handleA.editorId },
            });
            view.props.onFocusChange({
                nativeEvent: { isFocused: false, editorId: handleA.editorId },
            });
            view.props.onContentHeightChange({
                nativeEvent: { contentHeight: 240, editorId: handleA.editorId },
            });
            view.props.onToolbarAction({
                nativeEvent: { key: 'late-action', editorId: handleA.editorId },
            });
        });

        expect(onSelectionChange).not.toHaveBeenCalled();
        expect(onFocus).not.toHaveBeenCalled();
        expect(onBlur).not.toHaveBeenCalled();
        expect(onToolbarAction).not.toHaveBeenCalled();
        expect(view.props.style).not.toEqual(expect.objectContaining({ height: 240 }));

        act(() => {
            view.props.onSelectionChange({
                nativeEvent: { anchor: 2, head: 2, editorId: handleB.editorId },
            });
            view.props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handleB.editorId },
            });
            view.props.onContentHeightChange({
                nativeEvent: { contentHeight: 240, editorId: handleB.editorId },
            });
            view.props.onToolbarAction({
                nativeEvent: { key: 'B-action', editorId: handleB.editorId },
            });
        });

        expect(onSelectionChange).toHaveBeenCalledWith({ type: 'text', anchor: 2, head: 2 });
        expect(onFocus).toHaveBeenCalledTimes(1);
        expect(onToolbarAction).toHaveBeenCalledWith('B-action');
        expect(getByTestId('native-editor-view').props.style).toEqual(
            expect.objectContaining({ height: 240 })
        );
        handleA.destroy();
        handleB.destroy();
    });

    it('keeps editor-bound awareness hooks inert after localAwareness is removed', () => {
        const handle = createV2RoomHandle({ withSnapshot: true });
        let localAwareness: { userId: string; name: string; color: string } | undefined = {
            userId: '1',
            name: 'Alice',
            color: '#f00',
        };
        let collaboration: ReturnType<typeof useYjsCollaboration> | null = null;

        function CollaborationBoundEditor() {
            collaboration = useYjsCollaboration({
                documentId: 'doc-1',
                handle,
                localAwareness,
                transport: { url: V2_TRANSPORT_URL, connect: true },
            });
            return <NativeRichTextEditor {...collaboration.editorBindings} />;
        }

        const { getByTestId, rerender } = render(<CollaborationBoundEditor />);
        act(() => {
            v2Runtime.transportOpen(handle.editorId);
            v2Runtime.transportReceive(handle.editorId, V2_FAKE_STEP2_FRAME);
        });
        localAwareness = undefined;
        rerender(<CollaborationBoundEditor />);

        expect(v2Runtime.session(handle.editorId).desiredAwareness).toBeNull();
        const awarenessCallsAfterClear =
            mockNativeModule.editorV2CollaborationSetAwareness.mock.calls.length;
        act(() => {
            getByTestId('native-editor-view').props.onSelectionChange({
                nativeEvent: { anchor: 1, head: 3, editorId: handle.editorId },
            });
            getByTestId('native-editor-view').props.onFocusChange({
                nativeEvent: { isFocused: true, editorId: handle.editorId },
            });
        });

        expect(mockNativeModule.editorV2CollaborationSetAwareness).toHaveBeenCalledTimes(
            awarenessCallsAfterClear
        );
        expect(v2Runtime.session(handle.editorId).desiredAwareness).toBeNull();
        expect(collaboration).not.toBeNull();
        handle.destroy();
    });
});
