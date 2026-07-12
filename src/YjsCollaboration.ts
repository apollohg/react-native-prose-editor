import { useEffect, useMemo, useRef, useState } from 'react';

import {
    NativeCollaborationBridge,
    type CollaborationPeer,
    type CollaborationResult,
    type DocumentJSON,
    type EncodedCollaborationStateInput,
    type Selection,
    encodeCollaborationStateBase64,
} from './NativeEditorBridge';
import type { RemoteSelectionDecoration } from './NativeRichTextEditor';
import {
    resolveEditorResourceLimits,
    type EditorResourceLimits,
    type ResolvedEditorResourceLimits,
} from './ResourceLimits';
import { NativeEditorBoundaryError } from './NativeEditorBoundaryError';
import {
    normalizeDocumentJson,
    resolveDocumentDescriptor,
    type ResolvedDocumentSchema,
    type SchemaDefinition,
} from './schemas';

export type YjsTransportStatus = 'idle' | 'connecting' | 'connected' | 'disconnected' | 'error';

export interface YjsRetryContext {
    attempt: number;
    documentId: string;
    lastError?: Error;
}

export type YjsRetryInterval = number | ((context: YjsRetryContext) => number | null | false);

const DEFAULT_RETRY_BASE_INTERVAL_MS = 500;
const DEFAULT_RETRY_MAX_INTERVAL_MS = 30_000;

export interface LocalAwarenessUser {
    userId: string;
    name: string;
    color: string;
    avatarUrl?: string;
    extra?: Record<string, unknown>;
}

export interface LocalAwarenessState {
    user: LocalAwarenessUser;
    selection?: {
        anchor: number;
        head: number;
    };
    focused?: boolean;
}

export interface YjsCollaborationState {
    documentId: string;
    status: YjsTransportStatus;
    isConnected: boolean;
    documentJson: DocumentJSON;
    /** Local selection remapped through the latest collaboration document update. */
    selectionOnValueJSONReset?: Selection;
    lastError?: Error;
}

export interface YjsCollaborationOptions {
    documentId: string;
    createWebSocket: () => WebSocket;
    connect?: boolean;
    retryIntervalMs?: YjsRetryInterval | false;
    fragmentName?: string;
    schema?: SchemaDefinition;
    initialDocumentJson?: DocumentJSON;
    initialEncodedState?: EncodedCollaborationStateInput;
    maxLength?: number;
    resourceLimits?: EditorResourceLimits;
    localAwareness: LocalAwarenessUser;
    onPeersChange?: (peers: CollaborationPeer[]) => void;
    onStateChange?: (state: YjsCollaborationState) => void;
    onError?: (error: Error) => void;
}

export interface YjsCollaborationController {
    readonly state: YjsCollaborationState;
    readonly peers: CollaborationPeer[];
    connect(): void;
    disconnect(): void;
    reconnect(): void;
    destroy(): void;
    getEncodedState(): Uint8Array;
    getEncodedStateBase64(): string;
    applyEncodedState(encodedState: EncodedCollaborationStateInput): void;
    replaceEncodedState(encodedState: EncodedCollaborationStateInput): void;
    updateLocalAwareness(partial: Partial<LocalAwarenessState>): void;
    handleLocalDocumentChange(doc: DocumentJSON): void;
    handleSelectionChange(selection: Selection): void;
    handleFocusChange(focused: boolean): void;
}

export interface UseYjsCollaborationResult {
    state: YjsCollaborationState;
    peers: CollaborationPeer[];
    isConnected: boolean;
    connect(): void;
    disconnect(): void;
    reconnect(): void;
    getEncodedState(): Uint8Array;
    getEncodedStateBase64(): string;
    applyEncodedState(encodedState: EncodedCollaborationStateInput): void;
    replaceEncodedState(encodedState: EncodedCollaborationStateInput): void;
    updateLocalAwareness(partial: Partial<LocalAwarenessState>): void;
    editorBindings: {
        valueJSON: DocumentJSON;
        valueJSONUpdateMode: 'reset';
        preserveSelectionOnValueJSONReset: true;
        selectionOnValueJSONReset?: Selection;
        remoteSelections: RemoteSelectionDecoration[];
        onContentChangeJSON: (doc: DocumentJSON) => void;
        onSelectionChange: (selection: Selection) => void;
        onFocus: () => void;
        onBlur: () => void;
    };
}

interface MutableCallbacks {
    onStateChange?: (state: YjsCollaborationState) => void;
    onPeersChange?: (peers: CollaborationPeer[]) => void;
    onError?: (error: Error) => void;
}

const Y_WEBSOCKET_MESSAGE_QUERY_AWARENESS = 3;
const DEFAULT_YJS_FRAGMENT_NAME = 'default';
const SELECTION_AWARENESS_DEBOUNCE_MS = 40;

function cloneJsonValue<T>(value: T): T {
    if (Array.isArray(value)) {
        return value.map((item) => cloneJsonValue(item)) as T;
    }
    if (value != null && typeof value === 'object') {
        const clone: Record<string, unknown> = {};
        for (const [key, nestedValue] of Object.entries(value as Record<string, unknown>)) {
            clone[key] = cloneJsonValue(nestedValue);
        }
        return clone as T;
    }
    return value;
}

function initialFallbackDocument(
    options: YjsCollaborationOptions,
    descriptor: ResolvedDocumentSchema = resolveDocumentDescriptor(
        options.schema,
        options.resourceLimits
    )
): DocumentJSON {
    return options.initialDocumentJson
        ? normalizeDocumentJson(cloneJsonValue(options.initialDocumentJson), descriptor)
        : cloneJsonValue(descriptor.emptyDocument);
}

function shouldUseFallbackForNativeDocument(
    doc: DocumentJSON,
    options: YjsCollaborationOptions,
    descriptor: ResolvedDocumentSchema
): boolean {
    if (options.initialDocumentJson != null || options.initialEncodedState != null) {
        return false;
    }
    if (doc.type !== descriptor.documentNodeName) {
        return false;
    }
    return !Array.isArray(doc.content) || doc.content.length === 0;
}

function awarenessToRecord(awareness: LocalAwarenessState): Record<string, unknown> {
    return awareness as unknown as Record<string, unknown>;
}

function localAwarenessEquals(left: LocalAwarenessState, right: LocalAwarenessState): boolean {
    const leftSelection = left.selection;
    const rightSelection = right.selection;
    if (leftSelection == null || rightSelection == null) {
        if (leftSelection !== rightSelection) {
            return false;
        }
    } else if (
        leftSelection.anchor !== rightSelection.anchor ||
        leftSelection.head !== rightSelection.head
    ) {
        return false;
    }

    const leftUser = left.user;
    const rightUser = right.user;
    return (
        left.focused === right.focused &&
        leftUser.userId === rightUser.userId &&
        leftUser.name === rightUser.name &&
        leftUser.color === rightUser.color &&
        leftUser.avatarUrl === rightUser.avatarUrl &&
        JSON.stringify(leftUser.extra ?? null) === JSON.stringify(rightUser.extra ?? null)
    );
}

function limitedUtf8ByteLength(value: string, limit: number): number {
    let bytes = 0;
    for (let index = 0; index < value.length; index += 1) {
        const code = value.charCodeAt(index);
        if (code <= 0x7f) bytes += 1;
        else if (code <= 0x7ff) bytes += 2;
        else if (code >= 0xd800 && code <= 0xdbff && index + 1 < value.length) {
            const low = value.charCodeAt(index + 1);
            if (low >= 0xdc00 && low <= 0xdfff) {
                bytes += 4;
                index += 1;
            } else bytes += 3;
        } else bytes += 3;
        if (bytes > limit) return bytes;
    }
    return bytes;
}

function messageLimitError(limit: number, actual: number): NativeEditorBoundaryError {
    return new NativeEditorBoundaryError(
        'INPUT_LIMIT_EXCEEDED',
        `collaboration message exceeds limit ${limit}: ${actual}`,
        limit,
        actual
    );
}

function normalizeMessageBytes(data: unknown, limit: number): number[] | null {
    if (data instanceof ArrayBuffer) {
        if (data.byteLength > limit) throw messageLimitError(limit, data.byteLength);
        return Array.from(new Uint8Array(data));
    }
    if (ArrayBuffer.isView(data)) {
        if (data.byteLength > limit) throw messageLimitError(limit, data.byteLength);
        return Array.from(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
    }
    if (typeof data === 'string') {
        const inputBytes = limitedUtf8ByteLength(data, limit);
        if (inputBytes > limit) throw messageLimitError(limit, inputBytes);
        try {
            const parsed = JSON.parse(data) as number[];
            if (!Array.isArray(parsed)) return null;
            if (parsed.length > limit) throw messageLimitError(limit, parsed.length);
            return parsed;
        } catch {
            return null;
        }
    }
    return null;
}

function sendBinaryMessages(socket: WebSocket | null, messages: readonly number[][]): void {
    if (!socket || socket.readyState !== WebSocket.OPEN) return;
    for (const message of messages) {
        socket.send(Uint8Array.from(message).buffer);
    }
}

function readFirstVarUint(bytes: readonly number[]): number | null {
    let value = 0;
    let shift = 0;

    for (let index = 0; index < bytes.length && index < 5; index += 1) {
        const byte = bytes[index];
        value |= (byte & 0x7f) << shift;
        if ((byte & 0x80) === 0) {
            return value;
        }
        shift += 7;
    }

    return null;
}

function selectionToAwarenessRange(
    selection: Selection
): LocalAwarenessState['selection'] | undefined {
    if (selection.type !== 'text') return undefined;
    return {
        anchor: selection.anchor ?? 0,
        head: selection.head ?? selection.anchor ?? 0,
    };
}

function selectionFromPeerState(state: Record<string, unknown> | null): Selection | undefined {
    if (!state || typeof state !== 'object') return undefined;
    const selection = state.selection;
    if (!selection || typeof selection !== 'object') return undefined;

    const anchor = Number((selection as Record<string, unknown>).anchor);
    const head = Number((selection as Record<string, unknown>).head);
    if (!Number.isFinite(anchor) || !Number.isFinite(head)) return undefined;

    return { type: 'text', anchor, head };
}

function localSelectionFromPeers(peers: readonly CollaborationPeer[]): Selection | undefined {
    const localPeer = peers.find((peer) => peer.isLocal);
    return localPeer ? selectionFromPeerState(localPeer.state) : undefined;
}

function peersToRemoteSelections(peers: readonly CollaborationPeer[]): RemoteSelectionDecoration[] {
    return peers.flatMap((peer) => {
        if (peer.isLocal || !peer.state || typeof peer.state !== 'object') {
            return [];
        }

        const state = peer.state as Record<string, unknown>;
        const selection = state.selection;
        if (!selection || typeof selection !== 'object') {
            return [];
        }
        const anchor = Number((selection as Record<string, unknown>).anchor);
        const head = Number((selection as Record<string, unknown>).head);
        if (!Number.isFinite(anchor) || !Number.isFinite(head)) {
            return [];
        }

        const user =
            state.user && typeof state.user === 'object'
                ? (state.user as Record<string, unknown>)
                : null;

        return [
            {
                clientId: peer.clientId,
                anchor,
                head,
                color:
                    typeof user?.color === 'string' && user.color.length > 0
                        ? user.color
                        : '#007AFF',
                name:
                    typeof user?.name === 'string' && user.name.length > 0 ? user.name : undefined,
                avatarUrl:
                    typeof user?.avatarUrl === 'string' && user.avatarUrl.length > 0
                        ? user.avatarUrl
                        : undefined,
                isFocused: state.focused !== false,
            },
        ];
    });
}

function encodedStateInputLength(encodedState: EncodedCollaborationStateInput): number {
    if (typeof encodedState !== 'string') return encodedState.length;
    let symbols = 0;
    let padding = 0;
    for (let index = 0; index < encodedState.length; index += 1) {
        const char = encodedState[index];
        if (/\s/.test(char)) continue;
        symbols += 1;
        padding = char === '=' ? padding + 1 : 0;
    }
    return symbols === 0
        ? 0
        : Math.max(0, Math.floor((symbols * 3) / 4) - Math.min(padding, 2));
}

function encodeInitialStateKey(
    encodedState: EncodedCollaborationStateInput | undefined,
    limits: ResolvedEditorResourceLimits
): string {
    if (encodedState == null) return '';
    const actual = encodedStateInputLength(encodedState);
    if (actual > limits.maxEncodedStateBytes) {
        throw new NativeEditorBoundaryError(
            'INPUT_LIMIT_EXCEEDED',
            `encoded state exceeds limit ${limits.maxEncodedStateBytes}: ${actual}`,
            limits.maxEncodedStateBytes,
            actual
        );
    }
    return encodeCollaborationStateBase64(encodedState);
}

class YjsCollaborationControllerImpl implements YjsCollaborationController {
    private readonly bridge: NativeCollaborationBridge;
    private readonly callbacks: MutableCallbacks;
    private readonly createWebSocket: () => WebSocket;
    private readonly retryIntervalMs?: YjsRetryInterval | false;
    private readonly resourceLimits: ResolvedEditorResourceLimits;
    private socket: WebSocket | null = null;
    private destroyed = false;
    private retryAttempt = 0;
    private retryTimer: ReturnType<typeof setTimeout> | null = null;
    private isManuallyDisconnected = false;
    private pendingAwarenessTimer: ReturnType<typeof setTimeout> | null = null;
    private localAwarenessState: LocalAwarenessState;
    private _state: YjsCollaborationState;
    private _peers: CollaborationPeer[] = [];

    constructor(
        options: YjsCollaborationOptions,
        callbacks: MutableCallbacks = {},
        documentDescriptor: ResolvedDocumentSchema = resolveDocumentDescriptor(
            options.schema,
            options.resourceLimits
        )
    ) {
        this.callbacks = callbacks;
        this.createWebSocket = options.createWebSocket;
        this.retryIntervalMs = options.retryIntervalMs;
        this.resourceLimits = resolveEditorResourceLimits(options.resourceLimits);
        this.localAwarenessState = {
            user: options.localAwareness,
            focused: false,
        };
        const normalizedInitialDocumentJson =
            options.initialDocumentJson == null
                ? undefined
                : normalizeDocumentJson(
                      cloneJsonValue(options.initialDocumentJson),
                      documentDescriptor
                  );
        this.bridge = NativeCollaborationBridge.create({
            fragmentName: options.fragmentName ?? DEFAULT_YJS_FRAGMENT_NAME,
            schema: options.schema == null ? undefined : documentDescriptor.schema,
            initialDocumentJson: normalizedInitialDocumentJson,
            initialEncodedState: options.initialEncodedState,
            maxLength: options.maxLength,
            localAwareness: awarenessToRecord(this.localAwarenessState),
            resourceLimits: options.resourceLimits,
        });
        const nativeDocumentJson = this.bridge.getDocumentJson();
        this._peers = this.bridge.getPeers();
        let initialDocumentJson: DocumentJSON;
        if (shouldUseFallbackForNativeDocument(nativeDocumentJson, options, documentDescriptor)) {
            initialDocumentJson = cloneJsonValue(documentDescriptor.emptyDocument);
        } else {
            initialDocumentJson = nativeDocumentJson;
        }
        this._state = {
            documentId: options.documentId,
            status: 'idle',
            isConnected: false,
            documentJson: initialDocumentJson,
            selectionOnValueJSONReset: localSelectionFromPeers(this._peers),
        };
        if (options.connect !== false) {
            this.connect();
        }
    }

    get state(): YjsCollaborationState {
        return this._state;
    }

    get peers(): CollaborationPeer[] {
        return this._peers;
    }

    connect(): void {
        if (this.destroyed) return;
        this.isManuallyDisconnected = false;
        this.cancelRetry();
        if (
            this.socket &&
            (this.socket.readyState === WebSocket.OPEN ||
                this.socket.readyState === WebSocket.CONNECTING)
        ) {
            return;
        }

        this.setState({
            status: 'connecting',
            isConnected: false,
            lastError: undefined,
        });

        let socket: WebSocket;
        try {
            socket = this.createWebSocket();
        } catch (cause) {
            const error =
                cause instanceof Error
                    ? cause
                    : new Error('Yjs collaboration transport initialization failed');
            this.setState({
                status: 'error',
                isConnected: false,
                lastError: error,
            });
            this.callbacks.onError?.(error);
            this.scheduleRetry(error);
            return;
        }
        this.socket = socket;
        const binarySocket = socket as WebSocket & { binaryType?: string };
        try {
            binarySocket.binaryType = 'arraybuffer';
        } catch {
            // React Native WebSocket implementations may ignore this.
        }

        socket.onopen = () => {
            if (this.destroyed || this.socket !== socket) return;
            try {
                const result = this.bridge.start();
                this.retryAttempt = 0;
                this.cancelRetry();
                this.applyResult(result);
                this.setState({
                    status: 'connected',
                    isConnected: true,
                    lastError: undefined,
                });
                sendBinaryMessages(socket, result.messages);
            } catch (error) {
                this.handleTransportFailure(
                    error instanceof Error ? error : new Error('Yjs collaboration protocol error'),
                    socket
                );
            }
        };

        socket.onmessage = (event) => {
            if (this.destroyed || this.socket !== socket) return;
            try {
                const bytes = normalizeMessageBytes(
                    event.data,
                    this.resourceLimits.maxCollaborationMessageBytes
                );
                if (!bytes) return;
                if (readFirstVarUint(bytes) === Y_WEBSOCKET_MESSAGE_QUERY_AWARENESS) {
                    this.commitLocalAwareness();
                    return;
                }
                if (this.pendingAwarenessTimer != null) {
                    this.commitLocalAwareness();
                }
                const result = this.bridge.handleMessage(bytes);
                this.applyResult(result);
                sendBinaryMessages(socket, result.messages);
            } catch (error) {
                this.handleTransportFailure(
                    error instanceof Error ? error : new Error('Yjs collaboration protocol error'),
                    socket
                );
            }
        };

        socket.onerror = () => {
            if (this.destroyed || this.socket !== socket) return;
            this.handleTransportFailure(
                new Error('Yjs collaboration transport error'),
                socket,
                false
            );
        };

        socket.onclose = () => {
            if (this.destroyed || this.socket !== socket) return;
            this.socket = null;
            this.clearPeers();
            this.setState({
                status: this._state.status === 'error' ? 'error' : 'disconnected',
                isConnected: false,
            });
            this.scheduleRetry(this._state.lastError);
        };
    }

    disconnect(): void {
        if (this.destroyed) return;
        this.isManuallyDisconnected = true;
        this.retryAttempt = 0;
        this.cancelRetry();
        const socket = this.socket;
        this.socket = null;
        if (socket?.readyState === WebSocket.OPEN) {
            const result = this.bridge.clearLocalAwareness();
            this.applyResult(result);
            sendBinaryMessages(socket, result.messages);
            socket.close();
        }
        if (
            socket &&
            socket.readyState !== WebSocket.CLOSED &&
            socket.readyState !== WebSocket.CLOSING
        ) {
            try {
                socket.close();
            } catch {
                // Ignore close failures while disconnecting locally.
            }
        }
        this.clearPeers();
        this.setState({
            status: 'disconnected',
            isConnected: false,
            lastError: undefined,
        });
    }

    reconnect(): void {
        this.disconnect();
        this.connect();
    }

    getEncodedState(): Uint8Array {
        if (this.destroyed) return new Uint8Array();
        return this.bridge.getEncodedState();
    }

    getEncodedStateBase64(): string {
        if (this.destroyed) return '';
        return this.bridge.getEncodedStateBase64();
    }

    applyEncodedState(encodedState: EncodedCollaborationStateInput): void {
        if (this.destroyed) return;
        const result = this.bridge.applyEncodedState(encodedState);
        this.applyResult(result);
        sendBinaryMessages(this.socket, result.messages);
    }

    replaceEncodedState(encodedState: EncodedCollaborationStateInput): void {
        if (this.destroyed) return;
        const result = this.bridge.replaceEncodedState(encodedState);
        this.applyResult(result);
        sendBinaryMessages(this.socket, result.messages);
    }

    destroy(): void {
        if (this.destroyed) return;
        this.cancelRetry();
        this.cancelPendingAwarenessSync();
        this.disconnect();
        this.destroyed = true;
        this.bridge.destroy();
    }

    updateLocalAwareness(partial: Partial<LocalAwarenessState>): void {
        if (this.destroyed) return;
        this.localAwarenessState = this.mergeLocalAwareness(partial);
        this.commitLocalAwareness();
    }

    handleLocalDocumentChange(doc: DocumentJSON): void {
        if (this.destroyed) return;
        const result = this.bridge.applyLocalDocumentJson(doc);
        this.applyResult(result);
        sendBinaryMessages(this.socket, result.messages);
        if (this.pendingAwarenessTimer != null) {
            this.commitLocalAwareness();
        }
    }

    handleSelectionChange(selection: Selection): void {
        if (this.destroyed) return;
        const nextAwareness = this.mergeLocalAwareness({
            focused: true,
            selection: selectionToAwarenessRange(selection),
        });
        if (localAwarenessEquals(nextAwareness, this.localAwarenessState)) {
            return;
        }
        this.localAwarenessState = nextAwareness;
        this.scheduleAwarenessSync();
    }

    handleFocusChange(focused: boolean): void {
        if (this.destroyed) return;
        const nextAwareness = this.mergeLocalAwareness({ focused });
        if (localAwarenessEquals(nextAwareness, this.localAwarenessState)) {
            if (this.pendingAwarenessTimer != null) {
                this.commitLocalAwareness();
            }
            return;
        }
        this.localAwarenessState = nextAwareness;
        this.commitLocalAwareness();
    }

    private applyResult(result: CollaborationResult): void {
        const didPeersChange = result.peersChanged && result.peers != null;
        if (didPeersChange) {
            this._peers = result.peers!;
        }

        if (result.documentChanged && result.documentJson) {
            this.setState({
                documentJson: result.documentJson,
                selectionOnValueJSONReset: localSelectionFromPeers(this._peers),
            });
        }
        if (didPeersChange) {
            this.callbacks.onPeersChange?.(this._peers);
        }
    }

    private clearPeers(): void {
        if (this._peers.length === 0) return;
        this._peers = [];
        this.callbacks.onPeersChange?.(this._peers);
    }

    private setState(patch: Partial<YjsCollaborationState>): void {
        this._state = {
            ...this._state,
            ...patch,
        };
        this.callbacks.onStateChange?.(this._state);
    }

    private mergeLocalAwareness(partial: Partial<LocalAwarenessState>): LocalAwarenessState {
        return {
            ...this.localAwarenessState,
            ...partial,
            user: {
                ...this.localAwarenessState.user,
                ...(partial.user ?? {}),
            },
        };
    }

    private scheduleAwarenessSync(): void {
        this.cancelPendingAwarenessSync();
        this.pendingAwarenessTimer = setTimeout(() => {
            this.pendingAwarenessTimer = null;
            this.commitLocalAwareness();
        }, SELECTION_AWARENESS_DEBOUNCE_MS);
    }

    private cancelPendingAwarenessSync(): void {
        if (this.pendingAwarenessTimer == null) return;
        clearTimeout(this.pendingAwarenessTimer);
        this.pendingAwarenessTimer = null;
    }

    private commitLocalAwareness(): void {
        this.cancelPendingAwarenessSync();
        const result = this.bridge.setLocalAwareness(awarenessToRecord(this.localAwarenessState));
        this.applyResult(result);
        sendBinaryMessages(this.socket, result.messages);
    }

    private handleTransportFailure(
        error: Error,
        socket: WebSocket,
        closeSocket: boolean = true
    ): void {
        if (this.destroyed || this.socket !== socket) return;
        this.socket = null;
        if (closeSocket && socket.readyState !== WebSocket.CLOSED) {
            try {
                socket.close();
            } catch {
                // Ignore close failures while reporting the original transport error.
            }
        }
        this.clearPeers();
        this.setState({
            status: 'error',
            isConnected: false,
            lastError: error,
        });
        this.callbacks.onError?.(error);
        this.scheduleRetry(error);
    }

    private scheduleRetry(lastError?: Error): void {
        if (this.destroyed || this.isManuallyDisconnected) return;
        const delayMs = this.resolveRetryDelay(lastError);
        if (delayMs == null) return;
        this.cancelRetry();
        this.retryAttempt += 1;
        this.retryTimer = setTimeout(() => {
            this.retryTimer = null;
            if (this.destroyed || this.isManuallyDisconnected) return;
            this.connect();
        }, delayMs);
    }

    private resolveRetryDelay(lastError?: Error): number | null {
        if (this.retryIntervalMs === false) return null;
        const attempt = this.retryAttempt + 1;
        const value =
            this.retryIntervalMs == null
                ? defaultRetryIntervalMs(attempt)
                : typeof this.retryIntervalMs === 'function'
                  ? this.retryIntervalMs({
                        attempt,
                        documentId: this._state.documentId,
                        lastError,
                    })
                  : this.retryIntervalMs;
        if (value === false || value == null) {
            return null;
        }
        if (!Number.isFinite(value) || value < 0) {
            return null;
        }
        return value;
    }

    private cancelRetry(): void {
        if (this.retryTimer == null) return;
        clearTimeout(this.retryTimer);
        this.retryTimer = null;
    }
}

function defaultRetryIntervalMs(attempt: number): number {
    return Math.min(
        DEFAULT_RETRY_BASE_INTERVAL_MS * 2 ** Math.max(0, attempt - 1),
        DEFAULT_RETRY_MAX_INTERVAL_MS
    );
}

export function createYjsCollaborationController(
    options: YjsCollaborationOptions
): YjsCollaborationController {
    return new YjsCollaborationControllerImpl(options, {
        onStateChange: options.onStateChange,
        onPeersChange: options.onPeersChange,
        onError: options.onError,
    });
}

export function useYjsCollaboration(options: YjsCollaborationOptions): UseYjsCollaborationResult {
    const callbacksRef = useRef<MutableCallbacks>({
        onPeersChange: options.onPeersChange,
        onStateChange: options.onStateChange,
        onError: options.onError,
    });
    callbacksRef.current = {
        onPeersChange: options.onPeersChange,
        onStateChange: options.onStateChange,
        onError: options.onError,
    };
    const createWebSocketRef = useRef(options.createWebSocket);
    createWebSocketRef.current = options.createWebSocket;

    const controllerRef = useRef<YjsCollaborationControllerImpl | null>(null);
    const resolvedResourceLimits = resolveEditorResourceLimits(options.resourceLimits);
    const initialEncodedStateKey = encodeInitialStateKey(
        options.initialEncodedState,
        resolvedResourceLimits
    );
    const localAwarenessKey = JSON.stringify(options.localAwareness);
    const schemaKey = JSON.stringify(options.schema ?? null);
    const resourceLimitsKey = JSON.stringify(options.resourceLimits ?? null);
    const documentDescriptor = useMemo(
        () => resolveDocumentDescriptor(options.schema, options.resourceLimits),
        // Stable serialized keys intentionally define semantic configuration equality.
        // eslint-disable-next-line react-hooks/exhaustive-deps
        [schemaKey, resourceLimitsKey]
    );
    const [state, setState] = useState<YjsCollaborationState>({
        documentId: options.documentId,
        status: 'idle',
        isConnected: false,
        documentJson: initialFallbackDocument(options, documentDescriptor),
    });
    const [peers, setPeers] = useState<CollaborationPeer[]>([]);

    useEffect(() => {
        try {
            const controller = new YjsCollaborationControllerImpl(
                {
                    ...options,
                    createWebSocket: () => createWebSocketRef.current(),
                },
                {
                    onStateChange: (nextState) => {
                        setState({ ...nextState });
                        callbacksRef.current.onStateChange?.(nextState);
                    },
                    onPeersChange: (nextPeers) => {
                        setPeers([...nextPeers]);
                        callbacksRef.current.onPeersChange?.(nextPeers);
                    },
                    onError: (error) => {
                        callbacksRef.current.onError?.(error);
                    },
                },
                documentDescriptor
            );
            controllerRef.current = controller;
            setState({ ...controller.state });
            setPeers([...controller.peers]);
        } catch (error) {
            const nextError =
                error instanceof Error
                    ? error
                    : new Error('Yjs collaboration initialization failed');
            const nextState: YjsCollaborationState = {
                documentId: options.documentId,
                status: 'error',
                isConnected: false,
                documentJson: initialFallbackDocument(options, documentDescriptor),
                lastError: nextError,
            };
            controllerRef.current = null;
            setState(nextState);
            setPeers([]);
            callbacksRef.current.onStateChange?.(nextState);
            callbacksRef.current.onError?.(nextError);
        }

        return () => {
            controllerRef.current?.destroy();
            controllerRef.current = null;
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [
        options.documentId,
        options.fragmentName,
        options.maxLength,
        schemaKey,
        resourceLimitsKey,
        initialEncodedStateKey,
    ]);

    useEffect(() => {
        controllerRef.current?.updateLocalAwareness({
            user: options.localAwareness,
        });
    }, [localAwarenessKey, options.localAwareness]);

    useEffect(() => {
        const controller = controllerRef.current;
        if (!controller) return;
        if (options.connect === false) {
            controller.disconnect();
        } else {
            controller.connect();
        }
    }, [options.connect, options.documentId]);

    return {
        state,
        peers,
        isConnected: state.isConnected,
        connect: () => controllerRef.current?.connect(),
        disconnect: () => controllerRef.current?.disconnect(),
        reconnect: () => controllerRef.current?.reconnect(),
        getEncodedState: () => controllerRef.current?.getEncodedState() ?? new Uint8Array(),
        getEncodedStateBase64: () =>
            controllerRef.current?.getEncodedStateBase64() ??
            encodeCollaborationStateBase64(new Uint8Array()),
        applyEncodedState: (encodedState) => controllerRef.current?.applyEncodedState(encodedState),
        replaceEncodedState: (encodedState) =>
            controllerRef.current?.replaceEncodedState(encodedState),
        updateLocalAwareness: (partial) => controllerRef.current?.updateLocalAwareness(partial),
        editorBindings: {
            valueJSON: state.documentJson,
            valueJSONUpdateMode: 'reset',
            preserveSelectionOnValueJSONReset: true,
            selectionOnValueJSONReset: state.selectionOnValueJSONReset,
            remoteSelections: peersToRemoteSelections(peers),
            onContentChangeJSON: (doc) => controllerRef.current?.handleLocalDocumentChange(doc),
            onSelectionChange: (selection) =>
                controllerRef.current?.handleSelectionChange(selection),
            onFocus: () => controllerRef.current?.handleFocusChange(true),
            onBlur: () => controllerRef.current?.handleFocusChange(false),
        },
    };
}
