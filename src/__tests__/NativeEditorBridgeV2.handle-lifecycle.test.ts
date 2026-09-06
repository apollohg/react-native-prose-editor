import './helpers/NativeEditorBridgeV2Fixture';
import {
    MOCK_V2_STATE,
    MOCK_SNAPSHOT_METADATA,
    MOCK_SNAPSHOT_BYTES,
    mockV2Error,
    okRecord,
    errRecord,
    mockNativeModule,
    createHandle,
    expectNonRetryable,
    catchThrown,
    catchRejectedNativeRecord,
} from './helpers/NativeEditorBridgeV2Fixture';
import {
    createNativeEditorDocumentHandle,
    type NativeEditorCreateConfig,
} from '../NativeEditorBridge';

import {
    NativeEditorEngineBoundaryError,
    NativeEditorErrorBase,
    NativeEditorOperationError,
} from '../NativeEditorBoundaryError';

describe('NativeEditorBridge v2', () => {
    describe('document handle lifecycle', () => {
        it('serializes normalized own-property copies from null-prototype records', () => {
            const initialization = Object.assign(Object.create(null), { type: 'localEmpty' });
            const policy = Object.assign(Object.create(null), { readOnly: true });
            const resource = Object.assign(Object.create(null), { maxInputBytes: 1024 });
            const limits = Object.assign(Object.create(null), { resource });
            const config = Object.assign(Object.create(null), {
                initialization,
                policy,
                limits,
            }) as NativeEditorCreateConfig;

            createNativeEditorDocumentHandle(config);

            const [configJson] = mockNativeModule.editorV2Create.mock.calls[0];
            expect(JSON.parse(configJson)).toEqual({
                initialization: { type: 'localEmpty' },
                policy: { readOnly: true },
                limits: { resource: { maxInputBytes: 1024 } },
            });
        });

        it('rejects explicit null throughout the complete create contract', () => {
            const invalidConfigs: unknown[] = [
                { initialization: null },
                { initialization: { type: null } },
                { initialization: { type: 'localEmpty' }, schema: null },
                { initialization: { type: 'localEmpty' }, fragmentName: null },
                { initialization: { type: 'localEmpty' }, policy: null },
                { initialization: { type: 'localEmpty' }, limits: null },
                { initialization: { type: 'localJson', json: null } },
                { initialization: { type: 'localHtml', html: null } },
                {
                    initialization: {
                        type: 'room',
                        documentId: null,
                        lineageId: 'lineage-1',
                    },
                },
                {
                    initialization: {
                        type: 'room',
                        documentId: 'doc-1',
                        lineageId: null,
                    },
                },
                {
                    initialization: {
                        type: 'room',
                        documentId: 'doc-1',
                        lineageId: 'lineage-1',
                        snapshot: null,
                    },
                },
                ...Object.keys(MOCK_SNAPSHOT_METADATA).map((field) => ({
                    initialization: {
                        type: 'room',
                        documentId: 'doc-1',
                        lineageId: 'lineage-1',
                        snapshot: {
                            metadata: { ...MOCK_SNAPSHOT_METADATA, [field]: null },
                            encodedState: MOCK_SNAPSHOT_BYTES,
                        },
                    },
                })),
                {
                    initialization: {
                        type: 'room',
                        documentId: 'doc-1',
                        lineageId: 'lineage-1',
                        snapshot: { metadata: null, encodedState: MOCK_SNAPSHOT_BYTES },
                    },
                },
                {
                    initialization: {
                        type: 'room',
                        documentId: 'doc-1',
                        lineageId: 'lineage-1',
                        snapshot: { metadata: MOCK_SNAPSHOT_METADATA, encodedState: null },
                    },
                },
            ];
            for (const field of ['maxLength', 'readOnly', 'inputFilter', 'allowBase64Images']) {
                invalidConfigs.push({
                    initialization: { type: 'localEmpty' },
                    policy: { [field]: null },
                });
            }
            for (const group of ['resource', 'editing', 'collaboration']) {
                invalidConfigs.push({
                    initialization: { type: 'localEmpty' },
                    limits: { [group]: null },
                });
            }
            for (const [group, field] of [
                ['resource', 'maxInputBytes'],
                ['resource', 'maxDocumentNodes'],
                ['resource', 'maxDocumentDepth'],
                ['resource', 'maxSchemaNodes'],
                ['resource', 'maxSchemaExpressionBytes'],
                ['resource', 'maxCollaborationMessageBytes'],
                ['resource', 'maxEncodedStateBytes'],
                ['editing', 'maxOperationsPerTransaction'],
                ['editing', 'maxUndoGroups'],
                ['editing', 'maxUndoRetainedUnits'],
                ['editing', 'maxDerivedOutputBytes'],
                ['collaboration', 'maxFramesPerMessage'],
                ['collaboration', 'maxFrameBytes'],
                ['collaboration', 'maxAggregateResponseBytes'],
                ['collaboration', 'maxAwarenessPeers'],
                ['collaboration', 'maxAwarenessPeerBytes'],
                ['collaboration', 'maxAwarenessBytes'],
                ['collaboration', 'maxPendingOutboxMessages'],
                ['collaboration', 'maxPendingOutboxBytes'],
                ['collaboration', 'maxPendingDependencyUpdateBytes'],
                ['collaboration', 'maxPendingDependencyUpdateWork'],
            ]) {
                invalidConfigs.push({
                    initialization: { type: 'localEmpty' },
                    limits: { [group]: { [field]: null } },
                });
            }

            for (const config of invalidConfigs) {
                const error = catchThrown(() =>
                    createNativeEditorDocumentHandle(config as NativeEditorCreateConfig)
                );
                expect((error as { code?: string }).code).toBe('CONFIG_INVALID');
            }
            expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        });

        it('rejects every non-positive, fractional, unsafe, and one-over integer limit', () => {
            const limitCases: Array<[string, string, number]> = [
                ['resource', 'maxInputBytes', 64 * 1024 * 1024],
                ['resource', 'maxDocumentNodes', 1_000_000],
                ['resource', 'maxDocumentDepth', 1_024],
                ['resource', 'maxSchemaNodes', 10_000],
                ['resource', 'maxSchemaExpressionBytes', 1024 * 1024],
                ['resource', 'maxCollaborationMessageBytes', 64 * 1024 * 1024],
                ['resource', 'maxEncodedStateBytes', 256 * 1024 * 1024],
                ['editing', 'maxOperationsPerTransaction', 4_096],
                ['editing', 'maxUndoGroups', 2_000],
                ['editing', 'maxUndoRetainedUnits', 8_000_000],
                ['editing', 'maxDerivedOutputBytes', 128 * 1024 * 1024],
                ['collaboration', 'maxFramesPerMessage', 1_024],
                ['collaboration', 'maxFrameBytes', 64 * 1024 * 1024],
                ['collaboration', 'maxAggregateResponseBytes', 64 * 1024 * 1024],
                ['collaboration', 'maxAwarenessPeers', 10_000],
                ['collaboration', 'maxAwarenessPeerBytes', 1024 * 1024],
                ['collaboration', 'maxAwarenessBytes', 64 * 1024 * 1024],
                ['collaboration', 'maxPendingOutboxMessages', 4_096],
                ['collaboration', 'maxPendingOutboxBytes', 64 * 1024 * 1024],
                ['collaboration', 'maxPendingDependencyUpdateBytes', 64 * 1024 * 1024],
                ['collaboration', 'maxPendingDependencyUpdateWork', 8_000_000],
            ];

            for (const [group, field, ceiling] of limitCases) {
                for (const value of [0, 1.5, Number.MAX_SAFE_INTEGER + 1, ceiling + 1]) {
                    const config = {
                        initialization: { type: 'localEmpty' },
                        limits: { [group]: { [field]: value } },
                    } as unknown as NativeEditorCreateConfig;
                    const error = catchThrown(() => createNativeEditorDocumentHandle(config));
                    expect((error as { code?: string }).code).toBe('INVALID_RESOURCE_LIMIT');
                }
            }
            expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        });

        it('does not trust a boundary error replayed by a later hostile create input', () => {
            const limitError = catchThrown(() =>
                createNativeEditorDocumentHandle({
                    initialization: { type: 'localEmpty' },
                    limits: { resource: { maxInputBytes: 0 } },
                })
            );
            expect((limitError as NativeEditorErrorBase).code).toBe('INVALID_RESOURCE_LIMIT');

            const replayingConfig = new Proxy(
                {},
                {
                    getPrototypeOf() {
                        throw limitError;
                    },
                }
            );
            const replayed = catchThrown(() =>
                createNativeEditorDocumentHandle(
                    replayingConfig as unknown as NativeEditorCreateConfig
                )
            );

            expect(replayed).not.toBe(limitError);
            expect(replayed).toBeInstanceOf(NativeEditorEngineBoundaryError);
            expect((replayed as NativeEditorErrorBase).code).toBe('CONFIG_INVALID');
            expect(mockNativeModule.editorV2Create).not.toHaveBeenCalled();
        });

        it('serializes the room create envelope with snapshot metadata and direct bytes', () => {
            createNativeEditorDocumentHandle({
                initialization: {
                    type: 'room',
                    documentId: 'doc-1',
                    lineageId: 'lineage-1',
                    snapshot: {
                        metadata: MOCK_SNAPSHOT_METADATA,
                        encodedState: MOCK_SNAPSHOT_BYTES,
                    },
                },
            });
            const [configJson, snapshotState] = mockNativeModule.editorV2Create.mock.calls[0];
            expect(JSON.parse(configJson)).toEqual({
                initialization: {
                    type: 'room',
                    documentId: 'doc-1',
                    lineageId: 'lineage-1',
                    snapshot: MOCK_SNAPSHOT_METADATA,
                },
            });
            expect(snapshotState).toBe(MOCK_SNAPSHOT_BYTES);
        });

        it('throws the typed boundary error when creation is rejected', () => {
            mockNativeModule.editorV2Create.mockReturnValueOnce(
                errRecord({
                    domain: 'boundary',
                    code: 'CONFIG_INVALID',
                    message: 'snapshot state bytes require a room initialization',
                })
            );
            const error = catchThrown(() => createHandle());
            expect(error).toBeInstanceOf(NativeEditorEngineBoundaryError);
            expect((error as NativeEditorErrorBase).code).toBe('CONFIG_INVALID');
        });

        it('rejects a malformed create editorId', () => {
            mockNativeModule.editorV2Create.mockReturnValueOnce(
                okRecord(JSON.stringify({ editorId: '01' }))
            );
            expectNonRetryable(
                catchRejectedNativeRecord(() => createHandle()),
                'FFI_RESULT_INVALID'
            );
        });

        it('destroys exactly once and keeps repeated destroy safe', () => {
            const handle = createHandle();
            handle.destroy();
            expect(handle.isDestroyed).toBe(true);
            expect(mockNativeModule.editorV2Destroy).toHaveBeenCalledTimes(1);
            expect(mockNativeModule.editorV2Destroy).toHaveBeenCalledWith('1');
            handle.destroy();
            expect(mockNativeModule.editorV2Destroy).toHaveBeenCalledTimes(1);
        });

        it('does not throw when the native session is already gone at destroy', () => {
            const handle = createHandle();
            mockNativeModule.editorV2Destroy.mockReturnValueOnce(
                errRecord({
                    domain: 'lifecycle',
                    code: 'ENGINE_DESTROYED',
                    message: 'editor session is not registered',
                })
            );
            expect(() => handle.destroy()).not.toThrow();
            expect(handle.isDestroyed).toBe(true);
        });

        it('retains a live handle and error listeners when destroy fails before a successful retry', () => {
            const handle = createHandle();
            const received: NativeEditorErrorBase[] = [];
            handle.addErrorListener((error) => received.push(error));
            mockNativeModule.editorV2Destroy
                .mockReturnValueOnce(errRecord(mockV2Error()))
                .mockReturnValueOnce(okRecord(true));

            const failure = catchThrown(() => handle.destroy());
            expect(failure).toBeInstanceOf(NativeEditorOperationError);
            expect((failure as NativeEditorErrorBase).code).toBe('OPERATION_INVALID');
            expect(handle.isDestroyed).toBe(false);

            handle.bridge._emitAutonomousError(mockV2Error());
            expect(received).toHaveLength(1);

            expect(() => handle.destroy()).not.toThrow();
            expect(handle.isDestroyed).toBe(true);
            expect(mockNativeModule.editorV2Destroy).toHaveBeenCalledTimes(2);

            handle.bridge._emitAutonomousError(mockV2Error());
            expect(received).toHaveLength(1);
        });

        it('classifies calls after destroy as non-retryable', () => {
            const handle = createHandle();
            handle.destroy();
            for (const call of [
                () => handle.bridge.getState(),
                () => handle.bridge.getDocumentJson(),
                () => handle.bridge.undo(),
                () => handle.bridge.setLocalAwareness(null),
            ]) {
                const error = catchThrown(call);
                expectNonRetryable(error, 'ENGINE_DESTROYED');
                expect((error as NativeEditorErrorBase).domain).toBe('lifecycle');
            }
        });

        it('classifies a native lifecycle error for a live handle as non-retryable', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockReturnValueOnce(
                errRecord({
                    domain: 'lifecycle',
                    code: 'ENGINE_DESTROYED',
                    message: 'editor session is not registered',
                })
            );
            expectNonRetryable(
                catchThrown(() => handle.bridge.getState()),
                'ENGINE_DESTROYED'
            );
        });

        it('classifies a result racing a re-entrant destroy as non-retryable', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockImplementationOnce(() => {
                handle.destroy();
                return okRecord(JSON.stringify(MOCK_V2_STATE));
            });
            expectNonRetryable(
                catchThrown(() => handle.bridge.getState()),
                'ENGINE_DESTROYED'
            );
        });
    });
});
