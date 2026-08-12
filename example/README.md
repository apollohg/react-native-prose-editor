# Example App

A local Expo SDK 57 app for developing and manually testing the native editor package in this repository.

The package requires Expo Modules, and Expo SDK 57 is the version tested in this repo.

## What it covers

The screen is a coverage harness: every prop, ref method, and callback `NativeRichTextEditor` exposes is reachable from it. Settings are grouped one tab per API area so you can jump straight to whatever regressed.

| Tab      | Covers                                                                                                                        |
| -------- | ----------------------------------------------------------------------------------------------------------------------------- |
| Editor   | Base font size, mentions addon and schema, blockquote theme tokens                                                            |
| Toolbar  | `appearance` plus every toolbar theme token, colour and metric                                                                |
| Items    | `toolbarItems` — drag to reorder, add and remove, reset to package defaults                                                   |
| Content  | `value`, `valueJSON`, `valueJSONRevision`, `valueJSONUpdateMode`, `documentRevision`                                          |
| Commands | All 26 imperative ref methods as one-tap buttons. Mark buttons are read from the active schema, not hardcoded                 |
| Input    | `editable`, `autoFocus`, `autoCorrect`, `autoCapitalize`, `keyboardType`, `heightBehavior`, `showToolbar`, `toolbarPlacement` |
| Images   | `allowImageResizing`, every `imageLoadingPolicy` bound, insertion via picker and remote URL                                   |

The readout panel below the editor switches between the serialized HTML, the ProseMirror JSON, the mention payloads, and an event log carrying `onSelectionChange`, `onActiveStateChange`, `onHistoryStateChange`, `onLocalCommit`, `onToolbarAction`, `onFocus`, `onBlur`, `onRequestLink`, and `onRequestImage`.

**Not covered:** Yjs collaboration. `useYjsCollaboration` needs a running sync server to mean anything, so it is out of scope for this harness. See the [Collaboration Guide](https://github.com/apollohg/react-native-prose-editor/wiki/Collaboration).

## Theme presets

The four presets are **coverage fixtures, not recommended themes**. Do not copy them into a consuming app.

Each one commits to a different colour strategy _and_ a different point in the numeric range, because a preset set that varies only by hue proves nothing about the half of the theme API that takes numbers.

| Preset    | Strategy     | Covers                                                                                                                         |
| --------- | ------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| Example 1 | Restrained   | Every radius is `0` and the toolbar sits flush. Catches native code reading a falsy `0` as "unset" and substituting a default. |
| Example 2 | Committed    | Saturated chrome wrapped around a pale editor, at the loose end of the geometry range. Proves the two surfaces stay separable. |
| Example 3 | Full palette | Four role hues on one dark ground, and the only preset using `appearance: 'native'` on the toolbar.                            |
| Example 4 | Drenched     | Every surface carries real chroma (0.047–0.063), so a token silently falling back to a grey default is visible.                |

App chrome colours are separate from editor content colours on purpose, so a rendering bug in the package can never be mistaken for app styling. Chrome text tokens are held at 4.5:1 against every background they render on and channel indicators at 3:1; editor content tokens are free to diverge because they are the thing under test.

Colour controls are platform-split. iOS uses the system picker via `@expo/ui`'s SwiftUI `ColorPicker`; Android uses three RGB sliders. Both are constrained to opaque `#rrggbb` so a colour dialled in on one platform is reproducible on the other.

## Install

From the repository root:

```sh
cd example
npm install
```

## Prebuild

This package contains native code, so generate the native projects before building:

```sh
npm run prebuild
```

This runs Expo Prebuild in clean, non-installing mode. The generated `ios/` and `android/` directories are ignored, disposable, and must not contain source changes. Put native configuration in `app.config.ts` or the package config plugin.

The `ios` and `android` run commands prebuild their platform automatically. Run `npm run prebuild` directly when both projects are needed. Android device-test settings belong in the ignored `example/.android-device-test.env` file so regeneration cannot delete them.

## Run

From the repository root:

```sh
npm run run:example:ios
npm run run:example:android
```

Or directly inside `example/`:

```sh
npm run ios
npm run android
```

## Layout

```
App.tsx                  Screen composition, prop wiring, command dispatch, benchmark harness
constants.ts             Content fixtures, option lists, command ids, tab definitions
designTokens.ts          Radius, type, and spacing scales; touch-target minimum
themePresets.ts          The four presets and the app chrome token contract
sharedStyles.ts          Styles derived from designTokens
types.ts                 Grouped settings shapes and the event log entry
useReducedMotion.ts      OS reduce-motion subscription
components/              One file per control or panel
```

`components/` splits into primitives used everywhere (`ActionButton`, `ChoiceRow`, `SliderField`, `ToggleRow`, `PanelSection`, `CollapsibleSection`) and one panel per settings tab.

## Notes

- This package contains native code — use a development build, not Expo Go.
- The example app depends on the local package via `file:..` and resolves its types from `dist/`, so run the package build after changing `src/`.
- If you change native code or Rust bindings, rebuild the app after updating the package binaries.
- If the native build fails after pulling new changes, try running prebuild again.
- On iOS, the outer `ScrollView` adjusts its keyboard insets without shrinking its frame, so content can pass beneath the keyboard toolbar while remaining reachable.
- On Android, the example relies on the activity's `adjustResize` behavior instead of stacking `KeyboardAvoidingView` on top of the native editor insets.
- The editor still manages its own internal caret visibility and fixed-height scrolling. Screen-level keyboard avoidance and native editor viewport handling are complementary, not interchangeable.
- Slider-driven theme changes commit on release, not per step. An RGB channel at step 1 over 0-255 would otherwise push up to 255 theme rebuilds across the bridge in one drag.
- Collapsed sections unmount their children rather than clipping them, so a collapsed panel's ~40 controls do not render or lay out on every parent state change.

## Prepared viewer benchmark

The button in the header opens a `FlatList` harness that drives the checked-in corpus in `scripts/tests/viewer-performance-corpus.json` through cold, warm, and images-disabled phases, then exports native counters.

Its `renderItem` and `extraData` are memoized deliberately: an unstable `renderItem` puts React reconciliation cost inside the window the benchmark is measuring.
