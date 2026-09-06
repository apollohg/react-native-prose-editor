import { useCallback, useContext, useEffect, useRef, useState } from 'react';
import { View, useWindowDimensions } from 'react-native';
import { DEFAULT_EDITOR_TOOLBAR_ITEMS } from './EditorToolbarItems';
import { type EditorToolbarProps, type ToolbarMenuState } from './EditorToolbarTypes';
import {
    EditorToolbarFrameOwnerContext,
    useEditorToolbarMentionState,
    allocateEditorToolbarRegistrationId,
} from './EditorToolbarRegistry';

export function useEditorToolbarState({
    activeState,
    historyState,
    onToggleBold,
    onToggleItalic,
    onToggleUnderline,
    onToggleStrike,
    onToggleBulletList,
    onToggleHeading,
    onToggleBlockquote,
    onToggleOrderedList,
    onIndentList,
    onOutdentList,
    onInsertHorizontalRule,
    onInsertLineBreak,
    onUndo,
    onRedo,
    onToggleMark,
    onToggleListType,
    onInsertNodeType,
    onRunCommand,
    onToolbarAction,
    onRequestLink,
    onRequestImage,
    toolbarItems = DEFAULT_EDITOR_TOOLBAR_ITEMS,
    theme,
    showTopBorder,
    preserveEditorFocus = true,
}: EditorToolbarProps) {
    const marks = activeState.marks ?? {};

    const nodes = activeState.nodes ?? {};

    const commands = activeState.commands ?? {};

    const allowedMarks = activeState.allowedMarks ?? [];

    const insertableNodes = activeState.insertableNodes ?? [];

    const frameOwnerId = useContext(EditorToolbarFrameOwnerContext);

    const publishesFocusFrames = preserveEditorFocus || frameOwnerId != null;

    const rootRef = useRef<View | null>(null);

    const menuCardRef = useRef<View | null>(null);

    const groupButtonRefs = useRef(new Map<string, View | null>());

    const { width: windowWidth, height: windowHeight } = useWindowDimensions();

    const [expandedGroupKey, setExpandedGroupKey] = useState<string | null>(null);

    const [menuState, setMenuState] = useState<ToolbarMenuState | null>(null);

    const frameOwnerIdRef = useRef(frameOwnerId);

    frameOwnerIdRef.current = frameOwnerId;

    const publishesFocusFramesRef = useRef(publishesFocusFrames);

    publishesFocusFramesRef.current = publishesFocusFrames;

    const menuStateRef = useRef(menuState);

    menuStateRef.current = menuState;

    const framePublisherMountedRef = useRef(false);

    const mentionState = useEditorToolbarMentionState();

    const toolbarInteractionActiveRef = useRef(false);

    const framePublishAnimationFramesRef = useRef<number[]>([]);

    const framePublishTimeoutsRef = useRef<ReturnType<typeof setTimeout>[]>([]);

    const registrationIdRef = useRef<number | null>(null);

    const menuRegistrationIdRef = useRef<number | null>(null);

    if (registrationIdRef.current == null) {
        registrationIdRef.current = allocateEditorToolbarRegistrationId();
    }

    if (menuRegistrationIdRef.current == null) {
        menuRegistrationIdRef.current = allocateEditorToolbarRegistrationId();
    }

    useEffect(() => {
        framePublisherMountedRef.current = true;
        return () => {
            framePublisherMountedRef.current = false;
        };
    }, []);

    const isMarkActive = useCallback((mark: string) => !!marks[mark], [marks]);

    const canIndentList = !!commands['indentList'];

    const canOutdentList = !!commands['outdentList'];

    const shouldRenderMentionSuggestions =
        publishesFocusFrames && mentionState != null && mentionState.suggestions.length > 0;
    return {
        onToggleMark,
        onToggleBold,
        onToggleItalic,
        onToggleUnderline,
        onToggleStrike,
        onToggleListType,
        onToggleBulletList,
        onToggleOrderedList,
        onRequestLink,
        onRequestImage,
        onToggleHeading,
        onToggleBlockquote,
        onInsertNodeType,
        onInsertLineBreak,
        onInsertHorizontalRule,
        onRunCommand,
        onIndentList,
        onOutdentList,
        onUndo,
        onRedo,
        onToolbarAction,
        isMarkActive,
        allowedMarks,
        insertableNodes,
        nodes,
        commands,
        canIndentList,
        canOutdentList,
        historyState,
        toolbarItems,
        expandedGroupKey,
        menuState,
        showTopBorder,
        theme,
        registrationIdRef,
        rootRef,
        publishesFocusFrames,
        frameOwnerId,
        framePublisherMountedRef,
        publishesFocusFramesRef,
        frameOwnerIdRef,
        menuRegistrationIdRef,
        menuCardRef,
        menuStateRef,
        framePublishAnimationFramesRef,
        framePublishTimeoutsRef,
        windowHeight,
        windowWidth,
        toolbarInteractionActiveRef,
        setExpandedGroupKey,
        setMenuState,
        shouldRenderMentionSuggestions,
        preserveEditorFocus,
        groupButtonRefs,
        mentionState,
    };
}
