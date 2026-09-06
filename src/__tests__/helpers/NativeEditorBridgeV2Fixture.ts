// The mock returns raw UniFFI-record-shaped results exactly as the native
// adapters emit them: { value, error } records, direct Uint8Array binaries.

export const MOCK_DOCUMENT_JSON = JSON.stringify({
    type: 'doc',
    content: [
        {
            type: 'paragraph',
            content: [{ type: 'text', text: 'hello world' }],
        },
    ],
});

export const MOCK_V2_STATE = {
    documentState: 'LocalReady',
    transportState: 'Detached',
    renderState: 'Ready',
    documentRevision: '4',
    documentOrigin: 'jsApi',
    stateRevision: '2',
    canUndo: true,
    canRedo: false,
};

export const MOCK_V2_TRANSACTION = {
    type: 'transaction',
    changed: true,
    documentRevision: '5',
    stateRevision: '3',
    canUndo: true,
    canRedo: false,
};

export const HUGE_U64_DECIMAL = '18446744073709551615';

export const ONE_OVER_U64_DECIMAL = '18446744073709551616';

export const MOCK_ATOMIC_RENDER_SNAPSHOT = {
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
    documentIsEmpty: false,
};

export const MOCK_SNAPSHOT_METADATA = {
    formatVersion: 1,
    documentId: 'doc-1',
    lineageId: 'lineage-1',
    fragmentName: 'prosemirror',
    schemaFingerprint: '0123456789abcdef',
};

export const MOCK_SNAPSHOT_BYTES = new Uint8Array([0, 1, 2, 127, 128, 255, 7]);

export const MOCK_PROTOCOL_FRAME = new Uint8Array([0, 3, 9, 200, 17]);

export function mockV2Error(overrides: Record<string, unknown> = {}): Record<string, unknown> {
    return {
        domain: 'operation',
        code: 'OPERATION_INVALID',
        message: 'operation invalid',
        ...overrides,
    };
}

export function okRecord(value: unknown): Record<string, unknown> {
    return { value, error: null };
}

export function errRecord(error: unknown): Record<string, unknown> {
    return { value: null, error };
}

export let mockEditorIdCounter = 0;

export const mockNativeModule: Record<string, jest.Mock> = {};

export const mockCollaborationTransportListeners = new Set<(event: unknown) => void>();

export function resetMockNativeModule() {
    mockEditorIdCounter = 0;
    mockCollaborationTransportListeners.clear();
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
    mockNativeModule.editorV2CollaborationTick = jest.fn(() =>
        okRecord(
            JSON.stringify({
                nextDeadlineMillis: HUGE_U64_DECIMAL,
                renewedLocal: true,
                expiredPeers: ['7', HUGE_U64_DECIMAL],
                outboundChanged: true,
                peersChanged: false,
            })
        )
    );
    mockNativeModule.editorV2CollaborationDetach = jest.fn(() => okRecord(true));
    mockNativeModule.editorV2CollaborationReattach = jest.fn(() => okRecord(true));
    mockNativeModule.editorV2CollaborationConfigureTransport = jest.fn(() => okRecord(true));
    mockNativeModule.editorV2CollaborationResolveProtocolAdapter = jest.fn(() => okRecord(true));
    mockNativeModule.addListener = jest.fn(
        (_eventName: string, listener: (event: unknown) => void) => {
            mockCollaborationTransportListeners.add(listener);
            return {
                remove: jest.fn(() => mockCollaborationTransportListeners.delete(listener)),
            };
        }
    );
}

resetMockNativeModule();

jest.mock('expo-modules-core', () => ({
    requireNativeModule: () => mockNativeModule,
}));

import {
    createNativeEditorLocalAwarenessSelection,
    createNativeEditorDocumentHandle,
    type NativeEditorDocumentHandle,
    type NativeEditorCreateConfig,
    type NativeEditorLocalAwarenessIntent,
    normalizeNativeEditorV2Bytes,
    normalizeNativeEditorV2DecimalId,
    normalizeNativeEditorV2Result,
    normalizeNativeEditorV2Unit,
    requireNativeEditorV2U32,
    unwrapNativeEditorV2Result,
    validEditorMentionTheme,
    _resetNativeModuleCache,
} from '../../NativeEditorBridge';

import * as NativeEditorBridgeExports from '../../NativeEditorBridge';

import {
    NativeEditorBoundaryError,
    NativeEditorEngineBoundaryError,
    NativeEditorDocumentError,
    NativeEditorErrorBase,
    NativeEditorLifecycleError,
    NativeEditorNonRetryableError,
    NativeEditorOperationError,
    NativeEditorSnapshotError,
    NativeEditorTransportError,
    normalizeNativeEditorV2Error,
    type NativeEditorError,
} from '../../NativeEditorBoundaryError';

import { HARD_EDITOR_RESOURCE_LIMITS } from '../../ResourceLimits';

import { join } from 'path';

import ts from 'typescript';

export function createHandle(): NativeEditorDocumentHandle {
    return createNativeEditorDocumentHandle({
        initialization: { type: 'localEmpty' },
    });
}

export function parsedTypeScriptConfig(): ts.ParsedCommandLine {
    const configPath = join(process.cwd(), 'tsconfig.json');
    const config = ts.readConfigFile(configPath, ts.sys.readFile);
    if (config.error) {
        throw new Error(ts.formatDiagnostic(config.error, formatDiagnosticHost));
    }
    return ts.parseJsonConfigFileContent(config.config, ts.sys, process.cwd());
}

export const formatDiagnosticHost: ts.FormatDiagnosticsHost = {
    getCanonicalFileName: (fileName) => fileName,
    getCurrentDirectory: () => process.cwd(),
    getNewLine: () => '\n',
};

export function compileTypeScriptContractFixture(sourceText: string): string {
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

export function emitNativeEditorBridgeDeclaration(): { declaration: string; diagnostics: string } {
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
        if (/\/NativeEditor(?:Bridge|DocumentHandle)\.d\.ts$/.test(fileName)) declaration += output;
    });
    const diagnostics = ts.formatDiagnosticsWithColorAndContext(
        [...ts.getPreEmitDiagnostics(program), ...emit.diagnostics],
        formatDiagnosticHost
    );
    return { declaration, diagnostics };
}

export function expectNonRetryable(error: unknown, code: string): void {
    expect(error).toBeInstanceOf(NativeEditorNonRetryableError);
    expect(error).toBeInstanceOf(NativeEditorErrorBase);
    expect((error as NativeEditorErrorBase).code).toBe(code);
}

export function catchThrown(fn: () => unknown): unknown {
    try {
        fn();
    } catch (error) {
        return error;
    }
    throw new Error('expected the call to throw');
}

export function catchRejectedNativeRecord(fn: () => unknown): unknown {
    const consoleError = jest.spyOn(console, 'error').mockImplementation(() => {});
    try {
        const error = catchThrown(fn);
        expect(consoleError).toHaveBeenCalledTimes(1);
        expect(consoleError).toHaveBeenCalledWith(
            'NativeEditorBridge: native module returned a record this boundary rejected',
            expect.any(String)
        );
        return error;
    } finally {
        consoleError.mockRestore();
    }
}

/**
 * Drain the microtask queue completely. The bridge resolves one protocol
 * adapter event through a multi-hop promise chain around an async callback,
 * so a fixed number of `await Promise.resolve()` hops cannot see the end of
 * it. Yielding to a macrotask does, and stays deterministic.
 */
export function flushMicrotasks(): Promise<void> {
    return new Promise((resolve) => {
        setImmediate(resolve);
    });
}

beforeEach(() => {
    _resetNativeModuleCache();
    resetMockNativeModule();
});
