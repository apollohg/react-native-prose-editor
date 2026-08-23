# Changelog

## [1.0.0-alpha.4] - 2026-08-23

### Added

- Added external text composition sessions to `NativeRichTextEditorRef` on iOS
  and Android, including capability detection and provisional update, commit,
  cancel, and terminal-event APIs.
- Added provisional text presentation that can be revised without changing the
  document, undo history, collaboration state, or content callbacks until the
  session commits.

### Changed

- External composition commits now follow normal editor selection, policy,
  undo, collaboration reconciliation, error, interruption, and lifecycle
  behavior.

## [1.0.0-alpha.3] - 2026-08-15

### Fixed

- Fixed the Android native viewer intermittently rendering blank after Fabric
  prop updates.

## [1.0.0-alpha.2] - 2026-08-14

### Fixed

- Fixed JSON object inputs rejecting optional properties explicitly set to
  `undefined`, including collaboration awareness, custom schemas, and local
  document initialization.

## [1.0.0-alpha.1] - 2026-08-14

### Fixed

- Fixed the Android native keyboard toolbar so it animates in sync with the keyboard.

## [1.0.0-alpha] - 2026-08-01

This is a hard cutover to shared native document, collaboration, and viewer
boundaries. Pre-1.0 compatibility adapters are not retained; see
[`BREAKING_CHANGES.md`](./BREAKING_CHANGES.md) for upgrade guidance.

### Added

- Added `createNativeEditorDocumentHandle` as the document construction path,
  plus `useNativeEditorDocument`, document-scoped snapshots, and typed
  structured errors.
- Added the exact-size, JSON/HTML `NativeProseViewer` Fabric surface and public
  `ProseViewerView` facades for UIKit and Android Views.
- Added bounded document, editing, collaboration, and image-loading policies,
  with stricter custom-schema and content validation.
- Added task-list editing, native code-block rendering and commands, and
  start/scroll/end toolbar item placement.
- Added `NativeRichTextEditor.focusPreservingRefs` for external controls that
  should preserve editor focus, keyboard, and selection.
- Added `EditorTheme.list.markerGap` for spacing between markers and item
  content, plus `spacingAfter` for spacing after lists, including nested lists.
- Added `EditorTheme.list.orderedMarker` with decimal, alphabetic, and Roman
  numbering schemes plus `.` and `)` suffixes. Schemes cycle by nesting depth;
  the default cycle is `decimal`, `lowerAlpha`, `lowerRoman`.
- Added `useNativeEditorDocument().documentOrigin` to identify the source of
  the current document revision.

### Changed

- **Breaking:** `NativeRichTextEditor` now binds to a required document handle.
  One Yrs-backed session owns document state, local-only undo history, schema,
  policy, and limits across editor and collaboration consumers.
- **Breaking:** `useYjsCollaboration` now accepts the shared handle and
  `transport: { url, connect } | null`. Native code owns WebSockets while Rust
  owns y-sync, awareness, retries, peers, and outbound state; new rooms are
  initialized by the server handshake.
- Mounted editors now publish their live caret directly into Rust-owned sticky
  awareness, removing stale JavaScript selection mirroring from focus and
  state updates.
- `NativeProseViewer` now consumes document JSON or HTML directly, requires the
  React Native New Architecture, and measures from prepared native layout at
  the host width. Disabled images are omitted from layout and loading.
- Compiled Rust artifacts are no longer tracked in Git. Release validation
  builds them from the shipped source with the pinned toolchain, while
  published packages continue to include the required native binaries.
- `NativeEditorV2AtomicRenderSnapshot` now contains either full render blocks
  or an incremental render patch.

### Fixed

- Fixed collaboration document and cursor mapping around emoji and other
  non-ASCII text by separating Yjs UTF-16 offsets from editor scalar positions.
- Hardened native selection and awareness synchronization, bounded image
  loading, task-marker and viewer tap handling, schema admission, and atomic
  rejection of invalid or oversized content.
- Fixed dropped input and reduced typing latency during fast collaborative
  editing on iOS and Android, including when local edits overlap remote content
  or presence changes.
- Fixed `onContentChange` and `onContentChangeJSON` after native editor changes
  on iOS and Android.
- Fixed iOS toolbar group menus so they stay anchored to their parent button,
  open vertically, toggle closed on a second tap, and switch directly between
  groups.
- Fixed `NativeProseViewer` trailing-empty-paragraph collapse so every trailing
  empty paragraph is removed, including when the entire document is empty.
- Removed extra bottom space after the final viewer element and corrected
  spacing at nested list boundaries.

### Removed / migration

- Removed legacy component-owned initialization and engine configuration.
  Move `initialContent`/`initialJSON`, schema, `maxLength`, and base64-image
  admission to `createNativeEditorDocumentHandle()` and configure the new
  engine policies and limits there.
- Removed JavaScript WebSocket/retry plumbing, raw collaboration-state APIs,
  and document/selection mirroring through `editorBindings`. Use native
  transport configuration, document snapshots, and the handle-bound bindings.
- Removed Paper/Expo viewer registration, render-ops inputs, viewer render and
  height-measurement module functions, width/content height caches, and
  JavaScript mention render resolvers. Pass JSON or HTML directly to the Fabric
  viewer or native facade with serializable configuration.
- Removed `EditorTheme.mentions`. The mentions addon owns its own styling:
  set a base style with `addons.mentions.theme` and vary it per suggestion with
  `addons.mentions.resolveTheme`.
- **Breaking:** `EditorMentionTheme` is grouped by the surface each property
  styles instead of one flat set of keys. Use `node` for the mention rendered in
  the document, `suggestions` for the container behind the suggestion list, and
  `suggestions.option` for an individual row. The node and a suggestion row are
  now styled independently; previously `backgroundColor`, `border*`, and
  `fontWeight` silently drove both. `node.borderColor`, `node.borderWidth`, and
  `node.borderRadius` are drawn by `NativeProseViewer`; the editor renders the
  node with text spans and ignores them.

## [0.5.25] - 2026-06-28

### Fixed

- Mention suggestion chips now render raw suggestion labels prefixed by the configured trigger, so labels no longer need to include `@` in addon configuration.

### Tests

- Added React, Android, and iOS regression coverage for trigger-prefixed mention suggestion chip display with raw labels.

## [0.5.24] - 2026-06-22

### Fixed

- Android now defers host-level outside-tap observer and keyboard toolbar view removals, avoiding framework `ViewGroup.dispatchGetDisplayList` crashes during focused editor teardown and rapid re-entry.
- Android release builds now preserve native editor `View` class names in crash reports, making framework ViewGroup crashes actionable after minification.

### Tests

- Updated Android native view lifecycle coverage for deferred outside-tap observer cleanup.

## [0.5.23] - 2026-06-15

### Fixed

- Android auto-grow updates after editor recreation.

## [0.5.22] - 2026-06-12

### Fixed

- iOS no longer crashes with a stack overflow when a delegate-proxying keyboard library (e.g. react-native-keyboard-controller) wraps the focused editor's delegate. The editor now uses a dedicated internal `UITextViewDelegate` object instead of acting as its own delegate, so proxies can't forward private UITextView selectors back into the text view.

### Tests

- Added iOS regression coverage for keyboard-controller-style delegate proxies forwarding `keyboardInputChangedSelection:` back to the editor.

## [0.5.21] - 2026-06-06

### Added

- Added `NativeRichTextEditorRef.clearContent()` as a schema-aware cross-platform way to clear the editor for chat composer and draft reset flows.

### Fixed

- Android now preserves the same-editor input connection while refreshing IME state after JS-driven full-content replacements and native-mutation preflight.
- Android imperative whole-document content setters now retry after an in-flight native update is acknowledged, so chat-style composers can repeatedly call `clearContent()` or `setContentJson({ type: 'doc', content: [] })` after sending messages.
- Android now requests a fresh IME input connection when a focused editor is made editable again, preventing chat composers from getting stuck after send flows that briefly pass `editable={false}` while clearing content.
- Android `clearContent()` now uses a reset-style native update that bypasses composition preflight and clears stale pending update queues.
- Android now explicitly invalidates the rendered editor view after Rust-backed full-content updates, preventing `clearContent()` resets from leaving stale pre-send text visible until the next key press.
- Android `clearContent()` now also sends reset updates through a dedicated prop-backed native reset lane, so composer clears are delivered even when the immediate Expo view command is delayed, resolves `false`, or races native view readiness.
- Android Expo prebuild projects can use the package config plugin to exclude obsolete JNA `armeabi`, `mips`, and `mips64` native libraries, avoiding NDK strip warnings in consuming apps.

### Tests

- Added Android regression coverage for committing and composing text again after an external clear, including after preflight adopts a native text mutation.
- Added React wrapper regression coverage for retrying Android `setContentJson()` after an in-flight native update and for `clearContent()`.
- Added Android regression coverage for a focused read-only toggle keeping the old input connection blocked while allowing a fresh connection to type again.
- Added React wrapper and Android native view coverage for reset updates bypassing blocked preflight, invalidating rendered content, retrying when native readiness is delayed, and superseding stale in-flight editor updates.

## [0.5.20] - 2026-06-06

### Fixed

- Android refreshes the focused IME session after JS-driven full-content replacements.

## [0.5.19] - 2026-06-03

### Fixed

- iOS renders an emoji correctly when it is the first character in the editor.
- iOS native toolbar mention suggestions now use adaptive foreground rendering without nesting glass inside the toolbar chrome, with semibold glass labels, an animated chrome transition, and tighter item padding.
- iOS native toolbar disabled buttons now use transparent native foreground color instead of fixed gray.
- Enabling mentions after editing no longer leaves the example editor bound to a stale or reset native editor instance.

### Tests

- Added iOS regression coverage for leading emoji rendering without splitting UTF-16 surrogate pairs.
- Added iOS regression coverage for adaptive native toolbar mention suggestion text.
- Added React wrapper regression coverage for preserving uncontrolled content and native view commands across live mentions toggles.

## [0.5.18] - 2026-06-02

### Changed

- Android Rust render application is deferred after `finishComposingText` and composition commits so the `InputConnection` stays alive for Samsung's follow-up space or autocorrect commit.
- Android editor update events are debounced and batch-drained, skipping stale-editor payloads.
- Android IME context strips synthetic empty-block placeholders from surrounding text and reports sentence-caps at rendered line starts (including after list markers), with a Samsung Keyboard fallback for stale shift state.
- Android composing text carries transient paragraph font styling so Samsung's pre-commit composition matches the surrounding text.
- iOS tracks an authorized UTF-16 selection range to detect stale UIKit carets left behind by autocomplete/autocorrect and maps them through the replacement range before committing to Rust.
- iOS inline predictions (marked text without `insertText`) are no longer committed to Rust.
- Example app output panel updates are debounced during fast typing.

### Fixed

- Android `commitCorrection` followed by a matching `commitText` no longer double-inserts the corrected word.
- Android `commitCorrection` during active composition defers the Rust insert until the IME's follow-up `commitText`, preventing a missing-word or double-word race.
- Android `finishComposingText` no longer triggers an `InputConnection` recreation before Samsung's pending space commit arrives.
- Android single-character delete-and-replace autocorrections, such as Gboard changing `i` to `I`, no longer lose the replacement commit.
- Android skips redundant spannable re-application when the visible text already matches the incoming Rust render, avoiding a scroll-position reset.
- Android refreshes IME state after block-split operations and corrects Samsung Keyboard composing text when the keyboard ignores line-boundary sentence caps.
- iOS caret after autocomplete insertion, autocorrect replacement, and list-context mutations now lands after the replacement text instead of at the pre-mutation offset.
- iOS `setMarkedText` flushes any pending native autocorrect before entering composition.

### Tests

- Expanded iOS regression coverage for stale-caret mapping, inline prediction isolation, list-context mutations, length-changing autocorrect, and marked-text flushing.
- Expanded Android regression coverage for sentence-caps, placeholder stripping, Samsung keyboard composing, `InputConnection` lifecycle, correction commit deduplication, render deferral, optimistic render reuse, and surrounding-delete autocorrect handling.

## [0.5.17] - 2026-05-19

### Added

- Added Android device IME regression coverage and a `test:android:device:ime` script for native keyboard suggestion and autocorrect flows.

### Changed

- Hardened JS/native command coordination so toolbar actions, mention insertion, content updates, and cached reads prepare or flush native editor state before mutating Rust content.
- Added editor identity and document-version scoping across native events, mention queries, editor-ready retries, pending native actions, and command retries to ignore stale native work.
- Refreshed native iOS core framework metadata from the rebuilt editor core artifacts.

### Fixed

- iOS autocorrect, autocomplete, marked text, paste, blur, input trait updates, toolbar actions, and mention selection now preserve authorized native text changes through reconciliation.
- Android IME composition, autocorrect, autocomplete, hardware keys, selection/range mapping, destroyed-editor handling, and pending native actions are guarded against stale or conflicting updates.
- Native UTF-16/scalar position bridging now handles Unicode and list-marker offsets more consistently across Android and iOS.

### Tests

- Expanded Android, iOS, and JS regression coverage for autocorrect/autocomplete, command preflight and retry, mention query and selection scoping, controlled content sync, editor lifecycle guards, and Unicode position mapping.

## [0.5.16] - 2026-05-14

### Fixed

- iOS and Android keyboard suggestions and autocorrections now commit into the Rust-backed editor state instead of being immediately reverted by reconciliation.

## [0.5.15] - 2026-05-12

### Added

- `NativeRichTextEditor` now exposes `autoCapitalize`, `autoCorrect`, and `keyboardType` props on iOS and Android.

### Fixed

- Standalone `EditorToolbar` instances now render mention suggestions while the editor's built-in toolbar is hidden.
- Standalone mention suggestion chips now apply mention addon popover styles and per-suggestion `resolveTheme` overrides.
- iOS focus restoration calls now explicitly discard `becomeFirstResponder()` return values, removing Swift unused-result warnings.

## [0.5.14] - 2026-05-06

### Fixed

- Hardened Android consumer ProGuard rules and JNA dependency metadata so minified apps keep the UniFFI/JNA runtime classes, including `com.sun.jna.Native`.

## [0.5.13] - 2026-05-06

### Fixed

- Android release builds with R8 minification now include package consumer ProGuard rules for the UniFFI/JNA bridge, fixing missing `java.awt.Component` failures from JNA desktop-only references.

## [0.5.12] - 2026-05-06

### Fixed

- Android standalone toolbar taps no longer dismiss the keyboard before the first editor newline has been entered.
- Android backspace now continues correctly after breaking an empty trailing item out of a multi-item list.
- Backspacing over list marker prefixes now joins previous list items or unwraps the first item instead of leaving the cursor stuck.
- Empty-block placeholder backspace handling now sends the caret scalar on Android and tolerates the pre-placeholder scalar path in the Rust core.

## [0.5.11] - 2026-05-05

### Added

- `NativeRichTextEditorRef.getCaretRect()` returns the current caret rectangle and editor dimensions in editor-local coordinates on iOS and Android.
- Standalone `EditorToolbar` instances now preserve editor focus by default via measured toolbar hit-test frames, with `preserveEditorFocus={false}` available for embedded toolbar cases.

### Fixed

- Standalone toolbar taps no longer blur the editor or dismiss and immediately re-show the keyboard on Android and iOS.
- Android standalone toolbar hit testing now handles multiple toolbar frames and Android coordinate-space differences between React Native `measureInWindow` and native raw touch events.
- Android focus-preservation state now starts inactive and is cleared on real blur so outside taps can still dismiss the keyboard.
- Android no longer mutates the keyboard toolbar view hierarchy from inside `WindowInsets` dispatch, avoiding a framework `dispatchApplyWindowInsets` null-reference crash.
- Android native toolbar active-button color resolution now uses the AppCompat `colorPrimary` attribute fallback, avoiding a Material `R.attr.colorPrimary` `NoSuchFieldError`.
- Added regression coverage for caret geometry, standalone toolbar frame serialization, Android toolbar hit testing, and toolbar focus-preservation state.

## [0.5.10] - 2026-05-04

### Fixed

- Android CRDT editing now keeps Samsung Keyboard and other IME composing text transient until commit, then applies the final text at the original Rust-authorised range.
- iOS marked-text composition now uses the same original-range commit path, preventing transient `UITextView` composition state from shifting CRDT insertion or replacement offsets.
- Added Android and iOS regression coverage for composition commits, selection replacement, transient deletes, and reconciliation suppression during composing text.

## [0.5.9] - 2026-05-03

### Fixed

- iOS no longer crashes under `KeyboardProvider` when the editor is wrapped by `react-native-keyboard-controller`.
- Android now respects `theme.backgroundColor: "transparent"` instead of falling back to a white editor background.

## [0.5.8] - 2026-05-02

### Added
- `EditorTheme.placeholderColor` for customising the placeholder text color.

## [0.5.7] - 2026-05-02

### Added

- `NativeProseViewer.contentId` and `NativeProseViewer.containerWidth` props for height pre-measurement, removing the brief zero-height flash on mount in `FlatList`.
- `clearHeightCache()` export to free cached viewer heights.
- `EditorLinkTheme` interface and `EditorTheme.links` for styling link text.
- `MentionsAddonConfig.resolveTheme` callback for per-mention theme overrides at insertion time. The resolved theme is stored as a `mentionTheme` attribute on the mention node.
- Rust render engine now includes `mentionTheme` on `OpaqueInlineAtom` elements and applies `mentionSuggestionChar` trigger prefixes to mention labels.

### Fixed

- Mention insertion in `NativeRichTextEditor` now resolves per-mention theme via `resolveTheme` before writing attributes.

## [0.5.6] - 2026-05-01

### Fixed

- Updated Android JNA to `5.18.1` so bundled native JNA libraries support 16 KB page-size Android devices required by Google Play.

## [0.5.5] - 2026-04-30

### Fixed

- `NativeProseViewer` no longer emits height before the native text view has been laid out, fixing inflated heights for long messages in list contexts on both iOS and Android.
- `NativeProseViewer` now converts Android pixel heights to density-independent points, matching `NativeRichTextEditor` behaviour.
- `NativeProseViewer` on Android no longer shows spellcheck underlines on rendered text.

## [0.5.4] - 2026-04-30

### Fixed

- `NativeProseViewer` now collapses to zero height when `collapseTrailingEmptyParagraphs` is enabled and the rendered document contains only empty paragraphs.
- `NativeProseViewer` now accepts later corrected height measurements so very long messages do not keep stale extra bottom spacing.

## [0.5.3] - 2026-04-14

### Added

- `NativeProseViewer.collapseTrailingEmptyParagraphs` to trim trailing empty paragraph blocks in viewer output, enabled by default.
- `NativeProseViewer` link taps are now enabled by default, with `onPressLink` available to intercept taps and run custom actions instead of the native default open behaviour.
- `NativeProseViewer.contentHTML` for rendering read-only HTML input directly.
- `EditorTheme.list.baseIndentMultiplier` to control the starting list indent level independently from the per-level `indent` step.
- `EditorTheme.toolbar.marginTop` and `EditorTheme.toolbar.showTopBorder` to tune the inline toolbar spacing and top separator.

### Changed

- `NativeProseViewer.contentRevision` replaces the JSON-specific `contentJSONRevision` prop name.
- Mention suggestions now render inside the JS inline toolbar path when `NativeRichTextEditor.toolbarPlacement` is set to `"inline"`.

## [0.5.2] - 2026-04-13

### Added

- `NativeRichTextEditor.autoDetectLinks` for opt-in automatic URL detection.
- Mentions addon `resolveSelectionAttrs` to merge custom attrs into mention nodes.
- `NativeProseViewer` mention rendering hooks for prefix insertion and per-mention theme overrides.

### Fixed

- Mention insertions now preserve `mentionSuggestionChar` in node attrs for parity with Tiptap-style mention payloads.
- `NativeProseViewer` auto-height is more stable for long content in delayed-layout contexts and ignores stale late shrink measurements.
- iOS now removes the leftover leading text gutter when `contentInsets` are explicitly set to zero.
- iOS list bullets and ordered markers no longer inherit bold sizing or weight from the first inline run in a list item.

## [0.5.1] - 2026-04-12

### Added

- `NativeProseViewer` mention rendering hooks for viewer-side prefix insertion and per-mention theme overrides.

### Fixed

- Moved prettier to dev dependencies.

## [0.5.0] - 2026-04-12

### Added

- `NativeProseViewer` for rendering static ProseMirror/Prose JSON without creating a full editable editor instance.
- Tappable mention support in the viewer via `onPressMention`.
- Native `renderDocumentJson` bridge support for turning document JSON into render elements without mounting an editor surface.

## [0.4.3] - 2026-04-11

### Fixed

- iOS now restores sentence-start auto-capitalization when focus lands in a fully empty editor.
- `insertContentHtml()` and `insertContentJson()` now resolve mixed block fragments such as paragraph-plus-list content at the block level instead of trying to nest block nodes inside the current paragraph, preventing `paragraph has unexpected child 'paragraph'` transform errors.

## [0.4.2] - 2026-04-11

### Fixed

- Whole-document JSON entry points now normalize `{ type: 'doc', content: [] }` to a schema-valid empty document before passing content into the Rust core. This covers `initialJSON`, controlled `valueJSON`, and imperative `setContentJson()` or bridge-level JSON replacement calls, preventing `doc requires at least 1 child` transform errors with schemas that require block content.

## [0.4.1] - 2026-04-11

### Fixed

- Collaboration disconnect and destroy flows now clear and broadcast removal of the local awareness session so remote peers do not keep stale duplicate cursors after unmounts or reconnects.
- Collaboration state now preserves legitimately empty shared documents. The schema-aware empty-document fallback is bootstrap-only and is no longer reported back as durable shared content.
- Android placeholders now follow the effective paragraph text style and auto-grow height accounts for wrapped placeholder lines while the editor is empty.

### Performance

- iOS now applies localised render-block patches and faster attributed-text mutations for many in-place edits.
- iOS position conversion now uses cached UTF-16/scalar lookup tables with incremental patch updates.
- iOS auto-grow layout and remote-selection rendering now do less redundant work by caching measured content height, coalescing overlay refreshes, and skipping unchanged native prop payloads for theme, addons, toolbar, and remote selections.
- Android now skips full native render rebuilds for no-op updates when the resolved render blocks and visual styling are unchanged.
- Collaboration now avoids rebroadcasting identical local selection and focus awareness payloads.

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

[1.0.0-alpha.4]: https://github.com/apollohg/react-native-prose-editor/compare/1.0.0-alpha.3...1.0.0-alpha.4
[1.0.0-alpha.3]: https://github.com/apollohg/react-native-prose-editor/compare/1.0.0-alpha.2...1.0.0-alpha.3
[1.0.0-alpha.2]: https://github.com/apollohg/react-native-prose-editor/compare/1.0.0-alpha.1...1.0.0-alpha.2
[1.0.0-alpha.1]: https://github.com/apollohg/react-native-prose-editor/compare/1.0.0-alpha...1.0.0-alpha.1
[1.0.0-alpha]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.25...1.0.0-alpha
[0.5.25]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.24...0.5.25
[0.5.24]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.23...0.5.24
[0.5.23]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.22...0.5.23
[0.5.22]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.21...0.5.22
[0.5.21]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.20...0.5.21
[0.5.20]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.19...0.5.20
[0.5.19]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.18...0.5.19
[0.5.18]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.17...0.5.18
[0.5.17]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.16...0.5.17
[0.5.16]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.15...0.5.16
[0.5.15]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.14...0.5.15
[0.5.14]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.13...0.5.14
[0.5.13]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.12...0.5.13
[0.5.12]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.11...0.5.12
[0.5.11]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.10...0.5.11
[0.5.10]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.9...0.5.10
[0.5.9]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.8...0.5.9
[0.5.8]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.7...0.5.8
[0.5.7]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.6...0.5.7
[0.5.6]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.5...0.5.6
[0.5.5]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.4...0.5.5
[0.5.4]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.3...0.5.4
[0.5.3]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.2...0.5.3
[0.5.2]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.1...0.5.2
[0.5.1]: https://github.com/apollohg/react-native-prose-editor/compare/0.5.0...0.5.1
[0.5.0]: https://github.com/apollohg/react-native-prose-editor/compare/0.4.3...0.5.0
[0.4.3]: https://github.com/apollohg/react-native-prose-editor/compare/0.4.2...0.4.3
[0.4.2]: https://github.com/apollohg/react-native-prose-editor/compare/0.4.1...0.4.2
[0.4.1]: https://github.com/apollohg/react-native-prose-editor/compare/0.4.0...0.4.1
[0.4.0]: https://github.com/apollohg/react-native-prose-editor/compare/0.3.0...0.4.0
[0.3.0]: https://github.com/apollohg/react-native-prose-editor/compare/0.2.0...0.3.0
[0.2.0]: https://github.com/apollohg/react-native-prose-editor/compare/0.1.0...0.2.0
[0.1.1]: https://github.com/apollohg/react-native-prose-editor/compare/0.1.0...0.1.1
[0.1.0]: https://github.com/apollohg/react-native-prose-editor/releases/tag/0.1.0
