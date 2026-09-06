import type {
    EditorMentionNodeTheme,
    EditorMentionSuggestionOptionTheme,
    EditorMentionSuggestionsTheme,
    EditorMentionTheme,
} from './EditorTheme';
import { isPlainRecord, hasOnlyOwnKeys } from './NativeEditorResultNormalization';

export const MENTION_NODE_STRING_FIELDS = [
    'textColor',
    'backgroundColor',
    'borderColor',
] as const satisfies readonly (keyof EditorMentionNodeTheme)[];

export const MENTION_NODE_NUMBER_FIELDS = [
    'borderWidth',
    'borderRadius',
] as const satisfies readonly (keyof EditorMentionNodeTheme)[];

export const MENTION_OPTION_STRING_FIELDS = [
    'textColor',
    'secondaryTextColor',
    'backgroundColor',
    'borderColor',
    'highlightedBackgroundColor',
    'highlightedTextColor',
] as const satisfies readonly (keyof EditorMentionSuggestionOptionTheme)[];

export const MENTION_OPTION_NUMBER_FIELDS = [
    'borderWidth',
    'borderRadius',
] as const satisfies readonly (keyof EditorMentionSuggestionOptionTheme)[];

export const MENTION_SUGGESTIONS_STRING_FIELDS = [
    'backgroundColor',
    'borderColor',
    'shadowColor',
] as const satisfies readonly (keyof EditorMentionSuggestionsTheme)[];

export const MENTION_SUGGESTIONS_NUMBER_FIELDS = [
    'borderWidth',
    'borderRadius',
] as const satisfies readonly (keyof EditorMentionSuggestionsTheme)[];

export const EDITOR_MENTION_THEME_FONT_WEIGHTS: ReadonlySet<
    NonNullable<EditorMentionNodeTheme['fontWeight']>
> = new Set(['normal', 'bold', '100', '200', '300', '400', '500', '600', '700', '800', '900']);

export const EDITOR_MENTION_THEME_FIELDS = [
    'node',
    'suggestions',
] as const satisfies readonly (keyof EditorMentionTheme)[];

export function validMentionFontWeight(value: unknown): boolean {
    return (
        value === undefined ||
        (typeof value === 'string' &&
            EDITOR_MENTION_THEME_FONT_WEIGHTS.has(
                value as NonNullable<EditorMentionNodeTheme['fontWeight']>
            ))
    );
}

export function validMentionThemeSection(
    value: unknown,
    stringFields: readonly string[],
    numberFields: readonly string[],
    extraFields: readonly string[]
): value is Record<string, unknown> {
    if (!isPlainRecord(value)) return false;
    if (!hasOnlyOwnKeys(value, [...stringFields, ...numberFields, ...extraFields])) return false;
    return (
        stringFields.every(
            (field) => value[field] === undefined || typeof value[field] === 'string'
        ) &&
        numberFields.every(
            (field) =>
                value[field] === undefined ||
                (typeof value[field] === 'number' && Number.isFinite(value[field]))
        )
    );
}

export function validEditorMentionTheme(value: unknown): value is EditorMentionTheme {
    if (!isPlainRecord(value) || !hasOnlyOwnKeys(value, EDITOR_MENTION_THEME_FIELDS)) {
        return false;
    }

    const { node, suggestions } = value;
    if (node !== undefined) {
        if (
            !validMentionThemeSection(
                node,
                MENTION_NODE_STRING_FIELDS,
                MENTION_NODE_NUMBER_FIELDS,
                ['fontWeight']
            ) ||
            !validMentionFontWeight(node.fontWeight)
        ) {
            return false;
        }
    }
    if (suggestions === undefined) return true;
    if (
        !validMentionThemeSection(
            suggestions,
            MENTION_SUGGESTIONS_STRING_FIELDS,
            MENTION_SUGGESTIONS_NUMBER_FIELDS,
            ['option']
        )
    ) {
        return false;
    }

    const option = suggestions.option;
    if (option === undefined) return true;
    return (
        validMentionThemeSection(
            option,
            MENTION_OPTION_STRING_FIELDS,
            MENTION_OPTION_NUMBER_FIELDS,
            ['fontWeight']
        ) && validMentionFontWeight(option.fontWeight)
    );
}
