import React from 'react';
import { StyleSheet, Switch, Text, View } from 'react-native';
import Slider from '@react-native-community/slider';
import type { NativeRichTextEditorToolbarPlacement } from '@apollohg/react-native-prose-editor';
import type { ExampleThemePreset } from '../themePresets';
import { EXAMPLE_MENTION_SUGGESTIONS } from '../constants';
import { sharedStyles } from '../sharedStyles';

type EditorSettingsPanelProps = {
  baseFontSize: number;
  onBaseFontSizeChange: (size: number) => void;
  autoGrow: boolean;
  onAutoGrowChange: (value: boolean) => void;
  toolbarPlacement: NativeRichTextEditorToolbarPlacement;
  onToolbarPlacementChange: (value: NativeRichTextEditorToolbarPlacement) => void;
  mentionsEnabled: boolean;
  onMentionsEnabledChange: (value: boolean) => void;
  sliderTheme: ExampleThemePreset['slider'];
  appChrome: ExampleThemePreset['appChrome'];
};

export function EditorSettingsPanel({
  baseFontSize,
  onBaseFontSizeChange,
  autoGrow,
  onAutoGrowChange,
  toolbarPlacement,
  onToolbarPlacementChange,
  mentionsEnabled,
  onMentionsEnabledChange,
  sliderTheme,
  appChrome,
}: EditorSettingsPanelProps) {
  const inlineToolbar = toolbarPlacement === 'inline';

  return (
    <View style={sharedStyles.settingsPanel}>
      <View>
        <View style={sharedStyles.sliderHeader}>
          <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
            Base Font
          </Text>
          <Text style={[sharedStyles.sliderValue, { color: appChrome.sliderValueColor }]}>
            {baseFontSize}px
          </Text>
        </View>
        <Slider
          style={sharedStyles.slider}
          minimumValue={12}
          maximumValue={30}
          step={1}
          minimumTrackTintColor={sliderTheme.minimumTrackTintColor}
          maximumTrackTintColor={sliderTheme.maximumTrackTintColor}
          thumbTintColor={sliderTheme.thumbTintColor}
          value={baseFontSize}
          onValueChange={onBaseFontSizeChange}
        />
      </View>

      <View style={styles.switchRow}>
        <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
          Auto Grow
        </Text>
        <Switch
          value={autoGrow}
          onValueChange={onAutoGrowChange}
          trackColor={{
            false: appChrome.tabBorderColor,
            true: appChrome.tabActiveBorderColor,
          }}
          thumbColor={autoGrow ? appChrome.tabActiveTextColor : appChrome.tabTextColor}
        />
      </View>

      <View style={styles.switchRow}>
        <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
          Inline Toolbar
        </Text>
        <Switch
          value={inlineToolbar}
          onValueChange={(value) => onToolbarPlacementChange(value ? 'inline' : 'keyboard')}
          trackColor={{
            false: appChrome.tabBorderColor,
            true: appChrome.tabActiveBorderColor,
          }}
          thumbColor={inlineToolbar ? appChrome.tabActiveTextColor : appChrome.tabTextColor}
        />
      </View>

      <View style={styles.editorOptionCard}>
        <View style={styles.switchRow}>
          <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
            Mentions
          </Text>
          <Switch
            value={mentionsEnabled}
            onValueChange={onMentionsEnabledChange}
            trackColor={{
              false: appChrome.tabBorderColor,
              true: appChrome.tabActiveBorderColor,
            }}
            thumbColor={mentionsEnabled ? appChrome.tabActiveTextColor : appChrome.tabTextColor}
          />
        </View>

        <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
          Demo suggestions: {EXAMPLE_MENTION_SUGGESTIONS.map((item) => item.label).join(', ')}
        </Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  editorOptionCard: {
    gap: 10,
    paddingTop: 4,
  },
  switchRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingTop: 4,
  },
});
