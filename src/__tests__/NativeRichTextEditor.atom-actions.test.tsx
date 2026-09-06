import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeFocus,
    mockNativeModule,
    v2Runtime,
    V2_INITIAL_DOC,
    createV2LocalHandle,
    renderUpdateValue,
    counterAtomDefinition,
    atomBlock,
    installAtomRenderSource,
} from './helpers/NativeRichTextEditorFixture';

import { render, act } from '@testing-library/react-native';
import { NativeRichTextEditor } from '../NativeRichTextEditor';
import { type RenderBlocksPatch, type RenderElement } from '../NativeEditorBridge';
import { NativeEditorOperationError } from '../NativeEditorBoundaryError';

import { AtomUpdateAttrsError } from '../atomInstances';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it.each([false, true])(
        'blocks atom updates when disabled, including retained callbacks (initial editable=%s)',
        async (initialEditable) => {
            const { definition } = counterAtomDefinition();
            const handle = createV2LocalHandle(V2_INITIAL_DOC);
            installAtomRenderSource(() => ({ renderBlocks: atomBlock(), renderPatch: null }));
            const { getByTestId, rerender } = render(
                <NativeRichTextEditor
                    documentHandle={handle}
                    atoms={[definition]}
                    editable={initialEditable}
                />
            );
            act(() => {
                getByTestId('native-editor-view').props.onAtomLayout({
                    nativeEvent: { editorId: handle.editorId, width: 200 },
                });
            });
            const updateAttrs = getByTestId('counter-atom').props.atomProps.updateAttrs;
            const applyCommand = jest.spyOn(handle.bridge, 'applyCommand');
            rerender(
                <NativeRichTextEditor
                    documentHandle={handle}
                    atoms={[definition]}
                    editable={false}
                />
            );

            await act(async () => {
                await expect(updateAttrs({ title: 'blocked' })).rejects.toMatchObject({
                    name: 'AtomUpdateAttrsError',
                    code: 'not-applicable',
                });
            });
            expect(applyCommand).not.toHaveBeenCalled();

            rerender(
                <NativeRichTextEditor documentHandle={handle} atoms={[definition]} editable />
            );
            await act(async () => {
                await expect(updateAttrs({ title: 'enabled' })).rejects.toMatchObject({
                    code: 'not-applicable',
                });
            });
            expect(applyCommand).toHaveBeenCalledTimes(1);
            handle.destroy();
        }
    );

    it('exposes atom selection and caret actions using document positions', async () => {
        const { definition } = counterAtomDefinition();
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        installAtomRenderSource(() => ({
            renderBlocks: atomBlock('counterCard', 5, 'stable'),
            renderPatch: null,
        }));
        const view = render(<NativeRichTextEditor documentHandle={handle} atoms={[definition]} />);
        act(() =>
            view
                .getByTestId('native-editor-view')
                .props.onAtomLayout({ nativeEvent: { editorId: handle.editorId, width: 200 } })
        );
        const actions = view.getByTestId('counter-atom').props.atomProps.editor;
        expect(actions).toBeDefined();
        const state = handle.bridge.getState();
        const result = {
            value: JSON.stringify({
                type: 'transaction',
                changed: false,
                documentRevision: state.documentRevision,
                stateRevision: '2',
                canUndo: false,
                canRedo: false,
            }),
            error: null,
        };
        mockNativeModule.editorV2SetSelection.mockReturnValue(result);
        await act(async () => {
            await actions.select();
            await actions.focusBefore();
            await actions.focusAfter();
        });
        expect(
            mockNativeModule.editorV2SetSelection.mock.calls
                .slice(-3)
                .map((call) => JSON.parse(call[1]).selection)
        ).toEqual([
            { type: 'atom', docPos: 5, edge: 'node' },
            { type: 'atom', docPos: 5, edge: 'before' },
            { type: 'atom', docPos: 5, edge: 'after' },
        ]);
        expect(mockNativeFocus).toHaveBeenCalledTimes(2);
        view.rerender(
            <NativeRichTextEditor documentHandle={handle} atoms={[definition]} editable={false} />
        );
        await act(async () => {
            await expect(actions.delete()).rejects.toMatchObject({ code: 'not-applicable' });
        });
        view.unmount();
        await expect(actions.select()).rejects.toMatchObject({ code: 'not-ready' });
        handle.destroy();
    });

    it('groups functional atom updates into one transaction', async () => {
        const { definition } = counterAtomDefinition();
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        installAtomRenderSource(() => ({
            renderBlocks: atomBlock('counterCard', 1, 'stable'),
            renderPatch: null,
        }));
        const view = render(<NativeRichTextEditor documentHandle={handle} atoms={[definition]} />);
        act(() =>
            view
                .getByTestId('native-editor-view')
                .props.onAtomLayout({ nativeEvent: { editorId: handle.editorId, width: 200 } })
        );
        mockNativeModule.editorV2ApplyCommand.mockImplementationOnce(() => ({
            value: JSON.stringify({
                type: 'transaction',
                changed: true,
                documentRevision: handle.bridge.getState().documentRevision,
                stateRevision: '2',
                canUndo: true,
                canRedo: false,
            }),
            error: null,
        }));
        await act(async () =>
            view
                .getByTestId('counter-atom')
                .props.atomProps.updateAttrs([
                    (attrs: any) => ({ title: attrs.title + 'b' }),
                    (attrs: any) => ({ title: attrs.title + 'c' }),
                ])
        );
        expect(
            JSON.parse(mockNativeModule.editorV2ApplyCommand.mock.calls.at(-1)![1])
        ).toMatchObject({ command: { type: 'updateNodeAttrs', attrs: { title: 'abc' } } });
        handle.destroy();
    });

    it('virtualizes offscreen atom renderers while keeping their native hosts', () => {
        const { definition } = counterAtomDefinition();
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        installAtomRenderSource(() => ({
            renderBlocks: atomBlock('counterCard', 1, 'stable'),
            renderPatch: null,
        }));
        const view = render(
            <NativeRichTextEditor
                documentHandle={handle}
                atoms={[definition]}
                atomViewport={{ y: 0, height: 200, overscan: 0 }}
            />
        );
        act(() =>
            view.getByTestId('native-editor-view').props.onAtomLayout({
                nativeEvent: {
                    editorId: handle.editorId,
                    width: 200,
                    positions: [{ key: 'stable', x: 0, y: 1000 }],
                },
            })
        );
        expect(view.queryByTestId('counter-atom')).toBeNull();
        expect(view.UNSAFE_getByProps({ nativeID: 'prose-atom:stable' })).toBeTruthy();
        handle.destroy();
    });

    it('maps atom updateAttrs outcomes and uses the current document revision', async () => {
        const { definition } = counterAtomDefinition();
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        installAtomRenderSource(() => ({ renderBlocks: atomBlock(), renderPatch: null }));
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} atoms={[definition]} />
        );
        const view = getByTestId('native-editor-view');
        act(() => {
            view.props.onAtomLayout({
                nativeEvent: { editorId: handle.editorId, width: 200 },
            });
        });
        const updateAttrs = getByTestId('counter-atom').props.atomProps.updateAttrs as (
            attrs: Record<string, unknown>
        ) => Promise<void>;
        const revision = handle.bridge.getState().documentRevision;
        mockNativeModule.editorV2ApplyCommand.mockImplementationOnce(() => ({
            value: JSON.stringify({
                type: 'transaction',
                changed: true,
                documentRevision: revision,
                stateRevision: '2',
                canUndo: true,
                canRedo: false,
            }),
            error: null,
        }));
        await act(async () => updateAttrs({ title: 'b' }));
        expect(
            JSON.parse(mockNativeModule.editorV2ApplyCommand.mock.calls.at(-1)![1] as string)
        ).toMatchObject({
            baseDocumentRevision: revision,
            command: { type: 'updateNodeAttrs', docPos: 1, attrs: { title: 'b' } },
        });

        await act(async () => {
            await expect(updateAttrs({ title: 'c' })).rejects.toMatchObject<AtomUpdateAttrsError>({
                code: 'not-applicable',
            });
        });
        jest.spyOn(handle.bridge, 'applyCommand').mockImplementationOnce(() => {
            throw new NativeEditorOperationError({
                domain: 'operation',
                code: 'REVISION_MISMATCH',
                message: 'stale',
                requestId: null,
                operationIndex: null,
                limit: null,
                actual: null,
                details: null,
            });
        });
        await act(async () => {
            await expect(updateAttrs({ title: 'd' })).rejects.toMatchObject<AtomUpdateAttrsError>({
                code: 'stale-revision',
            });
        });
        jest.spyOn(handle.bridge, 'applyCommand').mockImplementationOnce(() => {
            throw new Error('boom');
        });
        await act(async () => {
            await expect(updateAttrs({ title: 'e' })).rejects.toMatchObject<AtomUpdateAttrsError>({
                code: 'engine-error',
            });
        });
        handle.destroy();
        await act(async () => {
            await expect(updateAttrs({ title: 'f' })).rejects.toMatchObject<AtomUpdateAttrsError>({
                code: 'not-ready',
            });
        });
    });

    it('resolves an atom position by its stable id when an older updateAttrs callback runs', async () => {
        const { definition } = counterAtomDefinition();
        let renderSource:
            | { renderBlocks: RenderElement[][]; renderPatch: null }
            | { renderBlocks: null; renderPatch: RenderBlocksPatch } = {
            renderBlocks: atomBlock('counterCard', 1, 'client-1:9'),
            renderPatch: null,
        };
        installAtomRenderSource(() => renderSource);
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} atoms={[definition]} />
        );
        const view = getByTestId('native-editor-view');
        act(() => {
            view.props.onAtomLayout({
                nativeEvent: { editorId: handle.editorId, width: 200 },
            });
        });
        const staleUpdateAttrs = getByTestId('counter-atom').props.atomProps.updateAttrs as (
            attrs: Record<string, unknown>
        ) => Promise<void>;

        v2Runtime.module.editorV2ApplyInput(
            handle.editorId,
            JSON.stringify({
                version: 1,
                requestId: '1',
                baseDocumentRevision: handle.bridge.getState().documentRevision,
                text: '!',
            })
        );
        renderSource = {
            renderBlocks: atomBlock('counterCard', 5, 'client-1:9'),
            renderPatch: null,
        };
        act(() => {
            view.props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            });
        });
        const revision = handle.bridge.getState().documentRevision;
        mockNativeModule.editorV2ApplyCommand.mockImplementationOnce(() => ({
            value: JSON.stringify({
                type: 'transaction',
                changed: true,
                documentRevision: revision,
                stateRevision: '2',
                canUndo: true,
                canRedo: false,
            }),
            error: null,
        }));

        await act(async () => staleUpdateAttrs({ title: 'b' }));

        expect(
            JSON.parse(mockNativeModule.editorV2ApplyCommand.mock.calls.at(-1)![1] as string)
        ).toMatchObject({ command: { type: 'updateNodeAttrs', docPos: 5 } });
        handle.destroy();
    });

    it('rejects an older positional atom callback after the document changes', async () => {
        const { definition } = counterAtomDefinition();
        let renderSource = { renderBlocks: atomBlock('counterCard', 1), renderPatch: null };
        installAtomRenderSource(() => renderSource);
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} atoms={[definition]} />
        );
        const view = getByTestId('native-editor-view');
        act(() =>
            view.props.onAtomLayout({ nativeEvent: { editorId: handle.editorId, width: 200 } })
        );
        const staleUpdateAttrs = getByTestId('counter-atom').props.atomProps.updateAttrs;

        v2Runtime.module.editorV2ApplyInput(
            handle.editorId,
            JSON.stringify({
                version: 1,
                requestId: '1',
                baseDocumentRevision: handle.bridge.getState().documentRevision,
                text: '!',
            })
        );
        renderSource = { renderBlocks: atomBlock('counterCard', 5), renderPatch: null };
        act(() =>
            view.props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            })
        );

        await act(async () => {
            await expect(staleUpdateAttrs({ title: 'b' })).rejects.toMatchObject({
                code: 'not-applicable',
            });
        });
        handle.destroy();
    });

    it('resyncs atom children when updateAttrs is no longer applicable', async () => {
        const { definition } = counterAtomDefinition();
        let renderSource:
            | { renderBlocks: RenderElement[][]; renderPatch: null }
            | { renderBlocks: null; renderPatch: RenderBlocksPatch } = {
            renderBlocks: atomBlock('counterCard', 1, 'client-1:9'),
            renderPatch: null,
        };
        installAtomRenderSource(() => renderSource);
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId, queryByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} atoms={[definition]} />
        );
        const view = getByTestId('native-editor-view');
        act(() => {
            view.props.onAtomLayout({
                nativeEvent: { editorId: handle.editorId, width: 200 },
            });
        });
        const updateAttrs = getByTestId('counter-atom').props.atomProps.updateAttrs as (
            attrs: Record<string, unknown>
        ) => Promise<void>;
        renderSource = { renderBlocks: [], renderPatch: null };

        await act(async () => {
            await expect(updateAttrs({ title: 'b' })).rejects.toMatchObject<AtomUpdateAttrsError>({
                code: 'not-applicable',
            });
        });

        expect(queryByTestId('counter-atom')).toBeNull();
        handle.destroy();
    });

    it('does not rerender an unchanged atom component for a native text commit', () => {
        const { component, definition } = counterAtomDefinition();
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        installAtomRenderSource(() => ({
            renderBlocks: atomBlock('counterCard', 1, 'client-1:9'),
            renderPatch: null,
        }));
        const { getByTestId } = render(
            <NativeRichTextEditor documentHandle={handle} atoms={[definition]} />
        );
        const view = getByTestId('native-editor-view');
        act(() => {
            view.props.onAtomLayout({
                nativeEvent: { editorId: handle.editorId, width: 200 },
            });
        });
        const renderCount = component.mock.calls.length;
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
            view.props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            });
        });

        expect(component).toHaveBeenCalledTimes(renderCount);
        handle.destroy();
    });
});
