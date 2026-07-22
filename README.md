# React Native Prose Editor [![NPM version](https://img.shields.io/npm/v/@apollohg/react-native-prose-editor.svg?style=flat)](https://www.npmjs.com/package/@apollohg/react-native-prose-editor)

`@apollohg/react-native-prose-editor` is a native rich text editor for React Native with a Rust document core, native iOS and Android rendering, configurable schemas, and a React-facing toolbar and theme API.

This project is currently in `alpha` and the API, behavior, and packaging may still change.

<p align="center">
  <img src="https://raw.githubusercontent.com/wiki/apollohg/react-native-prose-editor/images/example-ios.png" alt="Example editor iOS" width="45%" align="top" />
  <img src="https://raw.githubusercontent.com/wiki/apollohg/react-native-prose-editor/images/example-android.png" alt="Example editor Android" width="45%" align="top" />
</p>

This repository contains three main pieces:

- the editor package itself under [`src`](./src), [`ios`](./ios), [`android`](./android), and [`rust`](./rust)
- an Expo SDK 54 development app under [`example`](./example)
- a runnable iOS XCTest harness for native regression coverage

## Features

The editor already supports:

- HTML and ProseMirror JSON content input/output
- configurable schemas
- marks such as bold, italic, underline, strike, and links
- blockquotes
- bullet and ordered lists with indent/outdent behavior
- hard breaks and horizontal rules
- native @-mentions with themed suggestion UI in the toolbar area
- native theming for text, lists, horizontal rules, mentions, and the toolbar
- a shared document-session model (`NativeEditorDocumentHandle`) that powers both local editors and Yjs collaboration
- server-initialized realtime collaboration over standard Yjs sync and awareness, with remote cursors and automatic reconnect
- a Rust-backed, local-only undo/redo history model

## Repository Layout

- [`src`](./src): React Native component API, toolbar, schemas, and TypeScript types
- [`ios`](./ios): iOS native view, toolbar accessory, rendering bridge, and generated Rust bindings
- [`android`](./android): Android native view, rendering bridge, and Expo module wiring
- [`Rust Editor Core`](./rust/editor-core): document model, transforms, schema system, selection, history, serialization, and tests
- [`example`](./example): Expo 54 app for manual QA and development

Project documentation now lives in the [GitHub Wiki](https://github.com/apollohg/react-native-prose-editor/wiki).

## Installation

This package currently requires Expo Modules. Use it in an Expo development build or in a bare React Native app that has Expo Modules configured.

The minimum tested Expo version is SDK 54.

Required peer dependencies:

- `expo`
- `react`
- `react-native`
- `@expo/vector-icons`

Install the package:

```sh
npm install @apollohg/react-native-prose-editor@0.5.1
```

Expo prebuild apps should add the package config plugin so Android excludes
obsolete JNA ABI copies that modern NDKs cannot strip:

```ts
export default {
  expo: {
    plugins: ['@apollohg/react-native-prose-editor'],
  },
};
```

For bare React Native apps or existing generated Android projects, add the same
packaging exclude to `android/gradle.properties` when your template applies
`android.packagingOptions.*` properties:

```properties
android.packagingOptions.excludes=**/armeabi/libjnidispatch.so,**/mips/libjnidispatch.so,**/mips64/libjnidispatch.so
```

If your Android project does not read those Gradle properties, add the patterns
directly under the app module's `android.packagingOptions.jniLibs.excludes`.

For local package development in this repo:

```sh
npm install
npm --prefix example install
npm run example:prebuild
```

For full setup details, including peer dependencies, example app setup, and iOS pods, see the [Installation Guide](https://github.com/apollohg/react-native-prose-editor/wiki/Installation).

## Basic Usage

Every editor is bound to a `NativeEditorDocumentHandle` — a shared document
session created up front. Initial content lives in the handle's creation
config, not on the component:

```tsx
import React, { useEffect, useMemo, useRef } from 'react';
import {
  createNativeEditorDocumentHandle,
  NativeRichTextEditor,
  type NativeRichTextEditorRef,
} from '@apollohg/react-native-prose-editor';

export function EditorScreen() {
  const editorRef = useRef<NativeRichTextEditorRef>(null);

  const documentHandle = useMemo(
    () =>
      createNativeEditorDocumentHandle({
        initialization: { type: 'localHtml', html: '<p>Hello world</p>' },
      }),
    []
  );
  useEffect(() => () => documentHandle.destroy(), [documentHandle]);

  return (
    <NativeRichTextEditor
      ref={editorRef}
      documentHandle={documentHandle}
      placeholder="Start typing..."
      onContentChange={(html) => {
        console.log(html);
      }}
    />
  );
}
```

`NativeEditorV2CreateConfig` accepts `schema`, `fragmentName`, a required
`initialization`, grouped `policy` (`maxLength`, `readOnly`, `inputFilter`,
`allowBase64Images`), and grouped `limits` (`resource`, `editing`,
`collaboration`):

- `{ type: 'localEmpty' }` — an empty local document
- `{ type: 'localJson', json }` — a local document seeded from ProseMirror JSON
- `{ type: 'localHtml', html }` — a local document seeded from HTML
- `{ type: 'room', documentId, lineageId, snapshot? }` — a collaboration room;
  the server seeds the document during the standard Yjs sync handshake, or a
  previously exported room `snapshot` restores it offline

The component drives the retained document API through the handle: controlled
`value` / `valueJSON` (`valueJSONUpdateMode: 'replace' | 'reset'`),
`onContentChange` / `onContentChangeJSON`, and the ref methods `setContent`,
`setContentJson`, `clearContent`, `getContent`, `getContentJson`,
`getTextContent`, `undo` / `redo` / `canUndo` / `canRedo`, `focus` / `blur`,
and `getCaretRect`. Undo history is local-only: remote collaboration commits
never enter the local undo stack.

The component is a genuinely interactive editor: the native view binds to the
handle's session, so typing and IME commit through the native v2 adapters
(one transaction per commit; transient composing text never reaches the
engine), and selection, focus, content-height, and toolbar events flow back
to JS. The typing/formatting ref methods (`toggleMark`, `setLink`,
`unsetLink`, `toggleBlockquote`, `toggleHeading`, `toggleList`,
`indentListItem`, `outdentListItem`, `insertNode`, `insertImage`,
`insertText`, `insertContentHtml`, `insertContentJson`) apply real engine
commands at the engine selection and throw typed v2 errors — including
`MUTATION_REJECTED` when `editable={false}`, and refresh-without-retry on
`REVISION_MISMATCH`. Whole-document replacement (`value`, `valueJSON`,
`setContent`, `clearContent`) is rejected with
`WHOLE_DOCUMENT_REPLACEMENT_CONNECTED` while a collaboration transport is
connected, as enforced by the engine.

Props restored for the interactive contract: `editable` (default true),
`autoFocus`, `autoCapitalize`, `autoCorrect`, `keyboardType`,
`heightBehavior`, `showToolbar` (default true), `toolbarPlacement`,
`toolbarItems`, `onToolbarAction`, `onRequestLink`, `onRequestImage`,
`allowImageResizing`, `imageLoadingPolicy`, `onSelectionChange`,
`onActiveStateChange`, `onFocus`, and `onBlur`. Props that intentionally did
not return: `initialContent` / `initialJSON` (initialization lives in the
handle's creation config), `maxLength` / engine-enforced `allowBase64Images`
/ `readOnly` / `inputFilter` (also creation config), and `autoDetectLinks`,
`preserveSelectionOnValueJSONReset`, `selectionOnValueJSONReset` (removed
with the legacy bridge; no v2 equivalent). The `allowBase64Images` prop that
remains is advisory only — it is surfaced as
`ImageRequestContext.allowBase64` for the host image picker; enforcement
belongs to the handle config.

## Customization

The main extension points today are:

- `documentHandle` (required): the shared document session; initialization, schema, read-only, and input limits are set at handle creation
- `schema`: provide a custom schema definition (also settable on the handle config)
- `editable`, `autoFocus`, `autoCapitalize`, `autoCorrect`, `keyboardType`: interactive behavior and keyboard configuration
- `showToolbar`, `toolbarPlacement`, `toolbarItems`, `onToolbarAction`, `onRequestLink`, `onRequestImage`: formatting toolbar (native keyboard accessory or inline React toolbar)
- `heightBehavior`: internal scrolling or grow-with-content layout
- `allowImageResizing`, `imageLoadingPolicy`: image interaction and loading bounds
- `theme`: style text blocks, blockquotes, lists, horizontal rules, background, and toolbar chrome
- `addons`: configure optional features like @-mentions
- `remoteSelections`: render collaboration peers' selections as native overlays
- `resourceLimits`: bound schema, document, HTML/JSON, and collaboration admission

For setup and customization details, start with the [Documentation Index](https://github.com/apollohg/react-native-prose-editor/wiki).

## Collaboration

Realtime collaboration uses standard Yjs sync and awareness over a WebSocket,
with the Rust engine as the single source of truth. The editor and the
collaboration controller share one `NativeEditorDocumentHandle`; the controller
never creates or destroys sessions. Rooms are **server-initialized**: the
server speaks the y-sync protocol and sends the document during the handshake
(the editor renders nothing until the server document is promoted). There are
no client-side seeding or encoded-state APIs — see the migration table in the
[CHANGELOG](./CHANGELOG.md) for how to move off `initialDocumentJson` /
`initialEncodedState`.

```tsx
import React, { useEffect, useMemo } from 'react';
import {
  createNativeEditorDocumentHandle,
  NativeRichTextEditor,
  useYjsCollaboration,
} from '@apollohg/react-native-prose-editor';

export function CollaborativeEditor({ documentId }: { documentId: string }) {
  const documentHandle = useMemo(
    () =>
      createNativeEditorDocumentHandle({
        initialization: {
          type: 'room',
          documentId,
          lineageId: `my-app|${documentId}`,
        },
      }),
    [documentId]
  );
  useEffect(() => () => documentHandle.destroy(), [documentHandle]);

  const collaboration = useYjsCollaboration({
    documentId,
    handle: documentHandle,
    createWebSocket: () =>
      new WebSocket(`wss://example.com/collaboration?documentId=${documentId}`),
    localAwareness: { userId: 'u-1', name: 'Ada', color: '#0A84FF' },
  });

  return (
    <NativeRichTextEditor
      documentHandle={collaboration.editorBindings.documentHandle}
      documentRevision={collaboration.editorBindings.documentRevision}
      onLocalDocumentCommit={collaboration.editorBindings.onLocalDocumentCommit}
      remoteSelections={collaboration.editorBindings.remoteSelections}
    />
  );
}
```

`useYjsCollaboration` also renders `state` (transport status, rendered
document JSON/revision, last error) and `peers` (`NativeEditorV2PeerInfo[]` —
note `clientId` is a decimal **string**) for presence UI, and accepts
`connect`, `retryIntervalMs`, `onPeersChange`, `onStateChange`, and `onError`
options. `editorBindings.onSelectionChange` / `onFocus` / `onBlur` are the
awareness inputs the interactive editor props will call once they land.

### Offline snapshot restore

A room can be restored without the server from a document-scoped snapshot —
export once, persist the two fields, and hand them back at handle creation:

```ts
const exported = documentHandle.bridge.snapshotExport();
// Persist exported.metadataJson (string) and exported.encodedState (Uint8Array).

const restoredHandle = createNativeEditorDocumentHandle({
  initialization: {
    type: 'room',
    documentId,
    lineageId,
    snapshot: {
      metadata: JSON.parse(exported.metadataJson),
      encodedState: exported.encodedState,
    },
  },
});
```

For more on collaboration wiring and source-of-truth rules, see the [Collaboration Guide](https://github.com/apollohg/react-native-prose-editor/wiki/Collaboration).

Custom schema content expressions support ProseMirror-style sequences,
parentheses, alternation, `?`, `*`, `+`, and bounded/unbounded ranges such as
`{2}`, `{1,3}`, and `{1,}`. Invalid, unresolved, or non-constructible schemas
fall back to the default schema during native editor creation.

Image loading is bounded on both native platforms. Override the defaults on a
prose viewer when needed:

```tsx
<NativeProseViewer
  contentHTML={html}
  imageLoadingPolicy={{
    maxSourceBytes: 5 * 1024 * 1024,
    connectTimeoutMs: 8_000,
    readTimeoutMs: 15_000,
    requestTimeoutMs: 45_000,
    maxConcurrentRequests: 2,
    maxPendingRequests: 32,
    maxDecodeDimensionPx: 1_536,
  }}
/>
```

### Security and resource limits

All document, schema, collaboration, and image inputs are admitted through configurable limits with safe defaults and non-configurable hard ceilings. Resource limits can be set on editors, viewers, and document handles:

```tsx
<NativeRichTextEditor
  documentHandle={documentHandle}
  resourceLimits={{
    maxInputBytes: 20 * 1024 * 1024,
    maxDocumentNodes: 100_000,
    maxDocumentDepth: 256,
    maxSchemaNodes: 1_024,
    maxSchemaExpressionBytes: 64 * 1024,
    maxCollaborationMessageBytes: 10 * 1024 * 1024,
    maxEncodedStateBytes: 50 * 1024 * 1024,
  }}
/>
```

Failures cross the native boundary as structured v2 error envelopes
(`domain`, `code`, `message`, `requestId`, `operationIndex`, `limit`,
`actual`, `details`) and surface as typed `NativeEditorV2*` exceptions:
`NativeEditorV2BoundaryError`, `NativeEditorV2DocumentError`,
`NativeEditorV2OperationError`, `NativeEditorV2LifecycleError`,
`NativeEditorV2SnapshotError`, and `NativeEditorV2TransportError`, all
extending `NativeEditorV2ErrorBase`. Permanently non-retryable failures
(`ENGINE_INVARIANT_FAILED`, destroyed-session races) throw the distinct
`NativeEditorV2NonRetryableError`. Synchronous JS-side validation (for
example `resolveEditorResourceLimits`) throws the legacy
`NativeEditorBoundaryError`, whose stable `code`, `limit`, `actual`, and
`details` fields allow callers to distinguish configuration, size, schema,
document, collaboration, and image-policy failures.

Unknown nodes remain opaque and round-trip through JSON or HTML. Unknown marks and missing required attributes are rejected, and failed mutations do not partially update editor or collaboration state. These preservation rules are a compatibility mechanism, not an HTML trust boundary.

`getContent()` preserves opaque HTML and is a serializer, not a sanitizer. Sanitize
untrusted HTML before displaying it in an HTML-capable environment.

For whole-document JSON loads, `{ type: 'localJson' }` handle initialization, controlled `valueJSON`, and `setContentJson()` will normalize an empty root document like `{ type: 'doc', content: [] }` to the active schema's empty text block so block-constrained schemas still load a valid empty document. For chat composer or draft-reset flows, prefer the ref method `clearContent()`.

## Development

Common commands:

```sh
npm run typecheck
npm run bench:rust -- --quick
npm run publish:prepare
npm run example:start
npm run example:ios
npm run example:android
npm run build:rust
```

Tests:

```sh
npm test                                             # TypeScript unit tests
cargo test --manifest-path rust/editor-core/Cargo.toml  # Rust core tests
npm run android:test                                  # Android Robolectric tests
npm run android:test:perf                             # Android native perf test suite
npm run android:test:perf:device                      # Android on-device perf instrumentation suite
npm run ios:test:perf                                 # iOS native perf XCTest suite
npm run ios:test:perf:device                          # iOS on-device perf XCTest suite
```

Benchmarks:

```sh
npm run bench:rust -- --quick
npm run bench:rust -- --filter collaboration
npm run bench:rust -- --json > perf-results.json
npm run android:test:perf
npm run android:test:perf:device
npm run ios:test:perf
npm run ios:test:perf:device
```

## Documentation

Documentation is published in the [GitHub Wiki](https://github.com/apollohg/react-native-prose-editor/wiki).

- [Documentation Index](https://github.com/apollohg/react-native-prose-editor/wiki): main documentation index
- [Installation Guide](https://github.com/apollohg/react-native-prose-editor/wiki/Installation): installation and local setup
- [Getting Started](https://github.com/apollohg/react-native-prose-editor/wiki/Getting-Started): first setup and first editor
- [Collaboration Guide](https://github.com/apollohg/react-native-prose-editor/wiki/Collaboration): Yjs collaboration wiring, source-of-truth rules, and persistence
- [Toolbar Setup](https://github.com/apollohg/react-native-prose-editor/wiki/Toolbar-Setup): toolbar setup patterns and examples
- [Mentions Guide](https://github.com/apollohg/react-native-prose-editor/wiki/Mentions): @-mentions addon setup and configuration
- [Styling Guide](https://github.com/apollohg/react-native-prose-editor/wiki/Styling): content, toolbar, and mention styling
- [NativeRichTextEditor Reference](https://github.com/apollohg/react-native-prose-editor/wiki/NativeRichTextEditor-Reference): component props and ref methods
- [Design Decisions](https://github.com/apollohg/react-native-prose-editor/wiki/Design-Decisions): rationale for key API and architecture decisions

## Project Status

The project is usable and already covers the core editing flows, but the API and documentation are still evolving as the package moves toward wider use.
