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
- configurable toolbar items, including app-defined actions
- auto-grow height behavior for parent-managed scroll containers
- a Rust-backed undo/redo history model

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

```tsx
import React, { useRef } from 'react';
import {
  NativeRichTextEditor,
  type NativeRichTextEditorRef,
} from '@apollohg/react-native-prose-editor';

export function EditorScreen() {
  const editorRef = useRef<NativeRichTextEditorRef>(null);

  return (
    <NativeRichTextEditor
      ref={editorRef}
      initialContent="<p>Hello world</p>"
      placeholder="Start typing..."
      onContentChange={(html) => {
        console.log(html);
      }}
    />
  );
}
```

## Customization

The main extension points today are:

- `schema`: provide a custom schema definition
- `theme`: style text blocks, blockquotes, lists, horizontal rules, background, and toolbar chrome, including a native-looking keyboard toolbar mode
- `toolbarItems`: define the visible toolbar controls and order
- `onToolbarAction`: handle app-defined toolbar buttons
- `onRequestLink`: collect or edit hyperlink URLs when a toolbar link item is pressed
- `addons`: configure optional features like @-mentions
- `heightBehavior`: switch between internal scrolling and auto-grow
- `imageLoadingPolicy`: bound native image bytes, timeouts, concurrency, queue depth, and decode dimensions

For setup and customization details, start with the [Documentation Index](https://github.com/apollohg/react-native-prose-editor/wiki).

For realtime collaboration, including the correct `useYjsCollaboration()` wiring, encoded-state persistence, remote cursors, and automatic reconnect behavior, see the [Collaboration Guide](https://github.com/apollohg/react-native-prose-editor/wiki/Collaboration).

Custom schema content expressions support ProseMirror-style sequences,
parentheses, alternation, `?`, `*`, `+`, and bounded/unbounded ranges such as
`{2}`, `{1,3}`, and `{1,}`. Invalid, unresolved, or non-constructible schemas
fall back to the default schema during native editor creation.

Image loading is bounded on both native platforms. Override the defaults on an
editor or viewer when needed:

```tsx
<NativeRichTextEditor
  imageLoadingPolicy={{
    maxSourceBytes: 5 * 1024 * 1024,
    connectTimeoutMs: 8_000,
    readTimeoutMs: 15_000,
    maxConcurrentRequests: 2,
    maxPendingRequests: 32,
    maxDecodeDimensionPx: 1_536,
  }}
/>
```

`getHtml()` preserves opaque HTML and is a serializer, not a sanitizer. Sanitize
untrusted HTML before displaying it in an HTML-capable environment.

For whole-document JSON loads, `initialJSON`, controlled `valueJSON`, and `setContentJson()` will normalize an empty root document like `{ type: 'doc', content: [] }` to the active schema's empty text block so block-constrained schemas still load a valid empty document. For chat composer or draft-reset flows, prefer the ref method `clearContent()`.

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
