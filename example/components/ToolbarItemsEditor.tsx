import React, { useCallback, useMemo, useRef, useState } from 'react';
import { Animated, PanResponder, Pressable, StyleSheet, Text, View } from 'react-native';
import {
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    type EditorToolbarItem,
} from '@apollohg/react-native-prose-editor';

import { FONT_SIZE, MIN_TOUCH_TARGET, RADIUS, SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome } from '../themePresets';

/** Reorderable editor for `toolbarItems`, pooled from the package's own defaults. */

const ITEM_HEIGHT = MIN_TOUCH_TARGET;
const ITEM_GAP = SPACE.xs;
const ITEM_STRIDE = ITEM_HEIGHT + ITEM_GAP;
const DRAG_LIFT_SCALE = 1.03;
const DRAG_HANDLE_WIDTH = 36;
const REMOVE_BUTTON_SIZE = MIN_TOUCH_TARGET;
const DRAG_HANDLE_GLYPH = '≡';
const REMOVE_GLYPH = '×';

type ToolbarItemsEditorProps = {
    items: readonly EditorToolbarItem[];
    onItemsChange: (items: EditorToolbarItem[]) => void;
    onReset: () => void;
    chrome: ExampleAppChrome;
};

function getItemId(item: EditorToolbarItem): string {
    switch (item.type) {
        case 'group':
            return `group:${item.key}`;
        case 'mark':
            return `mark:${item.mark}`;
        case 'heading':
            return `heading:${item.level}`;
        case 'link':
            return 'link';
        case 'image':
            return 'image';
        case 'blockquote':
            return 'blockquote';
        case 'list':
            return `list:${item.listType}`;
        case 'command':
            return `command:${item.command}`;
        case 'node':
            return `node:${item.nodeType}`;
        case 'action':
            return `action:${item.key}`;
        case 'separator':
            return 'separator';
    }
}

function getItemLabel(item: EditorToolbarItem): string {
    switch (item.type) {
        case 'separator':
            return 'Separator';
        case 'group':
            return `${item.label} (${item.presentation ?? 'expand'})`;
        default:
            return item.label;
    }
}

function ToolbarItemsEditorInner({
    items,
    onItemsChange,
    onReset,
    chrome,
}: ToolbarItemsEditorProps) {
    // Refs so PanResponder closures always read latest values
    const itemsRef = useRef(items);
    itemsRef.current = items;
    const onChangeRef = useRef(onItemsChange);
    onChangeRef.current = onItemsChange;

    const [dragIndex, setDragIndex] = useState<number | null>(null);
    const hoverRef = useRef(-1);

    // Animated values — shared across renders via refs
    const panY = useRef(new Animated.Value(0)).current;
    const dragScale = useRef(new Animated.Value(1)).current;
    const shiftValues = useRef<Animated.Value[]>([]);
    while (shiftValues.current.length < items.length) {
        shiftValues.current.push(new Animated.Value(0));
    }

    const clampHover = useCallback(
        (from: number, dy: number) =>
            Math.max(0, Math.min(items.length - 1, from + Math.round(dy / ITEM_STRIDE))),
        [items.length]
    );

    const animateShifts = useCallback((from: number, to: number) => {
        const count = itemsRef.current.length;
        for (let i = 0; i < count; i++) {
            if (i === from) continue;
            let target = 0;
            if (from < to && i > from && i <= to) target = -ITEM_STRIDE;
            if (from > to && i >= to && i < from) target = ITEM_STRIDE;
            Animated.spring(shiftValues.current[i], {
                toValue: target,
                useNativeDriver: true,
                speed: 20,
                bounciness: 0,
            }).start();
        }
    }, []);

    const resetAnimations = useCallback(
        (count: number) => {
            panY.setValue(0);
            dragScale.setValue(1);
            for (let i = 0; i < count; i++) shiftValues.current[i].setValue(0);
            setDragIndex(null);
            hoverRef.current = -1;
        },
        [panY, dragScale]
    );

    const createResponder = useCallback(
        (index: number) =>
            PanResponder.create({
                onStartShouldSetPanResponder: () => true,
                onPanResponderTerminationRequest: () => false,
                onPanResponderGrant: () => {
                    hoverRef.current = index;
                    setDragIndex(index);
                    panY.setValue(0);
                    Animated.spring(dragScale, {
                        toValue: DRAG_LIFT_SCALE,
                        useNativeDriver: true,
                    }).start();
                },
                onPanResponderMove: (_, gesture) => {
                    panY.setValue(gesture.dy);
                    const hover = clampHover(index, gesture.dy);
                    if (hover !== hoverRef.current) {
                        hoverRef.current = hover;
                        animateShifts(index, hover);
                    }
                },
                onPanResponderRelease: (_, gesture) => {
                    const count = itemsRef.current.length;
                    const to = clampHover(index, gesture.dy);

                    // Reset animated values and commit in one JS frame so React batches, no flash.
                    resetAnimations(count);

                    if (index !== to) {
                        const copy = [...itemsRef.current];
                        const [moved] = copy.splice(index, 1);
                        copy.splice(to, 0, moved);
                        onChangeRef.current(copy);
                    }
                },
                onPanResponderTerminate: () => {
                    resetAnimations(itemsRef.current.length);
                },
            }),
        [panY, dragScale, clampHover, animateShifts, resetAnimations]
    );

    // Cache responders per index — rebuild when count changes
    const respondersRef = useRef<ReturnType<typeof PanResponder.create>[]>([]);
    const prevCountRef = useRef(-1);
    if (prevCountRef.current !== items.length) {
        respondersRef.current = items.map((_, i) => createResponder(i));
        prevCountRef.current = items.length;
    }

    const availableItems = useMemo(() => {
        const activeIds = new Set(items.map(getItemId));
        return DEFAULT_EDITOR_TOOLBAR_ITEMS.filter((item) => !activeIds.has(getItemId(item)));
    }, [items]);

    const removeItem = useCallback(
        (index: number) => {
            const copy = [...items];
            copy.splice(index, 1);
            onItemsChange(copy);
        },
        [items, onItemsChange]
    );

    const addItem = useCallback(
        (item: EditorToolbarItem) => {
            onItemsChange([...items, item]);
        },
        [items, onItemsChange]
    );

    const addSeparator = useCallback(() => {
        onItemsChange([...items, { type: 'separator' }]);
    }, [items, onItemsChange]);

    return (
        <View style={styles.container}>
            <Text style={[sharedStyles.controlHint, { color: chrome.controlHintColor }]}>
                Drag the handle to reorder, tap {REMOVE_GLYPH} to remove. The editor receives this
                exact array as `toolbarItems`.
            </Text>

            <View style={styles.itemList}>
                {items.map((item, index) => {
                    const isDragged = dragIndex === index;
                    const label = getItemLabel(item);
                    return (
                        <Animated.View
                            key={`${getItemId(item)}:${index}`}
                            style={[
                                styles.itemRow,
                                { backgroundColor: chrome.controlSurfaceColor },
                                isDragged
                                    ? {
                                          transform: [{ translateY: panY }, { scale: dragScale }],
                                          zIndex: 10,
                                      }
                                    : { transform: [{ translateY: shiftValues.current[index] }] },
                            ]}>
                            <View
                                accessible
                                accessibilityRole='adjustable'
                                accessibilityLabel={`Reorder ${label}, position ${index + 1} of ${items.length}`}
                                style={styles.dragHandle}
                                {...respondersRef.current[index].panHandlers}>
                                <Text style={[styles.dragIcon, { color: chrome.controlHintColor }]}>
                                    {DRAG_HANDLE_GLYPH}
                                </Text>
                            </View>

                            <Text
                                style={[
                                    styles.itemLabelText,
                                    { color: chrome.controlLabelColor },
                                    item.type === 'separator' && {
                                        color: chrome.controlHintColor,
                                        fontStyle: 'italic',
                                    },
                                ]}
                                numberOfLines={1}>
                                {label}
                            </Text>

                            <Text style={[styles.itemType, { color: chrome.controlHintColor }]}>
                                {item.type}
                            </Text>

                            <Pressable
                                accessibilityRole='button'
                                accessibilityLabel={`Remove ${label}`}
                                style={styles.removeButton}
                                onPress={() => removeItem(index)}>
                                <Text
                                    style={[styles.removeText, { color: chrome.destructiveColor }]}>
                                    {REMOVE_GLYPH}
                                </Text>
                            </Pressable>
                        </Animated.View>
                    );
                })}
            </View>

            {availableItems.length > 0 ? (
                <View style={styles.addSection}>
                    <Text style={[sharedStyles.sectionLabel, { color: chrome.sectionLabelColor }]}>
                        Available
                    </Text>
                    <View style={styles.addPool}>
                        {availableItems.map((item) => (
                            <Pressable
                                key={getItemId(item)}
                                accessibilityRole='button'
                                accessibilityLabel={`Add ${getItemLabel(item)}`}
                                style={[
                                    styles.addChip,
                                    { backgroundColor: chrome.controlSurfaceColor },
                                ]}
                                onPress={() => addItem(item)}>
                                <Text
                                    style={[
                                        styles.addChipText,
                                        { color: chrome.controlSurfaceTextColor },
                                    ]}>
                                    + {getItemLabel(item)}
                                </Text>
                            </Pressable>
                        ))}
                    </View>
                </View>
            ) : null}

            <View style={styles.footerRow}>
                <Pressable
                    accessibilityRole='button'
                    accessibilityLabel='Add separator'
                    style={[styles.addChip, { backgroundColor: chrome.controlSurfaceColor }]}
                    onPress={addSeparator}>
                    <Text style={[styles.addChipText, { color: chrome.controlSurfaceTextColor }]}>
                        + Separator
                    </Text>
                </Pressable>

                <Pressable
                    accessibilityRole='button'
                    accessibilityLabel='Reset toolbar items to package defaults'
                    style={[styles.addChip, { backgroundColor: chrome.controlSurfaceColor }]}
                    onPress={onReset}>
                    <Text style={[styles.addChipText, { color: chrome.controlSurfaceTextColor }]}>
                        Reset to defaults
                    </Text>
                </Pressable>
            </View>
        </View>
    );
}

export const ToolbarItemsEditor = React.memo(ToolbarItemsEditorInner);

const styles = StyleSheet.create({
    container: {
        gap: SPACE.md,
    },
    itemList: {
        gap: ITEM_GAP,
    },
    itemRow: {
        height: ITEM_HEIGHT,
        flexDirection: 'row',
        alignItems: 'center',
        borderRadius: RADIUS,
        paddingRight: SPACE.xs,
    },
    dragHandle: {
        width: DRAG_HANDLE_WIDTH,
        height: ITEM_HEIGHT,
        justifyContent: 'center',
        alignItems: 'center',
    },
    dragIcon: {
        fontSize: FONT_SIZE.heading,
        fontWeight: '700',
    },
    itemLabelText: {
        flex: 1,
        fontSize: FONT_SIZE.hint,
        fontWeight: '600',
    },
    itemType: {
        fontSize: FONT_SIZE.section,
        marginRight: SPACE.xs,
    },
    removeButton: {
        width: REMOVE_BUTTON_SIZE,
        height: REMOVE_BUTTON_SIZE,
        justifyContent: 'center',
        alignItems: 'center',
    },
    removeText: {
        fontSize: FONT_SIZE.heading,
        fontWeight: '600',
    },
    addSection: {
        gap: SPACE.sm,
    },
    addPool: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: SPACE.sm,
    },
    footerRow: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: SPACE.sm,
    },
    addChip: {
        paddingHorizontal: SPACE.md,
        minHeight: MIN_TOUCH_TARGET,
        justifyContent: 'center',
        borderRadius: RADIUS,
    },
    addChipText: {
        fontSize: FONT_SIZE.hint,
        fontWeight: '600',
    },
});
