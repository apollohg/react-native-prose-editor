import {
    NativeEditorErrorBase,
    NativeEditorNonRetryableError,
    nativeEditorV2ErrorToException,
    normalizeNativeEditorV2Error,
} from './NativeEditorBoundaryError';
import {
    type NativeCollaborationProtocolAdapter,
    type NativeCollaborationTransportConfig,
    collaborationTransportWireConfig,
    normalizeNativeCollaborationTransportEvent,
    type NativeCollaborationProtocolAdapterContext,
    serializeCollaborationProtocolAdapterResult,
    type NativeCollaborationTransportEvent,
} from './NativeEditorCollaborationTransport';
import {
    destroyedHandleError,
    unwrapNativeEditorV2Result,
    invalidV2RequestError,
    requireV2DecimalId,
    normalizeNativeEditorV2Unit,
    isPlainRecord,
    invalidV2ResultError,
    type NativeEditorState,
    normalizeNativeEditorV2StateValue,
    normalizeNativeEditorV2HtmlValue,
    requireNativeEditorV2U32,
    type NativeEditorCommitInfo,
    normalizeNativeEditorV2CommitValue,
    type NativeEditorMutationOutcome,
    normalizeNativeEditorV2MutationOutcomeValue,
    normalizeNativeEditorV2ChangedValue,
    requireV2Bytes,
} from './NativeEditorResultNormalization';
import {
    V2_ENVELOPE_VERSION,
    invokeNativeEditorV2,
    getNativeModule,
} from './NativeEditorNativeModule';
import {
    type DocumentJSON,
    type ContentSnapshot,
    type NativeEditorAtomicRenderSnapshot,
    type NativeEditorReplaceDocumentRequest,
    type NativeEditorInputRequest,
    type NativeEditorCommandRequest,
    type NativeEditorLocalApiRequest,
    type NativeEditorSelectionRequest,
    type NativeEditorSnapshotMetadata,
    type NativeEditorLocalAwarenessIntent,
} from './NativeEditorTypes';
import {
    normalizeNativeEditorV2DocumentJsonValue,
    normalizeNativeEditorV2ContentSnapshotValue,
    normalizeNativeEditorV2RenderUpdateValue,
    type NativeEditorSnapshotExport,
    normalizeNativeEditorV2SnapshotExportValue,
} from './NativeEditorRenderNormalization';
import {
    serializeLocalAwarenessIntent,
    validateLocalAwarenessIntent,
} from './NativeEditorLocalAwareness';

/**
 * Typed imperative v2 bridge bound to one decimal-string editor id. Every
 * entry normalizes the frozen result record, keeps revisions as decimal
 * strings, and throws typed errors; results that arrive for a destroyed
 * handle (including re-entrant destroy races) are classified non-retryable.
 */
export class NativeEditorDocumentBridge {
    private readonly _editorId: string;
    private _destroyed = false;
    private _nextRequestId = 0n;
    private readonly _errorListeners = new Set<(error: NativeEditorErrorBase) => void>();
    private _collaborationProtocolAdapter: NativeCollaborationProtocolAdapter | null = null;
    private _collaborationProtocolAdapterSubscription: { remove(): void } | null = null;

    /** @internal Created by createNativeEditorDocumentHandle. */
    constructor(editorId: string) {
        this._editorId = editorId;
    }

    get editorId(): string {
        return this._editorId;
    }

    get isDestroyed(): boolean {
        return this._destroyed;
    }

    private assertAlive(): void {
        if (this._destroyed) throw destroyedHandleError();
    }

    private callV2<T>(invoke: () => unknown, normalizeValue: (value: unknown) => T | null): T {
        this.assertAlive();
        const raw = invoke();
        // A re-entrant destroy racing the native call makes any result
        // arriving now a result for a destroyed handle: non-retryable.
        if (this._destroyed) throw destroyedHandleError();
        return unwrapNativeEditorV2Result(raw, normalizeValue);
    }

    private nextRequestId(): string {
        if (this._nextRequestId >= 18_446_744_073_709_551_615n) {
            throw invalidV2RequestError('NativeEditorBridge: v2 request id exhausted');
        }
        this._nextRequestId += 1n;
        return this._nextRequestId.toString();
    }

    /**
     * Serialize a request envelope with canonical decimal-string u64 fields.
     */
    private buildEnvelopeJson(
        payload: Record<string, unknown>,
        baseDocumentRevision?: string
    ): string {
        const parts: string[] = [
            `"version":${V2_ENVELOPE_VERSION}`,
            `"requestId":"${this.nextRequestId()}"`,
        ];
        if (baseDocumentRevision !== undefined) {
            const digits = requireV2DecimalId(baseDocumentRevision, 'baseDocumentRevision');
            parts.push(`"baseDocumentRevision":"${digits}"`);
        }
        const payloadJson = JSON.stringify(payload);
        const inner = payloadJson.slice(1, payloadJson.length - 1);
        if (inner.length > 0) parts.push(inner);
        return `{${parts.join(',')}}`;
    }

    /** Destroy the session. Repeated destroy is safe. */
    destroy(): void {
        if (this._destroyed) return;
        try {
            unwrapNativeEditorV2Result(
                invokeNativeEditorV2('editorV2Destroy', this._editorId),
                normalizeNativeEditorV2Unit
            );
        } catch (error) {
            // An already-destroyed native session still satisfies the
            // caller's goal; every other failure is reported.
            if (
                error instanceof NativeEditorNonRetryableError &&
                (error.code === 'ENGINE_DESTROYED' || error.code === 'ENGINE_DESTROYING')
            ) {
                // An already-destroyed native session still commits the
                // local teardown below.
            } else {
                throw error;
            }
        }
        this._destroyed = true;
        this._collaborationProtocolAdapter = null;
        this._collaborationProtocolAdapterSubscription?.remove();
        this._collaborationProtocolAdapterSubscription = null;
        this._errorListeners.clear();
    }

    /** Subscribe to autonomous native failures; returns the unsubscribe. */
    addErrorListener(listener: (error: NativeEditorErrorBase) => void): () => void {
        this._errorListeners.add(listener);
        return () => {
            this._errorListeners.delete(listener);
        };
    }

    /**
     * @internal Route one autonomous native failure (input/accessibility) to
     * the error listeners exactly once. Accepts a bare error record or the
     * frozen envelope form; malformed payloads surface as a non-retryable
     * contract violation so the view stays usable.
     */
    _emitAutonomousError(raw: unknown): void {
        if (this._destroyed) return;
        const candidate = isPlainRecord(raw) && 'error' in raw ? raw : { error: raw };
        const normalized = normalizeNativeEditorV2Error(candidate);
        const exception =
            normalized == null
                ? invalidV2ResultError()
                : nativeEditorV2ErrorToException(normalized);
        for (const listener of this._errorListeners) {
            listener(exception);
        }
    }

    getState(): NativeEditorState {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetState', this._editorId),
            normalizeNativeEditorV2StateValue
        );
    }

    getDocumentJson(): DocumentJSON {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetDocumentJson', this._editorId),
            normalizeNativeEditorV2DocumentJsonValue
        );
    }

    getDocumentHtml(): string {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetDocumentHtml', this._editorId),
            normalizeNativeEditorV2HtmlValue
        );
    }

    getContentSnapshot(): ContentSnapshot {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetContentSnapshot', this._editorId),
            normalizeNativeEditorV2ContentSnapshotValue
        );
    }

    /**
     * Fetch the complete typed, immutable render snapshot a bound native view
     * applies after a JS-driven engine change. Without a scalar mirror, its
     * selection is the engine-authoritative selection; a mirror resolves only
     * this snapshot's selection into document and scalar positions.
     */
    renderUpdate(mirrorScalarSelection?: {
        anchor: number;
        head: number;
    }): NativeEditorAtomicRenderSnapshot {
        this.assertAlive();
        const mirrorAnchor = mirrorScalarSelection?.anchor ?? null;
        const mirrorHead = mirrorScalarSelection?.head ?? null;
        if ((mirrorAnchor == null) !== (mirrorHead == null)) {
            throw invalidV2RequestError(
                'NativeEditorBridge: render update mirror requires both scalar anchor and head'
            );
        }
        if (mirrorAnchor != null) requireNativeEditorV2U32(mirrorAnchor, 'mirrorScalarAnchor');
        if (mirrorHead != null) requireNativeEditorV2U32(mirrorHead, 'mirrorScalarHead');
        return this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2RenderUpdate',
                    this._editorId,
                    mirrorAnchor,
                    mirrorHead
                ),
            normalizeNativeEditorV2RenderUpdateValue
        );
    }

    replaceDocument(request: NativeEditorReplaceDocumentRequest): NativeEditorCommitInfo {
        this.assertAlive();
        const payload: Record<string, unknown> = {};
        if (request.setJson !== undefined) payload.setJson = request.setJson;
        if (request.setHtml !== undefined) payload.setHtml = request.setHtml;
        payload.history = request.history;
        const requestJson = this.buildEnvelopeJson(payload);
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ReplaceDocument', this._editorId, requestJson),
            normalizeNativeEditorV2CommitValue
        );
    }

    applyInput(request: NativeEditorInputRequest): NativeEditorMutationOutcome {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson(
            { text: request.text },
            request.baseDocumentRevision
        );
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ApplyInput', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    applyCommand(request: NativeEditorCommandRequest): NativeEditorMutationOutcome {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson(
            { command: request.command },
            request.baseDocumentRevision
        );
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ApplyCommand', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    applyLocalApi(request: NativeEditorLocalApiRequest): NativeEditorMutationOutcome {
        this.assertAlive();
        const payload: Record<string, unknown> = {};
        if (request.setJson !== undefined) payload.setJson = request.setJson;
        if (request.setHtml !== undefined) payload.setHtml = request.setHtml;
        payload.history = request.history;
        const requestJson = this.buildEnvelopeJson(payload, request.baseDocumentRevision);
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ApplyLocalApi', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    setSelection(request: NativeEditorSelectionRequest): NativeEditorMutationOutcome {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson(
            { selection: request.selection },
            request.baseDocumentRevision
        );
        return this.callV2(
            () => invokeNativeEditorV2('editorV2SetSelection', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    undo(): boolean {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson({});
        return this.callV2(
            () => invokeNativeEditorV2('editorV2Undo', this._editorId, requestJson),
            normalizeNativeEditorV2ChangedValue
        );
    }

    redo(): boolean {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson({});
        return this.callV2(
            () => invokeNativeEditorV2('editorV2Redo', this._editorId, requestJson),
            normalizeNativeEditorV2ChangedValue
        );
    }

    snapshotExport(): NativeEditorSnapshotExport {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2SnapshotExport', this._editorId),
            normalizeNativeEditorV2SnapshotExportValue
        );
    }

    snapshotRestore(
        metadata: NativeEditorSnapshotMetadata,
        encodedState: Uint8Array
    ): NativeEditorCommitInfo {
        this.assertAlive();
        const bytes = requireV2Bytes(encodedState, 'snapshot encodedState');
        return this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2SnapshotRestore',
                    this._editorId,
                    JSON.stringify(metadata),
                    bytes
                ),
            normalizeNativeEditorV2CommitValue
        );
    }

    configureCollaborationTransport(config: NativeCollaborationTransportConfig | null): void {
        this.assertAlive();
        const wireConfig = config === null ? null : collaborationTransportWireConfig(config);
        const previousAdapter = this._collaborationProtocolAdapter;
        const nextAdapter = config?.protocolAdapter ?? null;
        if (nextAdapter !== null && this._collaborationProtocolAdapterSubscription === null) {
            this._collaborationProtocolAdapterSubscription = getNativeModule().addListener(
                'onCollaborationTransportEvent',
                (rawEvent) => this.handleCollaborationProtocolAdapterEvent(rawEvent)
            );
        }
        this._collaborationProtocolAdapter = nextAdapter;
        try {
            this.callV2(
                () =>
                    invokeNativeEditorV2(
                        'editorV2CollaborationConfigureTransport',
                        this._editorId,
                        wireConfig === null ? null : JSON.stringify(wireConfig)
                    ),
                normalizeNativeEditorV2Unit
            );
        } catch (error) {
            this._collaborationProtocolAdapter = previousAdapter;
            if (
                previousAdapter === null &&
                this._collaborationProtocolAdapterSubscription !== null
            ) {
                this._collaborationProtocolAdapterSubscription.remove();
                this._collaborationProtocolAdapterSubscription = null;
            }
            throw error;
        }
        if (nextAdapter === null && this._collaborationProtocolAdapterSubscription !== null) {
            this._collaborationProtocolAdapterSubscription.remove();
            this._collaborationProtocolAdapterSubscription = null;
        }
    }

    private handleCollaborationProtocolAdapterEvent(rawEvent: unknown): void {
        if (this._destroyed || this._collaborationProtocolAdapter === null) return;
        const event = normalizeNativeCollaborationTransportEvent(rawEvent);
        if (event?.editorId !== this._editorId || event.kind !== 'protocolAdapter') {
            return;
        }
        const adapter = this._collaborationProtocolAdapter;
        const context: NativeCollaborationProtocolAdapterContext = {
            attemptId: event.attemptId,
            generation: event.generation,
            negotiatedProtocol: event.negotiatedProtocol,
        };
        const callback =
            event.phase === 'open'
                ? () => adapter.onOpen(context)
                : () => adapter.onMessage(context, event.frame!);
        void Promise.resolve()
            .then(callback)
            .then((result) => serializeCollaborationProtocolAdapterResult(result))
            .catch(() => '{"action":"reject"}')
            .then((responseJson) => {
                if (this._destroyed) return;
                try {
                    this.callV2(
                        () =>
                            invokeNativeEditorV2(
                                'editorV2CollaborationResolveProtocolAdapter',
                                this._editorId,
                                event.attemptId,
                                event.eventId,
                                responseJson
                            ),
                        normalizeNativeEditorV2Unit
                    );
                } catch {
                    // Native treats stale attempt responses as no-ops. A live
                    // resolution failure is surfaced by its transport event;
                    // adapter payloads and callback errors are never logged.
                }
            });
    }

    setLocalAwareness(intent: NativeEditorLocalAwarenessIntent | null): void {
        this.assertAlive();
        const awarenessJson =
            intent === null
                ? 'null'
                : serializeLocalAwarenessIntent(validateLocalAwarenessIntent(intent));
        this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2CollaborationSetAwareness',
                    this._editorId,
                    awarenessJson
                ),
            normalizeNativeEditorV2Unit
        );
    }

    addCollaborationTransportListener(
        listener: (event: NativeCollaborationTransportEvent) => void
    ): () => void {
        this.assertAlive();
        const subscription = getNativeModule().addListener(
            'onCollaborationTransportEvent',
            (rawEvent) => {
                if (this._destroyed) return;
                const event = normalizeNativeCollaborationTransportEvent(rawEvent);
                if (event?.editorId === this._editorId) {
                    listener(event);
                }
            }
        );
        return () => subscription.remove();
    }
}
