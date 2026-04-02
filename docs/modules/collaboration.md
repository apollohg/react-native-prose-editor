[Back to docs index](../README.md)

# Collaboration

## Overview

`useYjsCollaboration()` is the intended integration path for realtime collaboration.

The collaboration controller owns the live shared document state, awareness state, and transport lifecycle. The editor should be bound to that controller, not to a separate app-managed JSON document state.

## Correct Wiring

Use the `editorBindings` returned by `useYjsCollaboration()` directly:

```tsx
import {
  NativeRichTextEditor,
  useYjsCollaboration,
} from '@apollohg/react-native-prose-editor';

export function CollaborativeEditor() {
  const collaboration = useYjsCollaboration({
    documentId: 'case-123',
    createWebSocket: () => new WebSocket('wss://example.com/yjs/case-123'),
    localAwareness: {
      userId: 'u1',
      name: 'Jayden',
      color: '#0A84FF',
    },
  });

  return (
    <NativeRichTextEditor
      valueJSON={collaboration.editorBindings.valueJSON}
      onContentChangeJSON={collaboration.editorBindings.onContentChangeJSON}
      onSelectionChange={collaboration.editorBindings.onSelectionChange}
      onFocus={collaboration.editorBindings.onFocus}
      onBlur={collaboration.editorBindings.onBlur}
      remoteSelections={collaboration.editorBindings.remoteSelections}
    />
  );
}
```

## `YjsCollaborationOptions`

```ts
interface YjsCollaborationOptions {
  documentId: string;
  createWebSocket: () => WebSocket;
  connect?: boolean;
  retryIntervalMs?: YjsRetryInterval | false;
  fragmentName?: string;
  initialDocumentJson?: DocumentJSON;
  initialEncodedState?: EncodedCollaborationStateInput;
  localAwareness: LocalAwarenessUser;
  onPeersChange?: (peers: CollaborationPeer[]) => void;
  onStateChange?: (state: YjsCollaborationState) => void;
  onError?: (error: Error) => void;
}
```

| Option | Type | Default | Description |
| --- | --- | --- | --- |
| `documentId` | `string` | — | Document identifier used to scope the collaboration session. |
| `createWebSocket` | `() => WebSocket` | — | Factory that returns a new WebSocket connection to the Yjs sync server. Called on initial connect and on every reconnect. |
| `connect` | `boolean` | `true` | Whether to connect automatically when the controller is created. Set to `false` to defer connection until `connect()` is called. |
| `retryIntervalMs` | `YjsRetryInterval \| false` | exponential backoff | Retry interval configuration. Pass `false` to disable automatic retry entirely. See [Reconnect Behavior](#reconnect-behavior). |
| `fragmentName` | `string` | `'default'` | Name of the Yjs XML fragment within the shared document. Only change this if your server uses a non-default fragment name. |
| `initialDocumentJson` | `DocumentJSON` | — | Local fallback document used when there is no encoded state yet. Not a durable collaboration restore format. See [Initial State Rules](#initial-state-rules). |
| `initialEncodedState` | `EncodedCollaborationStateInput` | — | Previously persisted CRDT state to restore from. Accepts `Uint8Array`, `readonly number[]`, or a base64 `string`. |
| `localAwareness` | `LocalAwarenessUser` | — | Local user identity and appearance. See [LocalAwarenessUser](#localawarenessuser). |
| `onPeersChange` | `(peers: CollaborationPeer[]) => void` | — | Called when the set of connected peers changes. |
| `onStateChange` | `(state: YjsCollaborationState) => void` | — | Called when the collaboration state changes (connection status, document content, errors). |
| `onError` | `(error: Error) => void` | — | Called when a collaboration error occurs (transport failures, sync errors). |

## `LocalAwarenessUser`

```ts
interface LocalAwarenessUser {
  userId: string;
  name: string;
  color: string;
  avatarUrl?: string;
  extra?: Record<string, unknown>;
}
```

| Field | Type | Description |
| --- | --- | --- |
| `userId` | `string` | Unique identifier for the local user. |
| `name` | `string` | Display name shown alongside the remote caret on other clients. |
| `color` | `string` | Color used for this user's remote selection highlight on other clients. |
| `avatarUrl` | `string \| undefined` | URL of the user's avatar image, broadcast to other peers via awareness. |
| `extra` | `Record<string, unknown> \| undefined` | Arbitrary metadata broadcast to other peers via awareness. Use for roles, status, or any app-specific data. |

## `LocalAwarenessState`

The full awareness state broadcast to other peers, combining user identity with editor state.

```ts
interface LocalAwarenessState {
  user: LocalAwarenessUser;
  selection?: {
    anchor: number;
    head: number;
  };
  focused?: boolean;
}
```

| Field | Type | Description |
| --- | --- | --- |
| `user` | `LocalAwarenessUser` | User identity and appearance. |
| `selection` | `{ anchor: number; head: number } \| undefined` | Current editor selection range. |
| `focused` | `boolean \| undefined` | Whether the user's editor is currently focused. |

## `YjsCollaborationState`

```ts
type YjsTransportStatus = 'idle' | 'connecting' | 'connected' | 'disconnected' | 'error';

interface YjsCollaborationState {
  documentId: string;
  status: YjsTransportStatus;
  isConnected: boolean;
  documentJson: DocumentJSON;
  lastError?: Error;
}
```

| Field | Type | Description |
| --- | --- | --- |
| `documentId` | `string` | The document identifier for this session. |
| `status` | `YjsTransportStatus` | Current transport status. |
| `isConnected` | `boolean` | Whether the transport is currently connected. |
| `documentJson` | `DocumentJSON` | Current document content as JSON. |
| `lastError` | `Error \| undefined` | Most recent error, if any. |

## `CollaborationPeer`

```ts
interface CollaborationPeer {
  clientId: number;
  isLocal: boolean;
  state: Record<string, unknown> | null;
}
```

| Field | Type | Description |
| --- | --- | --- |
| `clientId` | `number` | Unique Yjs client identifier for this peer. |
| `isLocal` | `boolean` | Whether this peer is the local user. |
| `state` | `Record<string, unknown> \| null` | Raw awareness state broadcast by this peer, or `null` if not yet received. |

## `EncodedCollaborationStateInput`

```ts
type EncodedCollaborationStateInput = Uint8Array | readonly number[] | string;
```

Accepted input formats for encoded CRDT state. When a `string` is provided it is treated as base64-encoded.

## `useYjsCollaboration()` Hook

```ts
function useYjsCollaboration(options: YjsCollaborationOptions): UseYjsCollaborationResult;
```

### `UseYjsCollaborationResult`

```ts
interface UseYjsCollaborationResult {
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
```

| Field | Type | Description |
| --- | --- | --- |
| `state` | `YjsCollaborationState` | Current collaboration state including connection status and document content. |
| `peers` | `CollaborationPeer[]` | All currently connected peers (including the local user). |
| `isConnected` | `boolean` | Shorthand for `state.isConnected`. |
| `connect()` | `() => void` | Open the WebSocket connection. Only needed if `connect: false` was passed. |
| `disconnect()` | `() => void` | Close the WebSocket connection and stop retrying. |
| `reconnect()` | `() => void` | Disconnect and immediately reconnect. |
| `getEncodedState()` | `() => Uint8Array` | Get the current encoded CRDT state as bytes. |
| `getEncodedStateBase64()` | `() => string` | Get the current encoded CRDT state as a base64 string. |
| `applyEncodedState(...)` | `(state) => void` | Merge an encoded CRDT state into the current document. |
| `replaceEncodedState(...)` | `(state) => void` | Replace the entire CRDT state. Use with caution — this overwrites the local document. |
| `updateLocalAwareness(...)` | `(partial) => void` | Update the local awareness state (selection, focus, user info) broadcast to other peers. |
| `editorBindings` | object | Props to spread onto `NativeRichTextEditor`. See [Correct Wiring](#correct-wiring). |

The hook does not expose a `destroy()` method. Lifecycle is managed automatically by the hook's `useEffect` cleanup when the component unmounts. Use `createYjsCollaborationController()` if you need manual lifecycle control.

## `createYjsCollaborationController()`

The imperative (non-hook) API for environments where React hooks are not available or when you need manual lifecycle control.

```ts
function createYjsCollaborationController(
  options: YjsCollaborationOptions
): YjsCollaborationController;
```

### `YjsCollaborationController`

```ts
interface YjsCollaborationController {
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
```

| Method / Property | Type | Description |
| --- | --- | --- |
| `state` | `YjsCollaborationState` | Current collaboration state (read-only). |
| `peers` | `CollaborationPeer[]` | Currently connected peers (read-only). |
| `connect()` | `() => void` | Open the WebSocket connection. |
| `disconnect()` | `() => void` | Close the WebSocket connection and stop retrying. |
| `reconnect()` | `() => void` | Disconnect and immediately reconnect. |
| `destroy()` | `() => void` | Disconnect and release all resources. The controller cannot be reused after this. |
| `getEncodedState()` | `() => Uint8Array` | Get the current encoded CRDT state as bytes. |
| `getEncodedStateBase64()` | `() => string` | Get the current encoded CRDT state as a base64 string. |
| `applyEncodedState(...)` | `(state) => void` | Merge an encoded CRDT state into the current document. |
| `replaceEncodedState(...)` | `(state) => void` | Replace the entire CRDT state. |
| `updateLocalAwareness(...)` | `(partial) => void` | Update the local awareness state broadcast to other peers. |
| `handleLocalDocumentChange(doc)` | `(doc) => void` | Feed a local document change into the collaboration session. Use this to wire the controller to `onContentChangeJSON`. |
| `handleSelectionChange(selection)` | `(selection) => void` | Feed a local selection change into the collaboration session. Use this to wire the controller to `onSelectionChange`. |
| `handleFocusChange(focused)` | `(focused) => void` | Feed a local focus change into the collaboration session. Use this to wire the controller to `onFocus`/`onBlur`. |

## Utility Functions

```ts
function encodeCollaborationStateBase64(encodedState: EncodedCollaborationStateInput): string;
function decodeCollaborationStateBase64(base64: string): Uint8Array;
```

| Function | Description |
| --- | --- |
| `encodeCollaborationStateBase64(state)` | Convert an encoded CRDT state (bytes or number array) to a base64 string for storage or transport. |
| `decodeCollaborationStateBase64(base64)` | Decode a base64 string back to a `Uint8Array` for use with `applyEncodedState` or `replaceEncodedState`. |

## `YjsRetryInterval`

```ts
interface YjsRetryContext {
  attempt: number;
  documentId: string;
  lastError?: Error;
}

type YjsRetryInterval = number | ((context: YjsRetryContext) => number | null | false);
```

When `retryIntervalMs` is a `number`, that fixed interval is used between every retry. When it is a function, it receives the retry context and should return the delay in milliseconds, or `null`/`false` to stop retrying.

## Reconnect Behavior

The collaboration transport retries automatically by default with exponential backoff after unexpected disconnects.

- attempt 1: `500ms`
- attempt 2: `1000ms`
- attempt 3: `2000ms`
- then doubling up to a `30000ms` cap

You can override that with `retryIntervalMs`:

```tsx
const collaboration = useYjsCollaboration({
  documentId: 'case-123',
  createWebSocket: () => new WebSocket('wss://example.com/yjs/case-123'),
  retryIntervalMs: ({ attempt, lastError }) => {
    if (lastError?.message.includes('auth')) {
      return false;
    }
    return Math.min(1000 * 2 ** (attempt - 1), 15000);
  },
  localAwareness: {
    userId: 'u1',
    name: 'Jayden',
    color: '#0A84FF',
  },
});
```

Use `retryIntervalMs: false` to disable automatic retry entirely.

## Source Of Truth

In collaboration mode:

- the collaboration session is the document source of truth
- `valueJSON` should come from `useYjsCollaboration().editorBindings.valueJSON`
- `onContentChangeJSON` should go back to `useYjsCollaboration().editorBindings.onContentChangeJSON`

Do not keep a second app-owned JSON document state and feed that back into `valueJSON` on every render. That creates competing sources of truth and can cause selection churn, stale restores, or remote updates being replayed incorrectly.

## What To Persist

For durable offline recovery or delayed sync, persist the encoded CRDT state, not just the visible JSON document.

Available controller methods:

- `getEncodedState()`
- `getEncodedStateBase64()`
- `applyEncodedState(...)`
- `replaceEncodedState(...)`

Use encoded state when you need to restore the actual Yjs/CRDT state later. JSON is only a content snapshot.

For storage and transport, the `encodeCollaborationStateBase64` and `decodeCollaborationStateBase64` utility functions convert between binary state and base64 strings.

## Initial State Rules

Prefer these inputs in this order:

1. `initialEncodedState` when you have previously persisted collaboration state
2. backend sync over WebSocket when the room loads
3. `initialDocumentJson` only as a local fallback when there is no encoded state yet

`initialDocumentJson` is not a durable collaboration restore format.

## Awareness And Remote Cursors

Remote cursors should be passed through `remoteSelections` from `editorBindings`. Do not map remote awareness peers onto the local editor selection yourself.

The package renders remote selections as native overlays. They are not meant to become the local user selection or move the active caret.

## Web Compatibility Notes

The collaboration transport is intended for standard Yjs sync + awareness peers such as:

- web clients using `y-websocket`-style providers
- backends that speak the standard Yjs sync and awareness protocol

The package adapts between:

- native editor numeric document selections
- standard Yjs awareness cursor payloads

## Common Mistake

This is the incorrect pattern:

```tsx
const [doc, setDoc] = useState(...)
const collaboration = useYjsCollaboration(...)

<NativeRichTextEditor
  valueJSON={doc}
  onContentChangeJSON={setDoc}
  remoteSelections={collaboration.editorBindings.remoteSelections}
/>
```

That makes your app state and the collaboration session compete.

Use the collaboration bindings as the editor bindings instead.

## Related Docs

- [NativeRichTextEditor Reference](../reference/native-rich-text-editor.md)
- [Editor State Reference](../reference/editor-state.md)
