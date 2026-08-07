import { Platform } from 'react-native';

/** One radius for every chrome surface. Editor radii are per-preset fixtures. */
export const RADIUS = 4;

/** Type scale. `section` is uppercase and letter-spaced, so it reads above `hint`. */
export const FONT_SIZE = {
    monoMicro: 11,
    section: 12,
    mono: 12,
    hint: 14,
    value: 14,
    label: 16,
    heading: 21,
} as const;

export const LINE_HEIGHT = {
    mono: 18,
    hint: 20,
    value: 20,
    label: 22,
    heading: 26,
} as const;

export const LETTER_SPACING = {
    section: 1,
    value: 0.4,
} as const;

export const SPACE = {
    xs: 4,
    sm: 8,
    md: 12,
    lg: 16,
    xl: 20,
    xxl: 32,
} as const;

/** WCAG 2.5.5 minimum, applied as `minHeight` so the visual target matches the touch target. */
export const MIN_TOUCH_TARGET = 44;

/** Numeric readouts sit in fixed columns, which 2x OS scaling clips. */
export const MAX_NUMERIC_FONT_SCALE = 1.3;

export const MONO_FONT_FAMILY = Platform.select({
    ios: 'Menlo',
    android: 'monospace',
    default: 'monospace',
});

/** Two-column width. flexBasis + flexGrow, not a percentage: 48% plus the gap collapsed. */
export const COLUMN_BASIS = '47%';
