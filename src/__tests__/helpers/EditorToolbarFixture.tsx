//
// Tests cover:
// - Renders all 12 buttons (B, I, U, S, bullet list, ordered list, indent, outdent, line break, HR, undo, redo)
// - Active mark state shows visual feedback (bold: true -> button has active style)
// - Disabled state for undo/redo (canUndo: false -> undo button disabled)
// - Button presses fire correct callbacks
// - Uses Record<string, boolean> for ActiveState (not string[])

import React from 'react';

import { Keyboard, ScrollView, StyleSheet, Text, View } from 'react-native';

import { render, fireEvent, act } from '@testing-library/react-native';

import {
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    EditorToolbar,
    _resetEditorToolbarFrameRegistryForTests,
    setActiveEditorToolbarFrameOwnerForEditor,
    setEditorToolbarMentionState,
    useEditorToolbarFrames,
    type EditorToolbarProps,
} from '../../EditorToolbar';

import type { ActiveState, HistoryState } from '../../NativeEditorBridge';

export const EMPTY_ACTIVE_STATE: ActiveState = {
    marks: {},
    markAttrs: {},
    nodes: {},
    commands: {},
    allowedMarks: [],
    insertableNodes: [],
};

export const ENABLED_BUTTONS_ACTIVE_STATE: ActiveState = {
    marks: {},
    markAttrs: {},
    nodes: {},
    commands: {
        wrapBulletList: true,
        wrapOrderedList: true,
    },
    allowedMarks: ['bold', 'italic', 'underline', 'strike'],
    insertableNodes: ['hard_break', 'horizontal_rule'],
};

export const EMPTY_HISTORY_STATE: HistoryState = { canUndo: false, canRedo: false };

export function renderToolbar(
    overrides: Partial<Omit<EditorToolbarProps, 'activeState' | 'historyState'>> & {
        activeState?: Partial<ActiveState>;
        historyState?: Partial<HistoryState>;
    } = {},
    options?: Parameters<typeof render>[1]
) {
    const defaultProps: EditorToolbarProps = {
        activeState: {
            ...EMPTY_ACTIVE_STATE,
            ...overrides.activeState,
            marks: {
                ...EMPTY_ACTIVE_STATE.marks,
                ...overrides.activeState?.marks,
            },
            markAttrs: {
                ...EMPTY_ACTIVE_STATE.markAttrs,
                ...overrides.activeState?.markAttrs,
            },
            nodes: {
                ...EMPTY_ACTIVE_STATE.nodes,
                ...overrides.activeState?.nodes,
            },
            commands: {
                ...EMPTY_ACTIVE_STATE.commands,
                ...overrides.activeState?.commands,
            },
            allowedMarks: overrides.activeState?.allowedMarks ?? EMPTY_ACTIVE_STATE.allowedMarks,
            insertableNodes:
                overrides.activeState?.insertableNodes ?? EMPTY_ACTIVE_STATE.insertableNodes,
        },
        historyState: {
            ...EMPTY_HISTORY_STATE,
            ...overrides.historyState,
        },
        onToggleBold: jest.fn(),
        onToggleItalic: jest.fn(),
        onToggleUnderline: jest.fn(),
        onToggleStrike: jest.fn(),
        onToggleBlockquote: jest.fn(),
        onToggleBulletList: jest.fn(),
        onToggleHeading: jest.fn(),
        onToggleOrderedList: jest.fn(),
        onIndentList: jest.fn(),
        onOutdentList: jest.fn(),
        onInsertHorizontalRule: jest.fn(),
        onInsertLineBreak: jest.fn(),
        onUndo: jest.fn(),
        onRedo: jest.fn(),
        ...overrides,
    };
    return { ...render(<EditorToolbar {...defaultProps} />, options), props: defaultProps };
}

export function ToolbarFrameProbe({ ownerId }: { ownerId: number }) {
    const frames = useEditorToolbarFrames(ownerId);
    return <Text testID='toolbar-frame-probe'>{JSON.stringify(frames)}</Text>;
}

afterEach(() => {
    act(() => {
        _resetEditorToolbarFrameRegistryForTests();
    });
});
