import { requireNativeModule } from 'expo-modules-core';

const ERR_DESTROYED = 'NativeEditorBridge: editor has been destroyed';
const ERR_NATIVE_RESPONSE = 'NativeEditorBridge: invalid JSON response from native module';

export interface NativeEditorModule {
    editorCreate(configJson: string): number;
    editorDestroy(editorId: number): void;
    editorSetHtml(editorId: number, html: string): string;
    editorGetHtml(editorId: number): string;
    editorSetJson(editorId: number, json: string): string;
    editorGetJson(editorId: number): string;
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
    editorSetSelection(editorId: number, anchor: number, head: number): void;
    editorGetSelection(editorId: number): string;
    editorGetCurrentState(editorId: number): string;
    // Scalar-position APIs (used by native views internally)
    editorInsertTextScalar(editorId: number, scalarPos: number, text: string): string;
    editorDeleteScalarRange(editorId: number, scalarFrom: number, scalarTo: number): string;
    editorReplaceTextScalar(editorId: number, scalarFrom: number, scalarTo: number, text: string): string;
    editorSplitBlockScalar(editorId: number, scalarPos: number): string;
    editorDeleteAndSplitScalar(editorId: number, scalarFrom: number, scalarTo: number): string;
    editorSetSelectionScalar(editorId: number, scalarAnchor: number, scalarHead: number): void;
    editorToggleMarkAtSelectionScalar(
        editorId: number,
        scalarAnchor: number,
        scalarHead: number,
        markName: string
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
    marks?: string[];
    nodeType?: string;
    depth?: number;
    docPos?: number;
    label?: string;
    listContext?: ListContext;
}

export interface ActiveState {
    marks: Record<string, boolean>;
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
    selection: Selection;
    activeState: ActiveState;
    historyState: HistoryState;
}

export interface DocumentJSON {
    [key: string]: unknown;
}

export function normalizeActiveState(raw: unknown): ActiveState {
    const obj = (raw as Record<string, unknown>) ?? {};
    return {
        marks: (obj.marks ?? {}) as Record<string, boolean>,
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

export function parseEditorUpdateJson(json: string): EditorUpdate | null {
    if (!json || json === '') return null;
    try {
        const parsed = JSON.parse(json) as Record<string, unknown>;
        if ('error' in parsed) {
            throw new Error(`NativeEditorBridge: ${parsed.error}`);
        }
        return {
            renderElements: (parsed.renderElements ?? []) as RenderElement[],
            selection: (parsed.selection ?? { type: 'text', anchor: 0, head: 0 }) as Selection,
            activeState: normalizeActiveState(parsed.activeState),
            historyState: (parsed.historyState ?? {
                canUndo: false,
                canRedo: false,
            }) as HistoryState,
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
            throw new Error(
                `NativeEditorBridge: ${(parsed as Record<string, unknown>).error}`
            );
        }
        return parsed;
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
    private _destroyed = false;
    private _lastSelection: Selection = { type: 'text', anchor: 0, head: 0 };

    private constructor(editorId: number) {
        this._editorId = editorId;
    }

    /** Create a new editor instance backed by the Rust engine. */
    static create(config?: { maxLength?: number; schemaJson?: string }): NativeEditorBridge {
        const configObj: Record<string, unknown> = {};
        if (config?.maxLength != null) configObj.maxLength = config.maxLength;
        if (config?.schemaJson != null) {
            try {
                configObj.schema = JSON.parse(config.schemaJson);
            } catch {
                // Fall back to the default schema when the provided JSON is invalid.
            }
        }
        const id = getNativeModule().editorCreate(JSON.stringify(configObj));
        return new NativeEditorBridge(id);
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
        getNativeModule().editorDestroy(this._editorId);
    }

    /** Set content from HTML. Returns render elements for display. */
    setHtml(html: string): RenderElement[] {
        this.assertNotDestroyed();
        const json = getNativeModule().editorSetHtml(this._editorId, html);
        return parseRenderElements(json);
    }

    /** Get content as HTML. */
    getHtml(): string {
        this.assertNotDestroyed();
        return getNativeModule().editorGetHtml(this._editorId);
    }

    /** Set content from ProseMirror JSON. Returns render elements. */
    setJson(doc: DocumentJSON): RenderElement[] {
        this.assertNotDestroyed();
        const json = getNativeModule().editorSetJson(
            this._editorId,
            JSON.stringify(doc)
        );
        return parseRenderElements(json);
    }

    /** Get content as ProseMirror JSON. */
    getJson(): DocumentJSON {
        this.assertNotDestroyed();
        const json = getNativeModule().editorGetJson(this._editorId);
        return parseDocumentJSON(json);
    }

    /** Insert text at a document position. Returns the full update. */
    insertText(pos: number, text: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorInsertText(this._editorId, pos, text);
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
    }

    /** Delete a range [from, to). Returns the full update. */
    deleteRange(from: number, to: number): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorDeleteRange(this._editorId, from, to);
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
    }

    /** Replace the current selection with text atomically. */
    replaceSelectionText(text: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorReplaceSelectionText(this._editorId, text);
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
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
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
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
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
    }

    /** Split the block at a position (Enter key). */
    splitBlock(pos: number): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorSplitBlock(this._editorId, pos);
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
    }

    /** Insert HTML content at the current selection. */
    insertContentHtml(html: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorInsertContentHtml(this._editorId, html);
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
    }

    /** Insert JSON content at the current selection. */
    insertContentJson(doc: DocumentJSON): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorInsertContentJson(
            this._editorId,
            JSON.stringify(doc)
        );
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
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
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
    }

    /** Replace entire document with HTML via transaction (preserves undo history). */
    replaceHtml(html: string): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorReplaceHtml(this._editorId, html);
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
    }

    /** Replace entire document with JSON via transaction (preserves undo history). */
    replaceJson(doc: DocumentJSON): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorReplaceJson(
            this._editorId,
            JSON.stringify(doc)
        );
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
    }

    /** Undo the last operation. Returns update or null if nothing to undo. */
    undo(): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorUndo(this._editorId);
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
    }

    /** Redo the last undone operation. Returns update or null if nothing to redo. */
    redo(): EditorUpdate | null {
        this.assertNotDestroyed();
        const json = getNativeModule().editorRedo(this._editorId);
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
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

        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
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
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
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
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
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
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
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
        const update = parseEditorUpdateJson(json);
        if (update) this._lastSelection = update.selection;
        return update;
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
