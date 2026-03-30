import React from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import type { ExampleThemePreset } from '../themePresets';
import { sharedStyles } from '../sharedStyles';

type ThemePresetPickerProps = {
  presets: readonly ExampleThemePreset[];
  selectedId: string;
  onSelect: (id: string) => void;
  appChrome: ExampleThemePreset['appChrome'];
};

export function ThemePresetPicker({
  presets,
  selectedId,
  onSelect,
  appChrome,
}: ThemePresetPickerProps) {
  return (
    <View style={styles.container}>
      <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
        Pick a preset to reload the editor and toolbar defaults.
      </Text>

      <View style={styles.themePicker}>
        {presets.map((preset) => (
          <Pressable
            key={preset.id}
            style={[
              styles.themeOption,
              {
                borderColor: appChrome.chipBorderColor,
                backgroundColor: appChrome.chipBackgroundColor,
              },
              selectedId === preset.id && {
                borderColor: appChrome.chipActiveBorderColor,
                backgroundColor: appChrome.chipActiveBackgroundColor,
              },
            ]}
            onPress={() => onSelect(preset.id)}
          >
            <Text
              style={[
                styles.themeOptionText,
                { color: appChrome.chipTextColor },
                selectedId === preset.id && {
                  color: appChrome.chipActiveTextColor,
                },
              ]}
            >
              {preset.label}
            </Text>
          </Pressable>
        ))}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    gap: 12,
  },
  themePicker: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 10,
  },
  themeOption: {
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderRadius: 999,
    borderWidth: 1,
  },
  themeOptionText: {
    fontSize: 13,
    fontWeight: '700',
  },
});
