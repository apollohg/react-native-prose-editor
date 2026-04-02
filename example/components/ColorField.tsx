import React from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import Slider from '@react-native-community/slider';
import type { ExampleThemePreset } from '../themePresets';
import { sharedStyles } from '../sharedStyles';

type RGBColor = {
    r: number;
    g: number;
    b: number;
};

function clampChannel(value: number): number {
    return Math.max(0, Math.min(255, Math.round(value)));
}

function parseHexColor(hex: string): RGBColor {
    const normalized = hex.trim().replace('#', '');
    const expanded =
        normalized.length === 3
            ? normalized
                  .split('')
                  .map((char) => `${char}${char}`)
                  .join('')
            : normalized;

    if (!/^[0-9a-fA-F]{6}$/.test(expanded)) {
        return { r: 0, g: 0, b: 0 };
    }

    return {
        r: parseInt(expanded.slice(0, 2), 16),
        g: parseInt(expanded.slice(2, 4), 16),
        b: parseInt(expanded.slice(4, 6), 16),
    };
}

function toHexColor({ r, g, b }: RGBColor): string {
    return `#${[r, g, b]
        .map((value) => clampChannel(value).toString(16).padStart(2, '0'))
        .join('')}`;
}

type ColorFieldProps = {
    label: string;
    value: string;
    chrome: ExampleThemePreset['appChrome'];
    isExpanded: boolean;
    onToggle: () => void;
    onChange: (value: string) => void;
};

export function ColorField({
    label,
    value,
    chrome,
    isExpanded,
    onToggle,
    onChange,
}: ColorFieldProps) {
    const color = parseHexColor(value);

    const updateChannel = (channel: keyof RGBColor, nextValue: number) => {
        onChange(
            toHexColor({
                ...color,
                [channel]: nextValue,
            })
        );
    };

    return (
        <View style={[styles.colorField, isExpanded && styles.colorFieldExpanded]}>
            <Pressable
                style={[
                    styles.colorTrigger,
                    {
                        borderColor: chrome.colorTriggerBorderColor,
                        backgroundColor: chrome.colorTriggerBackgroundColor,
                    },
                    isExpanded && {
                        borderColor: chrome.colorTriggerExpandedBorderColor,
                        backgroundColor: chrome.colorTriggerExpandedBackgroundColor,
                    },
                ]}
                onPress={onToggle}>
                <View style={[styles.colorSwatch, { backgroundColor: value }]} />
                <View style={styles.colorTriggerText}>
                    <Text style={[sharedStyles.controlLabel, { color: chrome.controlLabelColor }]}>
                        {label}
                    </Text>
                    <Text style={[styles.colorValue, { color: chrome.colorValueColor }]}>
                        {value.toUpperCase()}
                    </Text>
                </View>
            </Pressable>

            {isExpanded && (
                <View style={styles.channelGroup}>
                    <View style={styles.channelRow}>
                        <Text style={[styles.channelLabel, { color: chrome.channelLabelColor }]}>
                            R
                        </Text>
                        <Slider
                            style={styles.channelSlider}
                            minimumValue={0}
                            maximumValue={255}
                            step={1}
                            minimumTrackTintColor='#d94b4b'
                            maximumTrackTintColor='#ead6d6'
                            thumbTintColor='#b52f2f'
                            value={color.r}
                            onValueChange={(nextValue) => updateChannel('r', nextValue)}
                        />
                        <Text style={[styles.channelValue, { color: chrome.channelValueColor }]}>
                            {color.r}
                        </Text>
                    </View>

                    <View style={styles.channelRow}>
                        <Text style={[styles.channelLabel, { color: chrome.channelLabelColor }]}>
                            G
                        </Text>
                        <Slider
                            style={styles.channelSlider}
                            minimumValue={0}
                            maximumValue={255}
                            step={1}
                            minimumTrackTintColor='#4aa768'
                            maximumTrackTintColor='#d7eadf'
                            thumbTintColor='#2f7b49'
                            value={color.g}
                            onValueChange={(nextValue) => updateChannel('g', nextValue)}
                        />
                        <Text style={[styles.channelValue, { color: chrome.channelValueColor }]}>
                            {color.g}
                        </Text>
                    </View>

                    <View style={styles.channelRow}>
                        <Text style={[styles.channelLabel, { color: chrome.channelLabelColor }]}>
                            B
                        </Text>
                        <Slider
                            style={styles.channelSlider}
                            minimumValue={0}
                            maximumValue={255}
                            step={1}
                            minimumTrackTintColor='#4b7bd9'
                            maximumTrackTintColor='#d7dff1'
                            thumbTintColor='#2f56a8'
                            value={color.b}
                            onValueChange={(nextValue) => updateChannel('b', nextValue)}
                        />
                        <Text style={[styles.channelValue, { color: chrome.channelValueColor }]}>
                            {color.b}
                        </Text>
                    </View>
                </View>
            )}
        </View>
    );
}

const styles = StyleSheet.create({
    colorField: {
        width: '48%',
        gap: 8,
    },
    colorFieldExpanded: {
        width: '100%',
    },
    colorTrigger: {
        flexDirection: 'row',
        alignItems: 'center',
        gap: 12,
        paddingHorizontal: 12,
        paddingVertical: 10,
        borderRadius: 14,
        borderWidth: 1,
    },
    colorSwatch: {
        width: 28,
        height: 28,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: 'rgba(0,0,0,0.08)',
    },
    colorTriggerText: {
        gap: 8,
    },
    colorValue: {
        fontSize: 12,
        fontWeight: '700',
        letterSpacing: 0.6,
        textTransform: 'uppercase',
    },
    channelGroup: {
        gap: 8,
        paddingHorizontal: 6,
    },
    channelRow: {
        flexDirection: 'row',
        alignItems: 'center',
        gap: 10,
    },
    channelLabel: {
        width: 14,
        fontSize: 12,
        fontWeight: '700',
    },
    channelSlider: {
        flex: 1,
        height: 32,
    },
    channelValue: {
        width: 32,
        fontSize: 12,
        fontWeight: '700',
        textAlign: 'right',
    },
});
