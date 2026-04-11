import { useEffect, useRef, useState } from 'react';

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
import type { SchemaDefinition } from './schemas';

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
const EMPTY_DOCUMENT: DocumentJSON = {
    type: 'doc',
    content: [
        {
            type: 'paragraph',
        },
    ],
};
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

function acceptingGroupsForContent(content: string, existingChildCount: number): string[] {
    const tokens = content
        .trim()
        .split(/\s+/)
        .filter(Boolean)
        .map((token) => {
            const quantifier = token[token.length - 1];
            if (quantifier === '+' || quantifier === '*' || quantifier === '?') {
                return {
                    group: token.slice(0, -1),
                    min: quantifier === '+' ? 1 : 0,
                    max: quantifier === '?' ? 1 : null,
                };
            }
            return {
                group: token,
                min: 1,
                max: 1,
            };
        });

    let remaining = existingChildCount;
    const acceptingGroups: string[] = [];
    for (const token of tokens) {
        if (remaining >= token.min) {
            const consumed = token.max == null ? remaining : Math.min(remaining, token.max);
            remaining = Math.max(0, remaining - consumed);
            const atMax = token.max != null && consumed >= token.max;
            if (!atMax) {
                acceptingGroups.push(token.group);
            }
            continue;
        }

        acceptingGroups.push(token.group);
        break;
    }

    return acceptingGroups;
}

function defaultEmptyDocument(schema?: SchemaDefinition): DocumentJSON {
    if (!schema) {
        return {
            type: 'doc',
            content: [{ type: 'paragraph' }],
        };
    }

    const docNode = schema.nodes.find((node) => node.role === 'doc' || node.name === 'doc');
    const acceptingGroups =
        docNode == null ? [] : acceptingGroupsForContent(docNode.content ?? '', 0);
    const matchingTextBlocks = schema.nodes.filter(
        (node) =>
            node.role === 'textBlock' &&
            acceptingGroups.some((group) => node.name === group || node.group === group)
    );
    const preferredTextBlock =
        matchingTextBlocks.find((node) => node.htmlTag === 'p' || node.name === 'paragraph') ??
        matchingTextBlocks[0] ??
        schema.nodes.find((node) => node.htmlTag === 'p' || node.name === 'paragraph') ??
        schema.nodes.find((node) => node.role === 'textBlock');

    if (!preferredTextBlock) {
        return {
            type: 'doc',
            content: [{ type: 'paragraph' }],
        };
    }

    return {
        type: 'doc',
        content: [{ type: preferredTextBlock.name }],
    };
}

function initialFallbackDocument(options: YjsCollaborationOptions): DocumentJSON {
    return options.initialDocumentJson
        ? cloneJsonValue(options.initialDocumentJson)
        : defaultEmptyDocument(options.schema);
}

function shouldUseFallbackForNativeDocument(
    doc: DocumentJSON,
    options: YjsCollaborationOptions
): boolean {
    if (options.initialDocumentJson != null || options.initialEncodedState != null) {
        return false;
    }
    if (doc.type !== 'doc') {
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

function normalizeMessageBytes(data: unknown): number[] | null {
    if (data instanceof ArrayBuffer) {
        return Array.from(new Uint8Array(data));
    }
    if (ArrayBuffer.isView(data)) {
        return Array.from(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
    }
    if (typeof data === 'string') {
        try {
            const parsed = JSON.parse(data) as number[];
            return Array.isArray(parsed) ? parsed : null;
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

function encodeInitialStateKey(encodedState: EncodedCollaborationStateInput | undefined): string {
    if (encodedState == null) return '';
    return encodeCollaborationStateBase64(encodedState);
}

class YjsCollaborationControllerImpl implements YjsCollaborationController {
    private readonly bridge: NativeCollaborationBridge;
    private readonly callbacks: MutableCallbacks;
    private readonly createWebSocket: () => WebSocket;
    private readonly retryIntervalMs?: YjsRetryInterval | false;
    private socket: WebSocket | null = null;
    private destroyed = false;
    private retryAttempt = 0;
    private retryTimer: ReturnType<typeof setTimeout> | null = null;
    private isManuallyDisconnected = false;
    private pendingAwarenessTimer: ReturnType<typeof setTimeout> | null = null;
    private localAwarenessState: LocalAwarenessState;
    private _state: YjsCollaborationState;
    private _peers: CollaborationPeer[] = [];

    constructor(options: YjsCollaborationOptions, callbacks: MutableCallbacks = {}) {
        this.callbacks = callbacks;
        this.createWebSocket = options.createWebSocket;
        this.retryIntervalMs = options.retryIntervalMs;
        this.localAwarenessState = {
            user: options.localAwareness,
            focused: false,
        };
        this.bridge = NativeCollaborationBridge.create({
            fragmentName: options.fragmentName ?? DEFAULT_YJS_FRAGMENT_NAME,
            schema: options.schema,
            initialEncodedState: options.initialEncodedState,
            localAwareness: awarenessToRecord(this.localAwarenessState),
        });
        const nativeDocumentJson = this.bridge.getDocumentJson();
        this._state = {
            documentId: options.documentId,
            status: 'idle',
            isConnected: false,
            documentJson:
                options.initialDocumentJson != null
                    ? cloneJsonValue(options.initialDocumentJson)
                    : shouldUseFallbackForNativeDocument(nativeDocumentJson, options)
                      ? defaultEmptyDocument(options.schema)
                      : nativeDocumentJson,
        };
        this._peers = this.bridge.getPeers();
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
            this.retryAttempt = 0;
            this.cancelRetry();
            this.setState({
                status: 'connected',
                isConnected: true,
                lastError: undefined,
            });
            const result = this.bridge.start();
            this.applyResult(result);
            sendBinaryMessages(socket, result.messages);
        };

        socket.onmessage = (event) => {
            if (this.destroyed || this.socket !== socket) return;
            const bytes = normalizeMessageBytes(event.data);
            if (!bytes) return;
            if (readFirstVarUint(bytes) === Y_WEBSOCKET_MESSAGE_QUERY_AWARENESS) {
                try {
                    this.commitLocalAwareness();
                } catch (error) {
                    this.handleTransportFailure(
                        error instanceof Error
                            ? error
                            : new Error('Yjs collaboration protocol error'),
                        socket
                    );
                }
                return;
            }
            try {
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
        if (result.documentChanged && result.documentJson) {
            this.setState({
                documentJson: result.documentJson,
            });
        }
        if (result.peersChanged && result.peers) {
            this._peers = result.peers;
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
    const initialEncodedStateKey = encodeInitialStateKey(options.initialEncodedState);
    const localAwarenessKey = JSON.stringify(options.localAwareness);
    const schemaKey = JSON.stringify(options.schema ?? null);
    const [state, setState] = useState<YjsCollaborationState>({
        documentId: options.documentId,
        status: 'idle',
        isConnected: false,
        documentJson: initialFallbackDocument(options),
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
                }
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
                documentJson: initialFallbackDocument(options),
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
        schemaKey,
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
            remoteSelections: peersToRemoteSelections(peers),
            onContentChangeJSON: (doc) => controllerRef.current?.handleLocalDocumentChange(doc),
            onSelectionChange: (selection) =>
                controllerRef.current?.handleSelectionChange(selection),
            onFocus: () => controllerRef.current?.handleFocusChange(true),
            onBlur: () => controllerRef.current?.handleFocusChange(false),
        },
    };
}
