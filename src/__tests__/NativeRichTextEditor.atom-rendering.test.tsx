import './helpers/NativeRichTextEditorFixture';
import {
    mockNativeViewRender,
    v2Runtime,
    V2_INITIAL_DOC,
    V2_DOC_B,
    V2_SERVER_UPDATE_DOC,
    createV2RoomHandle,
    createV2LocalHandle,
    setupV2Controller,
    renderUpdateValue,
    counterAtomDefinition,
    atomBlock,
    installAtomRenderSource,
} from './helpers/NativeRichTextEditorFixture';
import { createRef } from 'react';
import { Platform, StyleSheet } from 'react-native';
import { render, act } from '@testing-library/react-native';
import { NativeRichTextEditor, type NativeRichTextEditorRef } from '../NativeRichTextEditor';
import {
    createNativeEditorDocumentHandle,
    type RenderBlocksPatch,
    type RenderElement,
} from '../NativeEditorBridge';

import { V2_FAKE_STEP2_FRAME, V2_FAKE_UPDATE_FRAME } from './helpers/nativeEditorV2Fake';

import { withAtomsSchema } from '../atoms';
import { DEFAULT_ATOM_CHIP_HEIGHT } from '../atomInstances';
import { tiptapCompatibleSchema } from '../schemas';

describe('NativeRichTextEditor (v2 document mode)', () => {
    it('does not schedule an extra render for an equivalent inline atoms array', () => {
        const { definition } = counterAtomDefinition();
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const editor = (label: string) => (
            <NativeRichTextEditor
                documentHandle={handle}
                accessibilityLabel={label}
                atoms={[definition]}
            />
        );
        const rendered = render(editor('Before'));
        const before = mockNativeViewRender.mock.calls.length;

        rendered.rerender(editor('After'));

        expect(mockNativeViewRender.mock.calls.length - before).toBe(1);
        handle.destroy();
    });

    it.each(['ios', 'android'] as const)(
        'seeds and positions atom hosts on %s and tracks selection',
        (platform) => {
            const platformSpy = jest.replaceProperty(Platform, 'OS', platform);
            const { definition } = counterAtomDefinition();
            const handle = createNativeEditorDocumentHandle({
                schema: withAtomsSchema(tiptapCompatibleSchema, [definition]),
                initialization: {
                    type: 'localJson',
                    json: {
                        type: 'doc',
                        content: [{ type: 'counterCard', attrs: { title: 'a' } }],
                    },
                },
            });
            installAtomRenderSource(() => ({
                renderBlocks: atomBlock('counterCard', 1, 'client-1:9'),
                renderPatch: null,
            }));

            const { getByTestId, queryByTestId, UNSAFE_getByProps } = render(
                <NativeRichTextEditor documentHandle={handle} atoms={[definition]} />
            );
            const nativeView = getByTestId('native-editor-view');
            expect(queryByTestId('counter-atom')).toBeNull();
            expect(JSON.parse(nativeView.props.atomsJson)).toEqual({
                nodeTypes: ['counterCard'],
                estimatedHeights: { counterCard: 120 },
            });

            act(() => {
                nativeView.props.onAtomLayout({
                    nativeEvent: {
                        editorId: handle.editorId,
                        width: 280,
                        positions: [{ key: 'client-1:9', x: 12, y: 34 }],
                    },
                });
            });
            const atom = getByTestId('counter-atom');
            const atomHost = UNSAFE_getByProps({ nativeID: 'prose-atom:client-1:9' });
            expect(getByTestId('atom-host').props.nativeID).toBe('prose-atom-content:client-1:9');
            expect(StyleSheet.flatten(atomHost.props.style)).toMatchObject({
                width: 280,
                left: 0,
                top: 0,
            });
            expect(atom.props.atomProps.selected).toBe(false);
            act(() => {
                nativeView.props.onAtomLayout({
                    nativeEvent: {
                        editorId: handle.editorId,
                        width: 280,
                        positions: [{ key: 'client-1:9', x: 24, y: 34, width: 240 }],
                    },
                });
            });
            expect(StyleSheet.flatten(atomHost.props.style)).toMatchObject({
                width: 240,
                left: 0,
                top: 0,
            });
            expect(atom.props.atomProps.isViewer).toBe(false);
            expect(atom.props.atomProps.readOnly).toBe(false);

            act(() => {
                nativeView.props.onSelectionChange({
                    nativeEvent: {
                        editorId: handle.editorId,
                        anchor: 0,
                        head: 2,
                        stateJson: JSON.stringify({ selection: { type: 'node', pos: 1 } }),
                    },
                });
            });
            expect(getByTestId('counter-atom').props.atomProps.selected).toBe(true);

            act(() => {
                nativeView.props.onSelectionChange({
                    nativeEvent: {
                        editorId: handle.editorId,
                        anchor: 0,
                        head: 0,
                        stateJson: JSON.stringify({
                            selection: { type: 'text', anchor: 0, head: 0 },
                        }),
                    },
                });
            });
            expect(getByTestId('counter-atom').props.atomProps.selected).toBe(false);
            platformSpy.restore();
            handle.destroy();
        }
    );

    it('refreshes atom children after JS and remote document changes', () => {
        const { definition } = counterAtomDefinition();
        let renderSource:
            | { renderBlocks: RenderElement[][]; renderPatch: null }
            | { renderBlocks: null; renderPatch: RenderBlocksPatch } = {
            renderBlocks: [],
            renderPatch: null,
        };
        installAtomRenderSource(() => renderSource);
        const localHandle = createV2LocalHandle(V2_INITIAL_DOC);
        const localRef = createRef<NativeRichTextEditorRef>();
        const local = render(
            <NativeRichTextEditor
                ref={localRef}
                documentHandle={localHandle}
                atoms={[definition]}
            />
        );
        act(() => {
            local.getByTestId('native-editor-view').props.onAtomLayout({
                nativeEvent: { editorId: localHandle.editorId, width: 240 },
            });
        });
        expect(local.queryByTestId('counter-atom')).toBeNull();

        renderSource = { renderBlocks: atomBlock(), renderPatch: null };
        act(() =>
            localRef.current!.insertContentJson(definition.buildFragmentJson({ title: 'a' }))
        );
        expect(local.getByTestId('counter-atom')).toBeTruthy();
        local.unmount();
        localHandle.destroy();

        const roomHandle = createV2RoomHandle({ withSnapshot: true });
        const { controller } = setupV2Controller(roomHandle);
        renderSource = { renderBlocks: [], renderPatch: null };
        const remote = render(
            <NativeRichTextEditor
                documentHandle={roomHandle}
                documentRevision={controller.state.documentRevision}
                atoms={[definition]}
            />
        );
        act(() => {
            remote.getByTestId('native-editor-view').props.onAtomLayout({
                nativeEvent: { editorId: roomHandle.editorId, width: 240 },
            });
            controller.connect();
            v2Runtime.transportOpen(roomHandle.editorId);
            v2Runtime.transportReceive(roomHandle.editorId, V2_FAKE_STEP2_FRAME);
        });
        renderSource = { renderBlocks: atomBlock(), renderPatch: null };
        act(() => {
            v2Runtime.pushRemoteDoc(roomHandle.editorId, V2_SERVER_UPDATE_DOC);
            v2Runtime.transportReceive(roomHandle.editorId, V2_FAKE_UPDATE_FRAME);
        });
        remote.rerender(
            <NativeRichTextEditor
                documentHandle={roomHandle}
                documentRevision={controller.state.documentRevision}
                atoms={[definition]}
            />
        );
        expect(remote.getByTestId('counter-atom')).toBeTruthy();
        roomHandle.destroy();
    });

    it('applies native render patches after the atom seed', () => {
        const { definition } = counterAtomDefinition();
        let renderSource:
            | { renderBlocks: RenderElement[][]; renderPatch: null }
            | { renderBlocks: null; renderPatch: RenderBlocksPatch } = {
            renderBlocks: [],
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

        act(() => {
            const baseDocumentVersion = handle.bridge.getState().documentRevision;
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
                renderBlocks: null,
                renderPatch: {
                    baseDocumentVersion,
                    startIndex: 0,
                    deleteCount: 0,
                    renderBlocks: atomBlock(),
                },
            };
            view.props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            });
        });
        expect(getByTestId('counter-atom')).toBeTruthy();

        act(() => {
            const baseDocumentVersion = handle.bridge.getState().documentRevision;
            v2Runtime.module.editorV2ApplyInput(
                handle.editorId,
                JSON.stringify({
                    version: 1,
                    requestId: '2',
                    baseDocumentRevision: handle.bridge.getState().documentRevision,
                    text: '?',
                })
            );
            renderSource = {
                renderBlocks: null,
                renderPatch: {
                    baseDocumentVersion,
                    startIndex: 0,
                    deleteCount: 1,
                    renderBlocks: [],
                },
            };
            view.props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: renderUpdateValue(handle.editorId),
                    documentRevision: handle.bridge.getState().documentRevision,
                },
            });
        });
        expect(queryByTestId('counter-atom')).toBeNull();
        handle.destroy();
    });

    it('resyncs instead of applying an atom patch to the wrong render base', () => {
        const { definition } = counterAtomDefinition();
        let renderSource:
            | { renderBlocks: RenderElement[][]; renderPatch: null }
            | { renderBlocks: null; renderPatch: RenderBlocksPatch } = {
            renderBlocks: atomBlock('counterCard', 1, 'atom-a'),
            renderPatch: null,
        };
        installAtomRenderSource(() => renderSource);
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        const { getByTestId, getAllByTestId, UNSAFE_getByProps } = render(
            <NativeRichTextEditor documentHandle={handle} atoms={[definition]} />
        );
        const view = getByTestId('native-editor-view');
        act(() => {
            view.props.onAtomLayout({
                nativeEvent: { editorId: handle.editorId, width: 200 },
            });
        });
        const initialRevision = handle.bridge.getState().documentRevision;

        v2Runtime.module.editorV2ApplyInput(
            handle.editorId,
            JSON.stringify({
                version: 1,
                requestId: '1',
                baseDocumentRevision: initialRevision,
                text: '!',
            })
        );
        const queuedRevision = handle.bridge.getState().documentRevision;
        const queued = JSON.parse(renderUpdateValue(handle.editorId)) as Record<string, unknown>;
        queued.renderBlocks = null;
        queued.renderPatch = {
            baseDocumentVersion: initialRevision,
            startIndex: 0,
            deleteCount: 1,
            renderBlocks: atomBlock('counterCard', 2, 'atom-b'),
        };
        handle.bridge.replaceDocument({ setJson: V2_DOC_B, history: 'undoableBoundary' });

        act(() => {
            view.props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: JSON.stringify(queued),
                    documentRevision: queuedRevision,
                },
            });
        });

        renderSource = {
            renderBlocks: atomBlock('counterCard', 3, 'atom-c'),
            renderPatch: null,
        };
        const currentRevision = handle.bridge.getState().documentRevision;
        const current = JSON.parse(renderUpdateValue(handle.editorId)) as Record<string, unknown>;
        current.renderBlocks = null;
        current.renderPatch = {
            baseDocumentVersion: queuedRevision,
            startIndex: 1,
            deleteCount: 0,
            renderBlocks: atomBlock('counterCard', 3, 'atom-c'),
        };

        act(() => {
            view.props.onEditorUpdate({
                nativeEvent: {
                    editorId: handle.editorId,
                    updateJson: JSON.stringify(current),
                    documentRevision: currentRevision,
                },
            });
        });

        expect(getAllByTestId('counter-atom')).toHaveLength(1);
        expect(getByTestId('counter-atom').props.atomProps.attrs.title).toBe('a');
        expect(UNSAFE_getByProps({ nativeID: 'prose-atom:atom-c' })).toBeTruthy();
        handle.destroy();
    });

    it('auto-registers unknown atom chips', () => {
        const warn = jest.spyOn(console, 'warn').mockImplementation(() => undefined);
        const handle = createV2LocalHandle(V2_INITIAL_DOC);
        installAtomRenderSource(() => ({
            renderBlocks: atomBlock('callout', 1),
            renderPatch: null,
        }));
        const { getByTestId, getByText } = render(<NativeRichTextEditor documentHandle={handle} />);
        const view = getByTestId('native-editor-view');
        expect(JSON.parse(view.props.atomsJson)).toEqual({
            nodeTypes: ['callout'],
            estimatedHeights: { callout: DEFAULT_ATOM_CHIP_HEIGHT },
        });
        act(() => {
            view.props.onAtomLayout({
                nativeEvent: { editorId: handle.editorId, width: 200 },
            });
        });
        expect(getByText('callout')).toBeTruthy();
        expect(warn).toHaveBeenCalledTimes(1);
        warn.mockRestore();
        handle.destroy();
    });
});
