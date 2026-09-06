jest.mock('expo-modules-core', () => ({
    requireNativeModule: jest.fn(() => ({ initialize: jest.fn(() => 1) })),
}));

import { createCodeHighlightingAddon } from '../../packages/code-highlighting/src';
import { serializeEditorAddons } from '../addons';
import { requireNativeModule } from 'expo-modules-core';

describe('code-highlighting companion factory', () => {
    it('registers on import then creates immutable descriptors without native calls', () => {
        const descriptor = createCodeHighlightingAddon({ theme: 'base16-ocean.dark' });
        expect(requireNativeModule).toHaveBeenCalledTimes(1);
        expect(descriptor).toEqual({
            id: 'code-highlighting',
            version: 1,
            capability: 'code-highlighting',
            options: { provider: 'syntect', theme: 'base16-ocean.dark' },
        });
        expect(Object.isFrozen(descriptor.options)).toBe(true);
        expect(JSON.parse(serializeEditorAddons([descriptor])!)).toEqual({
            codeHighlighting: descriptor.options,
        });
        createCodeHighlightingAddon({ theme: 'base16-ocean.light' });
        expect(requireNativeModule).toHaveBeenCalledTimes(1);
    });

    it('rejects invalid or unsupported themes at construction', () => {
        expect(() => createCodeHighlightingAddon({ theme: '' })).toThrow(/theme/);
        expect(() => createCodeHighlightingAddon({ theme: 'missing' })).toThrow(/theme/);
    });
    it('rejects an incompatible linked provider before exposing a factory', () => {
        (requireNativeModule as jest.Mock).mockReturnValueOnce({ initialize: () => 2 });
        expect(() =>
            jest.isolateModules(() => require('../../packages/code-highlighting/src'))
        ).toThrow(/version 1.*Rebuild/);
    });
});
