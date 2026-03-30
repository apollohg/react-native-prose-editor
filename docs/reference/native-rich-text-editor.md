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
  onContentChange?: (html: string) => void;
  onContentChangeJSON?: (json: DocumentJSON) => void;
  onSelectionChange?: (selection: Selection) => void;
  onActiveStateChange?: (state: ActiveState) => void;
  onFocus?: () => void;
  onBlur?: () => void;
  style?: StyleProp<ViewStyle>;
  theme?: EditorTheme;
  addons?: EditorAddons;
}
```

## Prop Table

| Prop | Type | Default | Description |
| --- | --- | --- | --- |
| `initialContent` | `string` | — | Initial uncontrolled HTML content. |
| `initialJSON` | `DocumentJSON` | — | Initial uncontrolled JSON content. |
| `value` | `string` | — | Controlled HTML content. Highest-priority content source. |
| `valueJSON` | `DocumentJSON` | — | Controlled JSON content. Ignored if `value` is set. |
| `schema` | `SchemaDefinition` | `tiptapSchema` | Schema definition passed to the Rust core. |
| `placeholder` | `string` | — | Placeholder text shown when the editor is empty. |
| `editable` | `boolean` | `true` | Enables or disables editing. |
| `maxLength` | `number` | — | Character limit enforced by the Rust core. |
| `autoFocus` | `boolean` | `false` | Focuses the editor when the native view first mounts. |
| `heightBehavior` | `'fixed' \| 'autoGrow'` | `'fixed'` | `fixed` scrolls internally. `autoGrow` expands the view to fit content, suitable for parent-managed scroll containers. |
| `showToolbar` | `boolean` | `true` | Shows or hides the built-in toolbar. |
| `toolbarPlacement` | `'keyboard' \| 'inline'` | `'keyboard'` | `keyboard` attaches the toolbar as a native keyboard accessory (iOS) or above-keyboard view (Android). `inline` renders the toolbar in React above the editor. |
| `toolbarItems` | `readonly EditorToolbarItem[]` | `DEFAULT_EDITOR_TOOLBAR_ITEMS` | Ordered toolbar button configuration. |
| `onToolbarAction` | `(key: string) => void` | — | Callback for `action`-type toolbar items. |
| `onContentChange` | `(html: string) => void` | — | Called when the document HTML changes. |
| `onContentChangeJSON` | `(json: DocumentJSON) => void` | — | Called when the document JSON changes. |
| `onSelectionChange` | `(selection: Selection) => void` | — | Called when the selection changes. |
| `onActiveStateChange` | `(state: ActiveState) => void` | — | Called when active marks, nodes, commands, or schema availability change. |
| `onFocus` | `() => void` | — | Called when the editor gains focus. |
| `onBlur` | `() => void` | — | Called when the editor loses focus. |
| `style` | `StyleProp<ViewStyle>` | — | Style applied to the outer native view container. Does not affect internal content styling. |
| `theme` | `EditorTheme` | — | Theme object for content, mentions, and toolbar styling. See [EditorTheme Reference](./editor-theme.md). |
| `addons` | `EditorAddons` | — | Optional addon configuration. Currently supports the mentions addon. See [Mentions Guide](../guides/mentions.md). |

## Height Behavior

```ts
type NativeRichTextEditorHeightBehavior = 'fixed' | 'autoGrow';
```

| Value | Behavior |
| --- | --- |
| `fixed` | The editor has a fixed height and scrolls internally. This is the default. |
| `autoGrow` | The editor grows vertically to fit its content. Use this when the editor is inside a parent `ScrollView`. |

### Keyboard Avoidance Notes

- `autoGrow` is designed for parent-managed scroll containers. If your screen uses a React Native `ScrollView`, `FlatList`, or similar container, you should still use app-level keyboard avoidance such as `KeyboardAvoidingView` or an equivalent screen-level inset strategy.
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
  toggleList(listType: 'bulletList' | 'orderedList'): void;
  indentListItem(): void;
  outdentListItem(): void;
  insertNode(nodeType: string): void;
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
| `toggleList(listType)` | `listType: 'bulletList' \| 'orderedList'` | `void` | Toggles a bullet or ordered list. |
| `indentListItem()` | — | `void` | Indents the current list item. |
| `outdentListItem()` | — | `void` | Outdents the current list item. |
| `insertNode(nodeType)` | `nodeType: string` | `void` | Inserts a node by schema name. |
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
- [Mentions Guide](../guides/mentions.md)
