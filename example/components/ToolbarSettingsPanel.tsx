import React from 'react';
import { StyleSheet, Text, View } from 'react-native';
import Slider from '@react-native-community/slider';
import type { EditorToolbarItem, EditorToolbarTheme } from '@apollohg/react-native-prose-editor';
import type { ExampleThemePreset } from '../themePresets';
import { TOOLBAR_COLOR_FIELDS, type ToolbarColorKey } from '../constants';
import { sharedStyles } from '../sharedStyles';
import { ColorField } from './ColorField';
import { ToolbarItemsEditor } from './ToolbarItemsEditor';

type ToolbarSettingsPanelProps = {
  toolbarItems: readonly EditorToolbarItem[];
  onToolbarItemsChange: (items: EditorToolbarItem[]) => void;
  toolbarTheme: Required<EditorToolbarTheme>;
  onToolbarThemeChange: (
    updater: (current: Required<EditorToolbarTheme>) => Required<EditorToolbarTheme>
  ) => void;
  expandedColor: ToolbarColorKey | null;
  onExpandedColorChange: (key: ToolbarColorKey | null) => void;
  sliderTheme: ExampleThemePreset['slider'];
  appChrome: ExampleThemePreset['appChrome'];
};

export function ToolbarSettingsPanel({
  toolbarItems,
  onToolbarItemsChange,
  toolbarTheme,
  onToolbarThemeChange,
  expandedColor,
  onExpandedColorChange,
  sliderTheme,
  appChrome,
}: ToolbarSettingsPanelProps) {
  const updateNumeric = (
    key: 'borderRadius' | 'borderWidth' | 'buttonBorderRadius' | 'keyboardOffset' | 'horizontalInset',
    value: number
  ) => {
    onToolbarThemeChange((current) => ({ ...current, [key]: value }));
  };

  const updateColor = (key: ToolbarColorKey, value: string) => {
    onToolbarThemeChange((current) => ({ ...current, [key]: value }));
  };

  return (
    <View style={sharedStyles.settingsPanel}>
      <ToolbarItemsEditor
        items={toolbarItems}
        onItemsChange={onToolbarItemsChange}
        appChrome={appChrome}
      />

      <View style={styles.toolbarCard}>
        <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
          Toolbar Theme
        </Text>
        <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
          Tweak every toolbar token and confirm the styling applies on both the iOS accessory bar and Android toolbar.
        </Text>

        <View style={sharedStyles.inputRow}>
          <View style={sharedStyles.inputGroup}>
            <View style={sharedStyles.sliderHeader}>
              <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                Toolbar Radius
              </Text>
              <Text style={[sharedStyles.sliderValue, { color: appChrome.sliderValueColor }]}>
                {toolbarTheme.borderRadius}px
              </Text>
            </View>
            <Slider
              style={sharedStyles.slider}
              minimumValue={0}
              maximumValue={24}
              step={1}
              minimumTrackTintColor={sliderTheme.minimumTrackTintColor}
              maximumTrackTintColor={sliderTheme.maximumTrackTintColor}
              thumbTintColor={sliderTheme.thumbTintColor}
              value={toolbarTheme.borderRadius}
              onValueChange={(value) => updateNumeric('borderRadius', value)}
            />
          </View>

          <View style={sharedStyles.inputGroup}>
            <View style={sharedStyles.sliderHeader}>
              <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                Button Radius
              </Text>
              <Text style={[sharedStyles.sliderValue, { color: appChrome.sliderValueColor }]}>
                {toolbarTheme.buttonBorderRadius}px
              </Text>
            </View>
            <Slider
              style={sharedStyles.slider}
              minimumValue={0}
              maximumValue={20}
              step={1}
              minimumTrackTintColor={sliderTheme.minimumTrackTintColor}
              maximumTrackTintColor={sliderTheme.maximumTrackTintColor}
              thumbTintColor={sliderTheme.thumbTintColor}
              value={toolbarTheme.buttonBorderRadius}
              onValueChange={(value) => updateNumeric('buttonBorderRadius', value)}
            />
          </View>
        </View>

        <View style={sharedStyles.inputRow}>
          <View style={sharedStyles.inputGroup}>
            <View style={sharedStyles.sliderHeader}>
              <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                Border Width
              </Text>
              <Text style={[sharedStyles.sliderValue, { color: appChrome.sliderValueColor }]}>
                {toolbarTheme.borderWidth}px
              </Text>
            </View>
            <Slider
              style={sharedStyles.slider}
              minimumValue={0}
              maximumValue={8}
              step={0.5}
              minimumTrackTintColor={sliderTheme.minimumTrackTintColor}
              maximumTrackTintColor={sliderTheme.maximumTrackTintColor}
              thumbTintColor={sliderTheme.thumbTintColor}
              value={toolbarTheme.borderWidth}
              onValueChange={(value) => updateNumeric('borderWidth', value)}
            />
          </View>

          <View style={sharedStyles.inputGroup}>
            <View style={sharedStyles.sliderHeader}>
              <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                Keyboard Offset
              </Text>
              <Text style={[sharedStyles.sliderValue, { color: appChrome.sliderValueColor }]}>
                {toolbarTheme.keyboardOffset}px
              </Text>
            </View>
            <Slider
              style={sharedStyles.slider}
              minimumValue={0}
              maximumValue={24}
              step={1}
              minimumTrackTintColor={sliderTheme.minimumTrackTintColor}
              maximumTrackTintColor={sliderTheme.maximumTrackTintColor}
              thumbTintColor={sliderTheme.thumbTintColor}
              value={toolbarTheme.keyboardOffset}
              onValueChange={(value) => updateNumeric('keyboardOffset', value)}
            />
          </View>

          <View style={sharedStyles.inputGroup}>
            <View style={sharedStyles.sliderHeader}>
              <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                Horizontal Inset
              </Text>
              <Text style={[sharedStyles.sliderValue, { color: appChrome.sliderValueColor }]}>
                {toolbarTheme.horizontalInset}px
              </Text>
            </View>
            <Slider
              style={sharedStyles.slider}
              minimumValue={0}
              maximumValue={32}
              step={1}
              minimumTrackTintColor={sliderTheme.minimumTrackTintColor}
              maximumTrackTintColor={sliderTheme.maximumTrackTintColor}
              thumbTintColor={sliderTheme.thumbTintColor}
              value={toolbarTheme.horizontalInset}
              onValueChange={(value) => updateNumeric('horizontalInset', value)}
            />
          </View>
        </View>

        <View style={styles.colorGrid}>
          {TOOLBAR_COLOR_FIELDS.map(({ key, label }) => (
            <ColorField
              key={key}
              label={label}
              value={toolbarTheme[key]}
              chrome={appChrome}
              isExpanded={expandedColor === key}
              onToggle={() =>
                onExpandedColorChange(expandedColor === key ? null : key)
              }
              onChange={(value) => updateColor(key, value)}
            />
          ))}
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  toolbarCard: {
    gap: 12,
    paddingTop: 4,
  },
  colorGrid: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    justifyContent: 'space-between',
    gap: 12,
  },
});
