import './helpers/NativeEditorBridgeV2Fixture';
import {
    HUGE_U64_DECIMAL,
    MOCK_ATOMIC_RENDER_SNAPSHOT,
    okRecord,
    mockNativeModule,
    createHandle,
    compileTypeScriptContractFixture,
    expectNonRetryable,
    catchRejectedNativeRecord,
} from './helpers/NativeEditorBridgeV2Fixture';

describe('NativeEditorBridge v2', () => {
    describe('atomic render snapshots', () => {
        it('returns one deeply frozen typed snapshot with exact revisions and state', () => {
            const handle = createHandle();
            const snapshot = handle.bridge.renderUpdate();

            expect(snapshot).toEqual(MOCK_ATOMIC_RENDER_SNAPSHOT);
            expect(snapshot.documentVersion).toBe(HUGE_U64_DECIMAL);
            expect(snapshot.stateRevision).toBe('3');
            expect(snapshot.scalarLength).toBe(11);
            expect(Object.isFrozen(snapshot)).toBe(true);
            expect(Object.isFrozen(snapshot.renderBlocks)).toBe(true);
            expect(Object.isFrozen(snapshot.renderBlocks[0])).toBe(true);
            expect(Object.isFrozen(snapshot.selection)).toBe(true);
            expect(Object.isFrozen(snapshot.activeState.marks)).toBe(true);
        });

        it('passes an exact optional mirror while retaining the atomic result shape', () => {
            const handle = createHandle();
            handle.bridge.renderUpdate({ anchor: 2, head: 5 });
            expect(mockNativeModule.editorV2RenderUpdate).toHaveBeenLastCalledWith(
                handle.editorId,
                2,
                5
            );
        });

        it('accepts JSON-safe custom attributes on nested render marks', () => {
            const expected = {
                ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                renderBlocks: [
                    [
                        {
                            type: 'textRun',
                            text: 'linked text',
                            marks: [
                                {
                                    type: 'link',
                                    href: 'https://example.test',
                                    metadata: { source: 'test', offsets: [0, 11, null] },
                                },
                            ],
                        },
                    ],
                ],
            };
            const handle = createHandle();
            mockNativeModule.editorV2RenderUpdate.mockReturnValueOnce(
                okRecord(JSON.stringify(expected))
            );

            expect(handle.bridge.renderUpdate()).toEqual(expected);
        });

        it('accepts an optional string atomId on voidBlock', () => {
            const expected = {
                ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                renderBlocks: [
                    [
                        {
                            type: 'voidBlock',
                            nodeType: 'counterCard',
                            docPos: 1,
                            atomId: 'y1-2',
                        },
                    ],
                ],
            };
            const handle = createHandle();
            mockNativeModule.editorV2RenderUpdate.mockReturnValueOnce(
                okRecord(JSON.stringify(expected))
            );

            expect(handle.bridge.renderUpdate()).toEqual(expected);
        });

        it('requires the native render base revision on incremental snapshots', () => {
            const expected = {
                ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                renderBlocks: null,
                renderPatch: {
                    baseDocumentVersion: '4',
                    startIndex: 0,
                    deleteCount: 1,
                    renderBlocks: MOCK_ATOMIC_RENDER_SNAPSHOT.renderBlocks,
                },
            };
            const handle = createHandle();
            mockNativeModule.editorV2RenderUpdate.mockReturnValueOnce(
                okRecord(JSON.stringify(expected))
            );

            expect(handle.bridge.renderUpdate()).toEqual(expected);

            const missingBase = {
                ...expected,
                renderPatch: { ...expected.renderPatch, baseDocumentVersion: undefined },
            };
            delete (missingBase.renderPatch as { baseDocumentVersion?: string })
                .baseDocumentVersion;
            mockNativeModule.editorV2RenderUpdate.mockReturnValueOnce(
                okRecord(JSON.stringify(missingBase))
            );
            expectNonRetryable(
                catchRejectedNativeRecord(() => handle.bridge.renderUpdate()),
                'FFI_RESULT_INVALID'
            );
        });

        const missingStateRevision = { ...MOCK_ATOMIC_RENDER_SNAPSHOT } as Record<string, unknown>;
        delete missingStateRevision.stateRevision;
        const missingSelection = { ...MOCK_ATOMIC_RENDER_SNAPSHOT } as Record<string, unknown>;
        delete missingSelection.selection;
        // The core emits documentIsEmpty on every render update. A payload
        // without it is a core that disagrees with this boundary, which is
        // exactly the drift that reached the device.
        const missingDocumentIsEmpty = { ...MOCK_ATOMIC_RENDER_SNAPSHOT } as Record<
            string,
            unknown
        >;
        delete missingDocumentIsEmpty.documentIsEmpty;
        it.each<[string, Record<string, unknown>]>([
            ['missing stateRevision', missingStateRevision],
            ['missing documentIsEmpty', missingDocumentIsEmpty],
            [
                'non-boolean documentIsEmpty',
                { ...MOCK_ATOMIC_RENDER_SNAPSHOT, documentIsEmpty: 'false' },
            ],
            ['numeric documentVersion', { ...MOCK_ATOMIC_RENDER_SNAPSHOT, documentVersion: 4 }],
            [
                'out-of-range scalarLength',
                { ...MOCK_ATOMIC_RENDER_SNAPSHOT, scalarLength: 0x1_0000_0000 },
            ],
            ['missing selection', missingSelection],
            [
                'malformed historyState',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    historyState: { canUndo: 1, canRedo: false },
                },
            ],
            [
                'malformed renderBlocks',
                { ...MOCK_ATOMIC_RENDER_SNAPSHOT, renderBlocks: [[{ type: 'surprise' }]] },
            ],
            [
                'numeric voidBlock atomId',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [
                        [
                            {
                                type: 'voidBlock',
                                nodeType: 'counterCard',
                                docPos: 1,
                                atomId: 7,
                            },
                        ],
                    ],
                },
            ],
            [
                'voidInline atomId',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [
                        [
                            {
                                type: 'voidInline',
                                nodeType: 'hardBreak',
                                docPos: 1,
                                atomId: 'y1-2',
                            },
                        ],
                    ],
                },
            ],
            [
                'unexpected nested render element field',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [
                        [{ type: 'blockStart', nodeType: 'paragraph', depth: 0, extra: true }],
                    ],
                },
            ],
            [
                'nested mark without a required type',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [
                        [
                            {
                                type: 'textRun',
                                text: 'text',
                                marks: [{ href: 'https://example.test' }],
                            },
                        ],
                    ],
                },
            ],
            [
                'nested mark with a non-string type',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [[{ type: 'textRun', text: 'text', marks: [{ type: 1 }] }]],
                },
            ],
            [
                'mention theme with an unknown field',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [
                        [
                            {
                                type: 'opaqueInlineAtom',
                                nodeType: 'mention',
                                label: 'Alice',
                                docPos: 1,
                                mentionTheme: { extra: true },
                            },
                        ],
                    ],
                },
            ],
            [
                'mention theme with a non-string color',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [
                        [
                            {
                                type: 'opaqueInlineAtom',
                                nodeType: 'mention',
                                label: 'Alice',
                                docPos: 1,
                                mentionTheme: { textColor: 1 },
                            },
                        ],
                    ],
                },
            ],
            [
                'mention theme with a non-numeric border width',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [
                        [
                            {
                                type: 'opaqueInlineAtom',
                                nodeType: 'mention',
                                label: 'Alice',
                                docPos: 1,
                                mentionTheme: { borderWidth: '1' },
                            },
                        ],
                    ],
                },
            ],
            [
                'mention theme with an unsupported font weight',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [
                        [
                            {
                                type: 'opaqueInlineAtom',
                                nodeType: 'mention',
                                label: 'Alice',
                                docPos: 1,
                                mentionTheme: { fontWeight: 'semibold' },
                            },
                        ],
                    ],
                },
            ],
            [
                'mention theme with null in an optional field',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [
                        [
                            {
                                type: 'opaqueInlineAtom',
                                nodeType: 'mention',
                                label: 'Alice',
                                docPos: 1,
                                mentionTheme: { suggestions: { borderRadius: null } },
                            },
                        ],
                    ],
                },
            ],
            ['unknown top-level field', { ...MOCK_ATOMIC_RENDER_SNAPSHOT, unexpected: true }],
        ])('rejects %s', (_name, malformed) => {
            const handle = createHandle();
            mockNativeModule.editorV2RenderUpdate.mockReturnValueOnce(
                okRecord(JSON.stringify(malformed))
            );
            expectNonRetryable(
                catchRejectedNativeRecord(() => handle.bridge.renderUpdate()),
                'FFI_RESULT_INVALID'
            );
        });

        it('accepts an inserted mention atom carrying its node attrs', () => {
            const handle = createHandle();
            // Rust emits `attrs` on every void/opaque element, so a document
            // holding a mention must normalize rather than poison every read.
            mockNativeModule.editorV2RenderUpdate.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                        renderBlocks: [
                            [
                                { depth: 0, nodeType: 'paragraph', type: 'blockStart' },
                                {
                                    type: 'opaqueInlineAtom',
                                    nodeType: 'mention',
                                    label: '@Alice Chen',
                                    docPos: 1,
                                    attrs: {
                                        id: 'user-alice',
                                        label: 'Alice Chen',
                                        mentionSuggestionChar: '@',
                                        type: 'user',
                                        mentionTheme: {
                                            node: { textColor: '#336EC1' },
                                            suggestions: { option: { textColor: '#336EC1' } },
                                        },
                                    },
                                    mentionTheme: {
                                        node: { textColor: '#336EC1' },
                                        suggestions: { option: { textColor: '#336EC1' } },
                                    },
                                },
                                { type: 'blockEnd' },
                            ],
                        ],
                    })
                )
            );

            const update = handle.bridge.renderUpdate();
            expect(update.renderBlocks?.[0]?.[1]).toEqual(
                expect.objectContaining({ type: 'opaqueInlineAtom', nodeType: 'mention' })
            );
        });

        it('accepts an opaque block atom carrying its node attrs', () => {
            const handle = createHandle();
            mockNativeModule.editorV2RenderUpdate.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                        renderBlocks: [
                            [
                                {
                                    type: 'opaqueBlockAtom',
                                    nodeType: 'customBlock',
                                    label: 'custom',
                                    docPos: 1,
                                    attrs: { id: 'block-1' },
                                },
                            ],
                        ],
                    })
                )
            );

            expect(() => handle.bridge.renderUpdate()).not.toThrow();
        });

        it('rejects a non-finite nested render mark attribute', () => {
            const malformed = JSON.stringify({
                ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                renderBlocks: [
                    [
                        {
                            type: 'textRun',
                            text: 'text',
                            marks: [{ type: 'link', score: 0 }],
                        },
                    ],
                ],
            }).replace('"score":0', '"score":1e999');
            const handle = createHandle();
            mockNativeModule.editorV2RenderUpdate.mockReturnValueOnce(okRecord(malformed));

            expectNonRetryable(
                catchRejectedNativeRecord(() => handle.bridge.renderUpdate()),
                'FFI_RESULT_INVALID'
            );
        });

        it('type-checks atomic snapshots as deeply readonly without freezing EditorUpdate', () => {
            const diagnostics = compileTypeScriptContractFixture(`
                import type {
                    EditorUpdate,
                    NativeEditorAtomicRenderSnapshot,
                } from '../index';

                declare const snapshot: NativeEditorAtomicRenderSnapshot;

                // @ts-expect-error atomic arrays are readonly
                snapshot.renderBlocks.push([]);
                // @ts-expect-error nested atomic arrays are readonly
                snapshot.renderBlocks[0].push({ type: 'blockEnd' });
                // @ts-expect-error atomic elements are readonly
                snapshot.renderBlocks[0][0].type = 'blockEnd';
                // @ts-expect-error snapshot selection is readonly
                snapshot.selection = { type: 'all' };
                if (snapshot.selection.type === 'text') {
                    // @ts-expect-error snapshot selection fields are readonly
                    snapshot.selection.anchor = 1;
                }
                // @ts-expect-error snapshot active-state maps are readonly
                snapshot.activeState.marks.bold = true;
                // @ts-expect-error nested snapshot active-state maps are readonly
                snapshot.activeState.markAttrs.bold = {};
                // @ts-expect-error snapshot history state is readonly
                snapshot.historyState.canUndo = false;
                if (snapshot.renderPatch != null) {
                    // @ts-expect-error snapshot patch fields are readonly
                    snapshot.renderPatch.startIndex = 1;
                    // @ts-expect-error snapshot patch blocks are readonly
                    snapshot.renderPatch.renderBlocks.push([]);
                }

                const update: EditorUpdate = {
                    renderElements: [],
                    renderBlocks: [],
                    renderPatch: {
                        baseDocumentVersion: '0',
                        startIndex: 0,
                        deleteCount: 0,
                        renderBlocks: [],
                    },
                    selection: { type: 'all' },
                    activeState: {
                        marks: {},
                        markAttrs: {},
                        nodes: {},
                        commands: {},
                        allowedMarks: [],
                        insertableNodes: [],
                    },
                    historyState: { canUndo: false, canRedo: false },
                };
                update.renderElements.push({ type: 'blockEnd' });
                update.renderBlocks!.push([]);
                update.renderPatch!.renderBlocks.push([]);
                update.activeState.marks.bold = true;
                update.historyState.canUndo = true;
            `);

            expect(diagnostics).toBe('');
        });
    });
});
