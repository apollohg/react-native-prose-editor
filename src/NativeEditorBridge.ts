import { requireNativeModule } from 'expo-modules-core';
import { normalizeDocumentJson, type SchemaDefinition } from './schemas';

const ERR_DESTROYED = 'NativeEditorBridge: editor has been destroyed';
const ERR_NATIVE_RESPONSE = 'NativeEditorBridge: invalid JSON response from native module';
const ERR_INVALID_ENCODED_STATE = 'NativeEditorBridge: invalid encoded collaboration state';
const BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';

export interface NativeEditorModule {
    editorCreate(configJson: string): number;
    editorDestroy(editorId: number): void;
    collaborationSessionCreate(configJson: string): number;
    collaborationSessionDestroy(sessionId: number): void;
    collaborationSessionGetDocumentJson(sessionId: number): string;
    collaborationSessionGetEncodedState(sessionId: number): string;
    collaborationSessionGetPeersJson(sessionId: number): string;
    collaborationSessionStart(sessionId: number): string;
    collaborationSessionApplyLocalDocumentJson(sessionId: number, json: string): string;
    collaborationSessionApplyEncodedState(sessionId: number, encodedStateJson: string): string;
    collaborationSessionReplaceEncodedState(sessionId: number, encodedStateJson: string): string;
    collaborationSessionHandleMessage(sessionId: number, messageJson: string): string;
    collaborationSessionSetLocalAwareness(sessionId: number, awarenessJson: string): string;
    collaborationSessionClearLocalAwareness(sessionId: number): string;
    editorSetHtml(editorId: number, html: string): string;
    editorGetHtml(editorId: number): string;
    editorSetJson(editorId: number, json: string): string;
    editorGetJson(editorId: number): string;
    editorGetContentSnapshot(editorId: number): string;
    editorReplaceHtml(editorId: number, html: string): string;
    editorReplaceJson(editorId: number, json: string): string;
    editorInsertText(editorId: number, pos: number, text: string): string;
    editorReplaceSelectionText(editorId: number, text: string): string;
    editorDeleteRange(editorId: number, from: number, to: number): string;
    editorSplitBlock(editorId: number, pos: number): string;
    editorInsertContentHtml(editorId: number, html: string): string;
    editorInsertContentJson(editorId: number, json: string): string;
    editorInsertContentJsonAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number,
        json: string
    ): string;
    editorToggleMark(editorId: number, markName: string): string;
    editorSetMark(editorId: number, markName: string, attrsJson: string): string;
    editorUnsetMark(editorId: number, markName: string): string;
    editorToggleBlockquote(editorId: number): string;
    editorToggleHeading(editorId: number, level: number): string;
    editorSetSelection(editorId: number, anchor: number, head: number): void;
    editorGetSelection(editorId: number): string;
    editorGetSelectionState(editorId: number): string;
    editorGetCurrentState(editorId: number): string;
    // Scalar-position APIs (used by native views internally)
    editorInsertTextScalar(editorId: number, scalarPos: number, text: string): string;
    editorDeleteScalarRange(editorId: number, scalarFrom: number, scalarTo: number): string;
    editorReplaceTextScalar(
        editorId: number,
        scalarFrom: number,
        scalarTo: number,
        text: string
    ): string;
    editorSplitBlockScalar(editorId: number, scalarPos: number): string;
    editorDeleteAndSplitScalar(editorId: number, scalarFrom: number, scalarTo: number): string;
    editorSetSelectionScalar(editorId: number, scalarAnchor: number, scalarHead: number): void;
    editorToggleMarkAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number,
        markName: string
    ): string;
    editorSetMarkAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number,
        markName: string,
        attrsJson: string
    ): string;
    editorUnsetMarkAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number,
        markName: string
    ): string;
    editorToggleBlockquoteAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number
    ): string;
    editorToggleHeadingAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number,
        level: number
    ): string;
    editorWrapInListAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number,
        listType: string
    ): string;
    editorUnwrapFromListAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number
    ): string;
    editorIndentListItemAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number
    ): string;
    editorOutdentListItemAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number
    ): string;
    editorInsertNodeAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number,
        nodeType: string
    ): string;
    editorDocToScalar(editorId: number, docPos: number): number;
    editorScalarToDoc(editorId: number, scalar: number): number;
    editorWrapInList(editorId: number, listType: string): string;
    editorUnwrapFromList(editorId: number): string;
    editorIndentListItem(editorId: number): string;
    editorOutdentListItem(editorId: number): string;
    editorInsertNode(editorId: number, nodeType: string): string;
    editorUndo(editorId: number): string;
    editorRedo(editorId: number): string;
    editorCanUndo(editorId: number): boolean;
    editorCanRedo(editorId: number): boolean;
}

export interface Selection {
    type: 'text' | 'node' | 'all';
    anchor?: number;
    head?: number;
    pos?: number;
}

export interface ListContext {
    ordered: boolean;
    index: number;
    total: number;
    start: number;
    isFirst: boolean;
    isLast: boolean;
}

export interface RenderMarkWithAttrs {
    type: string;
    [key: string]: unknown;
}

export type RenderMark = string | RenderMarkWithAttrs;

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
    label?: string;
    attrs?: Record<string, unknown>;
    listContext?: ListContext;
}

interface RenderBlocksPatch {
    startIndex: number;
    deleteCount: number;
    renderBlocks: RenderElement[][];
}

export interface ActiveState {
    marks: Record<string, boolean>;
    markAttrs: Record<string, Record<string, unknown>>;
    nodes: Record<string, boolean>;
    commands: Record<string, boolean>;
    allowedMarks: string[];
    insertableNodes: string[];
}

export interface HistoryState {
    canUndo: boolean;
    canRedo: boolean;
}

export interface EditorUpdate {
    renderElements: RenderElement[];
    renderBlocks?: RenderElement[][];
    renderPatch?: RenderBlocksPatch;
    selection: Selection;
    activeState: ActiveState;
    historyState: HistoryState;
    documentVersion?: number;
}

export interface ContentSnapshot {
    html: string;
    json: DocumentJSON;
}

export interface DocumentJSON {
    [key: string]: unknown;
}

export interface CollaborationPeer {
    clientId: number;
    isLocal: boolean;
    state: Record<string, unknown> | null;
}

export interface CollaborationResult {
    messages: number[][];
    documentChanged: boolean;
    documentJson?: DocumentJSON;
    peersChanged: boolean;
    peers?: CollaborationPeer[];
}

export type EncodedCollaborationStateInput = Uint8Array | readonly number[] | string;

export function normalizeActiveState(raw: unknown): ActiveState {
    const obj = (raw as Record<string, unknown>) ?? {};
    return {
        marks: (obj.marks ?? {}) as Record<string, boolean>,
        markAttrs: (obj.markAttrs ?? {}) as Record<string, Record<string, unknown>>,
        nodes: (obj.nodes ?? {}) as Record<string, boolean>,
        commands: (obj.commands ?? {}) as Record<string, boolean>,
        allowedMarks: (obj.allowedMarks ?? []) as string[],
        insertableNodes: (obj.insertableNodes ?? []) as string[],
    };
}

function parseRenderElements(json: string): RenderElement[] {
    if (!json || json === '[]') return [];
    try {
        const parsed: unknown = JSON.parse(json);
        if (
            parsed != null &&
            typeof parsed === 'object' &&
            !Array.isArray(parsed) &&
            'error' in parsed
        ) {
            throw new Error(`NativeEditorBridge: ${(parsed as { error: unknown }).error}`);
        }
        if (!Array.isArray(parsed)) {
            throw new Error(ERR_NATIVE_RESPONSE);
        }
        return parsed as RenderElement[];
    } catch (e) {
        if (e instanceof Error && e.message.startsWith('NativeEditorBridge:')) {
            throw e;
        }
        throw new Error(ERR_NATIVE_RESPONSE);
    }
}

export function parseEditorUpdateJson(
    json: string,
    previousRenderBlocks?: RenderElement[][]
): EditorUpdate | null {
    if (!json || json === '') return null;
    try {
        const parsed = JSON.parse(json) as Record<string, unknown>;
        if ('error' in parsed) {
            throw new Error(`NativeEditorBridge: ${parsed.error}`);
        }
        const renderBlocks = Array.isArray(parsed.renderBlocks)
            ? (parsed.renderBlocks as RenderElement[][])
            : applyRenderBlocksPatch(
                  previousRenderBlocks,
                  parsed.renderPatch != null && typeof parsed.renderPatch === 'object'
                      ? (parsed.renderPatch as RenderBlocksPatch)
                      : undefined
              );
        const renderPatch =
            parsed.renderPatch != null && typeof parsed.renderPatch === 'object'
                ? (parsed.renderPatch as RenderBlocksPatch)
                : undefined;
        return {
            renderElements: Array.isArray(parsed.renderElements)
                ? (parsed.renderElements as RenderElement[])
                : flattenRenderBlocks(renderBlocks),
            renderBlocks,
            renderPatch,
            selection: (parsed.selection ?? { type: 'text', anchor: 0, head: 0 }) as Selection,
            activeState: normalizeActiveState(parsed.activeState),
            historyState: (parsed.historyState ?? {
                canUndo: false,
                canRedo: false,
            }) as HistoryState,
            documentVersion:
                typeof parsed.documentVersion === 'number' ? parsed.documentVersion : undefined,
        };
    } catch (e) {
        if (e instanceof Error && e.message.startsWith('NativeEditorBridge:')) {
            throw e;
        }
        throw new Error(ERR_NATIVE_RESPONSE);
    }
}

function flattenRenderBlocks(renderBlocks?: RenderElement[][]): RenderElement[] {
    if (!renderBlocks || renderBlocks.length === 0) {
        return [];
    }
    return renderBlocks.flat();
}

function applyRenderBlocksPatch(
    previousRenderBlocks?: RenderElement[][],
    renderPatch?: RenderBlocksPatch
): RenderElement[][] | undefined {
    if (!previousRenderBlocks || !renderPatch) {
        return undefined;
    }

    const { startIndex, deleteCount, renderBlocks } = renderPatch;
    if (
        !Number.isInteger(startIndex) ||
        !Number.isInteger(deleteCount) ||
        startIndex < 0 ||
        deleteCount < 0 ||
        startIndex > previousRenderBlocks.length ||
        startIndex + deleteCount > previousRenderBlocks.length
    ) {
        return undefined;
    }

    return [
        ...previousRenderBlocks.slice(0, startIndex),
        ...renderBlocks,
        ...previousRenderBlocks.slice(startIndex + deleteCount),
    ];
}

function parseContentSnapshotJson(json: string): ContentSnapshot {
    try {
        const parsed = JSON.parse(json) as Record<string, unknown>;
        if ('error' in parsed) {
            throw new Error(`NativeEditorBridge: ${parsed.error}`);
        }
        return {
            html: typeof parsed.html === 'string' ? parsed.html : '',
            json:
                parsed.json != null && typeof parsed.json === 'object'
                    ? (parsed.json as DocumentJSON)
                    : {},
        };
    } catch (e) {
        if (e instanceof Error && e.message.startsWith('NativeEditorBridge:')) {
            throw e;
        }
        throw new Error(ERR_NATIVE_RESPONSE);
    }
}

function parseDocumentJSON(json: string): DocumentJSON {
    if (!json || json === '{}') return {};
    try {
        const parsed = JSON.parse(json) as DocumentJSON;
        if (
            parsed != null &&
            typeof parsed === 'object' &&
            'error' in (parsed as Record<string, unknown>)
        ) {
            throw new Error(`NativeEditorBridge: ${(parsed as Record<string, unknown>).error}`);
        }
        return parsed;
    } catch (e) {
        if (e instanceof Error && e.message.startsWith('NativeEditorBridge:')) {
            throw e;
        }
        throw new Error(ERR_NATIVE_RESPONSE);
    }
}

function parseCollaborationPeersJson(json: string): CollaborationPeer[] {
    if (!json || json === '[]') return [];
    try {
        const parsed = JSON.parse(json) as CollaborationPeer[];
        return Array.isArray(parsed) ? parsed : [];
    } catch {
        throw new Error(ERR_NATIVE_RESPONSE);
    }
}

function parseByteArrayJson(json: string): Uint8Array {
    if (!json || json === '[]') return new Uint8Array();
    try {
        const parsed = JSON.parse(json) as unknown;
        if (!Array.isArray(parsed)) {
            throw new Error(ERR_NATIVE_RESPONSE);
        }
        return Uint8Array.from(parsed as number[]);
    } catch (e) {
        if (e instanceof Error && e.message.startsWith('NativeEditorBridge:')) {
            throw e;
        }
        throw new Error(ERR_NATIVE_RESPONSE);
    }
}

function bytesToBase64(bytes: readonly number[]): string {
    let output = '';
    for (let index = 0; index < bytes.length; index += 3) {
        const byte1 = bytes[index] ?? 0;
        const byte2 = bytes[index + 1] ?? 0;
        const byte3 = bytes[index + 2] ?? 0;
        const chunk = (byte1 << 16) | (byte2 << 8) | byte3;

        output += BASE64_ALPHABET[(chunk >> 18) & 0x3f];
        output += BASE64_ALPHABET[(chunk >> 12) & 0x3f];
        output += index + 1 < bytes.length ? BASE64_ALPHABET[(chunk >> 6) & 0x3f] : '=';
        output += index + 2 < bytes.length ? BASE64_ALPHABET[chunk & 0x3f] : '=';
    }
    return output;
}

function base64ToBytes(base64: string): Uint8Array {
    const normalized = base64.replace(/\s+/g, '');
    if (normalized.length === 0) return new Uint8Array();
    if (normalized.length % 4 === 1) {
        throw new Error(ERR_INVALID_ENCODED_STATE);
    }

    const bytes: number[] = [];
    for (let index = 0; index < normalized.length; index += 4) {
        const chunk = normalized.slice(index, index + 4);
        const values = chunk.split('').map((char, charIndex) => {
            if (char === '=') {
                return charIndex >= 2 ? 0 : -1;
            }
            const value = BASE64_ALPHABET.indexOf(char);
            return value;
        });

        if (values[0] < 0 || values[1] < 0 || values.some((value) => value < 0)) {
            throw new Error(ERR_INVALID_ENCODED_STATE);
        }

        const combined = (values[0] << 18) | (values[1] << 12) | (values[2] << 6) | values[3];

        bytes.push((combined >> 16) & 0xff);
        if (chunk[2] !== '=') {
            bytes.push((combined >> 8) & 0xff);
        }
        if (chunk[3] !== '=') {
            bytes.push(combined & 0xff);
        }
    }

    return Uint8Array.from(bytes);
}

function normalizeEncodedStateInput(encodedState: EncodedCollaborationStateInput): number[] {
    if (typeof encodedState === 'string') {
        return Array.from(base64ToBytes(encodedState));
    }
    return Array.from(encodedState);
}

export function encodeCollaborationStateBase64(
    encodedState: EncodedCollaborationStateInput
): string {
    return bytesToBase64(normalizeEncodedStateInput(encodedState));
}

export function decodeCollaborationStateBase64(base64: string): Uint8Array {
    return base64ToBytes(base64);
}

function normalizeDocumentJsonString(jsonString: string, schema?: SchemaDefinition): string {
    try {
        const parsed = JSON.parse(jsonString) as DocumentJSON;
        const normalized = normalizeDocumentJson(parsed, schema);
        return normalized === parsed ? jsonString : JSON.stringify(normalized);
    } catch {
        return jsonString;
    }
}

export function parseCollaborationResultJson(json: string): CollaborationResult {
    if (!json || json === '') {
        return {
            messages: [],
            documentChanged: false,
            peersChanged: false,
        };
    }
    try {
        const parsed = JSON.parse(json) as Record<string, unknown>;
        if ('error' in parsed) {
            throw new Error(`NativeEditorBridge: ${parsed.error}`);
        }
        return {
            messages: Array.isArray(parsed.messages) ? (parsed.messages as number[][]) : [],
            documentChanged: parsed.documentChanged === true,
            documentJson:
                parsed.documentJson && typeof parsed.documentJson === 'object'
                    ? (parsed.documentJson as DocumentJSON)
                    : undefined,
            peersChanged: parsed.peersChanged === true,
            peers: Array.isArray(parsed.peers) ? (parsed.peers as CollaborationPeer[]) : undefined,
        };
    } catch (e) {
        if (e instanceof Error && e.message.startsWith('NativeEditorBridge:')) {
            throw e;
        }
        throw new Error(ERR_NATIVE_RESPONSE);
    }
}

let _nativeModule: NativeEditorModule | null = null;

function getNativeModule(): NativeEditorModule {
    if (!_nativeModule) {
        _nativeModule = requireNativeModule<NativeEditorModule>('NativeEditor');
    }
    return _nativeModule;
}

/** @internal Reset the cached native module reference. For testing only. */
export function _resetNativeModuleCache(): void {
    _nativeModule = null;
}

export class NativeEditorBridge {
    private _editorId: number;
    private _schema?: SchemaDefinition;
    private _destroyed = false;
    private _lastSelection: Selection = { type: 'text', anchor: 0, head: 0 };
    private _documentVersion = 0;
    private _cachedHtml: { version: number; value: string } | null = null;
    private _cachedJsonString: { version: number; value: string } | null = null;
    private _renderBlocksCache: RenderElement[][] | null = null;

    private constructor(editorId: number, schema?: SchemaDefinition) {
        this._editorId = editorId;
        this._schema = schema;
    }

    /** Create a new editor instance backed by the Rust engine. */
    static create(config?: {
        maxLength?: number;
        schemaJson?: string;
        allowBase64Images?: boolean;
    }): NativeEditorBridge {
        const configObj: Record<string, unknown> = {};
        let parsedSchema: SchemaDefinition | undefined;
        if (config?.maxLength != null) configObj.maxLength = config.maxLength;
        if (config?.allowBase64Images != null) {
            configObj.allowBase64Images = config.allowBase64Images;
        }
        if (config?.schemaJson != null) {
            try {
                parsedSchema = JSON.parse(config.schemaJson) as SchemaDefinition;
                configObj.schema = parsedSchema;
            } catch {
                // Fall back to the default schema when the provided JSON is invalid.
            }
        }
        const id = getNativeModule().editorCreate(JSON.stringify(configObj));
        return new NativeEditorBridge(id, parsedSchema);
    }

    /** The underlying native editor ID. */
    get editorId(): number {
        return this._editorId;
    }

    /** Whether this bridge has been destroyed. */
    get isDestroyed(): boolean {
        return this._destroyed;
    }

    /** Destroy the editor instance and free native resources. */
    destroy(): void {
        if (this._destroyed) return;
        this._destroyed = true;
        this._renderBlocksCache = null;
        getNativeModule().editorDestroy(this._editorId);
    }

    /** Set content from HTML. Returns render elements for display. */
    setHtml(html: string): RenderElement[] {
        this.assertNotDestroyed();
        this.invalidateContentCaches();
        this._renderBlocksCache = null;
        const json = getNativeModule().editorSetHtml(this._editorId, html);
        return parseRenderElements(json);
    }

    /** Get content as HTML. */
    getHtml(): string {
        this.assertNotDestroyed();
        if (this._cachedHtml?.version === this._documentVersion) {
            return this._cachedHtml.value;
        }
        const html = getNativeModule().editorGetHtml(this._editorId);
        this._cachedHtml = { version: this._documentVersion, value: html };
        return html;
    }

    /** Set content from ProseMirror JSON. Returns render elements. */
    setJson(doc: DocumentJSON): RenderElement[] {
        return this.setJsonString(JSON.stringify(normalizeDocumentJson(doc, this._schema)));
    }

    /** Set content from a serialized ProseMirror JSON string. Returns render elements. */
    setJsonString(jsonString: string): RenderElement[] {
        this.assertNotDestroyed();
        this.invalidateContentCaches();
        this._renderBlocksCache = null;
        const normalizedJsonString = normalizeDocumentJsonString(jsonString, this._schema);
        const json = getNativeModule().editorSetJson(this._editorId, normalizedJsonString);
        return parseRenderElements(json);
    }

    /** Get content as raw ProseMirror JSON string. */
    getJsonString(): string {
        this.assertNotDestroyed();
        if (this._cachedJsonString?.version === this._documentVersion) {
            return this._cachedJsonString.value;
        }
        const json = getNativeModule().editorGetJson(this._editorId);
        this._cachedJsonString = { version: this._documentVersion, value: json };
        return json;
    }

    /** Get content as ProseMirror JSON. */
    getJson(): DocumentJSON {
        return parseDocumentJSON(this.getJsonString());
    }

    /** Get both HTML and JSON content in one native roundtrip. */
    getContentSnapshot(): ContentSnapshot {
        this.assertNotDestroyed();
        if (
            this._cachedHtml?.version === this._documentVersion &&
            this._cachedJsonString?.version === this._documentVersion
        ) {
            return {
                html: this._cachedHtml.value,
                json: parseDocumentJSON(this._cachedJsonString.value),
            };
        }
        const snapshot = parseContentSnapshotJson(
            getNativeModule().editorGetContentSnapshot(this._editorId)
        );
        this._cachedHtml = { version: this._documentVersion, value: snapshot.html };
        this._cachedJsonString = {
            version: this._documentVersion,
            value: JSON.stringify(snapshot.json),
        };
        return snapshot;
    }

    /** Insert text at a document position. Returns the full update. */
    insertText(pos: number, text: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorInsertText(this._editorId, pos, text);
        return this.parseAndNoteUpdate(json);
    }

    /** Delete a range [from, to). Returns the full update. */
    deleteRange(from: number, to: number): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorDeleteRange(this._editorId, from, to);
        return this.parseAndNoteUpdate(json);
    }

    /** Replace the current selection with text atomically. */
    replaceSelectionText(text: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorReplaceSelectionText(this._editorId, text);
        return this.parseAndNoteUpdate(json);
    }

    /** Toggle a mark (bold, italic, etc.) on the current selection. */
    toggleMark(markType: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const scalarSelection = this.currentScalarSelection();
        const json = scalarSelection
            ? getNativeModule().editorToggleMarkAtSelectionScalar(
                  this._editorId,
                  scalarSelection.anchor,
                  scalarSelection.head,
                  markType
              )
            : getNativeModule().editorToggleMark(this._editorId, markType);
        return this.parseAndNoteUpdate(json);
    }

    /** Set a mark with attrs on the current selection. */
    setMark(markType: string, attrs: Record<string, unknown>): EditorUpdate | null {
        this.assertNotDestroyed();
        const attrsJson = JSON.stringify(attrs);
        const scalarSelection = this.currentScalarSelection();
        const json = scalarSelection
            ? getNativeModule().editorSetMarkAtSelectionScalar(
                  this._editorId,
                  scalarSelection.anchor,
                  scalarSelection.head,
                  markType,
                  attrsJson
              )
            : getNativeModule().editorSetMark(this._editorId, markType, attrsJson);
        return this.parseAndNoteUpdate(json);
    }

    /** Remove a mark from the current selection. */
    unsetMark(markType: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const scalarSelection = this.currentScalarSelection();
        const json = scalarSelection
            ? getNativeModule().editorUnsetMarkAtSelectionScalar(
                  this._editorId,
                  scalarSelection.anchor,
                  scalarSelection.head,
                  markType
              )
            : getNativeModule().editorUnsetMark(this._editorId, markType);
        return this.parseAndNoteUpdate(json);
    }

    /** Toggle blockquote wrapping for the current block selection. */
    toggleBlockquote(): EditorUpdate | null {
        this.assertNotDestroyed();
        const scalarSelection = this.currentScalarSelection();
        const json = scalarSelection
            ? getNativeModule().editorToggleBlockquoteAtSelectionScalar(
                  this._editorId,
                  scalarSelection.anchor,
                  scalarSelection.head
              )
            : getNativeModule().editorToggleBlockquote(this._editorId);
        return this.parseAndNoteUpdate(json);
    }

    /** Toggle a heading level on the current block selection. */
    toggleHeading(level: number): EditorUpdate | null {
        this.assertNotDestroyed();
        if (!Number.isInteger(level) || level < 1 || level > 6) {
            throw new Error('NativeEditorBridge: invalid heading level');
        }
        const scalarSelection = this.currentScalarSelection();
        const json = scalarSelection
            ? getNativeModule().editorToggleHeadingAtSelectionScalar(
                  this._editorId,
                  scalarSelection.anchor,
                  scalarSelection.head,
                  level
              )
            : getNativeModule().editorToggleHeading(this._editorId, level);
        return this.parseAndNoteUpdate(json);
    }

    /** Set the document selection by anchor and head positions. */
    setSelection(anchor: number, head: number): void {
        this.assertNotDestroyed();
        getNativeModule().editorSetSelection(this._editorId, anchor, head);
        this._lastSelection = { type: 'text', anchor, head };
    }

    /** Get the current selection from the Rust engine (synchronous native call).
     *  Always returns the live selection, not a stale cache. */
    getSelection(): Selection {
        if (this._destroyed) return { type: 'text', anchor: 0, head: 0 };
        try {
            const json = getNativeModule().editorGetSelection(this._editorId);
            const sel = JSON.parse(json) as Selection;
            this._lastSelection = sel;
            return sel;
        } catch {
            return this._lastSelection;
        }
    }

    /** Update the cached selection from native events (scalar offsets).
     *  Called by the React component when native selection change events arrive. */
    updateSelectionFromNative(anchor: number, head: number): void {
        if (this._destroyed) return;
        this._lastSelection = { type: 'text', anchor, head };
    }

    /** Get the current full state from Rust (render elements, selection, etc.). */
    getCurrentState(): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorGetCurrentState(this._editorId);
        return this.parseAndNoteUpdate(json);
    }

    /** Get the current selection-related state without render elements. */
    getSelectionState(): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorGetSelectionState(this._editorId);
        return this.parseAndNoteUpdate(json);
    }

    /** Split the block at a position (Enter key). */
    splitBlock(pos: number): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorSplitBlock(this._editorId, pos);
        return this.parseAndNoteUpdate(json);
    }

    /** Insert HTML content at the current selection. */
    insertContentHtml(html: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorInsertContentHtml(this._editorId, html);
        return this.parseAndNoteUpdate(json);
    }

    /** Insert JSON content at the current selection. */
    insertContentJson(doc: DocumentJSON): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorInsertContentJson(this._editorId, JSON.stringify(doc));
        return this.parseAndNoteUpdate(json);
    }

    /** Insert JSON content at an explicit scalar selection. */
    insertContentJsonAtSelectionScalar(
        scalarAnchor: number,
        scalarHead: number,
        doc: DocumentJSON
    ): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorInsertContentJsonAtSelectionScalar(
            this._editorId,
            scalarAnchor,
            scalarHead,
            JSON.stringify(doc)
        );
        return this.parseAndNoteUpdate(json);
    }

    /** Replace entire document with HTML via transaction (preserves undo history). */
    replaceHtml(html: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorReplaceHtml(this._editorId, html);
        return this.parseAndNoteUpdate(json);
    }

    /** Replace entire document with JSON via transaction (preserves undo history). */
    replaceJson(doc: DocumentJSON): EditorUpdate | null {
        return this.replaceJsonString(JSON.stringify(normalizeDocumentJson(doc, this._schema)));
    }

    /** Replace entire document with a serialized JSON transaction. */
    replaceJsonString(jsonString: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const normalizedJsonString = normalizeDocumentJsonString(jsonString, this._schema);
        const json = getNativeModule().editorReplaceJson(this._editorId, normalizedJsonString);
        return this.parseAndNoteUpdate(json);
    }

    /** Undo the last operation. Returns update or null if nothing to undo. */
    undo(): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorUndo(this._editorId);
        return this.parseAndNoteUpdate(json);
    }

    /** Redo the last undone operation. Returns update or null if nothing to redo. */
    redo(): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorRedo(this._editorId);
        return this.parseAndNoteUpdate(json);
    }

    /** Check if undo is available. */
    canUndo(): boolean {
        this.assertNotDestroyed();
        return getNativeModule().editorCanUndo(this._editorId);
    }

    /** Check if redo is available. */
    canRedo(): boolean {
        this.assertNotDestroyed();
        return getNativeModule().editorCanRedo(this._editorId);
    }

    /** Toggle a list type on the current selection. Wraps if not in list, unwraps if already in that list type. */
    toggleList(listType: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const isActive = this.getCurrentState()?.activeState?.nodes?.[listType] === true;
        const scalarSelection = this.currentScalarSelection();

        const json = isActive
            ? scalarSelection
                ? getNativeModule().editorUnwrapFromListAtSelectionScalar(
                      this._editorId,
                      scalarSelection.anchor,
                      scalarSelection.head
                  )
                : getNativeModule().editorUnwrapFromList(this._editorId)
            : scalarSelection
              ? getNativeModule().editorWrapInListAtSelectionScalar(
                    this._editorId,
                    scalarSelection.anchor,
                    scalarSelection.head,
                    listType
                )
              : getNativeModule().editorWrapInList(this._editorId, listType);
        return this.parseAndNoteUpdate(json);
    }

    /** Unwrap the current list item back to a paragraph. */
    unwrapFromList(): EditorUpdate | null {
        this.assertNotDestroyed();
        const scalarSelection = this.currentScalarSelection();
        const json = scalarSelection
            ? getNativeModule().editorUnwrapFromListAtSelectionScalar(
                  this._editorId,
                  scalarSelection.anchor,
                  scalarSelection.head
              )
            : getNativeModule().editorUnwrapFromList(this._editorId);
        return this.parseAndNoteUpdate(json);
    }

    /** Indent the current list item into a nested list. */
    indentListItem(): EditorUpdate | null {
        this.assertNotDestroyed();
        const scalarSelection = this.currentScalarSelection();
        const json = scalarSelection
            ? getNativeModule().editorIndentListItemAtSelectionScalar(
                  this._editorId,
                  scalarSelection.anchor,
                  scalarSelection.head
              )
            : getNativeModule().editorIndentListItem(this._editorId);
        return this.parseAndNoteUpdate(json);
    }

    /** Outdent the current list item to the parent list level. */
    outdentListItem(): EditorUpdate | null {
        this.assertNotDestroyed();
        const scalarSelection = this.currentScalarSelection();
        const json = scalarSelection
            ? getNativeModule().editorOutdentListItemAtSelectionScalar(
                  this._editorId,
                  scalarSelection.anchor,
                  scalarSelection.head
              )
            : getNativeModule().editorOutdentListItem(this._editorId);
        return this.parseAndNoteUpdate(json);
    }

    /** Insert a void node (e.g. 'horizontalRule') at the current selection. */
    insertNode(nodeType: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const scalarSelection = this.currentScalarSelection();
        const json = scalarSelection
            ? getNativeModule().editorInsertNodeAtSelectionScalar(
                  this._editorId,
                  scalarSelection.anchor,
                  scalarSelection.head,
                  nodeType
              )
            : getNativeModule().editorInsertNode(this._editorId, nodeType);
        return this.parseAndNoteUpdate(json);
    }

    parseUpdateJson(json: string): EditorUpdate | null {
        this.assertNotDestroyed();
        return this.parseAndNoteUpdate(json);
    }

    private noteUpdate(update: EditorUpdate | null): void {
        if (!update) {
            return;
        }
        this._lastSelection = update.selection;
        if (update.renderBlocks) {
            this._renderBlocksCache = update.renderBlocks;
        }
        if (typeof update.documentVersion !== 'number') {
            this.invalidateContentCaches();
            return;
        }
        if (update.documentVersion !== this._documentVersion) {
            this._documentVersion = update.documentVersion;
            this.invalidateContentCaches();
        }
    }

    private parseAndNoteUpdate(json: string): EditorUpdate | null {
        let update = parseEditorUpdateJson(json, this._renderBlocksCache ?? undefined);
        if (update?.renderPatch && !update.renderBlocks) {
            update = parseEditorUpdateJson(
                getNativeModule().editorGetCurrentState(this._editorId),
                this._renderBlocksCache ?? undefined
            );
        }
        this.noteUpdate(update);
        return update;
    }

    private invalidateContentCaches(): void {
        this._cachedHtml = null;
        this._cachedJsonString = null;
    }

    private assertNotDestroyed(): void {
        if (this._destroyed) {
            throw new Error(ERR_DESTROYED);
        }
    }

    private currentScalarSelection(): { anchor: number; head: number } | null {
        const selection = this._lastSelection;
        const nativeModule = getNativeModule();

        if (selection.type === 'text') {
            const anchor = selection.anchor ?? 0;
            const head = selection.head ?? anchor;
            return {
                anchor: nativeModule.editorDocToScalar(this._editorId, anchor),
                head: nativeModule.editorDocToScalar(this._editorId, head),
            };
        }

        if (selection.type === 'node' && typeof selection.pos === 'number') {
            const scalar = nativeModule.editorDocToScalar(this._editorId, selection.pos);
            return { anchor: scalar, head: scalar };
        }

        return null;
    }
}

export class NativeCollaborationBridge {
    private _sessionId: number;
    private _destroyed = false;

    private constructor(sessionId: number) {
        this._sessionId = sessionId;
    }

    static create(config?: {
        clientId?: number;
        fragmentName?: string;
        schema?: SchemaDefinition;
        initialDocumentJson?: DocumentJSON;
        initialEncodedState?: EncodedCollaborationStateInput;
        localAwareness?: Record<string, unknown>;
    }): NativeCollaborationBridge {
        const { initialEncodedState, ...normalizedConfig } = config ?? {};
        const id = getNativeModule().collaborationSessionCreate(JSON.stringify(normalizedConfig));
        const bridge = new NativeCollaborationBridge(id);
        if (initialEncodedState != null) {
            try {
                bridge.replaceEncodedState(initialEncodedState);
            } catch (error) {
                bridge.destroy();
                throw error;
            }
        }
        return bridge;
    }

    get sessionId(): number {
        return this._sessionId;
    }

    get isDestroyed(): boolean {
        return this._destroyed;
    }

    destroy(): void {
        if (this._destroyed) return;
        this._destroyed = true;
        getNativeModule().collaborationSessionDestroy(this._sessionId);
    }

    getDocumentJson(): DocumentJSON {
        this.assertNotDestroyed();
        return parseDocumentJSON(
            getNativeModule().collaborationSessionGetDocumentJson(this._sessionId)
        );
    }

    getEncodedState(): Uint8Array {
        this.assertNotDestroyed();
        return parseByteArrayJson(
            getNativeModule().collaborationSessionGetEncodedState(this._sessionId)
        );
    }

    getEncodedStateBase64(): string {
        return encodeCollaborationStateBase64(this.getEncodedState());
    }

    getPeers(): CollaborationPeer[] {
        this.assertNotDestroyed();
        return parseCollaborationPeersJson(
            getNativeModule().collaborationSessionGetPeersJson(this._sessionId)
        );
    }

    start(): CollaborationResult {
        this.assertNotDestroyed();
        return parseCollaborationResultJson(
            getNativeModule().collaborationSessionStart(this._sessionId)
        );
    }

    applyLocalDocumentJson(doc: DocumentJSON): CollaborationResult {
        this.assertNotDestroyed();
        return parseCollaborationResultJson(
            getNativeModule().collaborationSessionApplyLocalDocumentJson(
                this._sessionId,
                JSON.stringify(doc)
            )
        );
    }

    applyEncodedState(encodedState: EncodedCollaborationStateInput): CollaborationResult {
        this.assertNotDestroyed();
        return parseCollaborationResultJson(
            getNativeModule().collaborationSessionApplyEncodedState(
                this._sessionId,
                JSON.stringify(normalizeEncodedStateInput(encodedState))
            )
        );
    }

    replaceEncodedState(encodedState: EncodedCollaborationStateInput): CollaborationResult {
        this.assertNotDestroyed();
        return parseCollaborationResultJson(
            getNativeModule().collaborationSessionReplaceEncodedState(
                this._sessionId,
                JSON.stringify(normalizeEncodedStateInput(encodedState))
            )
        );
    }

    handleMessage(bytes: readonly number[]): CollaborationResult {
        this.assertNotDestroyed();
        return parseCollaborationResultJson(
            getNativeModule().collaborationSessionHandleMessage(
                this._sessionId,
                JSON.stringify(Array.from(bytes))
            )
        );
    }

    setLocalAwareness(state: Record<string, unknown>): CollaborationResult {
        this.assertNotDestroyed();
        return parseCollaborationResultJson(
            getNativeModule().collaborationSessionSetLocalAwareness(
                this._sessionId,
                JSON.stringify(state)
            )
        );
    }

    clearLocalAwareness(): CollaborationResult {
        this.assertNotDestroyed();
        return parseCollaborationResultJson(
            getNativeModule().collaborationSessionClearLocalAwareness(this._sessionId)
        );
    }

    private assertNotDestroyed(): void {
        if (this._destroyed) {
            throw new Error(ERR_DESTROYED);
        }
    }
}
