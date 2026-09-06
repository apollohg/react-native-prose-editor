import './helpers/NativeEditorBridgeV2Fixture';

import { validEditorMentionTheme } from '../NativeEditorBridge';

describe('NativeEditorBridge v2', () => {
    describe('mention theme validation (render read path)', () => {
        it('accepts a surface-grouped theme with both node and option styling', () => {
            expect(
                validEditorMentionTheme({
                    node: { textColor: '#0A84FF' },
                    suggestions: { option: { textColor: '#0A84FF' } },
                })
            ).toBe(true);
        });

        it('accepts a base theme carrying box styling on the option', () => {
            expect(
                validEditorMentionTheme({
                    node: { fontWeight: '600' },
                    suggestions: {
                        option: {
                            backgroundColor: '#FFFFFF',
                            borderRadius: 9999,
                            fontWeight: '600',
                        },
                    },
                })
            ).toBe(true);
        });

        it('accepts a React Native numeric fontWeight', () => {
            expect(validEditorMentionTheme({ node: { fontWeight: 600 } })).toBe(true);
        });

        it('rejects a non-numeric borderRadius', () => {
            expect(
                validEditorMentionTheme({ suggestions: { option: { borderRadius: '50%' } } })
            ).toBe(false);
        });

        it('rejects the pre-1.0 flat shape', () => {
            expect(validEditorMentionTheme({ textColor: '#CC0000' })).toBe(false);
        });
    });
});

it('rejects malformed legacy color aliases before mention insertion', () => {
    expect(validEditorMentionTheme({ node: { textColor: 'not-a-color', fontSize: 18 } })).toBe(
        false
    );
});
