import { type ResolvedDocumentSchema } from './schemas';
import { NativeEditorErrorBase } from './NativeEditorBoundaryError';
import { NativeEditorDocumentBridge } from './NativeEditorDocumentBridge';
import {
    type NativeCollaborationTransportConfig,
    type NativeCollaborationTransportEvent,
} from './NativeEditorCollaborationTransport';
import {
    type NativeEditorLocalAwarenessIntent,
    type NativeEditorCreateConfig,
} from './NativeEditorTypes';
import {
    invalidV2RequestError,
    unwrapNativeEditorV2Result,
} from './NativeEditorResultNormalization';
import { buildV2CreateRequest } from './NativeEditorCreateConfig';
import { invokeNativeEditorV2 } from './NativeEditorNativeModule';
import { normalizeNativeEditorV2CreateValue } from './NativeEditorRenderNormalization';

export const NATIVE_EDITOR_DOCUMENT_HANDLE_BRAND: unique symbol = Symbol(
    'NativeEditorDocumentHandle.brand'
);

export const NATIVE_EDITOR_DOCUMENT_HANDLE_TOKEN = Object.freeze({});

export const AUTHENTIC_NATIVE_EDITOR_DOCUMENT_HANDLES = new WeakSet<object>();

export const NATIVE_EDITOR_DOCUMENT_HANDLE_DESCRIPTORS = new WeakMap<
    object,
    ResolvedDocumentSchema
>();

/**
 * One native document session, shared by everything that touches the
 * document: the editor view, the headless `useNativeEditorDocument` binding,
 * and the collaboration controller. Obtain one from
 * {@link createNativeEditorDocumentHandle} — it cannot be constructed
 * directly — and `destroy()` it when its owner unmounts.
 */
export interface NativeEditorDocumentHandle {
    readonly [NATIVE_EDITOR_DOCUMENT_HANDLE_BRAND]: true;
    /** Decimal-string session id, shared with the native view. */
    readonly editorId: string;
    /** Typed imperative access to the engine. The React APIs use this for you. */
    readonly bridge: NativeEditorDocumentBridge;
    readonly isDestroyed: boolean;
    /** Release the native session. Every later call fails as non-retryable. */
    destroy(): void;
    /** Observe engine failures raised outside a caller's own call. Returns an unsubscribe function. */
    addErrorListener(listener: (error: NativeEditorErrorBase) => void): () => void;
    /** Point the native transport at a server, or pass null to detach. */
    configureCollaborationTransport(config: NativeCollaborationTransportConfig | null): void;
    /** Publish local presence, or pass null to withdraw it. */
    setLocalAwareness(intent: NativeEditorLocalAwarenessIntent | null): void;
    /** Observe transport state, errors, and protocol-adapter requests. Returns an unsubscribe function. */
    addCollaborationTransportListener(
        listener: (event: NativeCollaborationTransportEvent) => void
    ): () => void;
}

export class NativeEditorDocumentHandleImpl implements NativeEditorDocumentHandle {
    readonly [NATIVE_EDITOR_DOCUMENT_HANDLE_BRAND] = true as const;

    constructor(
        token: typeof NATIVE_EDITOR_DOCUMENT_HANDLE_TOKEN,
        public readonly editorId: string,
        public readonly bridge: NativeEditorDocumentBridge,
        documentDescriptor: ResolvedDocumentSchema
    ) {
        if (token !== NATIVE_EDITOR_DOCUMENT_HANDLE_TOKEN) {
            throw invalidV2RequestError(
                'NativeEditorBridge: NativeEditorDocumentHandle cannot be constructed directly'
            );
        }
        AUTHENTIC_NATIVE_EDITOR_DOCUMENT_HANDLES.add(this);
        NATIVE_EDITOR_DOCUMENT_HANDLE_DESCRIPTORS.set(this, documentDescriptor);
    }

    get isDestroyed(): boolean {
        return this.bridge.isDestroyed;
    }

    destroy(): void {
        this.bridge.destroy();
    }

    addErrorListener(listener: (error: NativeEditorErrorBase) => void): () => void {
        return this.bridge.addErrorListener(listener);
    }

    configureCollaborationTransport(config: NativeCollaborationTransportConfig | null): void {
        this.bridge.configureCollaborationTransport(config);
    }

    setLocalAwareness(intent: NativeEditorLocalAwarenessIntent | null): void {
        this.bridge.setLocalAwareness(intent);
    }

    addCollaborationTransportListener(
        listener: (event: NativeCollaborationTransportEvent) => void
    ): () => void {
        return this.bridge.addCollaborationTransportListener(listener);
    }
}

/** @internal Handle-owned schema metadata for source-module view bindings. */
export function _getNativeEditorDocumentHandleDescriptor(
    handle: NativeEditorDocumentHandle
): ResolvedDocumentSchema {
    _assertNativeEditorDocumentHandle(handle);
    const documentDescriptor = NATIVE_EDITOR_DOCUMENT_HANDLE_DESCRIPTORS.get(handle);
    if (documentDescriptor === undefined) {
        throw invalidV2RequestError(
            'NativeEditorBridge: authentic NativeEditorDocumentHandle has no document descriptor'
        );
    }
    return documentDescriptor;
}

/** @internal Source-module boundary assertion; intentionally absent from the package index. */
export function _assertNativeEditorDocumentHandle(
    value: unknown
): asserts value is NativeEditorDocumentHandle {
    if (
        (typeof value !== 'object' && typeof value !== 'function') ||
        value === null ||
        !AUTHENTIC_NATIVE_EDITOR_DOCUMENT_HANDLES.has(value)
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: expected an authentic NativeEditorDocumentHandle'
        );
    }
}

/**
 * Create the native document session every other API binds to. Create it once
 * per document — `useMemo`, not on each render — and destroy it when its
 * owner unmounts.
 *
 * @throws NativeEditorErrorBase when the config is rejected: a malformed
 * schema, an out-of-range limit, or content the engine cannot parse.
 *
 * @example
 * ```ts
 * const documentHandle = useMemo(
 *     () => createNativeEditorDocumentHandle({
 *         initialization: { type: 'localHtml', html: '<p>Hello world</p>' },
 *     }),
 *     []
 * );
 * useEffect(() => () => documentHandle.destroy(), [documentHandle]);
 * ```
 */
export function createNativeEditorDocumentHandle(
    config: NativeEditorCreateConfig
): NativeEditorDocumentHandle {
    const { configJson, snapshotState, documentDescriptor } = buildV2CreateRequest(config);
    const value = unwrapNativeEditorV2Result(
        invokeNativeEditorV2('editorV2Create', configJson, snapshotState),
        normalizeNativeEditorV2CreateValue
    );
    return new NativeEditorDocumentHandleImpl(
        NATIVE_EDITOR_DOCUMENT_HANDLE_TOKEN,
        value.editorId,
        new NativeEditorDocumentBridge(value.editorId),
        documentDescriptor
    );
}
