[Back to docs index](../README.md)

# Styling

## Styling Model

There are two styling entry points:

- `style` for the outer editor container
- `theme` for editor content and toolbar styling

This split is intentional. The content is rendered natively, so React Native stylesheets are not the full styling API for internal paragraphs, list markers, horizontal rules, or the iOS accessory toolbar.

For the design rationale, see [Design Decisions](../explanations/design-decisions.md).

## Why Not A Plain Stylesheet API?

The editor internals are not normal React Native children. They are rendered through native views and a Rust-generated render model.

That means a plain `StyleSheet` cannot cleanly style:

- internal text blocks
- list markers
- horizontal rules
- native-only toolbar chrome

So the package uses:

- `style` for layout and outer chrome
- `theme` for content and toolbar appearance

## Basic Typography Theme

```tsx
const theme = {
  text: {
    fontFamily: 'Georgia',
    fontSize: 17,
    lineHeight: 26,
    color: '#1f2937',
  },
  paragraph: {
    spacingAfter: 12,
  },
};
```

## Editor Plus Toolbar Theme

```tsx
const theme = {
  backgroundColor: '#ffffff',
  borderRadius: 16,
  contentInsets: {
    top: 16,
    right: 16,
    bottom: 16,
    left: 16,
  },
  text: {
    fontFamily: 'Avenir Next',
    fontSize: 16,
    lineHeight: 24,
    color: '#111827',
  },
  list: {
    indent: 22,
    itemSpacing: 6,
    markerColor: '#0f766e',
  },
  toolbar: {
    backgroundColor: '#f8fafc',
    borderColor: '#cbd5e1',
    borderRadius: 14,
    keyboardOffset: 8,
    horizontalInset: 12,
    buttonColor: '#334155',
    buttonActiveColor: '#0f172a',
    buttonDisabledColor: '#94a3b8',
    buttonActiveBackgroundColor: '#e2e8f0',
    buttonBorderRadius: 10,
  },
};
```

## Container Style Plus Theme

```tsx
<NativeRichTextEditor
  style={{ minHeight: 240, borderWidth: 1, borderColor: '#d1d5db' }}
  theme={{
    backgroundColor: '#ffffff',
    text: {
      fontSize: 16,
      lineHeight: 24,
      color: '#111827',
    },
  }}
/>;
```

## Toolbar Styling

Toolbar styling lives under `theme.toolbar`.

Current toolbar tokens:

- `appearance`
- `backgroundColor`
- `borderColor`
- `borderWidth`
- `borderRadius`
- `keyboardOffset`
- `horizontalInset`
- `separatorColor`
- `buttonColor`
- `buttonActiveColor`
- `buttonDisabledColor`
- `buttonActiveBackgroundColor`
- `buttonBorderRadius`

Set `theme.toolbar.appearance` to `'native'` to use platform-native keyboard toolbar chrome instead of the fully custom painted toolbar. In that mode, the native iOS and Android keyboard-hosted toolbars ignore the visual color and radius tokens and keep only the layout tokens such as `keyboardOffset` and `horizontalInset`.

## Editor Container Theme Tokens

These top-level `theme` fields control the native editor surface itself:

- `backgroundColor`
- `borderRadius`
- `contentInsets`

## Toolbar Fallback Defaults

If `theme.toolbar.appearance` is omitted or set to `'custom'`, the built-in toolbar uses these defaults:

| Field | Default |
| --- | --- |
| `appearance` | `custom` |
| `backgroundColor` | `#FFFFFF` |
| `borderColor` | `#E5E5EA` |
| `borderRadius` | `0` |
| `keyboardOffset` | `0` |
| `horizontalInset` | `0` |
| `separatorColor` | `#E5E5EA` |
| `buttonColor` | `#666666` |
| `buttonActiveColor` | `#007AFF` |
| `buttonDisabledColor` | `#C7C7CC` |
| `buttonActiveBackgroundColor` | `rgba(0, 122, 255, 0.12)` |
| `buttonBorderRadius` | `6` |

If `appearance` is set to `native`, the keyboard-hosted toolbar keeps the native placement defaults of `keyboardOffset: 6` and `horizontalInset: 10`, while the chrome itself comes from the platform. On Android that uses a Material 3 docked-toolbar treatment; on iOS 26+, that path depends on the host app allowing the current system design, and `UIDesignRequiresCompatibility` falls back to the legacy non-glass appearance.

## Mention Styling

Mention inline chips and the native mention suggestions are styled through `theme.mentions`. The suggestion UI is rendered in the toolbar area rather than a floating popover. See the [Mentions Guide](../modules/mentions.md) for setup and the [EditorMentionTheme reference](../reference/editor-theme.md#editormentiontheme) for the full token list.

## Related Docs

- [Toolbar Setup](./toolbar-setup.md)
- [Mentions Guide](../modules/mentions.md)
- [EditorTheme Reference](../reference/editor-theme.md)
