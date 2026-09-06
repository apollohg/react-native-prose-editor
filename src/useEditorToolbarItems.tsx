import { useCallback, useMemo } from 'react';
import { useEditorToolbarState } from './useEditorToolbarState';
import {
    type EditorToolbarLeafItem,
    type EditorToolbarItemPlacement,
    type ToolbarButton,
    type ToolbarRenderedItem,
    type ToolbarGroupButton,
} from './EditorToolbarTypes';
import { resolveToolbarItemPlacement } from './EditorToolbarItems';

export function useEditorToolbarItems(
    context: Pick<
        ReturnType<typeof useEditorToolbarState>,
        | 'onToggleMark'
        | 'onToggleBold'
        | 'onToggleItalic'
        | 'onToggleUnderline'
        | 'onToggleStrike'
        | 'onToggleListType'
        | 'onToggleBulletList'
        | 'onToggleOrderedList'
        | 'onRequestLink'
        | 'onRequestImage'
        | 'onToggleHeading'
        | 'onToggleBlockquote'
        | 'onInsertNodeType'
        | 'onInsertLineBreak'
        | 'onInsertHorizontalRule'
        | 'onRunCommand'
        | 'onIndentList'
        | 'onOutdentList'
        | 'onUndo'
        | 'onRedo'
        | 'onToolbarAction'
        | 'isMarkActive'
        | 'allowedMarks'
        | 'insertableNodes'
        | 'nodes'
        | 'commands'
        | 'canIndentList'
        | 'canOutdentList'
        | 'historyState'
        | 'toolbarItems'
        | 'expandedGroupKey'
        | 'menuState'
    >
) {
    const {
        onToggleMark,
        onToggleBold,
        onToggleItalic,
        onToggleUnderline,
        onToggleStrike,
        onToggleListType,
        onToggleBulletList,
        onToggleOrderedList,
        onRequestLink,
        onRequestImage,
        onToggleHeading,
        onToggleBlockquote,
        onInsertNodeType,
        onInsertLineBreak,
        onInsertHorizontalRule,
        onRunCommand,
        onIndentList,
        onOutdentList,
        onUndo,
        onRedo,
        onToolbarAction,
        isMarkActive,
        allowedMarks,
        insertableNodes,
        nodes,
        commands,
        canIndentList,
        canOutdentList,
        historyState,
        toolbarItems,
        expandedGroupKey,
        menuState,
    } = context;

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
                    return item.listType === 'bulletList' || item.listType === 'bullet_list'
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
                        case 'hard_break':
                            return onInsertLineBreak ?? null;
                        case 'horizontalRule':
                        case 'horizontal_rule':
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
                            item.listType === 'bulletList' || item.listType === 'bullet_list'
                                ? 'wrapBulletList'
                                : 'wrapOrderedList'
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
    return { startItems, scrollItems, endItems, groupsByKey };
}
