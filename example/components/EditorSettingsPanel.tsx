import React from 'react';
import { StyleSheet, Switch, Text, View } from 'react-native';
import Slider from '@react-native-community/slider';
import type { ExampleThemePreset } from '../themePresets';
import { EXAMPLE_MENTION_SUGGESTIONS } from '../constants';
import { sharedStyles } from '../sharedStyles';
import { ColorField } from './ColorField';

type EditorSettingsPanelProps = {
    baseFontSize: number;
    onBaseFontSizeChange: (size: number) => void;
    mentionsEnabled: boolean;
    onMentionsEnabledChange: (value: boolean) => void;
    blockquoteBorderColor: string;
    onBlockquoteBorderColorChange: (value: string) => void;
    expandedColor: 'blockquoteBorderColor' | null;
    onExpandedColorChange: (key: 'blockquoteBorderColor' | null) => void;
    sliderTheme: ExampleThemePreset['slider'];
    appChrome: ExampleThemePreset['appChrome'];
};

export function EditorSettingsPanel({
    baseFontSize,
    onBaseFontSizeChange,
    mentionsEnabled,
    onMentionsEnabledChange,
    blockquoteBorderColor,
    onBlockquoteBorderColorChange,
    expandedColor,
    onExpandedColorChange,
    sliderTheme,
    appChrome,
}: EditorSettingsPanelProps) {
    return (
        <View style={sharedStyles.settingsPanel}>
            <View>
                <View style={sharedStyles.sliderHeader}>
                    <Text
                        style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
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

            <View style={styles.editorOptionCard}>
                <View style={styles.switchRow}>
                    <Text
                        style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                        Mentions
                    </Text>
                    <Switch
                        value={mentionsEnabled}
                        onValueChange={onMentionsEnabledChange}
                        trackColor={{
                            false: appChrome.tabBorderColor,
                            true: appChrome.tabActiveBorderColor,
                        }}
                        thumbColor={
                            mentionsEnabled ? appChrome.tabActiveTextColor : appChrome.tabTextColor
                        }
                    />
                </View>

                <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
                    Demo suggestions:{' '}
                    {EXAMPLE_MENTION_SUGGESTIONS.map((item) => item.label).join(', ')}
                </Text>
            </View>

            <View style={styles.editorOptionCard}>
                <Text style={[sharedStyles.controlLabel, { color: appChrome.controlLabelColor }]}>
                    Blockquote
                </Text>
                <Text style={[sharedStyles.controlHint, { color: appChrome.controlHintColor }]}>
                    Adjust the quote line color to confirm blockquote theme updates apply live on
                    both iOS and Android.
                </Text>
                <View style={sharedStyles.inputRow}>
                    <ColorField
                        label='Quote Line'
                        value={blockquoteBorderColor}
                        chrome={appChrome}
                        isExpanded={expandedColor === 'blockquoteBorderColor'}
                        onToggle={() =>
                            onExpandedColorChange(
                                expandedColor === 'blockquoteBorderColor'
                                    ? null
                                    : 'blockquoteBorderColor'
                            )
                        }
                        onChange={onBlockquoteBorderColorChange}
                    />
                </View>
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
