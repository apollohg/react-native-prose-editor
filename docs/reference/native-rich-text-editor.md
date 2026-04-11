[Back to docs index](../README.md)

# `NativeRichTextEditor`

## Props

```ts
interface NativeRichTextEditorProps {
  initialContent?: string;
  initialJSON?: DocumentJSON;
  value?: string;
  valueJSON?: DocumentJSON;
  schema?: SchemaDefinition;
  placeholder?: string;
  editable?: boolean;
  maxLength?: number;
  autoFocus?: boolean;
  heightBehavior?: NativeRichTextEditorHeightBehavior;
  showToolbar?: boolean;
  toolbarPlacement?: NativeRichTextEditorToolbarPlacement;
  toolbarItems?: readonly EditorToolbarItem[];
  onToolbarAction?: (key: string) => void;
  onRequestLink?: (context: LinkRequestContext) => void;
  onRequestImage?: (context: ImageRequestContext) => void;
  allowBase64Images?: boolean;
  allowImageResizing?: boolean;
  onContentChange?: (html: string) => void;
  onContentChangeJSON?: (json: DocumentJSON) => void;
  onSelectionChange?: (selection: Selection) => void;
  onActiveStateChange?: (state: ActiveState) => void;
  onHistoryStateChange?: (state: HistoryState) => void;
  onFocus?: () => void;
  onBlur?: () => void;
  style?: StyleProp<ViewStyle>;
  containerStyle?: StyleProp<ViewStyle>;
  theme?: EditorTheme;
  addons?: EditorAddons;
  remoteSelections?: readonly RemoteSelectionDecoration[];
}
```

## Prop Table

| Prop | Type | Default | Description |
| --- | --- | --- | --- |
| `initialContent` | `string` | — | Initial uncontrolled HTML content. |
| `initialJSON` | `DocumentJSON` | — | Initial uncontrolled JSON content. |
| `value` | `string` | — | Controlled HTML content. Highest-priority content source. |
| `valueJSON` | `DocumentJSON` | — | Controlled JSON content. Ignored if `value` is set. In collaboration mode, bind this from `useYjsCollaboration().editorBindings.valueJSON`, not from separate app-owned document state. |
| `schema` | `SchemaDefinition` | `tiptapSchema` | Schema definition passed to the Rust core. |
| `placeholder` | `string` | — | Placeholder text shown when the editor is empty. On native platforms it follows the effective `paragraph` text style for font family, weight, and size. |
| `editable` | `boolean` | `true` | Enables or disables editing. |
| `maxLength` | `number` | — | Character limit enforced by the Rust core. |
| `autoFocus` | `boolean` | `false` | Focuses the editor when the native view first mounts. |
| `heightBehavior` | `'fixed' \| 'autoGrow'` | `'autoGrow'` | `fixed` scrolls internally. `autoGrow` expands the view to fit content, suitable for parent-managed scroll containers. |
| `showToolbar` | `boolean` | `true` | Shows or hides the built-in toolbar. |
| `toolbarPlacement` | `'keyboard' \| 'inline'` | `'keyboard'` | `keyboard` attaches the toolbar as a native keyboard accessory (iOS) or above-keyboard view (Android). `inline` renders the toolbar in React above the editor. |
| `toolbarItems` | `readonly EditorToolbarItem[]` | `DEFAULT_EDITOR_TOOLBAR_ITEMS` | Ordered toolbar button configuration. Built-in items now include blockquote by default. Link and image items are supported, but the package does not show its own URL prompt or file picker. Use `group` items to collapse multiple actions behind one toolbar button. |
| `onToolbarAction` | `(key: string) => void` | — | Callback for `action`-type toolbar items. |
| `onRequestLink` | `(context: LinkRequestContext) => void` | — | Called when a toolbar `link` item is pressed. Use it to collect, edit, or clear the target URL. |
| `onRequestImage` | `(context: ImageRequestContext) => void` | — | Called when a toolbar `image` item is pressed. Use it to launch your own picker or upload flow, then call `insertImage(...)` with the chosen URL or base64 data URI. |
| `allowBase64Images` | `boolean` | `false` | Opt-in support for `data:image/...` sources when inserting images imperatively or parsing HTML. Mirrors Tiptap's `allowBase64` behavior. |
| `allowImageResizing` | `boolean` | `true` | Controls whether selected images expose native drag handles for resizing on iOS and Android. When `false`, images still render and insert normally, but the native resize interaction is disabled. |
| `onContentChange` | `(html: string) => void` | — | Called when the document HTML changes. |
| `onContentChangeJSON` | `(json: DocumentJSON) => void` | — | Called when the document JSON changes. |
| `onSelectionChange` | `(selection: Selection) => void` | — | Called when the selection changes. |
| `onActiveStateChange` | `(state: ActiveState) => void` | — | Called when active marks, nodes, commands, or schema availability change. |
| `onHistoryStateChange` | `(state: HistoryState) => void` | — | Called when undo or redo availability changes. Useful when driving a standalone `EditorToolbar`. |
| `onFocus` | `() => void` | — | Called when the editor gains focus. |
| `onBlur` | `() => void` | — | Called when the editor loses focus. |
| `style` | `StyleProp<ViewStyle>` | — | Style applied to the native editor view itself. Does not affect internal content styling. |
| `containerStyle` | `StyleProp<ViewStyle>` | — | Style applied to the outer React container that wraps the editor and any inline toolbar. |
| `theme` | `EditorTheme` | — | Theme object for content, mentions, and toolbar styling. See [EditorTheme Reference](./editor-theme.md). |
| `addons` | `EditorAddons` | — | Optional addon configuration. Currently supports the mentions addon. See [Mentions Guide](../modules/mentions.md). |
| `remoteSelections` | `readonly RemoteSelectionDecoration[]` | — | Remote awareness selections rendered as native overlays. Used by the collaboration module. |

## Placeholder Behavior

- The native placeholder uses the resolved `paragraph` text style from `theme`, so font family, weight, and size stay aligned with normal empty-paragraph rendering.
- The placeholder still uses the platform hint color rather than the paragraph text color.
- On Android, `heightBehavior="autoGrow"` measures wrapped placeholder lines while the editor is empty, so a multiline placeholder can expand the view before any content is entered.

## `RemoteSelectionDecoration`

```ts
interface RemoteSelectionDecoration {
  clientId: number;
  anchor: number;
  head: number;
  color: string;
  name?: string;
  avatarUrl?: string;
  isFocused?: boolean;
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `clientId` | `number` | Unique client identifier for this remote peer. |
| `anchor` | `number` | Anchor position of the remote selection in the document. |
| `head` | `number` | Head position of the remote selection in the document. |
| `color` | `string` | Color used to render the remote selection highlight and caret. |
| `name` | `string \| undefined` | Display name shown alongside the remote caret. |
| `avatarUrl` | `string \| undefined` | URL of the remote user's avatar image. |
| `isFocused` | `boolean \| undefined` | Whether the remote user's editor is currently focused. |

## Hyperlinks

- Hyperlink application is host-driven. Add a toolbar item with `{ type: 'link', ... }` and handle URL entry in `onRequestLink`.
- The editor will report the current link state through `LinkRequestContext`, including the active `href` when the selection is already inside a link.
- You can also apply or remove links imperatively through the editor ref with `setLink(href)` and `unsetLink()`.

```tsx
const toolbarItems = [
  { type: 'mark', mark: 'bold', label: 'Bold', icon: { type: 'default', id: 'bold' } },
  { type: 'link', label: 'Link', icon: { type: 'default', id: 'link' } },
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

## `LinkRequestContext`

```ts
interface LinkRequestContext {
  href?: string;
  isActive: boolean;
  selection: Selection;
  setLink: (href: string) => void;
  unsetLink: () => void;
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `href` | `string \| undefined` | The current link target when the selection is inside an active link. |
| `isActive` | `boolean` | Whether the current selection is already linked. |
| `selection` | `Selection` | Current editor selection when the link request is triggered. |
| `setLink` | `(href: string) => void` | Apply or update the link on the current selection. |
| `unsetLink` | `() => void` | Remove the link from the current selection. |

## Images

- Add a toolbar item with `{ type: 'image', ... }` and handle picking or uploading in `onRequestImage`.
- Finish the host flow by calling `insertImage(src, attrs?)`.
- `src` can be a remote URL, local file URL, or a `data:image/...` URI when `allowBase64Images` is enabled.
- The built-in schemas already include a block `image` node with `src`, `alt`, `title`, `width`, and `height` attrs.
- On iOS and Android, users can resize selected images with native drag handles. The updated size is written back to `width` and `height`.
- Set `allowImageResizing={false}` if images should stay fixed after insertion.
- You can also insert images imperatively through the editor ref with `insertImage(src, attrs?)`.

```tsx
const toolbarItems = [
  { type: 'mark', mark: 'bold', label: 'Bold', icon: { type: 'default', id: 'bold' } },
  { type: 'image', label: 'Image', icon: { type: 'default', id: 'image' } },
] as const;

<NativeRichTextEditor
  showToolbar
  toolbarItems={toolbarItems}
  allowBase64Images={false}
  onRequestImage={({ insertImage }) => {
    const uploadedUrl = 'https://cdn.example.com/cat.png';
    insertImage(uploadedUrl, { alt: 'Cat', title: 'Hero image' });
  }}
/>;
```

## `ImageRequestContext`

```ts
interface ImageRequestContext {
  selection: Selection;
  allowBase64: boolean;
  insertImage: (
    src: string,
    attrs?: {
      alt?: string | null;
      title?: string | null;
      width?: number | null;
      height?: number | null;
    }
  ) => void;
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `selection` | `Selection` | Current editor selection when the image request is triggered. |
| `allowBase64` | `boolean` | Whether this editor instance accepts `data:image/...` sources. |
| `insertImage` | `(src: string, attrs?: { alt?: string \| null; title?: string \| null; width?: number \| null; height?: number \| null }) => void` | Inserts a block image node at the captured selection. `width` and `height` seed the initial rendered size and are updated when users resize the image natively. |

## Collaboration Usage

- `NativeRichTextEditor` is still the editor component used in collaboration mode.
- The intended collaboration wiring is to pass `valueJSON`, `onContentChangeJSON`, selection/focus callbacks, and `remoteSelections` from `useYjsCollaboration().editorBindings`.
- Do not mix collaboration bindings with a second app-owned controlled `valueJSON` state for the same editor instance.
- Remote awareness cursors should be rendered through `remoteSelections`. They should not be mapped onto the local user selection manually.

See the [Collaboration Guide](../modules/collaboration.md) for the full pattern.

## Height Behavior

```ts
type NativeRichTextEditorHeightBehavior = 'fixed' | 'autoGrow';
```

| Value | Behavior |
| --- | --- |
| `fixed` | The editor has a fixed height and scrolls internally. |
| `autoGrow` | The editor grows vertically to fit its content. Use this when the editor is inside a parent `ScrollView`. This is the default. |

### Keyboard Avoidance Notes

- `autoGrow` is designed for parent-managed scroll containers. If your screen uses a React Native `ScrollView`, `FlatList`, or similar container, you should still use app-level keyboard avoidance such as `KeyboardAvoidingView` or an equivalent screen-level inset strategy.
- In the empty state, auto-grow sizing uses the larger of the content height and placeholder height. This matters most for multiline placeholders on Android.
- `fixed` keeps scrolling inside the native editor. The editor handles its own internal viewport and caret visibility, but your app still needs to ensure the outer screen responds to the keyboard when the editor itself sits low on the page.
- On Android with `toolbarPlacement="keyboard"`, the built-in toolbar renders above the keyboard. The editor reserves that obscured bottom space internally, but that does not replace outer layout avoidance for the rest of the screen.
- If you use `KeyboardAvoidingView` on Android with `toolbarPlacement="keyboard"`, also budget for the native toolbar height in `keyboardVerticalOffset`. The example app currently uses `60` as a practical default for the built-in toolbar.

## Toolbar Placement

```ts
type NativeRichTextEditorToolbarPlacement = 'keyboard' | 'inline';
```

| Value | Behavior |
| --- | --- |
| `keyboard` | The toolbar is attached as a native keyboard accessory (iOS) or rendered above the keyboard (Android). This is the default. |
| `inline` | The toolbar is rendered in React above the editor view. Use this when you need the toolbar visible without the keyboard. |

### Layout Responsibility

- `toolbarPlacement="keyboard"` controls where the formatting toolbar lives. It does not make the surrounding React Native screen keyboard-aware by itself.
- If the editor is embedded in a longer form or settings page, treat keyboard avoidance as a screen concern and editor scrolling/caret visibility as an editor concern.
- On Android, screen-level keyboard avoidance should account for both the IME and the built-in native toolbar. Without an additional offset, the keyboard may avoid correctly while the toolbar still overlaps the focused editor area.

## Ref Methods

```ts
interface NativeRichTextEditorRef {
  focus(): void;
  blur(): void;
  toggleMark(markType: string): void;
  setLink(href: string): void;
  unsetLink(): void;
  toggleBlockquote(): void;
  toggleHeading(level: 1 | 2 | 3 | 4 | 5 | 6): void;
  toggleList(listType: 'bulletList' | 'orderedList'): void;
  indentListItem(): void;
  outdentListItem(): void;
  insertNode(nodeType: string): void;
  insertImage(
    src: string,
    attrs?: {
      alt?: string | null;
      title?: string | null;
      width?: number | null;
      height?: number | null;
    }
  ): void;
  insertText(text: string): void;
  insertContentHtml(html: string): void;
  insertContentJson(doc: DocumentJSON): void;
  setContent(html: string): void;
  setContentJson(doc: DocumentJSON): void;
  getContent(): string;
  getContentJson(): DocumentJSON;
  getTextContent(): string;
  undo(): void;
  redo(): void;
  canUndo(): boolean;
  canRedo(): boolean;
}
```

| Method | Arguments | Returns | Description |
| --- | --- | --- | --- |
| `focus()` | — | `void` | Focuses the editor. |
| `blur()` | — | `void` | Blurs the editor. |
| `toggleMark(markType)` | `markType: string` | `void` | Toggles a mark by schema name. |
| `setLink(href)` | `href: string` | `void` | Apply or update a hyperlink on the current selection. |
| `unsetLink()` | — | `void` | Remove a hyperlink from the current selection. |
| `toggleBlockquote()` | — | `void` | Wrap or unwrap the current block selection in a blockquote. |
| `toggleHeading(level)` | `level: 1 \| 2 \| 3 \| 4 \| 5 \| 6` | `void` | Toggles the current text block selection between the requested heading and `paragraph`. |
| `toggleList(listType)` | `listType: 'bulletList' \| 'orderedList'` | `void` | Toggles a bullet or ordered list. |
| `indentListItem()` | — | `void` | Indents the current list item. |
| `outdentListItem()` | — | `void` | Outdents the current list item. |
| `insertNode(nodeType)` | `nodeType: string` | `void` | Inserts a node by schema name. |
| `insertImage(src, attrs?)` | `src: string`, `attrs?: { alt?: string \| null; title?: string \| null; width?: number \| null; height?: number \| null }` | `void` | Inserts a block image node. Base64 data URIs require `allowBase64Images={true}`. `width` and `height` let hosts seed a preferred size. |
| `insertText(text)` | `text: string` | `void` | Inserts plain text at the current selection. |
| `insertContentHtml(html)` | `html: string` | `void` | Inserts parsed HTML at the current selection. |
| `insertContentJson(doc)` | `doc: DocumentJSON` | `void` | Inserts JSON content at the current selection. |
| `setContent(html)` | `html: string` | `void` | Replaces the entire document with HTML. |
| `setContentJson(doc)` | `doc: DocumentJSON` | `void` | Replaces the entire document with JSON. |
| `getContent()` | — | `string` | Returns the current HTML. |
| `getContentJson()` | — | `DocumentJSON` | Returns the current JSON. |
| `getTextContent()` | — | `string` | Returns the current plain text content. |
| `undo()` | — | `void` | Performs an undo. |
| `redo()` | — | `void` | Performs a redo. |
| `canUndo()` | — | `boolean` | Whether undo is available. |
| `canRedo()` | — | `boolean` | Whether redo is available. |

## Related Docs

- [EditorToolbar Reference](./editor-toolbar.md)
- [EditorTheme Reference](./editor-theme.md)
- [Editor State Reference](./editor-state.md)
- [Schema Reference](./schemas.md)
- [Collaboration Guide](../modules/collaboration.md)
- [Mentions Guide](../modules/mentions.md)
