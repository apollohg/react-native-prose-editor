# Documentation

This is the main entry point for the project documentation.

The docs are split into four groups:

- guides for setup and common integration tasks
- reference pages for the public API and exported types
- explanations for architecture and design rationale
- development notes for working on the package itself

If you are new to the project, follow this order:

1. [Installation Guide](./guides/installation.md)
2. [Getting Started](./guides/getting-started.md)
3. [Toolbar Setup](./guides/toolbar-setup.md)
4. [Styling Guide](./guides/styling.md)
5. [NativeRichTextEditor Reference](./reference/native-rich-text-editor.md)

## Guides

| Document | Description |
| --- | --- |
| [Installation Guide](./guides/installation.md) | Peer dependencies, local repository setup, example app setup, and platform notes. |
| [Getting Started](./guides/getting-started.md) | First local setup, first editor, controlled vs uncontrolled mode, and common setup patterns. |
| [Toolbar Setup](./guides/toolbar-setup.md) | Built-in toolbar setup, custom toolbar configuration, standalone toolbar wiring, and action buttons. |
| [Styling Guide](./guides/styling.md) | Content theming, toolbar theming, mention theming, default toolbar fallbacks, and styling examples. |

## Modules

| Document | Description |
| --- | --- |
| [Mentions Guide](./modules/mentions.md) | @-mention addon setup, suggestion configuration, query/selection callbacks, and mention styling. |
| [Collaboration Guide](./modules/collaboration.md) | `useYjsCollaboration()` wiring, `createYjsCollaborationController()` imperative API, options, state types, peer types, retry configuration, encoded state persistence, utility functions, and remote awareness integration. |

## Reference

| Document | Description |
| --- | --- |
| [NativeRichTextEditor Reference](./reference/native-rich-text-editor.md) | `NativeRichTextEditor` props, callback signatures, and `NativeRichTextEditorRef` methods. |
| [EditorToolbar Reference](./reference/editor-toolbar.md) | `EditorToolbar`, `EditorToolbarItem`, built-in toolbar defaults, icon types, and command values. |
| [EditorTheme Reference](./reference/editor-theme.md) | `EditorTheme`, text/list/rule/toolbar theme types, editor surface insets/radius, and default toolbar token values. |
| [Editor State Reference](./reference/editor-state.md) | `Selection`, `ActiveState` (including `markAttrs`), `HistoryState`, `EditorUpdate`, `RenderElement`, and `ListContext`. |
| [Schema Reference](./reference/schemas.md) | `SchemaDefinition`, `NodeSpec`, `MarkSpec`, `AttrSpec`, and the built-in schema presets. |

## Explanations

| Document | Description |
| --- | --- |
| [Architecture Overview](./explanations/architecture.md) | How the React Native layer, native views, render bridges, and Rust core fit together. |
| [Design Decisions](./explanations/design-decisions.md) | Why the package uses typed themes, typed toolbar icons, Rust semantics, and different toolbar implementations by platform. |

## Development

| Document | Description |
| --- | --- |
| [Development Workflow](./development/workflow.md) | Day-to-day commands, rebuild workflow, test entry points, and when to rebuild native artifacts. |
