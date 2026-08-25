/**
 * Font weight accepted by every themed text style. Numeric weights are
 * strings, matching React Native's `fontWeight`.
 */
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

/** Font slant accepted by every themed text style. */
export type EditorFontStyle = 'normal' | 'italic';

/** The mention itself, rendered inline in the document. */
export interface EditorMentionNodeTheme {
    /** Color of the mention label. */
    textColor?: string;
    /** Fill drawn behind the mention label. */
    backgroundColor?: string;
    /** Weight of the mention label. */
    fontWeight?: EditorFontWeight;
    /** Drawn by `NativeProseViewer`. The editor renders the node with text
     *  spans and cannot stroke a border, so these are ignored there. */
    borderColor?: string;
    borderWidth?: number;
    borderRadius?: number;
}

/** A single row in the suggestion list. */
export interface EditorMentionSuggestionOptionTheme {
    /** Color of the row's primary text (`MentionSuggestion.title`). */
    textColor?: string;
    /** Color of the row's secondary text (`MentionSuggestion.subtitle`). */
    secondaryTextColor?: string;
    /** Fill drawn behind an unhighlighted row. */
    backgroundColor?: string;
    borderColor?: string;
    /** Row border width, in layout units. */
    borderWidth?: number;
    /** Row corner radius, in layout units. */
    borderRadius?: number;
    /** Weight of the row's primary text. */
    fontWeight?: EditorFontWeight;
    /** Fill drawn behind the row under the cursor or pointer. */
    highlightedBackgroundColor?: string;
    /** Primary text color for the row under the cursor or pointer. */
    highlightedTextColor?: string;
}

/** The container behind the suggestion rows. Honored by the JavaScript
 *  `EditorToolbar`; the native keyboard toolbar styles rows only. */
export interface EditorMentionSuggestionsTheme {
    backgroundColor?: string;
    borderColor?: string;
    /** Container border width, in layout units. */
    borderWidth?: number;
    /** Container corner radius, in layout units. */
    borderRadius?: number;
    shadowColor?: string;
    /** Styling for each suggestion row inside the container. */
    option?: EditorMentionSuggestionOptionTheme;
}

/**
 * Mention styling. It is configured on the mentions addon
 * (`EditorAddons.mentions.theme` for the editor,
 * `NativeProseViewerAddons.mentions.theme` for the viewer) rather than on
 * {@link EditorTheme}, and can be overridden per mention through
 * `MentionsAddonConfig.resolveTheme`.
 */
export interface EditorMentionTheme {
    node?: EditorMentionNodeTheme;
    suggestions?: EditorMentionSuggestionsTheme;
}

/**
 * Typography for one kind of text block. Unset fields fall back to the
 * enclosing style — see the cascade described on {@link EditorTheme}.
 */
export interface EditorTextStyle {
    /** Platform font family name. Falls back to the system font when unresolvable. */
    fontFamily?: string;
    /** Text size, in layout units. */
    fontSize?: number;
    fontWeight?: EditorFontWeight;
    fontStyle?: EditorFontStyle;
    /** Text color. */
    color?: string;
    /** Total line height, in layout units — not a multiplier of `fontSize`. */
    lineHeight?: number;
    /** Space below each block using this style, in layout units. */
    spacingAfter?: number;
}

/** Styling for text carrying the `link` mark. */
export interface EditorLinkTheme {
    fontFamily?: string;
    /** Text size, in layout units. */
    fontSize?: number;
    fontWeight?: EditorFontWeight;
    fontStyle?: EditorFontStyle;
    color?: string;
    /** Fill drawn behind the link text. */
    backgroundColor?: string;
    /** Whether link text is underlined. */
    underline?: boolean;
}

/** Per-level heading typography, merged over the base `text` style. */
export interface EditorHeadingTheme {
    h1?: EditorTextStyle;
    h2?: EditorTextStyle;
    h3?: EditorTextStyle;
    h4?: EditorTextStyle;
    h5?: EditorTextStyle;
    h6?: EditorTextStyle;
}

export type EditorOrderedListNumberingScheme =
    | 'decimal'
    | 'lowerAlpha'
    | 'upperAlpha'
    | 'lowerRoman'
    | 'upperRoman';

export interface EditorOrderedListMarkerTheme {
    /**
     * Numbering schemes selected by visual list depth and cycled when necessary.
     * Defaults to `decimal`, `lowerAlpha`, `lowerRoman`.
     */
    schemes?: readonly EditorOrderedListNumberingScheme[];
    /** Punctuation drawn after the formatted index. Defaults to `.`. */
    suffix?: '.' | ')';
}

/** Layout of list indentation and markers. */
export interface EditorListTheme {
    /** Indentation added per nesting depth, in layout units. */
    indent?: number;
    /** Multiplier applied to the indentation of the outermost list level. */
    baseIndentMultiplier?: number;
    /** Space between consecutive list items, in layout units. */
    itemSpacing?: number;
    /** Space after a list before following content, including nested lists, in layout units. */
    spacingAfter?: number;
    /** Color of bullets and numbers. Defaults to the resolved text color. */
    markerColor?: string;
    /** Scale applied to the bullet glyph of unordered lists, relative to the item's font size. */
    markerScale?: number;
    /** Gap between the marker and the item text, in layout units. */
    markerGap?: number;
    /** Ordered-list marker presentation by visual nesting depth. */
    orderedMarker?: EditorOrderedListMarkerTheme;
}

/** The rule drawn for `horizontalRule` nodes. */
export interface EditorHorizontalRuleTheme {
    color?: string;
    /** Rule line thickness, in layout units. */
    thickness?: number;
    /** Space above and below the rule, in layout units. */
    verticalMargin?: number;
}

/** Blockquote typography and the leading border bar. */
export interface EditorBlockquoteTheme {
    /** Typography for text inside a blockquote, merged over the base `text` style. */
    text?: EditorTextStyle;
    /** Leading inset reserved per blockquote depth, in layout units. */
    indent?: number;
    /** Color of the vertical bar drawn beside the quote. */
    borderColor?: string;
    /** Width of the vertical bar, in layout units. */
    borderWidth?: number;
    /** Gap between the bar and the quoted text, in layout units. */
    markerGap?: number;
}

/** Code block typography and its background panel. */
export interface EditorCodeBlockTheme {
    /** Typography for code text. Falls back to the platform monospace font. */
    text?: EditorTextStyle;
    /** Fill drawn behind the block. */
    backgroundColor?: string;
    /** Panel corner radius, in layout units. */
    borderRadius?: number;
    /** Inner horizontal padding, in layout units. */
    paddingHorizontal?: number;
    /** Inner vertical padding, in layout units. */
    paddingVertical?: number;
}

/**
 * Toolbar chrome style. `'custom'` renders the package's own chrome from
 * this theme; `'native'` adopts the platform keyboard-accessory look, so the
 * platform supplies chrome metrics this theme would otherwise set.
 */
export type EditorToolbarAppearance = 'custom' | 'native';

/**
 * Toolbar chrome and button colors. Honored by both the JavaScript
 * `EditorToolbar` and the native keyboard toolbar unless a field says
 * otherwise.
 */
export interface EditorToolbarTheme {
    /** Chrome style. Defaults to `'custom'`. */
    appearance?: EditorToolbarAppearance;
    /** Toolbar height, in layout units. Defaults to a height that fits the buttons. */
    height?: number;
    backgroundColor?: string;
    borderColor?: string;
    /** Toolbar border width, in layout units. */
    borderWidth?: number;
    /** Toolbar corner radius, in layout units. */
    borderRadius?: number;
    /** Gap above an inline-placed toolbar, in layout units. Defaults to 8.
     *  Ignored for `toolbarPlacement="keyboard"`. */
    marginTop?: number;
    /** Whether the JavaScript toolbar draws its top separator line. Defaults to
     *  false through `NativeRichTextEditor`, true for a standalone `EditorToolbar`. */
    showTopBorder?: boolean;
    /** Gap between the keyboard and the toolbar, in layout units. Applies to
     *  `toolbarPlacement="keyboard"`; the platform default depends on `appearance`. */
    keyboardOffset?: number;
    /** Inset on each side of the keyboard-attached toolbar, in layout units.
     *  The platform default depends on `appearance`. */
    horizontalInset?: number;
    /** Color of `separator` toolbar items. */
    separatorColor?: string;
    /** Icon color of an idle button. */
    buttonColor?: string;
    /** Icon size for toolbar buttons, in layout units. */
    buttonIconSize?: number;
    /** Icon color of a button whose mark or node is active at the selection. */
    buttonActiveColor?: string;
    /** Icon color of a button the current selection cannot apply. */
    buttonDisabledColor?: string;
    /** Fill drawn behind an active button. */
    buttonActiveBackgroundColor?: string;
    /** Button corner radius, in layout units. */
    buttonBorderRadius?: number;
}

/** Padding between the editor's edges and its content, in layout units. */
export interface EditorContentInsets {
    top?: number;
    right?: number;
    bottom?: number;
    left?: number;
}

/**
 * Native content theme for `NativeRichTextEditor` and `NativeProseViewer`.
 *
 * Numeric values are layout units — density-independent pixels on Android,
 * points on iOS. Colors are strings, and the portable formats are `#RGB`,
 * `#RGBA`, `#RRGGBB`, `#RRGGBBAA`, `rgb(r, g, b)`, `rgba(r, g, b, a)`, and
 * `transparent`; named colors beyond `black`, `white`, `red`, `green`,
 * `blue`, and `gray` resolve on Android only.
 *
 * Text styles cascade: `text` is the base every block inherits, then
 * `blockquote.text` (inside a blockquote), then the node's own entry —
 * `paragraph`, `codeBlock.text`, or `headings.h1`…`headings.h6`. One
 * exception: `text.lineHeight` does not reach paragraphs, so set
 * `paragraph.lineHeight` to give paragraphs a line height.
 *
 * Mention styling lives on the mentions addon, not here — see
 * {@link EditorMentionTheme}.
 */
export interface EditorTheme {
    /** Base typography inherited by every block. */
    text?: EditorTextStyle;
    /** Paragraph typography, merged over `text`. */
    paragraph?: EditorTextStyle;
    blockquote?: EditorBlockquoteTheme;
    codeBlock?: EditorCodeBlockTheme;
    headings?: EditorHeadingTheme;
    list?: EditorListTheme;
    horizontalRule?: EditorHorizontalRuleTheme;
    links?: EditorLinkTheme;
    toolbar?: EditorToolbarTheme;
    /** Color of the editor's placeholder text. */
    placeholderColor?: string;
    /** Fill drawn behind the content. */
    backgroundColor?: string;
    /** Corner radius of the content surface, in layout units. */
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

// Mentions are configured on the mentions addon, not on EditorTheme; native
// still reads them from the theme payload.
export function serializeEditorTheme(
    theme?: EditorTheme,
    mentionTheme?: EditorMentionTheme
): string | undefined {
    const cleanedTheme = theme ? stripUndefined(theme) : undefined;
    const base =
        cleanedTheme && typeof cleanedTheme === 'object'
            ? (cleanedTheme as Record<string, unknown>)
            : undefined;
    const cleanedMentions = mentionTheme ? stripUndefined(mentionTheme) : undefined;

    if (cleanedMentions == null) {
        return base ? JSON.stringify(base) : undefined;
    }
    return JSON.stringify({ ...(base ?? {}), mentions: cleanedMentions });
}
