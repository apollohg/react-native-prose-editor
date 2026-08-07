/** Hex helpers shared by both `ColorField` implementations. */

export const CHANNEL_MIN = 0;
export const CHANNEL_MAX = 255;

export type RGBColor = {
    r: number;
    g: number;
    b: number;
};

export type ChannelKey = keyof RGBColor;

export function clampChannel(value: number): number {
    return Math.max(CHANNEL_MIN, Math.min(CHANNEL_MAX, Math.round(value)));
}

export function parseHexColor(hex: string): RGBColor {
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

export function toHexColor({ r, g, b }: RGBColor): string {
    return `#${[r, g, b]
        .map((value) => clampChannel(value).toString(16).padStart(2, '0'))
        .join('')}`;
}

/** Force opaque `#rrggbb`: SwiftUI can return alpha, the Android sliders cannot. */
export function normalizeHexColor(hex: string): string {
    return toHexColor(parseHexColor(hex.trim().replace('#', '').slice(0, 6)));
}
