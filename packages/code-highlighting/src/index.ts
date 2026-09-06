import { requireNativeModule } from 'expo-modules-core';
import type { CodeHighlightingAddon } from 'react-native-rich-text-editor';

const nativeModule = requireNativeModule<{ initialize(): number }>('NativeCodeHighlighting');
if (nativeModule.initialize() !== 1) {
    throw new Error(
        'Code highlighting requires native provider interface version 1. Rebuild the app.'
    );
}

export const codeHighlightingThemes = Object.freeze([
    'base16-ocean.dark',
    'base16-eighties.dark',
    'base16-mocha.dark',
    'base16-ocean.light',
    'InspiredGitHub',
    'Solarized (dark)',
    'Solarized (light)',
] as const);

export type CodeHighlightingTheme = (typeof codeHighlightingThemes)[number];

export interface CodeHighlightingOptions {
    readonly theme: CodeHighlightingTheme;
}

export function createCodeHighlightingAddon(
    options: CodeHighlightingOptions
): CodeHighlightingAddon {
    if (!options || !codeHighlightingThemes.includes(options.theme)) {
        throw new Error(
            'Unsupported code-highlighting theme. Use a name from codeHighlightingThemes.'
        );
    }
    return Object.freeze({
        id: 'code-highlighting',
        version: 1,
        capability: 'code-highlighting',
        options: Object.freeze({ provider: 'syntect', theme: options.theme }),
    });
}
