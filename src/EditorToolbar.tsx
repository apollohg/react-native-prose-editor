import { MaterialIcons } from '@expo/vector-icons';
import React, {
    createContext,
    useCallback,
    useContext,
    useEffect,
    useMemo,
    useRef,
    useSyncExternalStore,
    useState,
} from 'react';
import {
    Keyboard,
    Modal,
    Pressable,
    ScrollView,
    StyleSheet,
    Text,
    TouchableOpacity,
    View,
    useWindowDimensions,
} from 'react-native';

import type { HistoryState, ReadonlyActiveState } from './NativeEditorBridge';
import type { EditorMentionTheme, EditorToolbarTheme } from './EditorTheme';
import type { MentionSuggestion } from './addons';

/** List kinds a first-class `list` toolbar item can toggle. */
export type EditorToolbarListType = 'bulletList' | 'orderedList';
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

interface ToolbarButton {
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

interface ToolbarGroupButton {
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

interface ToolbarMenuState {
    groupKey: string;
    x: number;
    y: number;
    width: number;
    height: number;
}

export interface EditorToolbarFrame {
    x: number;
    y: number;
    width: number;
    height: number;
}

type EditorToolbarFrameListener = () => void;

interface EditorToolbarFrameRegistration {
    ownerId: number | null;
    frame: EditorToolbarFrame;
}

const editorToolbarFrames = new Map<number, EditorToolbarFrameRegistration>();
const editorToolbarFrameListeners = new Set<EditorToolbarFrameListener>();
const editorToolbarMentionStateListeners = new Set<EditorToolbarFrameListener>();
let nextEditorToolbarRegistrationId = 1;
let activeEditorToolbarInteractions = 0;
let editorToolbarFocusPreserveUntil = 0;
let activeEditorToolbarFrameOwnerId: number | null = null;
const EditorToolbarFrameOwnerContext = createContext<number | null>(null);

export function EditorToolbarFrameOwnerProvider({
    ownerId,
    children,
}: {
    ownerId: number;
    children: React.ReactNode;
}) {
    return (
        <EditorToolbarFrameOwnerContext.Provider value={ownerId}>
            {children}
        </EditorToolbarFrameOwnerContext.Provider>
    );
}

interface EditorToolbarMentionState {
    ownerId: number;
    trigger: string;
    suggestions: readonly MentionSuggestion[];
    theme?: EditorMentionTheme;
    suggestionThemes?: Readonly<Record<string, EditorMentionTheme | undefined>>;
    onSelectSuggestion: (suggestion: MentionSuggestion) => void;
}

let editorToolbarMentionState: EditorToolbarMentionState | null = null;
const EDITOR_TOOLBAR_FOCUS_PRESERVE_MS = 750;

function areToolbarFramesEqual(
    left: EditorToolbarFrame | undefined,
    right: EditorToolbarFrame | undefined
): boolean {
    return (
        left?.x === right?.x &&
        left?.y === right?.y &&
        left?.width === right?.width &&
        left?.height === right?.height
    );
}

function areToolbarFrameListsEqual(
    left: readonly EditorToolbarFrame[],
    right: readonly EditorToolbarFrame[]
): boolean {
    if (left.length !== right.length) {
        return false;
    }
    return left.every((frame, index) => areToolbarFramesEqual(frame, right[index]));
}

function notifyEditorToolbarFrameListeners() {
    editorToolbarFrameListeners.forEach((listener) => listener());
}

function notifyEditorToolbarMentionStateListeners() {
    editorToolbarMentionStateListeners.forEach((listener) => listener());
}

function getEditorToolbarFramesSnapshot(ownerId: number): EditorToolbarFrame[] {
    return Array.from(editorToolbarFrames.values())
        .filter(
            (registration) =>
                registration.ownerId === ownerId ||
                (registration.ownerId == null && activeEditorToolbarFrameOwnerId === ownerId)
        )
        .map((registration) => registration.frame);
}

function subscribeEditorToolbarMentionState(listener: EditorToolbarFrameListener) {
    editorToolbarMentionStateListeners.add(listener);
    return () => {
        editorToolbarMentionStateListeners.delete(listener);
    };
}

function getEditorToolbarMentionStateSnapshot(): EditorToolbarMentionState | null {
    return editorToolbarMentionState;
}

function useEditorToolbarMentionState(): EditorToolbarMentionState | null {
    return useSyncExternalStore(
        subscribeEditorToolbarMentionState,
        getEditorToolbarMentionStateSnapshot,
        getEditorToolbarMentionStateSnapshot
    );
}

function registerEditorToolbarFrame(
    id: number,
    frame: EditorToolbarFrame | null,
    ownerId: number | null
) {
    if (frame == null || frame.width <= 0 || frame.height <= 0) {
        if (editorToolbarFrames.delete(id)) {
            notifyEditorToolbarFrameListeners();
        }
        return;
    }

    const currentRegistration = editorToolbarFrames.get(id);
    if (
        currentRegistration?.ownerId === ownerId &&
        areToolbarFramesEqual(currentRegistration.frame, frame)
    ) {
        return;
    }

    editorToolbarFrames.set(id, { ownerId, frame });
    notifyEditorToolbarFrameListeners();
}

function unregisterEditorToolbarFrame(id: number) {
    if (editorToolbarFrames.delete(id)) {
        notifyEditorToolbarFrameListeners();
    }
}

function preserveEditorToolbarFocusForNextBlur() {
    editorToolbarFocusPreserveUntil = Date.now() + EDITOR_TOOLBAR_FOCUS_PRESERVE_MS;
}

function beginEditorToolbarInteraction() {
    activeEditorToolbarInteractions += 1;
    preserveEditorToolbarFocusForNextBlur();
}

function endEditorToolbarInteraction() {
    activeEditorToolbarInteractions = Math.max(0, activeEditorToolbarInteractions - 1);
    preserveEditorToolbarFocusForNextBlur();
}

export function isEditorToolbarFocusPreservationActive(): boolean {
    return activeEditorToolbarInteractions > 0 || Date.now() <= editorToolbarFocusPreserveUntil;
}

export function setActiveEditorToolbarFrameOwnerForEditor(ownerId: number, isActive: boolean) {
    const nextOwnerId = isActive
        ? ownerId
        : activeEditorToolbarFrameOwnerId === ownerId
          ? null
          : activeEditorToolbarFrameOwnerId;
    if (activeEditorToolbarFrameOwnerId === nextOwnerId) {
        return;
    }
    activeEditorToolbarFrameOwnerId = nextOwnerId;
    notifyEditorToolbarFrameListeners();
}

export function useEditorToolbarFrames(ownerId: number): readonly EditorToolbarFrame[] {
    const [frames, setFrames] = useState<EditorToolbarFrame[]>(() =>
        getEditorToolbarFramesSnapshot(ownerId)
    );

    useEffect(() => {
        const listener = () => {
            const nextFrames = getEditorToolbarFramesSnapshot(ownerId);
            setFrames((currentFrames) =>
                areToolbarFrameListsEqual(currentFrames, nextFrames) ? currentFrames : nextFrames
            );
        };
        editorToolbarFrameListeners.add(listener);
        listener();
        return () => {
            editorToolbarFrameListeners.delete(listener);
        };
    }, [ownerId]);

    return frames;
}

export function setEditorToolbarMentionState(
    ownerId: number,
    state: Omit<EditorToolbarMentionState, 'ownerId'> | null
) {
    if (state == null) {
        if (editorToolbarMentionState?.ownerId !== ownerId) {
            return;
        }
        editorToolbarMentionState = null;
        notifyEditorToolbarMentionStateListeners();
        return;
    }

    editorToolbarMentionState = {
        ownerId,
        ...state,
    };
    notifyEditorToolbarMentionStateListeners();
}

export function _setEditorToolbarFrameForTests(
    id: number,
    frame: EditorToolbarFrame | null,
    ownerId: number | null = null
) {
    registerEditorToolbarFrame(id, frame, ownerId);
}

export function _resetEditorToolbarFrameRegistryForTests() {
    editorToolbarFrames.clear();
    editorToolbarMentionState = null;
    activeEditorToolbarInteractions = 0;
    editorToolbarFocusPreserveUntil = 0;
    activeEditorToolbarFrameOwnerId = null;
    notifyEditorToolbarFrameListeners();
    notifyEditorToolbarMentionStateListeners();
}

export function _beginEditorToolbarInteractionForTests() {
    beginEditorToolbarInteraction();
}

export function _endEditorToolbarInteractionForTests() {
    endEditorToolbarInteraction();
}

type ToolbarRenderedItem =
    | { type: 'separator'; key: string; placement: EditorToolbarItemPlacement }
    | { type: 'button'; button: ToolbarButton }
    | { type: 'group'; group: ToolbarGroupButton };

function defaultIcon(id: EditorToolbarDefaultIconId): EditorToolbarIcon {
    return { type: 'default', id };
}

function resolveToolbarItemPlacement(
    placement: EditorToolbarItemPlacement | undefined
): EditorToolbarItemPlacement {
    return placement ?? 'scroll';
}

/**
 * The toolbar layout used when `toolbarItems` is not supplied. Spread it to
 * extend the built-in set rather than restating it:
 * `[...DEFAULT_EDITOR_TOOLBAR_ITEMS, myItem]`.
 */
export const DEFAULT_EDITOR_TOOLBAR_ITEMS: readonly EditorToolbarItem[] = [
    { type: 'mark', mark: 'bold', label: 'Bold', icon: defaultIcon('bold') },
    { type: 'mark', mark: 'italic', label: 'Italic', icon: defaultIcon('italic') },
    { type: 'mark', mark: 'underline', label: 'Underline', icon: defaultIcon('underline') },
    { type: 'mark', mark: 'strike', label: 'Strikethrough', icon: defaultIcon('strike') },
    { type: 'blockquote', label: 'Blockquote', icon: defaultIcon('blockquote') },
    { type: 'separator' },
    { type: 'list', listType: 'bulletList', label: 'Bullet List', icon: defaultIcon('bulletList') },
    {
        type: 'list',
        listType: 'orderedList',
        label: 'Ordered List',
        icon: defaultIcon('orderedList'),
    },
    {
        type: 'command',
        command: 'indentList',
        label: 'Indent List',
        icon: defaultIcon('indentList'),
    },
    {
        type: 'command',
        command: 'outdentList',
        label: 'Outdent List',
        icon: defaultIcon('outdentList'),
    },
    { type: 'node', nodeType: 'hardBreak', label: 'Line Break', icon: defaultIcon('lineBreak') },
    {
        type: 'node',
        nodeType: 'horizontalRule',
        label: 'Horizontal Rule',
        icon: defaultIcon('horizontalRule'),
    },
    { type: 'separator' },
    { type: 'command', command: 'undo', label: 'Undo', icon: defaultIcon('undo') },
    { type: 'command', command: 'redo', label: 'Redo', icon: defaultIcon('redo') },
] as const;

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
     * Keep NativeRichTextEditor focused when this toolbar is rendered outside
     * the editor wrapper. Defaults to true.
     */
    preserveEditorFocus?: boolean;
}

const BUTTON_HIT = 44;
const BUTTON_VISIBLE = 32;
const TOOLBAR_PADDING_H = 12;
const TOOLBAR_PADDING_V = 4;
const MAX_BUTTON_SIZE = 40;
const BUTTON_HEIGHT_INSET = 4;
const MENU_MARGIN = 8;
const MENU_WIDTH = 192;
const KEYBOARD_FRAME_REMEASURE_DELAYS_MS = [50, 150, 300] as const;

const ACTIVE_BG = 'rgba(0, 122, 255, 0.12)';
const ACTIVE_COLOR = '#007AFF';
const DEFAULT_COLOR = '#666666';
const DISABLED_COLOR = '#C7C7CC';
const SEPARATOR_COLOR = '#E5E5EA';
const TOOLBAR_BG = '#FFFFFF';
const TOOLBAR_BORDER = '#E5E5EA';
const TOOLBAR_RADIUS = 0;
const BUTTON_RADIUS = 6;
const MENU_BORDER = '#D1D1D6';
const MENU_SHADOW = '#000000';

const DEFAULT_GLYPH_ICONS: Record<EditorToolbarDefaultIconId, string> = {
    bold: 'B',
    italic: 'I',
    underline: 'U',
    strike: 'S',
    link: '🔗',
    image: '🖼',
    blockquote: '❝',
    h1: 'H1',
    h2: 'H2',
    h3: 'H3',
    h4: 'H4',
    h5: 'H5',
    h6: 'H6',
    bulletList: '•≡',
    orderedList: '1.',
    indentList: '→',
    outdentList: '←',
    lineBreak: '↵',
    horizontalRule: '—',
    undo: '↩',
    redo: '↪',
};

const DEFAULT_MATERIAL_ICONS: Partial<Record<EditorToolbarDefaultIconId, string>> = {
    bold: 'format-bold',
    italic: 'format-italic',
    underline: 'format-underlined',
    strike: 'strikethrough-s',
    link: 'link',
    image: 'image',
    h1: 'title',
    h2: 'title',
    h3: 'title',
    h4: 'title',
    h5: 'title',
    h6: 'title',
    blockquote: 'format-quote',
    bulletList: 'format-list-bulleted',
    orderedList: 'format-list-numbered',
    indentList: 'format-indent-increase',
    outdentList: 'format-indent-decrease',
    lineBreak: 'keyboard-return',
    horizontalRule: 'horizontal-rule',
    undo: 'undo',
    redo: 'redo',
};

function resolveMentionSuggestionDisplayLabel(
    suggestion: MentionSuggestion,
    trigger: string
): string {
    const label = suggestion.label?.trim() || suggestion.title;
    return trigger.length > 0 && !label.startsWith(trigger) ? `${trigger}${label}` : label;
}

/**
 * A JavaScript formatting toolbar. `NativeRichTextEditor` renders one for you
 * when `showToolbar` is set — reach for this component directly only to place
 * the toolbar somewhere the editor cannot, in which case wire every handler
 * to the editor's ref and feed it `activeState` and `historyState` from the
 * editor's callbacks.
 *
 * The native keyboard-attached toolbar (`toolbarPlacement="keyboard"`) is a
 * separate implementation that consumes the same {@link EditorToolbarItem}
 * list.
 */
export function EditorToolbar({
    activeState,
    historyState,
    onToggleBold,
    onToggleItalic,
    onToggleUnderline,
    onToggleStrike,
    onToggleBulletList,
    onToggleHeading,
    onToggleBlockquote,
    onToggleOrderedList,
    onIndentList,
    onOutdentList,
    onInsertHorizontalRule,
    onInsertLineBreak,
    onUndo,
    onRedo,
    onToggleMark,
    onToggleListType,
    onInsertNodeType,
    onRunCommand,
    onToolbarAction,
    onRequestLink,
    onRequestImage,
    toolbarItems = DEFAULT_EDITOR_TOOLBAR_ITEMS,
    theme,
    showTopBorder,
    preserveEditorFocus = true,
}: EditorToolbarProps) {
    const marks = activeState.marks ?? {};
    const nodes = activeState.nodes ?? {};
    const commands = activeState.commands ?? {};
    const allowedMarks = activeState.allowedMarks ?? [];
    const insertableNodes = activeState.insertableNodes ?? [];
    const frameOwnerId = useContext(EditorToolbarFrameOwnerContext);
    const publishesFocusFrames = preserveEditorFocus || frameOwnerId != null;
    const rootRef = useRef<View | null>(null);
    const menuCardRef = useRef<View | null>(null);
    const groupButtonRefs = useRef(new Map<string, View | null>());
    const { width: windowWidth, height: windowHeight } = useWindowDimensions();
    const [expandedGroupKey, setExpandedGroupKey] = useState<string | null>(null);
    const [menuState, setMenuState] = useState<ToolbarMenuState | null>(null);
    const frameOwnerIdRef = useRef(frameOwnerId);
    frameOwnerIdRef.current = frameOwnerId;
    const publishesFocusFramesRef = useRef(publishesFocusFrames);
    publishesFocusFramesRef.current = publishesFocusFrames;
    const menuStateRef = useRef(menuState);
    menuStateRef.current = menuState;
    const framePublisherMountedRef = useRef(false);
    const mentionState = useEditorToolbarMentionState();
    const toolbarInteractionActiveRef = useRef(false);
    const framePublishAnimationFramesRef = useRef<number[]>([]);
    const framePublishTimeoutsRef = useRef<ReturnType<typeof setTimeout>[]>([]);
    const registrationIdRef = useRef<number | null>(null);
    const menuRegistrationIdRef = useRef<number | null>(null);
    if (registrationIdRef.current == null) {
        registrationIdRef.current = nextEditorToolbarRegistrationId++;
    }
    if (menuRegistrationIdRef.current == null) {
        menuRegistrationIdRef.current = nextEditorToolbarRegistrationId++;
    }

    useEffect(() => {
        framePublisherMountedRef.current = true;
        return () => {
            framePublisherMountedRef.current = false;
        };
    }, []);

    const isMarkActive = useCallback((mark: string) => !!marks[mark], [marks]);

    const canIndentList = !!commands['indentList'];
    const canOutdentList = !!commands['outdentList'];
    const shouldRenderMentionSuggestions =
        publishesFocusFrames && mentionState != null && mentionState.suggestions.length > 0;

    const getActionForItem = useCallback(
        (item: EditorToolbarLeafItem): (() => void) | null => {
            switch (item.type) {
                case 'mark':
                    if (onToggleMark) {
                        return () => onToggleMark(item.mark);
                    }
                    switch (item.mark) {
                        case 'bold':
                            return onToggleBold;
                        case 'italic':
                            return onToggleItalic;
                        case 'underline':
                            return onToggleUnderline;
                        case 'strike':
                            return onToggleStrike;
                        default:
                            return null;
                    }
                case 'list':
                    if (onToggleListType) {
                        return () => onToggleListType(item.listType);
                    }
                    return item.listType === 'bulletList'
                        ? (onToggleBulletList ?? null)
                        : (onToggleOrderedList ?? null);
                case 'link':
                    return onRequestLink ?? null;
                case 'image':
                    return onRequestImage ?? null;
                case 'heading':
                    return onToggleHeading ? () => onToggleHeading(item.level) : null;
                case 'blockquote':
                    return onToggleBlockquote ?? null;
                case 'node':
                    if (onInsertNodeType) {
                        return () => onInsertNodeType(item.nodeType);
                    }
                    switch (item.nodeType) {
                        case 'hardBreak':
                            return onInsertLineBreak ?? null;
                        case 'horizontalRule':
                            return onInsertHorizontalRule ?? null;
                        default:
                            return null;
                    }
                case 'command':
                    if (onRunCommand) {
                        return () => onRunCommand(item.command);
                    }
                    switch (item.command) {
                        case 'indentList':
                            return onIndentList ?? null;
                        case 'outdentList':
                            return onOutdentList ?? null;
                        case 'undo':
                            return onUndo;
                        case 'redo':
                            return onRedo;
                    }
                case 'action':
                    return onToolbarAction ? () => onToolbarAction(item.key) : null;
            }
        },
        [
            onIndentList,
            onInsertHorizontalRule,
            onInsertLineBreak,
            onInsertNodeType,
            onOutdentList,
            onRedo,
            onRunCommand,
            onRequestImage,
            onRequestLink,
            onToggleBlockquote,
            onToggleBold,
            onToggleBulletList,
            onToggleHeading,
            onToggleItalic,
            onToggleListType,
            onToggleMark,
            onToggleOrderedList,
            onToggleStrike,
            onToggleUnderline,
            onToolbarAction,
            onUndo,
        ]
    );

    const makeButtonKey = useCallback(
        (item: EditorToolbarLeafItem, index: number, prefix = '') =>
            item.key != null
                ? `${prefix}${item.key}`
                : item.type === 'mark'
                  ? `${prefix}mark:${item.mark}:${index}`
                  : item.type === 'link'
                    ? `${prefix}link:${index}`
                    : item.type === 'image'
                      ? `${prefix}image:${index}`
                      : item.type === 'heading'
                        ? `${prefix}heading:${item.level}:${index}`
                        : item.type === 'blockquote'
                          ? `${prefix}blockquote:${index}`
                          : item.type === 'list'
                            ? `${prefix}list:${item.listType}:${index}`
                            : item.type === 'command'
                              ? `${prefix}command:${item.command}:${index}`
                              : item.type === 'node'
                                ? `${prefix}node:${item.nodeType}:${index}`
                                : `${prefix}action:${item.key}:${index}`,
        []
    );

    const resolveButton = useCallback(
        (
            item: EditorToolbarLeafItem,
            index: number,
            prefix = '',
            groupKey?: string,
            placement: EditorToolbarItemPlacement = 'scroll'
        ): ToolbarButton | null => {
            const resolvedPlacement = resolveToolbarItemPlacement(item.placement ?? placement);
            const action = getActionForItem(item);
            if (!action) {
                return null;
            }

            let isActive = false;
            let isDisabled = false;
            switch (item.type) {
                case 'mark':
                    isActive = isMarkActive(item.mark);
                    isDisabled = !allowedMarks.includes(item.mark);
                    break;
                case 'link':
                    isActive = isMarkActive('link');
                    isDisabled = !allowedMarks.includes('link') || !onRequestLink;
                    break;
                case 'image':
                    isDisabled = !insertableNodes.includes('image') || !onRequestImage;
                    break;
                case 'heading': {
                    const headingNodeType = `h${item.level}`;
                    isActive = !!nodes[headingNodeType];
                    isDisabled = !commands[`toggleHeading${item.level}`];
                    break;
                }
                case 'blockquote':
                    isActive = !!nodes['blockquote'];
                    isDisabled = !commands['toggleBlockquote'];
                    break;
                case 'list':
                    isActive = !!nodes[item.listType];
                    isDisabled =
                        !commands[
                            item.listType === 'bulletList' ? 'wrapBulletList' : 'wrapOrderedList'
                        ];
                    break;
                case 'command':
                    switch (item.command) {
                        case 'indentList':
                            isDisabled = !canIndentList;
                            break;
                        case 'outdentList':
                            isDisabled = !canOutdentList;
                            break;
                        case 'undo':
                            isDisabled = !historyState.canUndo;
                            break;
                        case 'redo':
                            isDisabled = !historyState.canRedo;
                            break;
                    }
                    break;
                case 'action':
                    isActive = !!item.isActive;
                    isDisabled = !!item.isDisabled || !onToolbarAction;
                    break;
                case 'node':
                    isActive = !!nodes[item.nodeType];
                    isDisabled = !insertableNodes.includes(item.nodeType);
                    break;
            }

            return {
                key: makeButtonKey(item, index, prefix),
                label: item.label,
                icon: item.icon,
                buttonStyle: item.buttonStyle,
                action,
                isActive,
                isDisabled,
                groupKey,
                placement: resolvedPlacement,
            };
        },
        [
            allowedMarks,
            canIndentList,
            canOutdentList,
            commands,
            getActionForItem,
            historyState.canRedo,
            historyState.canUndo,
            insertableNodes,
            isMarkActive,
            makeButtonKey,
            nodes,
            onRequestImage,
            onRequestLink,
            onToolbarAction,
        ]
    );

    const compactRenderedItems = (entries: ToolbarRenderedItem[]): ToolbarRenderedItem[] =>
        entries.filter((entry, index, list) => {
            if (entry.type !== 'separator') {
                return true;
            }
            const previous = list[index - 1];
            const next = list[index + 1];
            return (
                previous != null &&
                previous.type !== 'separator' &&
                next != null &&
                next.type !== 'separator'
            );
        });

    const { startItems, scrollItems, endItems, groupsByKey } = useMemo(() => {
        const startEntries: ToolbarRenderedItem[] = [];
        const scrollEntries: ToolbarRenderedItem[] = [];
        const endEntries: ToolbarRenderedItem[] = [];
        const nextGroups = new Map<string, ToolbarGroupButton>();
        const entriesForPlacement = (placement: EditorToolbarItemPlacement) =>
            placement === 'start' ? startEntries : placement === 'end' ? endEntries : scrollEntries;

        for (let index = 0; index < toolbarItems.length; index += 1) {
            const item = toolbarItems[index];
            const placement = resolveToolbarItemPlacement(item.placement);
            const targetEntries = entriesForPlacement(placement);
            if (item.type === 'separator') {
                targetEntries.push({
                    type: 'separator',
                    key: item.key ?? `separator:${index}`,
                    placement,
                });
                continue;
            }

            if (item.type === 'group') {
                const children = item.items
                    .map((child, childIndex) =>
                        resolveButton(child, childIndex, `${item.key}:`, item.key, placement)
                    )
                    .filter((child): child is ToolbarButton => child != null);
                if (children.length === 0) {
                    continue;
                }
                const presentation = item.presentation ?? 'expand';
                const isExpanded = presentation === 'expand' && expandedGroupKey === item.key;
                const isMenuOpen = presentation === 'menu' && menuState?.groupKey === item.key;
                const group: ToolbarGroupButton = {
                    key: item.key,
                    label: item.label,
                    icon: item.icon,
                    buttonStyle: item.buttonStyle,
                    presentation,
                    placement,
                    children,
                    isActive: children.some((child) => child.isActive) || isExpanded || isMenuOpen,
                    isDisabled: children.every((child) => child.isDisabled),
                    isExpanded,
                    isOpen: isExpanded || isMenuOpen,
                };
                nextGroups.set(group.key, group);
                targetEntries.push({ type: 'group', group });
                if (group.isExpanded) {
                    for (const child of children) {
                        entriesForPlacement(child.placement).push({
                            type: 'button',
                            button: child,
                        });
                    }
                }
                continue;
            }

            const button = resolveButton(item, index, '', undefined, placement);
            if (button) {
                targetEntries.push({ type: 'button', button });
            }
        }

        return {
            startItems: compactRenderedItems(startEntries),
            scrollItems: compactRenderedItems(scrollEntries),
            endItems: compactRenderedItems(endEntries),
            groupsByKey: nextGroups,
        };
    }, [expandedGroupKey, menuState?.groupKey, resolveButton, toolbarItems]);

    const resolvedShowTopBorder = showTopBorder ?? theme?.showTopBorder ?? true;
    const publishToolbarFrame = useCallback(() => {
        const registrationId = registrationIdRef.current;
        const toolbar = rootRef.current;
        if (!publishesFocusFrames || registrationId == null || !toolbar) {
            if (registrationId != null) {
                unregisterEditorToolbarFrame(registrationId);
            }
            return;
        }

        if (typeof toolbar.measureInWindow !== 'function') {
            return;
        }

        const measuredFrameOwnerId = frameOwnerId;
        toolbar.measureInWindow((x, y, width, height) => {
            if (
                !framePublisherMountedRef.current ||
                !publishesFocusFramesRef.current ||
                frameOwnerIdRef.current !== measuredFrameOwnerId
            ) {
                return;
            }
            registerEditorToolbarFrame(
                registrationId,
                { x, y, width, height },
                measuredFrameOwnerId
            );
        });
    }, [frameOwnerId, publishesFocusFrames]);

    const publishMenuFrame = useCallback(() => {
        const registrationId = menuRegistrationIdRef.current;
        const menuCard = menuCardRef.current;
        if (!publishesFocusFrames || menuState == null || registrationId == null || !menuCard) {
            if (registrationId != null) {
                unregisterEditorToolbarFrame(registrationId);
            }
            return;
        }

        if (typeof menuCard.measureInWindow !== 'function') {
            return;
        }

        const measuredFrameOwnerId = frameOwnerId;
        const measuredMenuState = menuState;
        menuCard.measureInWindow((x, y, width, height) => {
            if (
                !framePublisherMountedRef.current ||
                !publishesFocusFramesRef.current ||
                frameOwnerIdRef.current !== measuredFrameOwnerId ||
                menuStateRef.current !== measuredMenuState
            ) {
                return;
            }
            registerEditorToolbarFrame(
                registrationId,
                { x, y, width, height },
                measuredFrameOwnerId
            );
        });
    }, [frameOwnerId, menuState, publishesFocusFrames]);

    const publishToolbarFrames = useCallback(() => {
        publishToolbarFrame();
        publishMenuFrame();
    }, [publishMenuFrame, publishToolbarFrame]);

    const cancelScheduledFramePublishes = useCallback(() => {
        framePublishAnimationFramesRef.current.forEach((frame) => cancelAnimationFrame(frame));
        framePublishAnimationFramesRef.current = [];
        framePublishTimeoutsRef.current.forEach((timeout) => clearTimeout(timeout));
        framePublishTimeoutsRef.current = [];
    }, []);

    const scheduleToolbarFramePublish = useCallback(() => {
        if (!publishesFocusFrames) {
            return;
        }

        cancelScheduledFramePublishes();
        publishToolbarFrames();

        framePublishAnimationFramesRef.current.push(requestAnimationFrame(publishToolbarFrames));
        KEYBOARD_FRAME_REMEASURE_DELAYS_MS.forEach((delay) => {
            framePublishTimeoutsRef.current.push(setTimeout(publishToolbarFrames, delay));
        });
    }, [cancelScheduledFramePublishes, publishesFocusFrames, publishToolbarFrames]);

    const handleToolbarLayout = useCallback(() => {
        requestAnimationFrame(publishToolbarFrame);
    }, [publishToolbarFrame]);

    const handleMenuLayout = useCallback(() => {
        requestAnimationFrame(publishMenuFrame);
    }, [publishMenuFrame]);

    useEffect(() => {
        if (!publishesFocusFrames) {
            const registrationId = registrationIdRef.current;
            if (registrationId != null) {
                unregisterEditorToolbarFrame(registrationId);
            }
            return;
        }

        const frame = requestAnimationFrame(publishToolbarFrame);
        return () => cancelAnimationFrame(frame);
    }, [
        expandedGroupKey,
        menuState?.groupKey,
        publishesFocusFrames,
        publishToolbarFrame,
        startItems.length,
        scrollItems.length,
        endItems.length,
        windowHeight,
        windowWidth,
    ]);

    useEffect(() => {
        if (!publishesFocusFrames || menuState == null) {
            const registrationId = menuRegistrationIdRef.current;
            if (registrationId != null) {
                unregisterEditorToolbarFrame(registrationId);
            }
            return;
        }

        const frame = requestAnimationFrame(publishMenuFrame);
        return () => cancelAnimationFrame(frame);
    }, [menuState, publishesFocusFrames, publishMenuFrame, windowHeight, windowWidth]);

    useEffect(() => {
        const registrationId = registrationIdRef.current;
        const menuRegistrationId = menuRegistrationIdRef.current;
        return () => {
            cancelScheduledFramePublishes();
            if (toolbarInteractionActiveRef.current) {
                toolbarInteractionActiveRef.current = false;
                endEditorToolbarInteraction();
            }
            if (registrationId != null) {
                unregisterEditorToolbarFrame(registrationId);
            }
            if (menuRegistrationId != null) {
                unregisterEditorToolbarFrame(menuRegistrationId);
            }
        };
    }, [cancelScheduledFramePublishes]);

    useEffect(() => {
        if (!publishesFocusFrames) {
            cancelScheduledFramePublishes();
            return;
        }

        const subscriptions = [
            Keyboard.addListener('keyboardDidShow', scheduleToolbarFramePublish),
            Keyboard.addListener('keyboardDidHide', scheduleToolbarFramePublish),
            Keyboard.addListener('keyboardDidChangeFrame', scheduleToolbarFramePublish),
        ];

        return () => {
            subscriptions.forEach((subscription) => subscription.remove());
            cancelScheduledFramePublishes();
        };
    }, [cancelScheduledFramePublishes, publishesFocusFrames, scheduleToolbarFramePublish]);

    useEffect(() => {
        if (expandedGroupKey != null && !groupsByKey.has(expandedGroupKey)) {
            setExpandedGroupKey(null);
        }
    }, [expandedGroupKey, groupsByKey]);

    useEffect(() => {
        if (menuState != null && !groupsByKey.has(menuState.groupKey)) {
            setMenuState(null);
        }
    }, [groupsByKey, menuState]);

    useEffect(() => {
        if (shouldRenderMentionSuggestions) {
            setExpandedGroupKey(null);
            setMenuState(null);
        }
    }, [shouldRenderMentionSuggestions]);

    const handleButtonPress = useCallback((button: ToolbarButton) => {
        button.action();
        if (button.groupKey) {
            setExpandedGroupKey((current) => (current === button.groupKey ? null : current));
        }
        setMenuState(null);
    }, []);

    const handleToolbarPressIn = useCallback(() => {
        if (preserveEditorFocus && !toolbarInteractionActiveRef.current) {
            toolbarInteractionActiveRef.current = true;
            beginEditorToolbarInteraction();
        }
    }, [preserveEditorFocus]);

    const handleToolbarPressOut = useCallback(() => {
        if (preserveEditorFocus && toolbarInteractionActiveRef.current) {
            toolbarInteractionActiveRef.current = false;
            endEditorToolbarInteraction();
        }
    }, [preserveEditorFocus]);

    const handleGroupPress = useCallback((group: ToolbarGroupButton) => {
        if (group.isDisabled) {
            return;
        }
        if (group.presentation === 'expand') {
            setMenuState(null);
            setExpandedGroupKey((current) => (current === group.key ? null : group.key));
            return;
        }

        const anchor = groupButtonRefs.current.get(group.key);
        if (!anchor) {
            return;
        }
        anchor.measureInWindow((x, y, width, height) => {
            setExpandedGroupKey(null);
            setMenuState((current) =>
                current?.groupKey === group.key
                    ? null
                    : {
                          groupKey: group.key,
                          x,
                          y,
                          width,
                          height,
                      }
            );
        });
    }, []);

    const menuGroup = menuState != null ? (groupsByKey.get(menuState.groupKey) ?? null) : null;
    const menuHeight = menuGroup ? menuGroup.children.length * 40 + 16 : 0;
    // Sizing contract shared with ios/NativeEditorExpoView.swift
    // (resolvedToolbarHeight/resolvedButtonSize) and
    // android/NativeToolbar.kt (resolvedToolbarHeightDp/resolvedButtonSizeDp):
    // an explicit theme height is honored as-is; buttons are
    // max(1, min(MAX_BUTTON_SIZE, height - BUTTON_HEIGHT_INSET)).
    const resolvedToolbarHeight = Math.max(
        theme?.height ?? BUTTON_VISIBLE + TOOLBAR_PADDING_V * 2,
        1
    );
    const resolvedButtonHeight =
        theme?.height == null
            ? BUTTON_VISIBLE
            : Math.max(1, Math.min(MAX_BUTTON_SIZE, resolvedToolbarHeight - BUTTON_HEIGHT_INSET));
    const resolvedToolbarPaddingV =
        theme?.height == null
            ? TOOLBAR_PADDING_V
            : Math.max(0, (resolvedToolbarHeight - resolvedButtonHeight) / 2);
    const resolvedSeparatorHeight = Math.max(16, resolvedButtonHeight - 12);
    const menuTop =
        menuState == null
            ? 0
            : Math.max(
                  MENU_MARGIN,
                  Math.min(
                      menuState.y + menuState.height + 8,
                      windowHeight - menuHeight - MENU_MARGIN
                  )
              );
    const menuLeft =
        menuState == null
            ? 0
            : Math.max(
                  MENU_MARGIN,
                  Math.min(
                      menuState.x + menuState.width - MENU_WIDTH,
                      windowWidth - MENU_WIDTH - MENU_MARGIN
                  )
              );

    const resolveButtonVisuals = (
        button: Pick<ToolbarButton, 'buttonStyle' | 'isActive' | 'isDisabled'>
    ) => {
        const activeColor =
            button.buttonStyle?.activeColor ?? theme?.buttonActiveColor ?? ACTIVE_COLOR;
        const defaultColor = button.buttonStyle?.color ?? theme?.buttonColor ?? DEFAULT_COLOR;
        const disabledColor =
            button.buttonStyle?.disabledColor ?? theme?.buttonDisabledColor ?? DISABLED_COLOR;
        const backgroundColor =
            button.buttonStyle?.backgroundColor ?? theme?.buttonBackgroundColor ?? 'transparent';
        const activeBackgroundColor =
            button.buttonStyle?.activeBackgroundColor ??
            theme?.buttonActiveBackgroundColor ??
            ACTIVE_BG;
        const disabledBackgroundColor =
            button.buttonStyle?.disabledBackgroundColor ??
            theme?.buttonDisabledBackgroundColor ??
            (button.isActive ? activeBackgroundColor : backgroundColor);
        const requestedIconSize = button.buttonStyle?.iconSize ?? theme?.buttonIconSize;
        return {
            color: button.isDisabled ? disabledColor : button.isActive ? activeColor : defaultColor,
            backgroundColor: button.isDisabled
                ? disabledBackgroundColor
                : button.isActive
                  ? activeBackgroundColor
                  : backgroundColor,
            iconSize:
                requestedIconSize != null &&
                Number.isFinite(requestedIconSize) &&
                requestedIconSize > 0
                    ? Math.min(requestedIconSize, resolvedButtonHeight)
                    : undefined,
            borderRadius: Math.max(
                0,
                button.buttonStyle?.borderRadius ?? theme?.buttonBorderRadius ?? BUTTON_RADIUS
            ),
        };
    };

    const renderButton = (
        button: Pick<
            ToolbarButton,
            'key' | 'label' | 'icon' | 'buttonStyle' | 'isActive' | 'isDisabled'
        >,
        onPress: () => void,
        options?: {
            anchorGroupKey?: string;
            showsDisclosure?: boolean;
            expanded?: boolean;
        }
    ) => {
        const visuals = resolveButtonVisuals(button);
        const anchorGroupKey = options?.anchorGroupKey;

        return (
            <View
                key={button.key}
                ref={
                    anchorGroupKey == null
                        ? undefined
                        : (node) => {
                              if (node) {
                                  groupButtonRefs.current.set(anchorGroupKey, node);
                              } else {
                                  groupButtonRefs.current.delete(anchorGroupKey);
                              }
                          }
                }
                collapsable={false}
                style={styles.buttonAnchor}>
                <TouchableOpacity
                    onPressIn={handleToolbarPressIn}
                    onPressOut={handleToolbarPressOut}
                    onPress={onPress}
                    disabled={button.isDisabled}
                    style={[
                        styles.button,
                        {
                            height: resolvedButtonHeight,
                            borderRadius: visuals.borderRadius,
                            backgroundColor: visuals.backgroundColor,
                        },
                    ]}
                    activeOpacity={0.5}
                    accessibilityRole='button'
                    accessibilityLabel={button.label}
                    accessibilityState={{
                        selected: button.isActive,
                        disabled: button.isDisabled,
                        expanded: options?.showsDisclosure ? options.expanded : undefined,
                    }}>
                    <View>
                        <ToolbarIcon
                            icon={button.icon}
                            color={visuals.color}
                            size={visuals.iconSize}
                        />
                    </View>
                </TouchableOpacity>
                {options?.showsDisclosure ? (
                    <Text style={[styles.groupDisclosure, { color: visuals.color }]}>
                        {'\u25BE'}
                    </Text>
                ) : null}
            </View>
        );
    };

    const renderSeparator = (key: string) => (
        <View
            key={key}
            style={[
                styles.separator,
                { height: resolvedSeparatorHeight },
                theme?.separatorColor != null ? { backgroundColor: theme.separatorColor } : null,
            ]}
        />
    );

    const renderToolbarItems = (items: ToolbarRenderedItem[]) =>
        items.map((item) => {
            if (item.type === 'separator') {
                return renderSeparator(item.key);
            }
            if (item.type === 'group') {
                return renderButton(
                    {
                        key: item.group.key,
                        label: item.group.label,
                        icon: item.group.icon,
                        buttonStyle: item.group.buttonStyle,
                        isActive: item.group.isActive,
                        isDisabled: item.group.isDisabled,
                    },
                    () => handleGroupPress(item.group),
                    {
                        anchorGroupKey: item.group.key,
                        showsDisclosure: true,
                        expanded: item.group.isOpen,
                    }
                );
            }
            return renderButton(item.button, () => handleButtonPress(item.button));
        });

    return (
        <View
            ref={rootRef}
            testID='editor-toolbar-root'
            collapsable={false}
            onLayout={handleToolbarLayout}
            style={[
                styles.container,
                !resolvedShowTopBorder && styles.containerWithoutTopBorder,
                {
                    minHeight: resolvedToolbarHeight,
                    paddingVertical: resolvedToolbarPaddingV,
                },
                theme?.backgroundColor != null ? { backgroundColor: theme.backgroundColor } : null,
                theme?.borderColor != null
                    ? resolvedShowTopBorder
                        ? { borderTopColor: theme.borderColor }
                        : null
                    : null,
                theme?.borderWidth != null
                    ? resolvedShowTopBorder
                        ? { borderTopWidth: theme.borderWidth }
                        : null
                    : null,
                {
                    borderRadius: theme?.borderRadius ?? TOOLBAR_RADIUS,
                },
            ]}>
            <View style={styles.toolbarRow}>
                {startItems.length > 0 ? (
                    <View style={[styles.fixedSection, styles.startFixedSection]}>
                        {renderToolbarItems(startItems)}
                    </View>
                ) : null}
                {shouldRenderMentionSuggestions && mentionState != null ? (
                    <ScrollView
                        testID='editor-toolbar-mention-suggestions'
                        horizontal
                        showsHorizontalScrollIndicator={false}
                        style={[
                            styles.scrollSection,
                            styles.mentionSuggestionsScroll,
                            {
                                backgroundColor:
                                    mentionState.theme?.suggestions?.backgroundColor ??
                                    'transparent',
                                borderColor:
                                    mentionState.theme?.suggestions?.borderColor ?? 'transparent',
                                borderWidth: mentionState.theme?.suggestions?.borderWidth ?? 0,
                                borderRadius: mentionState.theme?.suggestions?.borderRadius ?? 0,
                            },
                            mentionState.theme?.suggestions?.shadowColor != null
                                ? {
                                      shadowColor: mentionState.theme.suggestions.shadowColor,
                                      shadowOpacity: 0.14,
                                      shadowRadius: 12,
                                      shadowOffset: { width: 0, height: 4 },
                                      elevation: 8,
                                  }
                                : null,
                        ]}
                        contentContainerStyle={styles.mentionSuggestionsContent}
                        keyboardShouldPersistTaps='always'>
                        {mentionState.suggestions.map((suggestion) => {
                            const label = resolveMentionSuggestionDisplayLabel(
                                suggestion,
                                mentionState.trigger
                            );
                            const optionTheme = (
                                mentionState.suggestionThemes?.[suggestion.key] ??
                                mentionState.theme
                            )?.suggestions?.option;
                            return (
                                <Pressable
                                    key={suggestion.key}
                                    testID={`editor-toolbar-mention-suggestion-${suggestion.key}`}
                                    accessibilityRole='button'
                                    accessibilityLabel={label}
                                    onPressIn={handleToolbarPressIn}
                                    onPressOut={handleToolbarPressOut}
                                    onPress={() => mentionState.onSelectSuggestion(suggestion)}
                                    style={({ pressed }) => [
                                        styles.mentionSuggestion,
                                        {
                                            backgroundColor: pressed
                                                ? (optionTheme?.highlightedBackgroundColor ??
                                                  'rgba(0, 122, 255, 0.12)')
                                                : (optionTheme?.backgroundColor ?? '#F2F2F7'),
                                            borderColor: optionTheme?.borderColor ?? 'transparent',
                                            borderWidth: optionTheme?.borderWidth ?? 0,
                                            borderRadius: optionTheme?.borderRadius ?? 12,
                                        },
                                    ]}>
                                    {({ pressed }) => (
                                        <>
                                            <Text
                                                numberOfLines={1}
                                                style={[
                                                    styles.mentionSuggestionTitle,
                                                    {
                                                        fontWeight:
                                                            optionTheme?.fontWeight ?? '600',
                                                        color: pressed
                                                            ? (optionTheme?.highlightedTextColor ??
                                                              optionTheme?.textColor ??
                                                              '#000000')
                                                            : (optionTheme?.textColor ?? '#000000'),
                                                    },
                                                ]}>
                                                {label}
                                            </Text>
                                            {suggestion.subtitle ? (
                                                <Text
                                                    numberOfLines={1}
                                                    style={[
                                                        styles.mentionSuggestionSubtitle,
                                                        {
                                                            color:
                                                                optionTheme?.secondaryTextColor ??
                                                                '#8E8E93',
                                                        },
                                                    ]}>
                                                    {suggestion.subtitle}
                                                </Text>
                                            ) : null}
                                        </>
                                    )}
                                </Pressable>
                            );
                        })}
                    </ScrollView>
                ) : (
                    <ScrollView
                        horizontal
                        showsHorizontalScrollIndicator={false}
                        style={styles.scrollSection}
                        contentContainerStyle={styles.scrollContent}
                        keyboardShouldPersistTaps='always'
                        onScrollBeginDrag={() => setMenuState(null)}>
                        {renderToolbarItems(scrollItems)}
                    </ScrollView>
                )}
                {endItems.length > 0 ? (
                    <View style={[styles.fixedSection, styles.endFixedSection]}>
                        {renderToolbarItems(endItems)}
                    </View>
                ) : null}
            </View>
            {!shouldRenderMentionSuggestions && menuState != null && menuGroup != null ? (
                <Modal
                    transparent
                    visible
                    animationType='fade'
                    onRequestClose={() => setMenuState(null)}>
                    <Pressable style={styles.menuBackdrop} onPress={() => setMenuState(null)}>
                        <View
                            ref={menuCardRef}
                            testID='editor-toolbar-menu-card'
                            collapsable={false}
                            onLayout={handleMenuLayout}
                            style={[
                                styles.menuCard,
                                {
                                    top: menuTop,
                                    left: menuLeft,
                                    backgroundColor: theme?.backgroundColor ?? TOOLBAR_BG,
                                    borderColor: theme?.borderColor ?? MENU_BORDER,
                                },
                            ]}>
                            {menuGroup.children.map((button) => {
                                const visuals = resolveButtonVisuals(button);
                                return (
                                    <Pressable
                                        key={button.key}
                                        onPressIn={handleToolbarPressIn}
                                        onPressOut={handleToolbarPressOut}
                                        onPress={() => handleButtonPress(button)}
                                        disabled={button.isDisabled}
                                        style={({ pressed }) => [
                                            styles.menuItem,
                                            {
                                                backgroundColor: visuals.backgroundColor,
                                            },
                                            pressed &&
                                                !button.isDisabled && {
                                                    opacity: 0.75,
                                                },
                                        ]}
                                        accessibilityRole='button'
                                        accessibilityLabel={button.label}
                                        accessibilityState={{
                                            selected: button.isActive,
                                            disabled: button.isDisabled,
                                        }}>
                                        <ToolbarIcon
                                            icon={button.icon}
                                            color={visuals.color}
                                            size={visuals.iconSize}
                                        />
                                        <Text style={[styles.menuLabel, { color: visuals.color }]}>
                                            {button.label}
                                        </Text>
                                    </Pressable>
                                );
                            })}
                        </View>
                    </Pressable>
                </Modal>
            ) : null}
        </View>
    );
}

function ToolbarIcon({
    icon,
    color,
    size,
}: {
    icon: EditorToolbarIcon;
    color: string;
    size?: number;
}) {
    const materialIconName = resolveMaterialIconName(icon);
    if (materialIconName) {
        return (
            <View style={styles.iconContainer}>
                <MaterialIcons name={materialIconName as never} size={size ?? 20} color={color} />
            </View>
        );
    }

    const glyph = resolveGlyphText(icon) ?? '?';
    return (
        <View style={styles.iconContainer}>
            <Text style={[styles.iconText, size == null ? null : { fontSize: size }, { color }]}>
                {glyph}
            </Text>
        </View>
    );
}

function resolveMaterialIconName(icon: EditorToolbarIcon): string | undefined {
    switch (icon.type) {
        case 'default':
            return DEFAULT_MATERIAL_ICONS[icon.id];
        case 'platform':
            return icon.android?.type === 'material' ? icon.android.name : undefined;
        case 'glyph':
            return undefined;
    }
}

function resolveGlyphText(icon: EditorToolbarIcon): string | undefined {
    switch (icon.type) {
        case 'default':
            return DEFAULT_GLYPH_ICONS[icon.id];
        case 'glyph':
            return icon.text;
        case 'platform':
            return icon.fallbackText;
    }
}

const styles = StyleSheet.create({
    container: {
        backgroundColor: TOOLBAR_BG,
        borderTopWidth: StyleSheet.hairlineWidth,
        borderTopColor: TOOLBAR_BORDER,
        paddingVertical: TOOLBAR_PADDING_V,
        overflow: 'hidden',
    },
    containerWithoutTopBorder: {
        borderTopWidth: 0,
    },
    scrollContent: {
        flexDirection: 'row',
        alignItems: 'center',
        paddingHorizontal: TOOLBAR_PADDING_H,
        minWidth: '100%',
    },
    mentionSuggestionsContent: {
        paddingHorizontal: 12,
        paddingVertical: 4,
        alignItems: 'center',
        minWidth: '100%',
    },
    mentionSuggestionsScroll: {
        overflow: 'hidden',
    },
    toolbarRow: {
        flexDirection: 'row',
        alignItems: 'center',
    },
    fixedSection: {
        flexDirection: 'row',
        alignItems: 'center',
        flexShrink: 0,
    },
    startFixedSection: {
        paddingStart: TOOLBAR_PADDING_H,
    },
    endFixedSection: {
        paddingEnd: TOOLBAR_PADDING_H,
    },
    scrollSection: {
        flex: 1,
        minWidth: 0,
    },
    mentionSuggestion: {
        minWidth: 88,
        minHeight: 40,
        marginRight: 8,
        paddingHorizontal: 12,
        paddingVertical: 8,
        justifyContent: 'center',
    },
    mentionSuggestionTitle: {
        fontSize: 14,
        fontWeight: '600',
    },
    mentionSuggestionSubtitle: {
        marginTop: 1,
        fontSize: 12,
    },
    buttonAnchor: {
        position: 'relative',
    },
    button: {
        width: BUTTON_HIT,
        height: BUTTON_VISIBLE,
        justifyContent: 'center',
        alignItems: 'center',
        borderRadius: BUTTON_RADIUS,
    },
    groupDisclosure: {
        position: 'absolute',
        right: 5,
        bottom: 2,
        fontSize: 9,
        fontWeight: '700',
    },
    separator: {
        width: StyleSheet.hairlineWidth,
        height: 20,
        marginHorizontal: 4,
        backgroundColor: SEPARATOR_COLOR,
    },
    iconContainer: {
        justifyContent: 'center',
        alignItems: 'center',
    },
    iconText: {
        fontSize: 16,
        fontWeight: '600',
    },
    menuBackdrop: {
        flex: 1,
    },
    menuCard: {
        position: 'absolute',
        width: MENU_WIDTH,
        borderRadius: 14,
        borderWidth: StyleSheet.hairlineWidth,
        paddingVertical: 8,
        shadowColor: MENU_SHADOW,
        shadowOpacity: 0.16,
        shadowRadius: 18,
        shadowOffset: { width: 0, height: 8 },
        elevation: 10,
    },
    menuItem: {
        minHeight: 40,
        paddingHorizontal: 12,
        flexDirection: 'row',
        alignItems: 'center',
    },
    menuLabel: {
        flex: 1,
        marginLeft: 10,
        fontSize: 14,
        fontWeight: '500',
    },
});
