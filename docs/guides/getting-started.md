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

For whole-document JSON loads, an empty root doc like `{ type: 'doc', content: [] }` is normalized to a schema-valid empty text block for the active schema. That applies to `initialJSON`, controlled `valueJSON`, and imperative whole-document replacement APIs such as `setContentJson()`. Fragment insertion APIs such as `insertContentJson()` still use the content you pass through unchanged.

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

### Collaboration

In collaboration mode, do not treat `valueJSON` like ordinary app-owned controlled state.

Instead, bind the editor directly to `useYjsCollaboration()`:

```tsx
const collaboration = useYjsCollaboration({
  documentId: 'doc-123',
  createWebSocket: () => new WebSocket('wss://example.com/yjs/doc-123'),
  localAwareness: {
    userId: 'u1',
    name: 'Jayden',
    color: '#0A84FF',
  },
});

<NativeRichTextEditor
  valueJSON={collaboration.editorBindings.valueJSON}
  onContentChangeJSON={collaboration.editorBindings.onContentChangeJSON}
  onSelectionChange={collaboration.editorBindings.onSelectionChange}
  onFocus={collaboration.editorBindings.onFocus}
  onBlur={collaboration.editorBindings.onBlur}
  remoteSelections={collaboration.editorBindings.remoteSelections}
/>;
```

Do not keep a second app-level JSON document state and feed that into `valueJSON` at the same time. In collaboration mode, the collaboration controller is the source of truth.

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
- [Collaboration Guide](../modules/collaboration.md) for the correct Yjs binding and persistence model
- [Toolbar Setup](./toolbar-setup.md) for toolbar configuration patterns
- [Mentions Guide](../modules/mentions.md) for the @-mentions addon
- [Styling Guide](./styling.md) for editor, toolbar, and mention styling
- [NativeRichTextEditor Reference](../reference/native-rich-text-editor.md) for all props and ref methods
