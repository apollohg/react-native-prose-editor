import { useEffect, useRef, useState } from 'react';

import {
    _assertNativeEditorDocumentHandle,
    createNativeEditorLocalAwarenessSelection,
    type DocumentJSON,
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

export type YjsTransportStatus =
    | 'idle'
    | 'disconnected'
    | 'connecting'
    | 'handshaking'
    | 'synchronized'
    | 'incompatible'
    | 'destroyed';

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
    documentJson: DocumentJSON | null;
    documentRevision: string | null;
    lastError?: Error;
}

export interface YjsCollaborationOptions {
    documentId: string;
    handle: NativeEditorDocumentHandle;
    transport: NativeCollaborationTransportConfig | null;
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
}

export interface YjsCollaborationEditorBindings {
    documentHandle: NativeEditorDocumentHandle;
    documentRevision: string | null;
    remoteSelections: RemoteSelectionDecoration[];
    onSelectionChange: (selection: Selection) => void;
    onFocus: () => void;
    onBlur: () => void;
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

function normalizeAwarenessSelection(
    selection: Selection
): NativeEditorLocalAwarenessSelection | undefined {
    if (selection.type !== 'text') return undefined;
    if (selection.anchor === undefined || selection.head === undefined) return undefined;
    return createNativeEditorLocalAwarenessSelection(selection.anchor, selection.head);
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
    if ('selection' in partial) {
        if (partial.selection !== undefined) {
            const selection = normalizeAwarenessSelection(partial.selection);
            if (selection !== undefined) next.selection = selection;
        }
    } else if (base.selection !== undefined) {
        next.selection = base.selection;
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
    private lastEventSequence: string | null = null;
    private destroyed = false;
    private _state: YjsCollaborationState;
    private _peers: NativeEditorV2PeerInfo[] = [];

    constructor(options: YjsCollaborationOptions, callbacks: MutableCallbacks = {}) {
        _assertNativeEditorDocumentHandle(options.handle);
        this.handle = options.handle;
        this.documentId = options.documentId;
        this.callbacks = callbacks;
        this.transport =
            options.transport === null
                ? null
                : {
                    url: options.transport.url,
                    connect: options.transport.connect,
                    ...(options.transport.connectionInit === undefined
                        ? {}
                        : {
                            connectionInit: {
                                jwt: options.transport.connectionInit.jwt,
                            },
                        }),
                };
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
            this.desiredAwareness = null;
            this.handle.setLocalAwareness(null);
            return;
        }
        const current = this.desiredAwareness;
        this.publishAwareness({
            state: {
                ...(current?.state ?? {}),
                user: { ...user },
            },
            focused: current?.focused ?? false,
            ...(current?.selection === undefined ? {} : { selection: current.selection }),
        });
    }

    private setConnectionEnabled(connect: boolean): void {
        if (this.destroyed || this.transport === null) return;
        this.transport = { ...this.transport, connect };
        this.handle.configureCollaborationTransport(this.transport);
    }

    private publishAwareness(intent: NativeEditorLocalAwarenessIntent): void {
        this.handle.setLocalAwareness(intent);
        this.desiredAwareness = intent;
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
        if (event.state == null || event.peers == null) return;
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
    const transportConnectionInitJWT = options.transport?.connectionInit?.jwt ?? null;
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
            controller = new YjsCollaborationControllerImpl(options, {
                onStateChange: (nextState) => {
                    setState({ ...nextState });
                    callbacksRef.current.onStateChange?.(nextState);
                },
                onPeersChange: (nextPeers) => {
                    setPeers([...nextPeers]);
                    callbacksRef.current.onPeersChange?.(nextPeers);
                },
                onError: (error) => callbacksRef.current.onError?.(error),
            });
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
    }, [options.documentId, options.handle, transportConnectionInitJWT, transportUrl]);

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
    }, [transportConnect, transportConnectionInitJWT, transportUrl]);

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
        },
    };
}
