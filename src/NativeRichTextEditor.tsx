import React, {
    forwardRef,
    useCallback,
    useEffect,
    useImperativeHandle,
    useMemo,
    useRef,
    useState,
} from 'react';
import {
    PixelRatio,
    Platform,
    StyleSheet,
    View,
    type NativeSyntheticEvent,
    type StyleProp,
    type ViewStyle,
} from 'react-native';
import { requireNativeViewManager } from 'expo-modules-core';

import {
    _assertNativeEditorDocumentHandle,
    _getNativeEditorDocumentHandleDescriptor,
    normalizeNativeEditorV2DecimalId,
    requireNativeEditorV2U32,
    type ActiveState,
    type DocumentJSON,
    type HistoryState,
    type NativeEditorDocumentHandle,
    type Selection,
} from './NativeEditorBridge';
import { NativeEditorV2OperationError } from './NativeEditorBoundaryError';
import { useNativeEditorDocument } from './useNativeEditor';
import {
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    EditorToolbar,
    type EditorToolbarCommand,
    type EditorToolbarGroupChildItem,
    type EditorToolbarHeadingLevel,
    type EditorToolbarIcon,
    type EditorToolbarItem,
    type EditorToolbarListType,
} from './EditorToolbar';
import { serializeEditorTheme, type EditorTheme } from './EditorTheme';
import {
    serializeEditorImageLoadingPolicy,
    type EditorImageLoadingPolicy,
} from './ImageLoadingPolicy';
import { serializeEditorAddons, type EditorAddons } from './addons';
import {
    buildImageFragmentJson,
    IMAGE_NODE_NAME,
    normalizeDocumentJson,
    type ImageNodeAttributes,
} from './schemas';

interface NativeEditorViewHandle {
    focus?: () => void;
    blur?: () => void;
    getCaretRect?: () => Promise<string | null> | string | null;
}

interface NativeEditorViewProps {
    style?: StyleProp<ViewStyle>;
    accessibilityLabel?: string;
    accessibilityHint?: string;
    editorId: string;
    placeholder?: string;
    editable: boolean;
    autoFocus: boolean;
    autoCapitalize?: NativeRichTextEditorAutoCapitalize;
    autoCorrect?: boolean;
    keyboardType?: NativeRichTextEditorKeyboardType;
    showToolbar: boolean;
    toolbarPlacement: NativeRichTextEditorToolbarPlacement;
    heightBehavior: NativeRichTextEditorHeightBehavior;
    allowImageResizing: boolean;
    imageLoadingPolicyJson?: string;
    themeJson?: string;
    addonsJson?: string;
    toolbarItemsJson?: string;
    remoteSelectionsJson?: string;
    editorUpdateJson?: string;
    editorUpdateEditorId?: string;
    editorUpdateRevision?: number;
    onEditorUpdate: (event: NativeSyntheticEvent<NativeUpdateEvent>) => void;
    onSelectionChange: (event: NativeSyntheticEvent<NativeSelectionEvent>) => void;
    onFocusChange: (event: NativeSyntheticEvent<NativeFocusEvent>) => void;
    onContentHeightChange: (event: NativeSyntheticEvent<NativeContentHeightEvent>) => void;
    onToolbarAction: (event: NativeSyntheticEvent<NativeToolbarActionEvent>) => void;
    onAddonEvent: (event: NativeSyntheticEvent<NativeAddonEvent>) => void;
}

const NativeEditorView = requireNativeViewManager('NativeEditor') as React.ComponentType<
    NativeEditorViewProps & React.RefAttributes<NativeEditorViewHandle>
>;

interface NativeUpdateEvent {
    updateJson?: string;
    editorId?: string;
    documentVersion?: string;
}

interface NativeSelectionEvent {
    anchor: number;
    head: number;
    stateJson?: string;
    editorId?: string;
    documentVersion?: string;
}

interface NativeFocusEvent {
    isFocused: boolean;
    editorId?: string;
}

interface NativeContentHeightEvent {
    contentHeight: number;
    editorId?: string;
}

interface NativeToolbarActionEvent {
    key: string;
    editorId?: string;
    updateJson?: string;
    stateJson?: string;
    documentVersion?: string;
}

interface NativeAddonEvent {
    eventJson: string;
    editorId?: string;
}

const LINK_TOOLBAR_ACTION_KEY = '__native-editor-link__';
const IMAGE_TOOLBAR_ACTION_KEY = '__native-editor-image__';

const EMPTY_ACTIVE_STATE: ActiveState = {
    marks: {},
    markAttrs: {},
    nodes: {},
    commands: {},
    allowedMarks: [],
    insertableNodes: [],
};

function isRecord(value: unknown): value is Record<string, unknown> {
    return value != null && typeof value === 'object' && !Array.isArray(value);
}

function parseSelectionFromUpdate(value: unknown): Selection | null {
    if (!isRecord(value)) return null;
    if (value.type === 'all') return { type: 'all' };
    if (value.type === 'node' && typeof value.pos === 'number') {
        return { type: 'node', pos: value.pos };
    }
    if (
        value.type === 'text' &&
        typeof value.anchor === 'number' &&
        typeof value.head === 'number'
    ) {
        return { type: 'text', anchor: value.anchor, head: value.head };
    }
    return null;
}

function stringArray(value: unknown): string[] {
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];
}

function booleanMap(value: unknown): Record<string, boolean> {
    if (!isRecord(value)) return {};
    const result: Record<string, boolean> = {};
    for (const key of Object.keys(value)) {
        if (typeof value[key] === 'boolean') result[key] = value[key] as boolean;
    }
    return result;
}

function parseActiveStateFromUpdate(value: unknown): ActiveState | null {
    if (!isRecord(value)) return null;
    return {
        marks: booleanMap(value.marks),
        markAttrs: isRecord(value.markAttrs)
            ? (value.markAttrs as Record<string, Record<string, unknown>>)
            : {},
        nodes: booleanMap(value.nodes),
        commands: booleanMap(value.commands),
        allowedMarks: stringArray(value.allowedMarks),
        insertableNodes: stringArray(value.insertableNodes),
    };
}

function isRevisionMismatchError(error: unknown): boolean {
    return (
        error instanceof NativeEditorV2OperationError && error.code === 'REVISION_MISMATCH'
    );
}

function mapToolbarChildForNative(
    item: EditorToolbarGroupChildItem,
    activeState: ActiveState,
    editable: boolean,
    onRequestLink?: NativeRichTextEditorProps['onRequestLink'],
    onRequestImage?: NativeRichTextEditorProps['onRequestImage']
): EditorToolbarGroupChildItem {
    if (item.type === 'link') {
        return {
            type: 'action',
            key: LINK_TOOLBAR_ACTION_KEY,
            label: item.label,
            icon: item.icon as EditorToolbarIcon,
            placement: item.placement,
            isActive: activeState.marks.link === true,
            isDisabled: !editable || !onRequestLink || !activeState.allowedMarks.includes('link'),
        };
    }
    if (item.type === 'image') {
        return {
            type: 'action',
            key: IMAGE_TOOLBAR_ACTION_KEY,
            label: item.label,
            icon: item.icon as EditorToolbarIcon,
            placement: item.placement,
            isActive: false,
            isDisabled:
                !editable ||
                !onRequestImage ||
                !activeState.insertableNodes.includes(IMAGE_NODE_NAME),
        };
    }
    return item;
}

function mapToolbarItemsForNative(
    items: readonly EditorToolbarItem[],
    activeState: ActiveState,
    editable: boolean,
    onRequestLink?: NativeRichTextEditorProps['onRequestLink'],
    onRequestImage?: NativeRichTextEditorProps['onRequestImage']
): EditorToolbarItem[] {
    return items.map((item) => {
        if (item.type === 'group') {
            return {
                ...item,
                items: item.items.map((child) =>
                    mapToolbarChildForNative(child, activeState, editable, onRequestLink, onRequestImage)
                ),
            };
        }
        if (item.type === 'separator') {
            return item;
        }
        return mapToolbarChildForNative(item, activeState, editable, onRequestLink, onRequestImage);
    });
}

function serializeRemoteSelections(
    remoteSelections?: readonly RemoteSelectionDecoration[]
): string | undefined {
    if (!remoteSelections || remoteSelections.length === 0) {
        return undefined;
    }
    const normalized = remoteSelections.map((selection) => {
        const clientId = normalizeNativeEditorV2DecimalId(selection.clientId);
        if (clientId == null) {
            throw new Error('NativeRichTextEditor: remote clientId must be canonical decimal u64');
        }
        return {
            ...selection,
            clientId,
            anchor: requireNativeEditorV2U32(selection.anchor, 'remote selection anchor'),
            head: requireNativeEditorV2U32(selection.head, 'remote selection head'),
        };
    });
    return stringifyCachedJson(normalized);
}

function parseCaretRectJson(raw: string | null | undefined): NativeRichTextEditorCaretRect | null {
    if (!raw) {
        return null;
    }

    try {
        const parsed = JSON.parse(raw) as Record<string, unknown>;
        const x = typeof parsed.x === 'number' ? parsed.x : null;
        const y = typeof parsed.y === 'number' ? parsed.y : null;
        const width = typeof parsed.width === 'number' ? parsed.width : null;
        const height = typeof parsed.height === 'number' ? parsed.height : null;
        const editorWidth = typeof parsed.editorWidth === 'number' ? parsed.editorWidth : null;
        const editorHeight = typeof parsed.editorHeight === 'number' ? parsed.editorHeight : null;
        if (
            x == null ||
            y == null ||
            width == null ||
            height == null ||
            editorWidth == null ||
            editorHeight == null
        ) {
            return null;
        }
        return { x, y, width, height, editorWidth, editorHeight };
    } catch {
        return null;
    }
}

const serializedJsonCache = new WeakMap<object, string>();

function stringifyCachedJson(value: unknown): string {
    if (value != null && typeof value === 'object') {
        const cached = serializedJsonCache.get(value);
        if (cached != null) {
            return cached;
        }
        const serialized = JSON.stringify(value);
        serializedJsonCache.set(value, serialized);
        return serialized;
    }
    return JSON.stringify(value);
}

function useSerializedValue<T>(
    value: T | null | undefined,
    serialize: (value: T) => string | undefined,
    revision?: unknown
): string | undefined {
    const cacheRef = useRef<{
        value: T | null | undefined;
        revision: unknown;
        hasRevision: boolean;
        serialized: string | undefined;
    } | null>(null);
    const hasRevision = revision !== undefined;
    const cached = cacheRef.current;

    if (cached) {
        if (hasRevision && cached.hasRevision && Object.is(cached.revision, revision)) {
            return cached.serialized;
        }
        if (Object.is(cached.value, value) && cached.hasRevision === hasRevision) {
            return cached.serialized;
        }
    }

    const serialized = value == null ? undefined : serialize(value);
    cacheRef.current = {
        value,
        revision,
        hasRevision,
        serialized,
    };
    return serialized;
}

export type NativeRichTextEditorHeightBehavior = 'fixed' | 'autoGrow';
export type NativeRichTextEditorToolbarPlacement = 'keyboard' | 'inline';
export type NativeRichTextEditorValueJSONUpdateMode = 'replace' | 'reset';
export type NativeRichTextEditorAutoCapitalize = 'none' | 'sentences' | 'words' | 'characters';
export type NativeRichTextEditorKeyboardType =
    | 'default'
    | 'email-address'
    | 'numeric'
    | 'phone-pad'
    | 'ascii-capable'
    | 'numbers-and-punctuation'
    | 'url'
    | 'number-pad'
    | 'name-phone-pad'
    | 'decimal-pad'
    | 'twitter'
    | 'web-search'
    | 'visible-password'
    | 'ascii-capable-number-pad';

export interface RemoteSelectionDecoration {
    clientId: string;
    anchor: number;
    head: number;
    color: string;
    name?: string;
    avatarUrl?: string;
    isFocused?: boolean;
}

export interface LinkRequestContext {
    /** Current link href at the selection, when one is active. */
    href?: string;
    /** Whether a link mark is active at the selection. */
    isActive: boolean;
    /** The selection the request was issued for (engine doc positions). */
    selection: Selection;
    /** Apply or update the link on the engine selection. */
    setLink: (href: string) => void;
    /** Remove the link from the engine selection. */
    unsetLink: () => void;
}

export interface ImageRequestContext {
    /** The selection the request was issued for (engine doc positions). */
    selection: Selection;
    /** Insert a block image node at the engine selection. */
    insertImage: (src: string, attrs?: Omit<ImageNodeAttributes, 'src'>) => void;
}

/**
 * The v2 prop contract. Initialization and input policy are NOT props:
 * - `initialContent` / `initialJSON` are gone: initial document state
 *   belongs to the handle's creation config (`NativeEditorV2Initialization`:
 *   `localEmpty` / `localJson` / `localHtml` / `room`).
 * - `schema` and `fragmentName` belong to `NativeEditorV2CreateConfig`.
 * - `maxLength`, engine-enforced `allowBase64Images`, `readOnly`, and
 *   `inputFilter` belong to `NativeEditorV2CreateConfig.policy`.
 * - `resourceLimits` belongs to `NativeEditorV2CreateConfig.limits.resource`.
 *   `editable` is only a per-view interaction gate; it never changes the
 *   handle's engine read-only policy.
 * - `autoDetectLinks`, `preserveSelectionOnValueJSONReset`, and
 *   `selectionOnValueJSONReset` were removed with the legacy bridge and have
 *   no v2 equivalent.
 */
export interface NativeRichTextEditorProps {
    /** Accessible name announced for the native editable control. */
    accessibilityLabel?: string;
    /** Additional guidance announced for the native editable control. */
    accessibilityHint?: string;
    /** Controlled HTML content. External changes are diffed and applied. */
    value?: string;
    /** Controlled ProseMirror JSON content. Ignored if value is set. */
    valueJSON?: DocumentJSON;
    /** Optional stable revision hint for `valueJSON` to avoid reserializing equal docs on rerender. */
    valueJSONRevision?: string;
    /** Controls how external `valueJSON` changes are applied. Defaults to preserving undo history. */
    valueJSONUpdateMode?: NativeRichTextEditorValueJSONUpdateMode;
    /** Placeholder text shown when editor is empty. */
    placeholder?: string;
    /** Whether the editor is editable. Defaults to true. When false, every mutation ref method rejects with MUTATION_REJECTED; selection and controlled content still flow. */
    editable?: boolean;
    /** Whether to auto-focus on mount. */
    autoFocus?: boolean;
    /** Controls native keyboard auto-capitalization. Defaults to sentences. */
    autoCapitalize?: NativeRichTextEditorAutoCapitalize;
    /** Controls native keyboard autocorrection. Defaults to the platform-specific editor default. */
    autoCorrect?: boolean;
    /** Controls the native keyboard layout. Defaults to the platform default keyboard. */
    keyboardType?: NativeRichTextEditorKeyboardType;
    /** Controls whether the editor scrolls internally or grows with content. */
    heightBehavior?: NativeRichTextEditorHeightBehavior;
    /** Whether to show the formatting toolbar. Defaults to true. */
    showToolbar?: boolean;
    /** Whether the toolbar is attached to the keyboard natively or rendered inline in React. */
    toolbarPlacement?: NativeRichTextEditorToolbarPlacement;
    /** Displayed toolbar buttons, in order. Supports custom marks/nodes. */
    toolbarItems?: readonly EditorToolbarItem[];
    /** Called when a custom `action` toolbar item is pressed. */
    onToolbarAction?: (key: string) => void;
    /** Called when a toolbar link item is pressed so the host can collect/edit a URL. */
    onRequestLink?: (context: LinkRequestContext) => void;
    /** Called when a toolbar image item is pressed so the host can choose an image source. */
    onRequestImage?: (context: ImageRequestContext) => void;
    /** Bounds native data-URL and remote image loading. */
    imageLoadingPolicy?: EditorImageLoadingPolicy;
    /** Whether selected images show native resize handles. */
    allowImageResizing?: boolean;
    /** Called when content changes with the current HTML. */
    onContentChange?: (html: string) => void;
    /** Called when content changes with the current ProseMirror JSON. */
    onContentChangeJSON?: (json: DocumentJSON) => void;
    /** Called when selection changes (engine doc positions). */
    onSelectionChange?: (selection: Selection) => void;
    /** Called when active formatting state changes. */
    onActiveStateChange?: (state: ActiveState) => void;
    /** Called when undo/redo availability changes. */
    onHistoryStateChange?: (state: HistoryState) => void;
    /** Called when the editor gains focus. */
    onFocus?: () => void;
    /** Called when the editor loses focus. */
    onBlur?: () => void;
    /** Style applied to the native editor view. */
    style?: StyleProp<ViewStyle>;
    /** Style applied to the outer React container wrapping the editor and inline toolbar. */
    containerStyle?: StyleProp<ViewStyle>;
    /** Optional native content theme applied to rendered blocks and typing attrs. */
    theme?: EditorTheme;
    /** Optional addon configuration. */
    addons?: EditorAddons;
    /** Remote awareness selections rendered as native overlays. */
    remoteSelections?: readonly RemoteSelectionDecoration[];
    /**
     * Shared v2 document session — the only construction path. The native
     * view binds to the same session (its editorId is passed straight to the
     * view), so typing, IME, selection, and toolbar commands flow through
     * the native v2 adapters into the shared engine while this component
     * drives the retained document API (`value`, `valueJSON`, `setContent`,
     * `setContentJson`, `getContent`, `getContentJson`) through the same
     * handle. The same handle is handed to the collaboration controller
     * (one session). Initialization belongs to the handle's creation config.
     * A room handle awaiting the server document renders nothing until Rust
     * promotes an accepted Step 2.
     */
    documentHandle: NativeEditorDocumentHandle;
    /**
     * Revision signal rendered by the collaboration controller. Advances
     * trigger an authoritative engine re-read (remote commits, promotions).
     */
    documentRevision?: string | null;
    /** Pinged after each successful local engine mutation (JS- or adapter-driven) so the collaboration controller can flush outbound frames. */
    onLocalDocumentCommit?: () => void;
}

export interface NativeRichTextEditorRef {
    /** Programmatically focus the editor. */
    focus(): void;
    /** Programmatically blur the editor. */
    blur(): void;
    /** Toggle a formatting mark (e.g. 'bold', 'italic'). */
    toggleMark(markType: string): void;
    /** Apply or update a hyperlink on the current selection. */
    setLink(href: string): void;
    /** Remove a hyperlink from the current selection. */
    unsetLink(): void;
    /** Toggle blockquote wrapping around the current block selection. */
    toggleBlockquote(): void;
    /** Toggle a heading level on the current block selection. */
    toggleHeading(level: EditorToolbarHeadingLevel): void;
    /** Toggle a list type supported by the active schema. */
    toggleList(listType: string): void;
    /** Indent the current list item. */
    indentListItem(): void;
    /** Outdent the current list item. */
    outdentListItem(): void;
    /** Insert a void node (e.g. 'horizontalRule'). */
    insertNode(nodeType: string): void;
    /** Insert a block image node with the given source and optional metadata. */
    insertImage(src: string, attrs?: Omit<ImageNodeAttributes, 'src'>): void;
    /** Insert text at the current cursor position. */
    insertText(text: string): void;
    /** Insert HTML content at the current selection. */
    insertContentHtml(html: string): void;
    /** Insert JSON content at the current selection. */
    insertContentJson(doc: DocumentJSON): void;
    /** Replace entire document with HTML (preserves undo history). */
    setContent(html: string): void;
    /** Replace entire document with JSON (preserves undo history). */
    setContentJson(doc: DocumentJSON): void;
    /** Clear the document to the active schema's empty text block. */
    clearContent(): void;
    /** Get the current HTML content. */
    getContent(): string;
    /** Get the current content as ProseMirror JSON. */
    getContentJson(): DocumentJSON;
    /** Get the plain text content (no markup). */
    getTextContent(): string;
    /** Get the current caret rectangle in editor-local layout coordinates. */
    getCaretRect(): Promise<NativeRichTextEditorCaretRect | null>;
    /** Undo the last operation. */
    undo(): void;
    /** Redo the last undone operation. */
    redo(): void;
    /** Check if undo is available. */
    canUndo(): boolean;
    /** Check if redo is available. */
    canRedo(): boolean;
}

export interface NativeRichTextEditorCaretRect {
    /** Left edge of the caret, relative to the editor root view. */
    x: number;
    /** Top edge of the caret, relative to the editor root view. */
    y: number;
    /** Caret width. */
    width: number;
    /** Caret height. */
    height: number;
    /** Current editor root view width. */
    editorWidth: number;
    /** Current editor root view height. */
    editorHeight: number;
}

/**
 * Renders a shared v2 document session as a genuinely interactive editor.
 * The native view binds to the handle's session id, so the Task 15 native
 * v2 adapters own typing/IME (one commit per transaction; transient
 * composing text never reaches the engine), selection mirroring, and the
 * native toolbar. This component owns the JS side: controlled
 * `value`/`valueJSON`, the typing/command ref methods (routed through the
 * v2 bridge with refresh-never-retry mismatch semantics), the link/image
 * request flows, and pushing the engine's render update back to the view
 * after every JS-driven change. A room document awaiting the server renders
 * nothing (loading), never an unshared fallback paragraph.
 */
export const NativeRichTextEditor = forwardRef<
    NativeRichTextEditorRef,
    NativeRichTextEditorProps
>(function NativeRichTextEditor(
    {
        documentHandle,
        documentRevision,
        value,
        valueJSON,
        valueJSONRevision,
        valueJSONUpdateMode = 'replace',
        placeholder,
        editable = true,
        autoFocus = false,
        autoCapitalize,
        autoCorrect,
        keyboardType,
        heightBehavior = 'autoGrow',
        showToolbar = true,
        toolbarPlacement = 'keyboard',
        toolbarItems = DEFAULT_EDITOR_TOOLBAR_ITEMS,
        onToolbarAction,
        onRequestLink,
        onRequestImage,
        imageLoadingPolicy,
        accessibilityLabel,
        accessibilityHint,
        style,
        containerStyle,
        theme,
        addons,
        remoteSelections,
        allowImageResizing = true,
        onContentChange,
        onContentChangeJSON,
        onSelectionChange,
        onActiveStateChange,
        onHistoryStateChange,
        onFocus,
        onBlur,
        onLocalDocumentCommit,
    },
    ref
) {
    _assertNativeEditorDocumentHandle(documentHandle);
    const documentDescriptor = _getNativeEditorDocumentHandleDescriptor(documentHandle);

    const serializedValueJson = useSerializedValue(
        valueJSON,
        (doc) => stringifyCachedJson(normalizeDocumentJson(doc, documentDescriptor)),
        valueJSONRevision
    );
    const controlledValueJSON = useMemo(
        () =>
            serializedValueJson == null
                ? undefined
                : (JSON.parse(serializedValueJson) as DocumentJSON),
        [serializedValueJson]
    );

    const document = useNativeEditorDocument({
        handle: documentHandle,
        value,
        valueJSON: controlledValueJSON,
        valueJSONUpdateMode,
        revisionSignal: documentRevision ?? null,
        onContentChange,
        onContentChangeJSON,
        onHistoryStateChange,
        onLocalDocumentCommit,
    });

    const bridge = documentHandle.bridge;
    const editorId = documentHandle.editorId;

    // ── Prop refs ───────────────────────────────────────────────
    const onSelectionChangeRef = useRef(onSelectionChange);
    onSelectionChangeRef.current = onSelectionChange;
    const onActiveStateChangeRef = useRef(onActiveStateChange);
    onActiveStateChangeRef.current = onActiveStateChange;
    const onFocusRef = useRef(onFocus);
    onFocusRef.current = onFocus;
    const onBlurRef = useRef(onBlur);
    onBlurRef.current = onBlur;
    const onToolbarActionRef = useRef(onToolbarAction);
    onToolbarActionRef.current = onToolbarAction;
    const onRequestLinkRef = useRef(onRequestLink);
    onRequestLinkRef.current = onRequestLink;
    const onRequestImageRef = useRef(onRequestImage);
    onRequestImageRef.current = onRequestImage;
    const onLocalDocumentCommitRef = useRef(onLocalDocumentCommit);
    onLocalDocumentCommitRef.current = onLocalDocumentCommit;

    // ── Engine-observed interactive state ───────────────────────
    const [activeState, setActiveState] = useState<ActiveState>(EMPTY_ACTIVE_STATE);
    const activeStateRef = useRef<ActiveState>(EMPTY_ACTIVE_STATE);
    const activeStateKeyRef = useRef<string | null>(null);
    const selectionRef = useRef<Selection>({ type: 'text', anchor: 0, head: 0 });
    const isFocusedRef = useRef(false);
    const latestRevisionRef = useRef<string | null>(null);
    latestRevisionRef.current = document.documentRevision;

    const applyUpdateState = useCallback((updateJson: string | null | undefined) => {
        if (typeof updateJson !== 'string' || updateJson.length === 0) return null;
        let parsed: Record<string, unknown>;
        try {
            const candidate = JSON.parse(updateJson) as unknown;
            if (!isRecord(candidate)) return null;
            parsed = candidate;
        } catch {
            return null;
        }
        const nextSelection = parseSelectionFromUpdate(parsed.selection);
        if (nextSelection) {
            selectionRef.current = nextSelection;
        }
        const nextActiveState = parseActiveStateFromUpdate(parsed.activeState);
        if (nextActiveState) {
            activeStateRef.current = nextActiveState;
            setActiveState(nextActiveState);
            const key = stringifyCachedJson(nextActiveState);
            if (key !== activeStateKeyRef.current) {
                activeStateKeyRef.current = key;
                onActiveStateChangeRef.current?.(nextActiveState);
            }
        }
        return parsed;
    }, []);

    // ── View update pushes (JS-driven engine changes only) ──────
    const [pushedUpdate, setPushedUpdate] = useState<{ json: string; revision: number } | null>(
        null
    );
    const pushRevisionRef = useRef(0);
    const lastPushedEngineRevisionRef = useRef<string | null>(null);
    const lastNativeDrivenRevisionRef = useRef<string | null>(null);
    const didObserveInitialRevisionRef = useRef(false);

    const pushEngineUpdateToView = useCallback(() => {
        if (documentHandle.isDestroyed) return;
        const updateJson = bridge.renderUpdate();
        const parsed = applyUpdateState(updateJson);
        const documentVersion = parsed?.documentVersion;
        if (typeof documentVersion === 'string') {
            lastPushedEngineRevisionRef.current = documentVersion;
        }
        pushRevisionRef.current += 1;
        setPushedUpdate({ json: updateJson, revision: pushRevisionRef.current });
    }, [applyUpdateState, bridge, documentHandle]);

    // After a JS-driven engine change (controlled apply, remote commit,
    // document-API mutation) the view learns the new state here. Native-
    // driven commits (typing, native toolbar) already updated the view
    // through the adapter and are never echoed back. The first observed
    // revision is skipped: the view pulls the initial state natively on bind.
    useEffect(() => {
        if (!document.isReady || document.documentRevision == null) return;
        const revision = document.documentRevision;
        if (!didObserveInitialRevisionRef.current) {
            didObserveInitialRevisionRef.current = true;
            lastPushedEngineRevisionRef.current = revision;
            return;
        }
        if (revision === lastPushedEngineRevisionRef.current) return;
        if (revision === lastNativeDrivenRevisionRef.current) {
            lastPushedEngineRevisionRef.current = revision;
            return;
        }
        pushEngineUpdateToView();
    }, [document.isReady, document.documentRevision, pushEngineUpdateToView]);

    // ── Engine mutation path (ref commands + toolbar requests) ──
    const afterLocalEngineMutation = useCallback(() => {
        onLocalDocumentCommitRef.current?.();
        document.refresh();
        pushEngineUpdateToView();
    }, [document, pushEngineUpdateToView]);

    const editableRef = useRef(editable);
    editableRef.current = editable;

    const runEngineMutation = useCallback(
        (invoke: (baseDocumentRevision: string) => unknown) => {
            if (!editableRef.current) {
                throw new NativeEditorV2OperationError({
                    domain: 'operation',
                    code: 'MUTATION_REJECTED',
                    message:
                        'NativeRichTextEditor: mutation rejected while editable is false',
                    requestId: null,
                    operationIndex: null,
                    limit: null,
                    actual: null,
                    details: null,
                });
            }
            const baseRevision = latestRevisionRef.current;
            if (baseRevision == null) {
                // Engine not ready (room awaiting the server document).
                return;
            }
            try {
                invoke(baseRevision);
            } catch (error) {
                if (isRevisionMismatchError(error)) {
                    // Refresh from the engine; NEVER retry against guessed
                    // positions (native adapter parity).
                    document.refresh();
                    return;
                }
                throw error;
            }
            afterLocalEngineMutation();
        },
        [afterLocalEngineMutation, document]
    );

    const applyEngineCommand = useCallback(
        (command: Record<string, unknown>) => {
            runEngineMutation((baseDocumentRevision) =>
                bridge.applyCommand({ command, baseDocumentRevision })
            );
        },
        [bridge, runEngineMutation]
    );

    const commandToggleMark = useCallback(
        (markType: string) => applyEngineCommand({ type: 'toggleMark', markType }),
        [applyEngineCommand]
    );
    const commandSetLink = useCallback(
        (href: string) => {
            const trimmedHref = href.trim();
            if (!trimmedHref) return;
            applyEngineCommand({
                type: 'setMark',
                markType: 'link',
                attrs: { href: trimmedHref },
            });
        },
        [applyEngineCommand]
    );
    const commandUnsetLink = useCallback(
        () => applyEngineCommand({ type: 'unsetMark', markType: 'link' }),
        [applyEngineCommand]
    );
    const commandToggleBlockquote = useCallback(
        () => applyEngineCommand({ type: 'toggleBlockquote' }),
        [applyEngineCommand]
    );
    const commandToggleHeading = useCallback(
        (level: EditorToolbarHeadingLevel) =>
            applyEngineCommand({ type: 'toggleHeading', level }),
        [applyEngineCommand]
    );
    const commandToggleList = useCallback(
        (listType: string) => {
            if (activeStateRef.current.nodes[listType] === true) {
                applyEngineCommand({ type: 'unwrapFromList' });
                return;
            }
            applyEngineCommand({
                type: 'wrapInList',
                listType,
                itemType: listType === 'taskList' ? 'taskItem' : 'listItem',
            });
        },
        [applyEngineCommand]
    );
    const commandIndentListItem = useCallback(
        () => applyEngineCommand({ type: 'indentListItem' }),
        [applyEngineCommand]
    );
    const commandOutdentListItem = useCallback(
        () => applyEngineCommand({ type: 'outdentListItem' }),
        [applyEngineCommand]
    );
    const commandInsertNode = useCallback(
        (nodeType: string) => applyEngineCommand({ type: 'insertNode', nodeType }),
        [applyEngineCommand]
    );
    const commandInsertImage = useCallback(
        (src: string, attrs?: Omit<ImageNodeAttributes, 'src'>) => {
            applyEngineCommand({
                type: 'insertContentJson',
                json: buildImageFragmentJson({ src, ...attrs }, documentDescriptor),
            });
        },
        [applyEngineCommand, documentDescriptor]
    );
    const commandInsertText = useCallback(
        (text: string) => {
            if (!text) return;
            runEngineMutation((baseDocumentRevision) =>
                bridge.applyInput({ text, baseDocumentRevision })
            );
        },
        [bridge, runEngineMutation]
    );
    const commandInsertContentHtml = useCallback(
        (html: string) => applyEngineCommand({ type: 'insertContentHtml', html }),
        [applyEngineCommand]
    );
    const commandInsertContentJson = useCallback(
        (doc: DocumentJSON) => applyEngineCommand({ type: 'insertContentJson', json: doc }),
        [applyEngineCommand]
    );

    // ── Link / image request flows ──────────────────────────────
    const openLinkRequest = useCallback(() => {
        const linkAttrs = activeStateRef.current.markAttrs?.link;
        onRequestLinkRef.current?.({
            href: typeof linkAttrs?.href === 'string' ? linkAttrs.href : undefined,
            isActive: activeStateRef.current.marks.link === true,
            selection: selectionRef.current,
            setLink: commandSetLink,
            unsetLink: commandUnsetLink,
        });
    }, [commandSetLink, commandUnsetLink]);

    const openImageRequest = useCallback(() => {
        onRequestImageRef.current?.({
            selection: selectionRef.current,
            insertImage: commandInsertImage,
        });
    }, [commandInsertImage]);

    // ── Ref surface ─────────────────────────────────────────────
    const nativeViewRef = useRef<NativeEditorViewHandle | null>(null);
    useImperativeHandle(
        ref,
        (): NativeRichTextEditorRef => ({
            focus() {
                nativeViewRef.current?.focus?.();
            },
            blur() {
                nativeViewRef.current?.blur?.();
            },
            toggleMark: commandToggleMark,
            setLink: commandSetLink,
            unsetLink: commandUnsetLink,
            toggleBlockquote: commandToggleBlockquote,
            toggleHeading: commandToggleHeading,
            toggleList: commandToggleList,
            indentListItem: commandIndentListItem,
            outdentListItem: commandOutdentListItem,
            insertNode: commandInsertNode,
            insertImage: commandInsertImage,
            insertText: commandInsertText,
            insertContentHtml: commandInsertContentHtml,
            insertContentJson: commandInsertContentJson,
            setContent: document.setContent,
            setContentJson: document.setContentJson,
            clearContent: document.clearContent,
            getContent: document.getContent,
            getContentJson: document.getContentJson,
            getTextContent: document.getTextContent,
            async getCaretRect(): Promise<NativeRichTextEditorCaretRect | null> {
                const nativeView = nativeViewRef.current;
                if (!nativeView?.getCaretRect) return null;
                const raw = await Promise.resolve(nativeView.getCaretRect());
                return parseCaretRectJson(raw);
            },
            undo: document.undo,
            redo: document.redo,
            canUndo: document.canUndo,
            canRedo: document.canRedo,
        }),
        [
            document,
            commandToggleMark,
            commandSetLink,
            commandUnsetLink,
            commandToggleBlockquote,
            commandToggleHeading,
            commandToggleList,
            commandIndentListItem,
            commandOutdentListItem,
            commandInsertNode,
            commandInsertImage,
            commandInsertText,
            commandInsertContentHtml,
            commandInsertContentJson,
        ]
    );

    // ── Native event handlers ───────────────────────────────────
    const isForThisEditor = useCallback(
        (payload: { editorId?: string }) => payload.editorId == null || payload.editorId === editorId,
        [editorId]
    );

    const handleEditorUpdate = useCallback(
        (event: NativeSyntheticEvent<NativeUpdateEvent>) => {
            if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
            const { updateJson, documentVersion } = event.nativeEvent;
            if (typeof documentVersion === 'string') {
                lastNativeDrivenRevisionRef.current = documentVersion;
            }
            applyUpdateState(updateJson);
            // The adapter already committed; re-read for content callbacks
            // and let collaboration flush the outbound frame.
            document.refresh();
            onLocalDocumentCommitRef.current?.();
        },
        [applyUpdateState, document, documentHandle, isForThisEditor]
    );

    const handleSelectionChange = useCallback(
        (event: NativeSyntheticEvent<NativeSelectionEvent>) => {
            if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
            const { anchor, head, stateJson } = event.nativeEvent;
            let selection: Selection = { type: 'text', anchor, head };
            const parsed = applyUpdateState(stateJson);
            const parsedSelection = parseSelectionFromUpdate(parsed?.selection);
            if (parsedSelection) {
                selection = parsedSelection;
            }
            selectionRef.current = selection;
            onSelectionChangeRef.current?.(selection);
        },
        [applyUpdateState, documentHandle, isForThisEditor]
    );

    const handleFocusChange = useCallback(
        (event: NativeSyntheticEvent<NativeFocusEvent>) => {
            if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
            const focused = event.nativeEvent.isFocused;
            const wasFocused = isFocusedRef.current;
            isFocusedRef.current = focused;
            if (focused && !wasFocused) {
                onFocusRef.current?.();
            } else if (!focused && wasFocused) {
                onBlurRef.current?.();
            }
        },
        [documentHandle, isForThisEditor]
    );

    const [autoGrowHeight, setAutoGrowHeight] = useState<number | null>(null);
    const handleContentHeightChange = useCallback(
        (event: NativeSyntheticEvent<NativeContentHeightEvent>) => {
            if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
            if (heightBehavior !== 'autoGrow') return;
            const density = Platform.OS === 'android' ? PixelRatio.get() : 1;
            const nextHeight = Math.ceil(event.nativeEvent.contentHeight / density);
            if (!(nextHeight > 0)) return;
            setAutoGrowHeight((prev) => (prev === nextHeight ? prev : nextHeight));
        },
        [documentHandle, heightBehavior, isForThisEditor]
    );

    const handleToolbarAction = useCallback(
        (event: NativeSyntheticEvent<NativeToolbarActionEvent>) => {
            if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
            const { key, updateJson, stateJson, documentVersion } = event.nativeEvent;
            if (typeof documentVersion === 'string') {
                lastNativeDrivenRevisionRef.current = documentVersion;
            }
            applyUpdateState(typeof updateJson === 'string' ? updateJson : stateJson);
            // The native toolbar already applied the engine command through
            // the adapter; resync the document binding and flush outbound.
            document.refresh();
            onLocalDocumentCommitRef.current?.();
            if (key === LINK_TOOLBAR_ACTION_KEY) {
                openLinkRequest();
                return;
            }
            if (key === IMAGE_TOOLBAR_ACTION_KEY) {
                openImageRequest();
                return;
            }
            onToolbarActionRef.current?.(key);
        },
        [applyUpdateState, document, documentHandle, isForThisEditor, openImageRequest, openLinkRequest]
    );

    const handleAddonEvent = useCallback(
        (_event: NativeSyntheticEvent<NativeAddonEvent>) => {
            // Addon (mention) queries are served natively in v2; there is no
            // JS suggestion feed on the document-handle surface.
        },
        []
    );

    // ── Serialized view props ───────────────────────────────────
    const themeJson = useSerializedValue(theme, serializeEditorTheme);
    const addonsJson = useSerializedValue(addons, (value) => serializeEditorAddons(value));
    const imageLoadingPolicyJson = useSerializedValue(imageLoadingPolicy, (value) =>
        serializeEditorImageLoadingPolicy(value)
    );
    const remoteSelectionsJson = useSerializedValue(remoteSelections, (selections) =>
        serializeRemoteSelections(selections)
    );

    const toolbarItemsSerializationCacheRef = useRef<{
        toolbarItems: readonly EditorToolbarItem[];
        editable: boolean;
        isLinkActive: boolean;
        allowsLink: boolean;
        canRequestLink: boolean;
        canRequestImage: boolean;
        canInsertImage: boolean;
        serialized: string;
    } | null>(null);
    const isLinkActive = activeState.marks.link === true;
    const allowsLink = activeState.allowedMarks.includes('link');
    const canInsertImage = activeState.insertableNodes.includes(IMAGE_NODE_NAME);
    const canRequestLink = typeof onRequestLink === 'function';
    const canRequestImage = typeof onRequestImage === 'function';
    const cachedToolbarItems = toolbarItemsSerializationCacheRef.current;
    let toolbarItemsJson: string;
    if (
        cachedToolbarItems &&
        cachedToolbarItems.toolbarItems === toolbarItems &&
        cachedToolbarItems.editable === editable &&
        cachedToolbarItems.isLinkActive === isLinkActive &&
        cachedToolbarItems.allowsLink === allowsLink &&
        cachedToolbarItems.canRequestLink === canRequestLink &&
        cachedToolbarItems.canRequestImage === canRequestImage &&
        cachedToolbarItems.canInsertImage === canInsertImage
    ) {
        toolbarItemsJson = cachedToolbarItems.serialized;
    } else {
        const mappedItems = mapToolbarItemsForNative(
            toolbarItems,
            activeState,
            editable,
            onRequestLink,
            onRequestImage
        );
        toolbarItemsJson = stringifyCachedJson(mappedItems);
        toolbarItemsSerializationCacheRef.current = {
            toolbarItems,
            editable,
            isLinkActive,
            allowsLink,
            canRequestLink,
            canRequestImage,
            canInsertImage,
            serialized: toolbarItemsJson,
        };
    }

    // A room document awaiting the server renders nothing (loading), never an
    // unshared fallback paragraph.
    if (!document.isReady) return null;

    const usesNativeKeyboardToolbar =
        toolbarPlacement === 'keyboard' && (Platform.OS === 'ios' || Platform.OS === 'android');
    const shouldRenderJsToolbar = showToolbar && !usesNativeKeyboardToolbar && editable;
    const inlineToolbarMarginTop = theme?.toolbar?.marginTop ?? 8;
    const containerMinHeight = StyleSheet.flatten(containerStyle)?.minHeight;
    const nativeViewStyleParts: StyleProp<ViewStyle>[] = [];
    if (containerMinHeight != null) {
        nativeViewStyleParts.push({ minHeight: containerMinHeight });
    }
    if (style != null) {
        nativeViewStyleParts.push(style);
    }
    if (heightBehavior === 'autoGrow' && autoGrowHeight != null) {
        nativeViewStyleParts.push({ height: autoGrowHeight });
    }
    const nativeViewStyle =
        nativeViewStyleParts.length <= 1 ? nativeViewStyleParts[0] : nativeViewStyleParts;

    return (
        <View style={[styles.container, containerStyle]}>
            <NativeEditorView
                ref={nativeViewRef}
                style={nativeViewStyle}
                accessibilityLabel={accessibilityLabel}
                accessibilityHint={accessibilityHint}
                editorId={editorId}
                placeholder={placeholder}
                editable={editable}
                autoFocus={autoFocus}
                autoCapitalize={autoCapitalize}
                autoCorrect={autoCorrect}
                keyboardType={keyboardType}
                showToolbar={showToolbar}
                toolbarPlacement={toolbarPlacement}
                heightBehavior={heightBehavior}
                allowImageResizing={allowImageResizing}
                imageLoadingPolicyJson={imageLoadingPolicyJson}
                themeJson={themeJson}
                addonsJson={addonsJson}
                toolbarItemsJson={toolbarItemsJson}
                remoteSelectionsJson={remoteSelectionsJson}
                editorUpdateJson={pushedUpdate?.json}
                editorUpdateEditorId={pushedUpdate != null ? editorId : undefined}
                editorUpdateRevision={pushedUpdate?.revision ?? 0}
                onEditorUpdate={handleEditorUpdate}
                onSelectionChange={handleSelectionChange}
                onFocusChange={handleFocusChange}
                onContentHeightChange={handleContentHeightChange}
                onToolbarAction={handleToolbarAction}
                onAddonEvent={handleAddonEvent}
            />
            {shouldRenderJsToolbar ? (
                <View
                    testID='native-editor-js-toolbar'
                    style={[styles.inlineToolbar, { marginTop: inlineToolbarMarginTop }]}>
                    <EditorToolbar
                        activeState={activeState}
                        historyState={document.historyState}
                        toolbarItems={toolbarItems}
                        theme={theme?.toolbar}
                        showTopBorder={theme?.toolbar?.showTopBorder ?? false}
                        preserveEditorFocus={false}
                        onToggleMark={commandToggleMark}
                        onToggleListType={(listType: EditorToolbarListType) =>
                            commandToggleList(listType)
                        }
                        onToggleHeading={commandToggleHeading}
                        onToggleBlockquote={commandToggleBlockquote}
                        onInsertNodeType={commandInsertNode}
                        onRunCommand={(command: EditorToolbarCommand) => {
                            switch (command) {
                                case 'indentList':
                                    commandIndentListItem();
                                    break;
                                case 'outdentList':
                                    commandOutdentListItem();
                                    break;
                                case 'undo':
                                    document.undo();
                                    break;
                                case 'redo':
                                    document.redo();
                                    break;
                            }
                        }}
                        onRequestLink={onRequestLink ? openLinkRequest : undefined}
                        onRequestImage={onRequestImage ? openImageRequest : undefined}
                        onToolbarAction={onToolbarAction}
                        onToggleBold={() => commandToggleMark('bold')}
                        onToggleItalic={() => commandToggleMark('italic')}
                        onToggleUnderline={() => commandToggleMark('underline')}
                        onToggleStrike={() => commandToggleMark('strike')}
                        onUndo={document.undo}
                        onRedo={document.redo}
                    />
                </View>
            ) : null}
        </View>
    );
});

const styles = StyleSheet.create({
    container: {
        position: 'relative',
    },
    inlineToolbar: {
        flexDirection: 'row',
        alignItems: 'center',
    },
});
