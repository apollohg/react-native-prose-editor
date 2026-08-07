import React, { useCallback, useEffect, useRef, useState } from 'react';
import { StyleSheet, Text, View } from 'react-native';
import Slider from '@react-native-community/slider';

import { MAX_NUMERIC_FONT_SCALE, SPACE } from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ExampleAppChrome, ExampleThemePreset } from '../themePresets';

/** Commits on release: `onValueChange` fires per step, and each step rebuilds the theme. */

type SliderFieldProps = {
    label: string;
    value: number;
    min: number;
    max: number;
    step: number;
    onCommit: (value: number) => void;
    chrome: ExampleAppChrome;
    sliderTheme: ExampleThemePreset['slider'];
    /** Renders the readout. Defaults to `${value}px`. */
    format?: (value: number) => string;
};

function SliderFieldInner({
    label,
    value,
    min,
    max,
    step,
    onCommit,
    chrome,
    sliderTheme,
    format,
}: SliderFieldProps) {
    const [draft, setDraft] = useState(value);
    const isDraggingRef = useRef(false);

    // Adopt external changes unless mid-drag, which would fight the thumb.
    useEffect(() => {
        if (isDraggingRef.current) return;
        setDraft(value);
    }, [value]);

    const handleSlidingStart = useCallback(() => {
        isDraggingRef.current = true;
    }, []);

    const handleSlidingComplete = useCallback(
        (next: number) => {
            isDraggingRef.current = false;
            setDraft(next);
            onCommit(next);
        },
        [onCommit]
    );

    const readout = format ? format(draft) : `${draft}px`;

    return (
        <View style={styles.field}>
            <View style={sharedStyles.sliderHeader}>
                <Text
                    numberOfLines={2}
                    style={[sharedStyles.controlLabel, { color: chrome.controlLabelColor }]}>
                    {label}
                </Text>
                <Text
                    maxFontSizeMultiplier={MAX_NUMERIC_FONT_SCALE}
                    style={[sharedStyles.numericValue, { color: chrome.sliderValueColor }]}>
                    {readout}
                </Text>
            </View>
            <Slider
                accessibilityLabel={label}
                accessibilityValue={{ min, max, now: draft, text: readout }}
                style={sharedStyles.slider}
                minimumValue={min}
                maximumValue={max}
                step={step}
                minimumTrackTintColor={sliderTheme.minimumTrackTintColor}
                maximumTrackTintColor={sliderTheme.maximumTrackTintColor}
                thumbTintColor={sliderTheme.thumbTintColor}
                value={draft}
                onValueChange={setDraft}
                onSlidingStart={handleSlidingStart}
                onSlidingComplete={handleSlidingComplete}
            />
        </View>
    );
}

export const SliderField = React.memo(SliderFieldInner);

const styles = StyleSheet.create({
    field: {
        gap: SPACE.xs,
    },
});
