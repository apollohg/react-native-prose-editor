import { StyleSheet } from 'react-native';

import {
    COLUMN_BASIS,
    FONT_SIZE,
    LETTER_SPACING,
    LINE_HEIGHT,
    MONO_FONT_FAMILY,
    SPACE,
} from './designTokens';

export const sharedStyles = StyleSheet.create({
    sectionLabel: {
        fontSize: FONT_SIZE.section,
        fontWeight: '700',
        textTransform: 'uppercase',
        letterSpacing: LETTER_SPACING.section,
    },
    heading: {
        fontSize: FONT_SIZE.heading,
        lineHeight: LINE_HEIGHT.heading,
        fontWeight: '700',
    },
    controlLabel: {
        fontSize: FONT_SIZE.label,
        lineHeight: LINE_HEIGHT.label,
        fontWeight: '600',
    },
    controlHint: {
        fontSize: FONT_SIZE.hint,
        lineHeight: LINE_HEIGHT.hint,
    },
    /** Numeric readout beside a control. Tabular so digits do not jitter. */
    numericValue: {
        fontSize: FONT_SIZE.value,
        lineHeight: LINE_HEIGHT.value,
        fontWeight: '700',
        fontVariant: ['tabular-nums'],
    },
    monoReadout: {
        fontFamily: MONO_FONT_FAMILY,
        fontSize: FONT_SIZE.mono,
        lineHeight: LINE_HEIGHT.mono,
    },
    settingsPanel: {
        gap: SPACE.lg,
    },
    /** flexBasis + flexGrow, not a percentage: 48% plus the gap collapsed the row. */
    columnGrid: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: SPACE.md,
    },
    column: {
        flexBasis: COLUMN_BASIS,
        flexGrow: 1,
        minWidth: 0,
        gap: SPACE.sm,
    },
    columnWide: {
        flexBasis: '100%',
        flexGrow: 1,
        minWidth: 0,
        gap: SPACE.sm,
    },
    sliderHeader: {
        flexDirection: 'row',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: SPACE.md,
    },
    slider: {
        width: '100%',
        height: 36,
    },
    switchRow: {
        flexDirection: 'row',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: SPACE.md,
    },
});
