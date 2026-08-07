# React Native Prose Editor [![NPM version](https://img.shields.io/npm/v/@apollohg/react-native-prose-editor.svg?style=flat)](https://www.npmjs.com/package/@apollohg/react-native-prose-editor)

`@apollohg/react-native-prose-editor` is a native rich text editor for React Native. It combines a Rust document core with native iOS and Android editing, a React toolbar and theme API, a Fabric prose viewer, and Yjs collaboration.

The package is under active development. Review the [changelog](./CHANGELOG.md) before upgrading between major versions.

<p align="center">
  <img src="https://raw.githubusercontent.com/wiki/apollohg/react-native-prose-editor/images/example-ios.png" alt="Example editor on iOS" width="45%" />
  <img src="https://raw.githubusercontent.com/wiki/apollohg/react-native-prose-editor/images/example-android.png" alt="Example editor on Android" width="45%" />
</p>

## Highlights

- Native iOS and Android editing backed by a Rust document engine
- HTML and ProseMirror JSON input and output
- Configurable schemas, marks, blockquotes, lists, links, images, and mentions
- Native toolbar, theming, selection, undo, and redo
- `NativeProseViewer`, an exact-size Fabric renderer for read-only content
- Shared document handles for local editing and Yjs collaboration
- Bounded document, collaboration, and image-loading resources

## Requirements

The package uses custom native code and Expo Modules. Use a development build or a bare React Native app with Expo Modules configured; it does not run in Expo Go.

Published peer ranges are Expo 52+, React Native 0.76+, React 18+, and `@expo/vector-icons` 14+. The checked-in example and current release validation use Expo SDK 57, React Native 0.86, and React 19.2.3. Expo SDK 57 uses the New Architecture and requires iOS 16.4+ and Android 7+.

`NativeProseViewer` is New Architecture-only.

## Installation

Install the package and its icon peer dependency:

```sh
npm install @apollohg/react-native-prose-editor
npx expo install @expo/vector-icons
```

Add the config plugin to an Expo app:

```ts
export default {
    expo: {
        plugins: ['@apollohg/react-native-prose-editor'],
    },
};
```

Then regenerate and rebuild the native app:

```sh
npx expo prebuild
npx expo run:ios       # or: npx expo run:android
```

The plugin adds Android packaging exclusions for obsolete JNA ABIs. Existing generated or bare Android projects that do not run the plugin must apply the equivalent exclusions themselves:

```properties
android.packagingOptions.excludes=**/armeabi/libjnidispatch.so,**/mips/libjnidispatch.so,**/mips64/libjnidispatch.so
```

See the [Installation Guide](https://github.com/apollohg/react-native-prose-editor/wiki/Installation) for bare React Native setup and native build details.

## Editor usage

Every editor binds to a `NativeEditorDocumentHandle`. Create the handle once, initialize its content there, and destroy it when its owner unmounts.

```tsx
import React, { useEffect, useMemo } from 'react';
import {
    createNativeEditorDocumentHandle,
    NativeRichTextEditor,
} from '@apollohg/react-native-prose-editor';

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
        <NativeRichTextEditor
            documentHandle={documentHandle}
            placeholder='Start typing…'
            onContentChange={(html) => console.log(html)}
        />
    );
}
```

Handle initialization supports empty, HTML, JSON, and collaboration-room documents. Schema, editing policy, and resource limits are fixed when the handle is created. The mounted view accepts controlled `value` or `valueJSON`, toolbar and theme options, event callbacks, and imperative ref methods for formatting and content changes.

See [Getting Started](https://github.com/apollohg/react-native-prose-editor/wiki/Getting-Started) and the [NativeRichTextEditor reference](https://github.com/apollohg/react-native-prose-editor/wiki/NativeRichTextEditor-Reference) for the complete API.

## Prose viewer

`NativeProseViewer` renders HTML or ProseMirror JSON without creating an editor session. Supply exactly one content source and ensure its host provides a finite width.

```tsx
import { NativeProseViewer } from '@apollohg/react-native-prose-editor';

<NativeProseViewer
    contentJSON={{
        type: 'doc',
        content: [
            {
                type: 'paragraph',
                content: [{ type: 'text', text: 'Read-only content' }],
            },
        ],
    }}
    renderImages={false}
    onPressLink={({ href }) => openLink(href)}
/>;
```

The viewer measures to its prepared native layout, supports themes, bounded image loading, link and mention events, and font-environment invalidation. UIKit and Android applications can also use the native `ProseViewerView` facades directly.

## Collaboration

`useYjsCollaboration` connects a room-backed document handle to a Yjs sync and awareness server. The editor and collaboration controller must share the same handle.

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

    return <NativeRichTextEditor {...collaboration.editorBindings} />;
}
```

Native code owns the WebSocket while Rust owns sync, awareness, retries, and outbound state. The server must seed new rooms during the standard Yjs handshake. Authentication handshakes can be implemented with a `protocolAdapter`; offline restoration uses exported room snapshots.

See the [Collaboration Guide](https://github.com/apollohg/react-native-prose-editor/wiki/Collaboration) for server requirements, persistence, authentication, and recovery.

## Security

- Sanitize untrusted HTML before rendering it in an HTML-capable environment; serialization is not sanitization.
- Set document limits when creating a handle and viewer/image limits on `NativeProseViewer` when the defaults do not fit your use case.
- Treat collaboration URLs, credentials, frames, document content, and awareness payloads as sensitive data.

## Development

Compiled Rust libraries are build outputs and are not tracked in Git. Build them after cloning and after changes under `rust/editor-core/src`:

```sh
npm install
npm --prefix example install
npm run build:rust
npm run prebuild:example
```

Common commands:

```sh
npm run typecheck
npm test
npm run build:android:library
npm run test:ios
npm run validate:package
npm run run:example:ios
npm run run:example:android
```

The example app uses Expo Continuous Native Generation. Its `ios/` and `android/` directories are ignored and disposable. Root Android commands generate the Android project before invoking Gradle, and `validate:package` generates both projects and installs the example CocoaPods dependencies before validating native consumers.

The example app uses Expo SDK 57. Native builds require the pinned Rust toolchain; Android builds also require `ANDROID_NDK_HOME` and `cargo-ndk`.

## Documentation

- [Documentation index](https://github.com/apollohg/react-native-prose-editor/wiki)
- [Installation](https://github.com/apollohg/react-native-prose-editor/wiki/Installation)
- [Getting started](https://github.com/apollohg/react-native-prose-editor/wiki/Getting-Started)
- [Collaboration](https://github.com/apollohg/react-native-prose-editor/wiki/Collaboration)
- [Toolbar setup](https://github.com/apollohg/react-native-prose-editor/wiki/Toolbar-Setup)
- [Mentions](https://github.com/apollohg/react-native-prose-editor/wiki/Mentions)
- [Styling](https://github.com/apollohg/react-native-prose-editor/wiki/Styling)
- [Changelog](./CHANGELOG.md)

## License

[Apache-2.0](./LICENSE)
