import React, { useCallback, useMemo, useRef, useState } from 'react';
import {
  Animated,
  PanResponder,
  Pressable,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import {
  DEFAULT_EDITOR_TOOLBAR_ITEMS,
  type EditorToolbarItem,
} from '@apollohg/react-native-prose-editor';
import type { ExampleThemePreset } from '../themePresets';
import { sharedStyles } from '../sharedStyles';

type ToolbarItemsEditorProps = {
  items: readonly EditorToolbarItem[];
  onItemsChange: (items: EditorToolbarItem[]) => void;
  appChrome: ExampleThemePreset['appChrome'];
};

function getItemId(item: EditorToolbarItem): string {
  switch (item.type) {
    case 'mark':
      return `mark:${item.mark}`;
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
    default:
      return item.label;
  }
}

const ITEM_HEIGHT = 36;
const ITEM_GAP = 4;
const ITEM_STRIDE = ITEM_HEIGHT + ITEM_GAP;

export function ToolbarItemsEditor({
  items,
  onItemsChange,
  appChrome,
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

  const animateShifts = useCallback(
    (from: number, to: number) => {
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
    },
    []
  );

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
            toValue: 1.03,
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

          // Reset all native animated values and commit reorder in the
          // same JS frame so React batches the re-render — no flash.
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
    return DEFAULT_EDITOR_TOOLBAR_ITEMS.filter(
      (item) => !activeIds.has(getItemId(item))
    );
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
      <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
        Toolbar Items
      </Text>
      <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
        Drag to reorder, or tap x to remove.
      </Text>

      <View style={styles.itemList}>
        {items.map((item, index) => {
          const isDragged = dragIndex === index;
          return (
            <Animated.View
              key={`${getItemId(item)}:${index}`}
              style={[
                styles.itemRow,
                {
                  borderColor: appChrome.tabBorderColor,
                  backgroundColor: appChrome.cardBackgroundColor,
                },
                isDragged
                  ? {
                      transform: [{ translateY: panY }, { scale: dragScale }],
                      zIndex: 10,
                    }
                  : { transform: [{ translateY: shiftValues.current[index] }] },
              ]}
            >
              <View
                style={styles.dragHandle}
                {...respondersRef.current[index].panHandlers}
              >
                <Text style={[styles.dragIcon, { color: appChrome.controlHintColor }]}>
                  {'\u2261'}
                </Text>
              </View>

              <Text
                style={[
                  styles.itemLabelText,
                  { color: appChrome.controlLabelColor },
                  item.type === 'separator' && {
                    color: appChrome.controlHintColor,
                    fontStyle: 'italic',
                  },
                ]}
                numberOfLines={1}
              >
                {getItemLabel(item)}
              </Text>

              <Text style={[styles.itemType, { color: appChrome.controlHintColor }]}>
                {item.type}
              </Text>

              <Pressable
                style={styles.removeButton}
                onPress={() => removeItem(index)}
                hitSlop={6}
              >
                <Text style={styles.removeText}>{'\u00D7'}</Text>
              </Pressable>
            </Animated.View>
          );
        })}
      </View>

      {availableItems.length > 0 && (
        <View style={styles.addSection}>
          <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
            Available items
          </Text>
          <View style={styles.addPool}>
            {availableItems.map((item) => (
              <Pressable
                key={getItemId(item)}
                style={[
                  styles.addChip,
                  {
                    borderColor: appChrome.chipBorderColor,
                    backgroundColor: appChrome.chipBackgroundColor,
                  },
                ]}
                onPress={() => addItem(item)}
              >
                <Text style={[styles.addChipText, { color: appChrome.chipTextColor }]}>
                  + {getItemLabel(item)}
                </Text>
              </Pressable>
            ))}
          </View>
        </View>
      )}

      <Pressable
        style={[
          styles.addChip,
          {
            borderColor: appChrome.chipBorderColor,
            backgroundColor: appChrome.chipBackgroundColor,
            alignSelf: 'flex-start',
          },
        ]}
        onPress={addSeparator}
      >
        <Text style={[styles.addChipText, { color: appChrome.chipTextColor }]}>
          + Separator
        </Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    gap: 12,
    paddingTop: 4,
  },
  itemList: {
    gap: ITEM_GAP,
  },
  itemRow: {
    height: ITEM_HEIGHT,
    flexDirection: 'row',
    alignItems: 'center',
    borderWidth: 1,
    borderRadius: 8,
    paddingRight: 4,
  },
  dragHandle: {
    width: 32,
    height: ITEM_HEIGHT,
    justifyContent: 'center',
    alignItems: 'center',
  },
  dragIcon: {
    fontSize: 18,
    lineHeight: 20,
    fontWeight: '700',
  },
  itemLabelText: {
    flex: 1,
    fontSize: 13,
    fontWeight: '600',
  },
  itemType: {
    fontSize: 11,
    marginRight: 4,
  },
  removeButton: {
    width: 28,
    height: 28,
    justifyContent: 'center',
    alignItems: 'center',
  },
  removeText: {
    fontSize: 16,
    fontWeight: '600',
    color: '#c44',
  },
  addSection: {
    gap: 8,
  },
  addPool: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 8,
  },
  addChip: {
    paddingHorizontal: 12,
    paddingVertical: 8,
    borderRadius: 999,
    borderWidth: 1,
  },
  addChipText: {
    fontSize: 12,
    fontWeight: '600',
  },
});
