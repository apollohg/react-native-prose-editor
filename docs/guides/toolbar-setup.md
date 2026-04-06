[Back to docs index](../README.md)

# Toolbar Setup

## Overview

The package supports two toolbar integration modes:

- Use the built-in toolbar through `NativeRichTextEditor` (recommended).
- Render `EditorToolbar` yourself for custom layout or placement.

## Built-In Toolbar

This is the recommended starting point. The toolbar is hosted natively as a keyboard accessory (iOS) or above-keyboard view (Android).

```tsx
<NativeRichTextEditor
  showToolbar
  toolbarItems={DEFAULT_EDITOR_TOOLBAR_ITEMS}
/>;
```

This gives you the default button set, platform-specific hosting, and minimal setup.

If the editor lives inside a larger React Native screen that scrolls, keep app-level keyboard avoidance in place. The built-in keyboard toolbar handles native hosting, but it does not replace `KeyboardAvoidingView` or another parent-level keyboard inset strategy for the rest of your layout.

### Toolbar Placement

Control where the toolbar renders with `toolbarPlacement`:

```tsx
// Native keyboard accessory (default)
<NativeRichTextEditor showToolbar toolbarPlacement="keyboard" />

// Inline React view above the editor
<NativeRichTextEditor showToolbar toolbarPlacement="inline" />
```

| Value | Behavior |
| --- | --- |
| `keyboard` | Attached as a native keyboard accessory (iOS) or above-keyboard view (Android). Default. |
| `inline` | Rendered in React above the editor. Visible even when the keyboard is closed. |

Notes:

- `keyboard` is the preferred mode when you want platform-native toolbar behavior.
- On Android, the editor accounts for the built-in above-keyboard toolbar inside the editor viewport.
- You should still make the surrounding screen keyboard-aware if the editor sits inside a parent `ScrollView` or a longer form.
- If you use `KeyboardAvoidingView` on Android, include a `keyboardVerticalOffset` that covers the built-in toolbar height as well as the IME transition. The example app currently uses `60`.

Example screen wrapper:

```tsx
<KeyboardAvoidingView
  style={{ flex: 1 }}
  behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
  keyboardVerticalOffset={Platform.OS === 'android' ? 60 : 0}
>
  <ScrollView keyboardShouldPersistTaps="handled">
    <NativeRichTextEditor
      showToolbar
      toolbarPlacement="keyboard"
    />
  </ScrollView>
</KeyboardAvoidingView>
```

## Custom Toolbar Items

Use `toolbarItems` to define your own button list while still using the built-in host:

```tsx
const toolbarItems = [
  { type: 'mark', mark: 'bold', label: 'Bold', icon: { type: 'default', id: 'bold' } },
  { type: 'mark', mark: 'italic', label: 'Italic', icon: { type: 'default', id: 'italic' } },
  { type: 'link', label: 'Link', icon: { type: 'default', id: 'link' } },
  { type: 'separator' },
  { type: 'node', nodeType: 'hardBreak', label: 'Line Break', icon: { type: 'default', id: 'lineBreak' } },
  { type: 'node', nodeType: 'horizontalRule', label: 'Horizontal Rule', icon: { type: 'default', id: 'horizontalRule' } },
] as const;

<NativeRichTextEditor
  showToolbar
  toolbarItems={toolbarItems}
  onRequestLink={({ href, setLink, unsetLink }) => {
    const nextHref = prompt('Enter URL', href ?? 'https://');
    if (nextHref == null) return;
    if (nextHref.trim() === '') {
      unsetLink();
      return;
    }
    setLink(nextHref);
  }}
/>;
```

Link buttons are host-driven. The editor does not show a built-in URL prompt. Your app owns the URL entry UI and applies the result through `onRequestLink`.

Image buttons are host-driven too. Handle your picker or upload flow in `onRequestImage`, then call `insertImage(...)`.

## App-Defined Action Buttons

Use an `action` item plus `onToolbarAction` to add buttons with custom behavior:

```tsx
const toolbarItems = [
  { type: 'mark', mark: 'bold', label: 'Bold', icon: { type: 'default', id: 'bold' } },
  {
    type: 'action',
    key: 'insertMention',
    label: 'Mention',
    icon: {
      type: 'platform',
      ios: { type: 'sfSymbol', name: 'at' },
      android: { type: 'material', name: 'alternate-email' },
      fallbackText: '@',
    },
  },
] as const;

<NativeRichTextEditor
  showToolbar
  toolbarItems={toolbarItems}
  onToolbarAction={(key) => {
    if (key === 'insertMention') {
      // your app logic here
    }
  }}
/>;
```

## Custom Mark and Node Buttons

Toolbar items address schema names directly, so custom marks and nodes work by name:

```tsx
const toolbarItems = [
  {
    type: 'mark',
    mark: 'highlight',
    label: 'Highlight',
    icon: {
      type: 'platform',
      ios: { type: 'sfSymbol', name: 'highlighter' },
      android: { type: 'material', name: 'ink-highlighter' },
      fallbackText: 'H',
    },
  },
] as const;
```

This only works if your schema includes the `highlight` mark.

## Hyperlink Controls

Hyperlinks are slightly different from ordinary mark buttons because they need a URL. Use a `link` toolbar item and handle `onRequestLink`:

```tsx
const toolbarItems = [
  { type: 'mark', mark: 'bold', label: 'Bold', icon: { type: 'default', id: 'bold' } },
  { type: 'link', label: 'Link', icon: { type: 'default', id: 'link' } },
] as const;

<NativeRichTextEditor
  showToolbar
  toolbarItems={toolbarItems}
  onRequestLink={({ href, isActive, setLink, unsetLink }) => {
    if (isActive && href) {
      // show your edit/remove UI
    }
    setLink('https://example.com');
  }}
/>;
```

If you need an imperative path, the editor ref also exposes `setLink(href)` and `unsetLink()`.

## Image Controls

Image toolbar items call back into your app. Choose the image however you want, then finish by calling `insertImage(...)`:

```tsx
const toolbarItems = [
  { type: 'mark', mark: 'bold', label: 'Bold', icon: { type: 'default', id: 'bold' } },
  { type: 'image', label: 'Image', icon: { type: 'default', id: 'image' } },
] as const;

<NativeRichTextEditor
  showToolbar
  toolbarItems={toolbarItems}
  onRequestImage={({ allowBase64, insertImage }) => {
    const source = allowBase64
      ? 'data:image/png;base64,AAAA'
      : 'https://cdn.example.com/cat.png';
    insertImage(source, { alt: 'Cat', width: 320, height: 180 });
  }}
/>;
```

If you need imperative insertion, the editor ref also exposes `insertImage(src, attrs?)`. Base64 data URIs require `allowBase64Images={true}`. Optional `width` and `height` attrs seed the initial size and are updated when the user resizes the image natively.

Set `allowImageResizing={false}` if inserted images should stay fixed.

## Standalone Toolbar

Use `EditorToolbar` directly when you need full layout control. This requires wiring up the editor ref and state callbacks manually.

```tsx
const editorRef = useRef<NativeRichTextEditorRef>(null);
const [activeState, setActiveState] = useState<ActiveState>({
  marks: {},
  markAttrs: {},
  nodes: {},
  commands: {},
  allowedMarks: [],
  insertableNodes: [],
});
const [historyState, setHistoryState] = useState<HistoryState>({
  canUndo: false,
  canRedo: false,
});

<>
  <NativeRichTextEditor
    ref={editorRef}
    showToolbar={false}
    onActiveStateChange={setActiveState}
    onHistoryStateChange={setHistoryState}
  />
  <EditorToolbar
    activeState={activeState}
    historyState={historyState}
    onToggleMark={(mark) => editorRef.current?.toggleMark(mark)}
    onToggleListType={(listType) => editorRef.current?.toggleList(listType)}
    onInsertNodeType={(nodeType) => editorRef.current?.insertNode(nodeType)}
    onRequestImage={() => editorRef.current?.insertImage('https://cdn.example.com/cat.png')}
    onRunCommand={(command) => {
      if (command === 'indentList') editorRef.current?.indentListItem();
      if (command === 'outdentList') editorRef.current?.outdentListItem();
    }}
    onUndo={() => editorRef.current?.undo()}
    onRedo={() => editorRef.current?.redo()}
    onToggleBold={() => editorRef.current?.toggleMark('bold')}
    onToggleItalic={() => editorRef.current?.toggleMark('italic')}
    onToggleUnderline={() => editorRef.current?.toggleMark('underline')}
    onToggleStrike={() => editorRef.current?.toggleMark('strike')}
  />
</>;
```

When using the generic handlers (`onToggleMark`, `onToggleListType`, `onInsertNodeType`, `onRunCommand`), they take priority over the specific built-in handlers. The specific handlers (`onToggleBold`, etc.) serve as fallbacks for built-in items when generic handlers are not provided.

## Which Path Should You Use?

| Need | Recommended Path |
| --- | --- |
| Simplest integration | Built-in toolbar via `showToolbar` |
| Custom button list | Built-in toolbar with `toolbarItems` |
| Toolbar visible without keyboard | `toolbarPlacement="inline"` |
| Custom layout or placement | Standalone `EditorToolbar` |
| App-defined buttons | Either path, using `action` items |

## Related Docs

- [Styling Guide](./styling.md) for toolbar styling tokens
- [EditorToolbar Reference](../reference/editor-toolbar.md) for exact toolbar types
- [NativeRichTextEditor Reference](../reference/native-rich-text-editor.md) for all component props
