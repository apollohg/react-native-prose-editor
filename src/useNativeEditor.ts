import { useCallback, useEffect, useRef, useState } from 'react';

import {
    NativeEditorBridge,
    type ActiveState,
    type EditorUpdate,
    type HistoryState,
    type RenderElement,
    type Selection,
} from './NativeEditorBridge';

export interface UseNativeEditorOptions {
    /** Maximum character length. Omit for no limit. */
    maxLength?: number;
    /** Initial HTML content to load after creation. */
    initialHtml?: string;
    /** Called when content changes after an editing operation. */
    onChange?: (html: string) => void;
    /** Called when selection changes. */
    onSelectionChange?: (selection: Selection) => void;
}

export interface UseNativeEditorReturn {
    /** The underlying bridge instance, or null before creation. */
    bridge: NativeEditorBridge | null;
    /** Whether the editor has been created and is ready. */
    isReady: boolean;
    /** Current selection state. */
    selection: Selection;
    /** Currently active marks/nodes at the selection. */
    activeState: ActiveState;
    /** Current undo/redo availability. */
    historyState: HistoryState;
    /** Current render elements. */
    renderElements: RenderElement[];
    /** Toggle a mark (e.g. 'bold', 'italic', 'underline'). */
    toggleMark: (markType: string) => void;
    /** Undo the last operation. */
    undo: () => void;
    /** Redo the last undone operation. */
    redo: () => void;
    /** Toggle blockquote wrapping around the current block selection. */
    toggleBlockquote: () => void;
    /** Toggle a heading level on the current block selection. */
    toggleHeading: (level: 1 | 2 | 3 | 4 | 5 | 6) => void;
    /** Insert text at a position. */
    insertText: (pos: number, text: string) => void;
    /** Delete a range [from, to). */
    deleteRange: (from: number, to: number) => void;
    /** Get the current HTML content. */
    getHtml: () => string;
}

const DEFAULT_SELECTION: Selection = { type: 'text', anchor: 0, head: 0 };
const DEFAULT_ACTIVE_STATE: ActiveState = {
    marks: {},
    markAttrs: {},
    nodes: {},
    commands: {},
    allowedMarks: [],
    insertableNodes: [],
};
const DEFAULT_HISTORY_STATE: HistoryState = { canUndo: false, canRedo: false };

export function useNativeEditor(options: UseNativeEditorOptions = {}): UseNativeEditorReturn {
    const { maxLength, initialHtml, onChange, onSelectionChange } = options;

    const bridgeRef = useRef<NativeEditorBridge | null>(null);
    const onChangeRef = useRef(onChange);
    onChangeRef.current = onChange;
    const onSelectionChangeRef = useRef(onSelectionChange);
    onSelectionChangeRef.current = onSelectionChange;

    const [isReady, setIsReady] = useState(false);
    const [selection, setSelection] = useState<Selection>(DEFAULT_SELECTION);
    const [activeState, setActiveState] = useState<ActiveState>(DEFAULT_ACTIVE_STATE);
    const [historyState, setHistoryState] = useState<HistoryState>(DEFAULT_HISTORY_STATE);
    const [renderElements, setRenderElements] = useState<RenderElement[]>([]);

    const syncStateFromUpdate = useCallback((update: EditorUpdate | null) => {
        if (!update) return;
        setRenderElements(update.renderElements);
        setSelection(update.selection);
        setActiveState(update.activeState);
        setHistoryState(update.historyState);
    }, []);

    const applyUpdate = useCallback(
        (update: EditorUpdate | null) => {
            if (!update) return;
            syncStateFromUpdate(update);
            onSelectionChangeRef.current?.(update.selection);

            // Fetch current HTML and notify onChange
            if (onChangeRef.current && bridgeRef.current && !bridgeRef.current.isDestroyed) {
                const html = bridgeRef.current.getHtml();
                onChangeRef.current(html);
            }
        },
        [syncStateFromUpdate]
    );

    useEffect(() => {
        const bridge = NativeEditorBridge.create(maxLength != null ? { maxLength } : undefined);
        bridgeRef.current = bridge;

        if (initialHtml) {
            bridge.setHtml(initialHtml);
        }

        syncStateFromUpdate(bridge.getCurrentState());
        setIsReady(true);

        return () => {
            bridge.destroy();
            bridgeRef.current = null;
            setIsReady(false);
        };
    }, [maxLength, initialHtml, syncStateFromUpdate]);

    const toggleMark = useCallback(
        (markType: string) => {
            if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;
            const update = bridgeRef.current.toggleMark(markType);
            applyUpdate(update);
        },
        [applyUpdate]
    );

    const undo = useCallback(() => {
        if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;
        const update = bridgeRef.current.undo();
        applyUpdate(update);
    }, [applyUpdate]);

    const redo = useCallback(() => {
        if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;
        const update = bridgeRef.current.redo();
        applyUpdate(update);
    }, [applyUpdate]);

    const toggleBlockquote = useCallback(() => {
        if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;
        const update = bridgeRef.current.toggleBlockquote();
        applyUpdate(update);
    }, [applyUpdate]);

    const toggleHeading = useCallback(
        (level: 1 | 2 | 3 | 4 | 5 | 6) => {
            if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;
            const update = bridgeRef.current.toggleHeading(level);
            applyUpdate(update);
        },
        [applyUpdate]
    );

    const insertText = useCallback(
        (pos: number, text: string) => {
            if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;
            const update = bridgeRef.current.insertText(pos, text);
            applyUpdate(update);
        },
        [applyUpdate]
    );

    const deleteRange = useCallback(
        (from: number, to: number) => {
            if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;
            const update = bridgeRef.current.deleteRange(from, to);
            applyUpdate(update);
        },
        [applyUpdate]
    );

    const getHtml = useCallback((): string => {
        if (!bridgeRef.current || bridgeRef.current.isDestroyed) return '';
        return bridgeRef.current.getHtml();
    }, []);

    return {
        bridge: bridgeRef.current,
        isReady,
        selection,
        activeState,
        historyState,
        renderElements,
        toggleMark,
        undo,
        redo,
        toggleBlockquote,
        toggleHeading,
        insertText,
        deleteRange,
        getHtml,
    };
}
