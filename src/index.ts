export {
    NativeRichTextEditor,
    type NativeRichTextEditorProps,
    type NativeRichTextEditorRef,
    type NativeRichTextEditorHeightBehavior,
    type NativeRichTextEditorToolbarPlacement,
    type RemoteSelectionDecoration,
    type LinkRequestContext,
    type ImageRequestContext,
} from './NativeRichTextEditor';

export {
    NativeProseViewer,
    type NativeProseViewerProps,
    type NativeProseViewerLinkPressEvent,
    type NativeProseViewerMentionRenderContext,
    type NativeProseViewerMentionPressEvent,
} from './NativeProseViewer';

export {
    EditorToolbar,
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    type EditorToolbarProps,
    type EditorToolbarItem,
    type EditorToolbarLeafItem,
    type EditorToolbarGroupChildItem,
    type EditorToolbarGroupItem,
    type EditorToolbarGroupPresentation,
    type EditorToolbarIcon,
    type EditorToolbarDefaultIconId,
    type EditorToolbarSFSymbolIcon,
    type EditorToolbarMaterialIcon,
    type EditorToolbarCommand,
    type EditorToolbarHeadingLevel,
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
    EditorToolbarAppearance,
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
    type MentionSelectionAttrsEvent,
    type MentionSelectEvent,
    type EditorAddonEvent,
} from './addons';

export {
    tiptapSchema,
    prosemirrorSchema,
    IMAGE_NODE_NAME,
    imageNodeSpec,
    withImagesSchema,
    buildImageFragmentJson,
    type SchemaDefinition,
    type NodeSpec,
    type MarkSpec,
    type AttrSpec,
    type ImageNodeAttributes,
} from './schemas';

export {
    createYjsCollaborationController,
    useYjsCollaboration,
    type YjsCollaborationOptions,
    type YjsCollaborationState,
    type YjsTransportStatus,
    type LocalAwarenessState,
    type LocalAwarenessUser,
    type UseYjsCollaborationResult,
    type YjsCollaborationController,
} from './YjsCollaboration';

// Read-only types (no mutation methods)
export type {
    Selection,
    ActiveState,
    HistoryState,
    EditorUpdate,
    DocumentJSON,
    CollaborationPeer,
    EncodedCollaborationStateInput,
} from './NativeEditorBridge';

export {
    encodeCollaborationStateBase64,
    decodeCollaborationStateBase64,
} from './NativeEditorBridge';
