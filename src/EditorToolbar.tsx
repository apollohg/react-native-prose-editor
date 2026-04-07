import { MaterialIcons } from '@expo/vector-icons';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
    Modal,
    Pressable,
    ScrollView,
    StyleSheet,
    Text,
    TouchableOpacity,
    View,
    useWindowDimensions,
} from 'react-native';

import type { ActiveState, HistoryState } from './NativeEditorBridge';
import type { EditorToolbarTheme } from './EditorTheme';

export type EditorToolbarListType = 'bulletList' | 'orderedList';
export type EditorToolbarHeadingLevel = 1 | 2 | 3 | 4 | 5 | 6;
export type EditorToolbarCommand = 'indentList' | 'outdentList' | 'undo' | 'redo';
export type EditorToolbarGroupPresentation = 'expand' | 'menu';

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

export interface EditorToolbarSFSymbolIcon {
    type: 'sfSymbol';
    name: string;
}

export interface EditorToolbarMaterialIcon {
    type: 'material';
    name: string;
}

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

export type EditorToolbarLeafItem =
    | {
          type: 'mark';
          mark: string;
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
      }
    | {
          type: 'link';
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
      }
    | {
          type: 'image';
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
      }
    | {
          type: 'heading';
          level: EditorToolbarHeadingLevel;
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
      }
    | {
          type: 'blockquote';
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
      }
    | {
          type: 'list';
          listType: EditorToolbarListType;
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
      }
    | {
          type: 'command';
          command: EditorToolbarCommand;
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
      }
    | {
          type: 'node';
          nodeType: string;
          label: string;
          icon: EditorToolbarIcon;
          key?: string;
      }
    | {
          type: 'action';
          key: string;
          label: string;
          icon: EditorToolbarIcon;
          isActive?: boolean;
          isDisabled?: boolean;
      };

export type EditorToolbarGroupChildItem = EditorToolbarLeafItem;

export interface EditorToolbarGroupItem {
    type: 'group';
    key: string;
    label: string;
    icon: EditorToolbarIcon;
    presentation?: EditorToolbarGroupPresentation;
    items: readonly EditorToolbarGroupChildItem[];
}

export type EditorToolbarItem =
    | EditorToolbarLeafItem
    | EditorToolbarGroupItem
    | {
          type: 'separator';
          key?: string;
      };

interface ToolbarButton {
    key: string;
    label: string;
    icon: EditorToolbarIcon;
    action: () => void;
    isActive?: boolean;
    isDisabled?: boolean;
    groupKey?: string;
}

interface ToolbarGroupButton {
    key: string;
    label: string;
    icon: EditorToolbarIcon;
    presentation: EditorToolbarGroupPresentation;
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

type ToolbarRenderedItem =
    | { type: 'separator'; key: string }
    | { type: 'button'; button: ToolbarButton }
    | { type: 'group'; group: ToolbarGroupButton };

function defaultIcon(id: EditorToolbarDefaultIconId): EditorToolbarIcon {
    return { type: 'default', id };
}

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
    activeState: ActiveState;
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
}

const BUTTON_HIT = 44;
const BUTTON_VISIBLE = 32;
const TOOLBAR_PADDING_H = 12;
const TOOLBAR_PADDING_V = 4;
const MENU_MARGIN = 8;
const MENU_WIDTH = 192;

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
    showTopBorder = true,
}: EditorToolbarProps) {
    const marks = activeState.marks ?? {};
    const nodes = activeState.nodes ?? {};
    const commands = activeState.commands ?? {};
    const allowedMarks = activeState.allowedMarks ?? [];
    const insertableNodes = activeState.insertableNodes ?? [];
    const groupButtonRefs = useRef(new Map<string, View | null>());
    const { width: windowWidth, height: windowHeight } = useWindowDimensions();
    const [expandedGroupKey, setExpandedGroupKey] = useState<string | null>(null);
    const [menuState, setMenuState] = useState<ToolbarMenuState | null>(null);

    const isMarkActive = useCallback((mark: string) => !!marks[mark], [marks]);

    const isInList = !!nodes['bulletList'] || !!nodes['orderedList'];
    const canIndentList = isInList && !!commands['indentList'];
    const canOutdentList = isInList && !!commands['outdentList'];

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
        (item: EditorToolbarLeafItem, index: number, prefix = '', groupKey?: string): ToolbarButton | null => {
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
                action,
                isActive,
                isDisabled,
                groupKey,
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

    const { renderedItems, groupsByKey } = useMemo(() => {
        const entries: ToolbarRenderedItem[] = [];
        const nextGroups = new Map<string, ToolbarGroupButton>();

        for (let index = 0; index < toolbarItems.length; index += 1) {
            const item = toolbarItems[index];
            if (item.type === 'separator') {
                entries.push({
                    type: 'separator',
                    key: item.key ?? `separator:${index}`,
                });
                continue;
            }

            if (item.type === 'group') {
                const children = item.items
                    .map((child, childIndex) =>
                        resolveButton(child, childIndex, `${item.key}:`, item.key)
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
                    presentation,
                    children,
                    isActive:
                        children.some((child) => child.isActive) || isExpanded || isMenuOpen,
                    isDisabled: children.every((child) => child.isDisabled),
                    isExpanded,
                    isOpen: isExpanded || isMenuOpen,
                };
                nextGroups.set(group.key, group);
                entries.push({ type: 'group', group });
                if (group.isExpanded) {
                    entries.push(
                        ...children.map(
                            (child): ToolbarRenderedItem => ({ type: 'button', button: child })
                        )
                    );
                }
                continue;
            }

            const button = resolveButton(item, index);
            if (button) {
                entries.push({ type: 'button', button });
            }
        }

        return {
            renderedItems: entries.filter((entry, index, list) => {
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
            }),
            groupsByKey: nextGroups,
        };
    }, [expandedGroupKey, menuState?.groupKey, resolveButton, toolbarItems]);

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

    const handleButtonPress = useCallback((button: ToolbarButton) => {
        button.action();
        if (button.groupKey) {
            setExpandedGroupKey((current) => (current === button.groupKey ? null : current));
        }
        setMenuState(null);
    }, []);

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

    const menuGroup = menuState != null ? groupsByKey.get(menuState.groupKey) ?? null : null;
    const menuHeight = menuGroup ? menuGroup.children.length * 40 + 16 : 0;
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

    const renderButton = (
        button: Pick<ToolbarButton, 'key' | 'label' | 'icon' | 'isActive' | 'isDisabled'>,
        onPress: () => void,
        options?: {
            anchorGroupKey?: string;
            showsDisclosure?: boolean;
            expanded?: boolean;
        }
    ) => {
        const activeColor = theme?.buttonActiveColor ?? ACTIVE_COLOR;
        const defaultColor = theme?.buttonColor ?? DEFAULT_COLOR;
        const disabledColor = theme?.buttonDisabledColor ?? DISABLED_COLOR;
        const color = button.isActive ? activeColor : button.isDisabled ? disabledColor : defaultColor;
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
                    onPress={onPress}
                    disabled={button.isDisabled}
                    style={[
                        styles.button,
                        {
                            borderRadius: theme?.buttonBorderRadius ?? BUTTON_RADIUS,
                        },
                        button.isActive && {
                            backgroundColor: theme?.buttonActiveBackgroundColor ?? ACTIVE_BG,
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
                        <ToolbarIcon icon={button.icon} color={color} />
                    </View>
                </TouchableOpacity>
                {options?.showsDisclosure ? (
                    <Text style={[styles.groupDisclosure, { color }]}>{'\u25BE'}</Text>
                ) : null}
            </View>
        );
    };

    const renderSeparator = (key: string) => (
        <View
            key={key}
            style={[
                styles.separator,
                theme?.separatorColor != null ? { backgroundColor: theme.separatorColor } : null,
            ]}
        />
    );

    return (
        <View
            style={[
                styles.container,
                !showTopBorder && styles.containerWithoutTopBorder,
                theme?.backgroundColor != null ? { backgroundColor: theme.backgroundColor } : null,
                theme?.borderColor != null
                    ? showTopBorder
                        ? { borderTopColor: theme.borderColor }
                        : null
                    : null,
                theme?.borderWidth != null
                    ? showTopBorder
                        ? { borderTopWidth: theme.borderWidth }
                        : null
                    : null,
                {
                    borderRadius: theme?.borderRadius ?? TOOLBAR_RADIUS,
                },
            ]}>
            <ScrollView
                horizontal
                showsHorizontalScrollIndicator={false}
                contentContainerStyle={styles.scrollContent}
                keyboardShouldPersistTaps='always'
                onScrollBeginDrag={() => setMenuState(null)}>
                {renderedItems.map((item) => {
                    if (item.type === 'separator') {
                        return renderSeparator(item.key);
                    }
                    if (item.type === 'group') {
                        return renderButton(
                            {
                                key: item.group.key,
                                label: item.group.label,
                                icon: item.group.icon,
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
                })}
            </ScrollView>
            {menuState != null && menuGroup != null ? (
                <Modal
                    transparent
                    visible
                    animationType='fade'
                    onRequestClose={() => setMenuState(null)}>
                    <Pressable style={styles.menuBackdrop} onPress={() => setMenuState(null)}>
                        <View
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
                                const activeColor = theme?.buttonActiveColor ?? ACTIVE_COLOR;
                                const defaultColor = theme?.buttonColor ?? DEFAULT_COLOR;
                                const disabledColor = theme?.buttonDisabledColor ?? DISABLED_COLOR;
                                const color = button.isActive
                                    ? activeColor
                                    : button.isDisabled
                                      ? disabledColor
                                      : defaultColor;
                                return (
                                    <Pressable
                                        key={button.key}
                                        onPress={() => handleButtonPress(button)}
                                        disabled={button.isDisabled}
                                        style={({ pressed }) => [
                                            styles.menuItem,
                                            button.isActive && {
                                                backgroundColor:
                                                    theme?.buttonActiveBackgroundColor ?? ACTIVE_BG,
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
                                        <ToolbarIcon icon={button.icon} color={color} />
                                        <Text style={[styles.menuLabel, { color }]}>
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

function ToolbarIcon({ icon, color }: { icon: EditorToolbarIcon; color: string }) {
    const materialIconName = resolveMaterialIconName(icon);
    if (materialIconName) {
        return (
            <View style={styles.iconContainer}>
                <MaterialIcons name={materialIconName as never} size={20} color={color} />
            </View>
        );
    }

    const glyph = resolveGlyphText(icon) ?? '?';
    return (
        <View style={styles.iconContainer}>
            <Text style={[styles.iconText, { color }]}>{glyph}</Text>
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
