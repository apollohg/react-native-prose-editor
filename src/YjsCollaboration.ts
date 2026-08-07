import { useEffect, useMemo, useRef, useState } from 'react';

import {
    _assertNativeEditorDocumentHandle,
    createNativeEditorLocalAwarenessSelection,
    type DocumentJSON,
    type NativeCollaborationProtocolAdapter,
    type NativeCollaborationTransportConfig,
    type NativeCollaborationTransportEvent,
    type NativeEditorDocumentHandle,
    type NativeEditorLocalAwarenessIntent,
    type NativeEditorLocalAwarenessSelection,
    type NativeEditorV2EditorState,
    type NativeEditorV2PeerInfo,
    type NativeEditorV2TransportState,
    type Selection,
} from './NativeEditorBridge';
import type { RemoteSelectionDecoration } from './NativeRichTextEditor';

/**
 * Transport lifecycle, projected from the Rust transport state.
 *
 * - `idle` — no transport configured; the handle is detached.
 * - `disconnected` — configured but not currently connected.
 * - `connecting` — opening the physical socket.
 * - `handshaking` — socket open, Yjs sync in progress.
 * - `synchronized` — sync complete; edits flow in both directions.
 * - `incompatible` — the server's document cannot be reconciled with this one.
 * - `destroyed` — the handle was destroyed.
 */
export type YjsTransportStatus =
    | 'idle'
    | 'disconnected'
    | 'connecting'
    | 'handshaking'
    | 'synchronized'
    | 'incompatible'
    | 'destroyed';

/** Identity this client publishes to other peers through awareness. */
export interface LocalAwarenessUser {
    /** Application-level user identity. */
    userId: string;
    /** Display name shown beside this user's remote caret. */
    name: string;
    /** Caret and label color, as a color string. */
    color: string;
    avatarUrl?: string;
    /** Extra application data carried alongside the user record. */
    extra?: Record<string, unknown>;
}

/** The full local awareness record: who this client is, and where its caret sits. */
export interface LocalAwarenessState {
    user: LocalAwarenessUser;
    /** Local caret. Only a text selection is published; anything else clears the cursor. */
    selection?: Selection;
    /** Whether the editor currently holds focus. */
    focused?: boolean;
}

/** Observable state of a collaboration session. */
export interface YjsCollaborationState {
    documentId: string;
    status: YjsTransportStatus;
    /** True only while `status` is `'synchronized'`. */
    isConnected: boolean;
    /** Latest engine document, or null while awaiting the server's document. */
    documentJson: DocumentJSON | null;
    /** Decimal-string engine revision, or null while awaiting the server's document. */
    documentRevision: string | null;
    /** The most recent transport or engine failure, if any. */
    lastError?: Error;
}

/**
 * Configuration for {@link useYjsCollaboration} and
 * {@link createYjsCollaborationController}.
 */
export interface YjsCollaborationOptions {
    /** Room identity. Must match the `documentId` the handle was created with. */
    documentId: string;
    /** The shared document session. Create it with `initialization.type === 'room'`. */
    handle: NativeEditorDocumentHandle;
    /** Server endpoint and connect intent, or null to stay detached. */
    transport: NativeCollaborationTransportConfig | null;
    /** Identity published to other peers. Omit to join without presence. */
    localAwareness?: LocalAwarenessUser;
    /** Called whenever the peer list changes. */
    onPeersChange?: (peers: NativeEditorV2PeerInfo[]) => void;
    /** Called on every state transition. */
    onStateChange?: (state: YjsCollaborationState) => void;
    /** Called on transport and engine failures. */
    onError?: (error: Error) => void;
}

/**
 * Imperative collaboration session, for hosts outside React. The React entry
 * point is {@link useYjsCollaboration}.
 */
export interface YjsCollaborationController {
    readonly state: YjsCollaborationState;
    readonly peers: NativeEditorV2PeerInfo[];
    readonly documentHandle: NativeEditorDocumentHandle;
    /** Open the transport. */
    connect(): void;
    /** Close the transport, keeping the document handle alive. */
    disconnect(): void;
    /** Close and reopen the transport. */
    reconnect(): void;
    /** Tear down the controller. Does not destroy the document handle. */
    destroy(): void;
    /** Publish a partial awareness update. Omitting `selection` keeps the published cursor. */
    updateLocalAwareness(partial: Partial<LocalAwarenessState>): void;
    /** Publish a new local caret position. */
    handleSelectionChange(selection: Selection): void;
    /** Publish a focus change. */
    handleFocusChange(focused: boolean): void;
}

/**
 * Props bound to one collaboration-aware editor view. There is no
 * selection binding: the native adapters publish the local caret straight
 * into Rust, which holds it as a sticky index. Mirroring it through
 * JavaScript would only add a stale copy of a document position.
 */
export interface YjsCollaborationEditorBindings {
    documentHandle: NativeEditorDocumentHandle;
    documentRevision: string | null;
    remoteSelections: RemoteSelectionDecoration[];
    onFocus: () => void;
    onBlur: () => void;
}

/** What {@link useYjsCollaboration} returns. */
export interface UseYjsCollaborationResult {
    state: YjsCollaborationState;
    /** Connected peers, including the local one. */
    peers: NativeEditorV2PeerInfo[];
    /** True only while the transport is synchronized. */
    isConnected: boolean;
    connect(): void;
    disconnect(): void;
    reconnect(): void;
    updateLocalAwareness(partial: Partial<LocalAwarenessState>): void;
    /** Spread onto `NativeRichTextEditor` to bind the editor to this session. */
    editorBindings: YjsCollaborationEditorBindings;
}

interface MutableCallbacks {
    onStateChange?: (state: YjsCollaborationState) => void;
    onPeersChange?: (peers: NativeEditorV2PeerInfo[]) => void;
    onError?: (error: Error) => void;
}

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

function asError(value: unknown, fallbackMessage: string): Error {
    return value instanceof Error ? value : new Error(fallbackMessage);
}

function isStrictlyNewerSequence(candidate: string, current: string | null): boolean {
    if (current === null) return true;
    if (candidate.length !== current.length) return candidate.length > current.length;
    return candidate > current;
}

function peersToRemoteSelections(
    peers: readonly NativeEditorV2PeerInfo[]
): RemoteSelectionDecoration[] {
    return peers.flatMap((peer) => {
        if (peer.isLocal || peer.cursor == null) return [];
        const applicationState =
            peer.state && typeof peer.state.state === 'object' && peer.state.state !== null
                ? (peer.state.state as Record<string, unknown>)
                : null;
        const user =
            applicationState &&
            typeof applicationState.user === 'object' &&
            applicationState.user !== null
                ? (applicationState.user as Record<string, unknown>)
                : null;
        return [
            {
                clientId: peer.clientId,
                anchor: peer.cursor.anchor,
                head: peer.cursor.head,
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
                isFocused: peer.state?.focused !== false,
            },
        ];
    });
}

/**
 * Narrow a caller selection to the text-only cursor awareness carries.
 * Anything else — a node or all-document selection, or an absent one — is
 * an explicit "no cursor", never a silently retained stale position.
 */
function normalizeAwarenessSelection(
    selection: Selection | undefined
): NativeEditorLocalAwarenessSelection | undefined {
    if (selection === undefined || selection.type !== 'text') return undefined;
    if (selection.anchor === undefined || selection.head === undefined) return undefined;
    return createNativeEditorLocalAwarenessSelection(selection.anchor, selection.head);
}

/**
 * Copy the caller's transport intent into a controller-owned record. A
 * nullish config means "no transport": the handle stays detached, exactly
 * as the transport-less reads elsewhere in this module assume.
 */
function copyTransportConfig(
    config: NativeCollaborationTransportConfig | null | undefined
): NativeCollaborationTransportConfig | null {
    if (config == null) return null;
    return {
        url: config.url,
        connect: config.connect,
        ...(config.protocolAdapter === undefined
            ? {}
            : { protocolAdapter: config.protocolAdapter }),
    };
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
        focused:
            'focused' in partial && partial.focused !== undefined
                ? partial.focused
                : base.focused,
    };
    // Selection is stated only when the caller states it. Rust holds the
    // local cursor as a sticky index that already tracks every document
    // change, so an omitted key means "retain it" — never "resend the last
    // position I happened to see", which the document may have invalidated.
    if ('selection' in partial) {
        next.selection = normalizeAwarenessSelection(partial.selection) ?? null;
    }
    return next;
}

class YjsCollaborationControllerImpl implements YjsCollaborationController {
    private readonly handle: NativeEditorDocumentHandle;
    private readonly documentId: string;
    private readonly callbacks: MutableCallbacks;
    private transport: NativeCollaborationTransportConfig | null;
    private removeTransportListener: (() => void) | null = null;
    private desiredAwareness: NativeEditorLocalAwarenessIntent | null = null;
    /** Serialized form of the last intent Rust accepted, for dedup. */
    private publishedAwarenessJson: string | null = null;
    private lastEventSequence: string | null = null;
    private destroyed = false;
    private _state: YjsCollaborationState;
    private _peers: NativeEditorV2PeerInfo[] = [];

    constructor(options: YjsCollaborationOptions, callbacks: MutableCallbacks = {}) {
        _assertNativeEditorDocumentHandle(options.handle);
        this.handle = options.handle;
        this.documentId = options.documentId;
        this.callbacks = callbacks;
        this.transport = copyTransportConfig(options.transport);
        this._state = this.readEngineState(this.handle.bridge.getState());
        this.removeTransportListener = this.handle.addCollaborationTransportListener((event) => {
            this.handleTransportEvent(event);
        });
        try {
            this.applyLocalAwarenessOption(options.localAwareness);
            this.handle.configureCollaborationTransport(this.transport);
        } catch (error) {
            this.removeTransportListener();
            this.removeTransportListener = null;
            throw error;
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
        this.setConnectionEnabled(true);
    }

    disconnect(): void {
        this.setConnectionEnabled(false);
    }

    reconnect(): void {
        if (this.destroyed || this.transport === null) return;
        this.handle.configureCollaborationTransport({ ...this.transport, connect: false });
        this.transport = { ...this.transport, connect: true };
        this.handle.configureCollaborationTransport(this.transport);
    }

    destroy(): void {
        if (this.destroyed) return;
        this.destroyed = true;
        this.removeTransportListener?.();
        this.removeTransportListener = null;
        if (!this.handle.isDestroyed) {
            this.handle.configureCollaborationTransport(null);
        }
    }

    updateLocalAwareness(partial: Partial<LocalAwarenessState>): void {
        if (this.destroyed) return;
        // Presence exists only while there is a local user. Once awareness
        // is withdrawn, focus and selection updates must not resurrect it
        // as an anonymous entry; only a fresh user re-establishes it.
        if (this.desiredAwareness === null && partial.user == null) return;
        const base = this.desiredAwareness ?? { state: {}, focused: false };
        this.publishAwareness(mergeAwarenessPartial(base, partial));
    }

    handleSelectionChange(selection: Selection): void {
        this.updateLocalAwareness({ selection });
    }

    handleFocusChange(focused: boolean): void {
        this.updateLocalAwareness({ focused });
    }

    applyLocalAwarenessOption(user?: LocalAwarenessUser): void {
        if (this.destroyed) return;
        if (user == null) {
            // Always withdraw: a fresh controller inherits whatever presence
            // a previous one left retained natively on this shared handle.
            try {
                this.handle.setLocalAwareness(null);
            } catch (error) {
                this.reportError(asError(error, 'Local awareness withdrawal failed'));
                return;
            }
            this.desiredAwareness = null;
            this.publishedAwarenessJson = null;
            return;
        }
        const current = this.desiredAwareness;
        // No `selection` key: the Rust-owned cursor carries across a user
        // change untouched.
        this.publishAwareness({
            state: {
                ...(current?.state ?? {}),
                user: { ...user },
            },
            focused: current?.focused ?? false,
        });
    }

    private setConnectionEnabled(connect: boolean): void {
        if (this.destroyed || this.transport === null) return;
        this.transport = { ...this.transport, connect };
        this.handle.configureCollaborationTransport(this.transport);
    }

    /**
     * Hand one desired presence to Rust. An intent identical to the last
     * accepted one is skipped: republishing it would mint a fresh awareness
     * clock and broadcast a frame that carries no new information.
     * `desiredAwareness` advances only after native acceptance.
     *
     * Presence is ambient UI state, not user data. A refusal is reported
     * through `onError` and retried on the next change — it never escapes
     * into a host focus, blur, or selection handler, and never fails an
     * edit. The candidate is discarded so state stays consistent with Rust.
     */
    private publishAwareness(intent: NativeEditorLocalAwarenessIntent): void {
        let intentJson: string;
        try {
            // Serializing caller-owned application state can itself fail on
            // a cyclic or non-encodable value, so it stays inside the guard.
            intentJson = JSON.stringify(intent);
            if (intentJson === this.publishedAwarenessJson) return;
            this.handle.setLocalAwareness(intent);
        } catch (error) {
            this.reportError(asError(error, 'Local awareness publication failed'));
            return;
        }
        this.desiredAwareness = intent;
        this.publishedAwarenessJson = intentJson;
    }

    private reportError(error: Error): void {
        this.setState({ lastError: error });
        this.callbacks.onError?.(error);
    }

    private handleTransportEvent(event: NativeCollaborationTransportEvent): void {
        if (
            this.destroyed ||
            !isStrictlyNewerSequence(event.eventSequence, this.lastEventSequence)
        ) {
            return;
        }
        this.lastEventSequence = event.eventSequence;
        if (event.kind === 'error') {
            const error = event.error ?? new Error('Native collaboration transport failed');
            this.setState({ lastError: error });
            this.callbacks.onError?.(error);
            return;
        }
        if (event.kind !== 'state') return;
        this._peers = [...event.peers];
        this.callbacks.onPeersChange?.(this._peers);
        try {
            this.setState({
                ...this.readEngineState(event.state, this._state),
                lastError: undefined,
            });
        } catch (error) {
            const nextError = asError(error, 'Native collaboration document refresh failed');
            this.setState({ lastError: nextError });
            this.callbacks.onError?.(nextError);
        }
    }

    private readEngineState(
        engineState: NativeEditorV2EditorState,
        previous?: YjsCollaborationState
    ): YjsCollaborationState {
        const awaitingRemote = engineState.documentState === 'AwaitRemote';
        const documentJson =
            !awaitingRemote &&
            previous?.documentRevision === engineState.documentRevision &&
            previous.documentJson !== null
                ? previous.documentJson
                : awaitingRemote
                  ? null
                  : this.handle.bridge.getDocumentJson();
        return {
            documentId: this.documentId,
            status: mapTransportState(engineState.transportState),
            isConnected: engineState.transportState === 'Synchronized',
            documentJson,
            documentRevision: awaitingRemote ? null : engineState.documentRevision,
        };
    }

    private setState(patch: Partial<YjsCollaborationState>): void {
        this._state = { ...this._state, ...patch };
        this.callbacks.onStateChange?.(this._state);
    }
}

/**
 * Create an imperative collaboration session over a room-backed document
 * handle. The caller owns the lifecycle: call `destroy()` on the controller,
 * then `destroy()` on the handle.
 *
 * React hosts should use {@link useYjsCollaboration} instead.
 */
export function createYjsCollaborationController(
    options: YjsCollaborationOptions
): YjsCollaborationController {
    return new YjsCollaborationControllerImpl(options, {
        onStateChange: options.onStateChange,
        onPeersChange: options.onPeersChange,
        onError: options.onError,
    });
}

/**
 * Connect a room-backed document handle to a Yjs sync and awareness server.
 * The editor and this hook must share the same handle; the hook never creates
 * or destroys it.
 *
 * Native owns the socket and Rust owns sync, awareness, and retries, so there
 * is no Yjs document in JavaScript to manage.
 *
 * @example
 * ```tsx
 * const collaboration = useYjsCollaboration({
 *     documentId,
 *     handle: documentHandle,
 *     transport: { url: `wss://example.com/collab?documentId=${documentId}`, connect: true },
 *     localAwareness: { userId: 'user-1', name: 'Ada', color: '#0A84FF' },
 * });
 *
 * return <NativeRichTextEditor {...collaboration.editorBindings} />;
 * ```
 */
export function useYjsCollaboration(options: YjsCollaborationOptions): UseYjsCollaborationResult {
    _assertNativeEditorDocumentHandle(options.handle);
    const callbacksRef = useRef<MutableCallbacks>({});
    callbacksRef.current = {
        onPeersChange: options.onPeersChange,
        onStateChange: options.onStateChange,
        onError: options.onError,
    };
    const controllerRef = useRef<YjsCollaborationControllerImpl | null>(null);
    const localAwarenessKey = JSON.stringify(options.localAwareness ?? null);
    const transportUrl = options.transport?.url ?? null;
    const transportConnect = options.transport?.connect ?? false;
    const transportProtocolAdapterRef = useRef<NativeCollaborationProtocolAdapter | null>(null);
    transportProtocolAdapterRef.current = options.transport?.protocolAdapter ?? null;
    const transportProtocolAdapterMetadata = JSON.stringify(
        options.transport?.protocolAdapter == null
            ? null
            : {
                protocols: options.transport.protocolAdapter.protocols,
                timeoutMillis: options.transport.protocolAdapter.timeoutMillis,
                terminalCloseCodes: options.transport.protocolAdapter.terminalCloseCodes,
            }
    );
    const stableTransportProtocolAdapter = useMemo<NativeCollaborationProtocolAdapter | null>(() => {
        const current = transportProtocolAdapterRef.current;
        if (current === null) return null;
        return {
            protocols: [...current.protocols],
            ...(current.timeoutMillis === undefined
                ? {}
                : { timeoutMillis: current.timeoutMillis }),
            ...(current.terminalCloseCodes === undefined
                ? {}
                : { terminalCloseCodes: [...current.terminalCloseCodes] }),
            onOpen: (context) =>
                transportProtocolAdapterRef.current?.onOpen(context) ?? {
                    action: 'reject',
                },
            onMessage: (context, frame) =>
                transportProtocolAdapterRef.current?.onMessage(context, frame) ?? {
                    action: 'reject',
                },
        };
        // Static metadata changes require a new native socket descriptor.
        // Callback identity alone is read through the ref and never reconnects.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [transportProtocolAdapterMetadata]);
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
                    transport:
                        options.transport == null
                            ? null
                            : {
                                url: options.transport.url,
                                connect: options.transport.connect,
                                ...(stableTransportProtocolAdapter === null
                                    ? {}
                                    : { protocolAdapter: stableTransportProtocolAdapter }),
                            },
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
                    onError: (error) => callbacksRef.current.onError?.(error),
                }
            );
            controllerRef.current = controller;
            setState({ ...controller.state });
            setPeers([...controller.peers]);
        } catch (error) {
            const nextError = asError(error, 'Native collaboration initialization failed');
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
        // The controller owns one URL/handle binding. Connection intent is
        // updated independently below without recreating the observer.
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [options.documentId, options.handle, stableTransportProtocolAdapter, transportUrl]);

    useEffect(() => {
        controllerRef.current?.applyLocalAwarenessOption(options.localAwareness);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [localAwarenessKey]);

    useEffect(() => {
        if (transportUrl === null) return;
        if (transportConnect) {
            controllerRef.current?.connect();
        } else {
            controllerRef.current?.disconnect();
        }
    }, [transportConnect, stableTransportProtocolAdapter, transportUrl]);

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
            onFocus: () => controllerRef.current?.handleFocusChange(true),
            onBlur: () => controllerRef.current?.handleFocusChange(false),
        },
    };
}
