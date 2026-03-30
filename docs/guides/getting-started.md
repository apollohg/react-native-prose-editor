[Back to docs index](../README.md)

# Getting Started

## What This Package Is

`@apollohg/react-native-prose-editor` is a native rich text editor for React Native.

It does not render inside a WebView. Instead:

- React Native hosts the public component API
- iOS and Android handle rendering and input natively
- the Rust core owns document structure, transforms, schema validation, and history

## Before You Start

If you have not installed the repo dependencies and example app yet, start with the [Installation Guide](./installation.md).

This package currently requires Expo Modules, and the minimum tested Expo version is SDK 54.

## First Editor

```tsx
import {
  NativeRichTextEditor,
  DEFAULT_EDITOR_TOOLBAR_ITEMS,
  tiptapSchema,
} from '@apollohg/react-native-prose-editor';

export function Example() {
  return (
    <NativeRichTextEditor
      initialContent="<p>Hello world</p>"
      placeholder="Write something..."
      schema={tiptapSchema}
      toolbarItems={DEFAULT_EDITOR_TOOLBAR_ITEMS}
      showToolbar
    />
  );
}
```

## Content Modes

The editor supports both uncontrolled and controlled usage.

### Content Priority

Initialization happens in this order:

| Priority | Prop |
| --- | --- |
| 1 | `value` |
| 2 | `valueJSON` |
| 3 | `initialJSON` |
| 4 | `initialContent` |

### Uncontrolled Mode

Use uncontrolled mode when the editor owns its own working document state.

- `initialContent`
- `initialJSON`

### Controlled Mode

Use controlled mode when your app owns the current document.

- `value`
- `valueJSON`

If both `value` and `valueJSON` are provided, `value` wins.

## Common Setup Patterns

### Simple Built-In Toolbar

```tsx
<NativeRichTextEditor
  initialContent="<p>Hello world</p>"
  showToolbar
  toolbarItems={DEFAULT_EDITOR_TOOLBAR_ITEMS}
/>;
```

### Controlled HTML

```tsx
const [html, setHtml] = useState('<p>Hello</p>');

<NativeRichTextEditor
  value={html}
  onContentChange={setHtml}
/>;
```

### Controlled JSON

```tsx
const [doc, setDoc] = useState<DocumentJSON>({
  type: 'doc',
  content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Hello' }] }],
});

<NativeRichTextEditor
  valueJSON={doc}
  onContentChangeJSON={setDoc}
/>;
```

## Height Behavior

By default, the editor has a fixed height and scrolls internally. To have it grow with content inside a parent `ScrollView`, use `heightBehavior`:

```tsx
<NativeRichTextEditor
  initialContent="<p>Hello</p>"
  heightBehavior="autoGrow"
/>;
```

If that parent screen also avoids the keyboard with `KeyboardAvoidingView`, keep that wrapper in place. On Android, when using `toolbarPlacement="keyboard"`, include a `keyboardVerticalOffset` for the built-in native toolbar as well. The example app currently uses `60`.

## Where To Go Next

- [Installation Guide](./installation.md) for setup and platform prerequisites
- [Toolbar Setup](./toolbar-setup.md) for toolbar configuration patterns
- [Mentions Guide](./mentions.md) for the @-mentions addon
- [Styling Guide](./styling.md) for editor, toolbar, and mention styling
- [NativeRichTextEditor Reference](../reference/native-rich-text-editor.md) for all props and ref methods
