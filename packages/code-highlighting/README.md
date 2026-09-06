# Native code highlighting

Optional syntax highlighting for `react-native-rich-text-editor`. Requires Expo native modules and a native app rebuild after installation.

```sh
npm install @react-native-rich-text-editor/code-highlighting
```

```tsx
import { RichTextEditor } from 'react-native-rich-text-editor';
import { createCodeHighlightingAddon } from '@react-native-rich-text-editor/code-highlighting';

const highlighting = createCodeHighlightingAddon({ theme: 'base16-ocean.dark' });

<RichTextEditor documentHandle={documentHandle} addons={[highlighting]} />;
```

`RichTextViewer` accepts the same addon. Set the code block's `language` attribute to select its grammar. Supported language values are `javascript`/`js`, `typescript`/`ts`, `tsx`, `jsx`, `swift`, `kotlin`/`kt`, `rust`/`rs`, `python`/`py`, `json`, `html`, `css`, and `bash`/`shell`/`sh`. JSX uses the TypeScript React grammar. Missing, plain, and unsupported language values render ordinary code without guessing.

`codeHighlightingThemes` lists the seven supported theme names. The selected theme contributes token foregrounds and bold/italic/underline traits. The editor's `codeBlock` style controls panel geometry and background.

The package registers its native provider synchronously when imported. Factories only create immutable configuration. Omit the descriptor to disable work for a view; uninstall and rebuild to remove the engine and grammar assets from the app. The base editor does not install this package automatically.

The engine uses syntect 5.3.0 with its pure Rust regex backend and the two-face 0.5.1 compatible grammar pack. Prebuilt libraries are included for iOS device/simulator and Android arm64-v8a, armeabi-v7a, x86, and x86_64. Consumer installation does not invoke Cargo.

Blocks over 64 KiB, lines over 4 KiB, or blocks over 1,000 lines fall back to ordinary styling. A worker-local cache retains at most 64 blocks and 1 MiB of accounted source/range data. Regex backtracking has the upstream finite step limit; this is not a wall-clock timeout. No syntax state or color marks are written to the document.

Maintainers build from this repository with Rust 1.95.0, the seven mobile Rust targets, Xcode, Android NDK and cargo-ndk installed:

```sh
npm run build
npm run test:rust
npm run build:native
npm pack
```
