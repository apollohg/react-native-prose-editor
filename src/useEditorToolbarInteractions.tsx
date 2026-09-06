import { useCallback, useEffect } from 'react';
import { Keyboard } from 'react-native';
import { useEditorToolbarState } from './useEditorToolbarState';
import { useEditorToolbarItems } from './useEditorToolbarItems';
import {
    unregisterEditorToolbarFrame,
    registerEditorToolbarFrame,
    endEditorToolbarInteraction,
    beginEditorToolbarInteraction,
} from './EditorToolbarRegistry';
import { KEYBOARD_FRAME_REMEASURE_DELAYS_MS } from './EditorToolbarVisuals';
import { type ToolbarButton, type ToolbarGroupButton } from './EditorToolbarTypes';

export function useEditorToolbarInteractions(
    context: Pick<
        ReturnType<typeof useEditorToolbarState>,
        | 'showTopBorder'
        | 'theme'
        | 'registrationIdRef'
        | 'rootRef'
        | 'publishesFocusFrames'
        | 'frameOwnerId'
        | 'framePublisherMountedRef'
        | 'publishesFocusFramesRef'
        | 'frameOwnerIdRef'
        | 'menuRegistrationIdRef'
        | 'menuCardRef'
        | 'menuState'
        | 'menuStateRef'
        | 'framePublishAnimationFramesRef'
        | 'framePublishTimeoutsRef'
        | 'expandedGroupKey'
        | 'windowHeight'
        | 'windowWidth'
        | 'toolbarInteractionActiveRef'
        | 'setExpandedGroupKey'
        | 'setMenuState'
        | 'shouldRenderMentionSuggestions'
        | 'preserveEditorFocus'
        | 'groupButtonRefs'
    > &
        Pick<
            ReturnType<typeof useEditorToolbarItems>,
            'startItems' | 'scrollItems' | 'endItems' | 'groupsByKey'
        >
) {
    const {
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
        menuState,
        menuStateRef,
        framePublishAnimationFramesRef,
        framePublishTimeoutsRef,
        expandedGroupKey,
        startItems,
        scrollItems,
        endItems,
        windowHeight,
        windowWidth,
        toolbarInteractionActiveRef,
        groupsByKey,
        setExpandedGroupKey,
        setMenuState,
        shouldRenderMentionSuggestions,
        preserveEditorFocus,
        groupButtonRefs,
    } = context;

    const resolvedShowTopBorder = showTopBorder ?? theme?.showTopBorder ?? true;

    const publishToolbarFrame = useCallback(() => {
        const registrationId = registrationIdRef.current;
        const toolbar = rootRef.current;
        if (!publishesFocusFrames || registrationId == null || !toolbar) {
            if (registrationId != null) {
                unregisterEditorToolbarFrame(registrationId);
            }
            return;
        }

        if (typeof toolbar.measureInWindow !== 'function') {
            return;
        }

        const measuredFrameOwnerId = frameOwnerId;
        toolbar.measureInWindow((x, y, width, height) => {
            if (
                !framePublisherMountedRef.current ||
                !publishesFocusFramesRef.current ||
                frameOwnerIdRef.current !== measuredFrameOwnerId
            ) {
                return;
            }
            registerEditorToolbarFrame(
                registrationId,
                { x, y, width, height },
                measuredFrameOwnerId
            );
        });
    }, [frameOwnerId, publishesFocusFrames]);

    const publishMenuFrame = useCallback(() => {
        const registrationId = menuRegistrationIdRef.current;
        const menuCard = menuCardRef.current;
        if (!publishesFocusFrames || menuState == null || registrationId == null || !menuCard) {
            if (registrationId != null) {
                unregisterEditorToolbarFrame(registrationId);
            }
            return;
        }

        if (typeof menuCard.measureInWindow !== 'function') {
            return;
        }

        const measuredFrameOwnerId = frameOwnerId;
        const measuredMenuState = menuState;
        menuCard.measureInWindow((x, y, width, height) => {
            if (
                !framePublisherMountedRef.current ||
                !publishesFocusFramesRef.current ||
                frameOwnerIdRef.current !== measuredFrameOwnerId ||
                menuStateRef.current !== measuredMenuState
            ) {
                return;
            }
            registerEditorToolbarFrame(
                registrationId,
                { x, y, width, height },
                measuredFrameOwnerId
            );
        });
    }, [frameOwnerId, menuState, publishesFocusFrames]);

    const publishToolbarFrames = useCallback(() => {
        publishToolbarFrame();
        publishMenuFrame();
    }, [publishMenuFrame, publishToolbarFrame]);

    const cancelScheduledFramePublishes = useCallback(() => {
        framePublishAnimationFramesRef.current.forEach((frame) => cancelAnimationFrame(frame));
        framePublishAnimationFramesRef.current = [];
        framePublishTimeoutsRef.current.forEach((timeout) => clearTimeout(timeout));
        framePublishTimeoutsRef.current = [];
    }, []);

    const scheduleToolbarFramePublish = useCallback(() => {
        if (!publishesFocusFrames) {
            return;
        }

        cancelScheduledFramePublishes();
        publishToolbarFrames();

        framePublishAnimationFramesRef.current.push(requestAnimationFrame(publishToolbarFrames));
        KEYBOARD_FRAME_REMEASURE_DELAYS_MS.forEach((delay) => {
            framePublishTimeoutsRef.current.push(setTimeout(publishToolbarFrames, delay));
        });
    }, [cancelScheduledFramePublishes, publishesFocusFrames, publishToolbarFrames]);

    const handleToolbarLayout = useCallback(() => {
        requestAnimationFrame(publishToolbarFrame);
    }, [publishToolbarFrame]);

    const handleMenuLayout = useCallback(() => {
        requestAnimationFrame(publishMenuFrame);
    }, [publishMenuFrame]);

    useEffect(() => {
        if (!publishesFocusFrames) {
            const registrationId = registrationIdRef.current;
            if (registrationId != null) {
                unregisterEditorToolbarFrame(registrationId);
            }
            return;
        }

        const frame = requestAnimationFrame(publishToolbarFrame);
        return () => cancelAnimationFrame(frame);
    }, [
        expandedGroupKey,
        menuState?.groupKey,
        publishesFocusFrames,
        publishToolbarFrame,
        startItems.length,
        scrollItems.length,
        endItems.length,
        windowHeight,
        windowWidth,
    ]);

    useEffect(() => {
        if (!publishesFocusFrames || menuState == null) {
            const registrationId = menuRegistrationIdRef.current;
            if (registrationId != null) {
                unregisterEditorToolbarFrame(registrationId);
            }
            return;
        }

        const frame = requestAnimationFrame(publishMenuFrame);
        return () => cancelAnimationFrame(frame);
    }, [menuState, publishesFocusFrames, publishMenuFrame, windowHeight, windowWidth]);

    useEffect(() => {
        const registrationId = registrationIdRef.current;
        const menuRegistrationId = menuRegistrationIdRef.current;
        return () => {
            cancelScheduledFramePublishes();
            if (toolbarInteractionActiveRef.current) {
                toolbarInteractionActiveRef.current = false;
                endEditorToolbarInteraction();
            }
            if (registrationId != null) {
                unregisterEditorToolbarFrame(registrationId);
            }
            if (menuRegistrationId != null) {
                unregisterEditorToolbarFrame(menuRegistrationId);
            }
        };
    }, [cancelScheduledFramePublishes]);

    useEffect(() => {
        if (!publishesFocusFrames) {
            cancelScheduledFramePublishes();
            return;
        }

        const subscriptions = [
            Keyboard.addListener('keyboardDidShow', scheduleToolbarFramePublish),
            Keyboard.addListener('keyboardDidHide', scheduleToolbarFramePublish),
            Keyboard.addListener('keyboardDidChangeFrame', scheduleToolbarFramePublish),
        ];

        return () => {
            subscriptions.forEach((subscription) => subscription.remove());
            cancelScheduledFramePublishes();
        };
    }, [cancelScheduledFramePublishes, publishesFocusFrames, scheduleToolbarFramePublish]);

    useEffect(() => {
        if (expandedGroupKey != null && !groupsByKey.has(expandedGroupKey)) {
            setExpandedGroupKey(null);
        }
    }, [expandedGroupKey, groupsByKey]);

    useEffect(() => {
        if (menuState != null && !groupsByKey.has(menuState.groupKey)) {
            setMenuState(null);
        }
    }, [groupsByKey, menuState]);

    useEffect(() => {
        if (shouldRenderMentionSuggestions) {
            setExpandedGroupKey(null);
            setMenuState(null);
        }
    }, [shouldRenderMentionSuggestions]);

    const handleButtonPress = useCallback((button: ToolbarButton) => {
        button.action();
        if (button.groupKey) {
            setExpandedGroupKey((current) => (current === button.groupKey ? null : current));
        }
        setMenuState(null);
    }, []);

    const handleToolbarPressIn = useCallback(() => {
        if (preserveEditorFocus && !toolbarInteractionActiveRef.current) {
            toolbarInteractionActiveRef.current = true;
            beginEditorToolbarInteraction();
        }
    }, [preserveEditorFocus]);

    const handleToolbarPressOut = useCallback(() => {
        if (preserveEditorFocus && toolbarInteractionActiveRef.current) {
            toolbarInteractionActiveRef.current = false;
            endEditorToolbarInteraction();
        }
    }, [preserveEditorFocus]);

    const handleGroupPress = useCallback((group: ToolbarGroupButton) => {
        if (group.isDisabled) {
            return;
        }
        if (group.presentation === 'expand') {
            setMenuState(null);
            setExpandedGroupKey((current) => (current === group.key ? null : group.key));
            return;
        }

        const anchor = groupButtonRefs.current.get(group.key);
        if (!anchor) {
            return;
        }
        anchor.measureInWindow((x, y, width, height) => {
            setExpandedGroupKey(null);
            setMenuState((current) =>
                current?.groupKey === group.key
                    ? null
                    : {
                          groupKey: group.key,
                          x,
                          y,
                          width,
                          height,
                      }
            );
        });
    }, []);
    return {
        handleToolbarPressIn,
        handleToolbarPressOut,
        handleGroupPress,
        handleButtonPress,
        handleToolbarLayout,
        resolvedShowTopBorder,
        handleMenuLayout,
    };
}
