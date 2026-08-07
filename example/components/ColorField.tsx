import React, { useCallback, useRef, useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import Slider from '@react-native-community/slider';

import {
    CHANNEL_MAX,
    CHANNEL_MIN,
    clampChannel,
    parseHexColor,
    toHexColor,
    type ChannelKey,
    type RGBColor,
} from '../colorHex';
import {
    FONT_SIZE,
    LETTER_SPACING,
    MAX_NUMERIC_FONT_SCALE,
    MIN_TOUCH_TARGET,
    RADIUS,
    SPACE,
} from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome } from '../themePresets';
import type { ColorFieldProps } from './ColorField.shared';

/** Android colour field: RGB sliders. iOS uses the system picker. */

const CHANNEL_STEP = 1;
const SWATCH_SIZE = 28;

const CHANNELS: readonly { key: ChannelKey; label: string; name: string }[] = [
    { key: 'r', label: 'R', name: 'red' },
    { key: 'g', label: 'G', name: 'green' },
    { key: 'b', label: 'B', name: 'blue' },
];

type ChannelSliderProps = {
    channel: ChannelKey;
    channelLabel: string;
    channelName: string;
    fieldLabel: string;
    value: number;
    trackColor: string;
    activeColor: string;
    chrome: ExampleAppChrome;
    onDrag: (channel: ChannelKey, value: number) => void;
    onCommit: (channel: ChannelKey, value: number) => void;
};

/** Owns its per-channel closures so the memo holds. */
const ChannelSlider = React.memo(function ChannelSlider({
    channel,
    channelLabel,
    channelName,
    fieldLabel,
    value,
    trackColor,
    activeColor,
    chrome,
    onDrag,
    onCommit,
}: ChannelSliderProps) {
    const handleDrag = useCallback((next: number) => onDrag(channel, next), [channel, onDrag]);
    const handleCommit = useCallback(
        (next: number) => onCommit(channel, next),
        [channel, onCommit]
    );

    return (
        <View style={styles.channelRow}>
            <Text
                maxFontSizeMultiplier={MAX_NUMERIC_FONT_SCALE}
                style={[styles.channelLabel, { color: chrome.channelLabelColor }]}>
                {channelLabel}
            </Text>
            <Slider
                accessibilityLabel={`${fieldLabel} ${channelName} channel`}
                accessibilityValue={{ min: CHANNEL_MIN, max: CHANNEL_MAX, now: value }}
                style={styles.channelSlider}
                minimumValue={CHANNEL_MIN}
                maximumValue={CHANNEL_MAX}
                step={CHANNEL_STEP}
                minimumTrackTintColor={activeColor}
                maximumTrackTintColor={trackColor}
                thumbTintColor={activeColor}
                value={value}
                onValueChange={handleDrag}
                onSlidingComplete={handleCommit}
            />
            <Text
                maxFontSizeMultiplier={MAX_NUMERIC_FONT_SCALE}
                style={[styles.channelValue, { color: chrome.channelValueColor }]}>
                {value}
            </Text>
        </View>
    );
});

function ColorFieldInner({
    label,
    value,
    chrome,
    isExpanded,
    onToggle,
    onChange,
}: ColorFieldProps) {
    /** Dragging lives here, not in the theme: one `onChange` on release. */
    const [draftColor, setDraftColor] = useState<RGBColor | null>(null);
    const draftRef = useRef<RGBColor | null>(null);

    const committedColor = parseHexColor(value);
    const color = draftColor ?? committedColor;

    const handleDrag = useCallback(
        (channel: ChannelKey, next: number) => {
            const base = draftRef.current ?? parseHexColor(value);
            const updated = { ...base, [channel]: clampChannel(next) };
            draftRef.current = updated;
            setDraftColor(updated);
        },
        [value]
    );

    const handleCommit = useCallback(
        (channel: ChannelKey, next: number) => {
            const base = draftRef.current ?? parseHexColor(value);
            const updated = { ...base, [channel]: clampChannel(next) };
            draftRef.current = null;
            setDraftColor(null);
            onChange(toHexColor(updated));
        },
        [onChange, value]
    );

    const channelColors: Record<ChannelKey, string> = {
        r: chrome.channelRedColor,
        g: chrome.channelGreenColor,
        b: chrome.channelBlueColor,
    };

    const displayHex = toHexColor(color).toUpperCase();

    return (
        <View style={isExpanded ? sharedStyles.columnWide : sharedStyles.column}>
            <Pressable
                accessibilityRole='button'
                accessibilityLabel={`${label} colour, ${displayHex}`}
                accessibilityState={{ expanded: isExpanded }}
                accessibilityHint={
                    isExpanded ? 'Hides the channel sliders.' : 'Shows red, green and blue sliders.'
                }
                style={[
                    styles.colorTrigger,
                    {
                        backgroundColor: isExpanded
                            ? chrome.controlExpandedColor
                            : chrome.controlSurfaceColor,
                    },
                ]}
                onPress={onToggle}>
                <View
                    style={[
                        styles.colorSwatch,
                        { backgroundColor: displayHex, borderColor: chrome.swatchBorderColor },
                    ]}
                />
                <View style={styles.colorTriggerText}>
                    <Text
                        numberOfLines={1}
                        style={[sharedStyles.controlLabel, { color: chrome.controlLabelColor }]}>
                        {label}
                    </Text>
                    <Text
                        maxFontSizeMultiplier={MAX_NUMERIC_FONT_SCALE}
                        style={[styles.colorValue, { color: chrome.colorValueColor }]}>
                        {displayHex}
                    </Text>
                </View>
            </Pressable>

            {isExpanded ? (
                <View style={styles.channelGroup}>
                    {CHANNELS.map(({ key, label: channelLabel, name }) => (
                        <ChannelSlider
                            key={key}
                            channelLabel={channelLabel}
                            channelName={name}
                            fieldLabel={label}
                            value={color[key]}
                            trackColor={chrome.channelTrackColor}
                            activeColor={channelColors[key]}
                            chrome={chrome}
                            channel={key}
                            onDrag={handleDrag}
                            onCommit={handleCommit}
                        />
                    ))}
                </View>
            ) : null}
        </View>
    );
}

export const ColorField = React.memo(ColorFieldInner);

const styles = StyleSheet.create({
    colorTrigger: {
        flexDirection: 'row',
        alignItems: 'center',
        gap: SPACE.md,
        paddingHorizontal: SPACE.md,
        paddingVertical: SPACE.sm,
        minHeight: MIN_TOUCH_TARGET,
        borderRadius: RADIUS,
    },
    colorSwatch: {
        width: SWATCH_SIZE,
        height: SWATCH_SIZE,
        borderRadius: RADIUS,
        borderWidth: 1,
    },
    colorTriggerText: {
        flexShrink: 1,
        gap: SPACE.xs,
    },
    colorValue: {
        fontSize: FONT_SIZE.value,
        fontWeight: '700',
        fontVariant: ['tabular-nums'],
        letterSpacing: LETTER_SPACING.value,
    },
    channelGroup: {
        gap: SPACE.sm,
        paddingHorizontal: SPACE.xs,
    },
    channelRow: {
        flexDirection: 'row',
        alignItems: 'center',
        gap: SPACE.sm,
    },
    channelLabel: {
        // minWidth, not width: a fixed column clips the glyph at 2x font scale.
        minWidth: 16,
        fontSize: FONT_SIZE.value,
        fontWeight: '700',
    },
    channelSlider: {
        flex: 1,
        height: 36,
    },
    channelValue: {
        minWidth: 36,
        fontSize: FONT_SIZE.value,
        fontWeight: '700',
        fontVariant: ['tabular-nums'],
        textAlign: 'right',
    },
});
