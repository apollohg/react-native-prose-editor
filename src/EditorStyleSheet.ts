import type { ExactEditorTheme } from './EditorStyleSheetTypes';
import { normalizeEditorTheme } from './EditorStyleSheetNormalization';

export const EditorStyleSheet = {
    create<const T extends object>(styles: T & ExactEditorTheme<T>): T {
        normalizeEditorTheme(styles);
        return styles;
    },
};
