// ─── NativeEditorBridge v2 Tests ───────────────────────────────
// Unit tests for the production v2 surface: the frozen exactly-one result
// envelope, decimal-string revisions/identifiers, direct binary values,
// typed per-domain imperative throws, the distinct non-retryable class,
// the document handle lifecycle (including destroy races), and autonomous
// error events. The native module is mocked; the mock returns raw
// UniFFI-record-shaped results exactly as the native adapters emit
// them ({ value, error } records; direct Uint8Array binaries).

// ─── Mock Data ──────────────────────────────────────────────────

const MOCK_DOCUMENT_JSON = JSON.stringify({
    type: 'doc',
    content: [
        {
            type: 'paragraph',
            content: [{ type: 'text', text: 'hello world' }],
        },
    ],
});

const MOCK_V2_STATE = {
    documentState: 'LocalReady',
    transportState: 'Detached',
    renderState: 'Ready',
    documentRevision: '4',
    stateRevision: '2',
    canUndo: true,
    canRedo: false,
};

const MOCK_V2_TRANSACTION = {
    type: 'transaction',
    changed: true,
    documentRevision: '5',
    stateRevision: '3',
    canUndo: true,
    canRedo: false,
};

const HUGE_U64_DECIMAL = '18446744073709551615';
const ONE_OVER_U64_DECIMAL = '18446744073709551616';

const MOCK_ATOMIC_RENDER_SNAPSHOT = {
    renderBlocks: [
        [
            { type: 'blockStart', nodeType: 'paragraph', depth: 0 },
            { type: 'textRun', text: 'hello world', marks: [] },
            { type: 'blockEnd' },
        ],
    ],
    renderPatch: null,
    selection: { type: 'text', anchor: 1, head: 1, anchorScalar: 0, headScalar: 0 },
    activeState: {
        marks: {},
        markAttrs: {},
        nodes: { paragraph: true },
        commands: { insertText: true },
        allowedMarks: ['bold'],
        insertableNodes: ['paragraph'],
    },
    historyState: { canUndo: true, canRedo: false },
    documentVersion: HUGE_U64_DECIMAL,
    stateRevision: '3',
    scalarLength: 11,
};

const MOCK_SNAPSHOT_METADATA = {
    formatVersion: 1,
    documentId: 'doc-1',
    lineageId: 'lineage-1',
    fragmentName: 'prosemirror',
    schemaFingerprint: '0123456789abcdef',
};

const MOCK_SNAPSHOT_BYTES = new Uint8Array([0, 1, 2, 127, 128, 255, 7]);
const MOCK_PROTOCOL_FRAME = new Uint8Array([0, 3, 9, 200, 17]);

function mockV2Error(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        domain: 'operation',
        code: 'OPERATION_INVALID',
        message: 'operation invalid',
        ...overrides,
    };
}

function okRecord(value: unknown): Record<string, unknown> {
    return { value, error: null };
}

function errRecord(error: unknown): Record<string, unknown> {
    return { value: null, error };
}

// ─── Mock Native Module ─────────────────────────────────────────

let mockEditorIdCounter = 0;

const mockNativeModule: Record<string, jest.Mock> = {};

function resetMockNativeModule() {
    mockEditorIdCounter = 0;
    for (const key of Object.keys(mockNativeModule)) {
        delete mockNativeModule[key];
    }
    mockNativeModule.editorV2Create = jest.fn((_configJson: string, _snapshot: unknown) =>
        okRecord(JSON.stringify({ editorId: String(++mockEditorIdCounter) }))
    );
    mockNativeModule.editorV2Destroy = jest.fn(() => okRecord(true));
    mockNativeModule.editorV2GetState = jest.fn(() => okRecord(JSON.stringify(MOCK_V2_STATE)));
    mockNativeModule.editorV2GetDocumentJson = jest.fn(() => okRecord(MOCK_DOCUMENT_JSON));
    mockNativeModule.editorV2GetDocumentHtml = jest.fn(() =>
        okRecord(JSON.stringify({ html: '<p>hello world</p>' }))
    );
    mockNativeModule.editorV2GetContentSnapshot = jest.fn(() =>
        okRecord(
            JSON.stringify({
                html: '<p>hello world</p>',
                json: JSON.parse(MOCK_DOCUMENT_JSON),
            })
        )
    );
    mockNativeModule.editorV2RenderUpdate = jest.fn(() =>
        okRecord(JSON.stringify(MOCK_ATOMIC_RENDER_SNAPSHOT))
    );
    mockNativeModule.editorV2ReplaceDocument = jest.fn(() =>
        okRecord(JSON.stringify({ changed: true, documentRevision: '6' }))
    );
    mockNativeModule.editorV2ApplyInput = jest.fn(() =>
        okRecord(JSON.stringify(MOCK_V2_TRANSACTION))
    );
    mockNativeModule.editorV2ApplyCommand = jest.fn(() =>
        okRecord(JSON.stringify(MOCK_V2_TRANSACTION))
    );
    mockNativeModule.editorV2ApplyLocalApi = jest.fn(() =>
        okRecord(JSON.stringify({ type: 'replacement', changed: true, documentRevision: '9' }))
    );
    mockNativeModule.editorV2SetSelection = jest.fn(() =>
        okRecord(JSON.stringify({ type: 'notApplicable' }))
    );
    mockNativeModule.editorV2Undo = jest.fn(() => okRecord(JSON.stringify({ changed: true })));
    mockNativeModule.editorV2Redo = jest.fn(() => okRecord(JSON.stringify({ changed: false })));
    mockNativeModule.editorV2SnapshotExport = jest.fn(() =>
        okRecord({
            metadataJson: JSON.stringify(MOCK_SNAPSHOT_METADATA),
            encodedState: MOCK_SNAPSHOT_BYTES,
        })
    );
    mockNativeModule.editorV2SnapshotRestore = jest.fn(() =>
        okRecord(JSON.stringify({ changed: true, documentRevision: HUGE_U64_DECIMAL }))
    );
    mockNativeModule.editorV2CollaborationBeginConnect = jest.fn(() =>
        okRecord(JSON.stringify({ generation: '7' }))
    );
    mockNativeModule.editorV2CollaborationSocketOpen = jest.fn(() => okRecord(MOCK_PROTOCOL_FRAME));
    mockNativeModule.editorV2CollaborationReceive = jest.fn(() =>
        okRecord(
            JSON.stringify({
                framesDecoded: 1,
                repliesEnqueued: 2,
                replyBytesEnqueued: 64,
                remoteCommitApplied: true,
                documentPromoted: false,
                transportState: 'Handshaking',
                close: null,
            })
        )
    );
    mockNativeModule.editorV2CollaborationSocketClose = jest.fn(() =>
        okRecord(JSON.stringify({ transportState: 'Disconnected' }))
    );
    mockNativeModule.editorV2CollaborationTakeOutbound = jest.fn(() =>
        okRecord(MOCK_PROTOCOL_FRAME)
    );
    mockNativeModule.editorV2CollaborationSetAwareness = jest.fn(() => okRecord(true));
    mockNativeModule.editorV2CollaborationPeers = jest.fn(() =>
        okRecord(
            JSON.stringify({
                peers: [
                    {
                        clientId: HUGE_U64_DECIMAL,
                        clock: 3,
                        isLocal: false,
                        state: { user: { name: 'Alice' } },
                        cursor: { anchor: 2, head: 5 },
                    },
                    { clientId: '1', clock: 0, isLocal: true, state: null, cursor: null },
                ],
            })
        )
    );
}

resetMockNativeModule();

jest.mock('expo-modules-core', () => ({
    requireNativeModule: () => mockNativeModule,
}));

// ─── Imports ────────────────────────────────────────────────────

import {
    createNativeEditorDocumentHandle,
    type NativeEditorDocumentHandle,
    type NativeEditorV2CreateConfig,
    normalizeNativeEditorV2Bytes,
    normalizeNativeEditorV2DecimalId,
    normalizeNativeEditorV2Result,
    normalizeNativeEditorV2Unit,
    requireNativeEditorV2U32,
    unwrapNativeEditorV2Result,
    _resetNativeModuleCache,
} from '../NativeEditorBridge';
import * as NativeEditorBridgeExports from '../NativeEditorBridge';
import {
    NativeEditorBoundaryError,
    NativeEditorV2BoundaryError,
    NativeEditorV2DocumentError,
    NativeEditorV2ErrorBase,
    NativeEditorV2LifecycleError,
    NativeEditorV2NonRetryableError,
    NativeEditorV2OperationError,
    NativeEditorV2SnapshotError,
    NativeEditorV2TransportError,
    normalizeNativeEditorV2Error,
    type NativeEditorV2Error,
} from '../NativeEditorBoundaryError';
import { HARD_EDITOR_RESOURCE_LIMITS } from '../ResourceLimits';
import { join } from 'path';
import ts from 'typescript';

// ─── Helpers ────────────────────────────────────────────────────

function createHandle(): NativeEditorDocumentHandle {
    return createNativeEditorDocumentHandle({
        initialization: { type: 'localEmpty' },
    });
}

function parsedTypeScriptConfig(): ts.ParsedCommandLine {
    const configPath = join(process.cwd(), 'tsconfig.json');
    const config = ts.readConfigFile(configPath, ts.sys.readFile);
    if (config.error) {
        throw new Error(ts.formatDiagnostic(config.error, formatDiagnosticHost));
    }
    return ts.parseJsonConfigFileContent(config.config, ts.sys, process.cwd());
}

const formatDiagnosticHost: ts.FormatDiagnosticsHost = {
    getCanonicalFileName: (fileName) => fileName,
    getCurrentDirectory: () => process.cwd(),
    getNewLine: () => '\n',
};

function compileCreateContractFixture(sourceText: string): string {
    const parsed = parsedTypeScriptConfig();
    const fixturePath = join(
        process.cwd(),
        'src',
        '__tests__',
        '__task_4_create_contract_fixture.ts'
    );
    const options: ts.CompilerOptions = {
        ...parsed.options,
        noEmit: true,
        types: [],
    };
    const host = ts.createCompilerHost(options);
    const fileExists = host.fileExists.bind(host);
    const readFile = host.readFile.bind(host);
    const getSourceFile = host.getSourceFile.bind(host);
    host.fileExists = (fileName) => fileName === fixturePath || fileExists(fileName);
    host.readFile = (fileName) => (fileName === fixturePath ? sourceText : readFile(fileName));
    host.getSourceFile = (fileName, languageVersion, onError, shouldCreateNewSourceFile) =>
        fileName === fixturePath
            ? ts.createSourceFile(fileName, sourceText, languageVersion, true, ts.ScriptKind.TS)
            : getSourceFile(fileName, languageVersion, onError, shouldCreateNewSourceFile);
    const program = ts.createProgram([fixturePath], options, host);
    return ts.formatDiagnosticsWithColorAndContext(
        ts.getPreEmitDiagnostics(program),
        formatDiagnosticHost
    );
}

function emitNativeEditorBridgeDeclaration(): { declaration: string; diagnostics: string } {
    const parsed = parsedTypeScriptConfig();
    const sourcePath = join(process.cwd(), 'src', 'NativeEditorBridge.ts');
    const options: ts.CompilerOptions = {
        ...parsed.options,
        declaration: true,
        declarationMap: false,
        emitDeclarationOnly: true,
        noEmit: false,
    };
    const host = ts.createCompilerHost(options);
    let declaration = '';
    const program = ts.createProgram([sourcePath], options, host);
    const emit = program.emit(undefined, (fileName, output) => {
        if (fileName.endsWith('/NativeEditorBridge.d.ts')) declaration = output;
    });
    const diagnostics = ts.formatDiagnosticsWithColorAndContext(
        [...ts.getPreEmitDiagnostics(program), ...emit.diagnostics],
        formatDiagnosticHost
    );
    return { declaration, diagnostics };
}

function expectNonRetryable(error: unknown, code: string): void {
    expect(error).toBeInstanceOf(NativeEditorV2NonRetryableError);
    expect(error).toBeInstanceOf(NativeEditorV2ErrorBase);
    expect((error as NativeEditorV2ErrorBase).code).toBe(code);
}

function catchThrown(fn: () => unknown): unknown {
    try {
        fn();
    } catch (error) {
        return error;
    }
    throw new Error('expected the call to throw');
}

// ─── Tests ──────────────────────────────────────────────────────

describe('NativeEditorBridge v2', () => {
    beforeEach(() => {
        _resetNativeModuleCache();
        resetMockNativeModule();
    });

    describe('exactly-one result record validation', () => {
        const identity = (value: unknown): unknown => value;

        it('accepts a value-only record (error null or omitted)', () => {
            expect(normalizeNativeEditorV2Result(okRecord('v'), identity)).toEqual({
                ok: true,
                value: 'v',
            });
            expect(normalizeNativeEditorV2Result({ value: 'v' }, identity)).toEqual({
                ok: true,
                value: 'v',
            });
        });

        it('accepts an error-only record (value null or omitted)', () => {
            const error = mockV2Error();
            expect(normalizeNativeEditorV2Result(errRecord(error), identity)).toEqual({
                ok: false,
                error: {
                    domain: 'operation',
                    code: 'OPERATION_INVALID',
                    message: 'operation invalid',
                    requestId: null,
                    operationIndex: null,
                    limit: null,
                    actual: null,
                    details: null,
                },
            });
            expect(normalizeNativeEditorV2Result({ error }, identity)).not.toBeNull();
        });

        it('rejects a record carrying both value and error', () => {
            expect(
                normalizeNativeEditorV2Result({ value: 'v', error: mockV2Error() }, identity)
            ).toBeNull();
        });

        it('rejects a record carrying neither value nor error', () => {
            expect(normalizeNativeEditorV2Result({}, identity)).toBeNull();
            expect(normalizeNativeEditorV2Result({ value: null, error: null }, identity)).toBeNull();
        });

        it('rejects non-object records', () => {
            for (const raw of [null, undefined, 42, 'oops', [], true]) {
                expect(normalizeNativeEditorV2Result(raw, identity)).toBeNull();
            }
        });

        it('rejects an error field of the wrong type', () => {
            for (const error of ['oops', 42, [], null]) {
                expect(normalizeNativeEditorV2Result(errRecord(error), identity)).toBeNull();
            }
        });

        it('rejects a value the value normalizer rejects', () => {
            expect(normalizeNativeEditorV2Result(okRecord('nope'), () => null)).toBeNull();
        });

        it('throws the non-retryable class for malformed records on the imperative path', () => {
            const error = catchThrown(() =>
                unwrapNativeEditorV2Result({ value: 'v', error: mockV2Error() }, (v) => v)
            );
            expectNonRetryable(error, 'FFI_RESULT_INVALID');
            expect((error as NativeEditorV2ErrorBase).domain).toBe('boundary');
        });
    });

    describe('error field, domain, and code validation', () => {
        const identity = (value: unknown): unknown => value;

        it.each([
            ['boundary', NativeEditorV2BoundaryError],
            ['document', NativeEditorV2DocumentError],
            ['operation', NativeEditorV2OperationError],
            ['lifecycle', NativeEditorV2LifecycleError],
            ['snapshot', NativeEditorV2SnapshotError],
            ['transport', NativeEditorV2TransportError],
        ])(
            'throws the structured %s error class for recoverable errors',
            (domain, expectedClass) => {
                const error = catchThrown(() =>
                    unwrapNativeEditorV2Result(
                        errRecord(
                            mockV2Error({
                                domain,
                                requestId: '7',
                                operationIndex: '2',
                                limit: '10',
                                actual: '11',
                                details: { field: 'text' },
                            })
                        ),
                        identity
                    )
                );
                expect(error).toBeInstanceOf(expectedClass);
                expect(error).toBeInstanceOf(NativeEditorV2ErrorBase);
                expect(error).not.toBeInstanceOf(NativeEditorV2NonRetryableError);
                const typed = error as NativeEditorV2ErrorBase;
                expect(typed.domain).toBe(domain);
                expect(typed.code).toBe('OPERATION_INVALID');
                expect(typed.message).toBe('operation invalid');
                expect(typed.requestId).toBe('7');
                expect(typed.operationIndex).toBe('2');
                expect(typed.limit).toBe('10');
                expect(typed.actual).toBe('11');
                expect(typed.details).toEqual({ field: 'text' });
            }
        );

        it('rejects an unknown domain', () => {
            expect(
                normalizeNativeEditorV2Result(errRecord(mockV2Error({ domain: 'quantum' })), identity)
            ).toBeNull();
        });

        it('rejects missing or mistyped required error fields', () => {
            expect(
                normalizeNativeEditorV2Result(
                    errRecord({ code: 'OPERATION_INVALID', message: 'm', domain: 'operation' }),
                    identity
                )
            ).not.toBeNull();
            expect(
                normalizeNativeEditorV2Result(errRecord({ domain: 'operation', message: 'm' }), identity)
            ).toBeNull();
            expect(
                normalizeNativeEditorV2Result(errRecord({ domain: 'operation', code: 'X' }), identity)
            ).toBeNull();
            expect(
                normalizeNativeEditorV2Result(
                    errRecord(mockV2Error({ code: 42 })),
                    identity
                )
            ).toBeNull();
            expect(
                normalizeNativeEditorV2Result(
                    errRecord(mockV2Error({ message: null })),
                    identity
                )
            ).toBeNull();
        });

        it.each(['0', '1', '42', HUGE_U64_DECIMAL])(
            'accepts canonical decimal-string requestId %s of any size',
            (requestId) => {
                const result = normalizeNativeEditorV2Result(
                    errRecord(mockV2Error({ requestId })),
                    identity
                );
                expect(result).not.toBeNull();
                expect(result?.ok).toBe(false);
                if (result && !result.ok) {
                    expect(result.error.requestId).toBe(requestId);
                }
            }
        );

        it.each(['', '01', '-1', '1.0', '+1', ' 1', '1e3', '1 ', '0x10'])(
            'rejects non-canonical requestId %p',
            (requestId) => {
                expect(
                    normalizeNativeEditorV2Result(errRecord(mockV2Error({ requestId })), identity)
                ).toBeNull();
            }
        );

        it('rejects a numeric requestId even when integral', () => {
            expect(
                normalizeNativeEditorV2Result(errRecord(mockV2Error({ requestId: 7 })), identity)
            ).toBeNull();
        });

        it.each(['0', '7', '1024', HUGE_U64_DECIMAL])(
            'accepts canonical decimal string %s for u64 error fields',
            (fieldValue) => {
            const result = normalizeNativeEditorV2Result(
                errRecord(
                    mockV2Error({ operationIndex: fieldValue, limit: fieldValue, actual: fieldValue })
                ),
                identity
            );
            expect(result).not.toBeNull();
            }
        );

        it.each([-1, 1.5, Number.MAX_SAFE_INTEGER + 1, '01', '+1', '7', NaN])(
            'rejects invalid limit field value %p',
            (fieldValue) => {
                expect(
                    normalizeNativeEditorV2Result(
                        errRecord(mockV2Error({ limit: fieldValue })),
                        identity
                    )
                ).toBeNull();
                expect(
                    normalizeNativeEditorV2Result(
                        errRecord(mockV2Error({ operationIndex: fieldValue })),
                        identity
                    )
                ).toBeNull();
                expect(
                    normalizeNativeEditorV2Result(
                        errRecord(mockV2Error({ actual: fieldValue })),
                        identity
                    )
                ).toBeNull();
            }
        );

        it('accepts an object details payload and parses detailsJson', () => {
            const withDetails = normalizeNativeEditorV2Result(
                errRecord(mockV2Error({ details: { field: 'content' } })),
                identity
            );
            expect(withDetails && !withDetails.ok && withDetails.error.details).toEqual({
                field: 'content',
            });
            const withDetailsJson = normalizeNativeEditorV2Result(
                errRecord(mockV2Error({ detailsJson: '{"field":"content"}' })),
                identity
            );
            expect(withDetailsJson && !withDetailsJson.ok && withDetailsJson.error.details).toEqual(
                { field: 'content' }
            );
        });

        it('accepts canonical nested u64 error details from detailsJson', () => {
            const revisionMismatch = normalizeNativeEditorV2Error(
                errRecord(
                    mockV2Error({
                        code: 'REVISION_MISMATCH',
                        detailsJson:
                            '{"expectedRevision":"9007199254740993","actualRevision":"18446744073709551615"}',
                    })
                )
            );
            expect(revisionMismatch?.details).toEqual({
                expectedRevision: '9007199254740993',
                actualRevision: HUGE_U64_DECIMAL,
            });

            const staleGeneration = normalizeNativeEditorV2Error(
                errRecord(
                    mockV2Error({
                        domain: 'transport',
                        code: 'TRANSPORT_STALE_GENERATION',
                        detailsJson:
                            '{"presentedGeneration":"9007199254740993","liveGeneration":null}',
                    })
                )
            );
            expect(staleGeneration?.details).toEqual({
                presentedGeneration: '9007199254740993',
                liveGeneration: null,
            });
        });

        it.each([
            [
                'revision mismatch numeric detailsJson value',
                mockV2Error({
                    code: 'REVISION_MISMATCH',
                    detailsJson:
                        '{"expectedRevision":9007199254740993,"actualRevision":"18446744073709551615"}',
                }),
            ],
            [
                'revision mismatch non-canonical decimal',
                mockV2Error({
                    code: 'REVISION_MISMATCH',
                    detailsJson:
                        '{"expectedRevision":"01","actualRevision":"18446744073709551615"}',
                }),
            ],
            [
                'revision mismatch missing actual revision',
                mockV2Error({
                    code: 'REVISION_MISMATCH',
                    detailsJson: '{"expectedRevision":"9007199254740993"}',
                }),
            ],
            [
                'revision mismatch value above u64 max',
                mockV2Error({
                    code: 'REVISION_MISMATCH',
                    detailsJson:
                        '{"expectedRevision":"18446744073709551615","actualRevision":"18446744073709551616"}',
                }),
            ],
            [
                'stale generation numeric detailsJson value',
                mockV2Error({
                    domain: 'transport',
                    code: 'TRANSPORT_STALE_GENERATION',
                    detailsJson:
                        '{"presentedGeneration":"9007199254740993","liveGeneration":18446744073709551615}',
                }),
            ],
            [
                'stale generation malformed presented value',
                mockV2Error({
                    domain: 'transport',
                    code: 'TRANSPORT_STALE_GENERATION',
                    detailsJson: '{"presentedGeneration":"1e3","liveGeneration":null}',
                }),
            ],
            [
                'stale generation value above u64 max',
                mockV2Error({
                    domain: 'transport',
                    code: 'TRANSPORT_STALE_GENERATION',
                    detailsJson:
                        '{"presentedGeneration":"18446744073709551616","liveGeneration":"18446744073709551615"}',
                }),
            ],
        ])('rejects malformed known nested u64 details: %s', (_label, error) => {
            expect(normalizeNativeEditorV2Error(errRecord(error))).toBeNull();
        });

        it('rejects non-object details payloads', () => {
            for (const details of [[1, 2], 'oops', 42]) {
                expect(
                    normalizeNativeEditorV2Result(errRecord(mockV2Error({ details })), identity)
                ).toBeNull();
            }
            expect(
                normalizeNativeEditorV2Result(
                    errRecord(mockV2Error({ detailsJson: '{invalid' })),
                    identity
                )
            ).toBeNull();
        });

        it('classifies ENGINE_INVARIANT_FAILED as non-retryable', () => {
            const error = catchThrown(() =>
                unwrapNativeEditorV2Result(
                    errRecord(mockV2Error({ code: 'ENGINE_INVARIANT_FAILED' })),
                    identity
                )
            );
            expectNonRetryable(error, 'ENGINE_INVARIANT_FAILED');
            expect(error).not.toBeInstanceOf(NativeEditorV2OperationError);
        });

        it.each(['ENGINE_DESTROYED', 'ENGINE_DESTROYING'])(
            'classifies lifecycle %s as non-retryable',
            (code) => {
                const error = catchThrown(() =>
                    unwrapNativeEditorV2Result(
                        errRecord(mockV2Error({ domain: 'lifecycle', code })),
                        identity
                    )
                );
                expectNonRetryable(error, code);
                expect((error as NativeEditorV2ErrorBase).domain).toBe('lifecycle');
                expect(error).not.toBeInstanceOf(NativeEditorV2LifecycleError);
            }
        );

        it('keeps WHOLE_DOCUMENT_REPLACEMENT_CONNECTED a recoverable lifecycle error', () => {
            const error = catchThrown(() =>
                unwrapNativeEditorV2Result(
                    errRecord(
                        mockV2Error({
                            domain: 'lifecycle',
                            code: 'WHOLE_DOCUMENT_REPLACEMENT_CONNECTED',
                        })
                    ),
                    identity
                )
            );
            expect(error).toBeInstanceOf(NativeEditorV2LifecycleError);
            expect(error).not.toBeInstanceOf(NativeEditorV2NonRetryableError);
        });
    });

    describe('decimal identifiers and unsafe integers', () => {
        it('normalizes canonical decimal strings of any size verbatim', () => {
            expect(normalizeNativeEditorV2DecimalId('0')).toBe('0');
            expect(normalizeNativeEditorV2DecimalId('42')).toBe('42');
            expect(normalizeNativeEditorV2DecimalId(HUGE_U64_DECIMAL)).toBe(HUGE_U64_DECIMAL);
        });

        it('accepts u64::MAX and rejects larger decimal strings without Number()', () => {
            expect(normalizeNativeEditorV2DecimalId(HUGE_U64_DECIMAL)).toBe(HUGE_U64_DECIMAL);
            expect(normalizeNativeEditorV2DecimalId(ONE_OVER_U64_DECIMAL)).toBeNull();
            expect(normalizeNativeEditorV2DecimalId('9'.repeat(256))).toBeNull();
        });

        it.each([0, 42, Number.MAX_SAFE_INTEGER, Number.MAX_SAFE_INTEGER + 1])(
            'rejects numeric compatibility value %p even when safely representable',
            (value) => {
                expect(normalizeNativeEditorV2DecimalId(value)).toBeNull();
            }
        );

        it.each([
            '',
            '01',
            '-1',
            '1.0',
            '+1',
            ' 1',
            '1 ',
            '1e3',
            '1E3',
        ])('rejects every non-canonical decimal string %p', (value) => {
            expect(normalizeNativeEditorV2DecimalId(value)).toBeNull();
        });

        it.each([Number.MAX_SAFE_INTEGER + 1, -1, 1.5, NaN, Infinity])(
            'rejects unsafe or non-integer number %p',
            (value) => {
                expect(normalizeNativeEditorV2DecimalId(value)).toBeNull();
            }
        );

        it('keeps huge decimal-string revisions verbatim in state results', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        ...MOCK_V2_STATE,
                        documentRevision: HUGE_U64_DECIMAL,
                        stateRevision: HUGE_U64_DECIMAL,
                    })
                )
            );
            const state = handle.bridge.getState();
            expect(state.documentRevision).toBe(HUGE_U64_DECIMAL);
            expect(state.stateRevision).toBe(HUGE_U64_DECIMAL);
            expect(typeof state.documentRevision).toBe('string');
        });

        it('rejects numeric revision compatibility values even below the JavaScript safe limit', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockReturnValueOnce(
                okRecord(JSON.stringify({ ...MOCK_V2_STATE, documentRevision: 4 }))
            );
            expectNonRetryable(catchThrown(() => handle.bridge.getState()), 'FFI_RESULT_INVALID');
        });

        it.each([
            [0, 0],
            [1, 1],
            [0xffff_ffff, 0xffff_ffff],
        ])('accepts exact u32 value %p for %s', (value, expected) => {
            expect(requireNativeEditorV2U32(value, 'scalar')).toBe(expected);
        });

        it.each([-1, 1.5, NaN, Infinity, 0x1_0000_0000])(
            'rejects non-exact or out-of-range u32 value %p',
            (value) => {
                expect(() => requireNativeEditorV2U32(value, 'scalar')).toThrow(
                    'invalid u32 scalar'
                );
            }
        );

        it('rejects an unsafe integer revision in a state result', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        ...MOCK_V2_STATE,
                        documentRevision: Number.MAX_SAFE_INTEGER + 1,
                    })
                )
            );
            expectNonRetryable(catchThrown(() => handle.bridge.getState()), 'FFI_RESULT_INVALID');
        });

        it('rejects a leading-zero revision string in a state result', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockReturnValueOnce(
                okRecord(JSON.stringify({ ...MOCK_V2_STATE, documentRevision: '04' }))
            );
            expectNonRetryable(catchThrown(() => handle.bridge.getState()), 'FFI_RESULT_INVALID');
        });

        it('normalizes transaction outcome revisions to decimal strings', () => {
            const handle = createHandle();
            const outcome = handle.bridge.applyInput({ baseDocumentRevision: '4', text: 'hi' });
            expect(outcome).toEqual({
                type: 'transaction',
                changed: true,
                documentRevision: '5',
                stateRevision: '3',
                canUndo: true,
                canRedo: false,
            });
        });

        it('rejects an unsafe integer revision in a transaction outcome', () => {
            const handle = createHandle();
            mockNativeModule.editorV2ApplyInput.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        ...MOCK_V2_TRANSACTION,
                        stateRevision: Number.MAX_SAFE_INTEGER + 1,
                    })
                )
            );
            expectNonRetryable(
                catchThrown(() =>
                    handle.bridge.applyInput({ baseDocumentRevision: 4 as never, text: 'x' })
                ),
                'FFI_RESULT_INVALID'
            );
        });
    });

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

        const missingStateRevision = { ...MOCK_ATOMIC_RENDER_SNAPSHOT } as Record<
            string,
            unknown
        >;
        delete missingStateRevision.stateRevision;
        const missingSelection = { ...MOCK_ATOMIC_RENDER_SNAPSHOT } as Record<string, unknown>;
        delete missingSelection.selection;
        it.each<[string, Record<string, unknown>]>([
            ['missing stateRevision', missingStateRevision],
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
                'unexpected nested render element field',
                {
                    ...MOCK_ATOMIC_RENDER_SNAPSHOT,
                    renderBlocks: [[{ type: 'blockStart', nodeType: 'paragraph', depth: 0, extra: true }]],
                },
            ],
            ['unknown top-level field', { ...MOCK_ATOMIC_RENDER_SNAPSHOT, unexpected: true }],
        ])('rejects %s', (_name, malformed) => {
            const handle = createHandle();
            mockNativeModule.editorV2RenderUpdate.mockReturnValueOnce(
                okRecord(JSON.stringify(malformed))
            );
            expectNonRetryable(catchThrown(() => handle.bridge.renderUpdate()), 'FFI_RESULT_INVALID');
        });
    });

    describe('document handle lifecycle', () => {
        it('type-checks only the exact grouped create shape and sole factory constructor', () => {
            const diagnostics = compileCreateContractFixture(`
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

        it('serializes normalized own-property copies from null-prototype records', () => {
            const initialization = Object.assign(Object.create(null), { type: 'localEmpty' });
            const policy = Object.assign(Object.create(null), { readOnly: true });
            const resource = Object.assign(Object.create(null), { maxInputBytes: 1024 });
            const limits = Object.assign(Object.create(null), { resource });
            const config = Object.assign(Object.create(null), {
                initialization,
                policy,
                limits,
            }) as NativeEditorV2CreateConfig;

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
                    createNativeEditorDocumentHandle(config as NativeEditorV2CreateConfig)
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
                    } as unknown as NativeEditorV2CreateConfig;
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
            expect((limitError as NativeEditorV2ErrorBase).code).toBe('INVALID_RESOURCE_LIMIT');

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
                    replayingConfig as unknown as NativeEditorV2CreateConfig
                )
            );

            expect(replayed).not.toBe(limitError);
            expect(replayed).toBeInstanceOf(NativeEditorV2BoundaryError);
            expect((replayed as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
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
            expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
            expect((error as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
        });

        it('rejects a malformed create editorId', () => {
            mockNativeModule.editorV2Create.mockReturnValueOnce(
                okRecord(JSON.stringify({ editorId: '01' }))
            );
            expectNonRetryable(catchThrown(() => createHandle()), 'FFI_RESULT_INVALID');
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

        it('classifies calls after destroy as non-retryable', () => {
            const handle = createHandle();
            handle.destroy();
            for (const call of [
                () => handle.bridge.getState(),
                () => handle.bridge.getDocumentJson(),
                () => handle.bridge.undo(),
                () => handle.bridge.collaborationTakeOutbound('1'),
            ]) {
                const error = catchThrown(call);
                expectNonRetryable(error, 'ENGINE_DESTROYED');
                expect((error as NativeEditorV2ErrorBase).domain).toBe('lifecycle');
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
            expectNonRetryable(catchThrown(() => handle.bridge.getState()), 'ENGINE_DESTROYED');
        });

        it('classifies a result racing a re-entrant destroy as non-retryable', () => {
            const handle = createHandle();
            mockNativeModule.editorV2GetState.mockImplementationOnce(() => {
                handle.destroy();
                return okRecord(JSON.stringify(MOCK_V2_STATE));
            });
            expectNonRetryable(catchThrown(() => handle.bridge.getState()), 'ENGINE_DESTROYED');
        });
    });

    describe('request envelopes', () => {
        it('builds the exact input envelope with auto-assigned request ids', () => {
            const handle = createHandle();
            handle.bridge.applyInput({ baseDocumentRevision: '4', text: 'hi' });
            handle.bridge.applyInput({ baseDocumentRevision: '5', text: 'there' });
            const calls = mockNativeModule.editorV2ApplyInput.mock.calls;
            expect(calls[0][0]).toBe('1');
            expect(JSON.parse(calls[0][1])).toEqual({
                version: 1,
                requestId: '1',
                baseDocumentRevision: '4',
                text: 'hi',
            });
            expect(JSON.parse(calls[1][1])).toEqual({
                version: 1,
                requestId: '2',
                baseDocumentRevision: '5',
                text: 'there',
            });
        });

        it('accepts u64::MAX once then rejects request-ID exhaustion locally', () => {
            const handle = createHandle();
            const bridgeForTest = handle.bridge as unknown as { _nextRequestId: bigint };
            bridgeForTest._nextRequestId = BigInt(HUGE_U64_DECIMAL) - 1n;

            handle.bridge.undo();
            expect(JSON.parse(mockNativeModule.editorV2Undo.mock.calls[0][1])).toEqual({
                version: 1,
                requestId: HUGE_U64_DECIMAL,
            });

            const error = catchThrown(() => handle.bridge.redo());
            expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
            expect((error as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
            expect((error as NativeEditorV2ErrorBase).domain).toBe('boundary');
            expect(mockNativeModule.editorV2Redo).not.toHaveBeenCalled();
        });

        it('embeds a huge baseDocumentRevision as canonical decimal text without Number()', () => {
            const handle = createHandle();
            handle.bridge.applyInput({ baseDocumentRevision: HUGE_U64_DECIMAL, text: 'x' });
            const requestJson = mockNativeModule.editorV2ApplyInput.mock.calls[0][1];
            expect(requestJson).toBe(
                `{"version":1,"requestId":"1","baseDocumentRevision":"${HUGE_U64_DECIMAL}","text":"x"}`
            );
        });

        it.each(['01', '1.5', '-1', '', 'nope'])(
            'rejects a malformed baseDocumentRevision %p before any native call',
            (baseDocumentRevision) => {
                const handle = createHandle();
                const error = catchThrown(() =>
                    handle.bridge.applyInput({ baseDocumentRevision, text: 'x' })
                );
                expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
                expect((error as NativeEditorV2ErrorBase).code).toBe('CONFIG_INVALID');
                expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
            }
        );

        it('rejects every numeric baseDocumentRevision before any native call', () => {
            const handle = createHandle();
            const error = catchThrown(() =>
                handle.bridge.applyInput({
                    baseDocumentRevision: Number.MAX_SAFE_INTEGER as never,
                    text: 'x',
                })
            );
            expect(error).toBeInstanceOf(NativeEditorV2BoundaryError);
            expect(mockNativeModule.editorV2ApplyInput).not.toHaveBeenCalled();
        });

        it('builds the exact undo/redo envelope and treats changed:false as success', () => {
            const handle = createHandle();
            expect(handle.bridge.undo()).toBe(true);
            expect(handle.bridge.redo()).toBe(false);
            expect(JSON.parse(mockNativeModule.editorV2Undo.mock.calls[0][1])).toEqual({
                version: 1,
                requestId: '1',
            });
            expect(JSON.parse(mockNativeModule.editorV2Redo.mock.calls[0][1])).toEqual({
                version: 1,
                requestId: '2',
            });
        });

        it('builds the exact replace envelope and normalizes the commit revision', () => {
            const handle = createHandle();
            const commit = handle.bridge.replaceDocument({
                setJson: { type: 'doc', content: [] },
                history: 'resetAndClear',
            });
            expect(JSON.parse(mockNativeModule.editorV2ReplaceDocument.mock.calls[0][1])).toEqual({
                version: 1,
                requestId: '1',
                setJson: { type: 'doc', content: [] },
                history: 'resetAndClear',
            });
            expect(commit).toEqual({ changed: true, documentRevision: '6' });
        });
    });

    describe('mutation outcomes', () => {
        it('normalizes the notApplicable outcome', () => {
            const handle = createHandle();
            expect(handle.bridge.setSelection({ baseDocumentRevision: '4', selection: {} })).toEqual(
                { type: 'notApplicable' }
            );
        });

        it('normalizes the replacement outcome', () => {
            const handle = createHandle();
            expect(
                handle.bridge.applyLocalApi({
                    baseDocumentRevision: '4',
                    setHtml: '<p>x</p>',
                    history: 'undoableBoundary',
                })
            ).toEqual({ type: 'replacement', changed: true, documentRevision: '9' });
        });

        it('rejects an unknown outcome type', () => {
            const handle = createHandle();
            mockNativeModule.editorV2ApplyCommand.mockReturnValueOnce(
                okRecord(JSON.stringify({ type: 'surprise' }))
            );
            expectNonRetryable(
                catchThrown(() =>
                    handle.bridge.applyCommand({ baseDocumentRevision: '4', command: {} })
                ),
                'FFI_RESULT_INVALID'
            );
        });
    });

    describe('binary transport', () => {
        it('returns socket-open protocol frame bytes untouched', () => {
            const handle = createHandle();
            const frame = handle.bridge.collaborationSocketOpen('7');
            expect(frame).toBe(MOCK_PROTOCOL_FRAME);
            expect(Array.from(frame)).toEqual([0, 3, 9, 200, 17]);
        });

        it('returns an empty outbound frame as empty bytes', () => {
            const handle = createHandle();
            mockNativeModule.editorV2CollaborationTakeOutbound.mockReturnValueOnce(
                okRecord(new Uint8Array(0))
            );
            const frame = handle.bridge.collaborationTakeOutbound('7');
            expect(frame).toBeInstanceOf(Uint8Array);
            expect(frame.length).toBe(0);
        });

        it('round-trips snapshot bytes byte-for-byte in both directions', () => {
            const handle = createHandle();
            const exported = handle.bridge.snapshotExport();
            expect(exported.metadataJson).toBe(JSON.stringify(MOCK_SNAPSHOT_METADATA));
            expect(exported.encodedState).toBe(MOCK_SNAPSHOT_BYTES);

            const commit = handle.bridge.snapshotRestore(
                MOCK_SNAPSHOT_METADATA,
                exported.encodedState
            );
            const [editorId, metadataJson, encodedState] =
                mockNativeModule.editorV2SnapshotRestore.mock.calls[0];
            expect(editorId).toBe('1');
            expect(JSON.parse(metadataJson)).toEqual(MOCK_SNAPSHOT_METADATA);
            expect(encodedState).toBe(MOCK_SNAPSHOT_BYTES);
            expect(commit.documentRevision).toBe(HUGE_U64_DECIMAL);
        });

        it('rejects JSON number arrays as binary values', () => {
            expect(normalizeNativeEditorV2Bytes([1, 2, 3])).toBeNull();
            const handle = createHandle();
            mockNativeModule.editorV2CollaborationSocketOpen.mockReturnValueOnce(
                okRecord([0, 3, 9])
            );
            expectNonRetryable(
                catchThrown(() => handle.bridge.collaborationSocketOpen('7')),
                'FFI_RESULT_INVALID'
            );
        });

        it('passes receive message bytes through unchanged', () => {
            const handle = createHandle();
            handle.bridge.collaborationReceive('7', MOCK_PROTOCOL_FRAME);
            const [editorId, generation, message] =
                mockNativeModule.editorV2CollaborationReceive.mock.calls[0];
            expect(editorId).toBe('1');
            expect(generation).toBe('7');
            expect(message).toBe(MOCK_PROTOCOL_FRAME);
        });
    });

    describe('collaboration results', () => {
        it('normalizes the connect generation as a decimal string', () => {
            const handle = createHandle();
            expect(handle.bridge.collaborationBeginConnect()).toBe('7');
            mockNativeModule.editorV2CollaborationBeginConnect.mockReturnValueOnce(
                okRecord(JSON.stringify({ generation: HUGE_U64_DECIMAL }))
            );
            expect(handle.bridge.collaborationBeginConnect()).toBe(HUGE_U64_DECIMAL);
        });

        it('normalizes the receive outcome including a structured close cause', () => {
            const handle = createHandle();
            expect(handle.bridge.collaborationReceive('7', MOCK_PROTOCOL_FRAME)).toEqual({
                framesDecoded: 1,
                repliesEnqueued: 2,
                replyBytesEnqueued: 64,
                remoteCommitApplied: true,
                documentPromoted: false,
                transportState: 'Handshaking',
                close: null,
            });
            mockNativeModule.editorV2CollaborationReceive.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        framesDecoded: 0,
                        repliesEnqueued: 0,
                        replyBytesEnqueued: 0,
                        remoteCommitApplied: false,
                        documentPromoted: false,
                        transportState: 'Incompatible',
                        close: {
                            disposition: 'incompatible',
                            error: {
                                domain: 'transport',
                                code: 'TRANSPORT_PROTOCOL_INVALID',
                                message: 'invalid protocol frame',
                                limit: '1024',
                                actual: '1025',
                            },
                        },
                    })
                )
            );
            const outcome = handle.bridge.collaborationReceive('7', MOCK_PROTOCOL_FRAME);
            expect(outcome.transportState).toBe('Incompatible');
            expect(outcome.close).toEqual({
                disposition: 'incompatible',
                error: {
                    domain: 'transport',
                    code: 'TRANSPORT_PROTOCOL_INVALID',
                    message: 'invalid protocol frame',
                    requestId: null,
                    operationIndex: null,
                    limit: '1024',
                    actual: '1025',
                    details: null,
                },
            });
        });

        it('rejects a malformed nested close error', () => {
            const handle = createHandle();
            mockNativeModule.editorV2CollaborationReceive.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        framesDecoded: 0,
                        repliesEnqueued: 0,
                        replyBytesEnqueued: 0,
                        remoteCommitApplied: false,
                        documentPromoted: false,
                        transportState: 'Incompatible',
                        close: { disposition: 'incompatible', error: { code: 42 } },
                    })
                )
            );
            expectNonRetryable(
                catchThrown(() => handle.bridge.collaborationReceive('7', MOCK_PROTOCOL_FRAME)),
                'FFI_RESULT_INVALID'
            );
        });

        it('keeps huge decimal-string peer client ids verbatim', () => {
            const handle = createHandle();
            const peers = handle.bridge.collaborationPeers();
            expect(peers).toEqual([
                {
                    clientId: HUGE_U64_DECIMAL,
                    clock: 3,
                    isLocal: false,
                    state: { user: { name: 'Alice' } },
                    cursor: { anchor: 2, head: 5 },
                },
                { clientId: '1', clock: 0, isLocal: true, state: null, cursor: null },
            ]);
        });

        it('rejects an unsafe integer peer clock', () => {
            const handle = createHandle();
            mockNativeModule.editorV2CollaborationPeers.mockReturnValueOnce(
                okRecord(
                    JSON.stringify({
                        peers: [
                            {
                                clientId: '1',
                                clock: Number.MAX_SAFE_INTEGER + 1,
                                isLocal: true,
                                state: null,
                                cursor: null,
                            },
                        ],
                    })
                )
            );
            expectNonRetryable(
                catchThrown(() => handle.bridge.collaborationPeers()),
                'FFI_RESULT_INVALID'
            );
        });

        it('normalizes the socket-close transport state and rejects unknown states', () => {
            const handle = createHandle();
            expect(handle.bridge.collaborationSocketClose('7', 1000, 'bye')).toBe('Disconnected');
            mockNativeModule.editorV2CollaborationSocketClose.mockReturnValueOnce(
                okRecord(JSON.stringify({ transportState: 'Floating' }))
            );
            expectNonRetryable(
                catchThrown(() => handle.bridge.collaborationSocketClose('7', null, null)),
                'FFI_RESULT_INVALID'
            );
        });

        it('serializes awareness state and the literal null withdrawal', () => {
            const handle = createHandle();
            handle.bridge.collaborationSetAwareness({ user: { name: 'Alice' } });
            handle.bridge.collaborationSetAwareness(null);
            const calls = mockNativeModule.editorV2CollaborationSetAwareness.mock.calls;
            expect(JSON.parse(calls[0][1])).toEqual({ user: { name: 'Alice' } });
            expect(calls[1][1]).toBe('null');
        });
    });

    describe('autonomous error events', () => {
        it('delivers exactly one typed error per emission', () => {
            const handle = createHandle();
            const received: NativeEditorV2ErrorBase[] = [];
            handle.addErrorListener((error) => received.push(error));
            handle.bridge._emitAutonomousError({
                domain: 'operation',
                code: 'POSITION_INVALID',
                message: 'position invalid',
                requestId: '3',
            });
            expect(received).toHaveLength(1);
            expect(received[0]).toBeInstanceOf(NativeEditorV2OperationError);
            expect(received[0].code).toBe('POSITION_INVALID');
            expect(received[0].requestId).toBe('3');
        });

        it('accepts the frozen envelope form for autonomous errors', () => {
            const handle = createHandle();
            const received: NativeEditorV2ErrorBase[] = [];
            handle.addErrorListener((error) => received.push(error));
            handle.bridge._emitAutonomousError({
                ok: false,
                error: { domain: 'document', code: 'DOCUMENT_INVALID', message: 'invalid' },
            });
            expect(received).toHaveLength(1);
            expect(received[0]).toBeInstanceOf(NativeEditorV2DocumentError);
        });

        it('reports a malformed autonomous error as a non-retryable contract violation', () => {
            const handle = createHandle();
            const received: NativeEditorV2ErrorBase[] = [];
            handle.addErrorListener((error) => received.push(error));
            handle.bridge._emitAutonomousError({ code: 42 });
            expect(received).toHaveLength(1);
            expectNonRetryable(received[0], 'FFI_RESULT_INVALID');
        });

        it('stops delivery after unsubscribe and after destroy', () => {
            const handle = createHandle();
            const received: NativeEditorV2ErrorBase[] = [];
            const unsubscribe = handle.addErrorListener((error) => received.push(error));
            const emission: NativeEditorV2Error = {
                domain: 'operation',
                code: 'OPERATION_INVALID',
                message: 'operation invalid',
                requestId: null,
                operationIndex: null,
                limit: null,
                actual: null,
                details: null,
            };
            handle.bridge._emitAutonomousError(emission);
            unsubscribe();
            handle.bridge._emitAutonomousError(emission);
            expect(received).toHaveLength(1);
            handle.addErrorListener((error) => received.push(error));
            handle.destroy();
            handle.bridge._emitAutonomousError(emission);
            expect(received).toHaveLength(1);
        });
    });

    describe('v2 surface availability', () => {
        it('fails clearly when the native module does not expose the v2 surface', () => {
            delete mockNativeModule.editorV2Create;
            expect(() => createHandle()).toThrow(/editorV2Create/);
        });
    });

    describe('unit results', () => {
        it('accepts only the literal true success marker', () => {
            expect(normalizeNativeEditorV2Unit(true)).toBe(true);
            expect(normalizeNativeEditorV2Unit(false)).toBeNull();
            expect(normalizeNativeEditorV2Unit('true')).toBeNull();
            expect(normalizeNativeEditorV2Unit(1)).toBeNull();
            expect(normalizeNativeEditorV2Unit(null)).toBeNull();
        });
    });
});
