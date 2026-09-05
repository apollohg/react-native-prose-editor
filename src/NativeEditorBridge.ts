import { requireNativeModule } from 'expo-modules-core';
import type {
    EditorMentionNodeTheme,
    EditorMentionSuggestionOptionTheme,
    EditorMentionSuggestionsTheme,
    EditorMentionTheme,
} from './EditorTheme';
import {
    HARD_EDITOR_RESOURCE_LIMITS,
    validateEditorCreateLimits,
    type EditorCollaborationLimits,
    type EditorEditingLimits,
    type EditorResourceLimits,
} from './ResourceLimits';
import {
    resolveDocumentDescriptor,
    type ResolvedDocumentSchema,
    type SchemaDefinition,
} from './schemas';
import {
    NativeEditorBoundaryError,
    NativeEditorV2BoundaryError,
    NativeEditorV2ErrorBase,
    NativeEditorV2NonRetryableError,
    nativeEditorV2ErrorToException,
    normalizeNativeEditorV2Error,
    type NativeEditorV2Error,
} from './NativeEditorBoundaryError';
import { normalizeNativeEditorV2U64 } from './NativeEditorV2Decimal';

// Neutral document/render state types shared by the v2 document
// handle, the React components, and the schema/addon helpers.

/**
 * A selection in engine document positions.
 *
 * `anchor`/`head` are set for a `'text'` selection, `pos` for a `'node'` one,
 * and neither for `'all'`. The `*Scalar` variants carry the same points
 * measured in Unicode scalars instead of document positions.
 */
export interface Selection {
    type: 'text' | 'node' | 'all';
    /** Fixed end of a text selection. */
    anchor?: number;
    /** Moving end of a text selection. Equals `anchor` for a collapsed caret. */
    head?: number;
    /** Position of the selected node, for a `'node'` selection. */
    pos?: number;
    /** `anchor` in Unicode scalars. */
    anchorScalar?: number;
    /** `head` in Unicode scalars. */
    headScalar?: number;
    /** `pos` in Unicode scalars. */
    posScalar?: number;
}

/**
 * An opaque, factory-created document range for local awareness publication.
 * Its provenance is verified at runtime before a native call; the type brand
 * prevents accidental literal construction in TypeScript consumers.
 */
class NativeEditorLocalAwarenessSelectionValue {
    private readonly _nativeEditorLocalAwarenessSelectionBrand!: undefined;

    constructor(
        readonly anchor: number,
        readonly head: number
    ) {}
}

export type NativeEditorLocalAwarenessSelection = NativeEditorLocalAwarenessSelectionValue;

/** What this client publishes to other peers in one awareness update. */
export interface NativeEditorLocalAwarenessIntent {
    /**
     * Application-defined presence payload, e.g. `{ user: { name, color } }`.
     * Published at the awareness root, so `cursor` and `focused` are reserved.
     */
    state: Record<string, unknown>;
    /** Whether this client currently holds editing focus. */
    focused: boolean;
    /**
     * How this intent treats the local cursor. Rust owns it as a sticky
     * index, so the three cases are deliberately distinct:
     *
     * - **omitted** — retain the cursor already published. Use this for
     *   focus-only or state-only updates: restating a document position
     *   that the document has since invalidated would be refused.
     * - **`null`** — publish presence with no cursor at all.
     * - **a factory selection** — set the cursor to these positions, which
     *   must resolve against the current document.
     */
    selection?: NativeEditorLocalAwarenessSelection | null;
}

/** Where a list item sits in its list, supplied with every rendered list block. */
export interface ListContext {
    /** Whether the enclosing list is numbered. */
    ordered: boolean;
    /** Zero-based position of this item among its siblings. */
    index: number;
    /** Number of sibling items in the enclosing list. */
    total: number;
    /** The list's `start` attribute — the number the first item takes. */
    start: number;
    isFirst: boolean;
    isLast: boolean;
    /** List variant, e.g. `'task'` for a checklist. */
    kind?: string | null;
    /** Checked state, for a task list item. */
    checked?: boolean | null;
}

/** A mark carrying attributes, e.g. a link with its `href`. */
export interface RenderMarkWithAttrs {
    type: string;
    [key: string]: unknown;
}

/** A mark on a rendered text run: its name alone, or its name plus attributes. */
export type RenderMark = string | RenderMarkWithAttrs;

/** One piece of the flattened render stream the engine produces for a document. */
export interface RenderElement {
    type:
        | 'textRun'
        | 'blockStart'
        | 'blockEnd'
        | 'voidInline'
        | 'voidBlock'
        | 'opaqueInlineAtom'
        | 'opaqueBlockAtom';
    text?: string;
    marks?: RenderMark[];
    nodeType?: string;
    depth?: number;
    docPos?: number;
    atomId?: string;
    label?: string;
    attrs?: Record<string, unknown>;
    mentionTheme?: EditorMentionTheme;
    listContext?: ListContext;
}

/**
 * A splice against the previously rendered block list: replace `deleteCount`
 * blocks at `startIndex` with `renderBlocks`. Lets a view redraw only the
 * blocks that changed.
 */
export interface RenderBlocksPatch {
    baseDocumentVersion: string;
    startIndex: number;
    deleteCount: number;
    renderBlocks: RenderElement[][];
}

/** What the current selection can do — the state a toolbar renders from. */
export interface ActiveState {
    /** Marks active at the selection, keyed by mark name. */
    marks: Record<string, boolean>;
    /** Attributes of each active mark, e.g. `{ link: { href } }`. */
    markAttrs: Record<string, Record<string, unknown>>;
    /** Node types enclosing the selection, keyed by node name. */
    nodes: Record<string, boolean>;
    /** Whether each command would apply at the selection, keyed by command name. */
    commands: Record<string, boolean>;
    /** Mark names the schema permits at the selection. */
    allowedMarks: string[];
    /** Node names that can be inserted at the selection. */
    insertableNodes: string[];
}

/** Undo and redo availability. */
export interface HistoryState {
    canUndo: boolean;
    canRedo: boolean;
}

type DeepReadonly<T> =
    T extends ReadonlyArray<infer Item>
        ? ReadonlyArray<DeepReadonly<Item>>
        : T extends object
          ? { readonly [Key in keyof T]: DeepReadonly<T[Key]> }
          : T;

/** A recursively immutable active-state view supplied by atomic render snapshots. */
export type ReadonlyActiveState = DeepReadonly<ActiveState>;

type NativeEditorV2AtomicRenderPayload =
    | { renderBlocks: RenderElement[][]; renderPatch: null }
    | { renderBlocks: null; renderPatch: RenderBlocksPatch };

type NativeEditorV2AtomicRenderSnapshotShape = NativeEditorV2AtomicRenderPayload & {
    selection: Selection;
    activeState: ActiveState;
    historyState: HistoryState;
    documentVersion: string;
    stateRevision: string;
    scalarLength: number;
    /** The core's own answer for whether the document holds no content. */
    documentIsEmpty: boolean;
};

/** A recursively immutable view of the value frozen by renderUpdate(). */
export type NativeEditorV2AtomicRenderSnapshot =
    DeepReadonly<NativeEditorV2AtomicRenderSnapshotShape>;

/** One coherent view of the document after a change: what to draw, plus selection and state. */
export interface EditorUpdate {
    /** The whole document as a flat render stream. */
    renderElements: RenderElement[];
    /** The same content grouped into blocks. */
    renderBlocks?: RenderElement[][];
    /** Blocks that changed since the previous update, when the engine could compute a splice. */
    renderPatch?: RenderBlocksPatch;
    selection: Selection;
    activeState: ActiveState;
    historyState: HistoryState;
    /** Decimal-string engine document revision this update describes. */
    documentVersion?: string;
}

/** The document in both serialized forms, read in one pass. */
export interface ContentSnapshot {
    html: string;
    json: DocumentJSON;
}

/**
 * A ProseMirror-style JSON document or fragment. Deliberately open: the shape
 * is whatever the active {@link SchemaDefinition} admits, so nodes carry
 * `type`, optional `attrs`, `content`, `marks`, and `text`.
 */
export interface DocumentJSON {
    [key: string]: unknown;
}

/** A participant in a collaboration room. */
export interface CollaborationPeer {
    /** Yjs client identity, as a decimal string. */
    clientId: string;
    /** Whether this record describes this device. */
    isLocal: boolean;
    /** The peer's published awareness state. */
    state: Record<string, unknown> | null;
}

export type CommandBlockedReason =
    | 'composition'
    | 'detached'
    | 'pendingUpdate'
    | 'destroyed'
    | 'unknown';

export interface CommandBlockedInfo {
    blocked: boolean;
    reason: CommandBlockedReason | null;
}

let _nativeModule: NativeEditorV2Module | null = null;

function getNativeModule(): NativeEditorV2Module {
    if (!_nativeModule) {
        _nativeModule = requireNativeModule<NativeEditorV2Module>('NativeEditor');
    }
    return _nativeModule;
}

/** @internal Reset the cached native module reference. For testing only. */
export function _resetNativeModuleCache(): void {
    _nativeModule = null;
}

// The only construction path. Consumes the frozen v2 result records
// ({ value, error }, exactly one side set), normalizes them at the JS boundary
// (decimal-string u64s, direct binaries, unsafe-integer rejection), and raises
// typed per-domain errors with a non-retryable class for
// ENGINE_INVARIANT_FAILED and destroyed lifecycles.

const ERR_V2_NATIVE_RESPONSE = 'NativeEditorBridge: invalid v2 result record from native module';
const ERR_V2_DESTROYED = 'NativeEditorBridge: v2 editor handle has been destroyed';
const V2_ENVELOPE_VERSION = 1;

/**
 * The v2 surface of the NativeEditor native module — the complete
 * production ABI. Every call resolves the method lazily and fails clearly
 * when the v2 surface is absent. Decimal-string identifiers keep full u64
 * fidelity across the JavaScript boundary, and binaries travel as direct
 * Uint8Array values (never JSON number arrays).
 */
export interface NativeEditorV2Module {
    editorV2Create(configJson: string, snapshotState: Uint8Array | null): unknown;
    editorV2Destroy(editorId: string): unknown;
    editorV2GetState(editorId: string): unknown;
    editorV2GetDocumentJson(editorId: string): unknown;
    editorV2GetDocumentHtml(editorId: string): unknown;
    editorV2GetContentSnapshot(editorId: string): unknown;
    editorV2ReplaceDocument(editorId: string, requestJson: string): unknown;
    editorV2ApplyInput(editorId: string, requestJson: string): unknown;
    editorV2ApplyCommand(editorId: string, requestJson: string): unknown;
    editorV2ApplyLocalApi(editorId: string, requestJson: string): unknown;
    editorV2SetSelection(editorId: string, requestJson: string): unknown;
    editorV2Undo(editorId: string, requestJson: string): unknown;
    editorV2Redo(editorId: string, requestJson: string): unknown;
    editorV2RenderUpdate(
        editorId: string,
        mirrorScalarAnchor: number | null,
        mirrorScalarHead: number | null
    ): unknown;
    editorV2CollaborationConfigureTransport(
        editorId: string,
        configJsonOrNull: string | null
    ): unknown;
    editorV2CollaborationResolveProtocolAdapter(
        editorId: string,
        attemptId: string,
        eventId: string,
        responseJson: string
    ): unknown;
    editorV2CollaborationSetAwareness(editorId: string, awarenessJson: string): unknown;
    editorV2SnapshotExport(editorId: string): unknown;
    editorV2SnapshotRestore(
        editorId: string,
        metadataJson: string,
        encodedState: Uint8Array
    ): unknown;
    addListener(
        eventName: 'onCollaborationTransportEvent',
        listener: (event: unknown) => void
    ): { remove(): void };
}

function invokeNativeEditorV2<K extends keyof NativeEditorV2Module>(
    name: K,
    ...args: Parameters<NativeEditorV2Module[K]>
): unknown {
    const nativeModule = getNativeModule() as unknown as Record<string, unknown>;
    const method = nativeModule[name as string];
    if (typeof method !== 'function') {
        throw new Error(
            `NativeEditorBridge: native module does not expose the v2 entry ${String(name)}`
        );
    }
    return (method as (...fnArgs: unknown[]) => unknown).apply(nativeModule, args);
}

/** The discriminated envelope every v2 result record normalizes into. */
export type NativeEditorV2Result<T> =
    | { ok: true; value: T }
    | { ok: false; error: NativeEditorV2Error };

function isPlainRecord(value: unknown): value is Record<string, unknown> {
    return value != null && typeof value === 'object' && !Array.isArray(value);
}

/**
 * Validate a raw v2 result record: exactly one of value/error, every error
 * field type, every domain/code string, and the caller-supplied value
 * normalizer. Returns null for any contract violation; the imperative layer
 * turns that into the non-retryable FFI_RESULT_INVALID error.
 */
export function normalizeNativeEditorV2Result<T>(
    raw: unknown,
    normalizeValue: (value: unknown) => T | null
): NativeEditorV2Result<T> | null {
    if (!isPlainRecord(raw)) return null;
    const hasValue = raw.value !== null && raw.value !== undefined;
    const hasError = raw.error !== null && raw.error !== undefined;
    if (hasValue === hasError) return null;
    if (hasError) {
        const error = normalizeNativeEditorV2Error({ error: raw.error });
        return error == null ? null : { ok: false, error };
    }
    const value = normalizeValue(raw.value);
    return value == null ? null : { ok: true, value };
}

/** Parse a JSON-string result value; anything else is a contract violation. */
export function parseNativeEditorV2JsonValue(value: unknown): unknown | null {
    if (typeof value !== 'string' || value === '') return null;
    try {
        return JSON.parse(value) as unknown;
    } catch {
        return null;
    }
}

/**
 * Validate a direct binary result value. JSON number arrays are rejected on
 * purpose: the frozen v2 contract moves bytes as bytes.
 */
export function normalizeNativeEditorV2Bytes(value: unknown): Uint8Array | null {
    if (value instanceof Uint8Array) return value;
    if (ArrayBuffer.isView(value)) {
        return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
    }
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    return null;
}

/** FfiUnitResult successes are exactly `true`; anything else is invalid. */
export function normalizeNativeEditorV2Unit(value: unknown): true | null {
    return value === true ? true : null;
}

/**
 * Normalize a request/revision identifier to its canonical decimal string.
 * V2 never accepts numeric compatibility values: JavaScript numbers cannot
 * represent the complete Rust u64 domain.
 */
export function normalizeNativeEditorV2DecimalId(value: unknown): string | null {
    return normalizeNativeEditorV2U64(value);
}

function normalizeRevisionField(record: Record<string, unknown>, field: string): string | null {
    return normalizeNativeEditorV2DecimalId(record[field]);
}

function nativeEditorV2U32(value: unknown): number | null {
    return typeof value === 'number' &&
        Number.isFinite(value) &&
        Number.isInteger(value) &&
        value >= 0 &&
        value <= 0xffff_ffff
        ? value
        : null;
}

/** Require one exact JavaScript/platform u32 value at the v2 boundary. */
export function requireNativeEditorV2U32(value: unknown, field: string): number {
    const normalized = nativeEditorV2U32(value);
    if (normalized == null) {
        throw invalidV2RequestError(`NativeEditorBridge: invalid u32 ${field}`);
    }
    return normalized;
}

function optionalBoolean(value: unknown): boolean | null {
    return typeof value === 'boolean' ? value : null;
}

const V2_DOCUMENT_STATES = ['LocalReady', 'AwaitRemote', 'RoomReady'] as const;
/**
 * Readiness of the engine document. `AwaitRemote` is a room document still
 * waiting for the server's copy: content getters return empty values and the
 * view renders nothing until it promotes to `RoomReady`.
 */
export type NativeEditorV2DocumentState = (typeof V2_DOCUMENT_STATES)[number];

const V2_TRANSPORT_STATES = [
    'Detached',
    'Disconnected',
    'Connecting',
    'Handshaking',
    'Synchronized',
    'Incompatible',
    'Destroying',
    'Destroyed',
] as const;
/** Raw transport lifecycle. `YjsTransportStatus` is the friendlier projection of it. */
export type NativeEditorV2TransportState = (typeof V2_TRANSPORT_STATES)[number];

const V2_RENDER_STATES = ['Loading', 'Ready'] as const;
/** Whether the engine has a render snapshot to draw. */
export type NativeEditorV2RenderState = (typeof V2_RENDER_STATES)[number];

const V2_DOCUMENT_ORIGINS = [
    'nativeView',
    'jsApi',
    'remoteCollaboration',
    'history',
    'restore',
    'import',
] as const;
export type NativeEditorV2DocumentOrigin = (typeof V2_DOCUMENT_ORIGINS)[number];

function whitelisted<T extends string>(value: unknown, allowed: readonly T[]): T | null {
    return typeof value === 'string' && (allowed as readonly string[]).includes(value)
        ? (value as T)
        : null;
}

/** The engine's own account of a session: readiness, revisions, and history. */
export interface NativeEditorV2EditorState {
    documentState: NativeEditorV2DocumentState;
    transportState: NativeEditorV2TransportState;
    renderState: NativeEditorV2RenderState;
    /** Decimal-string revision advancing on every document change. */
    documentRevision: string;
    /** Trusted origin of the transaction that produced documentRevision. */
    documentOrigin: NativeEditorV2DocumentOrigin;
    /** Decimal-string revision advancing on every state change, document or not. */
    stateRevision: string;
    canUndo: boolean;
    canRedo: boolean;
}

export function normalizeNativeEditorV2StateValue(
    value: unknown
): NativeEditorV2EditorState | null {
    const parsed = typeof value === 'string' ? parseNativeEditorV2JsonValue(value) : value;
    if (!isPlainRecord(parsed)) return null;
    const documentState = whitelisted(parsed.documentState, V2_DOCUMENT_STATES);
    const transportState = whitelisted(parsed.transportState, V2_TRANSPORT_STATES);
    const renderState = whitelisted(parsed.renderState, V2_RENDER_STATES);
    const documentRevision = normalizeRevisionField(parsed, 'documentRevision');
    const documentOrigin = whitelisted(parsed.documentOrigin, V2_DOCUMENT_ORIGINS);
    const stateRevision = normalizeRevisionField(parsed, 'stateRevision');
    const canUndo = optionalBoolean(parsed.canUndo);
    const canRedo = optionalBoolean(parsed.canRedo);
    if (
        documentState == null ||
        transportState == null ||
        renderState == null ||
        documentRevision == null ||
        documentOrigin == null ||
        stateRevision == null ||
        canUndo == null ||
        canRedo == null
    ) {
        return null;
    }
    return {
        documentState,
        transportState,
        renderState,
        documentRevision,
        documentOrigin,
        stateRevision,
        canUndo,
        canRedo,
    };
}

export type NativeEditorV2MutationOutcome =
    | {
          type: 'transaction';
          changed: boolean;
          documentRevision: string;
          stateRevision: string;
          canUndo: boolean;
          canRedo: boolean;
      }
    | { type: 'notApplicable' }
    | { type: 'replacement'; changed: boolean; documentRevision: string };

export function normalizeNativeEditorV2MutationOutcomeValue(
    value: unknown
): NativeEditorV2MutationOutcome | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    if (parsed.type === 'notApplicable') {
        return { type: 'notApplicable' };
    }
    if (parsed.type === 'replacement') {
        const changed = optionalBoolean(parsed.changed);
        const documentRevision = normalizeRevisionField(parsed, 'documentRevision');
        if (changed == null || documentRevision == null) return null;
        return { type: 'replacement', changed, documentRevision };
    }
    if (parsed.type === 'transaction') {
        const changed = optionalBoolean(parsed.changed);
        const documentRevision = normalizeRevisionField(parsed, 'documentRevision');
        const stateRevision = normalizeRevisionField(parsed, 'stateRevision');
        const canUndo = optionalBoolean(parsed.canUndo);
        const canRedo = optionalBoolean(parsed.canRedo);
        if (
            changed == null ||
            documentRevision == null ||
            stateRevision == null ||
            canUndo == null ||
            canRedo == null
        ) {
            return null;
        }
        return {
            type: 'transaction',
            changed,
            documentRevision,
            stateRevision,
            canUndo,
            canRedo,
        };
    }
    return null;
}

export interface NativeEditorV2CommitInfo {
    changed: boolean;
    documentRevision: string;
}

export function normalizeNativeEditorV2CommitValue(
    value: unknown
): NativeEditorV2CommitInfo | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    const changed = optionalBoolean(parsed.changed);
    const documentRevision = normalizeRevisionField(parsed, 'documentRevision');
    if (changed == null || documentRevision == null) return null;
    return { changed, documentRevision };
}

export function normalizeNativeEditorV2ChangedValue(value: unknown): boolean | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    return optionalBoolean(parsed.changed);
}

export function normalizeNativeEditorV2HtmlValue(value: unknown): string | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed) || typeof parsed.html !== 'string') return null;
    return parsed.html;
}

const RENDER_ELEMENT_TYPES = new Set<RenderElement['type']>([
    'textRun',
    'blockStart',
    'blockEnd',
    'voidInline',
    'voidBlock',
    'opaqueInlineAtom',
    'opaqueBlockAtom',
]);

function hasExactOwnKeys(record: Record<string, unknown>, expected: readonly string[]): boolean {
    const actual = Object.keys(record).sort();
    const sortedExpected = [...expected].sort();
    return (
        actual.length === sortedExpected.length &&
        actual.every((key, index) => key === sortedExpected[index])
    );
}

function hasOnlyOwnKeys(record: Record<string, unknown>, allowed: readonly string[]): boolean {
    const allowedKeys = new Set(allowed);
    return Object.keys(record).every((key) => allowedKeys.has(key));
}

function booleanRecord(value: unknown): value is Record<string, boolean> {
    return (
        isPlainRecord(value) && Object.values(value).every((entry) => typeof entry === 'boolean')
    );
}

function stringArray(value: unknown): value is string[] {
    return Array.isArray(value) && value.every((entry) => typeof entry === 'string');
}

function validJsonValue(value: unknown): boolean {
    if (value === null) return true;
    if (typeof value === 'string' || typeof value === 'boolean') return true;
    if (typeof value === 'number') return Number.isFinite(value);
    if (Array.isArray(value)) return value.every(validJsonValue);
    return isPlainRecord(value) && Object.values(value).every(validJsonValue);
}

function validRenderMark(value: unknown): value is RenderMark {
    return (
        typeof value === 'string' ||
        (isPlainRecord(value) &&
            typeof value.type === 'string' &&
            Object.values(value).every(validJsonValue))
    );
}

const MENTION_NODE_STRING_FIELDS = [
    'textColor',
    'backgroundColor',
    'borderColor',
] as const satisfies readonly (keyof EditorMentionNodeTheme)[];

const MENTION_NODE_NUMBER_FIELDS = [
    'borderWidth',
    'borderRadius',
] as const satisfies readonly (keyof EditorMentionNodeTheme)[];

const MENTION_OPTION_STRING_FIELDS = [
    'textColor',
    'secondaryTextColor',
    'backgroundColor',
    'borderColor',
    'highlightedBackgroundColor',
    'highlightedTextColor',
] as const satisfies readonly (keyof EditorMentionSuggestionOptionTheme)[];

const MENTION_OPTION_NUMBER_FIELDS = [
    'borderWidth',
    'borderRadius',
] as const satisfies readonly (keyof EditorMentionSuggestionOptionTheme)[];

const MENTION_SUGGESTIONS_STRING_FIELDS = [
    'backgroundColor',
    'borderColor',
    'shadowColor',
] as const satisfies readonly (keyof EditorMentionSuggestionsTheme)[];

const MENTION_SUGGESTIONS_NUMBER_FIELDS = [
    'borderWidth',
    'borderRadius',
] as const satisfies readonly (keyof EditorMentionSuggestionsTheme)[];

const EDITOR_MENTION_THEME_FONT_WEIGHTS: ReadonlySet<
    NonNullable<EditorMentionNodeTheme['fontWeight']>
> = new Set(['normal', 'bold', '100', '200', '300', '400', '500', '600', '700', '800', '900']);

const EDITOR_MENTION_THEME_FIELDS = [
    'node',
    'suggestions',
] as const satisfies readonly (keyof EditorMentionTheme)[];

function validMentionFontWeight(value: unknown): boolean {
    return (
        value === undefined ||
        (typeof value === 'string' &&
            EDITOR_MENTION_THEME_FONT_WEIGHTS.has(
                value as NonNullable<EditorMentionNodeTheme['fontWeight']>
            ))
    );
}

function validMentionThemeSection(
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

function validListContext(value: unknown): value is ListContext {
    if (!isPlainRecord(value)) return false;
    return (
        hasOnlyOwnKeys(value, [
            'ordered',
            'index',
            'total',
            'start',
            'isFirst',
            'isLast',
            'kind',
            'checked',
        ]) &&
        typeof value.ordered === 'boolean' &&
        nativeEditorV2U32(value.index) != null &&
        nativeEditorV2U32(value.total) != null &&
        nativeEditorV2U32(value.start) != null &&
        typeof value.isFirst === 'boolean' &&
        typeof value.isLast === 'boolean' &&
        (value.kind == null || typeof value.kind === 'string') &&
        (value.checked == null || typeof value.checked === 'boolean')
    );
}

function validRenderElement(value: unknown): value is RenderElement {
    if (!isPlainRecord(value) || !RENDER_ELEMENT_TYPES.has(value.type as RenderElement['type'])) {
        return false;
    }
    switch (value.type) {
        case 'textRun':
            return (
                hasExactOwnKeys(value, ['type', 'text', 'marks']) &&
                typeof value.text === 'string' &&
                Array.isArray(value.marks) &&
                value.marks.every(validRenderMark)
            );
        case 'blockStart':
            return (
                hasOnlyOwnKeys(value, ['type', 'nodeType', 'depth', 'listContext']) &&
                typeof value.nodeType === 'string' &&
                nativeEditorV2U32(value.depth) != null &&
                (value.listContext === undefined || validListContext(value.listContext))
            );
        case 'blockEnd':
            return hasExactOwnKeys(value, ['type']);
        case 'voidInline':
            return (
                hasOnlyOwnKeys(value, ['type', 'nodeType', 'docPos', 'attrs']) &&
                typeof value.nodeType === 'string' &&
                nativeEditorV2U32(value.docPos) != null &&
                (value.attrs === undefined || isPlainRecord(value.attrs))
            );
        case 'voidBlock':
            return (
                hasOnlyOwnKeys(value, ['type', 'nodeType', 'docPos', 'attrs', 'atomId']) &&
                typeof value.nodeType === 'string' &&
                nativeEditorV2U32(value.docPos) != null &&
                (value.attrs === undefined || isPlainRecord(value.attrs)) &&
                (value.atomId === undefined || typeof value.atomId === 'string')
            );
        case 'opaqueInlineAtom':
            return (
                hasOnlyOwnKeys(value, [
                    'type',
                    'nodeType',
                    'label',
                    'docPos',
                    'attrs',
                    'mentionTheme',
                ]) &&
                typeof value.nodeType === 'string' &&
                typeof value.label === 'string' &&
                nativeEditorV2U32(value.docPos) != null &&
                (value.attrs === undefined || isPlainRecord(value.attrs)) &&
                (value.mentionTheme === undefined || validEditorMentionTheme(value.mentionTheme))
            );
        case 'opaqueBlockAtom':
            return (
                hasOnlyOwnKeys(value, ['type', 'nodeType', 'label', 'docPos', 'attrs']) &&
                typeof value.nodeType === 'string' &&
                typeof value.label === 'string' &&
                nativeEditorV2U32(value.docPos) != null &&
                (value.attrs === undefined || isPlainRecord(value.attrs))
            );
    }
    return false;
}

function normalizeRenderBlocks(value: unknown): RenderElement[][] | null {
    if (!Array.isArray(value)) return null;
    return value.every(
        (block) => Array.isArray(block) && block.every((element) => validRenderElement(element))
    )
        ? (value as RenderElement[][])
        : null;
}

function normalizeRenderPatch(value: unknown): RenderBlocksPatch | null | undefined {
    if (value === null) return null;
    if (
        !isPlainRecord(value) ||
        !hasExactOwnKeys(value, [
            'baseDocumentVersion',
            'startIndex',
            'deleteCount',
            'renderBlocks',
        ])
    ) {
        return undefined;
    }
    const renderBlocks = normalizeRenderBlocks(value.renderBlocks);
    const baseDocumentVersion = normalizeNativeEditorV2DecimalId(value.baseDocumentVersion);
    const startIndex = nativeEditorV2U32(value.startIndex);
    const deleteCount = nativeEditorV2U32(value.deleteCount);
    if (
        renderBlocks == null ||
        baseDocumentVersion == null ||
        startIndex == null ||
        deleteCount == null
    ) {
        return undefined;
    }
    return { baseDocumentVersion, startIndex, deleteCount, renderBlocks };
}

function normalizeRenderSelection(value: unknown): Selection | null {
    if (!isPlainRecord(value)) return null;
    if (value.type === 'all') return hasExactOwnKeys(value, ['type']) ? { type: 'all' } : null;
    if (value.type === 'text') {
        if (!hasExactOwnKeys(value, ['type', 'anchor', 'head', 'anchorScalar', 'headScalar'])) {
            return null;
        }
        const anchor = nativeEditorV2U32(value.anchor);
        const head = nativeEditorV2U32(value.head);
        const anchorScalar = nativeEditorV2U32(value.anchorScalar);
        const headScalar = nativeEditorV2U32(value.headScalar);
        if (anchor == null || head == null || anchorScalar == null || headScalar == null)
            return null;
        return { type: 'text', anchor, head, anchorScalar, headScalar };
    }
    if (value.type === 'node') {
        if (!hasExactOwnKeys(value, ['type', 'pos', 'posScalar'])) return null;
        const pos = nativeEditorV2U32(value.pos);
        const posScalar = nativeEditorV2U32(value.posScalar);
        if (pos == null || posScalar == null) return null;
        return { type: 'node', pos, posScalar };
    }
    return null;
}

function normalizeRenderActiveState(value: unknown): ActiveState | null {
    if (!isPlainRecord(value)) return null;
    if (
        !hasExactOwnKeys(value, [
            'marks',
            'markAttrs',
            'nodes',
            'commands',
            'allowedMarks',
            'insertableNodes',
        ]) ||
        !booleanRecord(value.marks) ||
        !isPlainRecord(value.markAttrs) ||
        !Object.values(value.markAttrs).every(isPlainRecord) ||
        !booleanRecord(value.nodes) ||
        !booleanRecord(value.commands) ||
        !stringArray(value.allowedMarks) ||
        !stringArray(value.insertableNodes)
    ) {
        return null;
    }
    return value as unknown as ActiveState;
}

function normalizeRenderHistoryState(value: unknown): HistoryState | null {
    if (!isPlainRecord(value) || !hasExactOwnKeys(value, ['canUndo', 'canRedo'])) return null;
    const canUndo = optionalBoolean(value.canUndo);
    const canRedo = optionalBoolean(value.canRedo);
    return canUndo == null || canRedo == null ? null : { canUndo, canRedo };
}

function deepFreezeV2Value<T>(value: T): T {
    if (value != null && typeof value === 'object' && !Object.isFrozen(value)) {
        for (const child of Object.values(value as Record<string, unknown>)) {
            deepFreezeV2Value(child);
        }
        Object.freeze(value);
    }
    return value;
}

/** Validate and freeze the one complete render/state snapshot. */
export function normalizeNativeEditorV2RenderUpdateValue(
    value: unknown
): NativeEditorV2AtomicRenderSnapshot | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    if (
        !hasExactOwnKeys(parsed, [
            'renderBlocks',
            'renderPatch',
            'selection',
            'activeState',
            'historyState',
            'documentVersion',
            'stateRevision',
            'scalarLength',
            'documentIsEmpty',
        ])
    ) {
        return null;
    }
    const renderBlocks =
        parsed.renderBlocks === null ? null : normalizeRenderBlocks(parsed.renderBlocks);
    const renderPatch = normalizeRenderPatch(parsed.renderPatch);
    const selection = normalizeRenderSelection(parsed.selection);
    const activeState = normalizeRenderActiveState(parsed.activeState);
    const historyState = normalizeRenderHistoryState(parsed.historyState);
    const documentVersion = normalizeRevisionField(parsed, 'documentVersion');
    const stateRevision = normalizeRevisionField(parsed, 'stateRevision');
    const scalarLength = nativeEditorV2U32(parsed.scalarLength);
    const documentIsEmpty = parsed.documentIsEmpty;
    if (
        renderPatch === undefined ||
        selection == null ||
        activeState == null ||
        historyState == null ||
        documentVersion == null ||
        stateRevision == null ||
        scalarLength == null ||
        typeof documentIsEmpty !== 'boolean'
    ) {
        return null;
    }
    let renderPayload: NativeEditorV2AtomicRenderPayload;
    if (renderBlocks == null) {
        if (parsed.renderBlocks !== null || renderPatch == null) return null;
        renderPayload = { renderBlocks: null, renderPatch };
    } else {
        if (renderPatch !== null) return null;
        renderPayload = { renderBlocks, renderPatch: null };
    }
    return deepFreezeV2Value({
        ...renderPayload,
        selection,
        activeState,
        historyState,
        documentVersion,
        stateRevision,
        scalarLength,
        documentIsEmpty,
    });
}

export function normalizeNativeEditorV2DocumentJsonValue(value: unknown): DocumentJSON | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    return isPlainRecord(parsed) ? (parsed as DocumentJSON) : null;
}

export function normalizeNativeEditorV2ContentSnapshotValue(
    value: unknown
): ContentSnapshot | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed) || typeof parsed.html !== 'string' || !isPlainRecord(parsed.json)) {
        return null;
    }
    return { html: parsed.html, json: parsed.json as DocumentJSON };
}

export interface NativeEditorV2SnapshotExport {
    metadataJson: string;
    encodedState: Uint8Array;
}

/** The snapshot export record arrives as direct fields (JSON + bytes), not a JSON string. */
export function normalizeNativeEditorV2SnapshotExportValue(
    value: unknown
): NativeEditorV2SnapshotExport | null {
    if (!isPlainRecord(value) || typeof value.metadataJson !== 'string') return null;
    const encodedState = normalizeNativeEditorV2Bytes(value.encodedState);
    if (encodedState == null) return null;
    return { metadataJson: value.metadataJson, encodedState };
}

export function normalizeNativeEditorV2CreateValue(value: unknown): { editorId: string } | null {
    const parsed = parseNativeEditorV2JsonValue(value);
    if (!isPlainRecord(parsed)) return null;
    const editorId = normalizeNativeEditorV2DecimalId(parsed.editorId);
    return editorId == null ? null : { editorId };
}

/** One peer's awareness record, as Rust currently holds it. */
export interface NativeEditorV2PeerInfo {
    /** Yjs client identity, as a decimal string. */
    clientId: string;
    /** Awareness clock, advancing with each of this peer's updates. */
    clock: number;
    /** Whether this record describes this device. */
    isLocal: boolean;
    /** The peer's published application state, e.g. `{ user, focused }`. */
    state: Record<string, unknown> | null;
    /** The peer's caret in engine document positions, or null when it published none. */
    cursor: { anchor: number; head: number } | null;
}

/** One WebSocket frame exchanged during the protocol adapter's prelude. */
export type NativeCollaborationProtocolFrame =
    | { type: 'text'; data: string }
    | { type: 'binary'; data: Uint8Array };

/**
 * What the transport does next after an adapter callback:
 *
 * - `continue` — stay in the prelude and await the next frame.
 * - `ready` — the prelude succeeded; release Yjs traffic.
 * - `reject` — abandon this attempt.
 */
export type NativeCollaborationProtocolAdapterAction = 'continue' | 'ready' | 'reject';

/** An adapter callback's decision, plus any frames to send before it takes effect. */
export interface NativeCollaborationProtocolAdapterResult {
    action: NativeCollaborationProtocolAdapterAction;
    /** Frames sent on the socket before the action applies. */
    frames?: readonly NativeCollaborationProtocolFrame[];
}

/** Identifies the connection attempt an adapter callback is running for. */
export interface NativeCollaborationProtocolAdapterContext {
    /** Identity of this physical connection attempt. */
    attemptId: string;
    /** Transport generation, as a decimal string. Advances on each reconfiguration. */
    generation: string;
    /** Subprotocol the server selected, if any. */
    negotiatedProtocol: string | null;
}

/**
 * An RN-owned, attempt-scoped prelude for a native-owned WebSocket.
 *
 * Native blocks all Yjs traffic until `onMessage` returns `ready`. Callbacks
 * run once per native event and can read fresh credentials on every physical
 * reconnect. Their frames and return values are never persisted by native.
 */
export interface NativeCollaborationProtocolAdapter {
    /** WebSocket subprotocols offered during the physical handshake, in preference order. */
    protocols: readonly string[];
    /** Maximum time native permits the adapter to keep a physical socket pending. */
    timeoutMillis?: number;
    /** Close codes that park the Rust transport instead of entering automatic retry. */
    terminalCloseCodes?: readonly number[];
    /** Runs once when the socket opens — send credentials here. */
    onOpen(
        context: NativeCollaborationProtocolAdapterContext
    ): NativeCollaborationProtocolAdapterResult | Promise<NativeCollaborationProtocolAdapterResult>;
    /** Runs for each frame received before the prelude returns `ready`. */
    onMessage(
        context: NativeCollaborationProtocolAdapterContext,
        frame: NativeCollaborationProtocolFrame
    ): NativeCollaborationProtocolAdapterResult | Promise<NativeCollaborationProtocolAdapterResult>;
}

/** Where the native transport connects, and whether it should. */
export interface NativeCollaborationTransportConfig {
    /** WebSocket endpoint, `ws:` or `wss:`. Treat it as sensitive — it may carry credentials. */
    url: string;
    /** Whether to open the connection now. False configures the endpoint without connecting. */
    connect: boolean;
    /** Optional RN-owned prelude that gates the native Yjs transport. */
    protocolAdapter?: NativeCollaborationProtocolAdapter;
}

/** Why the transport woke and what it did — diagnostic only; no contract depends on these. */
export interface NativeCollaborationTransportDiagnostics {
    /** What woke the transport, e.g. a received message or an elapsed timer. */
    wakeReason: string;
    transportState: NativeEditorV2TransportState;
    /** Decimal-string deadline for the next scheduled wake, if one is pending. */
    nextDeadlineMillis: string | null;
    /** Whether this wake applied a remote commit. */
    remoteCommitApplied: boolean;
    /** Whether the peer list changed. */
    peersChanged: boolean;
    /** Whether the local awareness entry was republished. */
    renewedLocal: boolean;
    /** Peers dropped for having gone stale. */
    expiredPeerCount: number;
}

/** The transport advanced: new engine state, peers, and diagnostics. */
export interface NativeCollaborationTransportStateEvent {
    editorId: string;
    /** Monotonic decimal-string sequence across all events for this handle. */
    eventSequence: string;
    generation: string | null;
    kind: 'state';
    state: NativeEditorV2EditorState;
    peers: NativeEditorV2PeerInfo[];
    diagnostics: NativeCollaborationTransportDiagnostics;
}

/** The transport failed. Retryable failures are followed by further state events. */
export interface NativeCollaborationTransportErrorEvent {
    editorId: string;
    eventSequence: string;
    generation: string | null;
    kind: 'error';
    error: NativeEditorV2ErrorBase;
}

/** Native is asking the configured protocol adapter to handle a prelude step. */
export interface NativeCollaborationProtocolAdapterEvent {
    editorId: string;
    eventSequence: string;
    generation: string;
    kind: 'protocolAdapter';
    /** Identity of the physical connection attempt. */
    attemptId: string;
    /** Identity of this callback invocation; the reply must quote it. */
    eventId: string;
    negotiatedProtocol: string | null;
    /** Which adapter callback this event corresponds to. */
    phase: 'open' | 'message';
    /** The received frame, present when `phase` is `'message'`. */
    frame?: NativeCollaborationProtocolFrame;
}

/** Any event emitted to a `NativeEditorDocumentHandle` transport listener. */
export type NativeCollaborationTransportEvent =
    | NativeCollaborationTransportStateEvent
    | NativeCollaborationTransportErrorEvent
    | NativeCollaborationProtocolAdapterEvent;

interface NativeCollaborationProtocolAdapterDescriptor {
    protocols: string[];
    timeoutMillis?: number;
    terminalCloseCodes?: number[];
}

interface NativeCollaborationTransportWireConfig {
    url: string;
    connect: boolean;
    protocolAdapter?: NativeCollaborationProtocolAdapterDescriptor;
}

const COLLABORATION_PROTOCOL_TOKEN = /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/;
const MAX_COLLABORATION_PROTOCOLS = 16;
const MAX_COLLABORATION_PROTOCOL_BYTES = 128;
const MAX_COLLABORATION_ADAPTER_TIMEOUT_MILLIS = 60_000;
const MAX_COLLABORATION_ADAPTER_FRAMES = 16;
const MAX_COLLABORATION_ADAPTER_FRAME_BYTES = 64 * 1024;

function collaborationProtocolAdapterDescriptor(
    value: NativeCollaborationProtocolAdapter
): NativeCollaborationProtocolAdapterDescriptor {
    if (
        value === null ||
        typeof value !== 'object' ||
        !Array.isArray(value.protocols) ||
        value.protocols.length < 1 ||
        value.protocols.length > MAX_COLLABORATION_PROTOCOLS ||
        typeof value.onOpen !== 'function' ||
        typeof value.onMessage !== 'function'
    ) {
        throw invalidV2RequestError('NativeEditorBridge: invalid collaboration protocol adapter');
    }
    const protocols = value.protocols.map((protocol) => {
        if (
            typeof protocol !== 'string' ||
            protocol.length === 0 ||
            utf8V2JsonByteLength(protocol) > MAX_COLLABORATION_PROTOCOL_BYTES ||
            !COLLABORATION_PROTOCOL_TOKEN.test(protocol)
        ) {
            throw invalidV2RequestError(
                'NativeEditorBridge: invalid collaboration WebSocket subprotocol'
            );
        }
        return protocol;
    });
    if (new Set(protocols).size !== protocols.length) {
        throw invalidV2RequestError(
            'NativeEditorBridge: duplicate collaboration WebSocket subprotocol'
        );
    }
    const timeoutMillis = value.timeoutMillis;
    if (
        timeoutMillis !== undefined &&
        (!Number.isSafeInteger(timeoutMillis) ||
            timeoutMillis < 1 ||
            timeoutMillis > MAX_COLLABORATION_ADAPTER_TIMEOUT_MILLIS)
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: invalid collaboration protocol adapter timeout'
        );
    }
    const rawTerminalCloseCodes = value.terminalCloseCodes;
    if (
        rawTerminalCloseCodes !== undefined &&
        (!Array.isArray(rawTerminalCloseCodes) ||
            rawTerminalCloseCodes.some(
                (code) => !Number.isSafeInteger(code) || code < 1_000 || code > 4_999
            ))
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: invalid collaboration terminal close code'
        );
    }
    const terminalCloseCodes =
        rawTerminalCloseCodes === undefined ? undefined : [...rawTerminalCloseCodes];
    if (
        terminalCloseCodes !== undefined &&
        new Set(terminalCloseCodes).size !== terminalCloseCodes.length
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: duplicate collaboration terminal close code'
        );
    }
    return {
        protocols,
        ...(timeoutMillis === undefined ? {} : { timeoutMillis }),
        ...(terminalCloseCodes === undefined ? {} : { terminalCloseCodes }),
    };
}

function collaborationTransportWireConfig(
    config: NativeCollaborationTransportConfig
): NativeCollaborationTransportWireConfig {
    if (
        config === null ||
        typeof config !== 'object' ||
        typeof config.url !== 'string' ||
        typeof config.connect !== 'boolean'
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: invalid collaboration transport configuration'
        );
    }
    return {
        url: config.url,
        connect: config.connect,
        ...(config.protocolAdapter === undefined
            ? {}
            : {
                  protocolAdapter: collaborationProtocolAdapterDescriptor(config.protocolAdapter),
              }),
    };
}

function serializeCollaborationProtocolAdapterResult(
    value: NativeCollaborationProtocolAdapterResult
): string {
    if (
        value === null ||
        typeof value !== 'object' ||
        (value.action !== 'continue' && value.action !== 'ready' && value.action !== 'reject') ||
        (value.frames !== undefined &&
            (!Array.isArray(value.frames) ||
                value.frames.length > MAX_COLLABORATION_ADAPTER_FRAMES))
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: invalid collaboration protocol adapter result'
        );
    }
    const frames = (value.frames ?? []).map((frame) => {
        if (frame === null || typeof frame !== 'object') {
            throw invalidV2RequestError(
                'NativeEditorBridge: invalid collaboration protocol adapter frame'
            );
        }
        if (frame.type === 'text' && typeof frame.data === 'string') {
            if (utf8V2JsonByteLength(frame.data) > MAX_COLLABORATION_ADAPTER_FRAME_BYTES) {
                throw invalidV2RequestError(
                    'NativeEditorBridge: collaboration protocol adapter frame is too large'
                );
            }
            return { type: 'text' as const, data: frame.data };
        }
        if (
            frame.type === 'binary' &&
            frame.data instanceof Uint8Array &&
            frame.data.byteLength <= MAX_COLLABORATION_ADAPTER_FRAME_BYTES
        ) {
            return {
                type: 'binary' as const,
                data: encodeNativeCollaborationProtocolBytes(frame.data),
            };
        }
        throw invalidV2RequestError(
            'NativeEditorBridge: invalid collaboration protocol adapter frame'
        );
    });
    return JSON.stringify({
        action: value.action,
        ...(frames.length === 0 ? {} : { frames }),
    });
}

function normalizeNativeCollaborationTransportDiagnostics(
    value: unknown
): NativeCollaborationTransportDiagnostics | null {
    if (!isPlainRecord(value)) return null;
    const transportState = whitelisted(value.transportState, V2_TRANSPORT_STATES);
    const nextDeadlineMillis =
        value.nextDeadlineMillis === null
            ? null
            : normalizeNativeEditorV2DecimalId(value.nextDeadlineMillis);
    const remoteCommitApplied = optionalBoolean(value.remoteCommitApplied);
    const peersChanged = optionalBoolean(value.peersChanged);
    const renewedLocal = optionalBoolean(value.renewedLocal);
    const expiredPeerCount = nativeEditorV2U32(value.expiredPeerCount);
    if (
        typeof value.wakeReason !== 'string' ||
        transportState == null ||
        (value.nextDeadlineMillis !== null && nextDeadlineMillis == null) ||
        remoteCommitApplied == null ||
        peersChanged == null ||
        renewedLocal == null ||
        expiredPeerCount == null
    ) {
        return null;
    }
    return {
        wakeReason: value.wakeReason,
        transportState,
        nextDeadlineMillis,
        remoteCommitApplied,
        peersChanged,
        renewedLocal,
        expiredPeerCount,
    };
}

const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

function encodeNativeCollaborationProtocolBytes(bytes: Uint8Array): string {
    let encoded = '';
    for (let index = 0; index < bytes.length; index += 3) {
        const first = bytes[index];
        const second = index + 1 < bytes.length ? bytes[index + 1] : 0;
        const third = index + 2 < bytes.length ? bytes[index + 2] : 0;
        const value = (first << 16) | (second << 8) | third;
        encoded += BASE64_ALPHABET[(value >>> 18) & 63];
        encoded += BASE64_ALPHABET[(value >>> 12) & 63];
        encoded += index + 1 < bytes.length ? BASE64_ALPHABET[(value >>> 6) & 63] : '=';
        encoded += index + 2 < bytes.length ? BASE64_ALPHABET[value & 63] : '=';
    }
    return encoded;
}

function decodeNativeCollaborationProtocolBytes(value: string): Uint8Array | null {
    if (value.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(value)) return null;
    const firstPadding = value.indexOf('=');
    if (firstPadding >= 0 && firstPadding < value.length - 2) return null;
    const outputLength =
        (value.length / 4) * 3 - (value.endsWith('==') ? 2 : value.endsWith('=') ? 1 : 0);
    const bytes = new Uint8Array(outputLength);
    let outputIndex = 0;
    for (let index = 0; index < value.length; index += 4) {
        const first = BASE64_ALPHABET.indexOf(value[index]);
        const second = BASE64_ALPHABET.indexOf(value[index + 1]);
        const third = value[index + 2] === '=' ? 0 : BASE64_ALPHABET.indexOf(value[index + 2]);
        const fourth = value[index + 3] === '=' ? 0 : BASE64_ALPHABET.indexOf(value[index + 3]);
        if (first < 0 || second < 0 || third < 0 || fourth < 0) return null;
        const decoded = (first << 18) | (second << 12) | (third << 6) | fourth;
        if (outputIndex < outputLength) bytes[outputIndex++] = (decoded >>> 16) & 0xff;
        if (outputIndex < outputLength) bytes[outputIndex++] = (decoded >>> 8) & 0xff;
        if (outputIndex < outputLength) bytes[outputIndex++] = decoded & 0xff;
    }
    return bytes;
}

export function normalizeNativeCollaborationTransportEvent(
    value: unknown
): NativeCollaborationTransportEvent | null {
    if (!isPlainRecord(value)) return null;
    const editorId = normalizeNativeEditorV2DecimalId(value.editorId);
    const eventSequence = normalizeNativeEditorV2DecimalId(value.eventSequence);
    const generation =
        value.generation === null ? null : normalizeNativeEditorV2DecimalId(value.generation);
    if (
        editorId == null ||
        editorId === '0' ||
        eventSequence == null ||
        eventSequence === '0' ||
        (value.generation !== null && generation == null)
    ) {
        return null;
    }
    if (value.kind === 'state') {
        const state = normalizeNativeEditorV2StateValue(value.state);
        const peers = normalizeNativeEditorV2PeersValue({ peers: value.peers });
        const diagnostics = normalizeNativeCollaborationTransportDiagnostics(value.diagnostics);
        if (state == null || peers == null || diagnostics == null) return null;
        return {
            editorId,
            eventSequence,
            generation,
            kind: 'state',
            state,
            peers,
            diagnostics,
        };
    }
    if (value.kind === 'error') {
        const error = normalizeNativeEditorV2Error({ error: value.error });
        if (error == null) return null;
        return {
            editorId,
            eventSequence,
            generation,
            kind: 'error',
            error: nativeEditorV2ErrorToException(error),
        };
    }
    if (value.kind === 'protocolAdapter') {
        const eventId = normalizeNativeEditorV2DecimalId(value.eventId);
        if (
            generation == null ||
            typeof value.attemptId !== 'string' ||
            value.attemptId.length === 0 ||
            eventId == null ||
            eventId === '0' ||
            (value.negotiatedProtocol !== null && typeof value.negotiatedProtocol !== 'string') ||
            (value.phase !== 'open' && value.phase !== 'message')
        ) {
            return null;
        }
        const base = {
            editorId,
            eventSequence,
            generation,
            kind: 'protocolAdapter' as const,
            attemptId: value.attemptId,
            eventId,
            negotiatedProtocol: value.negotiatedProtocol as string | null,
        };
        if (value.phase === 'open') {
            if (value.frame !== undefined) return null;
            return { ...base, phase: 'open' };
        }
        if (!isPlainRecord(value.frame) || typeof value.frame.data !== 'string') return null;
        if (value.frame.type === 'text') {
            return {
                ...base,
                phase: 'message',
                frame: { type: 'text', data: value.frame.data },
            };
        }
        if (value.frame.type === 'binary') {
            const data = decodeNativeCollaborationProtocolBytes(value.frame.data);
            if (data == null) return null;
            return {
                ...base,
                phase: 'message',
                frame: { type: 'binary', data },
            };
        }
    }
    return null;
}

export function normalizeNativeEditorV2PeersValue(value: unknown): NativeEditorV2PeerInfo[] | null {
    const parsed = typeof value === 'string' ? parseNativeEditorV2JsonValue(value) : value;
    if (!isPlainRecord(parsed) || !Array.isArray(parsed.peers)) return null;
    const peers: NativeEditorV2PeerInfo[] = [];
    for (const rawPeer of parsed.peers) {
        if (!isPlainRecord(rawPeer)) return null;
        const clientId = normalizeNativeEditorV2DecimalId(rawPeer.clientId);
        const clock = nativeEditorV2U32(rawPeer.clock);
        const isLocal = optionalBoolean(rawPeer.isLocal);
        if (clientId == null || clock == null || isLocal == null) return null;
        if (rawPeer.state !== null && !isPlainRecord(rawPeer.state)) return null;
        let cursor: NativeEditorV2PeerInfo['cursor'] = null;
        if (rawPeer.cursor !== null && rawPeer.cursor !== undefined) {
            if (!isPlainRecord(rawPeer.cursor)) return null;
            const anchor = nativeEditorV2U32(rawPeer.cursor.anchor);
            const head = nativeEditorV2U32(rawPeer.cursor.head);
            if (anchor == null || head == null) return null;
            cursor = { anchor, head };
        }
        peers.push({
            clientId,
            clock,
            isLocal,
            state: (rawPeer.state as Record<string, unknown> | null) ?? null,
            cursor,
        });
    }
    return peers;
}

const REJECTED_V2_RECORD_PREVIEW_CHARS = 4_000;

/** Dev-only preview of a rejected boundary record, bounded so a large document never floods the log. */
function describeRejectedV2Record(raw: unknown): string {
    let serialized: string;
    try {
        serialized = JSON.stringify(raw);
    } catch {
        return `<unserializable ${typeof raw}>`;
    }
    if (serialized === undefined) return `<${typeof raw}>`;
    return serialized.length > REJECTED_V2_RECORD_PREVIEW_CHARS
        ? `${serialized.slice(0, REJECTED_V2_RECORD_PREVIEW_CHARS)}… (${serialized.length} chars)`
        : serialized;
}

function invalidV2ResultError(): NativeEditorV2NonRetryableError {
    return new NativeEditorV2NonRetryableError({
        domain: 'boundary',
        code: 'FFI_RESULT_INVALID',
        message: ERR_V2_NATIVE_RESPONSE,
        requestId: null,
        operationIndex: null,
        limit: null,
        actual: null,
        details: null,
    });
}

function destroyedHandleError(): NativeEditorV2NonRetryableError {
    return new NativeEditorV2NonRetryableError({
        domain: 'lifecycle',
        code: 'ENGINE_DESTROYED',
        message: ERR_V2_DESTROYED,
        requestId: null,
        operationIndex: null,
        limit: null,
        actual: null,
        details: null,
    });
}

function invalidV2RequestError(message: string): NativeEditorV2BoundaryError {
    return new NativeEditorV2BoundaryError({
        domain: 'boundary',
        code: 'CONFIG_INVALID',
        message,
        requestId: null,
        operationIndex: null,
        limit: null,
        actual: null,
        details: null,
    });
}

const LOCAL_AWARENESS_INTENT_KEYS = new Set(['state', 'focused', 'selection']);
const LOCAL_AWARENESS_SELECTION_VALUES = new WeakMap<
    object,
    Readonly<{ anchor: number; head: number }>
>();

function invalidLocalAwarenessIntent(message = 'invalid local awareness intent'): never {
    throw invalidV2RequestError(`NativeEditorBridge: ${message}`);
}

/**
 * Create the only caller-owned local-awareness selection accepted at the
 * JavaScript-to-native boundary. The private WeakMap makes provenance an API
 * capability instead of a structural object-shape check.
 */
export function createNativeEditorLocalAwarenessSelection(
    anchor: number,
    head: number
): NativeEditorLocalAwarenessSelection {
    const acceptedAnchor = nativeEditorV2U32(anchor);
    const acceptedHead = nativeEditorV2U32(head);
    if (acceptedAnchor == null || acceptedHead == null) invalidLocalAwarenessIntent();

    const selection: NativeEditorLocalAwarenessSelection =
        new NativeEditorLocalAwarenessSelectionValue(acceptedAnchor, acceptedHead);
    Object.freeze(selection);
    LOCAL_AWARENESS_SELECTION_VALUES.set(selection, selection);
    return selection;
}

function isLocalAwarenessRecord(value: unknown): value is Record<string, unknown> {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return false;
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
}

function localAwarenessOwnDataValue(record: Record<string, unknown>, key: string): unknown {
    const descriptor = Object.getOwnPropertyDescriptor(record, key);
    if (descriptor === undefined || !('value' in descriptor) || descriptor.enumerable !== true) {
        invalidLocalAwarenessIntent();
    }
    return descriptor.value;
}

function validateLocalAwarenessSelection(selection: unknown): { anchor: number; head: number } {
    if (selection == null || typeof selection !== 'object') invalidLocalAwarenessIntent();

    // Do not inspect caller-held values before provenance succeeds: a Proxy
    // can imitate every structural and own-data check, but it cannot inherit
    // this module's WeakMap identity.
    const factoryValue = LOCAL_AWARENESS_SELECTION_VALUES.get(selection);
    if (factoryValue === undefined) invalidLocalAwarenessIntent();

    const anchor = nativeEditorV2U32(factoryValue.anchor);
    const head = nativeEditorV2U32(factoryValue.head);
    if (anchor == null || head == null) {
        invalidLocalAwarenessIntent();
    }
    return { anchor, head };
}

/** Reject caller-owned sticky cursor data before a native call can occur. */
function rejectReservedAwarenessCursor(value: unknown): void {
    const pending: unknown[] = [value];
    const seen = new WeakSet<object>();
    while (pending.length > 0) {
        const current = pending.pop();
        if (current == null || typeof current !== 'object') continue;
        if (seen.has(current)) continue;
        seen.add(current);
        for (const key of Reflect.ownKeys(current)) {
            if (key === 'cursor') invalidLocalAwarenessIntent('reserved cursor key is not allowed');
            const descriptor = Object.getOwnPropertyDescriptor(current, key);
            if (descriptor == null || !('value' in descriptor)) invalidLocalAwarenessIntent();
            pending.push(descriptor.value);
        }
    }
}

function normalizeLocalAwarenessState(value: unknown): Record<string, unknown> {
    try {
        const normalized = normalizeV2JsonValue(value, 'local awareness state', {
            seen: new WeakSet<object>(),
            work: 0,
        });
        if (!isLocalAwarenessRecord(normalized) || Object.getPrototypeOf(normalized) !== null) {
            invalidLocalAwarenessIntent();
        }
        rejectReservedAwarenessCursor(normalized);
        return normalized;
    } catch (error) {
        if (error instanceof NativeEditorV2BoundaryError) throw error;
        invalidLocalAwarenessIntent();
    }
}

interface NativeEditorLocalAwarenessWireIntent {
    state: Record<string, unknown>;
    focused: boolean;
    /** Absent retains the Rust-owned cursor; `null` clears it. */
    selection?: { type: 'text'; anchor: number; head: number } | null;
}

function validateLocalAwarenessIntent(intent: unknown): NativeEditorLocalAwarenessWireIntent {
    try {
        if (
            !isLocalAwarenessRecord(intent) ||
            Reflect.ownKeys(intent).some(
                (key) => typeof key !== 'string' || !LOCAL_AWARENESS_INTENT_KEYS.has(key)
            ) ||
            !Object.prototype.hasOwnProperty.call(intent, 'state') ||
            !Object.prototype.hasOwnProperty.call(intent, 'focused')
        ) {
            invalidLocalAwarenessIntent();
        }
        const state = normalizeLocalAwarenessState(localAwarenessOwnDataValue(intent, 'state'));
        const focused = localAwarenessOwnDataValue(intent, 'focused');
        if (typeof focused !== 'boolean') invalidLocalAwarenessIntent();

        if (!Object.prototype.hasOwnProperty.call(intent, 'selection')) {
            // Absent: retain whatever cursor Rust already holds.
            return { state, focused };
        }
        const rawSelection = localAwarenessOwnDataValue(intent, 'selection');
        if (rawSelection === null) return { state, focused, selection: null };
        const selection = validateLocalAwarenessSelection(rawSelection);
        return { state, focused, selection: { type: 'text', ...selection } };
    } catch (error) {
        if (error instanceof NativeEditorV2BoundaryError) throw error;
        invalidLocalAwarenessIntent();
    }
}

function serializeLocalAwarenessIntent(intent: NativeEditorLocalAwarenessWireIntent): string {
    try {
        const wire = Object.create(null) as Record<string, unknown>;
        wire.state = intent.state;
        wire.focused = intent.focused;
        if (intent.selection === null) {
            wire.selection = null;
        } else if (intent.selection !== undefined) {
            const selection = Object.create(null) as Record<string, unknown>;
            selection.type = intent.selection.type;
            selection.anchor = intent.selection.anchor;
            selection.head = intent.selection.head;
            wire.selection = selection;
        }
        return serializeV2CreateEnvelope(wire);
    } catch {
        invalidLocalAwarenessIntent();
    }
}

/**
 * Unwrap a raw v2 result record on the imperative path: typed per-domain
 * throws for recoverable engine errors, the distinct non-retryable class for
 * ENGINE_INVARIANT_FAILED / lifecycle-destroyed states and for malformed
 * records (a bridge contract violation can never succeed on retry).
 */
export function unwrapNativeEditorV2Result<T>(
    raw: unknown,
    normalizeValue: (value: unknown) => T | null
): T {
    const result = normalizeNativeEditorV2Result(raw, normalizeValue);
    if (result == null) {
        // The thrown error cannot carry the payload, so name the rejected
        // record here — otherwise the failure is unattributable in the app.
        if (__DEV__) {
            console.error(
                'NativeEditorBridge: native module returned a record this boundary rejected',
                describeRejectedV2Record(raw)
            );
        }
        throw invalidV2ResultError();
    }
    if (!result.ok) throw nativeEditorV2ErrorToException(result.error);
    return result.value;
}

function requireV2DecimalId(value: string, field: string): string {
    const normalized = normalizeNativeEditorV2DecimalId(value);
    if (normalized == null) {
        throw invalidV2RequestError(`NativeEditorBridge: invalid ${field} for v2 request`);
    }
    return normalized;
}

function requireV2Bytes(value: unknown, field: string): Uint8Array {
    const normalized = normalizeNativeEditorV2Bytes(value);
    if (normalized == null) {
        throw invalidV2RequestError(`NativeEditorBridge: invalid ${field} for v2 request`);
    }
    return normalized;
}

export type NativeEditorV2HistoryMode = 'undoableBoundary' | 'resetAndClear';

/**
 * Provenance of an exported room snapshot. A snapshot only restores into a
 * handle whose document, lineage, fragment, and schema match.
 */
export interface NativeEditorV2SnapshotMetadata {
    /** Snapshot format version, so older exports can be recognized. */
    formatVersion: number;
    documentId: string;
    lineageId: string;
    fragmentName: string;
    /** Digest of the schema the snapshot was taken under. */
    schemaFingerprint: string;
}

/** An exported room document: its provenance plus the encoded Yjs state. */
export interface NativeEditorV2RoomSnapshot {
    metadata: NativeEditorV2SnapshotMetadata;
    /** Encoded Yjs state. Bounded by `EditorResourceLimits.maxEncodedStateBytes`. */
    encodedState: Uint8Array;
}

/**
 * How a document handle's content starts out. Fixed at creation — there is no
 * prop equivalent.
 *
 * - `localEmpty` — the schema's empty document.
 * - `localJson` / `localHtml` — seeded from the given content.
 * - `room` — a collaborative document. It renders nothing until the server's
 *   document arrives, unless a `snapshot` seeds it offline.
 */
export type NativeEditorV2Initialization =
    | { type: 'localEmpty' }
    | { type: 'localJson'; json: DocumentJSON }
    | { type: 'localHtml'; html: string }
    | {
          type: 'room';
          /** Room identity. Must match the collaboration controller's `documentId`. */
          documentId: string;
          /** Application-owned lineage tag, e.g. `` `my-app|${documentId}` ``. A
           *  snapshot only restores into a handle declaring the same lineage. */
          lineageId: string;
          /** Previously exported state, for offline restore. */
          snapshot?: NativeEditorV2RoomSnapshot;
      };

/**
 * Everything fixed for the lifetime of a document handle. Initialization,
 * schema, editing policy, and limits all live here rather than on the view,
 * so an editor and its collaboration controller cannot disagree about them.
 */
export interface NativeEditorV2CreateConfig {
    initialization: NativeEditorV2Initialization;
    /** Node and mark types this document admits. Defaults to `defaultSchema`. */
    schema?: SchemaDefinition;
    /** Yjs fragment the document lives in. Defaults to `'prosemirror'`. */
    fragmentName?: string;
    /** Engine-enforced editing policy. `RichTextEditor.editable` is a separate, per-view interaction gate. */
    policy?: {
        /** Maximum document text length in Unicode scalars. A change that would exceed it is rejected. */
        maxLength?: number;
        /** Whether the engine refuses every mutation, raising `MUTATION_REJECTED`. */
        readOnly?: boolean;
        /** Regular expression applied per character to typed input; characters that do not match are dropped. */
        inputFilter?: string;
        /** Whether `data:` image sources are admitted. Defaults to false. */
        allowBase64Images?: boolean;
    };
    /** Resource bounds. See {@link EditorResourceLimits}. */
    limits?: {
        resource?: EditorResourceLimits;
        editing?: EditorEditingLimits;
        collaboration?: EditorCollaborationLimits;
    };
}

export interface NativeEditorV2InputRequest {
    baseDocumentRevision: string;
    text: string;
}

export interface NativeEditorV2CommandRequest {
    baseDocumentRevision: string;
    command: Record<string, unknown>;
}

export interface NativeEditorV2LocalApiRequest {
    baseDocumentRevision: string;
    setJson?: DocumentJSON;
    setHtml?: string;
    history: NativeEditorV2HistoryMode;
}

export type NativeEditorV2OffsetKind = 'scalar' | 'utf16';

export type NativeEditorV2PositionAffinity = 'before' | 'after';

/**
 * Mirrors the Rust `PositionEnvelope`. `offset` is measured in the currency
 * named by `kind` — scalar offsets are Unicode scalars, not document positions.
 */
export interface NativeEditorV2PositionEnvelope {
    offset: number;
    kind: NativeEditorV2OffsetKind;
    affinity?: NativeEditorV2PositionAffinity;
}

export type NativeEditorV2SelectionEnvelope =
    | {
          type: 'text';
          anchor: NativeEditorV2PositionEnvelope;
          head: NativeEditorV2PositionEnvelope;
      }
    | { type: 'node'; at: NativeEditorV2PositionEnvelope }
    | { type: 'all' };

export interface NativeEditorV2SelectionRequest {
    baseDocumentRevision: string;
    selection: NativeEditorV2SelectionEnvelope;
}

export interface NativeEditorV2ReplaceDocumentRequest {
    setJson?: DocumentJSON;
    setHtml?: string;
    history: NativeEditorV2HistoryMode;
}

const V2_CREATE_CONFIG_KEYS = new Set([
    'initialization',
    'schema',
    'fragmentName',
    'policy',
    'limits',
]);
const V2_CREATE_POLICY_KEYS = new Set([
    'maxLength',
    'readOnly',
    'inputFilter',
    'allowBase64Images',
]);
const V2_CREATE_LIMIT_KEYS = new Set(['resource', 'editing', 'collaboration']);
const V2_CREATE_RESOURCE_LIMIT_KEYS = new Set([
    'maxInputBytes',
    'maxDocumentNodes',
    'maxDocumentDepth',
    'maxSchemaNodes',
    'maxSchemaExpressionBytes',
    'maxCollaborationMessageBytes',
    'maxEncodedStateBytes',
]);
const V2_CREATE_EDITING_LIMIT_KEYS = new Set([
    'maxOperationsPerTransaction',
    'maxUndoGroups',
    'maxUndoRetainedUnits',
    'maxDerivedOutputBytes',
]);
const V2_CREATE_COLLABORATION_LIMIT_KEYS = new Set([
    'maxFramesPerMessage',
    'maxFrameBytes',
    'maxAggregateResponseBytes',
    'maxAwarenessPeers',
    'maxAwarenessPeerBytes',
    'maxAwarenessBytes',
    'maxPendingOutboxMessages',
    'maxPendingOutboxBytes',
    'maxPendingDependencyUpdateBytes',
    'maxPendingDependencyUpdateWork',
]);
const V2_CREATE_INITIALIZATION_KEYS: Readonly<Record<string, ReadonlySet<string>>> = {
    localEmpty: new Set(['type']),
    localJson: new Set(['type', 'json']),
    localHtml: new Set(['type', 'html']),
    room: new Set(['type', 'documentId', 'lineageId', 'snapshot']),
};
const V2_CREATE_ROOM_SNAPSHOT_KEYS = new Set(['metadata', 'encodedState']);
const V2_CREATE_SNAPSHOT_METADATA_KEYS = new Set([
    'formatVersion',
    'documentId',
    'lineageId',
    'fragmentName',
    'schemaFingerprint',
]);
const V2_CREATE_MAX_U32 = 0xffff_ffff;
const V2_CREATE_JSON_MAX_BYTES = HARD_EDITOR_RESOURCE_LIMITS.maxInputBytes;
const V2_CREATE_JSON_MAX_DEPTH = HARD_EDITOR_RESOURCE_LIMITS.maxDocumentDepth * 2 + 16;
const V2_CREATE_JSON_MAX_WORK = HARD_EDITOR_RESOURCE_LIMITS.maxInputBytes;
const V2_CREATE_ENVELOPE_MAX_BYTES = 64 * 1024;
const V2_CREATE_WIRE_MAX_BYTES = V2_CREATE_JSON_MAX_BYTES * 7 + V2_CREATE_ENVELOPE_MAX_BYTES + 2;
const V2_CREATE_ENVELOPE_JSON_MAX_DEPTH = V2_CREATE_JSON_MAX_DEPTH + 8;
const V2_CREATE_JSON_OUTPUT_CHUNK_SIZE = 64 * 1024;
const V2_CREATE_STRING_CHAR_CODE_AT = String.prototype.charCodeAt;
const V2_CREATE_STRING_SLICE = String.prototype.slice;
const V2_CREATE_NUMBER_TO_STRING = Number.prototype.toString;

class NativeEditorV2CreateConfigError extends Error {
    constructor(message: string) {
        super(message);
        this.name = 'NativeEditorV2CreateConfigError';
    }
}

function invalidV2CreateRequestError(message: string): NativeEditorV2CreateConfigError {
    return new NativeEditorV2CreateConfigError(message);
}

function validateV2CreateLimits(limits: NativeEditorV2CreateConfig['limits']): void {
    try {
        validateEditorCreateLimits(limits);
    } catch (error) {
        if (!(error instanceof NativeEditorBoundaryError)) throw error;
        throw new NativeEditorV2BoundaryError({
            domain: 'boundary',
            code: error.code,
            message: error.message,
            requestId: null,
            operationIndex: null,
            limit: error.limit == null ? null : String(error.limit),
            actual: error.actual == null ? null : String(error.actual),
            details: error.details ?? null,
        });
    }
}

function emptyV2CreateRecord(): Record<string, unknown> {
    return Object.create(null) as Record<string, unknown>;
}

function isV2CreateRecord(value: unknown): value is Record<string, unknown> {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return false;
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
}

function hasOwnV2CreateKey(value: Record<string, unknown>, key: string): boolean {
    return Object.prototype.hasOwnProperty.call(value, key);
}

function ownV2CreateValue(value: Record<string, unknown>, key: string): unknown {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    if (descriptor === undefined) return undefined;
    if (!('value' in descriptor)) {
        throw invalidV2CreateRequestError(
            `NativeEditorBridge: accessor ${key} is not allowed for v2 create`
        );
    }
    return descriptor.value;
}

function requireKnownV2CreateKeys(
    value: unknown,
    allowed: ReadonlySet<string>,
    label: string
): asserts value is Record<string, unknown> {
    if (
        !isV2CreateRecord(value) ||
        Reflect.ownKeys(value).some((key) => typeof key !== 'string' || !allowed.has(key))
    ) {
        throw invalidV2CreateRequestError(`NativeEditorBridge: invalid ${label} for v2 create`);
    }
}

function normalizeV2CreateRecord(
    value: Record<string, unknown>,
    allowed: ReadonlySet<string>,
    label: string
): Record<string, unknown> {
    requireKnownV2CreateKeys(value, allowed, label);
    const normalized = emptyV2CreateRecord();
    for (const key of allowed) {
        if (!hasOwnV2CreateKey(value, key)) continue;
        const fieldValue = ownV2CreateValue(value, key);
        if (fieldValue === null) {
            throw invalidV2CreateRequestError(`NativeEditorBridge: invalid ${label} for v2 create`);
        }
        if (fieldValue !== undefined) normalized[key] = fieldValue;
    }
    return normalized;
}

function invalidV2JsonValue(label: string): never {
    throw invalidV2CreateRequestError(`NativeEditorBridge: invalid ${label} for v2 create`);
}

interface V2JsonNormalizationTraversal {
    readonly seen: WeakSet<object>;
    work: number;
}

interface V2JsonNormalizationBudget {
    bytes: number;
}

type V2JsonNormalizationTarget = Record<string, unknown> | unknown[] | null;

interface V2JsonNormalizationValueFrame {
    readonly type: 'value';
    readonly value: unknown;
    readonly depth: number;
    readonly target: V2JsonNormalizationTarget;
    readonly key: string | number | null;
}

interface V2JsonNormalizationArrayFrame {
    readonly type: 'array';
    readonly value: unknown[];
    readonly normalized: unknown[];
    readonly keys: readonly PropertyKey[];
    readonly length: number;
    readonly depth: number;
    readonly nextKeyIndex: number;
    readonly elementCount: number;
}

interface V2JsonNormalizationObjectFrame {
    readonly type: 'object';
    readonly value: Record<string, unknown>;
    readonly normalized: Record<string, unknown>;
    readonly keys: readonly PropertyKey[];
    readonly depth: number;
    readonly nextKeyIndex: number;
    readonly fieldCount: number;
}

type V2JsonNormalizationFrame =
    | V2JsonNormalizationValueFrame
    | V2JsonNormalizationArrayFrame
    | V2JsonNormalizationObjectFrame;

interface V2JsonSerializationState {
    bytes: number;
    work: number;
}

interface V2JsonSerializationValueFrame {
    readonly type: 'value';
    readonly value: unknown;
    readonly depth: number;
}

interface V2JsonSerializationArrayFrame {
    readonly type: 'array';
    readonly value: unknown[];
    readonly length: number;
    readonly depth: number;
    readonly index: number;
}

interface V2JsonSerializationObjectFrame {
    readonly type: 'object';
    readonly value: Record<string, unknown>;
    readonly keys: readonly string[];
    readonly depth: number;
    readonly index: number;
}

type V2JsonSerializationFrame =
    | V2JsonSerializationValueFrame
    | V2JsonSerializationArrayFrame
    | V2JsonSerializationObjectFrame;

function chargeV2JsonWork(state: V2JsonNormalizationTraversal, label: string): void {
    if (state.work >= V2_CREATE_JSON_MAX_WORK) invalidV2JsonValue(label);
    state.work += 1;
}

function chargeV2JsonBytes(budget: V2JsonNormalizationBudget, amount: number, label: string): void {
    if (
        !Number.isSafeInteger(amount) ||
        amount < 0 ||
        amount > V2_CREATE_JSON_MAX_BYTES - budget.bytes
    ) {
        invalidV2JsonValue(label);
    }
    budget.bytes += amount;
}

function chargeV2JsonSerializationWork(state: V2JsonSerializationState, label: string): void {
    if (state.work >= V2_CREATE_WIRE_MAX_BYTES) invalidV2JsonValue(label);
    state.work += 1;
}

function chargeV2JsonSerializationBytes(
    state: V2JsonSerializationState,
    amount: number,
    label: string
): void {
    if (
        !Number.isSafeInteger(amount) ||
        amount < 0 ||
        amount > V2_CREATE_WIRE_MAX_BYTES - state.bytes
    ) {
        invalidV2JsonValue(label);
    }
    state.bytes += amount;
}

function utf8V2JsonByteLength(value: string): number {
    let bytes = 0;
    for (let index = 0; index < value.length; index += 1) {
        const code = V2_CREATE_STRING_CHAR_CODE_AT.call(value, index);
        if (code <= 0x7f) {
            bytes += 1;
        } else if (code <= 0x7ff) {
            bytes += 2;
        } else if (code >= 0xd800 && code <= 0xdbff) {
            const next =
                index + 1 < value.length
                    ? V2_CREATE_STRING_CHAR_CODE_AT.call(value, index + 1)
                    : -1;
            if (next >= 0xdc00 && next <= 0xdfff) {
                bytes += 4;
                index += 1;
            } else {
                bytes += 3;
            }
        } else {
            bytes += 3;
        }
    }
    return bytes;
}

function serializeV2JsonNumber(value: number, label: string): string {
    if (!Number.isFinite(value)) invalidV2JsonValue(label);
    return value === 0 ? '0' : V2_CREATE_NUMBER_TO_STRING.call(value);
}

function chargeV2JsonStringBytes(
    value: string,
    budget: V2JsonNormalizationBudget,
    label: string
): void {
    chargeV2JsonBytes(budget, 2, label);
    for (let index = 0; index < value.length; index += 1) {
        const code = V2_CREATE_STRING_CHAR_CODE_AT.call(value, index);
        if (code === 0x22 || code === 0x5c || code === 0x08 || code === 0x09) {
            chargeV2JsonBytes(budget, 2, label);
        } else if (code === 0x0a || code === 0x0c || code === 0x0d) {
            chargeV2JsonBytes(budget, 2, label);
        } else if (code <= 0x1f) {
            chargeV2JsonBytes(budget, 6, label);
        } else if (code <= 0x7f) {
            chargeV2JsonBytes(budget, 1, label);
        } else if (code <= 0x7ff) {
            chargeV2JsonBytes(budget, 2, label);
        } else if (code >= 0xd800 && code <= 0xdbff) {
            const next =
                index + 1 < value.length
                    ? V2_CREATE_STRING_CHAR_CODE_AT.call(value, index + 1)
                    : -1;
            if (next >= 0xdc00 && next <= 0xdfff) {
                chargeV2JsonBytes(budget, 4, label);
                index += 1;
            } else {
                chargeV2JsonBytes(budget, 6, label);
            }
        } else if (code >= 0xdc00 && code <= 0xdfff) {
            chargeV2JsonBytes(budget, 6, label);
        } else {
            chargeV2JsonBytes(budget, 3, label);
        }
    }
}

function normalizeV2JsonValue(
    value: unknown,
    label: string,
    traversal: V2JsonNormalizationTraversal,
    budget: V2JsonNormalizationBudget = { bytes: 0 }
): unknown {
    let normalizedRoot: unknown;
    const frames: V2JsonNormalizationFrame[] = [
        { type: 'value', value, depth: 0, target: null, key: null },
    ];

    const installNormalizedValue = (
        target: V2JsonNormalizationTarget,
        key: string | number | null,
        normalized: unknown
    ): void => {
        if (target === null) {
            normalizedRoot = normalized;
            return;
        }
        if (key === null) invalidV2JsonValue(label);
        Object.defineProperty(target, key, {
            configurable: true,
            enumerable: true,
            value: normalized,
            writable: true,
        });
    };

    while (frames.length > 0) {
        const frame = frames.pop();
        if (frame === undefined) invalidV2JsonValue(label);

        if (frame.type === 'array') {
            if (frame.nextKeyIndex === frame.keys.length) {
                if (frame.elementCount !== frame.length) invalidV2JsonValue(label);
                Object.setPrototypeOf(frame.normalized, null);
                continue;
            }
            const key = frame.keys[frame.nextKeyIndex];
            if (key === 'length') {
                frames.push({ ...frame, nextKeyIndex: frame.nextKeyIndex + 1 });
                continue;
            }
            if (typeof key !== 'string' || !/^(0|[1-9]\d*)$/.test(key)) {
                invalidV2JsonValue(label);
            }
            const index = Number(key);
            if (
                !Number.isSafeInteger(index) ||
                index !== frame.elementCount ||
                index < 0 ||
                index >= frame.length
            ) {
                invalidV2JsonValue(label);
            }
            const descriptor = Object.getOwnPropertyDescriptor(frame.value, key);
            if (
                descriptor === undefined ||
                !('value' in descriptor) ||
                descriptor.enumerable !== true
            ) {
                invalidV2JsonValue(label);
            }
            if (frame.elementCount > 0) chargeV2JsonBytes(budget, 1, label);
            frames.push({
                ...frame,
                nextKeyIndex: frame.nextKeyIndex + 1,
                elementCount: frame.elementCount + 1,
            });
            frames.push({
                type: 'value',
                value: descriptor.value,
                depth: frame.depth,
                target: frame.normalized,
                key: frame.elementCount,
            });
            continue;
        }

        if (frame.type === 'object') {
            if (frame.nextKeyIndex === frame.keys.length) continue;
            const key = frame.keys[frame.nextKeyIndex];
            if (typeof key !== 'string') invalidV2JsonValue(label);
            const descriptor = Object.getOwnPropertyDescriptor(frame.value, key);
            if (
                descriptor === undefined ||
                !('value' in descriptor) ||
                descriptor.enumerable !== true
            ) {
                invalidV2JsonValue(label);
            }
            if (descriptor.value === undefined) {
                frames.push({
                    ...frame,
                    nextKeyIndex: frame.nextKeyIndex + 1,
                });
                continue;
            }
            if (frame.fieldCount > 0) chargeV2JsonBytes(budget, 1, label);
            chargeV2JsonStringBytes(key, budget, label);
            chargeV2JsonBytes(budget, 1, label);
            frames.push({
                ...frame,
                nextKeyIndex: frame.nextKeyIndex + 1,
                fieldCount: frame.fieldCount + 1,
            });
            frames.push({
                type: 'value',
                value: descriptor.value,
                depth: frame.depth,
                target: frame.normalized,
                key,
            });
            continue;
        }

        if (frame.depth > V2_CREATE_JSON_MAX_DEPTH) invalidV2JsonValue(label);
        chargeV2JsonWork(traversal, label);
        if (frame.value === null) {
            chargeV2JsonBytes(budget, 4, label);
            installNormalizedValue(frame.target, frame.key, frame.value);
            continue;
        }
        if (typeof frame.value === 'string') {
            chargeV2JsonStringBytes(frame.value, budget, label);
            installNormalizedValue(frame.target, frame.key, frame.value);
            continue;
        }
        if (typeof frame.value === 'boolean') {
            chargeV2JsonBytes(budget, frame.value ? 4 : 5, label);
            installNormalizedValue(frame.target, frame.key, frame.value);
            continue;
        }
        if (typeof frame.value === 'number') {
            const serialized = serializeV2JsonNumber(frame.value, label);
            chargeV2JsonBytes(budget, serialized.length, label);
            installNormalizedValue(frame.target, frame.key, frame.value);
            continue;
        }
        if (typeof frame.value !== 'object') invalidV2JsonValue(label);

        if (traversal.seen.has(frame.value)) invalidV2JsonValue(label);
        traversal.seen.add(frame.value);
        chargeV2JsonBytes(budget, 2, label);
        const childDepth = frame.depth + 1;
        if (Array.isArray(frame.value)) {
            const prototype = Object.getPrototypeOf(frame.value);
            if (prototype !== Array.prototype && prototype !== null) invalidV2JsonValue(label);
            const lengthDescriptor = Object.getOwnPropertyDescriptor(frame.value, 'length');
            if (
                lengthDescriptor === undefined ||
                !('value' in lengthDescriptor) ||
                typeof lengthDescriptor.value !== 'number'
            ) {
                invalidV2JsonValue(label);
            }
            const normalized: unknown[] = [];
            installNormalizedValue(frame.target, frame.key, normalized);
            frames.push({
                type: 'array',
                value: frame.value,
                normalized,
                keys: Reflect.ownKeys(frame.value),
                length: lengthDescriptor.value,
                depth: childDepth,
                nextKeyIndex: 0,
                elementCount: 0,
            });
            continue;
        }

        if (!isV2CreateRecord(frame.value)) invalidV2JsonValue(label);
        const normalized = emptyV2CreateRecord();
        installNormalizedValue(frame.target, frame.key, normalized);
        frames.push({
            type: 'object',
            value: frame.value,
            normalized,
            keys: Reflect.ownKeys(frame.value),
            depth: childDepth,
            nextKeyIndex: 0,
            fieldCount: 0,
        });
    }

    if (normalizedRoot === undefined) invalidV2JsonValue(label);
    return normalizedRoot;
}

class V2JsonSerializationWriter {
    private readonly _chunks: string[] = [];
    private _current = '';

    append(value: string, state: V2JsonSerializationState, label: string): void {
        chargeV2JsonSerializationBytes(state, utf8V2JsonByteLength(value), label);
        this._current += value;
        if (this._current.length >= V2_CREATE_JSON_OUTPUT_CHUNK_SIZE) {
            this._chunks.push(this._current);
            this._current = '';
        }
    }

    finish(): string {
        if (this._current.length > 0) this._chunks.push(this._current);
        return this._chunks.join('');
    }
}

function appendV2JsonString(
    writer: V2JsonSerializationWriter,
    value: string,
    state: V2JsonSerializationState,
    label: string
): void {
    writer.append('"', state, label);
    let segmentStart = 0;
    for (let index = 0; index < value.length; index += 1) {
        chargeV2JsonSerializationWork(state, label);
        const code = V2_CREATE_STRING_CHAR_CODE_AT.call(value, index);
        let escape: string | undefined;
        switch (code) {
            case 0x08:
                escape = '\\b';
                break;
            case 0x09:
                escape = '\\t';
                break;
            case 0x0a:
                escape = '\\n';
                break;
            case 0x0c:
                escape = '\\f';
                break;
            case 0x0d:
                escape = '\\r';
                break;
            case 0x22:
                escape = '\\"';
                break;
            case 0x5c:
                escape = '\\\\';
                break;
            default:
                if (code <= 0x1f) {
                    escape = `\\u00${code.toString(16).padStart(2, '0')}`;
                } else if (code >= 0xd800 && code <= 0xdbff) {
                    const next =
                        index + 1 < value.length
                            ? V2_CREATE_STRING_CHAR_CODE_AT.call(value, index + 1)
                            : -1;
                    if (next >= 0xdc00 && next <= 0xdfff) {
                        chargeV2JsonSerializationWork(state, label);
                        index += 1;
                    } else {
                        escape = `\\u${code.toString(16).padStart(4, '0')}`;
                    }
                } else if (code >= 0xdc00 && code <= 0xdfff) {
                    escape = `\\u${code.toString(16).padStart(4, '0')}`;
                }
                break;
        }
        if (escape === undefined) continue;
        if (segmentStart < index) {
            writer.append(V2_CREATE_STRING_SLICE.call(value, segmentStart, index), state, label);
        }
        writer.append(escape, state, label);
        segmentStart = index + 1;
    }
    if (segmentStart < value.length) {
        writer.append(V2_CREATE_STRING_SLICE.call(value, segmentStart), state, label);
    }
    writer.append('"', state, label);
}

function serializeV2CreateEnvelope(value: Record<string, unknown>): string {
    const label = 'v2 create config';
    const writer = new V2JsonSerializationWriter();
    const state: V2JsonSerializationState = { bytes: 0, work: 0 };
    const frames: V2JsonSerializationFrame[] = [{ type: 'value', value, depth: 0 }];

    while (frames.length > 0) {
        const frame = frames.pop();
        if (frame === undefined) invalidV2JsonValue(label);

        if (frame.type === 'array') {
            if (frame.index === frame.length) {
                writer.append(']', state, label);
                continue;
            }
            if (frame.index > 0) writer.append(',', state, label);
            const descriptor = Object.getOwnPropertyDescriptor(frame.value, String(frame.index));
            if (
                descriptor === undefined ||
                !('value' in descriptor) ||
                descriptor.enumerable !== true
            ) {
                invalidV2JsonValue(label);
            }
            frames.push({ ...frame, index: frame.index + 1 });
            frames.push({ type: 'value', value: descriptor.value, depth: frame.depth + 1 });
            continue;
        }

        if (frame.type === 'object') {
            if (frame.index === frame.keys.length) {
                writer.append('}', state, label);
                continue;
            }
            if (frame.index > 0) writer.append(',', state, label);
            const key = frame.keys[frame.index];
            const descriptor = Object.getOwnPropertyDescriptor(frame.value, key);
            if (
                descriptor === undefined ||
                !('value' in descriptor) ||
                descriptor.enumerable !== true
            ) {
                invalidV2JsonValue(label);
            }
            appendV2JsonString(writer, key, state, label);
            writer.append(':', state, label);
            frames.push({ ...frame, index: frame.index + 1 });
            frames.push({ type: 'value', value: descriptor.value, depth: frame.depth + 1 });
            continue;
        }

        if (frame.depth > V2_CREATE_ENVELOPE_JSON_MAX_DEPTH) invalidV2JsonValue(label);
        chargeV2JsonSerializationWork(state, label);
        if (frame.value === null) {
            writer.append('null', state, label);
        } else if (typeof frame.value === 'string') {
            appendV2JsonString(writer, frame.value, state, label);
        } else if (typeof frame.value === 'boolean') {
            writer.append(frame.value ? 'true' : 'false', state, label);
        } else if (typeof frame.value === 'number') {
            writer.append(serializeV2JsonNumber(frame.value, label), state, label);
        } else if (Array.isArray(frame.value)) {
            if (Object.getPrototypeOf(frame.value) !== null) invalidV2JsonValue(label);
            const lengthDescriptor = Object.getOwnPropertyDescriptor(frame.value, 'length');
            if (
                lengthDescriptor === undefined ||
                !('value' in lengthDescriptor) ||
                typeof lengthDescriptor.value !== 'number'
            ) {
                invalidV2JsonValue(label);
            }
            writer.append('[', state, label);
            frames.push({
                type: 'array',
                value: frame.value,
                length: lengthDescriptor.value,
                depth: frame.depth,
                index: 0,
            });
        } else if (isV2CreateRecord(frame.value) && Object.getPrototypeOf(frame.value) === null) {
            const keys = Reflect.ownKeys(frame.value);
            if (keys.some((key) => typeof key !== 'string')) invalidV2JsonValue(label);
            writer.append('{', state, label);
            frames.push({
                type: 'object',
                value: frame.value,
                keys: keys as string[],
                depth: frame.depth,
                index: 0,
            });
        } else {
            invalidV2JsonValue(label);
        }
    }

    return writer.finish();
}

function normalizeV2CreatePolicy(value: Record<string, unknown>): Record<string, unknown> {
    const policy = normalizeV2CreateRecord(value, V2_CREATE_POLICY_KEYS, 'policy');
    const maxLength = ownV2CreateValue(policy, 'maxLength');
    if (
        maxLength !== undefined &&
        (typeof maxLength !== 'number' ||
            !Number.isSafeInteger(maxLength) ||
            maxLength < 0 ||
            maxLength > V2_CREATE_MAX_U32)
    ) {
        invalidV2JsonValue('policy.maxLength');
    }
    for (const key of ['readOnly', 'allowBase64Images']) {
        const value = ownV2CreateValue(policy, key);
        if (value !== undefined && typeof value !== 'boolean') {
            invalidV2JsonValue(`policy.${key}`);
        }
    }
    const inputFilter = ownV2CreateValue(policy, 'inputFilter');
    if (inputFilter !== undefined && typeof inputFilter !== 'string') {
        invalidV2JsonValue('policy.inputFilter');
    }
    return policy;
}

function normalizeV2SnapshotMetadata(value: unknown): Record<string, unknown> {
    const metadata = normalizeV2CreateRecord(
        value as Record<string, unknown>,
        V2_CREATE_SNAPSHOT_METADATA_KEYS,
        'snapshot metadata'
    );
    const formatVersion = ownV2CreateValue(metadata, 'formatVersion');
    if (
        typeof formatVersion !== 'number' ||
        !Number.isSafeInteger(formatVersion) ||
        formatVersion < 0 ||
        formatVersion > V2_CREATE_MAX_U32
    ) {
        invalidV2JsonValue('snapshot metadata.formatVersion');
    }
    for (const key of ['documentId', 'lineageId', 'fragmentName', 'schemaFingerprint']) {
        if (typeof ownV2CreateValue(metadata, key) !== 'string') {
            invalidV2JsonValue(`snapshot metadata.${key}`);
        }
    }
    return metadata;
}

function buildV2CreateRequestUnchecked(config: NativeEditorV2CreateConfig): {
    envelope: Record<string, unknown>;
    limits: NativeEditorV2CreateConfig['limits'];
    snapshotState: Uint8Array | null;
} {
    if (!isV2CreateRecord(config)) {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid v2 create config');
    }
    requireKnownV2CreateKeys(config, V2_CREATE_CONFIG_KEYS, 'config');
    const initializationValue = ownV2CreateValue(config, 'initialization');
    if (!isV2CreateRecord(initializationValue)) {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid v2 create config');
    }
    const jsonTraversal: V2JsonNormalizationTraversal = {
        seen: new WeakSet<object>(),
        work: 0,
    };

    const policyValue = ownV2CreateValue(config, 'policy');
    const policy =
        policyValue === undefined
            ? undefined
            : normalizeV2CreatePolicy(policyValue as Record<string, unknown>);

    const limitsValue = ownV2CreateValue(config, 'limits');
    let limits: Record<string, unknown> | undefined;
    if (limitsValue !== undefined) {
        requireKnownV2CreateKeys(limitsValue, V2_CREATE_LIMIT_KEYS, 'limits');
        limits = emptyV2CreateRecord();
        for (const [group, keys] of [
            ['resource', V2_CREATE_RESOURCE_LIMIT_KEYS],
            ['editing', V2_CREATE_EDITING_LIMIT_KEYS],
            ['collaboration', V2_CREATE_COLLABORATION_LIMIT_KEYS],
        ] as const) {
            const overrides = ownV2CreateValue(limitsValue, group);
            if (overrides !== undefined) {
                limits[group] = normalizeV2CreateRecord(
                    overrides as Record<string, unknown>,
                    keys,
                    `${group} limits`
                );
            }
        }
    }
    const envelope = emptyV2CreateRecord();
    const schema = ownV2CreateValue(config, 'schema');
    if (schema === null) {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid schema for v2 create');
    }
    if (schema !== undefined) {
        if (!isV2CreateRecord(schema)) {
            throw invalidV2CreateRequestError('NativeEditorBridge: invalid schema for v2 create');
        }
        envelope.schema = normalizeV2JsonValue(schema, 'schema', jsonTraversal);
    }
    const fragmentName = ownV2CreateValue(config, 'fragmentName');
    if (fragmentName !== undefined && typeof fragmentName !== 'string') {
        throw invalidV2CreateRequestError('NativeEditorBridge: invalid fragmentName for v2 create');
    }
    if (fragmentName !== undefined) envelope.fragmentName = fragmentName;

    let snapshotState: Uint8Array | null = null;
    const initialization = initializationValue;
    const initializationType = ownV2CreateValue(initialization, 'type');
    const initializationKeys =
        typeof initializationType === 'string'
            ? V2_CREATE_INITIALIZATION_KEYS[initializationType]
            : undefined;
    if (initializationKeys === undefined) {
        throw invalidV2CreateRequestError('NativeEditorBridge: unknown v2 initialization type');
    }
    requireKnownV2CreateKeys(initialization, initializationKeys, 'initialization');
    switch (initializationType) {
        case 'localEmpty': {
            const localEmpty = emptyV2CreateRecord();
            localEmpty.type = 'localEmpty';
            envelope.initialization = localEmpty;
            break;
        }
        case 'localJson': {
            const json = ownV2CreateValue(initialization, 'json');
            if (!isV2CreateRecord(json)) {
                throw invalidV2CreateRequestError(
                    'NativeEditorBridge: invalid localJson initialization for v2 create'
                );
            }
            const localJson = emptyV2CreateRecord();
            localJson.type = 'localJson';
            localJson.json = normalizeV2JsonValue(json, 'localJson initialization', jsonTraversal);
            envelope.initialization = localJson;
            break;
        }
        case 'localHtml': {
            const html = ownV2CreateValue(initialization, 'html');
            if (typeof html !== 'string') {
                throw invalidV2CreateRequestError(
                    'NativeEditorBridge: invalid localHtml initialization for v2 create'
                );
            }
            const localHtml = emptyV2CreateRecord();
            localHtml.type = 'localHtml';
            localHtml.html = html;
            envelope.initialization = localHtml;
            break;
        }
        case 'room': {
            const documentId = ownV2CreateValue(initialization, 'documentId');
            const lineageId = ownV2CreateValue(initialization, 'lineageId');
            if (typeof documentId !== 'string' || typeof lineageId !== 'string') {
                throw invalidV2CreateRequestError(
                    'NativeEditorBridge: invalid room initialization for v2 create'
                );
            }
            const room = emptyV2CreateRecord();
            room.type = 'room';
            room.documentId = documentId;
            room.lineageId = lineageId;
            const snapshot = ownV2CreateValue(initialization, 'snapshot');
            if (snapshot !== undefined) {
                requireKnownV2CreateKeys(snapshot, V2_CREATE_ROOM_SNAPSHOT_KEYS, 'room snapshot');
                const metadataValue = ownV2CreateValue(snapshot, 'metadata');
                const metadata = normalizeV2SnapshotMetadata(metadataValue);
                room.snapshot = metadata;
                snapshotState = requireV2Bytes(
                    ownV2CreateValue(snapshot, 'encodedState'),
                    'snapshot encodedState'
                );
            }
            envelope.initialization = room;
            break;
        }
        default:
            throw invalidV2CreateRequestError('NativeEditorBridge: unknown v2 initialization type');
    }
    if (policy !== undefined) envelope.policy = policy;
    if (limits !== undefined) envelope.limits = limits;
    return {
        envelope,
        limits: limits as NativeEditorV2CreateConfig['limits'],
        snapshotState,
    };
}

function cloneAndFreezeDescriptorValue<T>(value: T): T {
    if (value == null || typeof value !== 'object') return value;

    type MutableDescriptorValue = Record<string, unknown>;
    const setOwnValue = (target: MutableDescriptorValue, key: string, child: unknown): void => {
        Object.defineProperty(target, key, {
            value: child,
            writable: true,
            enumerable: true,
            configurable: true,
        });
    };
    const cloneRoot: MutableDescriptorValue = (
        Array.isArray(value) ? [] : {}
    ) as MutableDescriptorValue;
    const clones = new Map<object, MutableDescriptorValue>([[value, cloneRoot]]);
    const pending: Array<{
        source: Record<string, unknown>;
        target: MutableDescriptorValue;
    }> = [{ source: value as Record<string, unknown>, target: cloneRoot }];
    const created = [cloneRoot];

    while (pending.length > 0) {
        const current = pending.pop();
        if (current === undefined) break;
        const keys = Array.isArray(current.source)
            ? Array.from({ length: (current.source as unknown[]).length }, (_, index) =>
                  String(index)
              )
            : Object.keys(current.source);
        for (const key of keys) {
            const child = current.source[key];
            if (child == null || typeof child !== 'object') {
                setOwnValue(current.target, key, child);
                continue;
            }
            const existing = clones.get(child);
            if (existing !== undefined) {
                setOwnValue(current.target, key, existing);
                continue;
            }
            const childClone: MutableDescriptorValue = (
                Array.isArray(child) ? [] : {}
            ) as MutableDescriptorValue;
            clones.set(child, childClone);
            created.push(childClone);
            setOwnValue(current.target, key, childClone);
            pending.push({
                source: child as Record<string, unknown>,
                target: childClone,
            });
        }
    }

    for (let index = created.length - 1; index >= 0; index -= 1) {
        Object.freeze(created[index]);
    }
    return cloneRoot as T;
}

function cloneAndFreezeDocumentDescriptor(
    descriptor: ResolvedDocumentSchema
): ResolvedDocumentSchema {
    return Object.freeze({
        schema: cloneAndFreezeDescriptorValue(descriptor.schema),
        documentNodeName: descriptor.documentNodeName,
        emptyDocument: cloneAndFreezeDescriptorValue(descriptor.emptyDocument),
    });
}

function buildV2CreateRequest(config: NativeEditorV2CreateConfig): {
    configJson: string;
    snapshotState: Uint8Array | null;
    documentDescriptor: ResolvedDocumentSchema;
} {
    let normalized: ReturnType<typeof buildV2CreateRequestUnchecked>;
    try {
        normalized = buildV2CreateRequestUnchecked(config);
    } catch (error) {
        const message =
            error instanceof NativeEditorV2CreateConfigError
                ? error.message
                : 'NativeEditorBridge: invalid v2 create config';
        throw invalidV2RequestError(message);
    }

    validateV2CreateLimits(normalized.limits);
    const documentDescriptor = cloneAndFreezeDocumentDescriptor(
        resolveDocumentDescriptor(
            normalized.envelope.schema as SchemaDefinition | undefined,
            normalized.limits?.resource as EditorResourceLimits | undefined
        )
    );
    try {
        const configJson = serializeV2CreateEnvelope(normalized.envelope);
        return { configJson, snapshotState: normalized.snapshotState, documentDescriptor };
    } catch {
        throw invalidV2RequestError('NativeEditorBridge: invalid v2 create config');
    }
}

/**
 * Typed imperative v2 bridge bound to one decimal-string editor id. Every
 * entry normalizes the frozen result record, keeps revisions as decimal
 * strings, and throws typed errors; results that arrive for a destroyed
 * handle (including re-entrant destroy races) are classified non-retryable.
 */
export class NativeEditorV2Bridge {
    private readonly _editorId: string;
    private _destroyed = false;
    private _nextRequestId = 0n;
    private readonly _errorListeners = new Set<(error: NativeEditorV2ErrorBase) => void>();
    private _collaborationProtocolAdapter: NativeCollaborationProtocolAdapter | null = null;
    private _collaborationProtocolAdapterSubscription: { remove(): void } | null = null;

    /** @internal Created by createNativeEditorDocumentHandle. */
    constructor(editorId: string) {
        this._editorId = editorId;
    }

    get editorId(): string {
        return this._editorId;
    }

    get isDestroyed(): boolean {
        return this._destroyed;
    }

    private assertAlive(): void {
        if (this._destroyed) throw destroyedHandleError();
    }

    private callV2<T>(invoke: () => unknown, normalizeValue: (value: unknown) => T | null): T {
        this.assertAlive();
        const raw = invoke();
        // A re-entrant destroy racing the native call makes any result
        // arriving now a result for a destroyed handle: non-retryable.
        if (this._destroyed) throw destroyedHandleError();
        return unwrapNativeEditorV2Result(raw, normalizeValue);
    }

    private nextRequestId(): string {
        if (this._nextRequestId >= 18_446_744_073_709_551_615n) {
            throw invalidV2RequestError('NativeEditorBridge: v2 request id exhausted');
        }
        this._nextRequestId += 1n;
        return this._nextRequestId.toString();
    }

    /**
     * Serialize a request envelope with canonical decimal-string u64 fields.
     */
    private buildEnvelopeJson(
        payload: Record<string, unknown>,
        baseDocumentRevision?: string
    ): string {
        const parts: string[] = [
            `"version":${V2_ENVELOPE_VERSION}`,
            `"requestId":"${this.nextRequestId()}"`,
        ];
        if (baseDocumentRevision !== undefined) {
            const digits = requireV2DecimalId(baseDocumentRevision, 'baseDocumentRevision');
            parts.push(`"baseDocumentRevision":"${digits}"`);
        }
        const payloadJson = JSON.stringify(payload);
        const inner = payloadJson.slice(1, payloadJson.length - 1);
        if (inner.length > 0) parts.push(inner);
        return `{${parts.join(',')}}`;
    }

    /** Destroy the session. Repeated destroy is safe. */
    destroy(): void {
        if (this._destroyed) return;
        try {
            unwrapNativeEditorV2Result(
                invokeNativeEditorV2('editorV2Destroy', this._editorId),
                normalizeNativeEditorV2Unit
            );
        } catch (error) {
            // An already-destroyed native session still satisfies the
            // caller's goal; every other failure is reported.
            if (
                error instanceof NativeEditorV2NonRetryableError &&
                (error.code === 'ENGINE_DESTROYED' || error.code === 'ENGINE_DESTROYING')
            ) {
                // An already-destroyed native session still commits the
                // local teardown below.
            } else {
                throw error;
            }
        }
        this._destroyed = true;
        this._collaborationProtocolAdapter = null;
        this._collaborationProtocolAdapterSubscription?.remove();
        this._collaborationProtocolAdapterSubscription = null;
        this._errorListeners.clear();
    }

    /** Subscribe to autonomous native failures; returns the unsubscribe. */
    addErrorListener(listener: (error: NativeEditorV2ErrorBase) => void): () => void {
        this._errorListeners.add(listener);
        return () => {
            this._errorListeners.delete(listener);
        };
    }

    /**
     * @internal Route one autonomous native failure (input/accessibility) to
     * the error listeners exactly once. Accepts a bare error record or the
     * frozen envelope form; malformed payloads surface as a non-retryable
     * contract violation so the view stays usable.
     */
    _emitAutonomousError(raw: unknown): void {
        if (this._destroyed) return;
        const candidate = isPlainRecord(raw) && 'error' in raw ? raw : { error: raw };
        const normalized = normalizeNativeEditorV2Error(candidate);
        const exception =
            normalized == null
                ? invalidV2ResultError()
                : nativeEditorV2ErrorToException(normalized);
        for (const listener of this._errorListeners) {
            listener(exception);
        }
    }

    getState(): NativeEditorV2EditorState {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetState', this._editorId),
            normalizeNativeEditorV2StateValue
        );
    }

    getDocumentJson(): DocumentJSON {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetDocumentJson', this._editorId),
            normalizeNativeEditorV2DocumentJsonValue
        );
    }

    getDocumentHtml(): string {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetDocumentHtml', this._editorId),
            normalizeNativeEditorV2HtmlValue
        );
    }

    getContentSnapshot(): ContentSnapshot {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2GetContentSnapshot', this._editorId),
            normalizeNativeEditorV2ContentSnapshotValue
        );
    }

    /**
     * Fetch the complete typed, immutable render snapshot a bound native view
     * applies after a JS-driven engine change. Without a scalar mirror, its
     * selection is the engine-authoritative selection; a mirror resolves only
     * this snapshot's selection into document and scalar positions.
     */
    renderUpdate(mirrorScalarSelection?: {
        anchor: number;
        head: number;
    }): NativeEditorV2AtomicRenderSnapshot {
        this.assertAlive();
        const mirrorAnchor = mirrorScalarSelection?.anchor ?? null;
        const mirrorHead = mirrorScalarSelection?.head ?? null;
        if ((mirrorAnchor == null) !== (mirrorHead == null)) {
            throw invalidV2RequestError(
                'NativeEditorBridge: render update mirror requires both scalar anchor and head'
            );
        }
        if (mirrorAnchor != null) requireNativeEditorV2U32(mirrorAnchor, 'mirrorScalarAnchor');
        if (mirrorHead != null) requireNativeEditorV2U32(mirrorHead, 'mirrorScalarHead');
        return this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2RenderUpdate',
                    this._editorId,
                    mirrorAnchor,
                    mirrorHead
                ),
            normalizeNativeEditorV2RenderUpdateValue
        );
    }

    replaceDocument(request: NativeEditorV2ReplaceDocumentRequest): NativeEditorV2CommitInfo {
        this.assertAlive();
        const payload: Record<string, unknown> = {};
        if (request.setJson !== undefined) payload.setJson = request.setJson;
        if (request.setHtml !== undefined) payload.setHtml = request.setHtml;
        payload.history = request.history;
        const requestJson = this.buildEnvelopeJson(payload);
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ReplaceDocument', this._editorId, requestJson),
            normalizeNativeEditorV2CommitValue
        );
    }

    applyInput(request: NativeEditorV2InputRequest): NativeEditorV2MutationOutcome {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson(
            { text: request.text },
            request.baseDocumentRevision
        );
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ApplyInput', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    applyCommand(request: NativeEditorV2CommandRequest): NativeEditorV2MutationOutcome {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson(
            { command: request.command },
            request.baseDocumentRevision
        );
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ApplyCommand', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    applyLocalApi(request: NativeEditorV2LocalApiRequest): NativeEditorV2MutationOutcome {
        this.assertAlive();
        const payload: Record<string, unknown> = {};
        if (request.setJson !== undefined) payload.setJson = request.setJson;
        if (request.setHtml !== undefined) payload.setHtml = request.setHtml;
        payload.history = request.history;
        const requestJson = this.buildEnvelopeJson(payload, request.baseDocumentRevision);
        return this.callV2(
            () => invokeNativeEditorV2('editorV2ApplyLocalApi', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    setSelection(request: NativeEditorV2SelectionRequest): NativeEditorV2MutationOutcome {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson(
            { selection: request.selection },
            request.baseDocumentRevision
        );
        return this.callV2(
            () => invokeNativeEditorV2('editorV2SetSelection', this._editorId, requestJson),
            normalizeNativeEditorV2MutationOutcomeValue
        );
    }

    undo(): boolean {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson({});
        return this.callV2(
            () => invokeNativeEditorV2('editorV2Undo', this._editorId, requestJson),
            normalizeNativeEditorV2ChangedValue
        );
    }

    redo(): boolean {
        this.assertAlive();
        const requestJson = this.buildEnvelopeJson({});
        return this.callV2(
            () => invokeNativeEditorV2('editorV2Redo', this._editorId, requestJson),
            normalizeNativeEditorV2ChangedValue
        );
    }

    snapshotExport(): NativeEditorV2SnapshotExport {
        return this.callV2(
            () => invokeNativeEditorV2('editorV2SnapshotExport', this._editorId),
            normalizeNativeEditorV2SnapshotExportValue
        );
    }

    snapshotRestore(
        metadata: NativeEditorV2SnapshotMetadata,
        encodedState: Uint8Array
    ): NativeEditorV2CommitInfo {
        this.assertAlive();
        const bytes = requireV2Bytes(encodedState, 'snapshot encodedState');
        return this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2SnapshotRestore',
                    this._editorId,
                    JSON.stringify(metadata),
                    bytes
                ),
            normalizeNativeEditorV2CommitValue
        );
    }

    configureCollaborationTransport(config: NativeCollaborationTransportConfig | null): void {
        this.assertAlive();
        const wireConfig = config === null ? null : collaborationTransportWireConfig(config);
        const previousAdapter = this._collaborationProtocolAdapter;
        const nextAdapter = config?.protocolAdapter ?? null;
        if (nextAdapter !== null && this._collaborationProtocolAdapterSubscription === null) {
            this._collaborationProtocolAdapterSubscription = getNativeModule().addListener(
                'onCollaborationTransportEvent',
                (rawEvent) => this.handleCollaborationProtocolAdapterEvent(rawEvent)
            );
        }
        this._collaborationProtocolAdapter = nextAdapter;
        try {
            this.callV2(
                () =>
                    invokeNativeEditorV2(
                        'editorV2CollaborationConfigureTransport',
                        this._editorId,
                        wireConfig === null ? null : JSON.stringify(wireConfig)
                    ),
                normalizeNativeEditorV2Unit
            );
        } catch (error) {
            this._collaborationProtocolAdapter = previousAdapter;
            if (
                previousAdapter === null &&
                this._collaborationProtocolAdapterSubscription !== null
            ) {
                this._collaborationProtocolAdapterSubscription.remove();
                this._collaborationProtocolAdapterSubscription = null;
            }
            throw error;
        }
        if (nextAdapter === null && this._collaborationProtocolAdapterSubscription !== null) {
            this._collaborationProtocolAdapterSubscription.remove();
            this._collaborationProtocolAdapterSubscription = null;
        }
    }

    private handleCollaborationProtocolAdapterEvent(rawEvent: unknown): void {
        if (this._destroyed || this._collaborationProtocolAdapter === null) return;
        const event = normalizeNativeCollaborationTransportEvent(rawEvent);
        if (event?.editorId !== this._editorId || event.kind !== 'protocolAdapter') {
            return;
        }
        const adapter = this._collaborationProtocolAdapter;
        const context: NativeCollaborationProtocolAdapterContext = {
            attemptId: event.attemptId,
            generation: event.generation,
            negotiatedProtocol: event.negotiatedProtocol,
        };
        const callback =
            event.phase === 'open'
                ? () => adapter.onOpen(context)
                : () => adapter.onMessage(context, event.frame!);
        void Promise.resolve()
            .then(callback)
            .then((result) => serializeCollaborationProtocolAdapterResult(result))
            .catch(() => '{"action":"reject"}')
            .then((responseJson) => {
                if (this._destroyed) return;
                try {
                    this.callV2(
                        () =>
                            invokeNativeEditorV2(
                                'editorV2CollaborationResolveProtocolAdapter',
                                this._editorId,
                                event.attemptId,
                                event.eventId,
                                responseJson
                            ),
                        normalizeNativeEditorV2Unit
                    );
                } catch {
                    // Native treats stale attempt responses as no-ops. A live
                    // resolution failure is surfaced by its transport event;
                    // adapter payloads and callback errors are never logged.
                }
            });
    }

    setLocalAwareness(intent: NativeEditorLocalAwarenessIntent | null): void {
        this.assertAlive();
        const awarenessJson =
            intent === null
                ? 'null'
                : serializeLocalAwarenessIntent(validateLocalAwarenessIntent(intent));
        this.callV2(
            () =>
                invokeNativeEditorV2(
                    'editorV2CollaborationSetAwareness',
                    this._editorId,
                    awarenessJson
                ),
            normalizeNativeEditorV2Unit
        );
    }

    addCollaborationTransportListener(
        listener: (event: NativeCollaborationTransportEvent) => void
    ): () => void {
        this.assertAlive();
        const subscription = getNativeModule().addListener(
            'onCollaborationTransportEvent',
            (rawEvent) => {
                if (this._destroyed) return;
                const event = normalizeNativeCollaborationTransportEvent(rawEvent);
                if (event?.editorId === this._editorId) {
                    listener(event);
                }
            }
        );
        return () => subscription.remove();
    }
}

const NATIVE_EDITOR_DOCUMENT_HANDLE_BRAND: unique symbol = Symbol(
    'NativeEditorDocumentHandle.brand'
);
const NATIVE_EDITOR_DOCUMENT_HANDLE_TOKEN = Object.freeze({});
const AUTHENTIC_NATIVE_EDITOR_DOCUMENT_HANDLES = new WeakSet<object>();
const NATIVE_EDITOR_DOCUMENT_HANDLE_DESCRIPTORS = new WeakMap<object, ResolvedDocumentSchema>();

/**
 * One native document session, shared by everything that touches the
 * document: the editor view, the headless `useNativeEditorDocument` binding,
 * and the collaboration controller. Obtain one from
 * {@link createNativeEditorDocumentHandle} — it cannot be constructed
 * directly — and `destroy()` it when its owner unmounts.
 */
export interface NativeEditorDocumentHandle {
    readonly [NATIVE_EDITOR_DOCUMENT_HANDLE_BRAND]: true;
    /** Decimal-string session id, shared with the native view. */
    readonly editorId: string;
    /** Typed imperative access to the engine. The React APIs use this for you. */
    readonly bridge: NativeEditorV2Bridge;
    readonly isDestroyed: boolean;
    /** Release the native session. Every later call fails as non-retryable. */
    destroy(): void;
    /** Observe engine failures raised outside a caller's own call. Returns an unsubscribe function. */
    addErrorListener(listener: (error: NativeEditorV2ErrorBase) => void): () => void;
    /** Point the native transport at a server, or pass null to detach. */
    configureCollaborationTransport(config: NativeCollaborationTransportConfig | null): void;
    /** Publish local presence, or pass null to withdraw it. */
    setLocalAwareness(intent: NativeEditorLocalAwarenessIntent | null): void;
    /** Observe transport state, errors, and protocol-adapter requests. Returns an unsubscribe function. */
    addCollaborationTransportListener(
        listener: (event: NativeCollaborationTransportEvent) => void
    ): () => void;
}

class NativeEditorDocumentHandleImpl implements NativeEditorDocumentHandle {
    readonly [NATIVE_EDITOR_DOCUMENT_HANDLE_BRAND] = true as const;

    constructor(
        token: typeof NATIVE_EDITOR_DOCUMENT_HANDLE_TOKEN,
        public readonly editorId: string,
        public readonly bridge: NativeEditorV2Bridge,
        documentDescriptor: ResolvedDocumentSchema
    ) {
        if (token !== NATIVE_EDITOR_DOCUMENT_HANDLE_TOKEN) {
            throw invalidV2RequestError(
                'NativeEditorBridge: NativeEditorDocumentHandle cannot be constructed directly'
            );
        }
        AUTHENTIC_NATIVE_EDITOR_DOCUMENT_HANDLES.add(this);
        NATIVE_EDITOR_DOCUMENT_HANDLE_DESCRIPTORS.set(this, documentDescriptor);
    }

    get isDestroyed(): boolean {
        return this.bridge.isDestroyed;
    }

    destroy(): void {
        this.bridge.destroy();
    }

    addErrorListener(listener: (error: NativeEditorV2ErrorBase) => void): () => void {
        return this.bridge.addErrorListener(listener);
    }

    configureCollaborationTransport(config: NativeCollaborationTransportConfig | null): void {
        this.bridge.configureCollaborationTransport(config);
    }

    setLocalAwareness(intent: NativeEditorLocalAwarenessIntent | null): void {
        this.bridge.setLocalAwareness(intent);
    }

    addCollaborationTransportListener(
        listener: (event: NativeCollaborationTransportEvent) => void
    ): () => void {
        return this.bridge.addCollaborationTransportListener(listener);
    }
}

/** @internal Handle-owned schema metadata for source-module view bindings. */
export function _getNativeEditorDocumentHandleDescriptor(
    handle: NativeEditorDocumentHandle
): ResolvedDocumentSchema {
    _assertNativeEditorDocumentHandle(handle);
    const documentDescriptor = NATIVE_EDITOR_DOCUMENT_HANDLE_DESCRIPTORS.get(handle);
    if (documentDescriptor === undefined) {
        throw invalidV2RequestError(
            'NativeEditorBridge: authentic NativeEditorDocumentHandle has no document descriptor'
        );
    }
    return documentDescriptor;
}

/** @internal Source-module boundary assertion; intentionally absent from the package index. */
export function _assertNativeEditorDocumentHandle(
    value: unknown
): asserts value is NativeEditorDocumentHandle {
    if (
        (typeof value !== 'object' && typeof value !== 'function') ||
        value === null ||
        !AUTHENTIC_NATIVE_EDITOR_DOCUMENT_HANDLES.has(value)
    ) {
        throw invalidV2RequestError(
            'NativeEditorBridge: expected an authentic NativeEditorDocumentHandle'
        );
    }
}

/**
 * Create the native document session every other API binds to. Create it once
 * per document — `useMemo`, not on each render — and destroy it when its
 * owner unmounts.
 *
 * @throws NativeEditorV2ErrorBase when the config is rejected: a malformed
 * schema, an out-of-range limit, or content the engine cannot parse.
 *
 * @example
 * ```ts
 * const documentHandle = useMemo(
 *     () => createNativeEditorDocumentHandle({
 *         initialization: { type: 'localHtml', html: '<p>Hello world</p>' },
 *     }),
 *     []
 * );
 * useEffect(() => () => documentHandle.destroy(), [documentHandle]);
 * ```
 */
export function createNativeEditorDocumentHandle(
    config: NativeEditorV2CreateConfig
): NativeEditorDocumentHandle {
    const { configJson, snapshotState, documentDescriptor } = buildV2CreateRequest(config);
    const value = unwrapNativeEditorV2Result(
        invokeNativeEditorV2('editorV2Create', configJson, snapshotState),
        normalizeNativeEditorV2CreateValue
    );
    return new NativeEditorDocumentHandleImpl(
        NATIVE_EDITOR_DOCUMENT_HANDLE_TOKEN,
        value.editorId,
        new NativeEditorV2Bridge(value.editorId),
        documentDescriptor
    );
}
