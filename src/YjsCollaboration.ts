import { useEffect, useRef, useState } from 'react';

import {
    _assertNativeEditorDocumentHandle,
    type DocumentJSON,
    type NativeEditorDocumentHandle,
    type NativeEditorLocalAwarenessIntent,
    type NativeEditorV2PeerInfo,
    type NativeEditorV2TransportState,
    type Selection,
} from './NativeEditorBridge';
/*
 * Keep handle authenticity tied to the same module instance that constructs
 * handles. A process-global registry would itself become discoverable and
 * forgeable; every consumer below imports this module-local assertion.
 */
import type { RemoteSelectionDecoration } from './NativeRichTextEditor';
import {
    NativeEditorV2NonRetryableError,
    NativeEditorV2TransportError,
    nativeEditorV2ErrorToException,
} from './NativeEditorBoundaryError';

/**
 * Transport status rendered by the collaboration controller. This is a pure
 * mapping of the Rust transport lifecycle — the controller keeps no
 * independent state machine:
 *
 *   Detached -> idle            Synchronized -> synchronized
 *   Disconnected -> disconnected  Incompatible -> incompatible
 *   Connecting -> connecting      Destroying/Destroyed -> destroyed
 *   Handshaking -> handshaking
 */
export type YjsTransportStatus =
    | 'idle'
    | 'disconnected'
    | 'connecting'
    | 'handshaking'
    | 'synchronized'
    | 'incompatible'
    | 'destroyed';

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
    selection?: Selection;
    focused?: boolean;
}

export interface YjsCollaborationState {
    documentId: string;
    status: YjsTransportStatus;
    isConnected: boolean;
    /** The last document rendered from Rust; null while the room awaits the server document. */
    documentJson: DocumentJSON | null;
    /** Decimal-string engine document revision; null while the room awaits the server document. */
    documentRevision: string | null;
    lastError?: Error;
}

export interface YjsCollaborationOptions {
    /** Room document identifier rendered in state (the handle owns the authoritative one). */
    documentId: string;
    /**
     * The shared document session. The editor and this controller use the
     * same handle; the controller never creates or destroys native sessions.
     */
    handle: NativeEditorDocumentHandle;
    createWebSocket: () => WebSocket;
    connect?: boolean;
    /** Backoff timer scheduling only — Rust owns retry eligibility. */
    retryIntervalMs?: YjsRetryInterval | false;
    localAwareness?: LocalAwarenessUser;
    onPeersChange?: (peers: NativeEditorV2PeerInfo[]) => void;
    onStateChange?: (state: YjsCollaborationState) => void;
    onError?: (error: Error) => void;
}

export interface YjsCollaborationController {
    readonly state: YjsCollaborationState;
    readonly peers: NativeEditorV2PeerInfo[];
    readonly documentHandle: NativeEditorDocumentHandle;
    connect(): void;
    disconnect(): void;
    reconnect(): void;
    destroy(): void;
    updateLocalAwareness(partial: Partial<LocalAwarenessState>): void;
    handleSelectionChange(selection: Selection): void;
    handleFocusChange(focused: boolean): void;
    /** Flush engine-queued outbound frames after a local engine commit. */
    handleLocalCommit(): void;
}

export interface YjsCollaborationEditorBindings {
    /** The shared session, to hand to the editor. */
    documentHandle: NativeEditorDocumentHandle;
    /** Advances whenever Rust reports a new document revision. */
    documentRevision: string | null;
    remoteSelections: RemoteSelectionDecoration[];
    onSelectionChange: (selection: Selection) => void;
    onFocus: () => void;
    onBlur: () => void;
    /** The editor pings this after each successful local engine mutation. */
    onLocalDocumentCommit: () => void;
}

export interface UseYjsCollaborationResult {
    state: YjsCollaborationState;
    peers: NativeEditorV2PeerInfo[];
    isConnected: boolean;
    connect(): void;
    disconnect(): void;
    reconnect(): void;
    updateLocalAwareness(partial: Partial<LocalAwarenessState>): void;
    editorBindings: YjsCollaborationEditorBindings;
}

interface MutableCallbacks {
    onStateChange?: (state: YjsCollaborationState) => void;
    onPeersChange?: (peers: NativeEditorV2PeerInfo[]) => void;
    onError?: (error: Error) => void;
}

const WS_NORMAL_CLOSE_CODE = 1000;
const WS_PROTOCOL_FAILURE_CLOSE_CODE = 1011;

function mapTransportState(transport: NativeEditorV2TransportState): YjsTransportStatus {
    switch (transport) {
        case 'Detached':
            return 'idle';
        case 'Disconnected':
            return 'disconnected';
        case 'Connecting':
            return 'connecting';
        case 'Handshaking':
            return 'handshaking';
        case 'Synchronized':
            return 'synchronized';
        case 'Incompatible':
            return 'incompatible';
        case 'Destroying':
        case 'Destroyed':
            return 'destroyed';
    }
}

function isStaleGenerationError(error: Error): boolean {
    return (
        error instanceof NativeEditorV2TransportError && error.code === 'TRANSPORT_STALE_GENERATION'
    );
}

function asError(value: unknown, fallbackMessage: string): Error {
    return value instanceof Error ? value : new Error(fallbackMessage);
}

function normalizeFrameBytes(data: unknown): Uint8Array | null {
    if (data instanceof ArrayBuffer) {
        return new Uint8Array(data);
    }
    if (ArrayBuffer.isView(data)) {
        return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    }
    return null;
}

function toExactArrayBuffer(frame: Uint8Array): ArrayBuffer {
    if (frame.byteOffset === 0 && frame.byteLength === frame.buffer.byteLength) {
        return frame.buffer as ArrayBuffer;
    }
    return frame.slice().buffer as ArrayBuffer;
}

function sendFrame(socket: WebSocket, frame: Uint8Array): void {
    if (frame.length === 0) return;
    if (socket.readyState !== WebSocket.OPEN) return;
    socket.send(toExactArrayBuffer(frame));
}

function peersToRemoteSelections(
    peers: readonly NativeEditorV2PeerInfo[]
): RemoteSelectionDecoration[] {
    return peers.flatMap((peer) => {
        if (peer.isLocal) return [];
        const range = peer.cursor;
        if (!range) return [];

        const state = peer.state;
        const user =
            state && typeof state.user === 'object' && state.user !== null
                ? (state.user as Record<string, unknown>)
                : null;

        return [
            {
                clientId: peer.clientId,
                anchor: range.anchor,
                head: range.head,
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
                isFocused: state?.focused !== false,
            },
        ];
    });
}

function mergeAwarenessPartial(
    base: NativeEditorLocalAwarenessIntent,
    partial: Partial<LocalAwarenessState>
): NativeEditorLocalAwarenessIntent {
    const state: Record<string, unknown> = { ...base.state };
    if (partial.user != null) {
        const baseUser =
            state.user != null && typeof state.user === 'object'
                ? (state.user as Record<string, unknown>)
                : {};
        state.user = { ...baseUser, ...partial.user };
    }
    const next: NativeEditorLocalAwarenessIntent = {
        state,
        focused: 'focused' in partial && partial.focused !== undefined ? partial.focused : base.focused,
    };
    if ('selection' in partial) {
        if (partial.selection !== undefined) next.selection = partial.selection;
    } else if (base.selection !== undefined) {
        next.selection = base.selection;
    }
    return next;
}

class YjsCollaborationControllerImpl implements YjsCollaborationController {
    private readonly handle: NativeEditorDocumentHandle;
    private readonly callbacks: MutableCallbacks;
    private readonly createWebSocket: () => WebSocket;
    private readonly retryIntervalMs?: YjsRetryInterval | false;
    private readonly documentId: string;
    private socket: WebSocket | null = null;
    private generation: string | null = null;
    private destroyed = false;
    private retryAttempt = 0;
    private retryTimer: ReturnType<typeof setTimeout> | null = null;
    private isManuallyDisconnected = false;
    private desiredAwareness: NativeEditorLocalAwarenessIntent | null = null;
    private _state: YjsCollaborationState;
    private _peers: NativeEditorV2PeerInfo[] = [];
    private _peersKey = '[]';

    constructor(options: YjsCollaborationOptions, callbacks: MutableCallbacks = {}) {
        this.handle = options.handle;
        this.callbacks = callbacks;
        this.createWebSocket = options.createWebSocket;
        this.retryIntervalMs = options.retryIntervalMs;
        this.documentId = options.documentId;
        if (options.localAwareness != null) {
            this.desiredAwareness = {
                state: { user: { ...options.localAwareness } },
                focused: false,
            };
            this.publishDesiredAwareness();
        }
        this._state = this.readEngineState();
        if (options.connect !== false) {
            this.connect();
        }
    }

    get state(): YjsCollaborationState {
        return this._state;
    }

    get peers(): NativeEditorV2PeerInfo[] {
        return this._peers;
    }

    get documentHandle(): NativeEditorDocumentHandle {
        return this.handle;
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

        let generation: string;
        try {
            generation = this.handle.bridge.collaborationBeginConnect();
        } catch (error) {
            // Rust owns retry eligibility: a begin_connect refusal stops
            // scheduling here, and the engine state is rendered as returned.
            const refusal = asError(error, 'Yjs collaboration connect refused');
            this.callbacks.onError?.(refusal);
            this.renderEngineState({ lastError: refusal });
            return;
        }

        let socket: WebSocket;
        try {
            socket = this.createWebSocket();
        } catch (cause) {
            const error = asError(cause, 'Yjs collaboration transport initialization failed');
            // Render from the transport Rust settled into when retiring the
            // issued generation; discarding it would leave the state stale.
            const transport = this.retireLiveGeneration(generation, null, null);
            this.callbacks.onError?.(error);
            if (transport != null) {
                this.renderTransport(transport, { lastError: error });
            } else {
                this.renderEngineState({ lastError: error });
            }
            this.scheduleRetry(error);
            return;
        }
        this.socket = socket;
        this.generation = generation;
        const binarySocket = socket as WebSocket & { binaryType?: string };
        try {
            binarySocket.binaryType = 'arraybuffer';
        } catch {
            // React Native WebSocket implementations may ignore this.
        }
        this.renderEngineState({ lastError: undefined });

        socket.onopen = () => {
            if (!this.isCurrentSocket(socket, generation)) return;
            try {
                const step1 = this.handle.bridge.collaborationSocketOpen(generation);
                sendFrame(socket, step1);
                this.renderEngineState({ lastError: undefined });
                this.drainOutbound(socket, generation);
            } catch (error) {
                this.handleTransportCallFailure(error, socket, generation);
            }
        };

        socket.onmessage = (event) => {
            if (!this.isCurrentSocket(socket, generation)) return;
            const bytes = normalizeFrameBytes(event.data);
            if (!bytes) return;
            let outcome;
            try {
                outcome = this.handle.bridge.collaborationReceive(generation, bytes);
            } catch (error) {
                this.handleTransportCallFailure(error, socket, generation);
                return;
            }
            if (outcome.close) {
                const error = nativeEditorV2ErrorToException(outcome.close.error);
                this.clearLiveSocket(socket);
                try {
                    socket.close();
                } catch {
                    // Ignore close failures while reporting the classified close.
                }
                this.renderTransport(outcome.transportState, { lastError: error });
                this.refreshPeers();
                this.callbacks.onError?.(error);
                if (outcome.close.disposition === 'retryable') {
                    this.scheduleRetry(error);
                }
                return;
            }
            if (outcome.transportState === 'Synchronized') {
                this.retryAttempt = 0;
            }
            this.renderTransport(outcome.transportState, { lastError: undefined });
            if (outcome.remoteCommitApplied || outcome.documentPromoted) {
                this.refreshDocument();
            }
            this.drainOutbound(socket, generation);
            this.refreshPeers();
        };

        socket.onerror = () => {
            if (!this.isCurrentSocket(socket, generation)) return;
            // Classification happens in onclose through Rust; an error event
            // only hurries the socket toward its close event.
            try {
                socket.close();
            } catch {
                // Ignore close failures while awaiting the close event.
            }
        };

        socket.onclose = (event) => {
            if (!this.isCurrentSocket(socket, generation)) return;
            this.clearLiveSocket(socket);
            const code = typeof event?.code === 'number' ? event.code : null;
            const reason = typeof event?.reason === 'string' ? event.reason : null;
            const transport = this.retireLiveGeneration(generation, code, reason);
            if (transport == null) return;
            this.renderTransport(transport);
            this.refreshPeers();
            if (transport === 'Disconnected') {
                this.scheduleRetry(this._state.lastError);
            }
        };
    }

    disconnect(): void {
        if (this.destroyed) return;
        this.isManuallyDisconnected = true;
        this.retryAttempt = 0;
        this.cancelRetry();
        const socket = this.socket;
        const generation = this.generation;
        this.socket = null;
        this.generation = null;
        if (socket && generation) {
            const transport = this.retireLiveGeneration(
                generation,
                WS_NORMAL_CLOSE_CODE,
                'client disconnect'
            );
            if (transport != null) {
                this.renderTransport(transport, { lastError: undefined });
            }
            try {
                socket.close();
            } catch {
                // Ignore close failures while disconnecting locally.
            }
        } else {
            this.renderEngineState({ lastError: undefined });
        }
        this.refreshPeers();
    }

    reconnect(): void {
        if (this.destroyed) return;
        this.disconnect();
        try {
            this.handle.bridge.collaborationDetach();
            this.handle.bridge.collaborationReattach();
        } catch (error) {
            const lifecycleError = asError(error, 'Yjs collaboration reconnect lifecycle failure');
            this.callbacks.onError?.(lifecycleError);
            this.renderEngineState({ lastError: lifecycleError });
            return;
        }
        this.connect();
    }

    destroy(): void {
        if (this.destroyed) return;
        this.destroyed = true;
        this.cancelRetry();
        const socket = this.socket;
        const generation = this.generation;
        this.socket = null;
        this.generation = null;
        if (socket && generation) {
            try {
                this.handle.bridge.collaborationSocketClose(
                    generation,
                    WS_NORMAL_CLOSE_CODE,
                    'controller destroyed'
                );
            } catch {
                // The shared handle may already be gone; destroy stays terminal.
            }
            try {
                socket.close();
            } catch {
                // Ignore close failures during teardown.
            }
        }
        this.setState({ status: 'destroyed', isConnected: false });
    }

    updateLocalAwareness(partial: Partial<LocalAwarenessState>): void {
        if (this.destroyed) return;
        const next = mergeAwarenessPartial(
            this.desiredAwareness ?? { state: {}, focused: false },
            partial
        );
        const nextKey = JSON.stringify(next);
        if (nextKey === JSON.stringify(this.desiredAwareness)) return;
        this.desiredAwareness = next;
        this.publishDesiredAwareness();
    }

    handleSelectionChange(selection: Selection): void {
        if (this.destroyed || this.desiredAwareness == null) return;
        const next = mergeAwarenessPartial(this.desiredAwareness, { focused: true, selection });
        if (JSON.stringify(next) === JSON.stringify(this.desiredAwareness)) return;
        this.desiredAwareness = next;
        this.publishDesiredAwareness();
    }

    handleFocusChange(focused: boolean): void {
        if (this.destroyed || this.desiredAwareness == null) return;
        const next = mergeAwarenessPartial(this.desiredAwareness, { focused });
        if (JSON.stringify(next) === JSON.stringify(this.desiredAwareness)) return;
        this.desiredAwareness = next;
        this.publishDesiredAwareness();
    }

    setLocalAwarenessUser(user: LocalAwarenessUser | null): void {
        if (this.destroyed) return;
        if (user == null) {
            if (this.desiredAwareness == null) return;
            this.desiredAwareness = null;
            this.publishDesiredAwareness();
            return;
        }
        this.updateLocalAwareness({ user });
    }

    handleLocalCommit(): void {
        if (this.destroyed) return;
        const socket = this.socket;
        const generation = this.generation;
        this.refreshDocument();
        if (!socket || !generation) return;
        this.drainOutbound(socket, generation);
    }

    // ── Internals ───────────────────────────────────────────────

    private isCurrentSocket(socket: WebSocket, generation: string): boolean {
        return !this.destroyed && this.socket === socket && this.generation === generation;
    }

    private clearLiveSocket(socket: WebSocket): void {
        if (this.socket === socket) {
            this.socket = null;
            this.generation = null;
        }
    }

    /**
     * Report a socket close for the live generation and return the transport
     * state Rust settled into; null when Rust rejected the report (a lost
     * generation race, which never mutates rendered state).
     */
    private retireLiveGeneration(
        generation: string,
        code: number | null,
        reason: string | null
    ): NativeEditorV2TransportState | null {
        try {
            return this.handle.bridge.collaborationSocketClose(generation, code, reason);
        } catch (error) {
            this.routeGenerationRaceError(error);
            return null;
        }
    }

    private handleTransportCallFailure(
        value: unknown,
        socket: WebSocket,
        generation: string
    ): void {
        const error = asError(value, 'Yjs collaboration protocol error');
        if (isStaleGenerationError(error)) {
            // A lost generation race: observable only through the autonomous
            // error listener, never through rendered state.
            this.callbacks.onError?.(error);
            return;
        }
        this.callbacks.onError?.(error);
        if (error instanceof NativeEditorV2NonRetryableError) {
            this.renderEngineState({ lastError: error });
            return;
        }
        this.clearLiveSocket(socket);
        try {
            socket.close();
        } catch {
            // Ignore close failures while reporting the failure to Rust.
        }
        const transport = this.retireLiveGeneration(
            generation,
            WS_PROTOCOL_FAILURE_CLOSE_CODE,
            'protocol failure'
        );
        if (transport == null) return;
        this.renderTransport(transport, { lastError: error });
        this.refreshPeers();
        if (transport === 'Disconnected') {
            this.scheduleRetry(error);
        }
    }

    private routeGenerationRaceError(value: unknown): void {
        const error = asError(value, 'Yjs collaboration transport error');
        this.callbacks.onError?.(error);
        if (!isStaleGenerationError(error) && !(error instanceof NativeEditorV2NonRetryableError)) {
            this.renderEngineState({ lastError: error });
        }
    }

    private drainOutbound(socket: WebSocket, generation: string): void {
        if (!this.isCurrentSocket(socket, generation)) return;
        if (socket.readyState !== WebSocket.OPEN) return;
        try {
            for (;;) {
                const frame = this.handle.bridge.collaborationTakeOutbound(generation);
                if (frame.length === 0) return;
                socket.send(toExactArrayBuffer(frame));
            }
        } catch (error) {
            this.handleTransportCallFailure(error, socket, generation);
        }
    }

    private publishDesiredAwareness(): void {
        try {
            this.handle.bridge.collaborationSetAwareness(this.desiredAwareness);
        } catch (error) {
            this.callbacks.onError?.(asError(error, 'Yjs collaboration awareness failure'));
            return;
        }
        const socket = this.socket;
        const generation = this.generation;
        if (socket && generation && socket.readyState === WebSocket.OPEN) {
            this.drainOutbound(socket, generation);
        }
        this.refreshPeers();
    }

    private readEngineState(): YjsCollaborationState {
        const engineState = this.handle.bridge.getState();
        const awaitingRemote = engineState.documentState === 'AwaitRemote';
        return {
            documentId: this.documentId,
            status: mapTransportState(engineState.transportState),
            isConnected: engineState.transportState === 'Synchronized',
            documentJson: awaitingRemote ? null : this.handle.bridge.getDocumentJson(),
            documentRevision: awaitingRemote ? null : engineState.documentRevision,
        };
    }

    private renderEngineState(patch?: Partial<YjsCollaborationState>): void {
        try {
            const next = this.readEngineState();
            this.setState({ ...next, ...patch });
        } catch (error) {
            this.setState({
                lastError: asError(error, 'Yjs collaboration engine state failure'),
                ...patch,
            });
        }
    }

    private renderTransport(
        transport: NativeEditorV2TransportState,
        patch?: Partial<YjsCollaborationState>
    ): void {
        this.setState({
            status: mapTransportState(transport),
            isConnected: transport === 'Synchronized',
            ...patch,
        });
    }

    private refreshDocument(): void {
        try {
            const engineState = this.handle.bridge.getState();
            const awaitingRemote = engineState.documentState === 'AwaitRemote';
            this.setState({
                documentJson: awaitingRemote ? null : this.handle.bridge.getDocumentJson(),
                documentRevision: awaitingRemote ? null : engineState.documentRevision,
            });
        } catch (error) {
            this.setState({
                lastError: asError(error, 'Yjs collaboration document refresh failure'),
            });
        }
    }

    private refreshPeers(): void {
        let peers: NativeEditorV2PeerInfo[];
        try {
            peers = this.handle.bridge.collaborationPeers();
        } catch {
            return;
        }
        const key = JSON.stringify(peers);
        if (key === this._peersKey) return;
        this._peers = peers;
        this._peersKey = key;
        this.callbacks.onPeersChange?.(peers);
    }

    private setState(patch: Partial<YjsCollaborationState>): void {
        this._state = {
            ...this._state,
            ...patch,
        };
        this.callbacks.onStateChange?.(this._state);
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
            // A retry timer can only request begin_connect; it stops when
            // Rust refuses.
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
                        documentId: this.documentId,
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
    _assertNativeEditorDocumentHandle(options.handle);
    return new YjsCollaborationControllerImpl(options, {
        onStateChange: options.onStateChange,
        onPeersChange: options.onPeersChange,
        onError: options.onError,
    });
}

export function useYjsCollaboration(options: YjsCollaborationOptions): UseYjsCollaborationResult {
    _assertNativeEditorDocumentHandle(options.handle);
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
    const localAwarenessKey = JSON.stringify(options.localAwareness ?? null);
    const [state, setState] = useState<YjsCollaborationState>({
        documentId: options.documentId,
        status: 'idle',
        isConnected: false,
        documentJson: null,
        documentRevision: null,
    });
    const [peers, setPeers] = useState<NativeEditorV2PeerInfo[]>([]);

    useEffect(() => {
        let controller: YjsCollaborationControllerImpl;
        try {
            controller = new YjsCollaborationControllerImpl(
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
            const nextError = asError(error, 'Yjs collaboration initialization failed');
            controllerRef.current = null;
            setState({
                documentId: options.documentId,
                status: 'idle',
                isConnected: false,
                documentJson: null,
                documentRevision: null,
                lastError: nextError,
            });
            setPeers([]);
            callbacksRef.current.onError?.(nextError);
        }

        return () => {
            controllerRef.current?.destroy();
            controllerRef.current = null;
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [options.documentId, options.handle]);

    useEffect(() => {
        controllerRef.current?.setLocalAwarenessUser(options.localAwareness ?? null);
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
        updateLocalAwareness: (partial) => controllerRef.current?.updateLocalAwareness(partial),
        editorBindings: {
            documentHandle: options.handle,
            documentRevision: state.documentRevision,
            remoteSelections: peersToRemoteSelections(peers),
            onSelectionChange: (selection) => controllerRef.current?.handleSelectionChange(selection),
            onFocus: () => controllerRef.current?.handleFocusChange(true),
            onBlur: () => controllerRef.current?.handleFocusChange(false),
            onLocalDocumentCommit: () => controllerRef.current?.handleLocalCommit(),
        },
    };
}
