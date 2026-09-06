import {
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    setActiveEditorToolbarFrameOwnerForEditor,
    setEditorToolbarMentionState,
    useEditorToolbarFrames,
    type EditorToolbarItem,
} from './EditorToolbar';
import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
    _assertNativeEditorDocumentHandle,
    _getNativeEditorDocumentHandleDescriptor,
    type DocumentJSON,
    type ReadonlyActiveState,
    type Selection,
} from './NativeEditorBridge';
import { type AtomComponent } from './atoms';
import { normalizeDocumentJson } from './schemas';
import { ExternalTextCompositionManager } from './ExternalTextComposition';
import { useNativeEditorDocument } from './useNativeEditor';
import { normalizeEditorAddons, type MentionQueryChangeEvent } from './addons';
import { type AtomViewport } from './AtomHost';
import { useFocusPreservingFrames } from './useFocusPreservingFrames';
import { type RichTextEditorProps, type RichTextEditorRef } from './RichTextEditorTypes';
import {
    useSerializedValue,
    stringifyCachedJson,
    externalCompositionErrorPayload,
    EMPTY_ACTIVE_STATE,
    type AtomRenderState,
    allocateToolbarFrameOwnerId,
} from './RichTextEditorSerialization';
import {
    type NativeEditorViewHandle,
    type ExternalCompositionDisposalToken,
    type ControlledValueDelivery,
    type NativeEditorErrorBinding,
    type NativeAtomPosition,
} from './RichTextEditorNativeTypes';

export function useRichTextEditorState(
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
        addons: addonDescriptors,
        atoms,
        atomsInteractive = true,
        virtualizeAtoms = false,
        atomViewport,
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
    }: RichTextEditorProps,
    ref: React.ForwardedRef<RichTextEditorRef>
) {
    const addons = useMemo(() => normalizeEditorAddons(addonDescriptors), [addonDescriptors]);
    _assertNativeEditorDocumentHandle(documentHandle);

    const documentDescriptor = _getNativeEditorDocumentHandleDescriptor(documentHandle);

    const registeredAtomTypeKey = JSON.stringify((atoms ?? []).map((atom) => atom.name));

    const registeredAtomTypes = useMemo(
        () => new Set((atoms ?? []).map((atom) => atom.name)),
        [registeredAtomTypeKey]
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
                latest.handle.bridge._emitAutonomousError(externalCompositionErrorPayload(error));
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
        resetJson?: string;
        revision: number;
        editorId: string;
    } | null>(null);

    const [autoGrowHeight, setAutoGrowHeight] = useState<number | null>(null);

    const [isFocused, setIsFocused] = useState(false);

    const [mentionQuery, setMentionQuery] = useState<MentionQueryChangeEvent | null>(null);

    const [atomState, setAtomState] = useState<AtomRenderState>({
        blocks: [],
        instanceBlocks: [],
        instances: [],
        documentVersion: null,
        hasOnlyStableAtomKeys: true,
    });

    const atomStateRef = useRef(atomState);

    const [selectedKeys, setSelectedKeys] = useState<ReadonlySet<string>>(new Set());

    const [atomContentWidth, setAtomContentWidth] = useState<number | null>(null);

    const [atomPositions, setAtomPositions] = useState<ReadonlyMap<string, NativeAtomPosition>>(
        new Map()
    );

    const [nativeAtomViewport, setNativeAtomViewport] = useState<AtomViewport>();

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
        const emptyAtomState: AtomRenderState = {
            blocks: [],
            instanceBlocks: [],
            instances: [],
            documentVersion: null,
            hasOnlyStableAtomKeys: true,
        };
        atomStateRef.current = emptyAtomState;
        atomSeedEditorIdRef.current = null;
        setAtomState(emptyAtomState);
        setSelectedKeys(new Set());
        setAtomContentWidth(null);
        setAtomPositions(new Map());
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
    return {
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
        editable,
        latestRevisionRef,
        nativeViewRef,
        documentDescriptor,
        onRequestLinkRef,
        onRequestImageRef,
        ref,
        externalCompositionManager,
        lastAcceptedNativeCommitRevisionRef,
        nativeErrorBindingRef,
        nativeErrorBinding,
        onSelectionChangeRef,
        isFocusedRef,
        setIsFocused,
        onBlurRef,
        heightBehavior,
        setAutoGrowHeight,
        setAtomContentWidth,
        setNativeAtomViewport,
        setAtomPositions,
        onToolbarActionRef,
        addonsRef,
        setMentionQuery,
        mentionQuery,
        addons,
        isFocused,
        theme,
        imageLoadingPolicy,
        androidInputOptions,
        remoteSelections,
        atoms,
        atomState,
        atomComponents,
        warnedUnknownAtomTypesRef,
        atomContentWidth,
        atomPositions,
        atomViewport,
        virtualizeAtoms,
        nativeAtomViewport,
        selectedKeys,
        atomsInteractive,
        activeState,
        onRequestLink,
        onRequestImage,
        toolbarItemsSerializationCacheRef,
        toolbarItems,
        toolbarPlacement,
        showToolbar,
        containerStyle: containerStyle as RichTextEditorProps['containerStyle'],
        style: style as RichTextEditorProps['style'],
        autoGrowHeight,
        pushedUpdate,
        registeredToolbarFrames,
        suppliedFocusPreservingFrames,
        refreshFocusPreservingFrames,
        accessibilityLabel,
        accessibilityHint,
        placeholder,
        autoFocus,
        autoCapitalize,
        autoCorrect,
        keyboardType,
        allowImageResizing,
        onToolbarAction,
    };
}
