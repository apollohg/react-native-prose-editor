[Back to docs index](../README.md)

# Design Decisions

## Why The Editor Is Native Instead Of WebView-Based

The project wants:

- native keyboard behavior
- better platform cursor and selection integration
- no DOM or browser runtime dependency
- one semantic editor core shared across iOS and Android

That is why the editor is rendered natively and why the Rust core owns document semantics.

## Why Document Semantics Live In Rust

The Rust core is the source of truth for:

- document structure
- schema validation
- transforms
- selection normalization
- undo/redo history
- active-state calculation

This keeps editor behavior consistent across iOS and Android.

## Why Styling Is Theme-Driven Instead Of Stylesheet-Driven

The rich text content is not a normal React Native subtree. It is rendered by native iOS and Android views using a render model produced by the Rust core.

That means:

- internal paragraphs are not React components
- list markers are not normal React children
- horizontal rules are native-rendered
- some toolbar UI is native-only

A normal React Native `StyleSheet` can style the outer container, but it cannot serve as the full cross-platform content styling API for editor internals.

That is why the package uses:

- `style` for the outer host container
- `theme` for content and toolbar styling

## Why Toolbar Icons Use Typed Descriptors

The icon model is intentionally explicit:

- `default`
- `glyph`
- `platform`

This exists because the iOS toolbar is a native keyboard accessory view. Arbitrary React elements cannot be rendered there. A typed icon descriptor provides a serializable cross-platform contract while still allowing explicit SF Symbol and Material icon mappings.

## Why The Toolbar Is Not Rendered The Same Way In Every Placement

The current split is:

- `toolbarPlacement="keyboard"`: native keyboard-hosted toolbar UI
- `toolbarPlacement="inline"`: React toolbar view above the editor

This reflects platform and layout fit rather than an attempt to force identical internals everywhere. The important thing is that the public API remains shared.

## Why There Is Both `showToolbar` And `EditorToolbar`

These serve different needs:

- `showToolbar` is the simplest integrated path
- `EditorToolbar` is the escape hatch for custom layout and advanced composition

## Why Schema Names Are String-Based

Operations such as `toggleMark('bold')` and `insertNode('hardBreak')` use schema names directly because the schema is configurable.

That means custom schemas can add their own nodes and marks without waiting for package-level enums to be expanded.

## Why The Example App Exists In-Repo

The editor spans:

- React Native
- Expo module wiring
- iOS native code
- Android native code
- Rust core behavior

Unit tests are necessary, but they do not replace a live host app for verifying focus, keyboard behavior, selection, theming, and toolbar interaction.

## Why There Is An iOS XCTest Harness As Well

Some bugs are too native-specific for the Expo example app alone. Native regression tests are useful for:

- position mapping
- marker rendering
- hard break edge cases
- native selection and cursor behavior
