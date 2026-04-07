# Changelog

## [0.4.0] - 2026-04-07

### Added

- Heading support with `h1` through `h6` schema nodes in both `tiptapSchema` and `prosemirrorSchema` presets.
- `group` toolbar item type for collapsing multiple buttons behind one slot, with `'expand'` and `'menu'` presentation modes.

## [0.3.0] - 2026-04-06

### Added

- Block image node with native resize handles on iOS and Android.
- `onRequestImage`, `insertImage(src, attrs?)`, `ImageRequestContext`, `allowBase64Images`, `allowImageResizing`.
- `imageNodeSpec`, `withImagesSchema`, `buildImageFragmentJson` schema helpers.
- `onHistoryStateChange` callback for standalone toolbar undo/redo state.
- `attrs` field on `RenderElement`.

### Fixed

- iOS native toolbar disabled buttons invisible on dark blur backgrounds.

### Changed

- Default toolbar icon set now includes `image`.

## [0.2.0] - 2026-04-02

### Added

- Real-time collaboration support via `useYjsCollaboration` hook and `createYjsCollaborationController` factory.
- Collaboration types: `YjsCollaborationOptions`, `YjsCollaborationState`, `YjsTransportStatus`, `LocalAwarenessUser`, `LocalAwarenessState`, `CollaborationPeer`, `EncodedCollaborationStateInput`.
- Remote selection decorations for rendering other users' cursors and selections as native overlays.
- Blockquote support: `toggleBlockquote()` ref method and default toolbar item.
- Link toolbar item type with `onRequestLink` callback and `LinkRequestContext` for host-driven URL entry.
- `markAttrs` field on `ActiveState` exposing active mark attributes (e.g. link `href`).
- `EditorLayoutManager` on iOS for custom glyph and list marker rendering.
- `PositionBridge` on iOS for UTF-16 to Unicode scalar offset conversion.
- `RemoteSelectionOverlayView` on Android for rendering remote user selections.
- Native collaboration session management in the bridge layer (create, destroy, encode/decode state).
- `encodeCollaborationStateBase64` and `decodeCollaborationStateBase64` utility functions.
- `buildMentionFragmentJson` helper for programmatic mention insertion.
- Collaboration guide with full API reference documentation.
- Documentation for `RemoteSelectionDecoration`, `EditorAddonEvent`, and mention schema helpers.

### Changed

- Default `heightBehavior` is now `'autoGrow'` instead of `'fixed'`.
- Android toolbar updated with blockquote and link icon mappings and Material Design color support.
- iOS native module now exposes `editorSetMark`, `editorUnsetMark`, and `editorToggleBlockquote`.
- Example app restructured with collaboration panel and updated demo components.

## [0.1.1] - 2026-03-30

### Fixed

- iOS test destination configuration.

## [0.1.0] - 2026-03-30

### Added

- Initial release with native rich text editor for React Native (Expo module).
- Rust-powered editor core with ProseMirror-compatible document model.
- iOS and Android native rendering with platform text views.
- Built-in formatting toolbar with configurable items.
- Tiptap and ProseMirror schema presets.
- `EditorTheme` system for content, mention, and toolbar styling.
- Mentions addon with native suggestion UI.
- Controlled and uncontrolled content modes (HTML and JSON).
- Undo/redo history.

[0.4.0]: https://github.com/apollohg/react-native-prose-editor/compare/0.3.0...0.4.0
[0.3.0]: https://github.com/apollohg/react-native-prose-editor/compare/0.2.0...0.3.0
[0.2.0]: https://github.com/apollohg/react-native-prose-editor/compare/0.1.0...0.2.0
[0.1.1]: https://github.com/apollohg/react-native-prose-editor/compare/0.1.0...0.1.1
[0.1.0]: https://github.com/apollohg/react-native-prose-editor/releases/tag/0.1.0
