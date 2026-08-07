import React from 'react';
import { Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import type { ChoiceOption } from '../constants';
import { FONT_SIZE, MIN_TOUCH_TARGET, RADIUS, SPACE } from '../designTokens';
import type { ExampleAppChrome } from '../themePresets';

type ChoiceRowProps<TValue extends string | number | boolean> = {
    options: readonly ChoiceOption<TValue>[];
    value: TValue;
    onChange: (value: TValue) => void;
    chrome: ExampleAppChrome;
    accessibilityLabel: string;
    /** Stretch options to share the row equally. Ignored when scrollable. */
    fill?: boolean;
    /** Scroll horizontally instead of wrapping. */
    scrollable?: boolean;
};

function ChoiceRowInner<TValue extends string | number | boolean>({
    options,
    value,
    onChange,
    chrome,
    accessibilityLabel,
    fill = false,
    scrollable = false,
}: ChoiceRowProps<TValue>) {
    const items = options.map((option) => {
        const selected = option.value === value;
        return (
            <Pressable
                key={String(option.value)}
                accessibilityRole='radio'
                accessibilityLabel={option.label}
                accessibilityState={{ selected }}
                onPress={() => onChange(option.value)}
                style={[
                    styles.option,
                    fill && !scrollable && styles.optionFill,
                    {
                        backgroundColor: selected
                            ? chrome.controlSelectedColor
                            : chrome.controlSurfaceColor,
                    },
                ]}>
                <Text
                    numberOfLines={1}
                    style={[
                        styles.optionText,
                        {
                            color: selected
                                ? chrome.controlSelectedTextColor
                                : chrome.controlSurfaceTextColor,
                        },
                    ]}>
                    {option.label}
                </Text>
            </Pressable>
        );
    });

    if (scrollable) {
        return (
            <ScrollView
                horizontal
                accessibilityRole='radiogroup'
                accessibilityLabel={accessibilityLabel}
                showsHorizontalScrollIndicator={false}
                keyboardShouldPersistTaps='always'
                contentContainerStyle={styles.scrollRow}>
                {items}
            </ScrollView>
        );
    }

    return (
        <View
            accessibilityRole='radiogroup'
            accessibilityLabel={accessibilityLabel}
            style={styles.row}>
            {items}
        </View>
    );
}

/** memo through a cast so the generic parameter survives. */
export const ChoiceRow = React.memo(ChoiceRowInner) as typeof ChoiceRowInner;

const styles = StyleSheet.create({
    row: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: SPACE.sm,
    },
    scrollRow: {
        flexDirection: 'row',
        gap: SPACE.sm,
        paddingRight: SPACE.sm,
    },
    option: {
        borderRadius: RADIUS,
        paddingHorizontal: SPACE.lg,
        // Guarantees 44pt rather than inferring it from padding plus line height.
        minHeight: MIN_TOUCH_TARGET,
        alignItems: 'center',
        justifyContent: 'center',
    },
    optionFill: {
        flexGrow: 1,
        flexBasis: 0,
        minWidth: 0,
    },
    optionText: {
        fontSize: FONT_SIZE.hint,
        fontWeight: '700',
    },
});
