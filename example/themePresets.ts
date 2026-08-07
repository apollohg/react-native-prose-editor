import type {
    EditorContentInsets,
    EditorFontWeight,
    EditorHeadingTheme,
    EditorHorizontalRuleTheme,
    EditorLinkTheme,
    EditorListTheme,
    EditorMentionTheme,
    EditorTextStyle,
    EditorTheme,
    EditorToolbarTheme,
} from '@apollohg/react-native-prose-editor';

/**
 * App chrome tokens. Text is held at >= 4.5:1 on every background it renders on
 * and UI indicators at >= 3:1. Editor content colours are separate fixtures and
 * may fail contrast on purpose.
 */
export interface ExampleAppChrome {
    screenBackgroundColor: string;
    cardBackgroundColor: string;
    cardSecondaryBackgroundColor: string;
    titleColor: string;
    subtitleColor: string;
    sectionLabelColor: string;
    controlLabelColor: string;
    controlHintColor: string;
    /** Accent on the screen. Navigation bar tint and bar actions. */
    accentColor: string;
    /** Structural rule. The navigation bar hairline. */
    separatorColor: string;
    /** Resting surface of every touchable. >= 1.25:1 against the card, since nothing outlines it. */
    controlSurfaceColor: string;
    controlSurfaceTextColor: string;
    /** Selected chip. A fill, not a tint: nothing else marks selection. */
    controlSelectedColor: string;
    controlSelectedTextColor: string;
    /** Colour trigger while its channel sliders are open. */
    controlExpandedColor: string;
    switchTrackColor: string;
    switchTrackActiveColor: string;
    switchThumbColor: string;
    sliderValueColor: string;
    colorValueColor: string;
    channelLabelColor: string;
    channelValueColor: string;
    /** Unfilled portion of a slider track. Deliberately subtle (~1.9:1). */
    channelTrackColor: string;
    channelRedColor: string;
    channelGreenColor: string;
    channelBlueColor: string;
    /** The one load-bearing border: a swatch matching the card would vanish. */
    swatchBorderColor: string;
    /** Removal and rejection affordances. Held at >= 3:1 against the card. */
    destructiveColor: string;
    actionButtonBackgroundColor: string;
    actionButtonTextColor: string;
    outputCardBackgroundColor: string;
    outputTextColor: string;
}

/** `EditorBlockquoteTheme` is not exported from the package index. */
export interface ExampleBlockquoteTheme {
    indent: number;
    borderColor: string;
    borderWidth: number;
    markerGap: number;
}

/** `fontSize` comes from the live base size; `fontFamily` stays inherited because
 * 'System' is not a real Android family. */
export type ExampleLinkTheme = Omit<Required<EditorLinkTheme>, 'fontSize' | 'fontFamily'>;

export interface ExampleThemePreset {
    id: string;
    label: string;
    /** Colour strategy plus what this fixture covers. Rendered under the picker. */
    covers: string;
    statusBarStyle: 'dark' | 'light';
    textColor: string;
    backgroundColor: string;
    placeholderColor: string;
    editorBorderRadius: number;
    contentInsets: Required<EditorContentInsets>;
    appChrome: ExampleAppChrome;
    paragraphSpacingAfter: number;
    /** Multiplied by the live base font size, because `lineHeight` is in points. */
    lineHeightRatio: number;
    headings: EditorHeadingTheme;
    blockquote: ExampleBlockquoteTheme;
    list: Required<EditorListTheme>;
    horizontalRule: Required<EditorHorizontalRuleTheme>;
    mentions: EditorMentionTheme;
    links: ExampleLinkTheme;
    toolbar: Required<EditorToolbarTheme>;
    slider: {
        minimumTrackTintColor: string;
        maximumTrackTintColor: string;
        thumbTintColor: string;
    };
}

export interface ExampleEditorThemeOverrides {
    blockquoteBorderColor?: string;
}

export const DEFAULT_EXAMPLE_THEME_PRESET_ID = 'press';

interface HeadingScale {
    /** h1 through h6, largest first. */
    sizes: readonly [number, number, number, number, number, number];
    /** Space below a heading, as a fraction of that heading's own size. */
    spacingRatio: number;
}

const HEADING_WEIGHTS: readonly EditorFontWeight[] = ['700', '700', '700', '600', '600', '600'];

function buildHeadingTheme(color: string, scale: HeadingScale): EditorHeadingTheme {
    const style = (index: number): EditorTextStyle => {
        const fontSize = scale.sizes[index];
        return {
            color,
            fontSize,
            fontWeight: HEADING_WEIGHTS[index],
            spacingAfter: Math.round(fontSize * scale.spacingRatio),
        };
    };

    return {
        h1: style(0),
        h2: style(1),
        h3: style(2),
        h4: style(3),
        h5: style(4),
        h6: style(5),
    };
}

/* Press: restrained, light, square. Every radius is 0, the falsy-zero fixture. */
const PRESS_APP_CHROME: ExampleAppChrome = {
    screenBackgroundColor: '#e0e5dc',
    cardBackgroundColor: '#f6f9f4',
    cardSecondaryBackgroundColor: '#fbfdfa',
    titleColor: '#13180f',
    subtitleColor: '#4f5549',
    sectionLabelColor: '#545b4e',
    controlLabelColor: '#1c2118',
    controlHintColor: '#545b4e',
    accentColor: '#285a2d',
    separatorColor: '#c6cdc0',
    controlSurfaceColor: '#d8ddd3',
    controlSurfaceTextColor: '#545b4e',
    controlSelectedColor: '#306d37',
    controlSelectedTextColor: '#eff4eb',
    controlExpandedColor: '#c4cfbb',
    switchTrackColor: '#c6cdc0',
    switchTrackActiveColor: '#2a6431',
    switchThumbColor: '#f3f6f1',
    sliderValueColor: '#545b4e',
    colorValueColor: '#4f5549',
    channelLabelColor: '#545b4e',
    channelValueColor: '#545b4e',
    channelTrackColor: '#b6b8b3',
    channelRedColor: '#b6322b',
    channelGreenColor: '#187e36',
    channelBlueColor: '#006bbb',
    swatchBorderColor: '#c6cdc0',
    destructiveColor: '#b32228',
    actionButtonBackgroundColor: '#1c2118',
    actionButtonTextColor: '#f3f6f1',
    outputCardBackgroundColor: '#181c15',
    outputTextColor: '#d3dacd',
};

/* Poster: committed. Vermilion chrome around a pale editor, loose geometry. */
const POSTER_APP_CHROME: ExampleAppChrome = {
    screenBackgroundColor: '#f5e9dc',
    cardBackgroundColor: '#95210d',
    cardSecondaryBackgroundColor: '#ad301c',
    titleColor: '#37120c',
    subtitleColor: '#7a3528',
    sectionLabelColor: '#f5e5d3',
    controlLabelColor: '#fcf4ea',
    controlHintColor: '#ebdbc9',
    accentColor: '#9e2511',
    separatorColor: '#c74d38',
    controlSurfaceColor: '#720600',
    controlSurfaceTextColor: '#f5e5d3',
    controlSelectedColor: '#f8e8d6',
    controlSelectedTextColor: '#5e0000',
    controlExpandedColor: '#590000',
    switchTrackColor: '#c74d38',
    switchTrackActiveColor: '#e9d0b1',
    switchThumbColor: '#fbf0e4',
    sliderValueColor: '#f5e5d3',
    colorValueColor: '#f6e9d9',
    channelLabelColor: '#f5e5d3',
    channelValueColor: '#f5e5d3',
    channelTrackColor: '#b4695a',
    channelRedColor: '#ffb2b2',
    channelGreenColor: '#74e791',
    channelBlueColor: '#85c3ff',
    swatchBorderColor: '#ede3d6',
    destructiveColor: '#ffa6b6',
    actionButtonBackgroundColor: '#f8e8d6',
    actionButtonTextColor: '#5c0400',
    outputCardBackgroundColor: '#2c0804',
    outputTextColor: '#eee3d5',
};

/* Signal: full palette on a green-black ground. The only native toolbar. */
const SIGNAL_APP_CHROME: ExampleAppChrome = {
    screenBackgroundColor: '#09100d',
    cardBackgroundColor: '#141e1a',
    cardSecondaryBackgroundColor: '#1b2722',
    titleColor: '#e4eee9',
    subtitleColor: '#86978f',
    sectionLabelColor: '#86978f',
    controlLabelColor: '#cedbd5',
    controlHintColor: '#809189',
    accentColor: '#ca9d42',
    separatorColor: '#303e38',
    controlSurfaceColor: '#24352d',
    controlSurfaceTextColor: '#92a39b',
    controlSelectedColor: '#c08f22',
    controlSelectedTextColor: '#181003',
    controlExpandedColor: '#152920',
    switchTrackColor: '#303e38',
    switchTrackActiveColor: '#c6952c',
    switchThumbColor: '#e4eee9',
    sliderValueColor: '#86978f',
    colorValueColor: '#92a39b',
    channelLabelColor: '#809189',
    channelValueColor: '#86978f',
    channelTrackColor: '#404b45',
    channelRedColor: '#d75072',
    channelGreenColor: '#23b374',
    channelBlueColor: '#537bd9',
    swatchBorderColor: '#2f3b35',
    destructiveColor: '#d75072',
    actionButtonBackgroundColor: '#c6952c',
    actionButtonTextColor: '#0f0800',
    outputCardBackgroundColor: '#050a07',
    outputTextColor: '#bfcfc7',
};

/* Oxblood: drenched. Every surface carries chroma. */
const OXBLOOD_APP_CHROME: ExampleAppChrome = {
    screenBackgroundColor: '#19030e',
    cardBackgroundColor: '#290a1b',
    cardSecondaryBackgroundColor: '#341023',
    titleColor: '#f8e6ed',
    subtitleColor: '#ac8a99',
    sectionLabelColor: '#ac8a99',
    controlLabelColor: '#e7d0da',
    controlHintColor: '#a58493',
    accentColor: '#ee83af',
    separatorColor: '#472335',
    controlSurfaceColor: '#481c33',
    controlSurfaceTextColor: '#b593a2',
    controlSelectedColor: '#e573a3',
    controlSelectedTextColor: '#1e0914',
    controlExpandedColor: '#3d0a27',
    switchTrackColor: '#472335',
    switchTrackActiveColor: '#ec79a9',
    switchThumbColor: '#f8e6ed',
    sliderValueColor: '#ac8a99',
    colorValueColor: '#b593a2',
    channelLabelColor: '#a58493',
    channelValueColor: '#ac8a99',
    channelTrackColor: '#5a3a49',
    channelRedColor: '#ee7746',
    channelGreenColor: '#43b966',
    channelBlueColor: '#5488ec',
    swatchBorderColor: '#552f42',
    destructiveColor: '#f56b76',
    actionButtonBackgroundColor: '#ec79a9',
    actionButtonTextColor: '#17010c',
    outputCardBackgroundColor: '#110208',
    outputTextColor: '#e6d1da',
};

/**
 * Four fixtures: four colour strategies, four geometry profiles. They must not
 * share geometry constants, or the numeric half of the theme API goes untested.
 */
export const EXAMPLE_THEME_PRESETS: readonly ExampleThemePreset[] = [
    {
        id: 'press',
        label: 'Press',
        covers: 'Restrained palette, zero radii, flush toolbar. Catches a falsy 0 read as unset.',
        statusBarStyle: 'dark',
        backgroundColor: '#f2f5ef',
        textColor: '#181c14',
        placeholderColor: '#83887f',
        editorBorderRadius: 0,
        contentInsets: { top: 12, right: 12, bottom: 12, left: 12 },
        appChrome: PRESS_APP_CHROME,
        paragraphSpacingAfter: 10,
        lineHeightRatio: 1.35,
        headings: buildHeadingTheme(PRESS_APP_CHROME.titleColor, {
            sizes: [26, 23, 20, 18, 16, 15],
            spacingRatio: 0.3,
        }),
        blockquote: {
            indent: 12,
            borderColor: '#56895a',
            borderWidth: 2,
            markerGap: 6,
        },
        list: {
            indent: 14,
            baseIndentMultiplier: 1,
            itemSpacing: 4,
            markerColor: '#2a6431',
            markerScale: 1.2,
        },
        horizontalRule: {
            color: '#c0c7ba',
            thickness: 1,
            verticalMargin: 8,
        },
        links: {
            color: '#195c24',
            backgroundColor: '#f2f5ef',
            underline: true,
            fontWeight: '600',
            fontStyle: 'normal',
        },
        mentions: {
            node: {
                textColor: '#003f0d',
                backgroundColor: '#d2edd3',
                borderColor: '#aac7ab',
                borderWidth: 1,
                borderRadius: 0,
                fontWeight: '700',
            },
            suggestions: {
                backgroundColor: '#fbfdfa',
                borderColor: '#c6cdc0',
                borderWidth: 1,
                borderRadius: 0,
                shadowColor: '#13180f',
                option: {
                    textColor: '#1c2118',
                    secondaryTextColor: '#545b4e',
                    backgroundColor: '#d2edd3',
                    borderColor: '#aac7ab',
                    borderWidth: 1,
                    borderRadius: 0,
                    fontWeight: '700',
                    highlightedBackgroundColor: '#d2edd3',
                    highlightedTextColor: '#003f0d',
                },
            },
        },
        toolbar: {
            appearance: 'custom',
            height: 36,
            backgroundColor: '#fbfdfa',
            borderColor: '#c6cdc0',
            borderWidth: 1,
            borderRadius: 0,
            marginTop: 4,
            showTopBorder: true,
            keyboardOffset: 0,
            horizontalInset: 0,
            separatorColor: '#dbe0d7',
            buttonColor: '#424b3a',
            buttonActiveColor: '#003f0d',
            buttonDisabledColor: '#a8aca5',
            buttonActiveBackgroundColor: '#d2edd3',
            buttonBorderRadius: 0,
        },
        slider: {
            minimumTrackTintColor: '#2a6431',
            maximumTrackTintColor: PRESS_APP_CHROME.channelTrackColor,
            thumbTintColor: '#0f4418',
        },
    },
    {
        id: 'poster',
        label: 'Poster',
        covers: 'Committed palette. Saturated chrome around a pale editor, at the loose end of the geometry range.',
        statusBarStyle: 'dark',
        backgroundColor: '#faf0e5',
        textColor: '#2e100b',
        placeholderColor: '#a4685c',
        editorBorderRadius: 24,
        contentInsets: { top: 22, right: 22, bottom: 22, left: 22 },
        appChrome: POSTER_APP_CHROME,
        paragraphSpacingAfter: 22,
        lineHeightRatio: 1.7,
        headings: buildHeadingTheme(POSTER_APP_CHROME.titleColor, {
            sizes: [38, 33, 28, 24, 21, 18],
            spacingRatio: 0.5,
        }),
        blockquote: {
            indent: 26,
            borderColor: '#c43922',
            borderWidth: 6,
            markerGap: 12,
        },
        list: {
            indent: 28,
            baseIndentMultiplier: 1.5,
            itemSpacing: 10,
            markerColor: '#b3260e',
            markerScale: 1.7,
        },
        horizontalRule: {
            color: '#d9553f',
            thickness: 3,
            verticalMargin: 20,
        },
        links: {
            color: '#9b1400',
            backgroundColor: '#ffc8bb',
            underline: false,
            fontWeight: '700',
            fontStyle: 'normal',
        },
        mentions: {
            node: {
                textColor: '#6f0100',
                backgroundColor: '#ffc8bb',
                borderColor: '#e37e6a',
                borderWidth: 2,
                borderRadius: 14,
                fontWeight: '700',
            },
            suggestions: {
                backgroundColor: '#f9ecdd',
                borderColor: '#d6b894',
                borderWidth: 2,
                borderRadius: 22,
                shadowColor: '#37120c',
                option: {
                    textColor: '#40140c',
                    secondaryTextColor: '#874033',
                    backgroundColor: '#ffc8bb',
                    borderColor: '#e37e6a',
                    borderWidth: 2,
                    borderRadius: 14,
                    fontWeight: '700',
                    highlightedBackgroundColor: '#ffc8bb',
                    highlightedTextColor: '#6f0100',
                },
            },
        },
        toolbar: {
            appearance: 'custom',
            height: 48,
            backgroundColor: '#7c1403',
            borderColor: '#c74d38',
            borderWidth: 2,
            borderRadius: 24,
            marginTop: 12,
            showTopBorder: false,
            keyboardOffset: 10,
            horizontalInset: 20,
            separatorColor: '#b23a26',
            buttonColor: '#f5e5d3',
            buttonActiveColor: '#fef3e7',
            buttonDisabledColor: '#bb6f60',
            buttonActiveBackgroundColor: '#c43922',
            buttonBorderRadius: 18,
        },
        slider: {
            minimumTrackTintColor: '#f8e8d6',
            maximumTrackTintColor: POSTER_APP_CHROME.channelTrackColor,
            thumbTintColor: '#fcf4ea',
        },
    },
    {
        id: 'signal',
        label: 'Signal',
        covers: 'Full palette. Four role hues on one dark ground, and the only native toolbar appearance.',
        statusBarStyle: 'light',
        backgroundColor: '#0d1612',
        textColor: '#d5e2db',
        placeholderColor: '#5f6d66',
        editorBorderRadius: 12,
        contentInsets: { top: 16, right: 16, bottom: 16, left: 16 },
        appChrome: SIGNAL_APP_CHROME,
        paragraphSpacingAfter: 16,
        lineHeightRatio: 1.5,
        headings: buildHeadingTheme(SIGNAL_APP_CHROME.titleColor, {
            sizes: [32, 28, 24, 20, 18, 16],
            spacingRatio: 0.42,
        }),
        blockquote: {
            indent: 18,
            borderColor: '#358e61',
            borderWidth: 3,
            markerGap: 8,
        },
        list: {
            indent: 20,
            baseIndentMultiplier: 1.2,
            itemSpacing: 8,
            markerColor: '#c6952c',
            markerScale: 1.4,
        },
        horizontalRule: {
            color: '#293630',
            thickness: 2,
            verticalMargin: 14,
        },
        links: {
            color: '#4dae7b',
            backgroundColor: '#0d1612',
            underline: true,
            fontWeight: '500',
            fontStyle: 'italic',
        },
        mentions: {
            node: {
                textColor: '#deaf56',
                backgroundColor: '#332405',
                borderColor: '#5a4414',
                borderWidth: 1,
                borderRadius: 6,
                fontWeight: '700',
            },
            suggestions: {
                backgroundColor: '#141e1a',
                borderColor: '#303e38',
                borderWidth: 1,
                borderRadius: 12,
                shadowColor: '#09100d',
                option: {
                    textColor: '#cedbd5',
                    secondaryTextColor: '#809189',
                    backgroundColor: '#332405',
                    borderColor: '#5a4414',
                    borderWidth: 1,
                    borderRadius: 6,
                    fontWeight: '700',
                    highlightedBackgroundColor: '#332405',
                    highlightedTextColor: '#deaf56',
                },
            },
        },
        toolbar: {
            appearance: 'native',
            height: 42,
            backgroundColor: '#141e1a',
            borderColor: '#303e38',
            borderWidth: 1,
            borderRadius: 12,
            marginTop: 8,
            showTopBorder: false,
            keyboardOffset: 6,
            horizontalInset: 12,
            separatorColor: '#1e2924',
            buttonColor: '#86978f',
            buttonActiveColor: '#deaf56',
            buttonDisabledColor: '#3b4540',
            buttonActiveBackgroundColor: '#332405',
            buttonBorderRadius: 8,
        },
        slider: {
            minimumTrackTintColor: '#c6952c',
            maximumTrackTintColor: SIGNAL_APP_CHROME.channelTrackColor,
            thumbTintColor: '#deaf56',
        },
    },
    {
        id: 'oxblood',
        label: 'Oxblood',
        covers: 'Drenched palette. Every surface carries chroma, so a grey fallback cannot hide.',
        statusBarStyle: 'light',
        backgroundColor: '#210715',
        textColor: '#ecd7e0',
        placeholderColor: '#805d6d',
        editorBorderRadius: 26,
        contentInsets: { top: 20, right: 20, bottom: 20, left: 20 },
        appChrome: OXBLOOD_APP_CHROME,
        paragraphSpacingAfter: 20,
        lineHeightRatio: 1.65,
        headings: buildHeadingTheme(OXBLOOD_APP_CHROME.titleColor, {
            sizes: [34, 30, 26, 22, 19, 17],
            spacingRatio: 0.46,
        }),
        blockquote: {
            indent: 24,
            borderColor: '#bb5c84',
            borderWidth: 5,
            markerGap: 12,
        },
        list: {
            indent: 26,
            baseIndentMultiplier: 1.4,
            itemSpacing: 10,
            markerColor: '#ec79a9',
            markerScale: 1.6,
        },
        horizontalRule: {
            color: '#4c283a',
            thickness: 2,
            verticalMargin: 18,
        },
        links: {
            color: '#e177a3',
            backgroundColor: '#210715',
            underline: true,
            fontWeight: '600',
            fontStyle: 'normal',
        },
        mentions: {
            node: {
                textColor: '#ff99c2',
                backgroundColor: '#4e1530',
                borderColor: '#7f3556',
                borderWidth: 1,
                borderRadius: 12,
                fontWeight: '700',
            },
            suggestions: {
                backgroundColor: '#290a1b',
                borderColor: '#472335',
                borderWidth: 1,
                borderRadius: 20,
                shadowColor: '#19030e',
                option: {
                    textColor: '#e7d0da',
                    secondaryTextColor: '#a58493',
                    backgroundColor: '#4e1530',
                    borderColor: '#7f3556',
                    borderWidth: 1,
                    borderRadius: 12,
                    fontWeight: '700',
                    highlightedBackgroundColor: '#4e1530',
                    highlightedTextColor: '#ff99c2',
                },
            },
        },
        toolbar: {
            appearance: 'custom',
            height: 46,
            backgroundColor: '#290a1b',
            borderColor: '#472335',
            borderWidth: 1,
            borderRadius: 23,
            marginTop: 10,
            showTopBorder: false,
            keyboardOffset: 8,
            horizontalInset: 18,
            separatorColor: '#3b192a',
            buttonColor: '#ac8a99',
            buttonActiveColor: '#ff99c2',
            buttonDisabledColor: '#573746',
            buttonActiveBackgroundColor: '#4e1530',
            buttonBorderRadius: 16,
        },
        slider: {
            minimumTrackTintColor: '#ec79a9',
            maximumTrackTintColor: OXBLOOD_APP_CHROME.channelTrackColor,
            thumbTintColor: '#ff99c2',
        },
    },
] as const;

export function getExampleThemePreset(id: string): ExampleThemePreset {
    return EXAMPLE_THEME_PRESETS.find((preset) => preset.id === id) ?? EXAMPLE_THEME_PRESETS[0];
}

export function buildExampleEditorTheme(
    preset: ExampleThemePreset,
    fontSize: number,
    toolbarTheme: Required<EditorToolbarTheme>,
    overrides: ExampleEditorThemeOverrides = {}
): EditorTheme {
    return {
        backgroundColor: preset.backgroundColor,
        borderRadius: preset.editorBorderRadius,
        contentInsets: preset.contentInsets,
        placeholderColor: preset.placeholderColor,
        text: {
            color: preset.textColor,
            fontSize,
        },
        paragraph: {
            spacingAfter: preset.paragraphSpacingAfter,
            // `lineHeight` is absolute points, so it tracks the live font size.
            lineHeight: Math.round(fontSize * preset.lineHeightRatio),
        },
        headings: preset.headings,
        blockquote: {
            ...preset.blockquote,
            borderColor: overrides.blockquoteBorderColor ?? preset.blockquote.borderColor,
        },
        list: preset.list,
        horizontalRule: preset.horizontalRule,
        links: {
            ...preset.links,
            fontSize,
        },
        toolbar: toolbarTheme,
    };
}
