import React from 'react';
import { Pressable, StyleSheet, Text } from 'react-native';

import { FONT_SIZE, MIN_TOUCH_TARGET, RADIUS, SPACE } from '../designTokens';
import type { ExampleAppChrome } from '../themePresets';

type ActionButtonProps = {
    label: string;
    onPress: () => void;
    chrome: ExampleAppChrome;
    disabled?: boolean;
    /** Quieter treatment for secondary actions in a row of several. */
    tone?: 'primary' | 'secondary';
    accessibilityHint?: string;
};

function ActionButtonInner({
    label,
    onPress,
    chrome,
    disabled = false,
    tone = 'primary',
    accessibilityHint,
}: ActionButtonProps) {
    const primary = tone === 'primary';

    return (
        <Pressable
            accessibilityRole='button'
            accessibilityLabel={label}
            accessibilityHint={accessibilityHint}
            accessibilityState={{ disabled }}
            disabled={disabled}
            onPress={onPress}
            style={({ pressed }) => [
                styles.button,
                primary
                    ? { backgroundColor: chrome.actionButtonBackgroundColor }
                    : { backgroundColor: chrome.controlSurfaceColor },
                pressed && styles.pressed,
                disabled && styles.disabled,
            ]}>
            <Text
                numberOfLines={1}
                style={[
                    styles.label,
                    {
                        color: primary
                            ? chrome.actionButtonTextColor
                            : chrome.controlSurfaceTextColor,
                    },
                ]}>
                {label}
            </Text>
        </Pressable>
    );
}

export const ActionButton = React.memo(ActionButtonInner);

const styles = StyleSheet.create({
    button: {
        paddingHorizontal: SPACE.lg,
        minHeight: MIN_TOUCH_TARGET,
        borderRadius: RADIUS,
        alignItems: 'center',
        justifyContent: 'center',
    },
    pressed: {
        opacity: 0.7,
    },
    disabled: {
        opacity: 0.4,
    },
    label: {
        fontSize: FONT_SIZE.hint,
        fontWeight: '700',
    },
});
