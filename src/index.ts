export {
    NativeRichTextEditor,
    type NativeRichTextEditorProps,
    type NativeRichTextEditorRef,
    type NativeRichTextEditorHeightBehavior,
    type NativeRichTextEditorToolbarPlacement,
} from './NativeRichTextEditor';

export {
    EditorToolbar,
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    type EditorToolbarProps,
    type EditorToolbarItem,
    type EditorToolbarIcon,
    type EditorToolbarDefaultIconId,
    type EditorToolbarSFSymbolIcon,
    type EditorToolbarMaterialIcon,
    type EditorToolbarCommand,
    type EditorToolbarListType,
} from './EditorToolbar';
export type {
    EditorContentInsets,
    EditorTheme,
    EditorTextStyle,
    EditorHeadingTheme,
    EditorListTheme,
    EditorHorizontalRuleTheme,
    EditorMentionTheme,
    EditorToolbarTheme,
    EditorFontStyle,
    EditorFontWeight,
} from './EditorTheme';

export {
    MENTION_NODE_NAME,
    mentionNodeSpec,
    withMentionsSchema,
    buildMentionFragmentJson,
    type EditorAddons,
    type MentionsAddonConfig,
    type MentionSuggestion,
    type MentionQueryChangeEvent,
    type MentionSelectEvent,
    type EditorAddonEvent,
} from './addons';

export {
    tiptapSchema,
    prosemirrorSchema,
    type SchemaDefinition,
    type NodeSpec,
    type MarkSpec,
    type AttrSpec,
} from './schemas';

// Read-only types (no mutation methods)
export type {
    Selection,
    ActiveState,
    HistoryState,
    EditorUpdate,
    DocumentJSON,
} from './NativeEditorBridge';
