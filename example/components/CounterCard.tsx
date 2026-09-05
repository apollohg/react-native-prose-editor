import { useCallback } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import { defineAtomNode, type AtomComponentProps } from 'react-native-rich-text-editor';

import { FONT_SIZE, LINE_HEIGHT, MIN_TOUCH_TARGET, PALETTE, RADIUS, SPACE } from '../theme';

const COUNTER_CARD_HEIGHT = 96;
const DEFAULT_TITLE = 'Untitled counter';

type CounterAttrs = { title: string; count: number };

function CounterCard({
    attrs,
    selected,
    readOnly,
    interactive,
    isViewer,
    updateAttrs,
    updatePending,
    updateError,
}: AtomComponentProps<CounterAttrs>) {
    const { title, count } = attrs;
    const adjust = useCallback(
        (delta: number) => {
            void updateAttrs((current) => ({ count: current.count + delta })).catch(() => {});
        },
        [updateAttrs]
    );

    const decrement = useCallback(() => adjust(-1), [adjust]);
    const increment = useCallback(() => adjust(1), [adjust]);

    return (
        <View
            accessibilityLabel={`${title}, ${count}`}
            accessibilityState={{ selected }}
            style={[styles.card, selected && styles.cardSelected]}>
            <View style={styles.summary}>
                <Text numberOfLines={1} style={styles.title}>
                    {title}
                </Text>
                <Text style={styles.hint}>
                    {updateError?.message ??
                        (updatePending ? 'Saving…' : undefined) ??
                        (readOnly
                            ? 'Read-only counter'
                            : isViewer
                              ? 'Viewer counter'
                              : 'Custom block')}
                </Text>
            </View>
            <View style={styles.stepper}>
                <Pressable
                    disabled={readOnly || interactive === false}
                    accessibilityState={{ disabled: readOnly || interactive === false }}
                    accessibilityRole='button'
                    accessibilityLabel='Subtract one'
                    hitSlop={SPACE.xs}
                    onPress={decrement}
                    style={({ pressed }) => [styles.stepButton, pressed && styles.pressed]}>
                    <Text style={styles.stepLabel}>−</Text>
                </Pressable>
                <Text style={styles.count}>{count}</Text>
                <Pressable
                    disabled={readOnly || interactive === false}
                    accessibilityState={{ disabled: readOnly || interactive === false }}
                    accessibilityRole='button'
                    accessibilityLabel='Add one'
                    hitSlop={SPACE.xs}
                    onPress={increment}
                    style={({ pressed }) => [styles.stepButton, pressed && styles.pressed]}>
                    <Text style={styles.stepLabel}>+</Text>
                </Pressable>
            </View>
        </View>
    );
}

export const counterCardAtom = defineAtomNode({
    name: 'counterCard',
    attrs: {
        title: { type: 'string', default: DEFAULT_TITLE },
        count: { type: 'number', default: 0 },
    },
    html: {
        tag: 'div',
        staticAttrs: { 'data-type': 'counter-card' },
        attrMap: { title: 'data-title', count: 'data-count' },
    },
    component: CounterCard,
    estimatedHeight: COUNTER_CARD_HEIGHT,
});

const styles = StyleSheet.create({
    card: {
        minHeight: COUNTER_CARD_HEIGHT,
        flexDirection: 'row',
        alignItems: 'center',
        gap: SPACE.md,
        paddingVertical: SPACE.md,
        paddingHorizontal: SPACE.lg,
        borderRadius: RADIUS.card,
        borderWidth: 1,
        borderColor: PALETTE.hairline,
        backgroundColor: PALETTE.wash,
    },
    cardSelected: {
        borderColor: PALETTE.spruce,
        backgroundColor: PALETTE.spruceTint,
    },
    summary: {
        flex: 1,
        gap: SPACE.xs,
    },
    title: {
        color: PALETTE.ink,
        fontSize: FONT_SIZE.body,
        lineHeight: LINE_HEIGHT.body,
        fontWeight: '600',
    },
    hint: {
        color: PALETTE.inkMuted,
        fontSize: FONT_SIZE.caption,
        lineHeight: LINE_HEIGHT.caption,
    },
    stepper: {
        flexDirection: 'row',
        alignItems: 'center',
        gap: SPACE.sm,
    },
    stepButton: {
        width: MIN_TOUCH_TARGET,
        height: MIN_TOUCH_TARGET,
        alignItems: 'center',
        justifyContent: 'center',
        borderRadius: MIN_TOUCH_TARGET / 2,
        backgroundColor: PALETTE.paper,
        borderWidth: 1,
        borderColor: PALETTE.hairline,
    },
    pressed: {
        opacity: 0.6,
    },
    stepLabel: {
        color: PALETTE.spruce,
        fontSize: FONT_SIZE.stat,
        lineHeight: LINE_HEIGHT.stat,
        fontWeight: '600',
    },
    count: {
        minWidth: SPACE.xxl,
        textAlign: 'center',
        color: PALETTE.ink,
        fontSize: FONT_SIZE.stat,
        lineHeight: LINE_HEIGHT.stat,
        fontWeight: '700',
        fontVariant: ['tabular-nums'],
    },
});
