import React, { useCallback } from 'react';
import { View } from 'react-native';
import type {
    EditorToolbarAppearance,
    EditorToolbarTheme,
} from '@apollohg/react-native-prose-editor';

import { TOOLBAR_COLOR_FIELDS, type ChoiceOption, type ToolbarColorKey } from '../constants';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome, ExampleThemePreset } from '../themePresets';
import { ChoiceRow } from './ChoiceRow';
import { ColorField } from './ColorField';
import { PanelSection } from './PanelSection';
import { SliderField } from './SliderField';
import { ToggleRow } from './ToggleRow';

type ToolbarNumericKey =
    | 'borderRadius'
    | 'borderWidth'
    | 'buttonBorderRadius'
    | 'buttonIconSize'
    | 'keyboardOffset'
    | 'horizontalInset'
    | 'marginTop';

const APPEARANCE_OPTIONS: readonly ChoiceOption<EditorToolbarAppearance>[] = [
    { value: 'custom', label: 'Custom' },
    { value: 'native', label: 'Native' },
];

/** Slider bounds per numeric token, plus whether native appearance honours it. */
const NUMERIC_FIELDS: readonly {
    key: ToolbarNumericKey;
    label: string;
    min: number;
    max: number;
    step: number;
    /** Native appearance ignores purely visual tokens. */
    visualOnly: boolean;
}[] = [
    { key: 'borderRadius', label: 'Toolbar radius', min: 0, max: 24, step: 1, visualOnly: true },
    {
        key: 'buttonBorderRadius',
        label: 'Button radius',
        min: 0,
        max: 20,
        step: 1,
        visualOnly: false,
    },
    { key: 'buttonIconSize', label: 'Icon size', min: 8, max: 32, step: 1, visualOnly: false },
    { key: 'borderWidth', label: 'Border width', min: 0, max: 8, step: 0.5, visualOnly: true },
    {
        key: 'keyboardOffset',
        label: 'Keyboard offset',
        min: 0,
        max: 24,
        step: 1,
        visualOnly: false,
    },
    {
        key: 'horizontalInset',
        label: 'Horizontal inset',
        min: 0,
        max: 32,
        step: 1,
        visualOnly: false,
    },
    { key: 'marginTop', label: 'Inline margin', min: 0, max: 24, step: 1, visualOnly: false },
];

/** Rows own their closures so the memo on SliderField and ColorField holds. */
const ToolbarNumericRow = React.memo(function ToolbarNumericRow({
    field,
    value,
    onCommitKey,
    chrome,
    sliderTheme,
}: {
    field: (typeof NUMERIC_FIELDS)[number];
    value: number;
    onCommitKey: (key: ToolbarNumericKey, value: number) => void;
    chrome: ExampleAppChrome;
    sliderTheme: ExampleThemePreset['slider'];
}) {
    const handleCommit = useCallback(
        (next: number) => onCommitKey(field.key, next),
        [field.key, onCommitKey]
    );
    return (
        <View style={sharedStyles.column}>
            <SliderField
                label={field.label}
                value={value}
                min={field.min}
                max={field.max}
                step={field.step}
                onCommit={handleCommit}
                chrome={chrome}
                sliderTheme={sliderTheme}
            />
        </View>
    );
});

const ToolbarColorRow = React.memo(function ToolbarColorRow({
    colorKey,
    label,
    value,
    isExpanded,
    onToggleKey,
    onChangeKey,
    chrome,
}: {
    colorKey: ToolbarColorKey;
    label: string;
    value: string;
    isExpanded: boolean;
    onToggleKey: (key: ToolbarColorKey) => void;
    onChangeKey: (key: ToolbarColorKey, value: string) => void;
    chrome: ExampleAppChrome;
}) {
    const handleToggle = useCallback(() => onToggleKey(colorKey), [colorKey, onToggleKey]);
    const handleChange = useCallback(
        (next: string) => onChangeKey(colorKey, next),
        [colorKey, onChangeKey]
    );
    return (
        <ColorField
            label={label}
            value={value}
            chrome={chrome}
            isExpanded={isExpanded}
            onToggle={handleToggle}
            onChange={handleChange}
        />
    );
});

type ToolbarSettingsPanelProps = {
    toolbarTheme: Required<EditorToolbarTheme>;
    onToolbarThemeChange: (
        updater: (current: Required<EditorToolbarTheme>) => Required<EditorToolbarTheme>
    ) => void;
    expandedColor: ToolbarColorKey | null;
    onExpandedColorChange: (key: ToolbarColorKey | null) => void;
    sliderTheme: ExampleThemePreset['slider'];
    chrome: ExampleAppChrome;
};

function ToolbarSettingsPanelInner({
    toolbarTheme,
    onToolbarThemeChange,
    expandedColor,
    onExpandedColorChange,
    sliderTheme,
    chrome,
}: ToolbarSettingsPanelProps) {
    const isNativeAppearance = toolbarTheme.appearance === 'native';

    const updateAppearance = useCallback(
        (appearance: EditorToolbarAppearance) => {
            onToolbarThemeChange((current) => ({ ...current, appearance }));
        },
        [onToolbarThemeChange]
    );

    const updateShowTopBorder = useCallback(
        (showTopBorder: boolean) => {
            onToolbarThemeChange((current) => ({ ...current, showTopBorder }));
        },
        [onToolbarThemeChange]
    );

    const commitNumeric = useCallback(
        (key: ToolbarNumericKey, value: number) => {
            onToolbarThemeChange((current) => ({ ...current, [key]: value }));
        },
        [onToolbarThemeChange]
    );

    const toggleColorKey = useCallback(
        (key: ToolbarColorKey) => {
            onExpandedColorChange(expandedColor === key ? null : key);
        },
        [expandedColor, onExpandedColorChange]
    );

    const changeColorKey = useCallback(
        (key: ToolbarColorKey, value: string) => {
            onToolbarThemeChange((current) => ({ ...current, [key]: value }));
        },
        [onToolbarThemeChange]
    );

    const numericFields = NUMERIC_FIELDS.filter(
        (field) => !isNativeAppearance || !field.visualOnly
    );

    return (
        <View style={sharedStyles.settingsPanel}>
            <PanelSection
                title='Appearance'
                hint={
                    isNativeAppearance
                        ? 'Native uses platform chrome, while explicit button colours, icon size, and button radius still override its defaults.'
                        : 'Custom honours every token below on both the keyboard toolbar and the inline bubble.'
                }
                chrome={chrome}>
                <ChoiceRow
                    fill
                    options={APPEARANCE_OPTIONS}
                    value={toolbarTheme.appearance}
                    onChange={updateAppearance}
                    chrome={chrome}
                    accessibilityLabel='Toolbar appearance'
                />
            </PanelSection>

            <PanelSection title='Metrics' chrome={chrome}>
                <View style={sharedStyles.columnGrid}>
                    {numericFields.map((field) => (
                        <ToolbarNumericRow
                            key={field.key}
                            field={field}
                            value={toolbarTheme[field.key]}
                            onCommitKey={commitNumeric}
                            chrome={chrome}
                            sliderTheme={sliderTheme}
                        />
                    ))}
                </View>
                <ToggleRow
                    label='Inline top border'
                    hint='Only affects the inline toolbar placement.'
                    value={toolbarTheme.showTopBorder}
                    onChange={updateShowTopBorder}
                    chrome={chrome}
                />
            </PanelSection>

            <PanelSection
                title='Colours'
                hint={
                    isNativeAppearance
                        ? 'Button colours override native defaults. Platform chrome may ignore background and border colours.'
                        : 'Tap a swatch to open its channel sliders. The theme commits on release.'
                }
                chrome={chrome}>
                <View style={sharedStyles.columnGrid}>
                    {TOOLBAR_COLOR_FIELDS.map(({ key, label }) => (
                        <ToolbarColorRow
                            key={key}
                            colorKey={key}
                            label={label}
                            value={toolbarTheme[key]}
                            isExpanded={expandedColor === key}
                            onToggleKey={toggleColorKey}
                            onChangeKey={changeColorKey}
                            chrome={chrome}
                        />
                    ))}
                </View>
            </PanelSection>
        </View>
    );
}

export const ToolbarSettingsPanel = React.memo(ToolbarSettingsPanelInner);
