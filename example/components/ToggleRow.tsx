import React from 'react';
import { StyleSheet, Switch, Text, View } from 'react-native';

import { SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome } from '../themePresets';

type ToggleRowProps = {
    label: string;
    hint?: string;
    value: boolean;
    onChange: (value: boolean) => void;
    chrome: ExampleAppChrome;
};

function ToggleRowInner({ label, hint, value, onChange, chrome }: ToggleRowProps) {
    return (
        <View style={styles.row}>
            <View style={styles.text}>
                <Text style={[sharedStyles.controlLabel, { color: chrome.controlLabelColor }]}>
                    {label}
                </Text>
                {hint != null ? (
                    <Text style={[sharedStyles.controlHint, { color: chrome.controlHintColor }]}>
                        {hint}
                    </Text>
                ) : null}
            </View>
            <Switch
                accessibilityLabel={label}
                accessibilityHint={hint}
                value={value}
                onValueChange={onChange}
                trackColor={{
                    false: chrome.switchTrackColor,
                    true: chrome.switchTrackActiveColor,
                }}
                thumbColor={chrome.switchThumbColor}
            />
        </View>
    );
}

export const ToggleRow = React.memo(ToggleRowInner);

const styles = StyleSheet.create({
    row: {
        flexDirection: 'row',
        alignItems: 'flex-start',
        justifyContent: 'space-between',
        gap: SPACE.md,
    },
    text: {
        flexShrink: 1,
        gap: SPACE.xs,
    },
});
