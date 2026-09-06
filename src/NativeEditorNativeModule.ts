import { requireNativeModule } from 'expo-modules-core';
import { type NativeEditorV2Error } from './NativeEditorBoundaryError';

export let _nativeModule: NativeEditorV2Module | null = null;

export function getNativeModule(): NativeEditorV2Module {
    if (!_nativeModule) {
        _nativeModule = requireNativeModule<NativeEditorV2Module>('NativeEditor');
    }
    return _nativeModule;
}

/** @internal Reset the cached native module reference. For testing only. */
export function _resetNativeModuleCache(): void {
    _nativeModule = null;
}

// The only construction path. Consumes the frozen v2 result records
// ({ value, error }, exactly one side set), normalizes them at the JS boundary
// (decimal-string u64s, direct binaries, unsafe-integer rejection), and raises
// typed per-domain errors with a non-retryable class for
// ENGINE_INVARIANT_FAILED and destroyed lifecycles.

export const ERR_V2_NATIVE_RESPONSE =
    'NativeEditorBridge: invalid v2 result record from native module';

export const ERR_V2_DESTROYED = 'NativeEditorBridge: v2 editor handle has been destroyed';

export const V2_ENVELOPE_VERSION = 1;

/**
 * The v2 surface of the NativeEditor native module — the complete
 * production ABI. Every call resolves the method lazily and fails clearly
 * when the v2 surface is absent. Decimal-string identifiers keep full u64
 * fidelity across the JavaScript boundary, and binaries travel as direct
 * Uint8Array values (never JSON number arrays).
 */
export interface NativeEditorV2Module {
    editorV2Create(configJson: string, snapshotState: Uint8Array | null): unknown;
    editorV2Destroy(editorId: string): unknown;
    editorV2GetState(editorId: string): unknown;
    editorV2GetDocumentJson(editorId: string): unknown;
    editorV2GetDocumentHtml(editorId: string): unknown;
    editorV2GetContentSnapshot(editorId: string): unknown;
    editorV2ReplaceDocument(editorId: string, requestJson: string): unknown;
    editorV2ApplyInput(editorId: string, requestJson: string): unknown;
    editorV2ApplyCommand(editorId: string, requestJson: string): unknown;
    editorV2ApplyLocalApi(editorId: string, requestJson: string): unknown;
    editorV2SetSelection(editorId: string, requestJson: string): unknown;
    editorV2Undo(editorId: string, requestJson: string): unknown;
    editorV2Redo(editorId: string, requestJson: string): unknown;
    editorV2RenderUpdate(
        editorId: string,
        mirrorScalarAnchor: number | null,
        mirrorScalarHead: number | null
    ): unknown;
    editorV2CollaborationConfigureTransport(
        editorId: string,
        configJsonOrNull: string | null
    ): unknown;
    editorV2CollaborationResolveProtocolAdapter(
        editorId: string,
        attemptId: string,
        eventId: string,
        responseJson: string
    ): unknown;
    editorV2CollaborationSetAwareness(editorId: string, awarenessJson: string): unknown;
    editorV2SnapshotExport(editorId: string): unknown;
    editorV2SnapshotRestore(
        editorId: string,
        metadataJson: string,
        encodedState: Uint8Array
    ): unknown;
    addListener(
        eventName: 'onCollaborationTransportEvent',
        listener: (event: unknown) => void
    ): { remove(): void };
}

export function invokeNativeEditorV2<K extends keyof NativeEditorV2Module>(
    name: K,
    ...args: Parameters<NativeEditorV2Module[K]>
): unknown {
    const nativeModule = getNativeModule() as unknown as Record<string, unknown>;
    const method = nativeModule[name as string];
    if (typeof method !== 'function') {
        throw new Error(
            `NativeEditorBridge: native module does not expose the v2 entry ${String(name)}`
        );
    }
    return (method as (...fnArgs: unknown[]) => unknown).apply(nativeModule, args);
}

/** The discriminated envelope every v2 result record normalizes into. */
export type NativeEditorV2Result<T> =
    | { ok: true; value: T }
    | { ok: false; error: NativeEditorV2Error };
