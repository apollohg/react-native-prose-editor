import React, { useCallback } from 'react';
import { StyleSheet, View } from 'react-native';
import { DEFAULT_EDITOR_IMAGE_LOADING_POLICY } from '@apollohg/react-native-prose-editor';

import { IMAGE_POLICY_FIELDS, SAMPLE_IMAGE_URL } from '../constants';
import { SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome, ExampleThemePreset } from '../themePresets';
import type { ImageSettings } from '../types';
import { ActionButton } from './ActionButton';
import { PanelSection } from './PanelSection';
import { SliderField } from './SliderField';
import { ToggleRow } from './ToggleRow';

const BYTES_PER_KILOBYTE = 1024;
const BYTES_PER_MEGABYTE = BYTES_PER_KILOBYTE * BYTES_PER_KILOBYTE;

/** Owns its commit closure so SliderField's memo holds across the seven bounds. */
const PolicyRow = React.memo(function PolicyRow({
    field,
    value,
    onCommitKey,
    chrome,
    sliderTheme,
}: {
    field: (typeof IMAGE_POLICY_FIELDS)[number];
    value: number;
    onCommitKey: (key: (typeof IMAGE_POLICY_FIELDS)[number]['key'], value: number) => void;
    chrome: ExampleAppChrome;
    sliderTheme: ExampleThemePreset['slider'];
}) {
    const handleCommit = useCallback(
        (next: number) => onCommitKey(field.key, next),
        [field.key, onCommitKey]
    );
    const format = useCallback(
        (current: number) => formatPolicyValue(current, field.unit),
        [field.unit]
    );
    return (
        <SliderField
            label={field.label}
            value={value}
            min={field.min}
            max={field.max}
            step={field.step}
            onCommit={handleCommit}
            chrome={chrome}
            sliderTheme={sliderTheme}
            format={format}
        />
    );
});

type ImageSettingsPanelProps = {
    settings: ImageSettings;
    onChange: (patch: Partial<ImageSettings>) => void;
    onPickImage: () => void;
    onInsertSampleImage: () => void;
    sliderTheme: ExampleThemePreset['slider'];
    chrome: ExampleAppChrome;
};

function formatPolicyValue(value: number, unit: string): string {
    if (unit !== 'bytes') {
        return unit === '' ? `${value}` : `${value}${unit}`;
    }

    if (value >= BYTES_PER_MEGABYTE) {
        return `${(value / BYTES_PER_MEGABYTE).toFixed(1)} MB`;
    }
    return `${Math.round(value / BYTES_PER_KILOBYTE)} KB`;
}

function ImageSettingsPanelInner({
    settings,
    onChange,
    onPickImage,
    onInsertSampleImage,
    sliderTheme,
    chrome,
}: ImageSettingsPanelProps) {
    const setAllowResizing = useCallback(
        (allowImageResizing: boolean) => onChange({ allowImageResizing }),
        [onChange]
    );

    const resetPolicy = useCallback(() => onChange({ policy: {} }), [onChange]);

    const commitPolicy = useCallback(
        (key: (typeof IMAGE_POLICY_FIELDS)[number]['key'], value: number) => {
            onChange({ policy: { ...settings.policy, [key]: value } });
        },
        [onChange, settings.policy]
    );

    return (
        <View style={sharedStyles.settingsPanel}>
            <PanelSection
                title='Insertion'
                hint='Both paths run through onRequestImage, so they exercise the same callback the toolbar image button uses.'
                chrome={chrome}>
                <View style={styles.buttonRow}>
                    <ActionButton
                        label='Pick from library'
                        onPress={onPickImage}
                        chrome={chrome}
                        accessibilityHint='Opens the system photo picker, then downsizes to the decode limit before inserting.'
                    />
                    <ActionButton
                        label='Insert remote'
                        tone='secondary'
                        onPress={onInsertSampleImage}
                        chrome={chrome}
                        accessibilityHint={`Inserts ${SAMPLE_IMAGE_URL}`}
                    />
                </View>
                <ToggleRow
                    label='Allow resizing'
                    hint='Shows native resize handles on a selected image.'
                    value={settings.allowImageResizing}
                    onChange={setAllowResizing}
                    chrome={chrome}
                />
            </PanelSection>

            <PanelSection
                title='Loading policy'
                hint='Bounds native image loading. Ranges straddle the package defaults so both the accepted and the rejected side of each bound is reachable.'
                chrome={chrome}>
                {IMAGE_POLICY_FIELDS.map((field) => (
                    <PolicyRow
                        key={field.key}
                        field={field}
                        value={
                            settings.policy[field.key] ??
                            DEFAULT_EDITOR_IMAGE_LOADING_POLICY[field.key]
                        }
                        onCommitKey={commitPolicy}
                        chrome={chrome}
                        sliderTheme={sliderTheme}
                    />
                ))}
                <View style={styles.buttonRow}>
                    <ActionButton
                        label='Reset policy'
                        tone='secondary'
                        onPress={resetPolicy}
                        chrome={chrome}
                        accessibilityHint='Returns every bound to the package default.'
                    />
                </View>
            </PanelSection>
        </View>
    );
}

export const ImageSettingsPanel = React.memo(ImageSettingsPanelInner);

const styles = StyleSheet.create({
    buttonRow: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: SPACE.sm,
    },
});
