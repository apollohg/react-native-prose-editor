import {
    NativeEditorErrorBase,
    nativeEditorV2ErrorToException,
    normalizeNativeEditorV2Error,
} from './NativeEditorBoundaryError';
import {
    type NativeEditorTransportState,
    type NativeEditorState,
    invalidV2RequestError,
    isPlainRecord,
    whitelisted,
    V2_TRANSPORT_STATES,
    normalizeNativeEditorV2DecimalId,
    optionalBoolean,
    nativeEditorV2U32,
    normalizeNativeEditorV2StateValue,
    normalizeNativeEditorV2PeersValue,
} from './NativeEditorResultNormalization';
import { utf8V2JsonByteLength } from './NativeEditorCreateJson';

/** One peer's awareness record, as Rust currently holds it. */
export interface NativeEditorPeerInfo {
    /** Yjs client identity, as a decimal string. */
    clientId: string;
    /** Awareness clock, advancing with each of this peer's updates. */
    clock: number;
    /** Whether this record describes this device. */
    isLocal: boolean;
    /** The peer's published application state, e.g. `{ user, focused }`. */
    state: Record<string, unknown> | null;
    /** The peer's caret in engine document positions, or null when it published none. */
    cursor: { anchor: number; head: number } | null;
}

/** One WebSocket frame exchanged during the protocol adapter's prelude. */
export type NativeCollaborationProtocolFrame =
    | { type: 'text'; data: string }
    | { type: 'binary'; data: Uint8Array };

/**
 * What the transport does next after an adapter callback:
 *
 * - `continue` — stay in the prelude and await the next frame.
 * - `ready` — the prelude succeeded; release Yjs traffic.
 * - `reject` — abandon this attempt.
 */
export type NativeCollaborationProtocolAdapterAction = 'continue' | 'ready' | 'reject';

/** An adapter callback's decision, plus any frames to send before it takes effect. */
export interface NativeCollaborationProtocolAdapterResult {
    action: NativeCollaborationProtocolAdapterAction;
    /** Frames sent on the socket before the action applies. */
    frames?: readonly NativeCollaborationProtocolFrame[];
}

/** Identifies the connection attempt an adapter callback is running for. */
export interface NativeCollaborationProtocolAdapterContext {
    /** Identity of this physical connection attempt. */
    attemptId: string;
    /** Transport generation, as a decimal string. Advances on each reconfiguration. */
    generation: string;
    /** Subprotocol the server selected, if any. */
    negotiatedProtocol: string | null;
}

/**
 * An RN-owned, attempt-scoped prelude for a native-owned WebSocket.
 *
 * Native blocks all Yjs traffic until `onMessage` returns `ready`. Callbacks
 * run once per native event and can read fresh credentials on every physical
 * reconnect. Their frames and return values are never persisted by native.
 */
export interface NativeCollaborationProtocolAdapter {
    /** WebSocket subprotocols offered during the physical handshake, in preference order. */
    protocols: readonly string[];
    /** Maximum time native permits the adapter to keep a physical socket pending. */
    timeoutMillis?: number;
    /** Close codes that park the Rust transport instead of entering automatic retry. */
    terminalCloseCodes?: readonly number[];
    /** Runs once when the socket opens — send credentials here. */
    onOpen(
        context: NativeCollaborationProtocolAdapterContext
    ): NativeCollaborationProtocolAdapterResult | Promise<NativeCollaborationProtocolAdapterResult>;
    /** Runs for each frame received before the prelude returns `ready`. */
    onMessage(
        context: NativeCollaborationProtocolAdapterContext,
        frame: NativeCollaborationProtocolFrame
    ): NativeCollaborationProtocolAdapterResult | Promise<NativeCollaborationProtocolAdapterResult>;
}

/** Where the native transport connects, and whether it should. */
export interface NativeCollaborationTransportConfig {
    /** WebSocket endpoint, `ws:` or `wss:`. Treat it as sensitive — it may carry credentials. */
    url: string;
    /** Whether to open the connection now. False configures the endpoint without connecting. */
    connect: boolean;
    /** Optional RN-owned prelude that gates the native Yjs transport. */
    protocolAdapter?: NativeCollaborationProtocolAdapter;
}

/** Why the transport woke and what it did — diagnostic only; no contract depends on these. */
export interface NativeCollaborationTransportDiagnostics {
    /** What woke the transport, e.g. a received message or an elapsed timer. */
    wakeReason: string;
    transportState: NativeEditorTransportState;
    /** Decimal-string deadline for the next scheduled wake, if one is pending. */
    nextDeadlineMillis: string | null;
    /** Whether this wake applied a remote commit. */
    remoteCommitApplied: boolean;
    /** Whether the peer list changed. */
    peersChanged: boolean;
    /** Whether the local awareness entry was republished. */
    renewedLocal: boolean;
    /** Peers dropped for having gone stale. */
    expiredPeerCount: number;
}

/** The transport advanced: new engine state, peers, and diagnostics. */
export interface NativeCollaborationTransportStateEvent {
    editorId: string;
    /** Monotonic decimal-string sequence across all events for this handle. */
    eventSequence: string;
    generation: string | null;
    kind: 'state';
    state: NativeEditorState;
    peers: NativeEditorPeerInfo[];
    diagnostics: NativeCollaborationTransportDiagnostics;
}

/** The transport failed. Retryable failures are followed by further state events. */
export interface NativeCollaborationTransportErrorEvent {
    editorId: string;
    eventSequence: string;
    generation: string | null;
    kind: 'error';
    error: NativeEditorErrorBase;
}

/** Native is asking the configured protocol adapter to handle a prelude step. */
export interface NativeCollaborationProtocolAdapterEvent {
    editorId: string;
    eventSequence: string;
    generation: string;
    kind: 'protocolAdapter';
    /** Identity of the physical connection attempt. */
    attemptId: string;
    /** Identity of this callback invocation; the reply must quote it. */
    eventId: string;
    negotiatedProtocol: string | null;
    /** Which adapter callback this event corresponds to. */
    phase: 'open' | 'message';
    /** The received frame, present when `phase` is `'message'`. */
    frame?: NativeCollaborationProtocolFrame;
}

/** Any event emitted to a `NativeEditorDocumentHandle` transport listener. */
export type NativeCollaborationTransportEvent =
    | NativeCollaborationTransportStateEvent
    | NativeCollaborationTransportErrorEvent
    | NativeCollaborationProtocolAdapterEvent;

export interface NativeCollaborationProtocolAdapterDescriptor {
    protocols: string[];
    timeoutMillis?: number;
    terminalCloseCodes?: number[];
}

export interface NativeCollaborationTransportWireConfig {
    url: string;
    connect: boolean;
    protocolAdapter?: NativeCollaborationProtocolAdapterDescriptor;
}

export const COLLABORATION_PROTOCOL_TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;

export const MAX_COLLABORATION_PROTOCOLS = 16;

export const MAX_COLLABORATION_PROTOCOL_BYTES = 128;

export const MAX_COLLABORATION_ADAPTER_TIMEOUT_MILLIS = 60_000;

export const MAX_COLLABORATION_ADAPTER_FRAMES = 16;

export const MAX_COLLABORATION_ADAPTER_FRAME_BYTES = 64 * 1024;

export function collaborationProtocolAdapterDescriptor(
    value: NativeCollaborationProtocolAdapter
): NativeCollaborationProtocolAdapterDescriptor {
    if (
        value === null ||
        typeof value !== 'object' ||
        !Array.isArray(value.protocols) ||
        value.protocols.length < 1 ||
        value.protocols.length > MAX_COLLABORATION_PROTOCOLS ||
        typeof value.onOpen !== 'function' ||
        typeof value.onMessage !== 'function'
    ) {
        throw invalidV2RequestError('NativeEditorBridge: invalid collaboration protocol adapter');
    }
    const protocols = value.protocols.map((protocol) => {
        if (
            typeof protocol !== 'string' ||
            protocol.length === 0 ||
            utf8V2JsonByteLength(protocol) > MAX_COLLABORATION_PROTOCOL_BYTES ||
            !COLLABORATION_PROTOCOL_TOKEN.test(protocol)
        ) {
            throw invalidV2RequestError(
                'NativeEditorBridge: invalid collaboration WebSocket subprotocol'
            );
        }
        return protocol;
    });
    if (new Set(protocols).size !== protocols.length) {
        throw invalidV2RequestError(
            'NativeEditorBridge: duplicate collaboration WebSocket subprotocol'
        );
    }
    const timeoutMillis = value.timeoutMillis;
    if (
        timeoutMillis !== undefined &&
        (!Number.isSafeInteger(timeoutMillis) ||
            timeoutMillis < 1 ||
            timeoutMillis > MAX_COLLABORATION_ADAPTER_TIMEOUT_MILLIS)
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: invalid collaboration protocol adapter timeout'
        );
    }
    const rawTerminalCloseCodes = value.terminalCloseCodes;
    if (
        rawTerminalCloseCodes !== undefined &&
        (!Array.isArray(rawTerminalCloseCodes) ||
            rawTerminalCloseCodes.some(
                (code) => !Number.isSafeInteger(code) || code < 1_000 || code > 4_999
            ))
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: invalid collaboration terminal close code'
        );
    }
    const terminalCloseCodes =
        rawTerminalCloseCodes === undefined ? undefined : [...rawTerminalCloseCodes];
    if (
        terminalCloseCodes !== undefined &&
        new Set(terminalCloseCodes).size !== terminalCloseCodes.length
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: duplicate collaboration terminal close code'
        );
    }
    return {
        protocols,
        ...(timeoutMillis === undefined ? {} : { timeoutMillis }),
        ...(terminalCloseCodes === undefined ? {} : { terminalCloseCodes }),
    };
}

export function collaborationTransportWireConfig(
    config: NativeCollaborationTransportConfig
): NativeCollaborationTransportWireConfig {
    if (
        config === null ||
        typeof config !== 'object' ||
        typeof config.url !== 'string' ||
        typeof config.connect !== 'boolean'
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: invalid collaboration transport configuration'
        );
    }
    return {
        url: config.url,
        connect: config.connect,
        ...(config.protocolAdapter === undefined
            ? {}
            : {
                  protocolAdapter: collaborationProtocolAdapterDescriptor(config.protocolAdapter),
              }),
    };
}

export function serializeCollaborationProtocolAdapterResult(
    value: NativeCollaborationProtocolAdapterResult
): string {
    if (
        value === null ||
        typeof value !== 'object' ||
        (value.action !== 'continue' && value.action !== 'ready' && value.action !== 'reject') ||
        (value.frames !== undefined &&
            (!Array.isArray(value.frames) ||
                value.frames.length > MAX_COLLABORATION_ADAPTER_FRAMES))
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: invalid collaboration protocol adapter result'
        );
    }
    const frames = (value.frames ?? []).map((frame) => {
        if (frame === null || typeof frame !== 'object') {
            throw invalidV2RequestError(
                'NativeEditorBridge: invalid collaboration protocol adapter frame'
            );
        }
        if (frame.type === 'text' && typeof frame.data === 'string') {
            if (utf8V2JsonByteLength(frame.data) > MAX_COLLABORATION_ADAPTER_FRAME_BYTES) {
                throw invalidV2RequestError(
                    'NativeEditorBridge: collaboration protocol adapter frame is too large'
                );
            }
            return { type: 'text' as const, data: frame.data };
        }
        if (
            frame.type === 'binary' &&
            frame.data instanceof Uint8Array &&
            frame.data.byteLength <= MAX_COLLABORATION_ADAPTER_FRAME_BYTES
        ) {
            return {
                type: 'binary' as const,
                data: encodeNativeCollaborationProtocolBytes(frame.data),
            };
        }
        throw invalidV2RequestError(
            'NativeEditorBridge: invalid collaboration protocol adapter frame'
        );
    });
    return JSON.stringify({
        action: value.action,
        ...(frames.length === 0 ? {} : { frames }),
    });
}

export function normalizeNativeCollaborationTransportDiagnostics(
    value: unknown
): NativeCollaborationTransportDiagnostics | null {
    if (!isPlainRecord(value)) return null;
    const transportState = whitelisted(value.transportState, V2_TRANSPORT_STATES);
    const nextDeadlineMillis =
        value.nextDeadlineMillis === null
            ? null
            : normalizeNativeEditorV2DecimalId(value.nextDeadlineMillis);
    const remoteCommitApplied = optionalBoolean(value.remoteCommitApplied);
    const peersChanged = optionalBoolean(value.peersChanged);
    const renewedLocal = optionalBoolean(value.renewedLocal);
    const expiredPeerCount = nativeEditorV2U32(value.expiredPeerCount);
    if (
        typeof value.wakeReason !== 'string' ||
        transportState == null ||
        (value.nextDeadlineMillis !== null && nextDeadlineMillis == null) ||
        remoteCommitApplied == null ||
        peersChanged == null ||
        renewedLocal == null ||
        expiredPeerCount == null
    ) {
        return null;
    }
    return {
        wakeReason: value.wakeReason,
        transportState,
        nextDeadlineMillis,
        remoteCommitApplied,
        peersChanged,
        renewedLocal,
        expiredPeerCount,
    };
}

export const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

export function encodeNativeCollaborationProtocolBytes(bytes: Uint8Array): string {
    let encoded = '';
    for (let index = 0; index < bytes.length; index += 3) {
        const first = bytes[index];
        const second = index + 1 < bytes.length ? bytes[index + 1] : 0;
        const third = index + 2 < bytes.length ? bytes[index + 2] : 0;
        const value = (first << 16) | (second << 8) | third;
        encoded += BASE64_ALPHABET[(value >>> 18) & 63];
        encoded += BASE64_ALPHABET[(value >>> 12) & 63];
        encoded += index + 1 < bytes.length ? BASE64_ALPHABET[(value >>> 6) & 63] : '=';
        encoded += index + 2 < bytes.length ? BASE64_ALPHABET[value & 63] : '=';
    }
    return encoded;
}

export function decodeNativeCollaborationProtocolBytes(value: string): Uint8Array | null {
    if (value.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(value)) return null;
    const firstPadding = value.indexOf('=');
    if (firstPadding >= 0 && firstPadding < value.length - 2) return null;
    const outputLength =
        (value.length / 4) * 3 - (value.endsWith('==') ? 2 : value.endsWith('=') ? 1 : 0);
    const bytes = new Uint8Array(outputLength);
    let outputIndex = 0;
    for (let index = 0; index < value.length; index += 4) {
        const first = BASE64_ALPHABET.indexOf(value[index]);
        const second = BASE64_ALPHABET.indexOf(value[index + 1]);
        const third = value[index + 2] === '=' ? 0 : BASE64_ALPHABET.indexOf(value[index + 2]);
        const fourth = value[index + 3] === '=' ? 0 : BASE64_ALPHABET.indexOf(value[index + 3]);
        if (first < 0 || second < 0 || third < 0 || fourth < 0) return null;
        const decoded = (first << 18) | (second << 12) | (third << 6) | fourth;
        if (outputIndex < outputLength) bytes[outputIndex++] = (decoded >>> 16) & 0xff;
        if (outputIndex < outputLength) bytes[outputIndex++] = (decoded >>> 8) & 0xff;
        if (outputIndex < outputLength) bytes[outputIndex++] = decoded & 0xff;
    }
    return bytes;
}

export function normalizeNativeCollaborationTransportEvent(
    value: unknown
): NativeCollaborationTransportEvent | null {
    if (!isPlainRecord(value)) return null;
    const editorId = normalizeNativeEditorV2DecimalId(value.editorId);
    const eventSequence = normalizeNativeEditorV2DecimalId(value.eventSequence);
    const generation =
        value.generation === null ? null : normalizeNativeEditorV2DecimalId(value.generation);
    if (
        editorId == null ||
        editorId === '0' ||
        eventSequence == null ||
        eventSequence === '0' ||
        (value.generation !== null && generation == null)
    ) {
        return null;
    }
    if (value.kind === 'state') {
        const state = normalizeNativeEditorV2StateValue(value.state);
        const peers = normalizeNativeEditorV2PeersValue({ peers: value.peers });
        const diagnostics = normalizeNativeCollaborationTransportDiagnostics(value.diagnostics);
        if (state == null || peers == null || diagnostics == null) return null;
        return {
            editorId,
            eventSequence,
            generation,
            kind: 'state',
            state,
            peers,
            diagnostics,
        };
    }
    if (value.kind === 'error') {
        const error = normalizeNativeEditorV2Error({ error: value.error });
        if (error == null) return null;
        return {
            editorId,
            eventSequence,
            generation,
            kind: 'error',
            error: nativeEditorV2ErrorToException(error),
        };
    }
    if (value.kind === 'protocolAdapter') {
        const eventId = normalizeNativeEditorV2DecimalId(value.eventId);
        if (
            generation == null ||
            typeof value.attemptId !== 'string' ||
            value.attemptId.length === 0 ||
            eventId == null ||
            eventId === '0' ||
            (value.negotiatedProtocol !== null && typeof value.negotiatedProtocol !== 'string') ||
            (value.phase !== 'open' && value.phase !== 'message')
        ) {
            return null;
        }
        const base = {
            editorId,
            eventSequence,
            generation,
            kind: 'protocolAdapter' as const,
            attemptId: value.attemptId,
            eventId,
            negotiatedProtocol: value.negotiatedProtocol as string | null,
        };
        if (value.phase === 'open') {
            if (value.frame !== undefined) return null;
            return { ...base, phase: 'open' };
        }
        if (!isPlainRecord(value.frame) || typeof value.frame.data !== 'string') return null;
        if (value.frame.type === 'text') {
            return {
                ...base,
                phase: 'message',
                frame: { type: 'text', data: value.frame.data },
            };
        }
        if (value.frame.type === 'binary') {
            const data = decodeNativeCollaborationProtocolBytes(value.frame.data);
            if (data == null) return null;
            return {
                ...base,
                phase: 'message',
                frame: { type: 'binary', data },
            };
        }
    }
    return null;
}
