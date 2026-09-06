import './helpers/EditorToolbarFixture';
import { renderToolbar } from './helpers/EditorToolbarFixture';

import { StyleSheet } from 'react-native';

describe('EditorToolbar', () => {
    // Sizing contract shared with iOS (resolvedToolbarHeight/resolvedButtonSize)
    // and Android (resolvedToolbarHeightDp/resolvedButtonSizeDp): an explicit
    // theme height is honored as-is; buttons are
    // max(1, min(MAX_BUTTON_SIZE, height - BUTTON_HEIGHT_INSET)).

    describe('theme sizing', () => {
        it('honors theme heights below 40 instead of flooring them', () => {
            const { getByLabelText, toJSON } = renderToolbar({
                theme: { height: 32 },
            });
            const rowStyle = StyleSheet.flatten(toJSON()?.props.style);
            const boldButtonStyle = StyleSheet.flatten(getByLabelText('Bold').props.style);

            // max(H, 1) = 32 — not floored to 40.
            expect(rowStyle.minHeight).toBe(32);
            // max(1, min(40, 32 - 4)) = 28 — matches iOS and Android exactly.
            expect(boldButtonStyle.height).toBe(28);
        });

        it('derives button size with the native formula for large heights', () => {
            const { getByLabelText, toJSON } = renderToolbar({
                theme: { height: 60 },
            });
            const rowStyle = StyleSheet.flatten(toJSON()?.props.style);
            const boldButtonStyle = StyleSheet.flatten(getByLabelText('Bold').props.style);

            expect(rowStyle.minHeight).toBe(60);
            // max(1, min(40, 60 - 4)) = min(40, 56) = 40.
            expect(boldButtonStyle.height).toBe(40);
        });

        it('cascades toolbar and per-button icon and state styles', () => {
            const { getByLabelText, getByText } = renderToolbar({
                theme: {
                    buttonIconSize: 18,
                    buttonColor: '#101010',
                    buttonBackgroundColor: '#050505',
                    buttonActiveColor: '#202020',
                    buttonDisabledColor: '#303030',
                    buttonActiveBackgroundColor: '#404040',
                    buttonDisabledBackgroundColor: '#505050',
                    buttonBorderRadius: 5,
                },
                toolbarItems: [
                    {
                        type: 'action',
                        key: 'global',
                        label: 'Global',
                        icon: { type: 'glyph', text: 'G' },
                    },
                    {
                        type: 'action',
                        key: 'idle',
                        label: 'Idle',
                        icon: { type: 'glyph', text: 'I' },
                        buttonStyle: {
                            iconSize: 24,
                            color: '#111111',
                            backgroundColor: '#121212',
                        },
                    },
                    {
                        type: 'action',
                        key: 'global-active',
                        label: 'Global Active',
                        icon: { type: 'glyph', text: 'T' },
                        isActive: true,
                    },
                    {
                        type: 'action',
                        key: 'active',
                        label: 'Active',
                        icon: { type: 'glyph', text: 'A' },
                        isActive: true,
                        buttonStyle: {
                            activeColor: '#222222',
                            activeBackgroundColor: '#333333',
                            borderRadius: 12,
                        },
                    },
                    {
                        type: 'action',
                        key: 'global-disabled',
                        label: 'Global Disabled',
                        icon: { type: 'glyph', text: 'E' },
                        isActive: true,
                        isDisabled: true,
                    },
                    {
                        type: 'action',
                        key: 'disabled',
                        label: 'Disabled',
                        icon: { type: 'glyph', text: 'D' },
                        isActive: true,
                        isDisabled: true,
                        buttonStyle: {
                            disabledColor: '#444444',
                            disabledBackgroundColor: '#555555',
                        },
                    },
                ],
                onToolbarAction: jest.fn(),
            });

            expect(StyleSheet.flatten(getByText('G').props.style)).toMatchObject({
                color: '#101010',
                fontSize: 18,
            });
            expect(StyleSheet.flatten(getByText('I').props.style)).toMatchObject({
                color: '#111111',
                fontSize: 24,
            });
            expect(StyleSheet.flatten(getByLabelText('Global').props.style)).toMatchObject({
                backgroundColor: '#050505',
                borderRadius: 5,
            });
            expect(StyleSheet.flatten(getByLabelText('Idle').props.style)).toMatchObject({
                backgroundColor: '#121212',
            });
            expect(StyleSheet.flatten(getByText('T').props.style)).toMatchObject({
                color: '#202020',
                fontSize: 18,
            });
            expect(StyleSheet.flatten(getByLabelText('Global Active').props.style)).toMatchObject({
                backgroundColor: '#404040',
                borderRadius: 5,
            });
            expect(StyleSheet.flatten(getByText('A').props.style)).toMatchObject({
                color: '#222222',
                fontSize: 18,
            });
            expect(StyleSheet.flatten(getByLabelText('Active').props.style)).toMatchObject({
                backgroundColor: '#333333',
                borderRadius: 12,
            });
            expect(StyleSheet.flatten(getByText('D').props.style)).toMatchObject({
                color: '#444444',
                fontSize: 18,
            });
            expect(StyleSheet.flatten(getByLabelText('Global Disabled').props.style)).toMatchObject(
                {
                    backgroundColor: '#505050',
                }
            );
            expect(StyleSheet.flatten(getByLabelText('Disabled').props.style)).toMatchObject({
                backgroundColor: '#555555',
            });
        });
    });
});
