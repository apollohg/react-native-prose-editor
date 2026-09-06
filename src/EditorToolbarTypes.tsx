import type { HistoryState, ReadonlyActiveState } from './NativeEditorBridge';
import type { EditorToolbarTheme } from './EditorTheme';

/** List kinds a first-class `list` toolbar item can toggle. */
export type EditorToolbarListType = 'bullet_list' | 'ordered_list' | 'bulletList' | 'orderedList';

/** Heading levels the toolbar and `toggleHeading` accept. */
export type EditorToolbarHeadingLevel = 1 | 2 | 3 | 4 | 5 | 6;

/** Commands a `command` toolbar item can run. */
export type EditorToolbarCommand = 'indentList' | 'outdentList' | 'undo' | 'redo';

/**
 * How a group's children are shown: `'expand'` reveals them inline in the
 * toolbar row, `'menu'` opens them in a popover.
 */
export type EditorToolbarGroupPresentation = 'expand' | 'menu';

/**
 * Which region of the toolbar an item sits in. `'start'` and `'end'` pin the
 * item to that edge; `'scroll'` (the default) puts it in the scrolling middle
 * row.
 */
export type EditorToolbarItemPlacement = 'start' | 'scroll' | 'end';

/** Icons the package draws itself, on both platforms. */
export type EditorToolbarDefaultIconId =
    | 'bold'
    | 'italic'
    | 'underline'
    | 'strike'
    | 'link'
    | 'image'
    | 'h1'
    | 'h2'
    | 'h3'
    | 'h4'
    | 'h5'
    | 'h6'
    | 'blockquote'
    | 'bulletList'
    | 'orderedList'
    | 'indentList'
    | 'outdentList'
    | 'lineBreak'
    | 'horizontalRule'
    | 'undo'
    | 'redo';

/** An SF Symbol, used for the iOS half of a `platform` icon. */
export interface EditorToolbarSFSymbolIcon {
    type: 'sfSymbol';
    /** SF Symbol name, e.g. `'bold'`. */
    name: string;
}

/** A Material icon, used for the Android half of a `platform` icon. */
export interface EditorToolbarMaterialIcon {
    type: 'material';
    /** Material icon name, e.g. `'format_bold'`. */
    name: string;
}

/**
 * A toolbar button's icon:
 *
 * - `default` — one of the package's built-in icons.
 * - `glyph` — arbitrary text or an emoji, drawn as-is.
 * - `platform` — per-platform native icons, with `fallbackText` when the
 *   platform cannot resolve the named icon.
 */
export type EditorToolbarIcon =
    | {
          type: 'default';
          id: EditorToolbarDefaultIconId;
      }
    | {
          type: 'glyph';
          text: string;
      }
    | {
          type: 'platform';
          ios?: EditorToolbarSFSymbolIcon;
          android?: EditorToolbarMaterialIcon;
          fallbackText?: string;
      };

/** Visual overrides for one toolbar button. */
export interface EditorToolbarButtonStyle {
    iconSize?: number;
    color?: string;
    backgroundColor?: string;
    activeColor?: string;
    disabledColor?: string;
    activeBackgroundColor?: string;
    disabledBackgroundColor?: string;
    borderRadius?: number;
}

/**
 * A single toolbar button. Every variant takes a `label` (its accessible
 * name), an `icon`, an optional `key` (defaults to something derived from the
 * variant), and an optional `placement`.
 *
 * - `mark` — toggles the named mark; active and enabled state come from the engine.
 * - `link` / `image` — calls the editor's `onRequestLink` / `onRequestImage`.
 * - `heading` — toggles the given heading level.
 * - `blockquote` — toggles blockquote wrapping.
 * - `list` — toggles the given list type.
 * - `command` — runs an {@link EditorToolbarCommand}.
 * - `node` — inserts the named node type, e.g. `'horizontalRule'`.
 * - `action` — host-defined; calls `onToolbarAction` with its `key`, and the
 *   host supplies `isActive`/`isDisabled` since the engine knows nothing about it.
 */
export type EditorToolbarLeafItem = (
    | {
          type: 'mark';
          mark: string;
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
          placement?: EditorToolbarItemPlacement;
      }
    | {
          type: 'link';
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
          placement?: EditorToolbarItemPlacement;
      }
    | {
          type: 'image';
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
          placement?: EditorToolbarItemPlacement;
      }
    | {
          type: 'heading';
          level: EditorToolbarHeadingLevel;
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
          placement?: EditorToolbarItemPlacement;
      }
    | {
          type: 'blockquote';
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
          placement?: EditorToolbarItemPlacement;
      }
    | {
          type: 'list';
          listType: EditorToolbarListType;
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
          placement?: EditorToolbarItemPlacement;
      }
    | {
          type: 'command';
          command: EditorToolbarCommand;
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
          placement?: EditorToolbarItemPlacement;
      }
    | {
          type: 'node';
          nodeType: string;
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
          placement?: EditorToolbarItemPlacement;
      }
    | {
          type: 'action';
          key: string;
          label: string;
          icon: EditorToolbarIcon;
          isActive?: boolean;
          isDisabled?: boolean;
          placement?: EditorToolbarItemPlacement;
      }
) & {
    buttonStyle?: EditorToolbarButtonStyle;
};

/** What a group may contain. Groups do not nest. */
export type EditorToolbarGroupChildItem = EditorToolbarLeafItem;

/**
 * Several buttons collapsed behind one — heading levels, say. The group
 * reports active when any child is active, and disabled when every child is.
 */
export interface EditorToolbarGroupItem {
    type: 'group';
    /** Identity of the group. Required, unlike a leaf item's key. */
    key: string;
    /** Accessible name for the group button. */
    label: string;
    icon: EditorToolbarIcon;
    /** How the children are revealed. Defaults to `'expand'`. */
    presentation?: EditorToolbarGroupPresentation;
    placement?: EditorToolbarItemPlacement;
    buttonStyle?: EditorToolbarButtonStyle;
    items: readonly EditorToolbarGroupChildItem[];
}

/**
 * One entry in `toolbarItems`: a button, a group of buttons, or a separator.
 * The same list drives the JavaScript `EditorToolbar` and the native keyboard
 * toolbar.
 */
export type EditorToolbarItem =
    | EditorToolbarLeafItem
    | EditorToolbarGroupItem
    | {
          type: 'separator';
          key?: string;
          placement?: EditorToolbarItemPlacement;
      };

export interface ToolbarButton {
    key: string;
    label: string;
    icon: EditorToolbarIcon;
    buttonStyle?: EditorToolbarButtonStyle;
    action: () => void;
    isActive?: boolean;
    isDisabled?: boolean;
    groupKey?: string;
    placement: EditorToolbarItemPlacement;
}

export interface ToolbarGroupButton {
    key: string;
    label: string;
    icon: EditorToolbarIcon;
    buttonStyle?: EditorToolbarButtonStyle;
    presentation: EditorToolbarGroupPresentation;
    placement: EditorToolbarItemPlacement;
    children: readonly ToolbarButton[];
    isActive: boolean;
    isDisabled: boolean;
    isExpanded: boolean;
    isOpen: boolean;
}

export interface ToolbarMenuState {
    groupKey: string;
    x: number;
    y: number;
    width: number;
    height: number;
}

export type ToolbarRenderedItem =
    | { type: 'separator'; key: string; placement: EditorToolbarItemPlacement }
    | { type: 'button'; button: ToolbarButton }
    | { type: 'group'; group: ToolbarGroupButton };

export interface EditorToolbarProps {
    /** Currently active marks and nodes from the Rust engine. */
    activeState: ReadonlyActiveState;
    /** Current undo/redo availability. */
    historyState: HistoryState;
    /** Toggle bold mark. */
    onToggleBold: () => void;
    /** Toggle italic mark. */
    onToggleItalic: () => void;
    /** Toggle underline mark. */
    onToggleUnderline: () => void;
    /** Toggle strikethrough mark. */
    onToggleStrike: () => void;
    /** Toggle bullet list. */
    onToggleBulletList?: () => void;
    /** Toggle blockquote wrapping. */
    onToggleBlockquote?: () => void;
    /** Toggle ordered list. */
    onToggleOrderedList?: () => void;
    /** Indent the current list item. */
    onIndentList?: () => void;
    /** Outdent the current list item. */
    onOutdentList?: () => void;
    /** Insert horizontal rule. */
    onInsertHorizontalRule?: () => void;
    /** Insert inline hard break. */
    onInsertLineBreak?: () => void;
    /** Undo the last operation. */
    onUndo: () => void;
    /** Redo the last undone operation. */
    onRedo: () => void;
    /** Generic mark toggle handler used by configurable mark buttons. */
    onToggleMark?: (mark: string) => void;
    /** Generic list toggle handler used by configurable list buttons. */
    onToggleListType?: (listType: EditorToolbarListType) => void;
    /** Generic heading toggle handler used by configurable heading buttons. */
    onToggleHeading?: (level: EditorToolbarHeadingLevel) => void;
    /** Generic node insertion handler used by configurable node buttons. */
    onInsertNodeType?: (nodeType: string) => void;
    /** Generic command handler used by configurable command buttons. */
    onRunCommand?: (command: EditorToolbarCommand) => void;
    /** Generic action handler for arbitrary JS-defined toolbar buttons. */
    onToolbarAction?: (key: string) => void;
    /** Link button handler used by first-class link toolbar items. */
    onRequestLink?: () => void;
    /** Image button handler used by first-class image toolbar items. */
    onRequestImage?: () => void;
    /** Displayed toolbar items, in order. Defaults to the built-in toolbar. */
    toolbarItems?: readonly EditorToolbarItem[];
    /** Optional theme overrides for toolbar chrome and button colors. */
    theme?: EditorToolbarTheme;
    /** Whether to render the built-in top separator line. */
    showTopBorder?: boolean;
    /**
     * Keep RichTextEditor focused when this toolbar is rendered outside
     * the editor wrapper. Defaults to true.
     */
    preserveEditorFocus?: boolean;
}
