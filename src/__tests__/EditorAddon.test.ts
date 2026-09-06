import { normalizeEditorAddons, serializeEditorAddons } from '../addons';
import { createMentionsAddon, type EditorAddons } from '../EditorAddon';

const highlighting = {
    id: 'code-highlighting',
    version: 1,
    capability: 'code-highlighting',
    options: { provider: 'syntect', theme: 'base16-ocean.dark' },
} as const;

describe('addon descriptors', () => {
    it('accepts readonly conditional arrays and preserves live mention callbacks', () => {
        const onQueryChange = jest.fn();
        const mentions = createMentionsAddon({ onQueryChange });
        const addons: EditorAddons = [false, mentions, null, highlighting, undefined] as const;
        expect(normalizeEditorAddons(addons)).toEqual({
            mentions: { onQueryChange },
            codeHighlighting: highlighting.options,
        });
        expect(normalizeEditorAddons([])).toEqual({});
        expect(normalizeEditorAddons()).toEqual({});
    });

    it('copies and freezes configuration without freezing caller data', () => {
        const options = { suggestions: [{ key: 'alice', title: 'Alice', attrs: { id: '1' } }] };
        const descriptor = createMentionsAddon(options);
        options.suggestions[0].attrs.id = '2';
        expect(descriptor.options.suggestions?.[0].attrs?.id).toBe('1');
        expect(Object.isFrozen(descriptor)).toBe(true);
        expect(Object.isFrozen(descriptor.options.suggestions?.[0].attrs)).toBe(true);
        expect(Object.isFrozen(options.suggestions)).toBe(false);
    });

    it.each([
        [[highlighting, highlighting], 'duplicate'],
        [[{ ...highlighting, version: 2 }], 'version'],
        [[{ ...highlighting, capability: 'unknown' }], 'capability'],
        [[{ ...highlighting, id: 'other' }], 'id'],
        [[{ ...highlighting, options: { provider: '', theme: 'dark' } }], 'provider'],
        [[{ ...highlighting, options: { provider: 'syntect', theme: '' } }], 'theme'],
        [[[highlighting]], 'descriptor'],
        [{ mentions: {} }, 'array'],
    ])('rejects invalid descriptors %#', (addons, message) => {
        expect(() => normalizeEditorAddons(addons as EditorAddons)).toThrow(
            new RegExp(message, 'i')
        );
    });

    it('serializes native capabilities without callbacks and clears removed addons', () => {
        const descriptor = createMentionsAddon({
            onPress: jest.fn(),
            onQueryChange: jest.fn(),
            resolveTheme: () => undefined,
        });
        expect(JSON.parse(serializeEditorAddons([descriptor, highlighting])!)).toEqual({
            mentions: { trigger: '@', suggestions: [], resolveTheme: true },
            codeHighlighting: highlighting.options,
        });
        expect(serializeEditorAddons([highlighting])).toBe(
            JSON.stringify({ codeHighlighting: highlighting.options })
        );
        expect(serializeEditorAddons([])).toBeUndefined();
    });
});
