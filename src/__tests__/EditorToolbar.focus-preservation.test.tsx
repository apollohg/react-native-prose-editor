import './helpers/EditorToolbarFixture';
import { renderToolbar, ToolbarFrameProbe } from './helpers/EditorToolbarFixture';
import React from 'react';
import { Keyboard, ScrollView, View } from 'react-native';
import { fireEvent, act } from '@testing-library/react-native';
import {
    setActiveEditorToolbarFrameOwnerForEditor,
    setEditorToolbarMentionState,
} from '../EditorToolbar';

describe('EditorToolbar', () => {
    describe('focus preservation', () => {
        it('publishes menu actions as a second owner-scoped native hit-test frame', () => {
            jest.useFakeTimers();
            const ownerId = 17;
            const Wrapper = ({ children }: { children: React.ReactNode }) => (
                <>
                    {children}
                    <ToolbarFrameProbe ownerId={ownerId} />
                </>
            );
            act(() => {
                setActiveEditorToolbarFrameOwnerForEditor(ownerId, true);
            });
            let pendingMenuMeasurement:
                | ((x: number, y: number, width: number, height: number) => void)
                | null = null;
            const viewPrototype = (
                View as unknown as {
                    prototype: {
                        measureInWindow: (
                            callback: (x: number, y: number, width: number, height: number) => void
                        ) => void;
                    };
                }
            ).prototype;
            const measureInWindow = jest
                .spyOn(viewPrototype, 'measureInWindow')
                .mockImplementation(function (callback) {
                    const testID = (this as unknown as { props?: { testID?: string } }).props
                        ?.testID;
                    if (testID === 'editor-toolbar-root') {
                        callback(12, 24, 320, 48);
                        return;
                    }
                    if (testID === 'editor-toolbar-menu-card') {
                        pendingMenuMeasurement = callback;
                        return;
                    }
                    callback(24, 24, 44, 44);
                });

            try {
                const { getByLabelText, getByTestId } = renderToolbar(
                    {
                        toolbarItems: [
                            {
                                type: 'group',
                                key: 'headings',
                                label: 'Headings',
                                icon: { type: 'glyph', text: 'H' },
                                presentation: 'menu',
                                items: [
                                    {
                                        type: 'heading',
                                        level: 1,
                                        label: 'Heading 1',
                                        icon: { type: 'default', id: 'h1' },
                                    },
                                ],
                            },
                        ],
                        activeState: { commands: { toggleHeading1: true } },
                    },
                    { wrapper: Wrapper }
                );

                act(() => {
                    jest.runOnlyPendingTimers();
                });
                expect(JSON.parse(getByTestId('toolbar-frame-probe').props.children)).toEqual([
                    { x: 12, y: 24, width: 320, height: 48 },
                ]);

                fireEvent.press(getByLabelText('Headings'));
                act(() => {
                    jest.runOnlyPendingTimers();
                    pendingMenuMeasurement?.(180, 96, 192, 88);
                });

                expect(JSON.parse(getByTestId('toolbar-frame-probe').props.children)).toEqual([
                    { x: 12, y: 24, width: 320, height: 48 },
                    { x: 180, y: 96, width: 192, height: 88 },
                ]);

                fireEvent.press(getByLabelText('Heading 1'));
                act(() => {
                    jest.runOnlyPendingTimers();
                });

                expect(JSON.parse(getByTestId('toolbar-frame-probe').props.children)).toEqual([
                    { x: 12, y: 24, width: 320, height: 48 },
                ]);

                act(() => {
                    pendingMenuMeasurement?.(180, 96, 192, 88);
                });
                expect(JSON.parse(getByTestId('toolbar-frame-probe').props.children)).toEqual([
                    { x: 12, y: 24, width: 320, height: 48 },
                ]);
            } finally {
                measureInWindow.mockRestore();
                jest.useRealTimers();
            }
        });

        it('keeps keyboard taps persistent on toolbar scroll views', () => {
            const { UNSAFE_getByType, unmount } = renderToolbar();

            expect(UNSAFE_getByType(ScrollView).props.keyboardShouldPersistTaps).toBe('always');
            unmount();

            act(() => {
                setEditorToolbarMentionState(1, {
                    trigger: '@',
                    suggestions: [{ key: 'u1', title: 'Alice', label: '@Alice' }],
                    onSelectSuggestion: jest.fn(),
                });
            });
            const mentionToolbar = renderToolbar();

            expect(
                mentionToolbar.getByTestId('editor-toolbar-mention-suggestions').props
                    .keyboardShouldPersistTaps
            ).toBe('always');
        });

        it('prefixes raw mention suggestion labels with the trigger when rendered', () => {
            act(() => {
                setEditorToolbarMentionState(1, {
                    trigger: '@',
                    suggestions: [{ key: 'u1', title: 'Alice Chen', label: 'alice' }],
                    onSelectSuggestion: jest.fn(),
                });
            });

            const { getByLabelText, getByText } = renderToolbar();

            expect(getByText('@alice')).toBeTruthy();
            expect(getByLabelText('@alice')).toBeTruthy();
        });

        it('subscribes to keyboard layout changes while preserving editor focus', () => {
            const keyboardListeners = new Map<string, () => void>();
            const removers: jest.Mock[] = [];
            const addListenerSpy = jest
                .spyOn(Keyboard, 'addListener')
                .mockImplementation((eventName, listener) => {
                    keyboardListeners.set(eventName, listener as () => void);
                    const remove = jest.fn();
                    removers.push(remove);
                    return { remove } as ReturnType<typeof Keyboard.addListener>;
                });

            try {
                const { unmount } = renderToolbar();

                expect([...keyboardListeners.keys()]).toEqual([
                    'keyboardDidShow',
                    'keyboardDidHide',
                    'keyboardDidChangeFrame',
                ]);

                unmount();
                expect(removers).toHaveLength(3);
                removers.forEach((remove) => expect(remove).toHaveBeenCalledTimes(1));
            } finally {
                addListenerSpy.mockRestore();
            }
        });
    });
});
