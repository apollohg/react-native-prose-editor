import React, { useCallback, useEffect, useRef, useState } from 'react';
import { StyleSheet, Text, View } from 'react-native';
import { ColorPicker, Host } from '@expo/ui/swift-ui';

import { normalizeHexColor } from '../colorHex';
import {
    FONT_SIZE,
    LETTER_SPACING,
    MAX_NUMERIC_FONT_SCALE,
    MIN_TOUCH_TARGET,
    RADIUS,
    SPACE,
} from '../designTokens';
import { sharedStyles } from '../sharedStyles';
import type { ColorFieldProps } from './ColorField.shared';

/** iOS colour field. Metro resolves this over `ColorField.tsx` by extension. */

/** The picker streams every intermediate colour and has no release event. */
const COLOR_COMMIT_DEBOUNCE_MS = 120;

/** Host stacks content .topLeading, so slack pushes the well up; matchContents removes it. */
const WELL_MIN_SIZE = 28;

function ColorFieldInner({ label, value, chrome, onChange }: ColorFieldProps) {
    const [draft, setDraft] = useState<string | null>(null);
    const commitTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Preset switches replace `value`, so drop any stale draft.
    useEffect(() => {
        setDraft(null);
    }, [value]);

    useEffect(
        () => () => {
            if (commitTimer.current != null) {
                clearTimeout(commitTimer.current);
            }
        },
        []
    );

    const handleSelectionChange = useCallback(
        (next: string) => {
            const normalized = normalizeHexColor(next);
            setDraft(normalized);

            if (commitTimer.current != null) {
                clearTimeout(commitTimer.current);
            }
            commitTimer.current = setTimeout(() => {
                commitTimer.current = null;
                onChange(normalized);
            }, COLOR_COMMIT_DEBOUNCE_MS);
        },
        [onChange]
    );

    const selection = draft ?? normalizeHexColor(value);
    const displayHex = selection.toUpperCase();

    return (
        <View style={sharedStyles.column}>
            <View
                accessibilityLabel={`${label} colour, ${displayHex}`}
                style={[styles.pickerCard, { backgroundColor: chrome.controlSurfaceColor }]}>
                <Host matchContents style={styles.well}>
                    <ColorPicker
                        selection={selection}
                        onSelectionChange={handleSelectionChange}
                        supportsOpacity={false}
                    />
                </Host>

                {/* Label lives here: SwiftUI cannot put the well leading. */}
                <View style={styles.text}>
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
            </View>
        </View>
    );
}

export const ColorField = React.memo(ColorFieldInner);

const styles = StyleSheet.create({
    pickerCard: {
        flexDirection: 'row',
        alignItems: 'center',
        gap: SPACE.md,
        paddingHorizontal: SPACE.md,
        paddingVertical: SPACE.md,
        minHeight: MIN_TOUCH_TARGET,
        borderRadius: RADIUS,
    },
    well: {
        minWidth: WELL_MIN_SIZE,
        minHeight: WELL_MIN_SIZE,
    },
    text: {
        flexShrink: 1,
        gap: SPACE.xs,
    },
    colorValue: {
        fontSize: FONT_SIZE.value,
        fontWeight: '700',
        fontVariant: ['tabular-nums'],
        letterSpacing: LETTER_SPACING.value,
    },
});
