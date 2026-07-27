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
npm install @apollohg/react-native-prose-editor@3.0.0
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
  prosemirrorSchema,
  type NativeRichTextEditorRef,
} from '@apollohg/react-native-prose-editor';

export function EditorScreen() {
  const editorRef = useRef<NativeRichTextEditorRef>(null);

  const documentHandle = useMemo(
    () =>
      createNativeEditorDocumentHandle({
        // The frozen document contract: configure it here, never on the view.
        schema: prosemirrorSchema,
        fragmentName: 'prosemirror',
        initialization: { type: 'localHtml', html: '<p>Hello world</p>' },
        policy: {
          maxLength: 100_000,
          readOnly: false,
          inputFilter: '.*',
          allowBase64Images: false,
        },
        limits: {
          resource: {
            maxInputBytes: 20 * 1024 * 1024,
            maxDocumentNodes: 100_000,
            maxDocumentDepth: 256,
            maxSchemaNodes: 1_024,
            maxSchemaExpressionBytes: 64 * 1024,
            maxCollaborationMessageBytes: 10 * 1024 * 1024,
            maxEncodedStateBytes: 50 * 1024 * 1024,
          },
          editing: {
            maxOperationsPerTransaction: 1_024,
            maxUndoGroups: 200,
            maxUndoRetainedUnits: 1_000_000,
            maxDerivedOutputBytes: 20 * 1024 * 1024,
          },
          collaboration: {
            maxFramesPerMessage: 128,
            maxFrameBytes: 10 * 1024 * 1024,
            maxAggregateResponseBytes: 20 * 1024 * 1024,
            maxAwarenessPeers: 200,
            maxAwarenessPeerBytes: 64 * 1024,
            maxAwarenessBytes: 2 * 1024 * 1024,
            maxPendingOutboxMessages: 256,
            maxPendingOutboxBytes: 10 * 1024 * 1024,
            maxPendingDependencyUpdateBytes: 10 * 1024 * 1024,
            maxPendingDependencyUpdateWork: 1_000_000,
          },
        },
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

`createNativeEditorDocumentHandle` is the only document-construction boundary.
Its frozen `NativeEditorV2CreateConfig` accepts `schema`, `fragmentName`, a
required `initialization`, grouped `policy`, and all three grouped `limits`.
The component receives only `documentHandle` plus view-facing props such as
`placeholder`, `editable`, theme, toolbar, and event callbacks. Always destroy
the handle in effect cleanup, as shown above; it owns the native session.

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

View-facing props include `editable` (default true),
`autoFocus`, `autoCapitalize`, `autoCorrect`, `keyboardType`,
`heightBehavior`, `showToolbar` (default true), `toolbarPlacement`,
`toolbarItems`, `onToolbarAction`, `onRequestLink`, `onRequestImage`,
`allowImageResizing`, `imageLoadingPolicy`, `onSelectionChange`,
`onActiveStateChange`, `onFocus`, and `onBlur`. `editable` only gates
interaction in this mounted view; it never changes the handle's
`policy.readOnly`. See the 1.0.0 migration table in the
[CHANGELOG](./CHANGELOG.md) for pre-cutover names and replacements.

## Prepared Prose Viewer

`NativeProseViewer` is a New Architecture-only Fabric component. It accepts
exactly one direct document source: `contentJSON` or `contentHTML`. It does not
accept a width prop: Yoga supplies the finite host width, and the first native
measurement returns the exact prepared size. There is no Paper view, Expo
viewer adapter, render-ops bridge, height callback, or JavaScript height cache.

```tsx
<NativeProseViewer
  contentJSON={{ type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Ready once measured.' }] }] }}
  schema={prosemirrorSchema}
  renderImages={false}
  addons={{ mentions: { trigger: '@', prefix: '@', onPress: ({ label }) => openProfile(label) } }}
  onError={(error) => reportViewerError(error)}
/>
```

`renderImages` defaults to `true`. With `false`, image nodes are omitted before
layout: they contribute no size, accessibility node, request, metadata lookup,
or decode work. Mention appearance is declarative through document attributes,
the serializable `addons.mentions` settings, and theme; JavaScript mention
render callbacks are not supported.

A viewer host must provide a finite width. It may constrain height to clip the
result, but that never changes intrinsic content measurement. A mounted source
stays frozen unless its effective width changes, an enabled unknown-size image
resolves intrinsic dimensions, a requested font becomes available, or the
system text scale changes. Image completion without new dimensions only redraws.
Call `fontEnvironmentRevision` after a React Native font loader registers a
previously unavailable family; native UIKit and Android hosts expose equivalent
font invalidation hooks below. Errors create a zero-height error artifact and
are reported once per generation via `onError`.

### Native embedding

UIKit and Android expose the same direct-source facade for application-owned
native modules. Both require a complete serialized viewer configuration (the
same schema/policy configuration used by the React Native component), plus an
optional serialized theme and image-loading policy. Neither facade creates an
editor handle or accepts render-ops JSON.

`completeViewerConfigurationJSON` / `completeViewerConfigurationJson` must be
a serialized `PreparedProseViewerConfiguration`: `initialization` is
`{ "type": "localEmpty" }`, and `schema` contains the complete node and mark
definitions accepted for the document. Reuse the configuration serialization
from your React Native boundary rather than constructing an editor session.

UIKit:

```swift
import UIKit
import ReactNativeProseEditor

let configuration = ProseViewerConfiguration(
    configJSON: completeViewerConfigurationJSON,
    themeJSON: themeJSON,
    imagePolicyJSON: imageLoadingPolicyJSON,
    imagesEnabled: true,
    collapsesWhenEmpty: true
)

final class MessageCell: UICollectionViewCell {
    let proseViewer = ProseViewerView(frame: .zero)

    func configure(documentJSON: String) {
        _ = proseViewer.apply(source: .json(documentJSON), configuration: configuration)
    }

    override func prepareForReuse() {
        super.prepareForReuse()
        proseViewer.prepareForReuse()
    }
}

// HTML uses the same configuration:
// proseViewer.apply(source: .html(html), configuration: configuration)
// Registering a requested font? Call ViewerFontEnvironment.sharedEnvironment.invalidateRegisteredFonts().
```

Android:

```kotlin
import androidx.recyclerview.widget.RecyclerView
import com.apollohg.editor.ProseViewerConfiguration
import com.apollohg.editor.ProseViewerSource
import com.apollohg.editor.ProseViewerView

val configuration = ProseViewerConfiguration(
    configJson = completeViewerConfigurationJson,
    themeJson = themeJson,
    imagePolicyJson = imageLoadingPolicyJson,
    imagesEnabled = true,
    collapsesWhenEmpty = true,
)

class MessageViewHolder(val proseViewer: ProseViewerView) : RecyclerView.ViewHolder(proseViewer) {
    fun bind(documentJson: String) {
        proseViewer.apply(ProseViewerSource.Json(documentJson), configuration)
    }

    fun recycle() = proseViewer.prepareForReuse()
}

// HTML: proseViewer.apply(ProseViewerSource.Html(html), configuration)
// Registering a requested font? proseViewer.invalidateFontEnvironment()
```

The first UIKit `sizeThatFits`/Auto Layout fitting call and Android `onMeasure`
call require a finite width and prepare the exact layout once for that width.
Repeated measurements are cache reads; `prepareForReuse()` cancels generation
work and releases the retained artifact while preserving the interaction
delegate/listener. `ProseViewerInteractionDelegate` and
`ProseViewerInteractionListener` receive link, mention, and one-per-generation
error callbacks. UIKit uses points; Android uses pixels.

## Customization

The main extension points today are:

- `documentHandle` (required): the shared document session; initialization,
  schema, fragment, engine policy, and all limits are set at handle creation
- `editable`, `autoFocus`, `autoCapitalize`, `autoCorrect`, `keyboardType`: interactive behavior and keyboard configuration
- `showToolbar`, `toolbarPlacement`, `toolbarItems`, `onToolbarAction`, `onRequestLink`, `onRequestImage`: formatting toolbar (native keyboard accessory or inline React toolbar)
- `heightBehavior`: internal scrolling or grow-with-content layout
- `allowImageResizing`, `imageLoadingPolicy`: image interaction and loading bounds
- `theme`: style text blocks, blockquotes, lists, horizontal rules, background, and toolbar chrome
- `addons`: configure optional features like @-mentions
- `remoteSelections`: render collaboration peers' selections as native overlays

For setup and customization details, start with the [Documentation Index](https://github.com/apollohg/react-native-prose-editor/wiki).

## Collaboration

Realtime collaboration uses standard Yjs sync and awareness over a WebSocket,
with the Rust engine as the single source of truth. The editor and the
collaboration controller share one `NativeEditorDocumentHandle`; the controller
never creates or destroys sessions. Rooms are **server-initialized**: the
server speaks the y-sync protocol and sends the document during the handshake
(the editor renders nothing until the server document is promoted). There are
no client-side seeding or raw encoded-state APIs; see the 1.0.0 migration table
in the [CHANGELOG](./CHANGELOG.md) for historical API names.

```tsx
import React, { useEffect, useMemo } from 'react';
import {
  createNativeEditorDocumentHandle,
  NativeRichTextEditor,
  useYjsCollaboration,
} from '@apollohg/react-native-prose-editor';

export function CollaborativeEditor({
  documentId,
  getCurrentCredential,
}: {
  documentId: string;
  getCurrentCredential: () => Promise<string>;
}) {
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
  const protocolAdapter = useMemo(
    () => ({
      protocols: ['example-auth-v1'],
      timeoutMillis: 5_000,
      terminalCloseCodes: [4401, 4408],
      async onOpen({ negotiatedProtocol }: { negotiatedProtocol: string | null }) {
        if (negotiatedProtocol !== 'example-auth-v1') {
          return { action: 'reject' as const };
        }
        const credential = await getCurrentCredential();
        return {
          action: 'continue' as const,
          frames: [
            {
              type: 'text' as const,
              data: JSON.stringify({ type: 'authenticate', credential }),
            },
          ],
        };
      },
      async onMessage(
        _context: unknown,
        frame: { type: 'text' | 'binary'; data: string | Uint8Array }
      ) {
        return frame.type === 'text' && frame.data === '{"type":"authenticated"}'
          ? { action: 'ready' as const }
          : { action: 'reject' as const };
      },
    }),
    [getCurrentCredential]
  );
  const collaboration = useYjsCollaboration({
    documentId,
    handle: documentHandle,
    transport: {
      url: `wss://example.com/collaboration?documentId=${documentId}`,
      connect: true,
      protocolAdapter,
    },
    localAwareness: { userId: 'u-1', name: 'Ada', color: '#0A84FF' },
  });
  useEffect(() => () => documentHandle.destroy(), [documentHandle]);

  return (
    <NativeRichTextEditor
      documentHandle={collaboration.editorBindings.documentHandle}
      documentRevision={collaboration.editorBindings.documentRevision}
      remoteSelections={collaboration.editorBindings.remoteSelections}
      onFocus={collaboration.editorBindings.onFocus}
      onBlur={collaboration.editorBindings.onBlur}
    />
  );
}
```

`useYjsCollaboration` also renders `state` (transport status, rendered
document JSON/revision, last error) and `peers` (`NativeEditorV2PeerInfo[]` —
note `clientId` is a decimal **string**) for presence UI, and accepts
`transport`, `onPeersChange`, `onStateChange`, and `onError`
options. Bind the complete `editorBindings` set above — `documentHandle`,
`documentRevision`, `remoteSelections`, `onFocus`, and `onBlur` — so document
rendering and awareness stay on the one shared handle. Native code owns the WebSocket and flushes local commits directly.
Swift owns `URLSessionWebSocketTask` on iOS, while Kotlin owns OkHttp on
Android. JavaScript never constructs the collaboration socket and owns no
retry, deadline, or outbound-drain timer.

Rust remains authoritative for y-sync, awareness, generations, retry
eligibility, deadlines, peers, and exact outbound leases. Native drivers only
execute Rust directives on one serialized context per handle. Frames are
binary-only after synchronization starts.

UIKit and Android publish the authoritative editor selection into retained
Rust awareness in the same native mutation path, before waking the transport
for the document update. Rust holds that cursor as a sticky index, so it keeps
tracking the document through every later edit without being restated.

React Native therefore owns awareness identity, application state, and focus —
never a document position. `NativeEditorLocalAwarenessIntent.selection` makes
this explicit: **omit** the key to retain the Rust-owned cursor (what focus and
state updates do), pass `null` to publish presence with no cursor, or pass a
`createNativeEditorLocalAwarenessSelection(anchor, head)` value to set it.
Headless callers that own their own selection can publish one through
`YjsCollaborationController.handleSelectionChange`; a position that does not
resolve against the current document is refused and reported through `onError`.

Awareness publication never throws into an editor binding or fails an edit.
Presence is ambient UI state: a refusal is reported through `onError` and the
next change retries.

`protocolAdapter` is optional and protocol-agnostic. Its `protocols` are
offered in the physical WebSocket handshake. On every physical open, RN
receives a fresh attempt context, can read current credentials, and returns
text or binary initialization frames. Pre-open server frames are delivered
only to `onMessage`; returning `ready` opens the Rust y-sync generation,
`continue` keeps it gated, and `reject` parks the attempt. Native ignores
late responses from retired attempts. Codes in `terminalCloseCodes` also park
the Rust transport without consuming queued local work. Without an adapter,
y-sync starts as soon as the physical socket opens.

Treat transport URLs as credentials: do not log the complete URL, query,
adapter frames, credentials, document content, awareness payload, frame bytes,
or native error details.
Log only redacted endpoint identity and bounded state/counter fields. The
native owner follows app foreground/background lifecycle and is destroyed
before its document handle.

Low-level bridge callers must create a local awareness range with
`createNativeEditorLocalAwarenessSelection(anchor, head)`. Plain objects,
clones, and proxies are rejected; Rust alone derives the sticky wire cursor.

### Reconnect recovery

For an explicit recovery attempt (including a transport parked as
incompatible), call `collaboration.reconnect()`; do not recreate the document
handle or construct a second controller. Native code retires the current
socket and asks Rust to start a fresh generation against the same session:

```tsx
function recoverCollaboration() {
  collaboration.reconnect();
}
```

The hook removes its native event subscription and detaches the native
transport on unmount. The owning component must still destroy
`documentHandle` in its cleanup effect, after the hook has been declared, so
transport teardown precedes session destruction.

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
`{2}`, `{1,3}`, and `{1,}`. The frozen creation contract rejects invalid,
unknown, unresolved, or non-constructible schemas, as well as unknown
configuration keys and invalid configuration values; it never falls back to a
default schema during native editor creation.

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

All document, schema, collaboration, and image inputs are admitted through configurable limits with safe defaults and non-configurable hard ceilings. Editor resource limits are set when creating the document handle (viewer limits remain viewer props):

```tsx
const documentHandle = createNativeEditorDocumentHandle({
  initialization: { type: 'localEmpty' },
  // See Basic Usage for every resource, editing, and collaboration field.
  limits: { resource: { maxInputBytes: 20 * 1024 * 1024 } },
});
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

### Building the native core

The compiled `editor-core` artifacts — `ios/EditorCore.xcframework` and
`rust/android/*/libeditor_core.so` — are **build outputs and are not tracked
in git**. CI builds them from the pinned Rust toolchain and publishes them
inside the npm tarball, so a released package is always built from the source
it ships beside.

After a fresh clone, build them once before anything that links them (the iOS
test workspace, the example app, or the Android connected tests):

```sh
npm run build:rust           # both platforms
npm run build:rust:ios       # iOS only
npm run build:rust:android   # Android only
```

Rebuild after any change under `rust/editor-core/src`. Commands that need the
artifacts fail up front with the exact build command rather than a linker
error. iOS needs the pinned toolchain from `rust/toolchain.sh`; Android also
needs `ANDROID_NDK_HOME` and `cargo-ndk`.

Consumers installing from npm never build Rust — the tarball carries the
compiled core.

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
