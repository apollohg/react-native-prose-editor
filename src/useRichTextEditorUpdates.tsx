import { useCallback, useEffect, useLayoutEffect } from 'react';
import {
    type NativeEditorAtomicRenderSnapshot,
    type NativeEditorPositionAffinity,
    type Selection,
} from './NativeEditorBridge';
import { applyRenderPatch, collectAtomInstanceBlocks } from './atomInstances';
import { allocateEditorUpdateRevision } from './EditorUpdateRevision';
import { setActiveEditorToolbarFrameOwnerForEditor } from './EditorToolbar';
import { useRichTextEditorState } from './useRichTextEditorState';
import {
    selectedAtomKeys,
    equalStringSets,
    equalAtomInstances,
    stringifyCachedJson,
    isRecord,
    parseSelectionFromUpdate,
    parseActiveStateFromUpdate,
    isPositionInvalidError,
} from './RichTextEditorSerialization';

export function useRichTextEditorUpdates(
    context: Pick<
        ReturnType<typeof useRichTextEditorState>,
        | 'atomStateRef'
        | 'setSelectedKeys'
        | 'bridge'
        | 'registeredAtomTypes'
        | 'setAtomState'
        | 'selectionRef'
        | 'document'
        | 'documentHandle'
        | 'atomSeedEditorIdRef'
        | 'editorId'
        | 'scalarSelectionRef'
        | 'activeStateRef'
        | 'setActiveState'
        | 'activeStateKeyRef'
        | 'onActiveStateChangeRef'
        | 'pushedUpdateBindingGenerationRef'
        | 'currentPushedUpdateEditorIdRef'
        | 'pushRevisionRef'
        | 'lastPushedEngineRevisionRef'
        | 'setPushedUpdate'
        | 'pendingFocusedRebindRef'
        | 'toolbarFrameOwnerId'
        | 'onFocusRef'
        | 'didRebindRevisionScope'
        | 'didObserveInitialRevisionRef'
        | 'lastNativeDrivenRevisionRef'
        | 'onLocalCommitRef'
    >
) {
    const {
        atomStateRef,
        setSelectedKeys,
        bridge,
        registeredAtomTypes,
        setAtomState,
        selectionRef,
        document,
        documentHandle,
        atomSeedEditorIdRef,
        editorId,
        scalarSelectionRef,
        activeStateRef,
        setActiveState,
        activeStateKeyRef,
        onActiveStateChangeRef,
        pushedUpdateBindingGenerationRef,
        currentPushedUpdateEditorIdRef,
        pushRevisionRef,
        lastPushedEngineRevisionRef,
        setPushedUpdate,
        pendingFocusedRebindRef,
        toolbarFrameOwnerId,
        onFocusRef,
        didRebindRevisionScope,
        didObserveInitialRevisionRef,
        lastNativeDrivenRevisionRef,
        onLocalCommitRef,
    } = context;

    const updateAtomSelection = useCallback(
        (selection: Selection, instances = atomStateRef.current.instances) => {
            const next = selectedAtomKeys(selection, instances);
            setSelectedKeys((current) => (equalStringSets(current, next) ? current : next));
        },
        []
    );

    const refreshAtomsFromUpdate = useCallback(
        (update: NativeEditorAtomicRenderSnapshot) => {
            const previous = atomStateRef.current;
            const source =
                update.renderPatch != null &&
                update.renderPatch.baseDocumentVersion !== previous.documentVersion
                    ? bridge.renderUpdate()
                    : update;
            if (
                source.renderPatch != null &&
                source.renderPatch.baseDocumentVersion !== previous.documentVersion
            ) {
                return;
            }
            const blocks =
                source.renderBlocks != null
                    ? source.renderBlocks
                    : applyRenderPatch(previous.blocks, {
                          startIndex: source.renderPatch.startIndex,
                          deleteCount: source.renderPatch.deleteCount,
                          renderBlocks: source.renderPatch.renderBlocks,
                      });
            const patchedCollection =
                source.renderPatch != null && previous.hasOnlyStableAtomKeys
                    ? collectAtomInstanceBlocks(
                          source.renderPatch.renderBlocks,
                          registeredAtomTypes
                      )
                    : null;
            const collection =
                source.renderPatch != null && patchedCollection?.hasOnlyStableKeys
                    ? {
                          instanceBlocks: applyRenderPatch(previous.instanceBlocks, {
                              startIndex: source.renderPatch.startIndex,
                              deleteCount: source.renderPatch.deleteCount,
                              renderBlocks: patchedCollection.instanceBlocks,
                          }),
                          hasOnlyStableKeys: true,
                      }
                    : collectAtomInstanceBlocks(blocks, registeredAtomTypes);
            const instances = collection.instanceBlocks.flat();
            const next = {
                blocks,
                instanceBlocks: collection.instanceBlocks,
                instances,
                documentVersion: source.documentVersion,
                hasOnlyStableAtomKeys: collection.hasOnlyStableKeys,
            };
            atomStateRef.current = next;
            if (!equalAtomInstances(previous.instances, next.instances)) {
                setAtomState(next);
            }
            updateAtomSelection(selectionRef.current, next.instances);
        },
        [bridge, registeredAtomTypes, updateAtomSelection]
    );

    useEffect(() => {
        const current = atomStateRef.current;
        const collection = collectAtomInstanceBlocks(current.blocks, registeredAtomTypes);
        const instances = collection.instances;
        const next = {
            blocks: current.blocks,
            instanceBlocks: collection.instanceBlocks,
            instances,
            documentVersion: current.documentVersion,
            hasOnlyStableAtomKeys: collection.hasOnlyStableKeys,
        };
        atomStateRef.current = next;
        if (!equalAtomInstances(current.instances, instances)) {
            setAtomState(next);
        }
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
            update: Pick<NativeEditorAtomicRenderSnapshot, 'selection' | 'activeState'>,
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
        const setSelection = (affinity: NativeEditorPositionAffinity) =>
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
            if (document.documentOrigin === 'remoteCollaboration' && !documentHandle.isDestroyed) {
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
    return {
        refreshAtomsFromUpdate,
        afterLocalEngineMutation,
        pushEngineUpdateToView,
        applyTypedUpdateState,
        applyUpdateState,
        updateAtomSelection,
    };
}
