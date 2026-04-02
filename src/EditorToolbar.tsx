import { MaterialIcons } from '@expo/vector-icons';
import React, { useCallback } from 'react';
import { ScrollView, StyleSheet, Text, TouchableOpacity, View } from 'react-native';

import type { ActiveState, HistoryState } from './NativeEditorBridge';
import type { EditorToolbarTheme } from './EditorTheme';

interface ToolbarButton {
    key: string;
    label: string;
    icon: EditorToolbarIcon;
    action: () => void;
    isActive?: boolean;
    isDisabled?: boolean;
}

export type EditorToolbarListType = 'bulletList' | 'orderedList';
export type EditorToolbarCommand = 'indentList' | 'outdentList' | 'undo' | 'redo';

export type EditorToolbarDefaultIconId =
    | 'bold'
    | 'italic'
    | 'underline'
    | 'strike'
    | 'link'
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

export type EditorToolbarItem =
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
          type: 'separator';
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
    /** Generic node insertion handler used by configurable node buttons. */
    onInsertNodeType?: (nodeType: string) => void;
    /** Generic command handler used by configurable command buttons. */
    onRunCommand?: (command: EditorToolbarCommand) => void;
    /** Generic action handler for arbitrary JS-defined toolbar buttons. */
    onToolbarAction?: (key: string) => void;
    /** Link button handler used by first-class link toolbar items. */
    onRequestLink?: () => void;
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

const ACTIVE_BG = 'rgba(0, 122, 255, 0.12)';
const ACTIVE_COLOR = '#007AFF';
const DEFAULT_COLOR = '#666666';
const DISABLED_COLOR = '#C7C7CC';
const SEPARATOR_COLOR = '#E5E5EA';
const TOOLBAR_BG = '#FFFFFF';
const TOOLBAR_BORDER = '#E5E5EA';
const TOOLBAR_RADIUS = 0;
const BUTTON_RADIUS = 6;

const DEFAULT_GLYPH_ICONS: Record<EditorToolbarDefaultIconId, string> = {
    bold: 'B',
    italic: 'I',
    underline: 'U',
    strike: 'S',
    link: '🔗',
    blockquote: '❝',
    bulletList: '•≡',
    orderedList: '1.',
    indentList: '→',
    outdentList: '←',
    lineBreak: '↵',
    horizontalRule: '—',
    undo: '↩',
    redo: '↪',
};

const DEFAULT_MATERIAL_ICONS: Record<EditorToolbarDefaultIconId, string> = {
    bold: 'format-bold',
    italic: 'format-italic',
    underline: 'format-underlined',
    strike: 'strikethrough-s',
    link: 'link',
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
    toolbarItems = DEFAULT_EDITOR_TOOLBAR_ITEMS,
    theme,
    showTopBorder = true,
}: EditorToolbarProps) {
    const marks = activeState.marks ?? {};
    const nodes = activeState.nodes ?? {};
    const commands = activeState.commands ?? {};
    const allowedMarks = activeState.allowedMarks ?? [];
    const insertableNodes = activeState.insertableNodes ?? [];

    const isMarkActive = useCallback((mark: string) => !!marks[mark], [marks]);

    const isInList = !!nodes['bulletList'] || !!nodes['orderedList'];
    const canIndentList = isInList && !!commands['indentList'];
    const canOutdentList = isInList && !!commands['outdentList'];

    const getActionForItem = useCallback(
        (item: EditorToolbarItem): (() => void) | null => {
            switch (item.type) {
                case 'separator':
                    return null;
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
            onRequestLink,
            onToolbarAction,
            onToggleBold,
            onToggleBlockquote,
            onToggleBulletList,
            onToggleItalic,
            onToggleListType,
            onToggleMark,
            onToggleOrderedList,
            onToggleStrike,
            onToggleUnderline,
            onUndo,
        ]
    );

    const makeButtonKey = useCallback(
        (item: Exclude<EditorToolbarItem, { type: 'separator' }>, index: number) =>
            item.key ??
            (item.type === 'mark'
                ? `mark:${item.mark}:${index}`
                : item.type === 'link'
                  ? `link:${index}`
                  : item.type === 'blockquote'
                    ? `blockquote:${index}`
                    : item.type === 'list'
                      ? `list:${item.listType}:${index}`
                      : item.type === 'command'
                        ? `command:${item.command}:${index}`
                        : item.type === 'node'
                          ? `node:${item.nodeType}:${index}`
                          : `action:${item.key}:${index}`),
        []
    );

    const renderedItems: Array<
        { type: 'separator'; key: string } | { type: 'button'; button: ToolbarButton }
    > = [];

    for (let index = 0; index < toolbarItems.length; index += 1) {
        const item = toolbarItems[index];
        if (item.type === 'separator') {
            renderedItems.push({
                type: 'separator',
                key: item.key ?? `separator:${index}`,
            });
            continue;
        }

        const action = getActionForItem(item);
        if (!action) {
            continue;
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

        renderedItems.push({
            type: 'button',
            button: {
                key: makeButtonKey(item, index),
                label: item.label,
                icon: item.icon,
                action,
                isActive,
                isDisabled,
            },
        });
    }

    const compactItems = renderedItems.filter((entry, index, list) => {
        if (entry.type !== 'separator') {
            return true;
        }
        const previous = list[index - 1];
        const next = list[index + 1];
        return previous?.type === 'button' && next?.type === 'button';
    });

    const renderButton = ({ key, label, icon, action, isActive, isDisabled }: ToolbarButton) => {
        const activeColor = theme?.buttonActiveColor ?? ACTIVE_COLOR;
        const defaultColor = theme?.buttonColor ?? DEFAULT_COLOR;
        const disabledColor = theme?.buttonDisabledColor ?? DISABLED_COLOR;
        const color = isActive ? activeColor : isDisabled ? disabledColor : defaultColor;

        return (
            <TouchableOpacity
                key={key}
                onPress={action}
                disabled={isDisabled}
                style={[
                    styles.button,
                    {
                        borderRadius: theme?.buttonBorderRadius ?? BUTTON_RADIUS,
                    },
                    isActive && {
                        backgroundColor: theme?.buttonActiveBackgroundColor ?? ACTIVE_BG,
                    },
                ]}
                activeOpacity={0.5}
                accessibilityRole='button'
                accessibilityLabel={label}
                accessibilityState={{ selected: isActive, disabled: isDisabled }}>
                <View>
                    <ToolbarIcon icon={icon} color={color} />
                </View>
            </TouchableOpacity>
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
                keyboardShouldPersistTaps='always'>
                {compactItems.map((item) =>
                    item.type === 'separator'
                        ? renderSeparator(item.key)
                        : renderButton(item.button)
                )}
            </ScrollView>
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
    button: {
        width: BUTTON_HIT,
        height: BUTTON_VISIBLE,
        justifyContent: 'center',
        alignItems: 'center',
        borderRadius: BUTTON_RADIUS,
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
});
