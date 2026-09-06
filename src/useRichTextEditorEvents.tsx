import { useCallback } from 'react';
import { PixelRatio, Platform, type NativeSyntheticEvent } from 'react-native';
import { normalizeNativeEditorV2DecimalId, type Selection } from './NativeEditorBridge';
import { setActiveEditorToolbarFrameOwnerForEditor } from './EditorToolbar';
import { useRichTextEditorState } from './useRichTextEditorState';
import { useRichTextEditorUpdates } from './useRichTextEditorUpdates';
import { useRichTextEditorCommands } from './useRichTextEditorCommands';
import {
    type NativeUpdateEvent,
    type NativeEditorErrorBinding,
    type NativeErrorEvent,
    type NativeExternalTextCompositionEvent,
    type NativeSelectionEvent,
    type NativeFocusEvent,
    type NativeContentHeightEvent,
    type NativeAtomLayoutEvent,
    type NativeAtomPosition,
    type NativeToolbarActionEvent,
} from './RichTextEditorNativeTypes';
import {
    acceptNativeCommitPayload,
    isRecord,
    externalCompositionErrorPayload,
    parseSelectionFromUpdate,
    LINK_TOOLBAR_ACTION_KEY,
    IMAGE_TOOLBAR_ACTION_KEY,
} from './RichTextEditorSerialization';

export function useRichTextEditorEvents(
    context: Pick<
        ReturnType<typeof useRichTextEditorState>,
        | 'editorId'
        | 'documentHandle'
        | 'lastAcceptedNativeCommitRevisionRef'
        | 'lastNativeDrivenRevisionRef'
        | 'document'
        | 'onLocalCommitRef'
        | 'nativeErrorBindingRef'
        | 'nativeErrorBinding'
        | 'externalCompositionManager'
        | 'scalarSelectionRef'
        | 'selectionRef'
        | 'onSelectionChangeRef'
        | 'isFocusedRef'
        | 'toolbarFrameOwnerId'
        | 'setIsFocused'
        | 'onFocusRef'
        | 'onBlurRef'
        | 'heightBehavior'
        | 'setAutoGrowHeight'
        | 'setAtomContentWidth'
        | 'setNativeAtomViewport'
        | 'setAtomPositions'
        | 'onToolbarActionRef'
    > &
        Pick<
            ReturnType<typeof useRichTextEditorUpdates>,
            | 'refreshAtomsFromUpdate'
            | 'applyTypedUpdateState'
            | 'applyUpdateState'
            | 'updateAtomSelection'
        > &
        Pick<ReturnType<typeof useRichTextEditorCommands>, 'openLinkRequest' | 'openImageRequest'>
) {
    const {
        editorId,
        documentHandle,
        lastAcceptedNativeCommitRevisionRef,
        lastNativeDrivenRevisionRef,
        refreshAtomsFromUpdate,
        applyTypedUpdateState,
        document,
        onLocalCommitRef,
        nativeErrorBindingRef,
        nativeErrorBinding,
        externalCompositionManager,
        scalarSelectionRef,
        applyUpdateState,
        selectionRef,
        updateAtomSelection,
        onSelectionChangeRef,
        isFocusedRef,
        toolbarFrameOwnerId,
        setIsFocused,
        onFocusRef,
        onBlurRef,
        heightBehavior,
        setAutoGrowHeight,
        setAtomContentWidth,
        setNativeAtomViewport,
        setAtomPositions,
        openLinkRequest,
        openImageRequest,
        onToolbarActionRef,
    } = context;

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
                externalCompositionManager.handleNativeEnd(binding.editorId, payload.resultJson);
            } catch (error) {
                binding.handle.bridge._emitAutonomousError(externalCompositionErrorPayload(error));
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
            const viewport = event.nativeEvent.viewport;
            if (
                viewport &&
                Number.isFinite(viewport.y) &&
                Number.isFinite(viewport.height) &&
                viewport.height >= 0
            ) {
                setNativeAtomViewport((previous) =>
                    previous?.y === viewport.y && previous.height === viewport.height
                        ? previous
                        : viewport
                );
            }
            const positions = event.nativeEvent.positions;
            if (!Array.isArray(positions)) return;
            const next = new Map<string, NativeAtomPosition>();
            for (const position of positions) {
                if (
                    typeof position?.key !== 'string' ||
                    !Number.isFinite(position.x) ||
                    !Number.isFinite(position.y)
                ) {
                    continue;
                }
                next.set(position.key, position);
            }
            setAtomPositions((current) => {
                if (current.size !== next.size) return next;
                for (const [key, position] of next) {
                    const previous = current.get(key);
                    if (
                        previous?.x !== position.x ||
                        previous.y !== position.y ||
                        previous.height !== position.height
                    )
                        return next;
                }
                return current;
            });
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
                if (typeof updateJson === 'string' && typeof documentRevision === 'string') {
                    const accepted = acceptNativeCommitPayload(
                        {
                            editorId: event.nativeEvent.editorId,
                            documentRevision,
                            updateJson,
                        },
                        documentHandle.editorId,
                        lastAcceptedNativeCommitRevisionRef.current
                    );
                    if (accepted != null) {
                        lastAcceptedNativeCommitRevisionRef.current = accepted.documentRevision;
                        lastNativeDrivenRevisionRef.current = accepted.documentRevision;
                        refreshAtomsFromUpdate(accepted.snapshot);
                        applyTypedUpdateState(accepted.snapshot);
                        document.refresh();
                        onLocalCommitRef.current?.();
                    }
                }
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
    return {
        isForThisEditor,
        handleEditorUpdate,
        handleEditorError,
        handleExternalTextCompositionEnd,
        handleSelectionChange,
        handleFocusChange,
        handleContentHeightChange,
        handleAtomLayout,
        handleToolbarAction,
    };
}
