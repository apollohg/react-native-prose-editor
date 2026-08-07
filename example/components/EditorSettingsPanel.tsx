import React, { useCallback } from 'react';
import { View } from 'react-native';

import { EXAMPLE_MENTION_SUGGESTIONS } from '../constants';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome, ExampleThemePreset } from '../themePresets';
import { ColorField } from './ColorField';
import { PanelSection } from './PanelSection';
import { SliderField } from './SliderField';
import { ToggleRow } from './ToggleRow';

const BASE_FONT_MIN = 12;
const BASE_FONT_MAX = 30;
const BASE_FONT_STEP = 1;

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
    chrome: ExampleAppChrome;
};

function EditorSettingsPanelInner({
    baseFontSize,
    onBaseFontSizeChange,
    mentionsEnabled,
    onMentionsEnabledChange,
    blockquoteBorderColor,
    onBlockquoteBorderColorChange,
    expandedColor,
    onExpandedColorChange,
    sliderTheme,
    chrome,
}: EditorSettingsPanelProps) {
    const toggleBlockquoteColor = useCallback(
        () =>
            onExpandedColorChange(
                expandedColor === 'blockquoteBorderColor' ? null : 'blockquoteBorderColor'
            ),
        [expandedColor, onExpandedColorChange]
    );

    return (
        <View style={sharedStyles.settingsPanel}>
            <SliderField
                label='Base font'
                value={baseFontSize}
                min={BASE_FONT_MIN}
                max={BASE_FONT_MAX}
                step={BASE_FONT_STEP}
                onCommit={onBaseFontSizeChange}
                chrome={chrome}
                sliderTheme={sliderTheme}
            />

            <PanelSection
                title='Mentions'
                hint={`Adds the mentions addon and schema. Suggestions: ${EXAMPLE_MENTION_SUGGESTIONS.map(
                    (item) => item.label
                ).join(', ')}.`}
                chrome={chrome}>
                <ToggleRow
                    label='Enable mentions'
                    hint='Type @ after a space, on a blank line, or after punctuation.'
                    value={mentionsEnabled}
                    onChange={onMentionsEnabledChange}
                    chrome={chrome}
                />
            </PanelSection>

            <PanelSection
                title='Blockquote'
                hint='Confirms blockquote theme updates apply live on both platforms.'
                chrome={chrome}>
                <View style={sharedStyles.columnGrid}>
                    <ColorField
                        label='Quote line'
                        value={blockquoteBorderColor}
                        chrome={chrome}
                        isExpanded={expandedColor === 'blockquoteBorderColor'}
                        onToggle={toggleBlockquoteColor}
                        onChange={onBlockquoteBorderColorChange}
                    />
                </View>
            </PanelSection>
        </View>
    );
}

export const EditorSettingsPanel = React.memo(EditorSettingsPanelInner);
