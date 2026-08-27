# Breaking changes in 1.0.0

Version 1.0.0 is a hard cutover from 0.5.25. It replaces component-owned
documents, JavaScript-owned collaboration transport, and the legacy prose
viewer boundary; compatibility adapters are not included. See the
[changelog](./CHANGELOG.md#100---2026-08-01) for the release summary.

## Upgrade checklist

- Enable the React Native New Architecture. Paper builds and the legacy Expo
  prose viewer are unsupported.
- Create one `NativeEditorDocumentHandle` for each document, keep it stable for
  that document's lifetime, and call `destroy()` when its owner unmounts.
- Move existing document initialization, schema, collaboration fragment,
  length, and base64-image settings to handle creation; configure the new
  engine policies and limits there as needed.
- To keep existing camelCase JSON and commands unchanged, pass
  `tiptapCompatibleSchema` when creating or rendering documents. Migrate node
  names to snake_case only when adopting the new default schema.
- Pass the same handle to `NativeRichTextEditor`, `useNativeEditorDocument`, and
  `useYjsCollaboration` when they share a document.
- Replace `createWebSocket` and JavaScript retry settings with
  `transport: { url, connect } | null`. Native code now owns the socket.
- Configure the y-sync server to initialize new rooms during the standard
  handshake. Client JSON no longer seeds a room.
- Pass HTML or ProseMirror JSON directly to `NativeProseViewer` inside a host
  with finite width; remove viewer width, content ID, and height-cache plumbing.

## Editor documents

### Before: component-owned document

```tsx
<NativeRichTextEditor
    initialContent='<p>Hello world</p>'
    schema={schema}
    maxLength={10_000}
    onContentChange={setHtml}
/>
```

### After: handle-owned document

```tsx
import React, { useEffect, useMemo } from 'react';
import {
    createNativeEditorDocumentHandle,
    NativeRichTextEditor,
} from '@apollohg/react-native-prose-editor';

function EditorScreen() {
    const documentHandle = useMemo(
        () =>
            createNativeEditorDocumentHandle({
                initialization: {
                    type: 'localHtml',
                    html: '<p>Hello world</p>',
                },
                schema,
                policy: { maxLength: 10_000 },
            }),
        []
    );

    useEffect(() => () => documentHandle.destroy(), [documentHandle]);

    return (
        <NativeRichTextEditor
            documentHandle={documentHandle}
            onContentChange={setHtml}
        />
    );
}
```

The handle owns the immutable document contract. Use
`initialization.type: 'localEmpty'`, `'localHtml'`, `'localJson'`, or `'room'`.
Put engine-wide `maxLength`, `readOnly`, `inputFilter`, and
`allowBase64Images` under `policy`; put resource, editing, and collaboration
ceilings under `limits`.

`editable={false}` only disables mutations through that mounted view.
`policy.readOnly` applies to the document engine and must be chosen when the
handle is created.

Undo history is local-only: remote collaboration commits do not enter the
local undo stack. `valueJSONUpdateMode='reset'` and `clearContent()` clear
history.

## Schema node names

> **Simplest upgrade path:** Existing camelCase documents do not need to be
> migrated. Pass `tiptapCompatibleSchema` when creating or rendering them and
> keep their JSON and command node names unchanged.

The default schema now uses ProseMirror's snake_case node names. The previous
`tiptapSchema` export and the implicit camelCase default are removed;
`defaultSchema` is the same schema as `prosemirrorSchema`.

| 0.5.x camelCase name | 1.0.0 default name |
| --- | --- |
| `bulletList` | `bullet_list` |
| `orderedList` | `ordered_list` |
| `listItem` | `list_item` |
| `hardBreak` | `hard_break` |
| `horizontalRule` | `horizontal_rule` |

This affects node `type` values in persisted JSON, local JSON initialization,
inserted fragments, custom toolbar items, and direct editor commands. HTML tags,
mark names, node attributes, and ordinary node names such as `doc`, `paragraph`,
`heading`, `blockquote`, `image`, and `text` are unchanged.

### Migrate to the new default

Rewrite every affected node `type`, including nested list items. For example:

```json
{
    "type": "doc",
    "content": [
        {
            "type": "bullet_list",
            "content": [
                {
                    "type": "list_item",
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "Item" }]
                        }
                    ]
                }
            ]
        }
    ]
}
```

Update custom toolbar items and direct commands to use the same names:

```ts
editor.toggleList('bullet_list');
editor.toggleList('ordered_list');
editor.insertNode('hard_break');
editor.insertNode('horizontal_rule');
```

Once JSON and commands are migrated, omit `schema` to use `defaultSchema`, or
pass `defaultSchema` or `prosemirrorSchema` explicitly.

### Keep existing Tiptap-compatible JSON

If stored documents must retain camelCase node names, replace the removed
`tiptapSchema` import with `tiptapCompatibleSchema` and pass it wherever the
document is created or rendered:

```tsx
import {
    createNativeEditorDocumentHandle,
    NativeProseViewer,
    tiptapCompatibleSchema,
} from '@apollohg/react-native-prose-editor';

const documentHandle = createNativeEditorDocumentHandle({
    initialization: { type: 'localJson', json: existingDocument },
    schema: tiptapCompatibleSchema,
});

const viewer = (
    <NativeProseViewer
        contentJSON={existingDocument}
        schema={tiptapCompatibleSchema}
    />
);
```

The schema is part of a document handle's immutable contract and collaboration
fingerprint. Existing rooms and snapshots that contain camelCase nodes must
continue using `tiptapCompatibleSchema` until their content and schema metadata
are migrated together. Do not connect handles using different naming
conventions to the same room.

## Collaboration

### Before: JavaScript-owned WebSocket and mirrored JSON

```tsx
const collaboration = useYjsCollaboration({
    documentId,
    createWebSocket: () => new WebSocket(roomUrl),
    connect: true,
    retryIntervalMs: 1_000,
    initialDocumentJson,
    localAwareness: user,
});

return <NativeRichTextEditor {...collaboration.editorBindings} />;
```

### After: shared room handle and native transport

```tsx
import React, { useEffect, useMemo } from 'react';
import {
    createNativeEditorDocumentHandle,
    NativeRichTextEditor,
    useYjsCollaboration,
} from '@apollohg/react-native-prose-editor';

function CollaborativeEditor() {
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

    const collaboration = useYjsCollaboration({
        documentId,
        handle: documentHandle,
        transport: {
            url: roomUrl,
            connect: true,
        },
        localAwareness: user,
    });

    useEffect(() => () => documentHandle.destroy(), [documentHandle]);

    return <NativeRichTextEditor {...collaboration.editorBindings} />;
}
```

Swift owns the iOS WebSocket and Kotlin/OkHttp owns the Android WebSocket.
Rust owns connection generations, y-sync protocol state, retry timing,
awareness, peers, and outbound delivery. Set `transport` to `null` to leave the
document detached from native transport. Use `transport.protocolAdapter` for
an authentication or initialization prelude that must complete before y-sync.

New room handles remain unready until the server supplies and Rust accepts the
initial y-sync Step 2. Do not replace `initialDocumentJson` with a local write:
initialize the room on the server or restore a matching offline snapshot.

### Room snapshots

Export `{ metadataJson, encodedState }` from the document handle and persist
both values. Parse `metadataJson` before supplying it as room initialization:

```ts
const exported = documentHandle.bridge.snapshotExport();

const snapshot = {
    metadata: JSON.parse(exported.metadataJson),
    encodedState: exported.encodedState,
};

const restoredHandle = createNativeEditorDocumentHandle({
    initialization: {
        type: 'room',
        documentId,
        lineageId,
        snapshot,
    },
});
```

Snapshot metadata must match the room's document ID, lineage ID, fragment
name, and schema fingerprint. Raw controller state methods such as
`getEncodedState()`, `applyEncodedState()`, and `replaceEncodedState()` are
removed.

### Awareness and identifiers

Mounted editors publish the authoritative caret from native code, so
`editorBindings.onSelectionChange` is removed. For low-level handle awareness,
the `selection` key has three distinct meanings:

```ts
handle.setLocalAwareness({ state, focused });
// selection omitted: retain Rust's current sticky cursor

handle.setLocalAwareness({ state, focused, selection: null });
// null: clear the cursor

handle.setLocalAwareness({
    state,
    focused,
    selection: createNativeEditorLocalAwarenessSelection(anchor, head),
});
// factory value: set the cursor
```

Plain selection objects, clones, and proxies are rejected. Engine IDs,
revisions, generations, request IDs, and peer `clientId` values are canonical
decimal strings; peer clocks and cursor positions remain exact u32 numbers.

## Prose viewer

### Before: caller-managed measurement identity

```tsx
<NativeProseViewer
    contentJSON={documentJson}
    contentId={messageId}
    containerWidth={availableWidth}
/>
```

### After: direct content and prepared measurement

```tsx
<View style={{ width: availableWidth }}>
    <NativeProseViewer
        contentJSON={documentJson}
        renderImages={false}
        onPressLink={({ href }) => openLink(href)}
    />
</View>
```

The Fabric component accepts exactly one of `contentJSON` or `contentHTML` and
measures to its prepared native layout at the finite width supplied by its
host. Remove `contentId`, `containerWidth`, revision hints, and height-cache
management. When `renderImages={false}`, images are absent from layout,
accessibility, resource requests, metadata, and decoding; no placeholder is
drawn.

The viewer no longer accepts render-ops JSON or JavaScript mention-render
callbacks. Use serializable schema, theme, image policy, and
`addons.mentions`; receive normal link and mention press events. UIKit and
Android hosts can use the native `ProseViewerView` facade with direct JSON or
HTML input.

## Removed API mapping

| Pre-1.0 API or behavior | 1.0.0 migration |
| --- | --- |
| `initialContent` / `initialJSON` | Use handle `initialization: { type: 'localHtml' | 'localJson', ... }`. |
| `tiptapSchema` and implicit camelCase default node names | Migrate JSON and commands to the snake_case `defaultSchema`, or use `tiptapCompatibleSchema` consistently. |
| Editor `schema`, `maxLength`, and `allowBase64Images` props | Move them to handle `schema` and `policy`. Configure the new `readOnly`/`inputFilter` policy and resource/editing/collaboration limits there too. |
| `autoDetectLinks`, `preserveSelectionOnValueJSONReset`, and `selectionOnValueJSONReset` | Removed without replacements. |
| Collaboration `schema`, `fragmentName`, and `initialDocumentJson` | Configure the room handle once; the server initializes new room content. |
| `createWebSocket`, `retryIntervalMs`, and JavaScript socket/timer hooks | Use `transport: { url, connect, protocolAdapter? } | null`. |
| `initialEncodedState`, encoded-state utilities, and controller raw-state methods | Persist and restore document-scoped snapshots through the handle. |
| `editorBindings.valueJSON`, content callbacks, reset-selection props, and `onSelectionChange` | Bind the handle and revision directly; content callbacks remain editor props and native code publishes the caret. |
| Numeric collaboration IDs | Use canonical decimal strings for u64 IDs and revisions. |
| Collaboration-provided selection mirroring and `handleSelectionChange()` | Mounted native editors publish the caret directly; use `createNativeEditorLocalAwarenessSelection(anchor, head)` only for low-level handle updates. |
| Numeric `valueJSONRevision`, peer IDs, and remote-selection client IDs | Use canonical decimal strings. |
| `ImageRequestContext.allowBase64` | Use the handle's `policy.allowBase64Images` for document admission. |
| Viewer `contentId`, `containerWidth`, `contentRevision`/`contentJSONRevision`, and `clearHeightCache()` | Give the Fabric viewer a finite host width and let it measure prepared content directly. |
| `renderDocumentJson`, `renderDocumentHtml`, `measureContentHeight`, and render-ops inputs | Pass JSON or HTML to `NativeProseViewer` or native `ProseViewerView`. |
| Paper/Expo prose-viewer registration | Enable the New Architecture and use the Codegen Fabric component. |
| JavaScript viewer mention prefix/theme resolvers | Use serializable `addons.mentions` configuration and press events. |
| `EditorTheme.mentions` | Style mentions on the mentions addon via `addons.mentions.theme` and `resolveTheme`. |
| Flat `EditorMentionTheme` keys (`popover*`, `option*`, and shared `textColor`/`backgroundColor`/`border*`/`fontWeight`) | Group by surface: `node` for the mention in the document, `suggestions` for the list container, and `suggestions.option` for one row. |

## Build and package changes

Compiled Rust outputs are no longer tracked in Git. Package consumers still
receive the required native binaries. Repository contributors should run
`npm run build:rust` after cloning; release validation rebuilds the artifacts
from the pinned Rust source and toolchain.

For current examples and operational guidance, see
[Editor usage](./README.md#editor-usage), [Prose viewer](./README.md#prose-viewer),
[Collaboration](./README.md#collaboration), [Security](./README.md#security),
and [Development](./README.md#development).
