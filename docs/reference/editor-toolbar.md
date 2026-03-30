[Back to docs index](../README.md)

# `EditorToolbar`

## `EditorToolbarProps`

```ts
interface EditorToolbarProps {
  activeState: ActiveState;
  historyState: HistoryState;
  onToggleBold: () => void;
  onToggleItalic: () => void;
  onToggleUnderline: () => void;
  onToggleStrike: () => void;
  onToggleBulletList?: () => void;
  onToggleOrderedList?: () => void;
  onIndentList?: () => void;
  onOutdentList?: () => void;
  onInsertHorizontalRule?: () => void;
  onInsertLineBreak?: () => void;
  onUndo: () => void;
  onRedo: () => void;
  onToggleMark?: (mark: string) => void;
  onToggleListType?: (listType: EditorToolbarListType) => void;
  onInsertNodeType?: (nodeType: string) => void;
  onRunCommand?: (command: EditorToolbarCommand) => void;
  onToolbarAction?: (key: string) => void;
  toolbarItems?: readonly EditorToolbarItem[];
  theme?: EditorToolbarTheme;
  showTopBorder?: boolean;
}
```

## Prop Table

| Prop | Type | Default | Meaning |
| --- | --- | --- | --- |
| `activeState` | `ActiveState` | required | Current active marks, nodes, commands, and schema availability. |
| `historyState` | `HistoryState` | required | Current undo/redo availability. |
| `onToggleBold` | `() => void` | required | Built-in bold toggle handler. |
| `onToggleItalic` | `() => void` | required | Built-in italic toggle handler. |
| `onToggleUnderline` | `() => void` | required | Built-in underline toggle handler. |
| `onToggleStrike` | `() => void` | required | Built-in strikethrough toggle handler. |
| `onToggleBulletList` | `(() => void) \| undefined` | none | Built-in bullet list toggle handler. |
| `onToggleOrderedList` | `(() => void) \| undefined` | none | Built-in ordered list toggle handler. |
| `onIndentList` | `(() => void) \| undefined` | none | Built-in indent handler. |
| `onOutdentList` | `(() => void) \| undefined` | none | Built-in outdent handler. |
| `onInsertHorizontalRule` | `(() => void) \| undefined` | none | Built-in horizontal rule insertion handler. |
| `onInsertLineBreak` | `(() => void) \| undefined` | none | Built-in line break insertion handler. |
| `onUndo` | `() => void` | required | Undo handler. |
| `onRedo` | `() => void` | required | Redo handler. |
| `onToggleMark` | `((mark: string) => void) \| undefined` | none | Generic mark handler for configurable mark buttons. |
| `onToggleListType` | `((listType: EditorToolbarListType) => void) \| undefined` | none | Generic list handler for configurable list buttons. |
| `onInsertNodeType` | `((nodeType: string) => void) \| undefined` | none | Generic node handler for configurable node buttons. |
| `onRunCommand` | `((command: EditorToolbarCommand) => void) \| undefined` | none | Generic command handler for configurable command buttons. |
| `onToolbarAction` | `((key: string) => void) \| undefined` | none | Handler for app-defined action buttons. |
| `toolbarItems` | `readonly EditorToolbarItem[] \| undefined` | `DEFAULT_EDITOR_TOOLBAR_ITEMS` | Ordered toolbar configuration. |
| `theme` | `EditorToolbarTheme \| undefined` | built-in fallback theme | Toolbar styling overrides. |
| `showTopBorder` | `boolean \| undefined` | `true` | Whether the built-in top separator line is rendered. Useful when wrapping the toolbar in your own bordered container. |

## Handler Resolution

| Item Type | Preferred Generic Handler | Built-In Fallback |
| --- | --- | --- |
| `mark` | `onToggleMark(mark)` | `onToggleBold`, `onToggleItalic`, `onToggleUnderline`, `onToggleStrike` |
| `list` | `onToggleListType(listType)` | `onToggleBulletList`, `onToggleOrderedList` |
| `node` | `onInsertNodeType(nodeType)` | `onInsertLineBreak`, `onInsertHorizontalRule` |
| `command` | `onRunCommand(command)` | `onIndentList`, `onOutdentList`, `onUndo`, `onRedo` |
| `action` | `onToolbarAction(key)` | none |

## Default Toolbar Items

| Order | Item Type | Value | Label | Default Icon ID |
| --- | --- | --- | --- | --- |
| 1 | `mark` | `bold` | `Bold` | `bold` |
| 2 | `mark` | `italic` | `Italic` | `italic` |
| 3 | `mark` | `underline` | `Underline` | `underline` |
| 4 | `mark` | `strike` | `Strikethrough` | `strike` |
| 5 | `separator` | none | none | none |
| 6 | `list` | `bulletList` | `Bullet List` | `bulletList` |
| 7 | `list` | `orderedList` | `Ordered List` | `orderedList` |
| 8 | `command` | `indentList` | `Indent List` | `indentList` |
| 9 | `command` | `outdentList` | `Outdent List` | `outdentList` |
| 10 | `node` | `hardBreak` | `Line Break` | `lineBreak` |
| 11 | `node` | `horizontalRule` | `Horizontal Rule` | `horizontalRule` |
| 12 | `separator` | none | none | none |
| 13 | `command` | `undo` | `Undo` | `undo` |
| 14 | `command` | `redo` | `Redo` | `redo` |

## `EditorToolbarItem`

```ts
type EditorToolbarItem =
  | { type: 'mark'; mark: string; label: string; icon: EditorToolbarIcon; key?: string }
  | { type: 'list'; listType: 'bulletList' | 'orderedList'; label: string; icon: EditorToolbarIcon; key?: string }
  | { type: 'command'; command: 'indentList' | 'outdentList' | 'undo' | 'redo'; label: string; icon: EditorToolbarIcon; key?: string }
  | { type: 'node'; nodeType: string; label: string; icon: EditorToolbarIcon; key?: string }
  | { type: 'separator'; key?: string }
  | { type: 'action'; key: string; label: string; icon: EditorToolbarIcon; isActive?: boolean; isDisabled?: boolean };
```

| Variant | Main Fields | Meaning |
| --- | --- | --- |
| `mark` | `mark`, `label`, `icon`, `key?` | Toggles a mark by schema mark name. |
| `list` | `listType`, `label`, `icon`, `key?` | Toggles a bullet or ordered list. |
| `command` | `command`, `label`, `icon`, `key?` | Runs one built-in editor command. |
| `node` | `nodeType`, `label`, `icon`, `key?` | Inserts a node by schema node name. |
| `separator` | `key?` | Visual separator only. |
| `action` | `key`, `label`, `icon`, `isActive?`, `isDisabled?` | App-defined toolbar button routed to `onToolbarAction`. |

## Built-In Command Values

| Command | Meaning |
| --- | --- |
| `indentList` | Indent the current list item. |
| `outdentList` | Outdent the current list item. |
| `undo` | Undo the last change. |
| `redo` | Redo the last undone change. |

## Icon Types

```ts
type EditorToolbarIcon =
  | { type: 'default'; id: EditorToolbarDefaultIconId }
  | { type: 'glyph'; text: string }
  | {
      type: 'platform';
      ios?: { type: 'sfSymbol'; name: string };
      android?: { type: 'material'; name: string };
      fallbackText?: string;
    };
```

| Variant | Fields | Meaning |
| --- | --- | --- |
| `default` | `id` | Package-defined semantic icon choice. |
| `glyph` | `text` | Literal text fallback. |
| `platform` | `ios?`, `android?`, `fallbackText?` | Explicit SF Symbol and Material icon mapping. |

## Default Icon Mapping

| Default Icon ID | Default Glyph | Default Material Icon |
| --- | --- | --- |
| `bold` | `B` | `format-bold` |
| `italic` | `I` | `format-italic` |
| `underline` | `U` | `format-underlined` |
| `strike` | `S` | `strikethrough-s` |
| `bulletList` | `•≡` | `format-list-bulleted` |
| `orderedList` | `1.` | `format-list-numbered` |
| `indentList` | `→` | `format-indent-increase` |
| `outdentList` | `←` | `format-indent-decrease` |
| `lineBreak` | `↵` | `keyboard-return` |
| `horizontalRule` | `—` | `horizontal-rule` |
| `undo` | `↩` | `undo` |
| `redo` | `↪` | `redo` |

## Related Docs

- [Toolbar Setup Guide](../guides/toolbar-setup.md)
- [NativeRichTextEditor Reference](./native-rich-text-editor.md)
- [EditorTheme Reference](./editor-theme.md)
