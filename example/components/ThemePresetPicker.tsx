import React, { useMemo } from 'react';
import { StyleSheet, Text, View } from 'react-native';

import type { ChoiceOption } from '../constants';
import { SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome, ExampleThemePreset } from '../themePresets';
import { ChoiceRow } from './ChoiceRow';

type ThemePresetPickerProps = {
    presets: readonly ExampleThemePreset[];
    selectedId: string;
    onSelect: (id: string) => void;
    chrome: ExampleAppChrome;
};

function ThemePresetPickerInner({ presets, selectedId, onSelect, chrome }: ThemePresetPickerProps) {
    const options = useMemo<readonly ChoiceOption<string>[]>(
        () => presets.map((preset) => ({ value: preset.id, label: preset.label })),
        [presets]
    );

    // Presets differ in geometry too, so which one is selected changes what a render proves.
    const covers = presets.find((preset) => preset.id === selectedId)?.covers;

    return (
        <View style={styles.container}>
            <Text style={[sharedStyles.controlHint, { color: chrome.controlHintColor }]}>
                Each preset covers the whole token surface, colours and geometry both. Picking one
                reloads the editor and toolbar defaults; they are coverage fixtures, not recommended
                themes.
            </Text>

            <ChoiceRow
                options={options}
                value={selectedId}
                onChange={onSelect}
                chrome={chrome}
                accessibilityLabel='Theme preset'
            />

            {covers != null ? (
                <Text style={[sharedStyles.controlHint, { color: chrome.sectionLabelColor }]}>
                    {covers}
                </Text>
            ) : null}
        </View>
    );
}

export const ThemePresetPicker = React.memo(ThemePresetPickerInner);

const styles = StyleSheet.create({
    container: {
        gap: SPACE.md,
    },
});
