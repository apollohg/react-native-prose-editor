import type { EditorTheme, EditorMentionStyle } from './EditorStyleSheetTypes';
import { normalizeEditorTheme } from './EditorStyleSheetNormalization';
import { normalizeEditorMentionTheme } from './EditorMentionThemeNormalization';
export type {
    EditorTheme,
    EditorTextStyle,
    EditorLinkTheme,
    EditorHeadingTheme,
    EditorListTheme,
    EditorOrderedListNumberingScheme,
    EditorOrderedListMarkerTheme,
    EditorHorizontalRuleTheme,
    EditorBlockquoteTheme,
    EditorCodeBlockTheme,
} from './EditorStyleSheetTypes';

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
export interface EditorMentionNodeTheme extends EditorMentionStyle {
    /** @deprecated Use color. Retained for persisted mention themes. */
    textColor?: string;
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
 * through `createMentionsAddon({ theme })`, and can be overridden per mention through
 * `MentionsAddonConfig.resolveTheme`.
 */
export interface EditorMentionTheme {
    node?: EditorMentionNodeTheme;
    suggestions?: EditorMentionSuggestionsTheme;
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
     *  false through `RichTextEditor`, true for a standalone `EditorToolbar`. */
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
    /** Fill drawn behind an idle button. */
    buttonBackgroundColor?: string;
    /** Icon size for toolbar buttons, in layout units. */
    buttonIconSize?: number;
    /** Icon color of a button whose mark or node is active at the selection. */
    buttonActiveColor?: string;
    /** Icon color of a button the current selection cannot apply. */
    buttonDisabledColor?: string;
    /** Fill drawn behind an active button. */
    buttonActiveBackgroundColor?: string;
    /** Fill drawn behind a disabled button. */
    buttonDisabledBackgroundColor?: string;
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
    const normalized = theme === undefined ? undefined : normalizeEditorTheme(theme);
    const cleanedTheme =
        normalized && (normalized.styles || normalized.toolbar)
            ? stripUndefined(normalized)
            : undefined;
    const base =
        cleanedTheme && typeof cleanedTheme === 'object'
            ? (cleanedTheme as Record<string, unknown>)
            : undefined;
    const cleanedMentions = mentionTheme
        ? stripUndefined(normalizeEditorMentionTheme(mentionTheme))
        : undefined;

    if (cleanedMentions == null) {
        return base ? JSON.stringify(base) : undefined;
    }
    return JSON.stringify({ version: 1, ...(base ?? {}), mentions: cleanedMentions });
}
