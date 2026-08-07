import React from 'react';
import { Pressable, StyleSheet, Text } from 'react-native';

import { FONT_SIZE, MIN_TOUCH_TARGET, RADIUS, SPACE } from '../designTokens';
import type { ExampleAppChrome } from '../themePresets';

/** Navigation bar action: a flat bar item, not the filled `ActionButton`. */
type HeaderActionButtonProps = {
    label: string;
    onPress: () => void;
    chrome: ExampleAppChrome;
    accessibilityHint?: string;
};

function HeaderActionButtonInner({
    label,
    onPress,
    chrome,
    accessibilityHint,
}: HeaderActionButtonProps) {
    return (
        <Pressable
            accessibilityRole='button'
            accessibilityLabel={label}
            accessibilityHint={accessibilityHint}
            hitSlop={SPACE.sm}
            style={({ pressed }) => [styles.button, pressed ? styles.pressed : null]}
            onPress={onPress}>
            <Text numberOfLines={1} style={[styles.label, { color: chrome.accentColor }]}>
                {label}
            </Text>
        </Pressable>
    );
}

export const HeaderActionButton = React.memo(HeaderActionButtonInner);

const styles = StyleSheet.create({
    button: {
        justifyContent: 'center',
        minHeight: MIN_TOUCH_TARGET,
        paddingHorizontal: SPACE.xs,
        borderRadius: RADIUS,
    },
    // No room for a pressed background in the bar.
    pressed: {
        opacity: 0.55,
    },
    label: {
        fontSize: FONT_SIZE.label,
        fontWeight: '600',
    },
});
