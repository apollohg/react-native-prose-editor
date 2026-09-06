import { useCallback, useImperativeHandle, useRef } from 'react';
import { type AtomAttrsUpdate } from './atoms';
import { AtomUpdateAttrsError, type AtomInstance } from './atomInstances';
import { resolveAtomAttrsUpdate } from './atomUpdates';
import { NativeEditorV2ErrorBase, NativeEditorV2OperationError } from './NativeEditorBoundaryError';
import { type DocumentJSON, type NativeEditorDocumentHandle } from './NativeEditorBridge';
import { type EditorToolbarHeadingLevel } from './EditorToolbar';
import { buildImageFragmentJson, type ImageNodeAttributes } from './schemas';
import {
    createExternalCompositionLifecycleError,
    type ExternalTextCompositionOptions,
    type ExternalTextCompositionSession,
} from './ExternalTextComposition';
import { useRichTextEditorState } from './useRichTextEditorState';
import { useRichTextEditorUpdates } from './useRichTextEditorUpdates';
import { isRevisionMismatchError, parseCaretRectJson } from './RichTextEditorSerialization';
import { type RichTextEditorRef, type RichTextEditorCaretRect } from './RichTextEditorTypes';

export function useRichTextEditorCommands(
    context: Pick<
        ReturnType<typeof useRichTextEditorState>,
        | 'editable'
        | 'documentHandle'
        | 'latestRevisionRef'
        | 'atomStateRef'
        | 'bridge'
        | 'document'
        | 'nativeViewRef'
        | 'activeStateRef'
        | 'documentDescriptor'
        | 'onRequestLinkRef'
        | 'selectionRef'
        | 'onRequestImageRef'
        | 'ref'
        | 'externalCompositionManager'
    > &
        Pick<
            ReturnType<typeof useRichTextEditorUpdates>,
            'refreshAtomsFromUpdate' | 'afterLocalEngineMutation' | 'pushEngineUpdateToView'
        >
) {
    const {
        editable,
        documentHandle,
        latestRevisionRef,
        atomStateRef,
        bridge,
        refreshAtomsFromUpdate,
        document,
        afterLocalEngineMutation,
        pushEngineUpdateToView,
        nativeViewRef,
        activeStateRef,
        documentDescriptor,
        onRequestLinkRef,
        selectionRef,
        onRequestImageRef,
        ref,
        externalCompositionManager,
    } = context;

    const editableRef = useRef(editable);

    editableRef.current = editable;

    const atomOwnerRef = useRef(documentHandle);

    atomOwnerRef.current = documentHandle;

    const updateAtomAttrs = useCallback(
        async (
            atomKey: string,
            nodeType: string,
            expectedDocPos: number,
            expectedDocumentVersion: string | null,
            hasStableKey: boolean,
            update: AtomAttrsUpdate
        ): Promise<void> => {
            const baseDocumentRevision = latestRevisionRef.current;
            if (documentHandle.isDestroyed || baseDocumentRevision == null) {
                throw new AtomUpdateAttrsError('not-ready', 'The editor is not ready');
            }
            if (!editableRef.current) {
                throw new AtomUpdateAttrsError('not-applicable', 'The editor is not editable');
            }
            const instance = atomStateRef.current.instances.find(
                (candidate) => candidate.key === atomKey && candidate.nodeType === nodeType
            );
            if (
                instance == null ||
                (!hasStableKey &&
                    (atomStateRef.current.documentVersion !== expectedDocumentVersion ||
                        instance.docPos !== expectedDocPos))
            ) {
                throw new AtomUpdateAttrsError(
                    'not-applicable',
                    'The atom no longer exists in the document'
                );
            }
            const attrs = resolveAtomAttrsUpdate(instance.attrs, update);
            let outcome;
            try {
                outcome = bridge.applyCommand({
                    baseDocumentRevision,
                    command: { type: 'updateNodeAttrs', docPos: instance.docPos, attrs },
                });
            } catch (error) {
                if (isRevisionMismatchError(error)) {
                    refreshAtomsFromUpdate(bridge.renderUpdate());
                    document.refresh();
                    throw new AtomUpdateAttrsError(
                        'stale-revision',
                        'The atom changed before its attributes were updated'
                    );
                }
                if (
                    error instanceof NativeEditorV2ErrorBase &&
                    (error.code === 'ENGINE_NOT_READY' ||
                        error.code === 'ENGINE_DESTROYING' ||
                        error.code === 'ENGINE_DESTROYED')
                ) {
                    throw new AtomUpdateAttrsError('not-ready', 'The editor is not ready');
                }
                throw new AtomUpdateAttrsError(
                    'engine-error',
                    error instanceof Error ? error.message : 'The atom update failed'
                );
            }
            if (outcome.type === 'notApplicable') {
                refreshAtomsFromUpdate(bridge.renderUpdate());
                document.refresh();
                throw new AtomUpdateAttrsError(
                    'not-applicable',
                    'The atom no longer exists at this document position'
                );
            }
            if (outcome.type !== 'transaction') {
                throw new AtomUpdateAttrsError('engine-error', 'Unexpected atom update outcome');
            }
            afterLocalEngineMutation();
        },
        [afterLocalEngineMutation, bridge, document, documentHandle, refreshAtomsFromUpdate]
    );

    const runAtomAction = useCallback(
        async (
            owner: NativeEditorDocumentHandle,
            instance: AtomInstance,
            documentVersion: string | null,
            action: 'select' | 'delete' | 'before' | 'after'
        ) => {
            if (
                owner !== atomOwnerRef.current ||
                documentHandle.isDestroyed ||
                latestRevisionRef.current == null
            )
                throw new AtomUpdateAttrsError('not-ready', 'The editor is not ready.');
            if (action === 'delete' && !editableRef.current)
                throw new AtomUpdateAttrsError('not-applicable', 'The editor is not editable.');
            const current = atomStateRef.current.instances.find(
                (candidate) =>
                    candidate.key === instance.key && candidate.nodeType === instance.nodeType
            );
            if (
                !current ||
                (!instance.hasStableKey &&
                    (documentVersion !== atomStateRef.current.documentVersion ||
                        current.docPos !== instance.docPos))
            )
                throw new AtomUpdateAttrsError('not-applicable', 'The atom no longer exists.');
            try {
                const baseDocumentRevision = latestRevisionRef.current;
                const selection = bridge.setSelection({
                    baseDocumentRevision,
                    selection: {
                        type: 'atom',
                        docPos: current.docPos,
                        edge: action === 'select' || action === 'delete' ? 'node' : action,
                    },
                });
                if (selection.type !== 'transaction')
                    throw new AtomUpdateAttrsError(
                        'not-applicable',
                        'The atom could not be selected.'
                    );
                if (action === 'delete') {
                    const outcome = bridge.applyCommand({
                        baseDocumentRevision,
                        command: { type: 'deleteBackward' },
                    });
                    if (outcome.type !== 'transaction')
                        throw new AtomUpdateAttrsError(
                            'not-applicable',
                            'The atom could not be deleted.'
                        );
                    afterLocalEngineMutation();
                } else {
                    pushEngineUpdateToView();
                    document.refresh();
                    if (action !== 'select') nativeViewRef.current?.focus?.();
                }
            } catch (error) {
                if (error instanceof AtomUpdateAttrsError) throw error;
                if (isRevisionMismatchError(error)) {
                    refreshAtomsFromUpdate(bridge.renderUpdate());
                    document.refresh();
                    throw new AtomUpdateAttrsError(
                        'stale-revision',
                        'The document changed before the atom action.'
                    );
                }
                throw new AtomUpdateAttrsError(
                    'engine-error',
                    error instanceof Error ? error.message : String(error)
                );
            }
        },
        [
            afterLocalEngineMutation,
            bridge,
            document,
            documentHandle,
            pushEngineUpdateToView,
            refreshAtomsFromUpdate,
        ]
    );

    const runEngineMutation = useCallback(
        (invoke: (baseDocumentRevision: string) => unknown) => {
            if (!editableRef.current) {
                throw new NativeEditorV2OperationError({
                    domain: 'operation',
                    code: 'MUTATION_REJECTED',
                    message: 'NativeRichTextEditor: mutation rejected while editable is false',
                    requestId: null,
                    operationIndex: null,
                    limit: null,
                    actual: null,
                    details: null,
                });
            }
            const baseRevision = latestRevisionRef.current;
            if (baseRevision == null) {
                // Engine not ready (room awaiting the server document).
                return;
            }
            try {
                invoke(baseRevision);
            } catch (error) {
                if (isRevisionMismatchError(error)) {
                    // Refresh from the engine; NEVER retry against guessed
                    // positions (native adapter parity).
                    document.refresh();
                    return;
                }
                throw error;
            }
            afterLocalEngineMutation();
        },
        [afterLocalEngineMutation, document]
    );

    const applyEngineCommand = useCallback(
        (command: Record<string, unknown>) => {
            runEngineMutation((baseDocumentRevision) =>
                bridge.applyCommand({ command, baseDocumentRevision })
            );
        },
        [bridge, runEngineMutation]
    );

    const commandToggleMark = useCallback(
        (markType: string) => applyEngineCommand({ type: 'toggleMark', markType }),
        [applyEngineCommand]
    );

    const commandSetLink = useCallback(
        (href: string) => {
            const trimmedHref = href.trim();
            if (!trimmedHref) return;
            applyEngineCommand({
                type: 'setMark',
                markType: 'link',
                attrs: { href: trimmedHref },
            });
        },
        [applyEngineCommand]
    );

    const commandUnsetLink = useCallback(
        () => applyEngineCommand({ type: 'unsetMark', markType: 'link' }),
        [applyEngineCommand]
    );

    const commandToggleBlockquote = useCallback(
        () => applyEngineCommand({ type: 'toggleBlockquote' }),
        [applyEngineCommand]
    );

    const commandToggleHeading = useCallback(
        (level: EditorToolbarHeadingLevel) => applyEngineCommand({ type: 'toggleHeading', level }),
        [applyEngineCommand]
    );

    const commandToggleList = useCallback(
        (listType: string) => {
            if (activeStateRef.current.nodes[listType] === true) {
                applyEngineCommand({ type: 'unwrapFromList' });
                return;
            }
            applyEngineCommand({
                type: 'wrapInList',
                listType,
                itemType:
                    listType === 'taskList'
                        ? 'taskItem'
                        : listType === 'bullet_list' || listType === 'ordered_list'
                          ? 'list_item'
                          : 'listItem',
            });
        },
        [applyEngineCommand]
    );

    const commandIndentListItem = useCallback(
        () => applyEngineCommand({ type: 'indentListItem' }),
        [applyEngineCommand]
    );

    const commandOutdentListItem = useCallback(
        () => applyEngineCommand({ type: 'outdentListItem' }),
        [applyEngineCommand]
    );

    const commandInsertNode = useCallback(
        (nodeType: string) => applyEngineCommand({ type: 'insertNode', nodeType }),
        [applyEngineCommand]
    );

    const commandInsertImage = useCallback(
        (src: string, attrs?: Omit<ImageNodeAttributes, 'src'>) => {
            applyEngineCommand({
                type: 'insertContentJson',
                json: buildImageFragmentJson({ src, ...attrs }, documentDescriptor),
            });
        },
        [applyEngineCommand, documentDescriptor]
    );

    const commandInsertText = useCallback(
        (text: string) => {
            if (!text) return;
            runEngineMutation((baseDocumentRevision) =>
                bridge.applyInput({ text, baseDocumentRevision })
            );
        },
        [bridge, runEngineMutation]
    );

    const commandInsertContentHtml = useCallback(
        (html: string) => applyEngineCommand({ type: 'insertContentHtml', html }),
        [applyEngineCommand]
    );

    const commandInsertContentJson = useCallback(
        (doc: DocumentJSON) => applyEngineCommand({ type: 'insertContentJson', json: doc }),
        [applyEngineCommand]
    );

    const openLinkRequest = useCallback(() => {
        const linkAttrs = activeStateRef.current.markAttrs?.link;
        onRequestLinkRef.current?.({
            href: typeof linkAttrs?.href === 'string' ? linkAttrs.href : undefined,
            isActive: activeStateRef.current.marks.link === true,
            selection: selectionRef.current,
            setLink: commandSetLink,
            unsetLink: commandUnsetLink,
        });
    }, [commandSetLink, commandUnsetLink]);

    const openImageRequest = useCallback(() => {
        onRequestImageRef.current?.({
            selection: selectionRef.current,
            insertImage: commandInsertImage,
        });
    }, [commandInsertImage]);

    useImperativeHandle(
        ref,
        (): RichTextEditorRef => ({
            focus() {
                nativeViewRef.current?.focus?.();
            },
            blur() {
                nativeViewRef.current?.blur?.();
            },
            supportsExternalTextComposition() {
                return externalCompositionManager.supports();
            },
            beginExternalTextComposition(
                options?: ExternalTextCompositionOptions
            ): Promise<ExternalTextCompositionSession> {
                if (!editable) {
                    return Promise.reject(
                        createExternalCompositionLifecycleError(
                            'EXTERNAL_COMPOSITION_UNAVAILABLE',
                            'The editor view is not editable'
                        )
                    );
                }
                return externalCompositionManager.begin(options);
            },
            toggleMark: commandToggleMark,
            setLink: commandSetLink,
            unsetLink: commandUnsetLink,
            toggleBlockquote: commandToggleBlockquote,
            toggleHeading: commandToggleHeading,
            toggleList: commandToggleList,
            indentListItem: commandIndentListItem,
            outdentListItem: commandOutdentListItem,
            insertNode: commandInsertNode,
            insertImage: commandInsertImage,
            insertText: commandInsertText,
            insertContentHtml: commandInsertContentHtml,
            insertContentJson: commandInsertContentJson,
            setContent: document.setContent,
            setContentJson: document.setContentJson,
            clearContent: document.clearContent,
            getContent: document.getContent,
            getContentJson: document.getContentJson,
            getIsEmpty: document.getIsEmpty,
            getTextContent: document.getTextContent,
            async getCaretRect(): Promise<RichTextEditorCaretRect | null> {
                const nativeView = nativeViewRef.current;
                if (!nativeView?.getCaretRect) return null;
                const raw = await Promise.resolve(nativeView.getCaretRect());
                return parseCaretRectJson(raw);
            },
            undo: document.undo,
            redo: document.redo,
            canUndo: document.canUndo,
            canRedo: document.canRedo,
        }),
        [
            document,
            commandToggleMark,
            commandSetLink,
            commandUnsetLink,
            commandToggleBlockquote,
            commandToggleHeading,
            commandToggleList,
            commandIndentListItem,
            commandOutdentListItem,
            commandInsertNode,
            commandInsertImage,
            commandInsertText,
            commandInsertContentHtml,
            commandInsertContentJson,
            editable,
            externalCompositionManager,
        ]
    );
    return {
        openLinkRequest,
        openImageRequest,
        editableRef,
        runAtomAction,
        updateAtomAttrs,
        atomOwnerRef,
        commandToggleMark,
        commandToggleList,
        commandToggleHeading,
        commandToggleBlockquote,
        commandInsertNode,
        commandIndentListItem,
        commandOutdentListItem,
    };
}
