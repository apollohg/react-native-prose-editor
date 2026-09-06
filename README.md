# React Native Rich Text Editor [![NPM version](https://img.shields.io/npm/v/react-native-rich-text-editor.svg?style=flat)](https://www.npmjs.com/package/react-native-rich-text-editor)

`react-native-rich-text-editor` is a native rich text editor for React Native. It combines a Rust document core with native iOS and Android editing, a React toolbar and theme API, a Fabric prose viewer, and Yjs collaboration.

See the [documentation](https://github.com/apollohg/react-native-rich-text-editor/wiki) for guides and API references. The package is under active development; review the [changelog](./CHANGELOG.md) before upgrading between major versions.

<img src="https://github.com/apollohg/react-native-rich-text-editor/wiki/images/banner.png" alt="Example editor on iOS" width="100%" />

## Highlights

- Native iOS and Android editing backed by a Rust document engine
- HTML and ProseMirror JSON input and output
- Configurable schemas, marks, blockquotes, lists, links, images, and mentions
- Custom atom nodes rendered with your React components
- Native toolbar, theming, selection, undo, and redo
- `RichTextViewer`, an exact-size Fabric renderer for read-only content
- Shared document handles for local editing and Yjs collaboration

`NativeRichTextEditor` and `NativeProseViewer`, along with their associated types, remain available as deprecated aliases of `RichTextEditor` and `RichTextViewer`.

## Requirements

The package uses custom native code and Expo Modules. Use a development build or a bare React Native app with Expo Modules configured; it does not run in Expo Go.

Requires Expo 52+, React Native 0.76+, React 18+, and `@expo/vector-icons` 14+. `RichTextViewer` requires the New Architecture. See the [Installation Guide](https://github.com/apollohg/react-native-rich-text-editor/wiki/Installation) for platform requirements.

## Installation

Install the package and its icon peer dependency:

```sh
npm install react-native-rich-text-editor
npx expo install @expo/vector-icons
```

Add the config plugin to an Expo app:

```ts
export default {
    expo: {
        plugins: ['react-native-rich-text-editor'],
    },
};
```

Then regenerate and rebuild the native app:

```sh
npx expo prebuild
npx expo run:ios       # or: npx expo run:android
```

See the [Installation Guide](https://github.com/apollohg/react-native-rich-text-editor/wiki/Installation) for bare React Native setup and migration from `@apollohg/react-native-prose-editor`.

## Editor usage

Every editor binds to a `NativeEditorDocumentHandle`. Create the handle once, initialize its content there, and destroy it when its owner unmounts.

```tsx
import React, { useEffect, useMemo } from 'react';
import {
    createNativeEditorDocumentHandle,
    RichTextEditor,
} from 'react-native-rich-text-editor';

export function EditorScreen() {
    const documentHandle = useMemo(
        () =>
            createNativeEditorDocumentHandle({
                initialization: {
                    type: 'localHtml',
                    html: '<p>Hello world</p>',
                },
            }),
        []
    );

    useEffect(() => () => documentHandle.destroy(), [documentHandle]);

    return (
        <RichTextEditor
            documentHandle={documentHandle}
            placeholder='Start typing…'
            onContentChange={(html) => console.log(html)}
        />
    );
}
```

See [Getting Started](https://github.com/apollohg/react-native-rich-text-editor/wiki/Getting-Started) and the [RichTextEditor reference](https://github.com/apollohg/react-native-rich-text-editor/wiki/RichTextEditor-Reference) for the complete API.

## Custom atom nodes

Render interactive cards, embeds, and other custom blocks with your own React components. Define an atom with `defineAtomNode`, add it to your document schema, and pass it through the editor or viewer's `atoms` prop.

See [Custom Atom Nodes](https://github.com/apollohg/react-native-rich-text-editor/wiki/Custom-Atom-Nodes) for a complete example and API details.

## Rich text viewer

`RichTextViewer` displays HTML or ProseMirror JSON without creating an editor session. Place it in a container with a finite width:

```tsx
import { RichTextViewer } from 'react-native-rich-text-editor';

<RichTextViewer contentHTML='<p>Read-only content</p>' />;
```

See the [Viewer Guide](https://github.com/apollohg/react-native-rich-text-editor/wiki/Viewer) for styling, images, interactions, and custom atoms.

## Element styling and addons

Version 2 uses a flat, typed stylesheet for both the editor and viewer:

```tsx
import { EditorStyleSheet, RichTextEditor } from 'react-native-rich-text-editor';

const theme = EditorStyleSheet.create({
    content: { padding: 16, backgroundColor: '#ffffff' },
    text: { fontSize: 16, color: '#202124' },
    paragraph: { marginBottom: 12 },
    link: { color: '#285dcc', textDecorationLine: 'underline' },
    blockquote: {
        backgroundColor: '#f3f5f8',
        padding: 12,
        borderWidth: 1,
        borderColor: '#dce1e8',
        borderLeftWidth: 4,
        borderLeftColor: '#285dcc',
        borderTopRightRadius: 8,
    },
    codeBlock: { backgroundColor: '#eff1f5', padding: 12, borderRadius: 8 },
    image: { backgroundColor: '#eff1f5', borderRadius: 12, resizeMode: 'cover' },
});

<RichTextEditor documentHandle={documentHandle} theme={theme} />;
```

Each entry accepts a style object or nested, conditional style arrays. Later entries override earlier properties; explicit `undefined` removes an earlier property. Side and corner properties override shorthands. Supported fields are checked by TypeScript and validated before native updates. These are editor styles: layout fields such as `flex`, transforms, and positioning are not accepted.

Text styling inherits from `text` and enclosing blocks. Backgrounds, spacing, borders, and corner radii belong to each element. Inline entries support typography and backgrounds; mentions also support borders. List containers, items, markers, and checkboxes have separate entries. `toolbar` retains its existing configuration.

Android editing uses per-block text layout for physical margins, borders, and padding, including RTL content. Per-block justification is supported on Android API 26+; API 24/25 use normal alignment.

Custom atom components mount inside Android's native scrolling content. Their measured height participates in document layout, and their controls receive normal React Native touch events.

Addons are a readonly array. Conditional `false`, `null`, and `undefined` entries are allowed; duplicate capabilities are rejected:

```tsx
import { createMentionsAddon } from 'react-native-rich-text-editor';
import { createCodeHighlightingAddon } from '@react-native-rich-text-editor/code-highlighting';

<RichTextEditor
    documentHandle={documentHandle}
    theme={theme}
    addons={[
        createMentionsAddon({ trigger: '@', suggestions }),
        enableHighlighting && createCodeHighlightingAddon({ theme: 'base16-ocean.dark' }),
    ]}
/>;
```

Syntax highlighting requires the separately installed [code-highlighting package](./packages/code-highlighting), followed by a native rebuild. The base editor does not include syntect or its grammars. Set a code block's `attrs.language`, for example `{ type: 'codeBlock', attrs: { language: 'typescript' }, content: [...] }`. Missing or unsupported languages keep ordinary code styling. The code block theme controls the panel; the highlighting addon controls token colors and font traits. Mention-enabled editor handles still require `withMentionsSchema` when creating their schema.

When migrating from version 1, replace `links` with `link`, heading maps with `h1`–`h6`, `spacingAfter` with `marginBottom`, and `contentInsets` with `content.padding*`. Flatten nested quote/code typography into their element entry, and move list appearance into the relevant container/item/marker entries. Addon objects become `[createMentionsAddon(options)]`. The new built-in schema declares code-block language metadata; coordinate schema changes across collaborating clients.

## Collaboration

`useYjsCollaboration` connects a room-backed document handle to a Yjs sync and awareness server. The editor and collaboration controller must share the same handle.

```tsx
import React, { useEffect, useMemo } from 'react';
import {
    createNativeEditorDocumentHandle,
    RichTextEditor,
    useYjsCollaboration,
} from 'react-native-rich-text-editor';

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

    const collaboration = useYjsCollaboration({
        documentId,
        handle: documentHandle,
        transport: {
            url: `wss://example.com/collaboration?documentId=${encodeURIComponent(documentId)}`,
            connect: true,
        },
        localAwareness: {
            userId: 'user-1',
            name: 'Ada',
            color: '#0A84FF',
        },
    });

    useEffect(() => () => documentHandle.destroy(), [documentHandle]);

    return <RichTextEditor {...collaboration.editorBindings} />;
}
```

See the [Collaboration Guide](https://github.com/apollohg/react-native-rich-text-editor/wiki/Collaboration) for server requirements, persistence, authentication, and recovery.

## Development

See the [example app](./example) to try the editor and viewer, and the [Development Workflow](https://github.com/apollohg/react-native-rich-text-editor/wiki/Development-Workflow) for local setup, native builds, and testing.

## Documentation

- [Documentation index](https://github.com/apollohg/react-native-rich-text-editor/wiki)
- [Installation](https://github.com/apollohg/react-native-rich-text-editor/wiki/Installation)
- [Getting started](https://github.com/apollohg/react-native-rich-text-editor/wiki/Getting-Started)
- [Editor API reference](https://github.com/apollohg/react-native-rich-text-editor/wiki/RichTextEditor-Reference)
- [Viewer API reference](https://github.com/apollohg/react-native-rich-text-editor/wiki/RichTextViewer-Reference)
- [Custom atom nodes](https://github.com/apollohg/react-native-rich-text-editor/wiki/Custom-Atom-Nodes)
- [Collaboration](https://github.com/apollohg/react-native-rich-text-editor/wiki/Collaboration)
- [Toolbar setup](https://github.com/apollohg/react-native-rich-text-editor/wiki/Toolbar-Setup)
- [Mentions](https://github.com/apollohg/react-native-rich-text-editor/wiki/Mentions)
- [Styling](https://github.com/apollohg/react-native-rich-text-editor/wiki/Styling)
- [Production limits and errors](https://github.com/apollohg/react-native-rich-text-editor/wiki/Production-Limits-and-Errors)
- [Changelog](./CHANGELOG.md)

## License

[Apache-2.0](./LICENSE)
