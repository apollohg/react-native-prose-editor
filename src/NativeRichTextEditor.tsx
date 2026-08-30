import React, {
    forwardRef,
    useCallback,
    useEffect,
    useImperativeHandle,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
    type ReactNode,
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
    normalizeNativeEditorV2RenderUpdateValue,
    requireNativeEditorV2U32,
    validEditorMentionTheme,
    type ActiveState,
    type DocumentJSON,
    type HistoryState,
    type NativeEditorDocumentHandle,
    type NativeEditorV2AtomicRenderSnapshot,
    type NativeEditorV2PositionAffinity,
    type ReadonlyActiveState,
    type RenderElement,
    type Selection,
} from './NativeEditorBridge';
import { NativeEditorV2ErrorBase, NativeEditorV2OperationError } from './NativeEditorBoundaryError';
import { allocateEditorUpdateRevision } from './EditorUpdateRevision';
import {
    ExternalTextCompositionManager,
    createExternalCompositionLifecycleError,
    type ExternalTextCompositionOptions,
    type ExternalTextCompositionSession,
    type NativeExternalTextCompositionHandle,
} from './ExternalTextComposition';
import { useNativeEditorDocument } from './useNativeEditor';
import {
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    EditorToolbar,
    EditorToolbarFrameOwnerProvider,
    setActiveEditorToolbarFrameOwnerForEditor,
    setEditorToolbarMentionState,
    useEditorToolbarFrames,
    type EditorToolbarCommand,
    type EditorToolbarFrame,
    type EditorToolbarGroupChildItem,
    type EditorToolbarHeadingLevel,
    type EditorToolbarIcon,
    type EditorToolbarItem,
    type EditorToolbarListType,
} from './EditorToolbar';
import { serializeEditorTheme, type EditorMentionTheme, type EditorTheme } from './EditorTheme';
import {
    serializeEditorImageLoadingPolicy,
    type EditorImageLoadingPolicy,
} from './ImageLoadingPolicy';
import {
    buildMentionFragmentJson,
    normalizeEditorAddons,
    serializeEditorAddons,
    type EditorAddonEvent,
    type EditorAddons,
    type MentionQueryChangeEvent,
    type MentionSelectionAttrsEvent,
    type MentionSuggestion,
} from './addons';
import {
    buildImageFragmentJson,
    IMAGE_NODE_NAME,
    normalizeDocumentJson,
    type ImageNodeAttributes,
} from './schemas';
import {
    useFocusPreservingFrames,
    type NativeRichTextEditorFocusPreservingRefs,
} from './useFocusPreservingFrames';
import { DefaultAtomChip } from './DefaultAtomChip';
import { serializeEditorAtoms, type AtomComponent, type AtomNodeDefinition } from './atoms';
import {
    AtomUpdateAttrsError,
    DEFAULT_ATOM_CHIP_HEIGHT,
    applyRenderPatch,
    atomSelected,
    collectAtomInstances,
    type AtomInstance,
} from './atomInstances';

export type {
    NativeRichTextEditorFocusPreservingElement,
    NativeRichTextEditorFocusPreservingRef,
    NativeRichTextEditorFocusPreservingRefs,
} from './useFocusPreservingFrames';

interface NativeExternalTextCompositionEvent {
    editorId: string;
    resultJson: string;
}

interface NativeEditorViewHandle extends NativeExternalTextCompositionHandle {
    focus?: () => void;
    blur?: () => void;
    getCaretRect?: () => Promise<string | null> | string | null;
}

interface NativeEditorViewProps {
    children?: ReactNode;
    style?: StyleProp<ViewStyle>;
    onLayout?: () => void;
    accessibilityLabel?: string;
    accessibilityHint?: string;
    editorId: string;
    placeholder?: string;
    editable: boolean;
    autoFocus: boolean;
    autoCapitalize?: NativeRichTextEditorAutoCapitalize;
    autoCorrect?: boolean;
    keyboardType?: NativeRichTextEditorKeyboardType;
    androidInputOptionsJson?: string;
    showToolbar: boolean;
    toolbarPlacement: NativeRichTextEditorToolbarPlacement;
    heightBehavior: NativeRichTextEditorHeightBehavior;
    allowImageResizing: boolean;
    imageLoadingPolicyJson?: string;
    themeJson?: string;
    addonsJson?: string;
    atomsJson?: string;
    toolbarItemsJson?: string;
    toolbarFrameJson?: string;
    remoteSelectionsJson?: string;
    editorUpdateJson?: string;
    editorUpdateEditorId?: string;
    editorUpdateRevision?: number;
    onEditorUpdate: (event: NativeSyntheticEvent<NativeUpdateEvent>) => void;
    onEditorError: (event: NativeSyntheticEvent<NativeErrorEvent>) => void;
    onExternalTextCompositionEnd: (
        event: NativeSyntheticEvent<NativeExternalTextCompositionEvent>
    ) => void;
    onSelectionChange: (event: NativeSyntheticEvent<NativeSelectionEvent>) => void;
    onFocusChange: (event: NativeSyntheticEvent<NativeFocusEvent>) => void;
    onContentHeightChange: (event: NativeSyntheticEvent<NativeContentHeightEvent>) => void;
    onAtomLayout: (event: NativeSyntheticEvent<NativeAtomLayoutEvent>) => void;
    onToolbarAction: (event: NativeSyntheticEvent<NativeToolbarActionEvent>) => void;
    onAddonEvent: (event: NativeSyntheticEvent<NativeAddonEvent>) => void;
}

const NativeEditorView = requireNativeViewManager('NativeEditor') as React.ComponentType<
    NativeEditorViewProps & React.RefAttributes<NativeEditorViewHandle>
>;

interface NativeUpdateEvent {
    updateJson: string;
    editorId: string;
    documentRevision: string;
}

interface NativeErrorEvent {
    editorId: string;
    error: unknown;
}

interface NativeSelectionEvent {
    anchor: number;
    head: number;
    stateJson?: string;
    editorId: string;
    documentVersion?: string;
}

interface NativeFocusEvent {
    isFocused: boolean;
    editorId: string;
}

interface NativeContentHeightEvent {
    contentHeight: number;
    editorId: string;
}

interface NativeAtomLayoutEvent {
    width: number;
    editorId: string;
}

interface NativeToolbarActionEvent {
    key: string;
    editorId: string;
    updateJson?: string;
    stateJson?: string;
    documentRevision?: string;
}

interface NativeAddonEvent {
    eventJson: string;
    editorId: string;
}

interface NativeEditorErrorBinding {
    readonly handle: NativeEditorDocumentHandle;
    readonly editorId: string;
    readonly generation: number;
    readonly mounted: boolean;
}

interface ControlledValueDelivery {
    manager: ExternalTextCompositionManager;
    key: string | null;
    value: string | undefined;
    valueJSON: DocumentJSON | undefined;
}

interface ExternalCompositionDisposalToken {
    cancelled: boolean;
}

function externalCompositionErrorPayload(error: unknown): unknown {
    return error instanceof NativeEditorV2ErrorBase ? error.error : error;
}

const LINK_TOOLBAR_ACTION_KEY = '__native-editor-link__';
const IMAGE_TOOLBAR_ACTION_KEY = '__native-editor-image__';
let nextNativeEditorToolbarFrameOwnerId = 1;

function mergeMentionSuggestionTheme(
    baseTheme: EditorMentionTheme | undefined,
    resolvedTheme: EditorMentionTheme | undefined
): EditorMentionTheme | undefined {
    if (baseTheme == null) return resolvedTheme;
    if (resolvedTheme == null) return baseTheme;

    return {
        node: { ...baseTheme.node, ...resolvedTheme.node },
        suggestions: {
            ...baseTheme.suggestions,
            ...resolvedTheme.suggestions,
            option: { ...baseTheme.suggestions?.option, ...resolvedTheme.suggestions?.option },
        },
    };
}

function allocateToolbarFrameOwnerId(): number {
    const ownerId = nextNativeEditorToolbarFrameOwnerId;
    nextNativeEditorToolbarFrameOwnerId += 1;
    return ownerId;
}

const EMPTY_ACTIVE_STATE: ActiveState = {
    marks: {},
    markAttrs: {},
    nodes: {},
    commands: {},
    allowedMarks: [],
    insertableNodes: [],
};

interface AtomRenderState {
    blocks: RenderElement[][];
    instances: AtomInstance[];
}

function copyRenderBlocks(
    blocks:
        | NonNullable<NativeEditorV2AtomicRenderSnapshot['renderBlocks']>
        | ReadonlyArray<ReadonlyArray<RenderElement>>
): RenderElement[][] {
    return blocks.map((block) =>
        block.map((element) => {
            const copy = { ...element } as RenderElement;
            if (element.marks != null) {
                copy.marks = element.marks.map((mark) =>
                    typeof mark === 'string' ? mark : { ...mark }
                );
            }
            if (element.attrs != null) copy.attrs = { ...element.attrs };
            return copy;
        })
    );
}

function selectedAtomKeys(selection: Selection, instances: readonly AtomInstance[]): Set<string> {
    return new Set(
        instances
            .filter((instance) => atomSelected(selection, instance.docPos))
            .map(({ key }) => key)
    );
}

function equalStringSets(left: ReadonlySet<string>, right: ReadonlySet<string>): boolean {
    return left.size === right.size && [...left].every((value) => right.has(value));
}

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
    return Array.isArray(value)
        ? value.filter((item): item is string => typeof item === 'string')
        : [];
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
    return error instanceof NativeEditorV2OperationError && error.code === 'REVISION_MISMATCH';
}

function isPositionInvalidError(error: unknown): boolean {
    return error instanceof NativeEditorV2OperationError && error.code === 'POSITION_INVALID';
}

interface NativeCommitPayload {
    editorId: string;
    documentRevision: string;
    updateJson: string;
}

interface AcceptedNativeCommit {
    documentRevision: string;
    snapshot: NativeEditorV2AtomicRenderSnapshot;
}

/**
 * Native commits are a transport boundary, not a best-effort view hint. The
 * payload must be a complete atomic snapshot paired with the same canonical
 * revision that native says it committed.
 */
function acceptNativeCommitPayload(
    payload: NativeCommitPayload,
    boundEditorId: string,
    lastAcceptedRevision: string | null
): AcceptedNativeCommit | null {
    const canonicalBoundEditorId = normalizeNativeEditorV2DecimalId(boundEditorId);
    const canonicalEditorId = normalizeNativeEditorV2DecimalId(payload.editorId);
    const canonicalRevision = normalizeNativeEditorV2DecimalId(payload.documentRevision);
    if (
        canonicalBoundEditorId == null ||
        canonicalBoundEditorId !== boundEditorId ||
        canonicalEditorId == null ||
        canonicalEditorId !== payload.editorId ||
        canonicalEditorId !== boundEditorId ||
        canonicalRevision == null ||
        canonicalRevision !== payload.documentRevision
    ) {
        return null;
    }
    const snapshot = normalizeNativeEditorV2RenderUpdateValue(payload.updateJson);
    if (snapshot == null || snapshot.documentVersion !== canonicalRevision) return null;
    if (lastAcceptedRevision != null && BigInt(canonicalRevision) <= BigInt(lastAcceptedRevision)) {
        return null;
    }
    return { documentRevision: canonicalRevision, snapshot };
}

function mapToolbarChildForNative(
    item: EditorToolbarGroupChildItem,
    activeState: ReadonlyActiveState,
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
            buttonStyle: item.buttonStyle,
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
            buttonStyle: item.buttonStyle,
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
    activeState: ReadonlyActiveState,
    editable: boolean,
    onRequestLink?: NativeRichTextEditorProps['onRequestLink'],
    onRequestImage?: NativeRichTextEditorProps['onRequestImage']
): EditorToolbarItem[] {
    return items.map((item) => {
        if (item.type === 'group') {
            return {
                ...item,
                items: item.items.map((child) =>
                    mapToolbarChildForNative(
                        child,
                        activeState,
                        editable,
                        onRequestLink,
                        onRequestImage
                    )
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

function serializeToolbarFrames(
    frames: readonly EditorToolbarFrame[] | null | undefined
): string | undefined {
    if (!frames || frames.length === 0) {
        return undefined;
    }
    return JSON.stringify(frames.length === 1 ? frames[0] : { frames });
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

/**
 * How the editor handles content taller than its frame: `'fixed'` scrolls
 * internally, `'autoGrow'` grows the view to fit.
 */
export type NativeRichTextEditorHeightBehavior = 'fixed' | 'autoGrow';
/**
 * Where the toolbar lives: `'keyboard'` attaches the native toolbar to the
 * keyboard, `'inline'` renders the JavaScript `EditorToolbar` below the editor.
 */
export type NativeRichTextEditorToolbarPlacement = 'keyboard' | 'inline';
/**
 * What an external `valueJSON` change does to undo history: `'replace'`
 * records one undoable step, `'reset'` clears history entirely.
 */
export type NativeRichTextEditorValueJSONUpdateMode = 'replace' | 'reset';
/** Native keyboard auto-capitalization behavior. */
export type NativeRichTextEditorAutoCapitalize = 'none' | 'sentences' | 'words' | 'characters';
/** Native keyboard layout. Values not supported by a platform fall back to its default. */
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

/** Android-specific options passed to the active input method. */
export interface NativeRichTextEditorAndroidInputOptions {
    /** Private options interpreted by the selected Android IME. */
    privateImeOptions?: string;
}

/**
 * One remote collaborator's caret or selection, drawn as a native overlay.
 * `useYjsCollaboration` builds these from awareness and passes them through
 * `editorBindings`.
 */
export interface RemoteSelectionDecoration {
    /** Peer identity. Also the overlay's React-style key. */
    clientId: string;
    /** Fixed end of the remote selection, in engine doc positions. */
    anchor: number;
    /** Moving end. Equals `anchor` for a collapsed remote caret. */
    head: number;
    /** Caret and label color, as a color string. */
    color: string;
    /** Name shown on the caret label. */
    name?: string;
    avatarUrl?: string;
    /** Whether that peer's editor holds focus. The caret bar is drawn only when
     *  true; the highlighted range is drawn either way. Absent counts as false. */
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
    /** Android-specific input-method options. Ignored on other platforms. */
    androidInputOptions?: NativeRichTextEditorAndroidInputOptions;
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
    onActiveStateChange?: (state: ReadonlyActiveState) => void;
    /** Called when undo/redo availability changes. */
    onHistoryStateChange?: (state: HistoryState) => void;
    /** Called when the editor gains focus. */
    onFocus?: () => void;
    /** Called when the editor loses focus. */
    onBlur?: () => void;
    /** External native views whose taps preserve this editor's focus, keyboard, and selection. */
    focusPreservingRefs?: NativeRichTextEditorFocusPreservingRefs;
    /** Style applied to the native editor view. */
    style?: StyleProp<ViewStyle>;
    /** Style applied to the outer React container wrapping the editor and inline toolbar. */
    containerStyle?: StyleProp<ViewStyle>;
    /** Optional native content theme applied to rendered blocks and typing attrs. */
    theme?: EditorTheme;
    /** Optional addon configuration. */
    addons?: EditorAddons;
    /** Custom void-block node definitions mounted as React children. */
    atoms?: readonly AtomNodeDefinition[];
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
    /** Application notification after a successful local mutation. Native transport wakes independently. */
    onLocalCommit?: () => void;
}

export interface NativeRichTextEditorRef {
    /** Programmatically focus the editor. */
    focus(): void;
    /** Programmatically blur the editor. */
    blur(): void;
    /** Check whether the mounted native editor supports external text composition. */
    supportsExternalTextComposition(): boolean;
    /** Begin an external text composition session. */
    beginExternalTextComposition(
        options?: ExternalTextCompositionOptions
    ): Promise<ExternalTextCompositionSession>;
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
    /** Ask the Rust editor core whether the current document is empty. */
    getIsEmpty(): boolean;
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
export const NativeRichTextEditor = forwardRef<NativeRichTextEditorRef, NativeRichTextEditorProps>(
    function NativeRichTextEditor(
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
            androidInputOptions,
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
            atoms,
            remoteSelections,
            allowImageResizing = true,
            onContentChange,
            onContentChangeJSON,
            onSelectionChange,
            onActiveStateChange,
            onHistoryStateChange,
            onFocus,
            onBlur,
            focusPreservingRefs,
            onLocalCommit,
        },
        ref
    ) {
        _assertNativeEditorDocumentHandle(documentHandle);
        const documentDescriptor = _getNativeEditorDocumentHandleDescriptor(documentHandle);
        const registeredAtomTypes = useMemo(
            () => new Set((atoms ?? []).map((atom) => atom.name)),
            [atoms]
        );
        const atomComponents = useMemo(() => {
            const components = new Map<string, AtomComponent>();
            for (const atom of atoms ?? []) {
                if (!components.has(atom.name)) components.set(atom.name, atom.component);
            }
            return components;
        }, [atoms]);

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

        const bridge = documentHandle.bridge;
        const editorId = documentHandle.editorId;
        const nativeViewRef = useRef<NativeEditorViewHandle | null>(null);
        const externalCompositionManager = useMemo(
            () => new ExternalTextCompositionManager(editorId, () => nativeViewRef.current),
            [editorId]
        );
        const managerDisposalsRef = useRef(
            new Map<ExternalTextCompositionManager, ExternalCompositionDisposalToken>()
        );
        useEffect(() => {
            const pendingDisposal = managerDisposalsRef.current.get(externalCompositionManager);
            if (pendingDisposal != null) {
                pendingDisposal.cancelled = true;
                managerDisposalsRef.current.delete(externalCompositionManager);
            }
            return () => {
                const token: ExternalCompositionDisposalToken = { cancelled: false };
                managerDisposalsRef.current.set(externalCompositionManager, token);
                void Promise.resolve().then(() => {
                    if (
                        token.cancelled ||
                        managerDisposalsRef.current.get(externalCompositionManager) !== token
                    ) {
                        return;
                    }
                    managerDisposalsRef.current.delete(externalCompositionManager);
                    externalCompositionManager.dispose();
                });
            };
        }, [externalCompositionManager]);

        const controlledValueKey =
            value != null
                ? `html:${value}`
                : serializedValueJson == null
                  ? null
                  : `json:${serializedValueJson}`;
        const currentControlledValue: ControlledValueDelivery = {
            manager: externalCompositionManager,
            key: controlledValueKey,
            value,
            valueJSON: controlledValueJSON,
        };
        const deliveredControlledValueRef = useRef<ControlledValueDelivery>(currentControlledValue);
        if (
            deliveredControlledValueRef.current.manager !== externalCompositionManager ||
            valueJSONUpdateMode !== 'reset' ||
            controlledValueKey == null ||
            deliveredControlledValueRef.current.key === controlledValueKey
        ) {
            deliveredControlledValueRef.current = currentControlledValue;
        }
        const latestControlledValueRef = useRef({
            ...currentControlledValue,
            mode: valueJSONUpdateMode,
            handle: documentHandle,
        });
        latestControlledValueRef.current = {
            ...currentControlledValue,
            mode: valueJSONUpdateMode,
            handle: documentHandle,
        };
        const pendingResetCancellationRef = useRef<{
            manager: ExternalTextCompositionManager;
        } | null>(null);
        const blockedControlledResetRef = useRef<{
            manager: ExternalTextCompositionManager;
            key: string;
        } | null>(null);
        const [, setControlledResetRevision] = useState(0);
        useLayoutEffect(() => {
            if (
                valueJSONUpdateMode !== 'reset' ||
                controlledValueKey == null ||
                deliveredControlledValueRef.current.manager !== externalCompositionManager ||
                deliveredControlledValueRef.current.key === controlledValueKey ||
                pendingResetCancellationRef.current?.manager === externalCompositionManager ||
                (blockedControlledResetRef.current?.manager === externalCompositionManager &&
                    blockedControlledResetRef.current.key === controlledValueKey)
            ) {
                return;
            }
            const pending = { manager: externalCompositionManager };
            pendingResetCancellationRef.current = pending;
            void externalCompositionManager.cancelForDocumentChange().then(
                () => {
                    if (pendingResetCancellationRef.current !== pending) return;
                    pendingResetCancellationRef.current = null;
                    const latest = latestControlledValueRef.current;
                    if (
                        latest.manager !== externalCompositionManager ||
                        latest.mode !== 'reset' ||
                        latest.key == null ||
                        deliveredControlledValueRef.current.key === latest.key ||
                        latest.handle.isDestroyed ||
                        managerDisposalsRef.current.has(externalCompositionManager)
                    ) {
                        return;
                    }
                    blockedControlledResetRef.current = null;
                    deliveredControlledValueRef.current = {
                        manager: latest.manager,
                        key: latest.key,
                        value: latest.value,
                        valueJSON: latest.valueJSON,
                    };
                    setControlledResetRevision((revision) => revision + 1);
                },
                (error: unknown) => {
                    if (pendingResetCancellationRef.current !== pending) return;
                    pendingResetCancellationRef.current = null;
                    const latest = latestControlledValueRef.current;
                    if (
                        latest.manager !== externalCompositionManager ||
                        latest.mode !== 'reset' ||
                        latest.key == null ||
                        deliveredControlledValueRef.current.key === latest.key ||
                        latest.handle.isDestroyed ||
                        managerDisposalsRef.current.has(externalCompositionManager)
                    ) {
                        return;
                    }
                    blockedControlledResetRef.current = {
                        manager: externalCompositionManager,
                        key: latest.key,
                    };
                    latest.handle.bridge._emitAutonomousError(
                        externalCompositionErrorPayload(error)
                    );
                }
            );
        }, [controlledValueKey, externalCompositionManager, valueJSONUpdateMode]);

        const deliveredControlledValue = deliveredControlledValueRef.current;

        const document = useNativeEditorDocument({
            handle: documentHandle,
            value: deliveredControlledValue.value,
            valueJSON: deliveredControlledValue.valueJSON,
            valueJSONUpdateMode,
            revisionSignal: documentRevision ?? null,
            onContentChange,
            onContentChangeJSON,
            onHistoryStateChange,
            onLocalCommit,
        });

        const nativeErrorBindingRef = useRef<NativeEditorErrorBinding>({
            handle: documentHandle,
            editorId,
            generation: 0,
            mounted: true,
        });
        if (nativeErrorBindingRef.current.handle !== documentHandle) {
            nativeErrorBindingRef.current = {
                handle: documentHandle,
                editorId,
                generation: nativeErrorBindingRef.current.generation + 1,
                mounted: true,
            };
        }
        const nativeErrorBinding = nativeErrorBindingRef.current;
        useEffect(() => {
            const currentBinding = nativeErrorBindingRef.current;
            if (
                currentBinding !== nativeErrorBinding &&
                !currentBinding.mounted &&
                currentBinding.handle === nativeErrorBinding.handle &&
                currentBinding.editorId === nativeErrorBinding.editorId &&
                currentBinding.generation === nativeErrorBinding.generation + 1
            ) {
                nativeErrorBindingRef.current = nativeErrorBinding;
            }
            return () => {
                if (nativeErrorBindingRef.current !== nativeErrorBinding) return;
                nativeErrorBindingRef.current = {
                    ...nativeErrorBinding,
                    generation: nativeErrorBinding.generation + 1,
                    mounted: false,
                };
            };
        }, [nativeErrorBinding]);

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
        const onLocalCommitRef = useRef(onLocalCommit);
        onLocalCommitRef.current = onLocalCommit;
        const addonsRef = useRef(addons);
        addonsRef.current = addons;
        const [activeState, setActiveState] = useState<ReadonlyActiveState>(EMPTY_ACTIVE_STATE);
        const [pushedUpdate, setPushedUpdate] = useState<{
            json: string;
            revision: number;
            editorId: string;
        } | null>(null);
        const [autoGrowHeight, setAutoGrowHeight] = useState<number | null>(null);
        const [isFocused, setIsFocused] = useState(false);
        const [mentionQuery, setMentionQuery] = useState<MentionQueryChangeEvent | null>(null);
        const [atomState, setAtomState] = useState<AtomRenderState>({
            blocks: [],
            instances: [],
        });
        const atomStateRef = useRef(atomState);
        const [selectedKeys, setSelectedKeys] = useState<ReadonlySet<string>>(new Set());
        const [atomContentWidth, setAtomContentWidth] = useState<number | null>(null);
        const warnedUnknownAtomTypesRef = useRef(new Set<string>());
        const atomSeedEditorIdRef = useRef<string | null>(null);
        const activeStateRef = useRef<ReadonlyActiveState>(EMPTY_ACTIVE_STATE);
        const activeStateKeyRef = useRef<string | null>(null);
        const selectionRef = useRef<Selection>({ type: 'text', anchor: 0, head: 0 });
        const scalarSelectionRef = useRef({ anchor: 0, head: 0 });
        const isFocusedRef = useRef(false);
        const toolbarFrameOwnerIdRef = useRef<number | null>(null);
        if (toolbarFrameOwnerIdRef.current == null) {
            toolbarFrameOwnerIdRef.current = allocateToolbarFrameOwnerId();
        }
        const toolbarFrameOwnerId = toolbarFrameOwnerIdRef.current;
        const registeredToolbarFrames = useEditorToolbarFrames(toolbarFrameOwnerId);
        const { frames: suppliedFocusPreservingFrames, refresh: refreshFocusPreservingFrames } =
            useFocusPreservingFrames(focusPreservingRefs, editable && isFocused);
        const latestRevisionRef = useRef<string | null>(null);
        const currentPushedUpdateEditorIdRef = useRef(editorId);
        currentPushedUpdateEditorIdRef.current = editorId;
        const pushedUpdateBindingGenerationRef = useRef(0);
        const pushRevisionRef = useRef(0);
        const lastPushedEngineRevisionRef = useRef<string | null>(null);
        const lastNativeDrivenRevisionRef = useRef<string | null>(null);
        const lastAcceptedNativeCommitRevisionRef = useRef<string | null>(null);
        const didObserveInitialRevisionRef = useRef(false);
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
        const revisionScopeEditorIdRef = useRef(editorId);
        const latestRevisionScopeEditorIdRef = useRef<string | null>(editorId);
        const pendingFocusedRebindRef = useRef<{
            editorId: string;
            anchor: number;
            head: number;
        } | null>(null);
        const didRebindRevisionScope = revisionScopeEditorIdRef.current !== editorId;
        if (didRebindRevisionScope) {
            pendingFocusedRebindRef.current = null;
            if (isFocusedRef.current && document.isReady) {
                pendingFocusedRebindRef.current = {
                    editorId,
                    ...scalarSelectionRef.current,
                };
            } else if (isFocusedRef.current) {
                isFocusedRef.current = false;
                setIsFocused(false);
            }
            pushedUpdateBindingGenerationRef.current += 1;
            revisionScopeEditorIdRef.current = editorId;
            latestRevisionScopeEditorIdRef.current = null;
            latestRevisionRef.current = null;
            selectionRef.current = { type: 'text', anchor: 0, head: 0 };
            activeStateRef.current = EMPTY_ACTIVE_STATE;
            activeStateKeyRef.current = null;
            toolbarItemsSerializationCacheRef.current = null;
            setActiveState(EMPTY_ACTIVE_STATE);
            setPushedUpdate(null);
            setAutoGrowHeight(null);
            const emptyAtomState: AtomRenderState = { blocks: [], instances: [] };
            atomStateRef.current = emptyAtomState;
            atomSeedEditorIdRef.current = null;
            setAtomState(emptyAtomState);
            setSelectedKeys(new Set());
            setAtomContentWidth(null);
            pushRevisionRef.current = 0;
            lastPushedEngineRevisionRef.current = null;
            lastNativeDrivenRevisionRef.current = null;
            lastAcceptedNativeCommitRevisionRef.current = null;
            didObserveInitialRevisionRef.current = false;
        } else if (latestRevisionScopeEditorIdRef.current === editorId) {
            latestRevisionRef.current = document.documentRevision;
        }

        useEffect(() => {
            if (heightBehavior !== 'autoGrow') {
                setAutoGrowHeight(null);
            }
        }, [heightBehavior]);

        // A changed handle initially shares the previous hook render. Do not
        // trust that render's revision: establish the new mutation base only by
        // reading the currently bound handle after the rebind commits.
        useEffect(() => {
            if (latestRevisionScopeEditorIdRef.current === editorId || documentHandle.isDestroyed) {
                return;
            }
            latestRevisionRef.current = documentHandle.bridge.getState().documentRevision;
            latestRevisionScopeEditorIdRef.current = editorId;
        }, [documentHandle, editorId]);

        useEffect(
            () => () => {
                setActiveEditorToolbarFrameOwnerForEditor(toolbarFrameOwnerId, false);
                setEditorToolbarMentionState(toolbarFrameOwnerId, null);
            },
            [editorId, toolbarFrameOwnerId]
        );

        const updateAtomSelection = useCallback(
            (selection: Selection, instances = atomStateRef.current.instances) => {
                const next = selectedAtomKeys(selection, instances);
                setSelectedKeys((current) => (equalStringSets(current, next) ? current : next));
            },
            []
        );

        const refreshAtomsFromUpdate = useCallback(
            (update: NativeEditorV2AtomicRenderSnapshot) => {
                const previous = atomStateRef.current;
                const blocks =
                    update.renderBlocks != null
                        ? copyRenderBlocks(update.renderBlocks)
                        : applyRenderPatch(previous.blocks, {
                              startIndex: update.renderPatch.startIndex,
                              deleteCount: update.renderPatch.deleteCount,
                              renderBlocks: copyRenderBlocks(update.renderPatch.renderBlocks),
                          });
                const next = {
                    blocks,
                    instances: collectAtomInstances(blocks, registeredAtomTypes),
                };
                atomStateRef.current = next;
                setAtomState(next);
                updateAtomSelection(selectionRef.current, next.instances);
            },
            [registeredAtomTypes, updateAtomSelection]
        );

        useEffect(() => {
            const current = atomStateRef.current;
            const instances = collectAtomInstances(current.blocks, registeredAtomTypes);
            const next = { blocks: current.blocks, instances };
            atomStateRef.current = next;
            setAtomState(next);
            updateAtomSelection(selectionRef.current, instances);
        }, [registeredAtomTypes, updateAtomSelection]);

        useLayoutEffect(() => {
            if (
                !document.isReady ||
                documentHandle.isDestroyed ||
                atomSeedEditorIdRef.current === editorId
            ) {
                return;
            }
            atomSeedEditorIdRef.current = editorId;
            refreshAtomsFromUpdate(bridge.renderUpdate());
        }, [bridge, document.isReady, documentHandle, editorId, refreshAtomsFromUpdate]);

        const applyTypedUpdateState = useCallback(
            (
                update: Pick<NativeEditorV2AtomicRenderSnapshot, 'selection' | 'activeState'>,
                isCurrent: () => boolean = () => true
            ) => {
                if (!isCurrent()) return false;
                selectionRef.current = update.selection;
                updateAtomSelection(update.selection);
                if (
                    update.selection.type === 'text' &&
                    update.selection.anchorScalar != null &&
                    update.selection.headScalar != null
                ) {
                    scalarSelectionRef.current = {
                        anchor: update.selection.anchorScalar,
                        head: update.selection.headScalar,
                    };
                }
                if (!isCurrent()) return false;
                const nextActiveState = update.activeState;
                activeStateRef.current = nextActiveState;
                if (!isCurrent()) return false;
                setActiveState(nextActiveState);
                if (!isCurrent()) return false;
                const key = stringifyCachedJson(nextActiveState);
                if (key !== activeStateKeyRef.current) {
                    if (!isCurrent()) return false;
                    activeStateKeyRef.current = key;
                    if (!isCurrent()) return false;
                    onActiveStateChangeRef.current?.(nextActiveState);
                }
                return isCurrent();
            },
            [updateAtomSelection]
        );

        const applyUpdateState = useCallback(
            (updateJson: string | null | undefined) => {
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
                const nextActiveState = parseActiveStateFromUpdate(parsed.activeState);
                if (nextSelection && nextActiveState) {
                    applyTypedUpdateState({
                        selection: nextSelection,
                        activeState: nextActiveState,
                    });
                } else if (nextSelection) {
                    selectionRef.current = nextSelection;
                } else if (nextActiveState) {
                    activeStateRef.current = nextActiveState;
                    setActiveState(nextActiveState);
                    const key = stringifyCachedJson(nextActiveState);
                    if (key !== activeStateKeyRef.current) {
                        activeStateKeyRef.current = key;
                        onActiveStateChangeRef.current?.(nextActiveState);
                    }
                }
                return parsed;
            },
            [applyTypedUpdateState]
        );

        const pushEngineUpdateToView = useCallback(() => {
            if (documentHandle.isDestroyed) return;
            const sourceEditorId = documentHandle.editorId;
            const sourceBindingGeneration = pushedUpdateBindingGenerationRef.current;
            const isCurrentSource = () => {
                const current =
                    sourceEditorId === currentPushedUpdateEditorIdRef.current &&
                    sourceBindingGeneration === pushedUpdateBindingGenerationRef.current;
                return current;
            };
            const allocation = allocateEditorUpdateRevision(pushRevisionRef.current);
            if ('error' in allocation) {
                bridge._emitAutonomousError(allocation.error);
                return;
            }
            const snapshot = bridge.renderUpdate();
            // renderUpdate can synchronously re-enter React and bind this
            // component to another handle. Do not let any side effect from A
            // reach B once that happens.
            if (!isCurrentSource()) return;
            refreshAtomsFromUpdate(snapshot);
            if (!isCurrentSource()) return;
            const updateJson = JSON.stringify(snapshot);
            if (!isCurrentSource()) return;
            if (!applyTypedUpdateState(snapshot, isCurrentSource)) {
                return;
            }
            if (!isCurrentSource()) return;
            lastPushedEngineRevisionRef.current = snapshot.documentVersion;
            if (!isCurrentSource()) return;
            pushRevisionRef.current = allocation.revision;
            if (!isCurrentSource()) return;
            setPushedUpdate({
                json: updateJson,
                revision: allocation.revision,
                editorId: sourceEditorId,
            });
        }, [applyTypedUpdateState, bridge, documentHandle, refreshAtomsFromUpdate]);

        useLayoutEffect(() => {
            const pending = pendingFocusedRebindRef.current;
            if (pending?.editorId !== editorId || !document.isReady) return;
            pendingFocusedRebindRef.current = null;

            const snapshot = bridge.renderUpdate();
            const anchor = Math.min(pending.anchor, snapshot.scalarLength);
            const head = Math.min(pending.head, snapshot.scalarLength);
            const collapsed = anchor === head;
            const setSelection = (affinity: NativeEditorV2PositionAffinity) =>
                bridge.setSelection({
                    baseDocumentRevision: snapshot.documentVersion,
                    selection: {
                        type: 'text',
                        anchor: { offset: anchor, kind: 'scalar', affinity },
                        head: { offset: head, kind: 'scalar', affinity },
                    },
                });

            try {
                try {
                    setSelection(collapsed ? 'after' : 'before');
                } catch (error) {
                    if (!collapsed || !isPositionInvalidError(error)) throw error;
                    setSelection('before');
                }
                pushEngineUpdateToView();
            } catch (error) {
                if (!isPositionInvalidError(error)) throw error;
            }

            setActiveEditorToolbarFrameOwnerForEditor(toolbarFrameOwnerId, true);
            onFocusRef.current?.();
        }, [bridge, document.isReady, editorId, pushEngineUpdateToView, toolbarFrameOwnerId]);

        // A pending update is owned by the handle that produced its snapshot.
        // Drop it when this component rebinds, so no old session state reaches
        // the next native view binding.
        useEffect(() => {
            setPushedUpdate((current) => {
                if (current == null || current.editorId === editorId) return current;
                return null;
            });
        }, [editorId]);

        // After a JS-driven engine change (controlled apply, remote commit,
        // document-API mutation) the view learns the new state here. Native-
        // driven commits (typing, native toolbar) already updated the view
        // through the adapter and are never echoed back. The first observed
        // revision is skipped: the view pulls the initial state natively on bind.
        useEffect(() => {
            // The document hook refreshes its state after the rebind commit. Its
            // first render can therefore still contain A's revision; do not let
            // that stale snapshot establish B's initial-observation state.
            if (didRebindRevisionScope) {
                return;
            }
            if (!document.isReady || document.documentRevision == null) return;
            const revision = document.documentRevision;
            if (!didObserveInitialRevisionRef.current) {
                didObserveInitialRevisionRef.current = true;
                lastPushedEngineRevisionRef.current = revision;
                return;
            }
            if (revision === lastPushedEngineRevisionRef.current) {
                return;
            }
            if (
                revision === lastNativeDrivenRevisionRef.current ||
                document.documentOrigin === 'nativeView' ||
                document.documentOrigin === 'remoteCollaboration'
            ) {
                if (
                    document.documentOrigin === 'remoteCollaboration' &&
                    !documentHandle.isDestroyed
                ) {
                    refreshAtomsFromUpdate(bridge.renderUpdate());
                }
                lastPushedEngineRevisionRef.current = revision;
                return;
            }
            lastNativeDrivenRevisionRef.current = null;
            pushEngineUpdateToView();
        }, [
            didRebindRevisionScope,
            document.isReady,
            document.documentRevision,
            document.documentOrigin,
            bridge,
            documentHandle,
            pushEngineUpdateToView,
            refreshAtomsFromUpdate,
        ]);

        const afterLocalEngineMutation = useCallback(() => {
            onLocalCommitRef.current?.();
            pushEngineUpdateToView();
            document.refresh();
        }, [document, pushEngineUpdateToView]);

        const updateAtomAttrs = useCallback(
            async (docPos: number, attrs: Record<string, unknown>): Promise<void> => {
                const baseDocumentRevision = latestRevisionRef.current;
                if (documentHandle.isDestroyed || baseDocumentRevision == null) {
                    throw new AtomUpdateAttrsError('not-ready', 'The editor is not ready');
                }
                let outcome;
                try {
                    outcome = bridge.applyCommand({
                        baseDocumentRevision,
                        command: { type: 'updateNodeAttrs', docPos, attrs },
                    });
                } catch (error) {
                    if (isRevisionMismatchError(error)) {
                        document.refresh();
                        throw new AtomUpdateAttrsError(
                            'stale-revision',
                            'The atom changed before its attributes were updated'
                        );
                    }
                    if (
                        error instanceof NativeEditorV2ErrorBase &&
                        (error.code === 'ENGINE_NOT_READY' ||
                            error.code === 'ENGINE_DESTROYING' ||
                            error.code === 'ENGINE_DESTROYED')
                    ) {
                        throw new AtomUpdateAttrsError('not-ready', 'The editor is not ready');
                    }
                    throw new AtomUpdateAttrsError(
                        'engine-error',
                        error instanceof Error ? error.message : 'The atom update failed'
                    );
                }
                if (outcome.type === 'notApplicable') {
                    throw new AtomUpdateAttrsError(
                        'not-applicable',
                        'The atom no longer exists at this document position'
                    );
                }
                if (outcome.type !== 'transaction') {
                    throw new AtomUpdateAttrsError(
                        'engine-error',
                        'Unexpected atom update outcome'
                    );
                }
                afterLocalEngineMutation();
            },
            [afterLocalEngineMutation, bridge, document, documentHandle]
        );

        const editableRef = useRef(editable);
        editableRef.current = editable;

        const runEngineMutation = useCallback(
            (invoke: (baseDocumentRevision: string) => unknown) => {
                if (!editableRef.current) {
                    throw new NativeEditorV2OperationError({
                        domain: 'operation',
                        code: 'MUTATION_REJECTED',
                        message: 'NativeRichTextEditor: mutation rejected while editable is false',
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
                    itemType:
                        listType === 'taskList'
                            ? 'taskItem'
                            : listType === 'bullet_list' || listType === 'ordered_list'
                              ? 'list_item'
                              : 'listItem',
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

        useImperativeHandle(
            ref,
            (): NativeRichTextEditorRef => ({
                focus() {
                    nativeViewRef.current?.focus?.();
                },
                blur() {
                    nativeViewRef.current?.blur?.();
                },
                supportsExternalTextComposition() {
                    return externalCompositionManager.supports();
                },
                beginExternalTextComposition(
                    options?: ExternalTextCompositionOptions
                ): Promise<ExternalTextCompositionSession> {
                    if (!editable) {
                        return Promise.reject(
                            createExternalCompositionLifecycleError(
                                'EXTERNAL_COMPOSITION_UNAVAILABLE',
                                'The editor view is not editable'
                            )
                        );
                    }
                    return externalCompositionManager.begin(options);
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
                getIsEmpty: document.getIsEmpty,
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
                editable,
                externalCompositionManager,
            ]
        );

        const isForThisEditor = useCallback(
            (payload: { editorId: string }) => payload.editorId === editorId,
            [editorId]
        );

        const handleEditorUpdate = useCallback(
            (event: NativeSyntheticEvent<NativeUpdateEvent>) => {
                if (documentHandle.isDestroyed) return;
                const accepted = acceptNativeCommitPayload(
                    event.nativeEvent,
                    documentHandle.editorId,
                    lastAcceptedNativeCommitRevisionRef.current
                );
                if (accepted == null) return;
                // Record both native-scoped revisions before any observable
                // state/callback/refresh work. This makes duplicate delivery and
                // the handle's same-revision signal deterministic.
                lastAcceptedNativeCommitRevisionRef.current = accepted.documentRevision;
                lastNativeDrivenRevisionRef.current = accepted.documentRevision;
                refreshAtomsFromUpdate(accepted.snapshot);
                applyTypedUpdateState(accepted.snapshot);
                // The adapter already committed; re-read for content callbacks.
                document.refresh();
                onLocalCommitRef.current?.();
            },
            [applyTypedUpdateState, document, documentHandle, refreshAtomsFromUpdate]
        );

        const admitNativeBindingEvent = useCallback(
            (payload: unknown): NativeEditorErrorBinding | null => {
                if (!isRecord(payload)) return null;
                const currentBinding = nativeErrorBindingRef.current;
                if (
                    currentBinding !== nativeErrorBinding ||
                    !currentBinding.mounted ||
                    currentBinding.generation !== nativeErrorBinding.generation ||
                    currentBinding.handle !== nativeErrorBinding.handle ||
                    currentBinding.editorId !== nativeErrorBinding.editorId ||
                    nativeErrorBinding.handle.isDestroyed
                ) {
                    return null;
                }
                const presentedEditorId = payload.editorId;
                if (
                    typeof presentedEditorId !== 'string' ||
                    normalizeNativeEditorV2DecimalId(presentedEditorId) !== presentedEditorId ||
                    presentedEditorId !== nativeErrorBinding.editorId
                ) {
                    return null;
                }
                return currentBinding;
            },
            [nativeErrorBinding]
        );

        const handleEditorError = useCallback(
            (event: NativeSyntheticEvent<NativeErrorEvent>) => {
                const payload = event?.nativeEvent;
                const binding = admitNativeBindingEvent(payload);
                if (binding == null || !isRecord(payload)) return;
                binding.handle.bridge._emitAutonomousError(payload.error);
            },
            [admitNativeBindingEvent]
        );

        const handleExternalTextCompositionEnd = useCallback(
            (event: NativeSyntheticEvent<NativeExternalTextCompositionEvent>) => {
                const payload = event?.nativeEvent;
                const binding = admitNativeBindingEvent(payload);
                if (binding == null || !isRecord(payload)) return;
                if (typeof payload.resultJson !== 'string') return;
                try {
                    externalCompositionManager.handleNativeEnd(
                        binding.editorId,
                        payload.resultJson
                    );
                } catch (error) {
                    binding.handle.bridge._emitAutonomousError(
                        externalCompositionErrorPayload(error)
                    );
                }
            },
            [admitNativeBindingEvent, externalCompositionManager]
        );

        const handleSelectionChange = useCallback(
            (event: NativeSyntheticEvent<NativeSelectionEvent>) => {
                if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
                const { anchor, head, stateJson } = event.nativeEvent;
                scalarSelectionRef.current = { anchor, head };
                let selection: Selection = { type: 'text', anchor, head };
                const parsed = applyUpdateState(stateJson);
                const parsedSelection = parseSelectionFromUpdate(parsed?.selection);
                if (parsedSelection) {
                    selection = parsedSelection;
                }
                selectionRef.current = selection;
                updateAtomSelection(selection);
                onSelectionChangeRef.current?.(selection);
            },
            [applyUpdateState, documentHandle, isForThisEditor, updateAtomSelection]
        );

        const handleFocusChange = useCallback(
            (event: NativeSyntheticEvent<NativeFocusEvent>) => {
                if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
                const focused = event.nativeEvent.isFocused;
                const wasFocused = isFocusedRef.current;
                isFocusedRef.current = focused;
                setActiveEditorToolbarFrameOwnerForEditor(toolbarFrameOwnerId, focused);
                setIsFocused(focused);
                if (focused && !wasFocused) {
                    onFocusRef.current?.();
                } else if (!focused && wasFocused) {
                    onBlurRef.current?.();
                }
            },
            [documentHandle, isForThisEditor, toolbarFrameOwnerId]
        );

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

        const handleAtomLayout = useCallback(
            (event: NativeSyntheticEvent<NativeAtomLayoutEvent>) => {
                if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
                const width = event.nativeEvent.width;
                if (!Number.isFinite(width) || width < 0) return;
                setAtomContentWidth((current) => (current === width ? current : width));
            },
            [documentHandle, isForThisEditor]
        );

        const handleToolbarAction = useCallback(
            (event: NativeSyntheticEvent<NativeToolbarActionEvent>) => {
                if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
                const { key, updateJson, stateJson, documentRevision } = event.nativeEvent;
                // A toolbar event carrying update data is a native commit. It
                // must satisfy the same atomic admission path as typing; a pure
                // action (link/image/custom key) has no commit to refresh.
                if (updateJson != null || documentRevision != null) {
                    if (typeof updateJson !== 'string' || typeof documentRevision !== 'string')
                        return;
                    const accepted = acceptNativeCommitPayload(
                        {
                            editorId: event.nativeEvent.editorId,
                            documentRevision,
                            updateJson,
                        },
                        documentHandle.editorId,
                        lastAcceptedNativeCommitRevisionRef.current
                    );
                    if (accepted == null) return;
                    lastAcceptedNativeCommitRevisionRef.current = accepted.documentRevision;
                    lastNativeDrivenRevisionRef.current = accepted.documentRevision;
                    refreshAtomsFromUpdate(accepted.snapshot);
                    applyTypedUpdateState(accepted.snapshot);
                    document.refresh();
                    onLocalCommitRef.current?.();
                } else if (typeof stateJson === 'string') {
                    applyUpdateState(stateJson);
                }
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
            [
                applyTypedUpdateState,
                applyUpdateState,
                document,
                documentHandle,
                isForThisEditor,
                openImageRequest,
                openLinkRequest,
                refreshAtomsFromUpdate,
            ]
        );

        const resolveMentionSelectionAttrs = useCallback(
            (selectionEvent: MentionSelectionAttrsEvent): Record<string, unknown> => {
                let resolvedAttrs: Record<string, unknown> | null | undefined;
                try {
                    resolvedAttrs =
                        addonsRef.current?.mentions?.resolveSelectionAttrs?.(selectionEvent);
                } catch (error) {
                    if (__DEV__) {
                        console.error(
                            'NativeRichTextEditor: mentions.resolveSelectionAttrs threw',
                            error
                        );
                    }
                }
                return isRecord(resolvedAttrs)
                    ? { ...selectionEvent.attrs, ...resolvedAttrs }
                    : selectionEvent.attrs;
            },
            []
        );

        const resolveMentionTheme = useCallback(
            (selectionEvent: MentionSelectionAttrsEvent): EditorMentionTheme | undefined => {
                let resolvedTheme: unknown;
                try {
                    resolvedTheme = addonsRef.current?.mentions?.resolveTheme?.(selectionEvent);
                } catch (error) {
                    if (__DEV__) {
                        console.error('NativeRichTextEditor: mentions.resolveTheme threw', error);
                    }
                }
                if (resolvedTheme === undefined || resolvedTheme === null) return undefined;
                // A rejected theme is dropped rather than written into the
                // document: every later renderUpdate revalidates it, so one bad
                // value would make the content permanently unrenderable.
                if (!validEditorMentionTheme(resolvedTheme)) {
                    if (__DEV__) {
                        console.error(
                            'NativeRichTextEditor: mentions.resolveTheme did not return an EditorMentionTheme; ignoring it',
                            resolvedTheme
                        );
                    }
                    return undefined;
                }
                return resolvedTheme;
            },
            []
        );

        const resolveMentionInsertionAttrs = useCallback(
            (selectionEvent: MentionSelectionAttrsEvent): Record<string, unknown> => {
                const attrs = resolveMentionSelectionAttrs(selectionEvent);
                const resolvedTheme = resolveMentionTheme({ ...selectionEvent, attrs });
                return resolvedTheme != null ? { ...attrs, mentionTheme: resolvedTheme } : attrs;
            },
            [resolveMentionSelectionAttrs, resolveMentionTheme]
        );

        const insertMentionSuggestion = useCallback(
            (request: {
                trigger: string;
                suggestion: MentionSuggestion;
                attrs: Record<string, unknown>;
                range: { anchor: number; head: number };
                documentVersion?: string;
            }) => {
                const mentions = addonsRef.current?.mentions;
                if (!mentions || !editableRef.current) return;

                const snapshot = bridge.renderUpdate({
                    anchor: request.range.anchor,
                    head: request.range.head,
                });
                if (
                    snapshot.selection.type !== 'text' ||
                    (request.documentVersion != null &&
                        request.documentVersion !== snapshot.documentVersion)
                ) {
                    return;
                }
                const markAttrs = Object.fromEntries(
                    Object.entries(snapshot.activeState.markAttrs).map(([mark, attrs]) => [
                        mark,
                        { ...attrs },
                    ])
                );
                const callbackEvent: MentionSelectionAttrsEvent = {
                    trigger: request.trigger,
                    suggestion: request.suggestion,
                    attrs: request.attrs,
                    markAttrs,
                    range: request.range,
                    documentVersion: snapshot.documentVersion,
                };
                const attrs = resolveMentionInsertionAttrs(callbackEvent);
                // Selection envelopes address scalars, not document positions.
                const anchorScalar = snapshot.selection.anchorScalar;
                const headScalar = snapshot.selection.headScalar;
                if (
                    documentHandle.isDestroyed ||
                    currentPushedUpdateEditorIdRef.current !== documentHandle.editorId ||
                    anchorScalar == null ||
                    headScalar == null
                ) {
                    return;
                }

                // Affinity policy mirrors the native adapters and the engine's
                // own cursor resolution: a collapsed caret prefers After with a
                // deterministic Before fallback at text-boundary positions; a
                // range uses Before. The fallback changes only the stickiness
                // of the SAME position — it is not a guessed-position retry.
                const collapsed = anchorScalar === headScalar;
                const syncSelection = (affinity: NativeEditorV2PositionAffinity) =>
                    bridge.setSelection({
                        baseDocumentRevision: snapshot.documentVersion,
                        selection: {
                            type: 'text',
                            anchor: { offset: anchorScalar, kind: 'scalar', affinity },
                            head: { offset: headScalar, kind: 'scalar', affinity },
                        },
                    });

                try {
                    try {
                        syncSelection(collapsed ? 'after' : 'before');
                    } catch (error) {
                        if (!collapsed || !isPositionInvalidError(error)) throw error;
                        syncSelection('before');
                    }
                    const outcome = bridge.applyCommand({
                        baseDocumentRevision: snapshot.documentVersion,
                        command: {
                            type: 'insertContentJson',
                            json: buildMentionFragmentJson(attrs, documentDescriptor, {
                                trailingSpace: true,
                            }),
                        },
                    });
                    if (outcome.type !== 'transaction' || !outcome.changed) return;
                    latestRevisionRef.current = outcome.documentRevision;
                } catch (error) {
                    if (isRevisionMismatchError(error)) {
                        document.refresh();
                        return;
                    }
                    throw error;
                }

                afterLocalEngineMutation();
                mentions.onSelect?.({
                    trigger: request.trigger,
                    suggestion: request.suggestion,
                    attrs,
                    documentVersion: snapshot.documentVersion,
                });
            },
            [
                afterLocalEngineMutation,
                bridge,
                document,
                documentDescriptor,
                documentHandle,
                resolveMentionInsertionAttrs,
            ]
        );

        const handleAddonEvent = useCallback(
            (event: NativeSyntheticEvent<NativeAddonEvent>) => {
                if (documentHandle.isDestroyed || !isForThisEditor(event.nativeEvent)) return;
                let parsed: EditorAddonEvent;
                try {
                    const value = JSON.parse(event.nativeEvent.eventJson) as unknown;
                    if (!isRecord(value) || typeof value.type !== 'string') return;
                    parsed = value as unknown as EditorAddonEvent;
                } catch {
                    return;
                }

                const mentions = addonsRef.current?.mentions;
                if (!mentions) return;
                const documentVersion =
                    typeof parsed.documentVersion === 'string' ? parsed.documentVersion : undefined;

                if (parsed.type === 'mentionsQueryChange') {
                    if (
                        typeof parsed.query !== 'string' ||
                        typeof parsed.trigger !== 'string' ||
                        typeof parsed.isActive !== 'boolean' ||
                        !isRecord(parsed.range) ||
                        typeof parsed.range.anchor !== 'number' ||
                        typeof parsed.range.head !== 'number'
                    ) {
                        return;
                    }
                    const queryEvent: MentionQueryChangeEvent = {
                        query: parsed.query,
                        trigger: parsed.trigger,
                        range: parsed.range,
                        isActive: parsed.isActive,
                        ...(documentVersion ? { documentVersion } : {}),
                    };
                    mentions.onQueryChange?.(queryEvent);
                    setMentionQuery(parsed.isActive ? queryEvent : null);
                    return;
                }

                if (parsed.type === 'mentionsSelect') {
                    if (
                        typeof parsed.trigger !== 'string' ||
                        typeof parsed.suggestionKey !== 'string' ||
                        !isRecord(parsed.attrs)
                    ) {
                        return;
                    }
                    const suggestion = mentions.suggestions?.find(
                        (candidate) => candidate.key === parsed.suggestionKey
                    );
                    if (!suggestion) return;
                    mentions.onSelect?.({
                        trigger: parsed.trigger,
                        suggestion,
                        attrs: parsed.attrs,
                        ...(documentVersion ? { documentVersion } : {}),
                    });
                    return;
                }

                if (
                    parsed.type !== 'mentionsSelectRequest' ||
                    typeof parsed.trigger !== 'string' ||
                    typeof parsed.suggestionKey !== 'string' ||
                    !isRecord(parsed.attrs) ||
                    !isRecord(parsed.range) ||
                    !Number.isInteger(parsed.range.anchor) ||
                    !Number.isInteger(parsed.range.head) ||
                    parsed.range.anchor < 0 ||
                    parsed.range.head < 0 ||
                    parsed.range.anchor > 0xffff_ffff ||
                    parsed.range.head > 0xffff_ffff
                ) {
                    return;
                }
                const suggestion = mentions.suggestions?.find(
                    (candidate) => candidate.key === parsed.suggestionKey
                );
                if (!suggestion) return;

                insertMentionSuggestion({
                    trigger: parsed.trigger,
                    suggestion,
                    attrs: parsed.attrs,
                    range: parsed.range,
                    documentVersion,
                });
            },
            [documentHandle, insertMentionSuggestion, isForThisEditor]
        );

        const handleMentionSuggestionPress = useCallback(
            (suggestion: MentionSuggestion) => {
                if (mentionQuery == null) return;
                const normalized = normalizeEditorAddons(
                    addonsRef.current
                )?.mentions?.suggestions.find((candidate) => candidate.key === suggestion.key);
                if (normalized == null) return;

                setMentionQuery(null);
                insertMentionSuggestion({
                    trigger: mentionQuery.trigger,
                    suggestion,
                    attrs: normalized.attrs,
                    range: mentionQuery.range,
                    documentVersion: mentionQuery.documentVersion,
                });
            },
            [insertMentionSuggestion, mentionQuery]
        );

        const mentionSuggestions = addons?.mentions?.suggestions;
        const mentionSuggestionTheme = addons?.mentions?.theme;
        const shouldPublishMentionSuggestions =
            editable && isFocused && mentionQuery != null && (mentionSuggestions?.length ?? 0) > 0;

        const mentionSuggestionThemes = useMemo(() => {
            if (
                mentionQuery == null ||
                mentionSuggestions == null ||
                typeof addons?.mentions?.resolveTheme !== 'function'
            ) {
                return undefined;
            }

            const normalized = normalizeEditorAddons(addons)?.mentions?.suggestions;
            if (normalized == null) return undefined;

            const themes: Record<string, EditorMentionTheme> = {};
            for (const suggestion of mentionSuggestions) {
                const normalizedSuggestion = normalized.find(
                    (candidate) => candidate.key === suggestion.key
                );
                if (normalizedSuggestion == null) continue;

                const selectionEvent: MentionSelectionAttrsEvent = {
                    trigger: mentionQuery.trigger,
                    suggestion,
                    attrs: normalizedSuggestion.attrs,
                    markAttrs: activeStateRef.current.markAttrs,
                    range: mentionQuery.range,
                    ...(mentionQuery.documentVersion
                        ? { documentVersion: mentionQuery.documentVersion }
                        : {}),
                };
                const attrs = resolveMentionSelectionAttrs(selectionEvent);
                const merged = mergeMentionSuggestionTheme(
                    mentionSuggestionTheme,
                    resolveMentionTheme({ ...selectionEvent, attrs })
                );
                if (merged != null) {
                    themes[suggestion.key] = merged;
                }
            }

            return Object.keys(themes).length > 0 ? themes : undefined;
        }, [
            addons,
            mentionQuery,
            mentionSuggestionTheme,
            mentionSuggestions,
            resolveMentionSelectionAttrs,
            resolveMentionTheme,
        ]);

        useEffect(() => {
            if (
                !shouldPublishMentionSuggestions ||
                mentionQuery == null ||
                mentionSuggestions == null
            ) {
                setEditorToolbarMentionState(toolbarFrameOwnerId, null);
                return;
            }

            setEditorToolbarMentionState(toolbarFrameOwnerId, {
                trigger: mentionQuery.trigger,
                suggestions: mentionSuggestions,
                theme: mentionSuggestionTheme,
                suggestionThemes: mentionSuggestionThemes,
                onSelectSuggestion: handleMentionSuggestionPress,
            });
        }, [
            handleMentionSuggestionPress,
            mentionQuery,
            mentionSuggestionTheme,
            mentionSuggestionThemes,
            mentionSuggestions,
            shouldPublishMentionSuggestions,
            toolbarFrameOwnerId,
        ]);

        const themeJson = useMemo(
            () => serializeEditorTheme(theme, mentionSuggestionTheme),
            [mentionSuggestionTheme, theme]
        );
        const addonsJson = useSerializedValue(addons, (value) => serializeEditorAddons(value));
        const imageLoadingPolicyJson = useSerializedValue(imageLoadingPolicy, (value) =>
            serializeEditorImageLoadingPolicy(value)
        );
        const androidInputOptionsJson = useSerializedValue(androidInputOptions, (value) =>
            JSON.stringify(value)
        );
        const remoteSelectionsJson = useSerializedValue(remoteSelections, (selections) =>
            serializeRemoteSelections(selections)
        );
        const atomsJson = useMemo(() => {
            const supplied = serializeEditorAtoms(atoms);
            const serialized =
                supplied == null
                    ? { nodeTypes: [] as string[], estimatedHeights: {} as Record<string, number> }
                    : (JSON.parse(supplied) as {
                          nodeTypes: string[];
                          estimatedHeights: Record<string, number>;
                      });
            for (const instance of atomState.instances) {
                if (
                    Object.prototype.hasOwnProperty.call(
                        serialized.estimatedHeights,
                        instance.nodeType
                    )
                ) {
                    continue;
                }
                serialized.nodeTypes.push(instance.nodeType);
                serialized.estimatedHeights[instance.nodeType] = DEFAULT_ATOM_CHIP_HEIGHT;
            }
            return serialized.nodeTypes.length === 0 ? undefined : JSON.stringify(serialized);
        }, [atomState.instances, atoms]);

        useEffect(() => {
            if (!__DEV__) return;
            for (const instance of atomState.instances) {
                if (
                    atomComponents.has(instance.nodeType) ||
                    warnedUnknownAtomTypesRef.current.has(instance.nodeType)
                ) {
                    continue;
                }
                warnedUnknownAtomTypesRef.current.add(instance.nodeType);
                console.warn(
                    `NativeRichTextEditor: rendering unknown atom type '${instance.nodeType}' as a chip`
                );
            }
        }, [atomComponents, atomState.instances]);

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
        const currentPushedUpdate = pushedUpdate?.editorId === editorId ? pushedUpdate : null;
        const focusPreservingFrames = [
            ...registeredToolbarFrames,
            ...suppliedFocusPreservingFrames,
        ];
        const toolbarFrameJson = serializeToolbarFrames(
            editable && isFocused ? focusPreservingFrames : undefined
        );
        const atomChildren =
            atomContentWidth == null
                ? null
                : atomState.instances.map((instance) => {
                      const Component = atomComponents.get(instance.nodeType) ?? DefaultAtomChip;
                      return (
                          <View
                              key={instance.key}
                              nativeID={`prose-atom:${instance.key}`}
                              collapsable={false}
                              style={{
                                  position: 'absolute',
                                  top: 0,
                                  left: 0,
                                  width: atomContentWidth,
                              }}>
                              <Component
                                  attrs={instance.attrs}
                                  selected={selectedKeys.has(instance.key)}
                                  nodeType={instance.nodeType}
                                  updateAttrs={(attrs) => updateAtomAttrs(instance.docPos, attrs)}
                              />
                          </View>
                      );
                  });

        return (
            <View style={[styles.container, containerStyle]}>
                <NativeEditorView
                    ref={nativeViewRef}
                    style={nativeViewStyle}
                    onLayout={refreshFocusPreservingFrames}
                    accessibilityLabel={accessibilityLabel}
                    accessibilityHint={accessibilityHint}
                    editorId={editorId}
                    placeholder={placeholder}
                    editable={editable}
                    autoFocus={autoFocus}
                    autoCapitalize={autoCapitalize}
                    autoCorrect={autoCorrect}
                    keyboardType={keyboardType}
                    {...(Platform.OS === 'android' ? { androidInputOptionsJson } : {})}
                    showToolbar={showToolbar}
                    toolbarPlacement={toolbarPlacement}
                    heightBehavior={heightBehavior}
                    allowImageResizing={allowImageResizing}
                    imageLoadingPolicyJson={imageLoadingPolicyJson}
                    themeJson={themeJson}
                    addonsJson={addonsJson}
                    atomsJson={atomsJson}
                    toolbarItemsJson={toolbarItemsJson}
                    toolbarFrameJson={toolbarFrameJson}
                    remoteSelectionsJson={remoteSelectionsJson}
                    editorUpdateJson={currentPushedUpdate?.json}
                    editorUpdateEditorId={currentPushedUpdate?.editorId}
                    editorUpdateRevision={currentPushedUpdate?.revision ?? 0}
                    onEditorUpdate={handleEditorUpdate}
                    onEditorError={handleEditorError}
                    onExternalTextCompositionEnd={handleExternalTextCompositionEnd}
                    onSelectionChange={handleSelectionChange}
                    onFocusChange={handleFocusChange}
                    onContentHeightChange={handleContentHeightChange}
                    onAtomLayout={handleAtomLayout}
                    onToolbarAction={handleToolbarAction}
                    onAddonEvent={handleAddonEvent}>
                    {atomChildren}
                </NativeEditorView>
                {shouldRenderJsToolbar ? (
                    <View
                        testID='native-editor-js-toolbar'
                        style={[styles.inlineToolbar, { marginTop: inlineToolbarMarginTop }]}>
                        <EditorToolbarFrameOwnerProvider ownerId={toolbarFrameOwnerId}>
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
                        </EditorToolbarFrameOwnerProvider>
                    </View>
                ) : null}
            </View>
        );
    }
);

const styles = StyleSheet.create({
    container: {
        position: 'relative',
    },
    inlineToolbar: {
        flexDirection: 'row',
        alignItems: 'center',
    },
});
