# Example App

A local Expo SDK 57 app showing `react-native-rich-text-editor` as a single, full-screen document editor with every editor feature switched on.

The package requires Expo Modules, and Expo SDK 57 is the version tested in this repo.

## What it shows

One screen: a title bar with a live word count, and the editor filling the rest of the screen with the toolbar attached to the keyboard.

| Feature           | Where                                                                                                      |
| ----------------- | ---------------------------------------------------------------------------------------------------------- |
| Marks             | Bold, italic, underline, strikethrough buttons                                                             |
| Headings          | A `menu` group offering levels 1 through 6                                                                 |
| Links             | The link button opens a URL sheet driven by `onRequestLink`; saving an empty URL removes the link          |
| Images            | The image button opens the photo library through `onRequestImage`; picks are downscaled before insertion  |
| Lists             | A menu group with bullet and numbered lists, indent and outdent                                            |
| Blocks            | Blockquote button, plus divider and line break in the insert menu                                          |
| History           | Undo and redo at the end of the toolbar                                                                    |
| Mentions          | Type `@` to see suggestions filtered by the query through `onQueryChange`                                  |
| Custom atoms      | The counter card is a React component inside the document; the insert menu's counter action adds another  |
| Image resizing    | `allowImageResizing` is on, so a selected image shows native resize handles                                |

**Not covered:** Yjs collaboration. `useYjsCollaboration` needs a running sync server, so it is out of scope here. See the [Collaboration Guide](https://github.com/apollohg/react-native-rich-text-editor/wiki/Collaboration).

## Layout

```
App.tsx                      Screen composition, editor wiring, image picking
content.ts                   Initial document, mention suggestions, toolbar items
theme.ts                     The one palette, spacing scale, editor and mention themes
components/CounterCard.tsx   The custom atom node and its React component
components/LinkEditorModal.tsx  URL sheet driven by onRequestLink
```

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

## Notes

- This package contains native code, so use a development build, not Expo Go.
- The example app depends on the local package via `file:..` and resolves its types from `dist/`, so run the package build after changing `src/`.
- If you change native code or Rust bindings, rebuild the app after updating the package binaries.
- If the native build fails after pulling new changes, try running prebuild again.
- The editor uses `heightBehavior="fixed"` and scrolls internally. On iOS the sheet keeps its full height and the keyboard height is added to the editor's bottom content inset, so content scrolls behind the keyboard; on Android the activity's resize behavior shrinks the window instead.


## Custom atoms in the viewer

`components/ViewerCounterExample.tsx` renders the same counter component in `NativeProseViewer`. Mount it inside a container with a finite width to try it independently of the editor screen. Its React state owns the attributes: `onUpdateAtomAttrs` applies requested changes, then supplies a new `contentJSON`. Pass `readOnly` to disable its controls.

Atom components receive `isViewer` and `readOnly`. In an editor, `isViewer` is false and `readOnly` reflects `editable`. In a viewer, `isViewer` is true and `readOnly` defaults to true. Set `readOnly={false}` to allow interactions; components can handle their own callbacks or use `updateAttrs` with the viewer's `onUpdateAtomAttrs` handler. The viewer never stores changes itself.
