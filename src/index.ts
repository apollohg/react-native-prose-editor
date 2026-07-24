export {
    NativeRichTextEditor,
    type NativeRichTextEditorProps,
    type NativeRichTextEditorRef,
    type NativeRichTextEditorCaretRect,
    type NativeRichTextEditorHeightBehavior,
    type NativeRichTextEditorToolbarPlacement,
    type NativeRichTextEditorValueJSONUpdateMode,
    type NativeRichTextEditorAutoCapitalize,
    type NativeRichTextEditorKeyboardType,
    type RemoteSelectionDecoration,
    type LinkRequestContext,
    type ImageRequestContext,
} from './NativeRichTextEditor';

export {
    NativeProseViewer,
    type NativeProseViewerProps,
    type NativeProseViewerAddons,
    type NativeProseViewerMentionsAddonConfig,
    type NativeProseViewerMentionPrefix,
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
    EditorLinkTheme,
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
    type MentionThemeResolveEvent,
    type MentionSelectEvent,
    type EditorAddonEvent,
} from './addons';

export {
    tiptapSchema,
    prosemirrorSchema,
    IMAGE_NODE_NAME,
    imageNodeSpec,
    withImagesSchema,
    buildDocumentFragmentJson,
    buildImageFragmentJson,
    resolveDocumentDescriptor,
    type ResolvedDocumentSchema,
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
    type YjsCollaborationEditorBindings,
} from './YjsCollaboration';

export {
    useNativeEditorDocument,
    type UseNativeEditorDocumentOptions,
    type UseNativeEditorDocumentReturn,
} from './useNativeEditor';

// Read-only types (no mutation methods)
export type {
    Selection,
    ActiveState,
    ReadonlyActiveState,
    HistoryState,
    EditorUpdate,
    RenderBlocksPatch,
    NativeEditorV2AtomicRenderSnapshot,
    DocumentJSON,
    CollaborationPeer,
} from './NativeEditorBridge';

export {
    createNativeEditorDocumentHandle,
    createNativeEditorLocalAwarenessSelection,
    type NativeEditorDocumentHandle,
    type NativeCollaborationProtocolAdapter,
    type NativeCollaborationProtocolAdapterAction,
    type NativeCollaborationProtocolAdapterContext,
    type NativeCollaborationProtocolAdapterEvent,
    type NativeCollaborationProtocolAdapterResult,
    type NativeCollaborationProtocolFrame,
    type NativeCollaborationTransportConfig,
    type NativeCollaborationTransportDiagnostics,
    type NativeCollaborationTransportEvent,
    type NativeEditorLocalAwarenessIntent,
    type NativeEditorLocalAwarenessSelection,
    type NativeEditorV2CreateConfig,
    type NativeEditorV2EditorState,
    type NativeEditorV2Initialization,
    type NativeEditorV2PeerInfo,
    type NativeEditorV2RoomSnapshot,
    type NativeEditorV2SnapshotMetadata,
} from './NativeEditorBridge';

export { clearHeightCache } from './heightCache';

export {
    DEFAULT_EDITOR_IMAGE_LOADING_POLICY,
    HARD_EDITOR_IMAGE_LOADING_POLICY,
    resolveEditorImageLoadingPolicy,
    type EditorImageLoadingPolicy,
    type ResolvedEditorImageLoadingPolicy,
} from './ImageLoadingPolicy';

export {
    DEFAULT_EDITOR_RESOURCE_LIMITS,
    HARD_EDITOR_RESOURCE_LIMITS,
    resolveEditorResourceLimits,
    type EditorCollaborationLimits,
    type EditorEditingLimits,
    type EditorResourceLimits,
    type ResolvedEditorResourceLimits,
} from './ResourceLimits';

export {
    NATIVE_EDITOR_BOUNDARY_ERROR_CODES,
    NATIVE_EDITOR_V2_NON_RETRYABLE_CODES,
    NativeEditorBoundaryError,
    NativeEditorV2ErrorBase,
    NativeEditorV2BoundaryError,
    NativeEditorV2DocumentError,
    NativeEditorV2OperationError,
    NativeEditorV2LifecycleError,
    NativeEditorV2SnapshotError,
    NativeEditorV2TransportError,
    NativeEditorV2NonRetryableError,
    parseNativeBoundaryError,
    type NativeEditorBoundaryErrorCode,
    type NativeEditorErrorDomain,
    type NativeEditorV2Error,
} from './NativeEditorBoundaryError';
