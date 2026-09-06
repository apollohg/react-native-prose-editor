import type { MentionsAddonConfig } from './addons';

export interface MentionPressEvent {
    docPos: number;
    label: string;
    attrs: Record<string, unknown>;
}

export interface MentionsAddonOptions extends MentionsAddonConfig {
    /** Prepended to viewer mention labels unless already present. */
    prefix?: string;
    /** Enables mention presses in the viewer. */
    onPress?: (event: MentionPressEvent) => void;
}

export interface CodeHighlightingAddonOptions {
    readonly provider: string;
    readonly theme: string;
}

export interface MentionsAddon {
    readonly id: 'mentions';
    readonly version: 1;
    readonly capability: 'mentions';
    readonly options: Readonly<MentionsAddonOptions>;
}

export interface CodeHighlightingAddon {
    readonly id: 'code-highlighting';
    readonly version: 1;
    readonly capability: 'code-highlighting';
    readonly options: CodeHighlightingAddonOptions;
}

export type EditorAddon = MentionsAddon | CodeHighlightingAddon;
export type EditorAddonEntry = EditorAddon | false | null | undefined;
export type EditorAddons = readonly EditorAddonEntry[];

function copyConfiguration<T>(value: T, ancestors = new Set<object>()): T {
    if (value === null || typeof value !== 'object') return value;
    if (ancestors.has(value)) throw new Error('Addon configuration must not contain cycles.');
    if (!Array.isArray(value) && Object.getPrototypeOf(value) !== Object.prototype) {
        throw new Error('Addon configuration must contain only plain objects and arrays.');
    }
    ancestors.add(value);
    const copy = Array.isArray(value)
        ? value.map((entry) => copyConfiguration(entry, ancestors))
        : Object.fromEntries(
              Object.entries(value).map(([key, entry]) => [
                  key,
                  copyConfiguration(entry, ancestors),
              ])
          );
    ancestors.delete(value);
    return Object.freeze(copy) as T;
}

export function createMentionsAddon(options: MentionsAddonOptions = {}): MentionsAddon {
    return Object.freeze({
        id: 'mentions',
        version: 1,
        capability: 'mentions',
        options: copyConfiguration(options),
    });
}
