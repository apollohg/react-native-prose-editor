import type { SerializedEditorMentionTheme } from './EditorMentionThemeNormalization';
import { type SchemaDefinition } from './schemas';
import {
    type EditorCollaborationLimits,
    type EditorEditingLimits,
    type EditorResourceLimits,
} from './ResourceLimits';

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
export class NativeEditorLocalAwarenessSelectionValue {
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
    mentionTheme?: SerializedEditorMentionTheme;
    listContext?: ListContext;
    language?: string;
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

export type DeepReadonly<T> =
    T extends ReadonlyArray<infer Item>
        ? ReadonlyArray<DeepReadonly<Item>>
        : T extends object
          ? { readonly [Key in keyof T]: DeepReadonly<T[Key]> }
          : T;

/** A recursively immutable active-state view supplied by atomic render snapshots. */
export type ReadonlyActiveState = DeepReadonly<ActiveState>;

export type NativeEditorAtomicRenderPayload =
    | { renderBlocks: RenderElement[][]; renderPatch: null }
    | { renderBlocks: null; renderPatch: RenderBlocksPatch };

export type NativeEditorAtomicRenderSnapshotShape = NativeEditorAtomicRenderPayload & {
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
export type NativeEditorAtomicRenderSnapshot =
    DeepReadonly<NativeEditorAtomicRenderSnapshotShape>;

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

export type NativeEditorHistoryMode = 'undoableBoundary' | 'resetAndClear';

/**
 * Provenance of an exported room snapshot. A snapshot only restores into a
 * handle whose document, lineage, fragment, and schema match.
 */
export interface NativeEditorSnapshotMetadata {
    /** Snapshot format version, so older exports can be recognized. */
    formatVersion: number;
    documentId: string;
    lineageId: string;
    fragmentName: string;
    /** Digest of the schema the snapshot was taken under. */
    schemaFingerprint: string;
}

/** An exported room document: its provenance plus the encoded Yjs state. */
export interface NativeEditorRoomSnapshot {
    metadata: NativeEditorSnapshotMetadata;
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
export type NativeEditorInitialization =
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
          snapshot?: NativeEditorRoomSnapshot;
      };

/**
 * Everything fixed for the lifetime of a document handle. Initialization,
 * schema, editing policy, and limits all live here rather than on the view,
 * so an editor and its collaboration controller cannot disagree about them.
 */
export interface NativeEditorCreateConfig {
    initialization: NativeEditorInitialization;
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

export interface NativeEditorInputRequest {
    baseDocumentRevision: string;
    text: string;
}

export interface NativeEditorCommandRequest {
    baseDocumentRevision: string;
    command: Record<string, unknown>;
}

export interface NativeEditorLocalApiRequest {
    baseDocumentRevision: string;
    setJson?: DocumentJSON;
    setHtml?: string;
    history: NativeEditorHistoryMode;
}

export type NativeEditorOffsetKind = 'scalar' | 'utf16';

export type NativeEditorPositionAffinity = 'before' | 'after';

/**
 * Mirrors the Rust `PositionEnvelope`. `offset` is measured in the currency
 * named by `kind` — scalar offsets are Unicode scalars, not document positions.
 */
export interface NativeEditorPositionEnvelope {
    offset: number;
    kind: NativeEditorOffsetKind;
    affinity?: NativeEditorPositionAffinity;
}

export type NativeEditorSelectionEnvelope =
    | {
          type: 'text';
          anchor: NativeEditorPositionEnvelope;
          head: NativeEditorPositionEnvelope;
      }
    | { type: 'node'; at: NativeEditorPositionEnvelope }
    | { type: 'atom'; docPos: number; edge: 'node' | 'before' | 'after' }
    | { type: 'all' };

export interface NativeEditorSelectionRequest {
    baseDocumentRevision: string;
    selection: NativeEditorSelectionEnvelope;
}

export interface NativeEditorReplaceDocumentRequest {
    setJson?: DocumentJSON;
    setHtml?: string;
    history: NativeEditorHistoryMode;
}
