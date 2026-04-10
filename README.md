# React Native Prose Editor [![NPM version](https://img.shields.io/npm/v/@apollohg/react-native-prose-editor.svg?style=flat)](https://www.npmjs.com/package/@apollohg/react-native-prose-editor)

`@apollohg/react-native-prose-editor` is a native rich text editor for React Native with a Rust document core, native iOS and Android rendering, configurable schemas, and a React-facing toolbar and theme API.

This project is currently in `alpha` and the API, behavior, and packaging may still change.

<p align="center">
  <img src="./docs/images/example-ios.png" alt="Example editor Android" width="45%" align="top" />
  <img src="./docs/images/example-android.png" alt="Example editor iOS" width="45%" align="top" />
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
- [`docs`](./docs): project documentation

## Installation

This package currently requires Expo Modules. Use it in an Expo development build or in a bare React Native app that has Expo Modules configured.

The minimum tested Expo version is SDK 54.

Required peer dependencies:

- `expo`
- `expo-modules-core`
- `react`
- `react-native`
- `@expo/vector-icons`

Install the package:

```sh
npm install @apollohg/react-native-prose-editor
```

For local package development in this repo:

```sh
npm install
npm --prefix example install
npm run example:prebuild
```

For full setup details, including peer dependencies, example app setup, and iOS pods, see the [Installation Guide](./docs/guides/installation.md).

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

For setup and customization details, start with the [Documentation Index](./docs/README.md).

For realtime collaboration, including the correct `useYjsCollaboration()` wiring, encoded-state persistence, remote cursors, and automatic reconnect behavior, see the [Collaboration Guide](./docs/modules/collaboration.md).

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

- [Documentation Index](./docs/README.md): main documentation index
- [Installation Guide](./docs/guides/installation.md): installation and local setup
- [Getting Started](./docs/guides/getting-started.md): first setup and first editor
- [Collaboration Guide](./docs/modules/collaboration.md): Yjs collaboration wiring, source-of-truth rules, and persistence
- [Toolbar Setup](./docs/guides/toolbar-setup.md): toolbar setup patterns and examples
- [Mentions Guide](./docs/modules/mentions.md): @-mentions addon setup and configuration
- [Styling Guide](./docs/guides/styling.md): content, toolbar, and mention styling
- [NativeRichTextEditor Reference](./docs/reference/native-rich-text-editor.md): component props and ref methods
- [Design Decisions](./docs/explanations/design-decisions.md): rationale for key API and architecture decisions

## Project Status

The project is usable and already covers the core editing flows, but the API and documentation are still evolving as the package moves toward wider use.
