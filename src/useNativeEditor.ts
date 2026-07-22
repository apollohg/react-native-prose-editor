import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import {
    _assertNativeEditorDocumentHandle,
    _getNativeEditorDocumentHandleDescriptor,
    type DocumentJSON,
    type HistoryState,
    type NativeEditorDocumentHandle,
    type NativeEditorV2DocumentState,
} from './NativeEditorBridge';
import { NativeEditorV2OperationError } from './NativeEditorBoundaryError';

// ─── v2 document binding ────────────────────────────────────────
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
    /** Pinged after each successful local engine mutation so collaboration can flush outbound frames. */
    onLocalDocumentCommit?: () => void;
}

export interface UseNativeEditorDocumentReturn {
    /** True once the engine document is ready (LocalReady/RoomReady). */
    isReady: boolean;
    documentState: NativeEditorV2DocumentState | null;
    /** Decimal-string engine document revision; null while awaiting the server document. */
    documentRevision: string | null;
    historyState: HistoryState;
    /**
     * Re-read the engine now, emitting content/history callbacks when the
     * engine advanced. The interactive editor calls this when the native
     * view reports an adapter-driven commit (typing, toolbar commands).
     */
    refresh(): void;
    getContent(): string;
    getContentJson(): DocumentJSON;
    getTextContent(): string;
    setContent(html: string): void;
    setContentJson(doc: DocumentJSON): void;
    clearContent(): void;
    undo(): void;
    redo(): void;
    canUndo(): boolean;
    canRedo(): boolean;
}

const DEFAULT_V2_HISTORY_STATE: HistoryState = { canUndo: false, canRedo: false };

function isRevisionMismatchError(error: unknown): boolean {
    return (
        error instanceof NativeEditorV2OperationError && error.code === 'REVISION_MISMATCH'
    );
}

interface V2EngineView {
    ready: boolean;
    documentState: NativeEditorV2DocumentState | null;
    documentRevision: string | null;
    historyState: HistoryState;
    contentKey: string | null;
}

export function useNativeEditorDocument(
    options: UseNativeEditorDocumentOptions
): UseNativeEditorDocumentReturn {
    const {
        handle,
        value,
        valueJSON,
        valueJSONUpdateMode = 'replace',
        revisionSignal,
        onLocalDocumentCommit,
    } = options;

    _assertNativeEditorDocumentHandle(handle);

    const onContentChangeRef = useRef(options.onContentChange);
    onContentChangeRef.current = options.onContentChange;
    const onContentChangeJSONRef = useRef(options.onContentChangeJSON);
    onContentChangeJSONRef.current = options.onContentChangeJSON;
    const onHistoryStateChangeRef = useRef(options.onHistoryStateChange);
    onHistoryStateChangeRef.current = options.onHistoryStateChange;
    const onLocalDocumentCommitRef = useRef(onLocalDocumentCommit);
    onLocalDocumentCommitRef.current = onLocalDocumentCommit;

    const [engineView, setEngineView] = useState<V2EngineView>({
        ready: false,
        documentState: null,
        documentRevision: null,
        historyState: DEFAULT_V2_HISTORY_STATE,
        contentKey: null,
    });
    const readyRef = useRef(false);
    const lastEmittedRevisionRef = useRef<string | null>(null);
    const lastEmittedContentKeyRef = useRef<string | null>(null);
    const lastEmittedHistoryKeyRef = useRef<string | null>(null);

    const refresh = useCallback(
        (emitContentCallbacks: boolean) => {
            if (handle.isDestroyed) return;
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
            const historyState: HistoryState = {
                canUndo: state.canUndo,
                canRedo: state.canRedo,
            };
            readyRef.current = ready;
            setEngineView({
                ready,
                documentState: state.documentState,
                documentRevision: state.documentRevision,
                historyState,
                contentKey,
            });

            const historyKey = `${state.canUndo}/${state.canRedo}`;
            if (historyKey !== lastEmittedHistoryKeyRef.current) {
                lastEmittedHistoryKeyRef.current = historyKey;
                onHistoryStateChangeRef.current?.(historyState);
            }

            if (ready) {
                const lastRevision = lastEmittedRevisionRef.current;
                if (
                    emitContentCallbacks &&
                    lastRevision != null &&
                    lastRevision !== state.documentRevision &&
                    lastEmittedContentKeyRef.current !== contentKey
                ) {
                    if (snapshotHtml != null) {
                        onContentChangeRef.current?.(snapshotHtml);
                    }
                    if (snapshotJson != null) {
                        onContentChangeJSONRef.current?.(snapshotJson);
                    }
                }
                lastEmittedRevisionRef.current = state.documentRevision;
                lastEmittedContentKeyRef.current = contentKey;
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
        if (!engineView.ready || handle.isDestroyed) return;
        if (engineView.documentRevision == null) return;
        const wantsHtml = value != null;
        const wantsJson = value == null && serializedControlledJson != null;
        if (!wantsHtml && !wantsJson) return;

        const snapshot = handle.bridge.getContentSnapshot();
        const currentJsonKey = JSON.stringify(snapshot.json);
        if (wantsHtml && snapshot.html === value) {
            lastEmittedRevisionRef.current = engineView.documentRevision;
            lastEmittedContentKeyRef.current = currentJsonKey;
            return;
        }
        if (wantsJson && currentJsonKey === serializedControlledJson) {
            lastEmittedRevisionRef.current = engineView.documentRevision;
            lastEmittedContentKeyRef.current = currentJsonKey;
            return;
        }

        try {
            handle.bridge.applyLocalApi({
                ...(wantsHtml
                    ? { setHtml: value }
                    : { setJson: JSON.parse(serializedControlledJson!) as DocumentJSON }),
                history: valueJSONUpdateMode === 'reset' ? 'resetAndClear' : 'undoableBoundary',
                baseDocumentRevision: engineView.documentRevision,
            });
            onLocalDocumentCommitRef.current?.();
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
        engineView.ready,
        engineView.documentRevision,
        refresh,
    ]);

    const mutate = useCallback(
        (operation: () => unknown) => {
            if (handle.isDestroyed) return;
            operation();
            onLocalDocumentCommitRef.current?.();
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
        mutate(() =>
            handle.bridge.replaceDocument({
                setJson: _getNativeEditorDocumentHandleDescriptor(handle).emptyDocument,
                history: 'resetAndClear',
            })
        );
    }, [handle, mutate]);

    const undo = useCallback(() => {
        mutate(() => handle.bridge.undo());
    }, [handle, mutate]);

    const redo = useCallback(() => {
        mutate(() => handle.bridge.redo());
    }, [handle, mutate]);

    const getContent = useCallback((): string => {
        if (handle.isDestroyed || !readyRef.current) return '';
        return handle.bridge.getDocumentHtml();
    }, [handle]);

    const getContentJson = useCallback((): DocumentJSON => {
        if (handle.isDestroyed || !readyRef.current) return {};
        return handle.bridge.getDocumentJson();
    }, [handle]);

    const getTextContent = useCallback((): string => {
        if (handle.isDestroyed || !readyRef.current) return '';
        return handle.bridge.getDocumentHtml().replace(/<[^>]+>/g, '');
    }, [handle]);

    const canUndo = useCallback((): boolean => {
        if (handle.isDestroyed) return false;
        return handle.bridge.getState().canUndo;
    }, [handle]);

    const canRedo = useCallback((): boolean => {
        if (handle.isDestroyed) return false;
        return handle.bridge.getState().canRedo;
    }, [handle]);

    return {
        isReady: engineView.ready,
        documentState: engineView.documentState,
        documentRevision: engineView.documentRevision,
        historyState: engineView.historyState,
        refresh: refreshFromEngine,
        getContent,
        getContentJson,
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
