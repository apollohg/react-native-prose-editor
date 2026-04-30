import type {
    EditorHeadingTheme,
    EditorTheme,
    EditorMentionTheme,
    EditorToolbarTheme,
} from '@apollohg/react-native-prose-editor';

export interface ExampleThemePreset {
    id: string;
    label: string;
    statusBarStyle: 'dark' | 'light';
    textColor: string;
    backgroundColor: string;
    appChrome: {
        screenBackgroundColor: string;
        cardBackgroundColor: string;
        cardSecondaryBackgroundColor: string;
        eyebrowColor: string;
        titleColor: string;
        subtitleColor: string;
        sectionLabelColor: string;
        controlLabelColor: string;
        controlHintColor: string;
        tabBorderColor: string;
        tabBackgroundColor: string;
        tabActiveBorderColor: string;
        tabActiveBackgroundColor: string;
        tabTextColor: string;
        tabActiveTextColor: string;
        chipBorderColor: string;
        chipBackgroundColor: string;
        chipActiveBorderColor: string;
        chipActiveBackgroundColor: string;
        chipTextColor: string;
        chipActiveTextColor: string;
        sliderValueColor: string;
        colorValueColor: string;
        channelLabelColor: string;
        channelValueColor: string;
        colorTriggerBorderColor: string;
        colorTriggerBackgroundColor: string;
        colorTriggerExpandedBorderColor: string;
        colorTriggerExpandedBackgroundColor: string;
        actionButtonBackgroundColor: string;
        actionButtonTextColor: string;
        outputCardBackgroundColor: string;
        outputTextColor: string;
    };
    paragraphSpacingAfter: number;
    headings: EditorHeadingTheme;
    blockquote: {
        indent: number;
        borderColor: string;
        borderWidth: number;
        markerGap: number;
    };
    list: {
        indent: number;
        itemSpacing: number;
        markerColor: string;
        markerScale: number;
    };
    horizontalRule: {
        color: string;
        thickness: number;
        verticalMargin: number;
    };
    mentions: EditorMentionTheme;
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

export const DEFAULT_EXAMPLE_THEME_PRESET_ID = 'sand';

const DEFAULT_PARAGRAPH_SPACING_AFTER = 16;

const DEFAULT_LIST_THEME = {
    indent: 18,
    itemSpacing: 8,
    markerScale: 1.5,
} as const;

const DEFAULT_HORIZONTAL_RULE_THEME = {
    thickness: 1,
    verticalMargin: 12,
} as const;

const DEFAULT_BLOCKQUOTE_THEME = {
    indent: 18,
    borderWidth: 3,
    markerGap: 8,
} as const;

const DEFAULT_EDITOR_CONTENT_INSETS = {
    top: 16,
    right: 16,
    bottom: 16,
    left: 16,
} as const;

const DEFAULT_EDITOR_BORDER_RADIUS = 16;

function buildHeadingTheme(color: string): EditorHeadingTheme {
    return {
        h1: { color, fontSize: 32, fontWeight: '700', spacingAfter: 14 },
        h2: { color, fontSize: 28, fontWeight: '700', spacingAfter: 12 },
        h3: { color, fontSize: 24, fontWeight: '700', spacingAfter: 10 },
        h4: { color, fontSize: 20, fontWeight: '600', spacingAfter: 10 },
        h5: { color, fontSize: 18, fontWeight: '600', spacingAfter: 8 },
        h6: { color, fontSize: 16, fontWeight: '600', spacingAfter: 8 },
    };
}

const WARM_APP_CHROME = {
    screenBackgroundColor: '#ebe4da',
    cardBackgroundColor: '#fffaf4',
    cardSecondaryBackgroundColor: '#fffdf8',
    eyebrowColor: '#8d5b3d',
    titleColor: '#20170f',
    subtitleColor: '#5e4d41',
    sectionLabelColor: '#7b6859',
    controlLabelColor: '#2d2117',
    controlHintColor: '#6b5748',
    tabBorderColor: '#dcc8b5',
    tabBackgroundColor: '#fffdf8',
    tabActiveBorderColor: '#9a4f2d',
    tabActiveBackgroundColor: '#f7e6d8',
    tabTextColor: '#6b5748',
    tabActiveTextColor: '#6c3218',
    chipBorderColor: '#dcc8b5',
    chipBackgroundColor: '#fffdf8',
    chipActiveBorderColor: '#9a4f2d',
    chipActiveBackgroundColor: '#f7e6d8',
    chipTextColor: '#6b5748',
    chipActiveTextColor: '#6c3218',
    sliderValueColor: '#7b6859',
    colorValueColor: '#7b6859',
    channelLabelColor: '#6b5748',
    channelValueColor: '#7b6859',
    colorTriggerBorderColor: '#dcc8b5',
    colorTriggerBackgroundColor: '#ffffff',
    colorTriggerExpandedBorderColor: '#9a4f2d',
    colorTriggerExpandedBackgroundColor: '#fff7ef',
    actionButtonBackgroundColor: '#2d2117',
    actionButtonTextColor: '#fff8ef',
    outputCardBackgroundColor: '#1e2024',
    outputTextColor: '#eef4ff',
} as const;

export const EXAMPLE_THEME_PRESETS: readonly ExampleThemePreset[] = [
    {
        id: 'sand',
        label: 'Sand',
        statusBarStyle: 'dark',
        backgroundColor: '#f6f1e8',
        textColor: '#2a2118',
        appChrome: WARM_APP_CHROME,
        paragraphSpacingAfter: DEFAULT_PARAGRAPH_SPACING_AFTER,
        headings: buildHeadingTheme(WARM_APP_CHROME.titleColor),
        blockquote: {
            ...DEFAULT_BLOCKQUOTE_THEME,
            borderColor: '#c38d68',
        },
        list: {
            ...DEFAULT_LIST_THEME,
            markerColor: '#9a4f2d',
        },
        horizontalRule: {
            ...DEFAULT_HORIZONTAL_RULE_THEME,
            color: '#c38d68',
        },
        mentions: {
            textColor: '#7b2d12',
            backgroundColor: '#f4d8c7',
            borderColor: '#d8b19a',
            borderWidth: 1,
            borderRadius: 10,
            fontWeight: '700',
            popoverBackgroundColor: '#fff8ef',
            popoverBorderColor: '#dcc8b5',
            popoverBorderWidth: 1,
            popoverBorderRadius: 18,
            popoverShadowColor: '#6c3218',
            optionTextColor: '#2d2117',
            optionSecondaryTextColor: '#7b6859',
            optionHighlightedBackgroundColor: '#f7e6d8',
            optionHighlightedTextColor: '#6c3218',
        },
        toolbar: {
            appearance: 'native',
            backgroundColor: '#fff8ef',
            borderColor: '#dcc8b5',
            borderWidth: 1,
            borderRadius: 20,
            marginTop: 8,
            showTopBorder: false,
            keyboardOffset: 6,
            horizontalInset: 12,
            separatorColor: '#e6d4c4',
            buttonColor: '#7b4a30',
            buttonActiveColor: '#7b2d12',
            buttonDisabledColor: '#c7b29f',
            buttonActiveBackgroundColor: '#f4d8c7',
            buttonBorderRadius: 12,
        },
        slider: {
            minimumTrackTintColor: '#9a4f2d',
            maximumTrackTintColor: '#dcc8b5',
            thumbTintColor: '#6c3218',
        },
    },
    {
        id: 'slate',
        label: 'Slate',
        statusBarStyle: 'dark',
        backgroundColor: '#f0f2f5',
        textColor: '#1c2128',
        appChrome: {
            screenBackgroundColor: '#e4e7ec',
            cardBackgroundColor: '#f6f7f9',
            cardSecondaryBackgroundColor: '#fafbfc',
            eyebrowColor: '#2d7a6a',
            titleColor: '#1c2128',
            subtitleColor: '#586069',
            sectionLabelColor: '#6b7785',
            controlLabelColor: '#24292f',
            controlHintColor: '#57606a',
            tabBorderColor: '#cdd3db',
            tabBackgroundColor: '#f6f7f9',
            tabActiveBorderColor: '#2d7a6a',
            tabActiveBackgroundColor: '#daf0eb',
            tabTextColor: '#656d76',
            tabActiveTextColor: '#1a5c4f',
            chipBorderColor: '#cdd3db',
            chipBackgroundColor: '#f6f7f9',
            chipActiveBorderColor: '#2d7a6a',
            chipActiveBackgroundColor: '#daf0eb',
            chipTextColor: '#656d76',
            chipActiveTextColor: '#1a5c4f',
            sliderValueColor: '#6b7785',
            colorValueColor: '#6b7785',
            channelLabelColor: '#57606a',
            channelValueColor: '#6b7785',
            colorTriggerBorderColor: '#cdd3db',
            colorTriggerBackgroundColor: '#ffffff',
            colorTriggerExpandedBorderColor: '#2d7a6a',
            colorTriggerExpandedBackgroundColor: '#edf7f4',
            actionButtonBackgroundColor: '#24292f',
            actionButtonTextColor: '#f6f8fa',
            outputCardBackgroundColor: '#161b22',
            outputTextColor: '#e6edf3',
        },
        paragraphSpacingAfter: DEFAULT_PARAGRAPH_SPACING_AFTER,
        headings: buildHeadingTheme('#1c2128'),
        blockquote: {
            ...DEFAULT_BLOCKQUOTE_THEME,
            borderColor: '#89b9ad',
        },
        list: {
            ...DEFAULT_LIST_THEME,
            markerColor: '#2d7a6a',
        },
        horizontalRule: {
            ...DEFAULT_HORIZONTAL_RULE_THEME,
            color: '#d0d7de',
        },
        mentions: {
            textColor: '#1a5c4f',
            backgroundColor: '#daf0eb',
            borderColor: '#a8d5cb',
            borderWidth: 1,
            borderRadius: 8,
            fontWeight: '700',
            popoverBackgroundColor: '#f6f7f9',
            popoverBorderColor: '#cdd3db',
            popoverBorderWidth: 1,
            popoverBorderRadius: 14,
            popoverShadowColor: '#1c2128',
            optionTextColor: '#24292f',
            optionSecondaryTextColor: '#57606a',
            optionHighlightedBackgroundColor: '#daf0eb',
            optionHighlightedTextColor: '#1a5c4f',
        },
        toolbar: {
            appearance: 'custom',
            backgroundColor: '#f6f7f9',
            borderColor: '#cdd3db',
            borderWidth: 1,
            borderRadius: 14,
            marginTop: 8,
            showTopBorder: false,
            keyboardOffset: 6,
            horizontalInset: 12,
            separatorColor: '#d8dee4',
            buttonColor: '#424a53',
            buttonActiveColor: '#2d7a6a',
            buttonDisabledColor: '#9aa5b1',
            buttonActiveBackgroundColor: '#daf0eb',
            buttonBorderRadius: 8,
        },
        slider: {
            minimumTrackTintColor: '#2d7a6a',
            maximumTrackTintColor: '#cdd3db',
            thumbTintColor: '#1a5c4f',
        },
    },
    {
        id: 'midnight',
        label: 'Midnight',
        statusBarStyle: 'light',
        backgroundColor: '#161a24',
        textColor: '#dce3f0',
        appChrome: {
            screenBackgroundColor: '#0e1117',
            cardBackgroundColor: '#1a1e28',
            cardSecondaryBackgroundColor: '#1e2230',
            eyebrowColor: '#6b9bff',
            titleColor: '#e8edf6',
            subtitleColor: '#8892a8',
            sectionLabelColor: '#7a8499',
            controlLabelColor: '#c8d0e0',
            controlHintColor: '#737e96',
            tabBorderColor: '#2e3548',
            tabBackgroundColor: '#1a1e28',
            tabActiveBorderColor: '#4d7fff',
            tabActiveBackgroundColor: '#1e2d4e',
            tabTextColor: '#6b7894',
            tabActiveTextColor: '#7faaff',
            chipBorderColor: '#2e3548',
            chipBackgroundColor: '#1a1e28',
            chipActiveBorderColor: '#4d7fff',
            chipActiveBackgroundColor: '#1e2d4e',
            chipTextColor: '#6b7894',
            chipActiveTextColor: '#7faaff',
            sliderValueColor: '#7a8499',
            colorValueColor: '#7a8499',
            channelLabelColor: '#737e96',
            channelValueColor: '#7a8499',
            colorTriggerBorderColor: '#2e3548',
            colorTriggerBackgroundColor: '#161a24',
            colorTriggerExpandedBorderColor: '#4d7fff',
            colorTriggerExpandedBackgroundColor: '#1a2440',
            actionButtonBackgroundColor: '#4d7fff',
            actionButtonTextColor: '#0e1117',
            outputCardBackgroundColor: '#0a0d12',
            outputTextColor: '#c0cadc',
        },
        paragraphSpacingAfter: DEFAULT_PARAGRAPH_SPACING_AFTER,
        headings: buildHeadingTheme('#e8edf6'),
        blockquote: {
            ...DEFAULT_BLOCKQUOTE_THEME,
            borderColor: '#cc8f59',
        },
        list: {
            ...DEFAULT_LIST_THEME,
            markerColor: '#4d7fff',
        },
        horizontalRule: {
            ...DEFAULT_HORIZONTAL_RULE_THEME,
            color: '#2e3548',
        },
        mentions: {
            textColor: '#7faaff',
            backgroundColor: '#1e2d4e',
            borderColor: '#304a7a',
            borderWidth: 1,
            borderRadius: 10,
            fontWeight: '700',
            popoverBackgroundColor: '#1a1e28',
            popoverBorderColor: '#2e3548',
            popoverBorderWidth: 1,
            popoverBorderRadius: 18,
            popoverShadowColor: '#000000',
            optionTextColor: '#c8d0e0',
            optionSecondaryTextColor: '#737e96',
            optionHighlightedBackgroundColor: '#1e2d4e',
            optionHighlightedTextColor: '#7faaff',
        },
        toolbar: {
            appearance: 'custom',
            backgroundColor: '#1a1e28',
            borderColor: '#2e3548',
            borderWidth: 1,
            borderRadius: 20,
            marginTop: 8,
            showTopBorder: false,
            keyboardOffset: 6,
            horizontalInset: 12,
            separatorColor: '#252a38',
            buttonColor: '#8892a8',
            buttonActiveColor: '#7faaff',
            buttonDisabledColor: '#3a4258',
            buttonActiveBackgroundColor: '#1e2d4e',
            buttonBorderRadius: 12,
        },
        slider: {
            minimumTrackTintColor: '#4d7fff',
            maximumTrackTintColor: '#2e3548',
            thumbTintColor: '#7faaff',
        },
    },
    {
        id: 'ember',
        label: 'Ember',
        statusBarStyle: 'light',
        backgroundColor: '#1a1614',
        textColor: '#e4dbd0',
        appChrome: {
            screenBackgroundColor: '#121010',
            cardBackgroundColor: '#201c18',
            cardSecondaryBackgroundColor: '#251f1a',
            eyebrowColor: '#d4903a',
            titleColor: '#ede5d8',
            subtitleColor: '#918478',
            sectionLabelColor: '#847768',
            controlLabelColor: '#d4c8b8',
            controlHintColor: '#7e7268',
            tabBorderColor: '#362e26',
            tabBackgroundColor: '#201c18',
            tabActiveBorderColor: '#c07a2a',
            tabActiveBackgroundColor: '#3a2a16',
            tabTextColor: '#7e7268',
            tabActiveTextColor: '#e0a04a',
            chipBorderColor: '#362e26',
            chipBackgroundColor: '#201c18',
            chipActiveBorderColor: '#c07a2a',
            chipActiveBackgroundColor: '#3a2a16',
            chipTextColor: '#7e7268',
            chipActiveTextColor: '#e0a04a',
            sliderValueColor: '#847768',
            colorValueColor: '#847768',
            channelLabelColor: '#7e7268',
            channelValueColor: '#847768',
            colorTriggerBorderColor: '#362e26',
            colorTriggerBackgroundColor: '#1a1614',
            colorTriggerExpandedBorderColor: '#c07a2a',
            colorTriggerExpandedBackgroundColor: '#2e2214',
            actionButtonBackgroundColor: '#d4903a',
            actionButtonTextColor: '#121010',
            outputCardBackgroundColor: '#0e0c0a',
            outputTextColor: '#c8baa8',
        },
        paragraphSpacingAfter: DEFAULT_PARAGRAPH_SPACING_AFTER,
        headings: buildHeadingTheme('#ede5d8'),
        blockquote: {
            ...DEFAULT_BLOCKQUOTE_THEME,
            borderColor: '#a48fa7',
        },
        list: {
            ...DEFAULT_LIST_THEME,
            markerColor: '#d4903a',
        },
        horizontalRule: {
            ...DEFAULT_HORIZONTAL_RULE_THEME,
            color: '#362e26',
        },
        mentions: {
            textColor: '#e0a04a',
            backgroundColor: '#3a2a16',
            borderColor: '#5c4220',
            borderWidth: 1,
            borderRadius: 10,
            fontWeight: '700',
            popoverBackgroundColor: '#201c18',
            popoverBorderColor: '#362e26',
            popoverBorderWidth: 1,
            popoverBorderRadius: 18,
            popoverShadowColor: '#000000',
            optionTextColor: '#d4c8b8',
            optionSecondaryTextColor: '#7e7268',
            optionHighlightedBackgroundColor: '#3a2a16',
            optionHighlightedTextColor: '#e0a04a',
        },
        toolbar: {
            appearance: 'custom',
            backgroundColor: '#201c18',
            borderColor: '#362e26',
            borderWidth: 1,
            borderRadius: 16,
            marginTop: 8,
            showTopBorder: false,
            keyboardOffset: 6,
            horizontalInset: 12,
            separatorColor: '#2c2620',
            buttonColor: '#918478',
            buttonActiveColor: '#e0a04a',
            buttonDisabledColor: '#443c32',
            buttonActiveBackgroundColor: '#3a2a16',
            buttonBorderRadius: 10,
        },
        slider: {
            minimumTrackTintColor: '#c07a2a',
            maximumTrackTintColor: '#362e26',
            thumbTintColor: '#e0a04a',
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
        borderRadius: DEFAULT_EDITOR_BORDER_RADIUS,
        contentInsets: DEFAULT_EDITOR_CONTENT_INSETS,
        text: {
            color: preset.textColor,
            fontSize,
        },
        paragraph: {
            spacingAfter: preset.paragraphSpacingAfter,
        },
        headings: preset.headings,
        blockquote: {
            ...preset.blockquote,
            borderColor: overrides.blockquoteBorderColor ?? preset.blockquote.borderColor,
        },
        list: preset.list,
        horizontalRule: preset.horizontalRule,
        mentions: preset.mentions,
        toolbar: toolbarTheme,
    };
}
