import './helpers/NativeEditorBridgeV2Fixture';
import {
    MOCK_SNAPSHOT_METADATA,
    MOCK_SNAPSHOT_BYTES,
    okRecord,
    mockNativeModule,
    createHandle,
    compileTypeScriptContractFixture,
    emitNativeEditorBridgeDeclaration,
    catchThrown,
} from './helpers/NativeEditorBridgeV2Fixture';
import {
    createNativeEditorDocumentHandle,
    type NativeEditorDocumentHandle,
    type NativeEditorV2CreateConfig,
} from '../NativeEditorBridge';
import * as NativeEditorBridgeExports from '../NativeEditorBridge';
import {
    NativeEditorBoundaryError,
    NativeEditorV2BoundaryError,
    NativeEditorV2ErrorBase,
} from '../NativeEditorBoundaryError';
import { HARD_EDITOR_RESOURCE_LIMITS } from '../ResourceLimits';

describe('NativeEditorBridge v2', () => {
    describe('document handle lifecycle', () => {
        it('type-checks only the exact grouped create shape and sole factory constructor', () => {
            const diagnostics = compileTypeScriptContractFixture(`
                import {
                    createNativeEditorDocumentHandle,
                    type NativeEditorDocumentHandle,
                    type NativeEditorV2CreateConfig,
                } from '../NativeEditorBridge';
                import type {
                    EditorCollaborationLimits,
                    EditorEditingLimits,
                    EditorResourceLimits,
                } from '../ResourceLimits';

                const resource: EditorResourceLimits = {
                    maxInputBytes: 1,
                    maxDocumentNodes: 1,
                    maxDocumentDepth: 1,
                    maxSchemaNodes: 1,
                    maxSchemaExpressionBytes: 1,
                    maxCollaborationMessageBytes: 1,
                    maxEncodedStateBytes: 1,
                };
                const editing: EditorEditingLimits = {
                    maxOperationsPerTransaction: 1,
                    maxUndoGroups: 1,
                    maxUndoRetainedUnits: 1,
                    maxDerivedOutputBytes: 1,
                };
                const collaboration: EditorCollaborationLimits = {
                    maxFramesPerMessage: 1,
                    maxFrameBytes: 1,
                    maxAggregateResponseBytes: 1,
                    maxAwarenessPeers: 1,
                    maxAwarenessPeerBytes: 1,
                    maxAwarenessBytes: 1,
                    maxPendingOutboxMessages: 1,
                    maxPendingOutboxBytes: 1,
                    maxPendingDependencyUpdateBytes: 1,
                    maxPendingDependencyUpdateWork: 1,
                };
                const config: NativeEditorV2CreateConfig = {
                    initialization: { type: 'localEmpty' },
                    schema: undefined,
                    fragmentName: 'prosemirror',
                    policy: {
                        maxLength: 100,
                        readOnly: true,
                        inputFilter: '[a-z]',
                        allowBase64Images: false,
                    },
                    limits: { resource, editing, collaboration },
                };
                createNativeEditorDocumentHandle(config);
                const removedRootPolicy: NativeEditorV2CreateConfig = {
                    initialization: { type: 'localEmpty' },
                    // @ts-expect-error maxLength belongs under policy
                    maxLength: 100,
                };
                void removedRootPolicy;
                // @ts-expect-error the class has no public static create constructor
                NativeEditorDocumentHandle.create(config);
            `);
            expect(diagnostics).toBe('');
        });

        it('omits the removed static constructor from declaration output', () => {
            const { declaration, diagnostics } = emitNativeEditorBridgeDeclaration();
            expect(diagnostics).toBe('');
            expect(declaration).toContain(
                'export declare function createNativeEditorDocumentHandle(config: NativeEditorV2CreateConfig): NativeEditorDocumentHandle;'
            );
            expect(declaration).toContain('export interface NativeEditorDocumentHandle');
            expect(declaration).not.toMatch(/static create\s*\(/);
        });

        it('does not expose a runtime document-handle constructor', () => {
            const runtimeConstructor = (
                NativeEditorBridgeExports as unknown as Record<string, unknown>
            ).NativeEditorDocumentHandle;
            expect(runtimeConstructor).toBeUndefined();
        });

        it('creates a handle with a decimal-string editorId and its bridge', () => {
            const handle = createHandle();
            expect(handle.editorId).toBe('1');
            expect(handle.bridge.editorId).toBe('1');
            expect(handle.isDestroyed).toBe(false);
            expect(handle.bridge.isDestroyed).toBe(false);
        });

        it('serializes the local initialization create envelope exactly', () => {
            createNativeEditorDocumentHandle({
                schema: { nodes: [], marks: [] } as never,
                fragmentName: 'prosemirror',
                initialization: { type: 'localJson', json: { type: 'doc', content: [] } },
                policy: {
                    maxLength: 100,
                    readOnly: true,
                    inputFilter: '[a-z]',
                    allowBase64Images: true,
                },
                limits: {
                    resource: {
                        maxInputBytes: 64 * 1024 * 1024,
                        maxDocumentNodes: 1_000_000,
                        maxDocumentDepth: 1_024,
                        maxSchemaNodes: 10_000,
                        maxSchemaExpressionBytes: 1024 * 1024,
                        maxCollaborationMessageBytes: 64 * 1024 * 1024,
                        maxEncodedStateBytes: 256 * 1024 * 1024,
                    },
                    editing: {
                        maxOperationsPerTransaction: 4_096,
                        maxUndoGroups: 2_000,
                        maxUndoRetainedUnits: 8_000_000,
                        maxDerivedOutputBytes: 128 * 1024 * 1024,
                    },
                    collaboration: {
                        maxFramesPerMessage: 1_024,
                        maxFrameBytes: 64 * 1024 * 1024,
                        maxAggregateResponseBytes: 64 * 1024 * 1024,
                        maxAwarenessPeers: 10_000,
                        maxAwarenessPeerBytes: 1024 * 1024,
                        maxAwarenessBytes: 64 * 1024 * 1024,
                        maxPendingOutboxMessages: 4_096,
                        maxPendingOutboxBytes: 64 * 1024 * 1024,
                        maxPendingDependencyUpdateBytes: 64 * 1024 * 1024,
                        maxPendingDependencyUpdateWork: 8_000_000,
                    },
                },
            });
            expect(mockNativeModule.editorV2Create).toHaveBeenCalledTimes(1);
            const [configJson, snapshotState] = mockNativeModule.editorV2Create.mock.calls[0];
            expect(JSON.parse(configJson)).toEqual({
                schema: { nodes: [], marks: [] },
                fragmentName: 'prosemirror',
                initialization: { type: 'localJson', json: { type: 'doc', content: [] } },
                policy: {
                    maxLength: 100,
                    readOnly: true,
                    inputFilter: '[a-z]',
                    allowBase64Images: true,
                },
                limits: {
                    resource: {
                        maxInputBytes: 64 * 1024 * 1024,
                        maxDocumentNodes: 1_000_000,
                        maxDocumentDepth: 1_024,
                        maxSchemaNodes: 10_000,
                        maxSchemaExpressionBytes: 1024 * 1024,
                        maxCollaborationMessageBytes: 64 * 1024 * 1024,
                        maxEncodedStateBytes: 256 * 1024 * 1024,
                    },
                    editing: {
                        maxOperationsPerTransaction: 4_096,
                        maxUndoGroups: 2_000,
                        maxUndoRetainedUnits: 8_000_000,
                        maxDerivedOutputBytes: 128 * 1024 * 1024,
                    },
                    collaboration: {
                        maxFramesPerMessage: 1_024,
                        maxFrameBytes: 64 * 1024 * 1024,
                        maxAggregateResponseBytes: 64 * 1024 * 1024,
                        maxAwarenessPeers: 10_000,
                        maxAwarenessPeerBytes: 1024 * 1024,
                        maxAwarenessBytes: 64 * 1024 * 1024,
                        maxPendingOutboxMessages: 4_096,
                        maxPendingOutboxBytes: 64 * 1024 * 1024,
                        maxPendingDependencyUpdateBytes: 64 * 1024 * 1024,
                        maxPendingDependencyUpdateWork: 8_000_000,
                    },
                },
            });
            expect(snapshotState).toBeNull();
        });

        it('omits undefined optional fields from custom schemas', () => {
            createNativeEditorDocumentHandle({
                initialization: { type: 'localEmpty' },
                schema: {
                    nodes: [
                        {
                            name: 'doc',
                            content: 'paragraph+',
                            role: 'doc',
                            group: undefined,
                        },
                        {
                            name: 'paragraph',
                            content: '',
                            role: 'textBlock',
                            group: 'block',
                        },
                    ],
                    marks: [],
                },
            });

            const [configJson] = mockNativeModule.editorV2Create.mock.calls[0];
            expect(JSON.parse(configJson)).toEqual({
                schema: {
                    nodes: [
                        { name: 'doc', content: 'paragraph+', role: 'doc' },
                        {
                            name: 'paragraph',
                            content: '',
                            role: 'textBlock',
                            group: 'block',
                        },
                    ],
                    marks: [],
                },
                initialization: { type: 'localEmpty' },
            });
        });

        it('omits undefined object fields from local JSON initialization', () => {
            createNativeEditorDocumentHandle({
                initialization: {
                    type: 'localJson',
                    json: {
                        type: 'doc',
                        attrs: { title: undefined, category: 'notes' },
                        content: [],
                    },
                },
            });

            const [configJson] = mockNativeModule.editorV2Create.mock.calls[0];
            expect(JSON.parse(configJson)).toEqual({
                initialization: {
                    type: 'localJson',
                    json: {
                        type: 'doc',
                        attrs: { category: 'notes' },
                        content: [],
                    },
                },
            });
        });

        it('creates an exact semantic-depth-1024 local JSON document without stack recursion', () => {
            const maxDepth = HARD_EDITOR_RESOURCE_LIMITS.maxDocumentDepth;
            let deepest: Record<string, unknown> = { type: 'paragraph' };
            let expectedDocumentJson = '{"type":"paragraph"}';
            for (let depth = 2; depth < maxDepth; depth += 1) {
                deepest = { type: 'blockquote', content: [deepest] };
                expectedDocumentJson = `{"type":"blockquote","content":[${expectedDocumentJson}]}`;
            }
            const document = { type: 'doc', content: [deepest] };

            expect(() =>
                createNativeEditorDocumentHandle({
                    initialization: { type: 'localJson', json: document },
                })
            ).not.toThrow();

            expect(mockNativeModule.editorV2Create).toHaveBeenCalledTimes(1);
            const [configJson, snapshotState] = mockNativeModule.editorV2Create.mock.calls[0];
            expect(configJson).toBe(
                `{"initialization":{"type":"localJson","json":{"type":"doc","content":[${expectedDocumentJson}]}}}`
            );
            expect(configJson.match(/"type":"blockquote"/g)).toHaveLength(maxDepth - 2);
            expect(snapshotState).toBeNull();
        });

        it('rejects unknown and removed create fields before native invocation', () => {
            const invalidConfigs: unknown[] = [
                { initialization: { type: 'localEmpty' }, unknown: true },
                { initialization: { type: 'localEmpty', unknown: true } },
                {
                    initialization: {
                        type: 'localJson',
                        json: { type: 'doc' },
                        unknown: true,
                    },
                },
                {
                    initialization: {
                        type: 'localHtml',
                        html: '<p>x</p>',
                        unknown: true,
                    },
                },
                {
                    initialization: {
                        type: 'room',
                        documentId: 'doc-1',
                        lineageId: 'lineage-1',
                        unknown: true,
                    },
                },
                { initialization: { type: 'localEmpty' }, maxLength: 1 },
                { initialization: { type: 'localEmpty' }, readOnly: true },
                { initialization: { type: 'localEmpty' }, inputFilter: 'x' },
                { initialization: { type: 'localEmpty' }, allowBase64Images: true },
                { initialization: { type: 'localEmpty' }, policy: { unknown: true } },
                { initialization: { type: 'localEmpty' }, limits: { unknown: {} } },
                {
                    initialization: { type: 'localEmpty' },
                    limits: { resource: { unknown: 1 } },
                },
                {
                    initialization: { type: 'localEmpty' },
                    limits: { editing: { unknown: 1 } },
                },
                {
                    initialization: { type: 'localEmpty' },
                    limits: { collaboration: { unknown: 1 } },
                },
            ];

            for (const config of invalidConfigs) {
                const error = catchThrown(() =>
                    createNativeEditorDocumentHandle(config as NativeEditorV2CreateConfig)
                );
                expect((error as { code?: string }).code).toBe('CONFIG_INVALID');
            }
            expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        });

        it('rejects arbitrary prototypes and inherited create values before native invocation', () => {
            const inheritedRoot = Object.create({ inherited: true }) as Record<string, unknown>;
            inheritedRoot.initialization = { type: 'localEmpty' };

            const inheritedInitialization = Object.create({ type: 'localEmpty' }) as Record<
                string,
                unknown
            >;

            const inheritedPolicy = Object.create({ maxLength: 100 }) as Record<string, unknown>;
            const inheritedResource = Object.create({ maxInputBytes: 1024 }) as Record<
                string,
                unknown
            >;

            const invalidConfigs: unknown[] = [
                inheritedRoot,
                { initialization: inheritedInitialization },
                { initialization: { type: 'localEmpty' }, policy: inheritedPolicy },
                {
                    initialization: { type: 'localEmpty' },
                    limits: { resource: inheritedResource },
                },
            ];

            for (const config of invalidConfigs) {
                const error = catchThrown(() =>
                    createNativeEditorDocumentHandle(config as NativeEditorV2CreateConfig)
                );
                expect((error as { code?: string }).code).toBe('CONFIG_INVALID');
            }
            expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        });

        it('rejects accessor-backed contract fields without invoking them', () => {
            let getterCalls = 0;
            const root = {} as Record<string, unknown>;
            Object.defineProperty(root, 'initialization', {
                enumerable: true,
                get() {
                    getterCalls += 1;
                    throw new Error('root getter must not run');
                },
            });
            const policy = {} as Record<string, unknown>;
            Object.defineProperty(policy, 'readOnly', {
                enumerable: true,
                get() {
                    getterCalls += 1;
                    throw new Error('policy getter must not run');
                },
            });
            const document = { type: 'doc' } as Record<string, unknown>;
            Object.defineProperty(document, 'content', {
                enumerable: true,
                get() {
                    getterCalls += 1;
                    throw new Error('document getter must not run');
                },
            });

            for (const config of [
                root,
                { initialization: { type: 'localEmpty' }, policy },
                { initialization: { type: 'localJson', json: document } },
            ]) {
                const error = catchThrown(() =>
                    createNativeEditorDocumentHandle(config as NativeEditorV2CreateConfig)
                );
                expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
                expect((error as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
            }
            expect(getterCalls).toBe(0);
            expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        });

        it('translates attacker-thrown boundary errors from contract traps to CONFIG_INVALID', () => {
            const config = new Proxy(
                { initialization: { type: 'localEmpty' } },
                {
                    getPrototypeOf() {
                        throw new NativeEditorBoundaryError(
                            'INVALID_RESOURCE_LIMIT',
                            'attacker-controlled trap'
                        );
                    },
                }
            );

            const error = catchThrown(() =>
                createNativeEditorDocumentHandle(config as NativeEditorV2CreateConfig)
            );
            expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
            expect((error as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
            expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        });

        it('rejects attacker toJSON hooks without invoking them', () => {
            let toJsonCalls = 0;
            const schema = {
                nodes: [],
                marks: [],
                toJSON() {
                    toJsonCalls += 1;
                    return { nodes: [], marks: [] };
                },
            };
            const document = {
                type: 'doc',
                toJSON() {
                    toJsonCalls += 1;
                    throw new Error('document toJSON must not run');
                },
            };

            for (const config of [
                { initialization: { type: 'localEmpty' }, schema },
                { initialization: { type: 'localJson', json: document } },
            ]) {
                const error = catchThrown(() =>
                    createNativeEditorDocumentHandle(
                        config as unknown as NativeEditorV2CreateConfig
                    )
                );
                expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
                expect((error as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
            }
            expect(toJsonCalls).toBe(0);
            expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        });

        it('serializes through containers that cannot inherit an attacker toJSON hook', () => {
            let toJsonCalls = 0;
            const original = Object.getOwnPropertyDescriptor(Object.prototype, 'toJSON');
            Object.defineProperty(Object.prototype, 'toJSON', {
                configurable: true,
                value() {
                    toJsonCalls += 1;
                    throw new Error('inherited toJSON must not run');
                },
            });
            mockNativeModule.editorV2Create.mockReturnValueOnce(okRecord('{"editorId":"1"}'));

            try {
                createNativeEditorDocumentHandle({
                    schema: { nodes: [], marks: [] } as never,
                    initialization: {
                        type: 'localJson',
                        json: { type: 'doc', content: [] },
                    },
                });
            } finally {
                if (original === undefined) {
                    delete (Object.prototype as { toJSON?: unknown }).toJSON;
                } else {
                    Object.defineProperty(Object.prototype, 'toJSON', original);
                }
            }

            expect(toJsonCalls).toBe(0);
            expect(mockNativeModule.editorV2Create).toHaveBeenCalledTimes(1);
        });

        it('translates cyclic schema and document serialization failures to CONFIG_INVALID', () => {
            const schema: Record<string, unknown> = { nodes: [], marks: [] };
            schema.self = schema;
            const document: Record<string, unknown> = { type: 'doc' };
            document.self = document;

            for (const config of [
                { initialization: { type: 'localEmpty' }, schema },
                { initialization: { type: 'localJson', json: document } },
            ]) {
                const error = catchThrown(() =>
                    createNativeEditorDocumentHandle(
                        config as unknown as NativeEditorV2CreateConfig
                    )
                );
                expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
                expect((error as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
            }
            expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        });

        it('bounds JSON normalization and rejects repeated-reference amplification', () => {
            const captureCode = (config: NativeEditorV2CreateConfig): string => {
                try {
                    createNativeEditorDocumentHandle(config);
                    return 'accepted';
                } catch (error) {
                    return (error as { code?: string }).code ?? 'unstructured';
                }
            };
            const maxBytes = HARD_EDITOR_RESOURCE_LIMITS.maxInputBytes;
            const documentOverhead = JSON.stringify({ payload: '' }).length;
            const exactPayload = 'x'.repeat(maxBytes - documentOverhead);

            expect(
                captureCode({
                    initialization: { type: 'localJson', json: { payload: exactPayload } },
                })
            ).toBe('accepted');

            let amplification: Record<string, unknown> = { value: 'x' };
            for (let depth = 0; depth < 8; depth += 1) {
                amplification = { left: amplification, right: amplification };
            }
            const outcomes = [
                captureCode({
                    initialization: {
                        type: 'localJson',
                        json: { payload: `${exactPayload}x` },
                    },
                }),
                captureCode({
                    initialization: { type: 'localJson', json: amplification },
                }),
            ];

            expect(outcomes).toEqual(['CONFIG_INVALID', 'CONFIG_INVALID']);
            expect(mockNativeModule.editorV2Create).toHaveBeenCalledTimes(1);
        });

        it('validates create policy and metadata scalars before native invocation', () => {
            const invalidConfigs: unknown[] = [
                { initialization: { type: 'localEmpty' }, fragmentName: 1 },
                { initialization: { type: 'localEmpty' }, policy: { maxLength: -1 } },
                { initialization: { type: 'localEmpty' }, policy: { maxLength: 1.5 } },
                {
                    initialization: { type: 'localEmpty' },
                    policy: { maxLength: 0x1_0000_0000 },
                },
                { initialization: { type: 'localEmpty' }, policy: { readOnly: 'true' } },
                { initialization: { type: 'localEmpty' }, policy: { inputFilter: 1 } },
                {
                    initialization: { type: 'localEmpty' },
                    policy: { allowBase64Images: 1 },
                },
                ...Object.keys(MOCK_SNAPSHOT_METADATA).map((field) => ({
                    initialization: {
                        type: 'room',
                        documentId: 'doc-1',
                        lineageId: 'lineage-1',
                        snapshot: {
                            metadata: { ...MOCK_SNAPSHOT_METADATA, [field]: true },
                            encodedState: MOCK_SNAPSHOT_BYTES,
                        },
                    },
                })),
            ];

            for (const config of invalidConfigs) {
                const error = catchThrown(() =>
                    createNativeEditorDocumentHandle(config as NativeEditorV2CreateConfig)
                );
                expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
                expect((error as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
            }
            expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        });
    });
});
