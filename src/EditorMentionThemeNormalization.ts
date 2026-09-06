import type { EditorMentionTheme } from './EditorTheme';
import { normalizeEditorTheme } from './EditorStyleSheetNormalization';
import type { NormalizedEditorStyle } from './EditorStyleSheetTypes';
import { validEditorMentionTheme } from './EditorMentionThemeValidation';

export type SerializedEditorMentionTheme = Omit<EditorMentionTheme, 'node'> & {
    node?: NonNullable<EditorMentionTheme['node']> & { style?: NormalizedEditorStyle };
};

export function normalizeEditorMentionTheme(
    theme?: EditorMentionTheme
): SerializedEditorMentionTheme | undefined {
    if (theme === undefined) return undefined;
    if (!validEditorMentionTheme(theme)) throw new TypeError('Invalid mention theme');
    if (!theme.node) return theme;
    const node = theme.node as Record<string, unknown>;
    const { textColor, style: existingStyle, ...appearance } = node;
    const style = normalizeEditorTheme({
        mention: [{ color: textColor }, appearance, existingStyle],
    }).styles?.mention;
    return {
        ...theme,
        node: { style },
    };
}
