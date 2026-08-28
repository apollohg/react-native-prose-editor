import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
    _assertNativeEditorDocumentHandle,
    _getNativeEditorDocumentHandleDescriptor,
    type DocumentJSON,
    type HistoryState,
    type NativeEditorDocumentHandle,
    type NativeEditorV2DocumentOrigin,
    type NativeEditorV2DocumentState,
} from './NativeEditorBridge';
import { NativeEditorV2OperationError } from './NativeEditorBoundaryError';

//
// Headless document/control binding over a shared NativeEditorDocumentHandle
// (one native session per handle; the collaboration controller attaches to
// the same one). It mirrors the editor's retained document API — controlled
// `value`/`valueJSON` with the replace/reset history policy, `setContent`,
// `setContentJson`, `getContent`, `getContentJson` — through the v2 engine:
//
// - `valueJSONUpdateMode="replace"` lowers to one undoable local-API
//   replacement (`undoableBoundary`); `"reset"` clears history
//   (`resetAndClear`). Imperative setContent/setContentJson are undoable;
//   clearContent is a non-undoable reset.
// - A room document in `AwaitRemote` renders as not-ready; content getters
//   return empty values until Rust promotes an accepted server Step 2.
// - Remote commits arrive through `revisionSignal` (the collaboration
//   controller's rendered revision): the binding re-reads the engine and
//   emits content callbacks. It never pushes document state anywhere.
// - Controlled applies carry the last rendered engine revision as their base;
//   a REVISION_MISMATCH refreshes from the engine and the effect re-applies
//   against the fresh revision — never against guessed positions.

/** Configuration for {@link useNativeEditorDocument}. */
export interface UseNativeEditorDocumentOptions {
    /** The shared document session. The binding never creates or destroys it. */
    handle: NativeEditorDocumentHandle;
    /** Controlled HTML content. */
    value?: string;
    /** Controlled ProseMirror JSON content (already normalized by the caller). Ignored when value is set. */
    valueJSON?: DocumentJSON;
    /** History policy for controlled replacements. Defaults to 'replace'. */
    valueJSONUpdateMode?: 'replace' | 'reset';
    /** External revision signal (e.g. from the collaboration controller) forcing an engine re-read. */
    revisionSignal?: string | null;
    onContentChange?: (html: string) => void;
    onContentChangeJSON?: (json: DocumentJSON) => void;
    onHistoryStateChange?: (state: HistoryState) => void;
    /** Application notification after a successful local mutation. Transport does not depend on it. */
    onLocalCommit?: () => void;
}

/** What {@link useNativeEditorDocument} returns. */
export interface UseNativeEditorDocumentReturn {
    /** True once the engine document is ready (LocalReady/RoomReady). */
    isReady: boolean;
    /** Raw engine readiness. Null before the first read. */
    documentState: NativeEditorV2DocumentState | null;
    /** Decimal-string engine document revision; null while awaiting the server document. */
    documentRevision: string | null;
    /** Trusted origin paired with documentRevision. */
    documentOrigin: NativeEditorV2DocumentOrigin | null;
    /** Current undo/redo availability. */
    historyState: HistoryState;
    /**
     * Re-read the engine now, emitting content/history callbacks when the
     * engine advanced. The interactive editor calls this when the native
     * view reports an adapter-driven commit (typing, toolbar commands).
     */
    refresh(): void;
    /** Current document as HTML. Empty until the document is ready. */
    getContent(): string;
    /** Current document as ProseMirror JSON. Empty until the document is ready. */
    getContentJson(): DocumentJSON;
    /** The engine's own answer for whether the document holds no content. */
    getIsEmpty(): boolean;
    /** Current document as plain text, with markup stripped. */
    getTextContent(): string;
    /** Replace the document with HTML, as one undoable step. */
    setContent(html: string): void;
    /** Replace the document with JSON, as one undoable step. */
    setContentJson(doc: DocumentJSON): void;
    /** Reset to the schema's empty document and clear undo history. */
    clearContent(): void;
    undo(): void;
    redo(): void;
    canUndo(): boolean;
    canRedo(): boolean;
}

const DEFAULT_V2_HISTORY_STATE: HistoryState = { canUndo: false, canRedo: false };

function isRevisionMismatchError(error: unknown): boolean {
    return error instanceof NativeEditorV2OperationError && error.code === 'REVISION_MISMATCH';
}

interface V2EngineView {
    editorId: string;
    ready: boolean;
    documentState: NativeEditorV2DocumentState | null;
    documentRevision: string | null;
    documentOrigin: NativeEditorV2DocumentOrigin | null;
    historyState: HistoryState;
    contentKey: string | null;
}

/**
 * Headless binding to a document handle: the same controlled content,
 * getters, and history the editor exposes, with no view attached. Use it to
 * read or drive a document that is not currently mounted — a preview, a
 * background save, a test.
 *
 * `NativeRichTextEditor` uses this internally, so mounting an editor on the
 * same handle does not conflict with it.
 */
export function useNativeEditorDocument(
    options: UseNativeEditorDocumentOptions
): UseNativeEditorDocumentReturn {
    const {
        handle,
        value,
        valueJSON,
        valueJSONUpdateMode = 'replace',
        revisionSignal,
        onLocalCommit,
    } = options;

    _assertNativeEditorDocumentHandle(handle);
    const editorId = handle.editorId;

    const onContentChangeRef = useRef(options.onContentChange);
    onContentChangeRef.current = options.onContentChange;
    const onContentChangeJSONRef = useRef(options.onContentChangeJSON);
    onContentChangeJSONRef.current = options.onContentChangeJSON;
    const onHistoryStateChangeRef = useRef(options.onHistoryStateChange);
    onHistoryStateChangeRef.current = options.onHistoryStateChange;
    const onLocalCommitRef = useRef(onLocalCommit);
    onLocalCommitRef.current = onLocalCommit;

    const [engineView, setEngineView] = useState<V2EngineView>({
        editorId,
        ready: false,
        documentState: null,
        documentRevision: null,
        documentOrigin: null,
        historyState: DEFAULT_V2_HISTORY_STATE,
        contentKey: null,
    });
    const currentEditorIdRef = useRef(editorId);
    currentEditorIdRef.current = editorId;
    const readyRef = useRef({ editorId, ready: false });
    const emittedRef = useRef({
        editorId,
        lastRevision: null as string | null,
        lastContentKey: null as string | null,
        lastHistoryKey: null as string | null,
    });
    const pendingControlledEchoesRef = useRef({
        editorId,
        keys: [] as string[],
    });
    const controlledKindRef = useRef<'html' | 'json' | null>(null);
    controlledKindRef.current = value != null ? 'html' : valueJSON != null ? 'json' : null;
    const ignoredControlledEchoRef = useRef<string | null>(null);
    const lastControlledContentKeyRef = useRef<string | null>(null);
    if (readyRef.current.editorId !== editorId) {
        readyRef.current = { editorId, ready: false };
    }
    if (emittedRef.current.editorId !== editorId) {
        emittedRef.current = {
            editorId,
            lastRevision: null,
            lastContentKey: null,
            lastHistoryKey: null,
        };
        pendingControlledEchoesRef.current = { editorId, keys: [] };
        ignoredControlledEchoRef.current = null;
        lastControlledContentKeyRef.current = null;
    }

    const refresh = useCallback(
        (emitContentCallbacks: boolean) => {
            const refreshedEditorId = handle.editorId;
            if (handle.isDestroyed || currentEditorIdRef.current !== refreshedEditorId) {
                return;
            }
            const state = handle.bridge.getState();
            const ready = state.documentState !== 'AwaitRemote';
            let contentKey: string | null = null;
            let snapshotHtml: string | null = null;
            let snapshotJson: DocumentJSON | null = null;
            if (ready) {
                const snapshot = handle.bridge.getContentSnapshot();
                snapshotHtml = snapshot.html;
                snapshotJson = snapshot.json;
                contentKey = JSON.stringify(snapshot.json);
            }
            if (currentEditorIdRef.current !== refreshedEditorId) return;
            const historyState: HistoryState = {
                canUndo: state.canUndo,
                canRedo: state.canRedo,
            };
            readyRef.current = { editorId: refreshedEditorId, ready };
            setEngineView({
                editorId: refreshedEditorId,
                ready,
                documentState: state.documentState,
                documentRevision: state.documentRevision,
                documentOrigin: state.documentOrigin,
                historyState,
                contentKey,
            });

            const historyKey = `${state.canUndo}/${state.canRedo}`;
            const emitted = emittedRef.current;
            if (
                currentEditorIdRef.current !== refreshedEditorId ||
                emitted.editorId !== refreshedEditorId
            ) {
                return;
            }
            if (historyKey !== emitted.lastHistoryKey) {
                emitted.lastHistoryKey = historyKey;
                onHistoryStateChangeRef.current?.(historyState);
            }

            if (ready) {
                const lastRevision = emitted.lastRevision;
                if (
                    emitContentCallbacks &&
                    lastRevision != null &&
                    lastRevision !== state.documentRevision &&
                    emitted.lastContentKey !== contentKey
                ) {
                    if (snapshotHtml != null) {
                        if (
                            controlledKindRef.current === 'html' &&
                            onContentChangeRef.current != null
                        ) {
                            const echoes = pendingControlledEchoesRef.current.keys;
                            echoes.push(`html:${snapshotHtml}`);
                            if (echoes.length > 32) echoes.shift();
                        }
                        onContentChangeRef.current?.(snapshotHtml);
                    }
                    if (snapshotJson != null) {
                        if (
                            controlledKindRef.current === 'json' &&
                            onContentChangeJSONRef.current != null
                        ) {
                            const echoes = pendingControlledEchoesRef.current.keys;
                            echoes.push(`json:${contentKey}`);
                            if (echoes.length > 32) echoes.shift();
                        }
                        onContentChangeJSONRef.current?.(snapshotJson);
                    }
                }
                emitted.lastRevision = state.documentRevision;
                emitted.lastContentKey = contentKey;
            }
        },
        [handle]
    );

    useEffect(() => {
        refresh(true);
    }, [refresh, revisionSignal]);

    const refreshFromEngine = useCallback(() => {
        refresh(true);
    }, [refresh]);

    const serializedControlledJson = useMemo(
        () => (value == null && valueJSON != null ? JSON.stringify(valueJSON) : null),
        [value, valueJSON]
    );

    // Controlled document flow: external value/valueJSON changes lower to one
    // local-API replacement carrying the rendered engine revision as its
    // base. Content callbacks stay suppressed (controlled sync parity).
    useEffect(() => {
        if (engineView.editorId !== editorId || !engineView.ready || handle.isDestroyed) return;
        if (engineView.documentRevision == null) return;
        const wantsHtml = value != null;
        const wantsJson = value == null && serializedControlledJson != null;
        if (!wantsHtml && !wantsJson) return;

        const controlledContentKey = wantsHtml
            ? `html:${value}`
            : `json:${serializedControlledJson}`;
        const controlledContentChanged =
            lastControlledContentKeyRef.current !== controlledContentKey;
        if (controlledContentChanged) {
            lastControlledContentKeyRef.current = controlledContentKey;
            ignoredControlledEchoRef.current = null;
        }
        if (ignoredControlledEchoRef.current === controlledContentKey) return;
        if (!controlledContentChanged && pendingControlledEchoesRef.current.keys.length > 0) return;

        const snapshot = handle.bridge.getContentSnapshot();
        if (currentEditorIdRef.current !== editorId || emittedRef.current.editorId !== editorId) {
            return;
        }
        const currentJsonKey = JSON.stringify(snapshot.json);
        if (wantsHtml && snapshot.html === value) {
            const echoIndex = pendingControlledEchoesRef.current.keys.indexOf(controlledContentKey);
            if (echoIndex >= 0) {
                pendingControlledEchoesRef.current.keys.splice(0, echoIndex + 1);
            }
            emittedRef.current.lastRevision = engineView.documentRevision;
            emittedRef.current.lastContentKey = currentJsonKey;
            return;
        }
        if (wantsJson && currentJsonKey === serializedControlledJson) {
            const echoIndex = pendingControlledEchoesRef.current.keys.indexOf(controlledContentKey);
            if (echoIndex >= 0) {
                pendingControlledEchoesRef.current.keys.splice(0, echoIndex + 1);
            }
            emittedRef.current.lastRevision = engineView.documentRevision;
            emittedRef.current.lastContentKey = currentJsonKey;
            return;
        }

        const echoIndex = pendingControlledEchoesRef.current.keys.indexOf(controlledContentKey);
        if (echoIndex >= 0) {
            pendingControlledEchoesRef.current.keys.splice(0, echoIndex + 1);
            ignoredControlledEchoRef.current = controlledContentKey;
            return;
        }

        try {
            pendingControlledEchoesRef.current.keys = [];
            handle.bridge.applyLocalApi({
                ...(wantsHtml
                    ? { setHtml: value }
                    : { setJson: JSON.parse(serializedControlledJson!) as DocumentJSON }),
                history: valueJSONUpdateMode === 'reset' ? 'resetAndClear' : 'undoableBoundary',
                baseDocumentRevision: engineView.documentRevision,
            });
            if (currentEditorIdRef.current !== editorId) return;
            onLocalCommitRef.current?.();
            refresh(false);
        } catch (error) {
            if (isRevisionMismatchError(error)) {
                // Refresh from the engine; the effect re-applies against the
                // fresh revision (never a guessed one).
                refresh(false);
                return;
            }
            throw error;
        }
    }, [
        handle,
        value,
        serializedControlledJson,
        valueJSONUpdateMode,
        editorId,
        engineView.editorId,
        engineView.ready,
        engineView.documentRevision,
        refresh,
    ]);

    const mutate = useCallback(
        (operation: () => unknown) => {
            if (handle.isDestroyed || currentEditorIdRef.current !== handle.editorId) {
                if (__DEV__) {
                    console.error(
                        'useNativeEditorDocument: dropped a mutation because the handle is not the current binding',
                        {
                            destroyed: handle.isDestroyed,
                            boundEditorId: currentEditorIdRef.current,
                            handleEditorId: handle.editorId,
                        }
                    );
                }
                return;
            }
            operation();
            if (currentEditorIdRef.current !== handle.editorId) return;
            onLocalCommitRef.current?.();
            refresh(true);
        },
        [handle, refresh]
    );

    const setContent = useCallback(
        (html: string) => {
            mutate(() =>
                handle.bridge.replaceDocument({ setHtml: html, history: 'undoableBoundary' })
            );
        },
        [handle, mutate]
    );

    const setContentJson = useCallback(
        (doc: DocumentJSON) => {
            mutate(() =>
                handle.bridge.replaceDocument({ setJson: doc, history: 'undoableBoundary' })
            );
        },
        [handle, mutate]
    );

    const clearContent = useCallback(() => {
        mutate(() => {
            return handle.bridge.replaceDocument({
                setJson: _getNativeEditorDocumentHandleDescriptor(handle).emptyDocument,
                history: 'resetAndClear',
            });
        });
    }, [handle, mutate]);

    const undo = useCallback(() => {
        mutate(() => handle.bridge.undo());
    }, [handle, mutate]);

    const redo = useCallback(() => {
        mutate(() => handle.bridge.redo());
    }, [handle, mutate]);

    const getContent = useCallback((): string => {
        if (
            handle.isDestroyed ||
            readyRef.current.editorId !== handle.editorId ||
            !readyRef.current.ready
        ) {
            return '';
        }
        return handle.bridge.getDocumentHtml();
    }, [handle]);

    const getContentJson = useCallback((): DocumentJSON => {
        if (
            handle.isDestroyed ||
            readyRef.current.editorId !== handle.editorId ||
            !readyRef.current.ready
        ) {
            return {};
        }
        return handle.bridge.getDocumentJson();
    }, [handle]);

    const getIsEmpty = useCallback((): boolean => {
        if (
            handle.isDestroyed ||
            readyRef.current.editorId !== handle.editorId ||
            !readyRef.current.ready
        ) {
            return true;
        }
        return handle.bridge.renderUpdate().documentIsEmpty;
    }, [handle]);

    const getTextContent = useCallback((): string => {
        if (
            handle.isDestroyed ||
            readyRef.current.editorId !== handle.editorId ||
            !readyRef.current.ready
        ) {
            return '';
        }
        return handle.bridge.getDocumentHtml().replace(/<[^>]+>/g, '');
    }, [handle]);

    const canUndo = useCallback((): boolean => {
        if (
            handle.isDestroyed ||
            readyRef.current.editorId !== handle.editorId ||
            !readyRef.current.ready
        ) {
            return false;
        }
        return handle.bridge.getState().canUndo;
    }, [handle]);

    const canRedo = useCallback((): boolean => {
        if (
            handle.isDestroyed ||
            readyRef.current.editorId !== handle.editorId ||
            !readyRef.current.ready
        ) {
            return false;
        }
        return handle.bridge.getState().canRedo;
    }, [handle]);

    let renderedEngineView = engineView;
    if (engineView.editorId !== editorId) {
        if (handle.isDestroyed) {
            renderedEngineView = {
                editorId,
                ready: false,
                documentState: null,
                documentRevision: null,
                documentOrigin: null,
                historyState: DEFAULT_V2_HISTORY_STATE,
                contentKey: null,
            };
        } else {
            const state = handle.bridge.getState();
            renderedEngineView = {
                editorId,
                ready: state.documentState !== 'AwaitRemote',
                documentState: state.documentState,
                documentRevision: state.documentRevision,
                documentOrigin: state.documentOrigin,
                historyState: { canUndo: state.canUndo, canRedo: state.canRedo },
                contentKey: null,
            };
        }
        readyRef.current = { editorId, ready: renderedEngineView.ready };
    }

    return {
        isReady: renderedEngineView.ready,
        documentState: renderedEngineView.documentState,
        documentRevision: renderedEngineView.documentRevision,
        documentOrigin: renderedEngineView.documentOrigin,
        historyState: renderedEngineView.historyState,
        refresh: refreshFromEngine,
        getContent,
        getContentJson,
        getIsEmpty,
        getTextContent,
        setContent,
        setContentJson,
        clearContent,
        undo,
        redo,
        canUndo,
        canRedo,
    };
}
