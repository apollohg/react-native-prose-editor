import type {
    EditorImageLoadingPolicy,
    NativeRichTextEditorAutoCapitalize,
    NativeRichTextEditorHeightBehavior,
    NativeRichTextEditorKeyboardType,
    NativeRichTextEditorToolbarPlacement,
    NativeRichTextEditorValueJSONUpdateMode,
} from '@apollohg/react-native-prose-editor';

import type { ControlledSourceMode } from './constants';

export type EditorEventEntry = {
    /** Monotonic, so React keys stay stable as the bounded log rolls over. */
    id: number;
    kind: string;
    detail: string;
    /** Seconds since the harness mounted. Avoids wall-clock noise in diffs. */
    atSeconds: number;
};

/** Grouped so the panels keep a consistent `settings` + `onChange` shape. */
export type EditorBehaviorSettings = {
    editable: boolean;
    autoFocus: boolean;
    autoCorrect: boolean;
    autoCapitalize: NativeRichTextEditorAutoCapitalize;
    keyboardType: NativeRichTextEditorKeyboardType;
    heightBehavior: NativeRichTextEditorHeightBehavior;
    showToolbar: boolean;
    toolbarPlacement: NativeRichTextEditorToolbarPlacement;
};

export type ImageSettings = {
    allowImageResizing: boolean;
    policy: EditorImageLoadingPolicy;
};

/** Which content source drives the editor, and how external writes apply. */
export type ControlledSettings = {
    mode: ControlledSourceMode;
    updateMode: NativeRichTextEditorValueJSONUpdateMode;
};
