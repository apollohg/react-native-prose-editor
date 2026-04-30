export type EditorFontWeight =
    | 'normal'
    | 'bold'
    | '100'
    | '200'
    | '300'
    | '400'
    | '500'
    | '600'
    | '700'
    | '800'
    | '900';

export type EditorFontStyle = 'normal' | 'italic';

export interface EditorMentionTheme {
    textColor?: string;
    backgroundColor?: string;
    borderColor?: string;
    borderWidth?: number;
    borderRadius?: number;
    fontWeight?: EditorFontWeight;
    popoverBackgroundColor?: string;
    popoverBorderColor?: string;
    popoverBorderWidth?: number;
    popoverBorderRadius?: number;
    popoverShadowColor?: string;
    optionTextColor?: string;
    optionSecondaryTextColor?: string;
    optionHighlightedBackgroundColor?: string;
    optionHighlightedTextColor?: string;
}

export interface EditorTextStyle {
    fontFamily?: string;
    fontSize?: number;
    fontWeight?: EditorFontWeight;
    fontStyle?: EditorFontStyle;
    color?: string;
    lineHeight?: number;
    spacingAfter?: number;
}

export interface EditorHeadingTheme {
    h1?: EditorTextStyle;
    h2?: EditorTextStyle;
    h3?: EditorTextStyle;
    h4?: EditorTextStyle;
    h5?: EditorTextStyle;
    h6?: EditorTextStyle;
}

export interface EditorListTheme {
    indent?: number;
    baseIndentMultiplier?: number;
    itemSpacing?: number;
    markerColor?: string;
    markerScale?: number;
}

export interface EditorHorizontalRuleTheme {
    color?: string;
    thickness?: number;
    verticalMargin?: number;
}

export interface EditorBlockquoteTheme {
    text?: EditorTextStyle;
    indent?: number;
    borderColor?: string;
    borderWidth?: number;
    markerGap?: number;
}

export type EditorToolbarAppearance = 'custom' | 'native';

export interface EditorToolbarTheme {
    appearance?: EditorToolbarAppearance;
    backgroundColor?: string;
    borderColor?: string;
    borderWidth?: number;
    borderRadius?: number;
    marginTop?: number;
    showTopBorder?: boolean;
    keyboardOffset?: number;
    horizontalInset?: number;
    separatorColor?: string;
    buttonColor?: string;
    buttonActiveColor?: string;
    buttonDisabledColor?: string;
    buttonActiveBackgroundColor?: string;
    buttonBorderRadius?: number;
}

export interface EditorContentInsets {
    top?: number;
    right?: number;
    bottom?: number;
    left?: number;
}

export interface EditorTheme {
    text?: EditorTextStyle;
    paragraph?: EditorTextStyle;
    blockquote?: EditorBlockquoteTheme;
    headings?: EditorHeadingTheme;
    list?: EditorListTheme;
    horizontalRule?: EditorHorizontalRuleTheme;
    mentions?: EditorMentionTheme;
    toolbar?: EditorToolbarTheme;
    backgroundColor?: string;
    borderRadius?: number;
    contentInsets?: EditorContentInsets;
}

function stripUndefined(value: unknown): unknown {
    if (Array.isArray(value)) {
        return value.map((item) => stripUndefined(item)).filter((item) => item !== undefined);
    }

    if (value != null && typeof value === 'object') {
        const entries = Object.entries(value as Record<string, unknown>)
            .map(([key, entryValue]) => [key, stripUndefined(entryValue)] as const)
            .filter(([, entryValue]) => entryValue !== undefined);
        if (entries.length === 0) {
            return undefined;
        }
        return Object.fromEntries(entries);
    }

    if (typeof value === 'number' && !Number.isFinite(value)) {
        return undefined;
    }

    return value;
}

export function serializeEditorTheme(theme?: EditorTheme): string | undefined {
    if (!theme) return undefined;
    const cleaned = stripUndefined(theme);
    if (!cleaned || typeof cleaned !== 'object') return undefined;
    return JSON.stringify(cleaned);
}
