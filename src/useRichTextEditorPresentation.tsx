import { useCallback, useEffect, useMemo, useRef } from 'react';
import { serializeEditorTheme } from './EditorTheme';
import { serializeEditorAddons } from './addons';
import { serializeEditorImageLoadingPolicy } from './ImageLoadingPolicy';
import { serializeEditorAtoms, type AtomAttrsUpdate } from './atoms';
import { AtomUpdateAttrsError, DEFAULT_ATOM_CHIP_HEIGHT, type AtomInstance } from './atomInstances';
import { type NativeEditorDocumentHandle } from './NativeEditorBridge';
import { DefaultAtomChip } from './DefaultAtomChip';
import { Platform, StyleSheet, View, type StyleProp, type ViewStyle } from 'react-native';
import { ATOM_NATIVE_ID_PREFIX } from './atomConstants';
import { AtomHost, atomIsVisible } from './AtomHost';
import { IMAGE_NODE_NAME } from './schemas';
import {
    EditorToolbar,
    EditorToolbarFrameOwnerProvider,
    type EditorToolbarCommand,
    type EditorToolbarListType,
} from './EditorToolbar';
import { useRichTextEditorState } from './useRichTextEditorState';
import { useRichTextEditorMentions } from './useRichTextEditorMentions';
import { useRichTextEditorCommands } from './useRichTextEditorCommands';
import { useRichTextEditorEvents } from './useRichTextEditorEvents';
import {
    useSerializedValue,
    serializeRemoteSelections,
    mapToolbarItemsForNative,
    stringifyCachedJson,
    serializeToolbarFrames,
} from './RichTextEditorSerialization';
import { styles, NativeEditorView } from './RichTextEditorNativeView';

export function useRichTextEditorPresentation(
    context: Pick<
        ReturnType<typeof useRichTextEditorState>,
        | 'theme'
        | 'addons'
        | 'imageLoadingPolicy'
        | 'androidInputOptions'
        | 'remoteSelections'
        | 'atoms'
        | 'atomState'
        | 'atomComponents'
        | 'warnedUnknownAtomTypesRef'
        | 'atomContentWidth'
        | 'atomPositions'
        | 'atomViewport'
        | 'virtualizeAtoms'
        | 'nativeAtomViewport'
        | 'selectedKeys'
        | 'editable'
        | 'atomsInteractive'
        | 'documentHandle'
        | 'activeState'
        | 'onRequestLink'
        | 'onRequestImage'
        | 'toolbarItemsSerializationCacheRef'
        | 'toolbarItems'
        | 'document'
        | 'toolbarPlacement'
        | 'showToolbar'
        | 'containerStyle'
        | 'style'
        | 'heightBehavior'
        | 'autoGrowHeight'
        | 'pushedUpdate'
        | 'editorId'
        | 'registeredToolbarFrames'
        | 'suppliedFocusPreservingFrames'
        | 'isFocused'
        | 'nativeViewRef'
        | 'refreshFocusPreservingFrames'
        | 'accessibilityLabel'
        | 'accessibilityHint'
        | 'placeholder'
        | 'autoFocus'
        | 'autoCapitalize'
        | 'autoCorrect'
        | 'keyboardType'
        | 'allowImageResizing'
        | 'toolbarFrameOwnerId'
        | 'onToolbarAction'
    > &
        Pick<
            ReturnType<typeof useRichTextEditorMentions>,
            'mentionSuggestionTheme' | 'handleAddonEvent'
        > &
        Pick<
            ReturnType<typeof useRichTextEditorCommands>,
            | 'runAtomAction'
            | 'updateAtomAttrs'
            | 'atomOwnerRef'
            | 'commandToggleMark'
            | 'commandToggleList'
            | 'commandToggleHeading'
            | 'commandToggleBlockquote'
            | 'commandInsertNode'
            | 'commandIndentListItem'
            | 'commandOutdentListItem'
            | 'openLinkRequest'
            | 'openImageRequest'
        > &
        Pick<
            ReturnType<typeof useRichTextEditorEvents>,
            | 'handleEditorUpdate'
            | 'handleEditorError'
            | 'handleExternalTextCompositionEnd'
            | 'handleSelectionChange'
            | 'handleFocusChange'
            | 'handleContentHeightChange'
            | 'handleAtomLayout'
            | 'handleToolbarAction'
        >
) {
    const {
        theme,
        mentionSuggestionTheme,
        addons,
        imageLoadingPolicy,
        androidInputOptions,
        remoteSelections,
        atoms,
        atomState,
        atomComponents,
        warnedUnknownAtomTypesRef,
        runAtomAction,
        updateAtomAttrs,
        atomOwnerRef,
        atomContentWidth,
        atomPositions,
        atomViewport,
        virtualizeAtoms,
        nativeAtomViewport,
        selectedKeys,
        editable,
        atomsInteractive,
        documentHandle,
        activeState,
        onRequestLink,
        onRequestImage,
        toolbarItemsSerializationCacheRef,
        toolbarItems,
        document,
        toolbarPlacement,
        showToolbar,
        containerStyle,
        style,
        heightBehavior,
        autoGrowHeight,
        pushedUpdate,
        editorId,
        registeredToolbarFrames,
        suppliedFocusPreservingFrames,
        isFocused,
        nativeViewRef,
        refreshFocusPreservingFrames,
        accessibilityLabel,
        accessibilityHint,
        placeholder,
        autoFocus,
        autoCapitalize,
        autoCorrect,
        keyboardType,
        allowImageResizing,
        handleEditorUpdate,
        handleEditorError,
        handleExternalTextCompositionEnd,
        handleSelectionChange,
        handleFocusChange,
        handleContentHeightChange,
        handleAtomLayout,
        handleToolbarAction,
        handleAddonEvent,
        toolbarFrameOwnerId,
        commandToggleMark,
        commandToggleList,
        commandToggleHeading,
        commandToggleBlockquote,
        commandInsertNode,
        commandIndentListItem,
        commandOutdentListItem,
        openLinkRequest,
        openImageRequest,
        onToolbarAction,
    } = context;

    const themeJson = useMemo(
        () => serializeEditorTheme(theme, mentionSuggestionTheme),
        [mentionSuggestionTheme, theme]
    );

    const addonsJson = useSerializedValue(addons, (value) => serializeEditorAddons(value));

    const imageLoadingPolicyJson = useSerializedValue(imageLoadingPolicy, (value) =>
        serializeEditorImageLoadingPolicy(value)
    );

    const androidInputOptionsJson = useSerializedValue(androidInputOptions, (value) =>
        JSON.stringify(value)
    );

    const remoteSelectionsJson = useSerializedValue(remoteSelections, (selections) =>
        serializeRemoteSelections(selections)
    );

    const atomsJson = useMemo(() => {
        const supplied = serializeEditorAtoms(atoms);
        const serialized =
            supplied == null
                ? { nodeTypes: [] as string[], estimatedHeights: {} as Record<string, number> }
                : (JSON.parse(supplied) as {
                      nodeTypes: string[];
                      estimatedHeights: Record<string, number>;
                  });
        for (const instance of atomState.instances) {
            if (
                Object.prototype.hasOwnProperty.call(serialized.estimatedHeights, instance.nodeType)
            ) {
                continue;
            }
            serialized.nodeTypes.push(instance.nodeType);
            serialized.estimatedHeights[instance.nodeType] = DEFAULT_ATOM_CHIP_HEIGHT;
        }
        return serialized.nodeTypes.length === 0 ? undefined : JSON.stringify(serialized);
    }, [atomState.instances, atoms]);

    useEffect(() => {
        if (!__DEV__) return;
        for (const instance of atomState.instances) {
            if (
                atomComponents.has(instance.nodeType) ||
                warnedUnknownAtomTypesRef.current.has(instance.nodeType)
            ) {
                continue;
            }
            warnedUnknownAtomTypesRef.current.add(instance.nodeType);
            console.warn(
                `NativeRichTextEditor: rendering unknown atom type '${instance.nodeType}' as a chip`
            );
        }
    }, [atomComponents, atomState.instances]);

    const runAtomActionRef = useRef(runAtomAction);

    runAtomActionRef.current = runAtomAction;

    const invokeAtomAction = useCallback(
        (...args: Parameters<typeof runAtomAction>) => runAtomActionRef.current(...args),
        []
    );

    const updateAtomAttrsRef = useRef(updateAtomAttrs);

    updateAtomAttrsRef.current = updateAtomAttrs;

    const invokeAtomAttrsUpdate = useCallback(
        (
            owner: NativeEditorDocumentHandle,
            instance: AtomInstance,
            documentVersion: string | null,
            attrs: AtomAttrsUpdate
        ) => {
            if (owner !== atomOwnerRef.current)
                return Promise.reject(
                    new AtomUpdateAttrsError('not-ready', 'The editor has rebound.')
                );
            return updateAtomAttrsRef.current(
                instance.key,
                instance.nodeType,
                instance.docPos,
                documentVersion,
                instance.hasStableKey,
                attrs
            );
        },
        []
    );

    const atomChildren = useMemo(
        () =>
            atomContentWidth == null
                ? null
                : atomState.instances.map((instance) => {
                      const Component = atomComponents.get(instance.nodeType) ?? DefaultAtomChip;
                      const position = atomPositions.get(instance.key);
                      return (
                          <View
                              key={instance.key}
                              nativeID={`${ATOM_NATIVE_ID_PREFIX}${instance.key}`}
                              collapsable={false}
                              style={{
                                  position: 'absolute',
                                  top: Platform.OS === 'ios' ? 0 : (position?.y ?? 0),
                                  left: Platform.OS === 'ios' ? 0 : (position?.x ?? 0),
                                  width: atomContentWidth,
                              }}>
                              <AtomHost
                                  component={Component}
                                  width={atomContentWidth}
                                  estimatedHeight={
                                      (atoms ?? []).find((atom) => atom.name === instance.nodeType)
                                          ?.estimatedHeight ?? DEFAULT_ATOM_CHIP_HEIGHT
                                  }
                                  visible={
                                      !position ||
                                      atomIsVisible(
                                          position.y,
                                          position.height ??
                                              (atoms ?? []).find(
                                                  (atom) => atom.name === instance.nodeType
                                              )?.estimatedHeight ??
                                              DEFAULT_ATOM_CHIP_HEIGHT,
                                          atomViewport ??
                                              (virtualizeAtoms ? nativeAtomViewport : undefined)
                                      )
                                  }
                                  atomProps={{
                                      attrs: instance.attrs,
                                      selected: selectedKeys.has(instance.key),
                                      readOnly: !editable,
                                      interactive: atomsInteractive,
                                      isViewer: false,
                                      nodeType: instance.nodeType,
                                      updateAttrs: (attrs) =>
                                          invokeAtomAttrsUpdate(
                                              documentHandle,
                                              instance,
                                              atomState.documentVersion,
                                              attrs
                                          ),
                                      editor: {
                                          select: () =>
                                              invokeAtomAction(
                                                  documentHandle,
                                                  instance,
                                                  atomState.documentVersion,
                                                  'select'
                                              ),
                                          delete: () =>
                                              invokeAtomAction(
                                                  documentHandle,
                                                  instance,
                                                  atomState.documentVersion,
                                                  'delete'
                                              ),
                                          focusBefore: () =>
                                              invokeAtomAction(
                                                  documentHandle,
                                                  instance,
                                                  atomState.documentVersion,
                                                  'before'
                                              ),
                                          focusAfter: () =>
                                              invokeAtomAction(
                                                  documentHandle,
                                                  instance,
                                                  atomState.documentVersion,
                                                  'after'
                                              ),
                                      },
                                  }}
                              />
                          </View>
                      );
                  }),
        [
            atoms,
            atomViewport,
            documentHandle,
            invokeAtomAction,
            virtualizeAtoms,
            nativeAtomViewport,
            atomsInteractive,
            atomComponents,
            atomContentWidth,
            atomPositions,
            atomState.instances,
            editable,
            invokeAtomAttrsUpdate,
            selectedKeys,
        ]
    );

    const isLinkActive = activeState.marks.link === true;

    const allowsLink = activeState.allowedMarks.includes('link');

    const canInsertImage = activeState.insertableNodes.includes(IMAGE_NODE_NAME);

    const canRequestLink = typeof onRequestLink === 'function';

    const canRequestImage = typeof onRequestImage === 'function';

    const cachedToolbarItems = toolbarItemsSerializationCacheRef.current;

    let toolbarItemsJson: string;

    if (
        cachedToolbarItems &&
        cachedToolbarItems.toolbarItems === toolbarItems &&
        cachedToolbarItems.editable === editable &&
        cachedToolbarItems.isLinkActive === isLinkActive &&
        cachedToolbarItems.allowsLink === allowsLink &&
        cachedToolbarItems.canRequestLink === canRequestLink &&
        cachedToolbarItems.canRequestImage === canRequestImage &&
        cachedToolbarItems.canInsertImage === canInsertImage
    ) {
        toolbarItemsJson = cachedToolbarItems.serialized;
    } else {
        const mappedItems = mapToolbarItemsForNative(
            toolbarItems,
            activeState,
            editable,
            onRequestLink,
            onRequestImage
        );
        toolbarItemsJson = stringifyCachedJson(mappedItems);
        toolbarItemsSerializationCacheRef.current = {
            toolbarItems,
            editable,
            isLinkActive,
            allowsLink,
            canRequestLink,
            canRequestImage,
            canInsertImage,
            serialized: toolbarItemsJson,
        };
    }

    // A room document awaiting the server renders nothing (loading), never an
    // unshared fallback paragraph.
    if (!document.isReady) return null;

    const usesNativeKeyboardToolbar =
        toolbarPlacement === 'keyboard' && (Platform.OS === 'ios' || Platform.OS === 'android');

    const shouldRenderJsToolbar = showToolbar && !usesNativeKeyboardToolbar && editable;

    const inlineToolbarMarginTop = theme?.toolbar?.marginTop ?? 8;

    const containerMinHeight = StyleSheet.flatten(containerStyle)?.minHeight;

    const nativeViewStyleParts: StyleProp<ViewStyle>[] = [];

    if (containerMinHeight != null) {
        nativeViewStyleParts.push({ minHeight: containerMinHeight });
    }

    if (style != null) {
        nativeViewStyleParts.push(style);
    }

    if (heightBehavior === 'autoGrow' && autoGrowHeight != null) {
        nativeViewStyleParts.push({ height: autoGrowHeight });
    }

    const nativeViewStyle =
        nativeViewStyleParts.length <= 1 ? nativeViewStyleParts[0] : nativeViewStyleParts;

    const currentPushedUpdate = pushedUpdate?.editorId === editorId ? pushedUpdate : null;

    const focusPreservingFrames = [...registeredToolbarFrames, ...suppliedFocusPreservingFrames];

    const toolbarFrameJson = serializeToolbarFrames(
        editable && isFocused ? focusPreservingFrames : undefined
    );

    return (
        <View style={[styles.container, containerStyle]}>
            <NativeEditorView
                ref={nativeViewRef}
                style={nativeViewStyle}
                onLayout={refreshFocusPreservingFrames}
                accessibilityLabel={accessibilityLabel}
                accessibilityHint={accessibilityHint}
                editorId={editorId}
                placeholder={placeholder}
                editable={editable}
                autoFocus={autoFocus}
                autoCapitalize={autoCapitalize}
                autoCorrect={autoCorrect}
                keyboardType={keyboardType}
                {...(Platform.OS === 'android' ? { androidInputOptionsJson } : {})}
                showToolbar={showToolbar}
                toolbarPlacement={toolbarPlacement}
                heightBehavior={heightBehavior}
                allowImageResizing={allowImageResizing}
                imageLoadingPolicyJson={imageLoadingPolicyJson}
                themeJson={themeJson}
                addonsJson={addonsJson}
                atomsJson={atomsJson}
                toolbarItemsJson={toolbarItemsJson}
                toolbarFrameJson={toolbarFrameJson}
                remoteSelectionsJson={remoteSelectionsJson}
                editorUpdateJson={currentPushedUpdate?.json}
                editorUpdateResetJson={currentPushedUpdate?.resetJson}
                editorUpdateEditorId={currentPushedUpdate?.editorId}
                editorUpdateRevision={currentPushedUpdate?.revision ?? 0}
                onEditorUpdate={handleEditorUpdate}
                onEditorError={handleEditorError}
                onExternalTextCompositionEnd={handleExternalTextCompositionEnd}
                onSelectionChange={handleSelectionChange}
                onFocusChange={handleFocusChange}
                onContentHeightChange={handleContentHeightChange}
                onAtomLayout={handleAtomLayout}
                onToolbarAction={handleToolbarAction}
                onAddonEvent={handleAddonEvent}>
                {atomChildren}
            </NativeEditorView>
            {shouldRenderJsToolbar ? (
                <View
                    testID='native-editor-js-toolbar'
                    style={[styles.inlineToolbar, { marginTop: inlineToolbarMarginTop }]}>
                    <EditorToolbarFrameOwnerProvider ownerId={toolbarFrameOwnerId}>
                        <EditorToolbar
                            activeState={activeState}
                            historyState={document.historyState}
                            toolbarItems={toolbarItems}
                            theme={theme?.toolbar}
                            showTopBorder={theme?.toolbar?.showTopBorder ?? false}
                            preserveEditorFocus={false}
                            onToggleMark={commandToggleMark}
                            onToggleListType={(listType: EditorToolbarListType) =>
                                commandToggleList(listType)
                            }
                            onToggleHeading={commandToggleHeading}
                            onToggleBlockquote={commandToggleBlockquote}
                            onInsertNodeType={commandInsertNode}
                            onRunCommand={(command: EditorToolbarCommand) => {
                                switch (command) {
                                    case 'indentList':
                                        commandIndentListItem();
                                        break;
                                    case 'outdentList':
                                        commandOutdentListItem();
                                        break;
                                    case 'undo':
                                        document.undo();
                                        break;
                                    case 'redo':
                                        document.redo();
                                        break;
                                }
                            }}
                            onRequestLink={onRequestLink ? openLinkRequest : undefined}
                            onRequestImage={onRequestImage ? openImageRequest : undefined}
                            onToolbarAction={onToolbarAction}
                            onToggleBold={() => commandToggleMark('bold')}
                            onToggleItalic={() => commandToggleMark('italic')}
                            onToggleUnderline={() => commandToggleMark('underline')}
                            onToggleStrike={() => commandToggleMark('strike')}
                            onUndo={document.undo}
                            onRedo={document.redo}
                        />
                    </EditorToolbarFrameOwnerProvider>
                </View>
            ) : null}
        </View>
    );
}
