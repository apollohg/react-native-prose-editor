import type {
    CodeHighlightingAddonOptions,
    EditorAddons,
    MentionsAddonOptions,
} from './EditorAddon';

export interface NormalizedEditorAddons {
    mentions?: Readonly<MentionsAddonOptions>;
    codeHighlighting?: CodeHighlightingAddonOptions;
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return value !== null && typeof value === 'object' && !Array.isArray(value);
}

export function normalizeEditorAddons(addons?: EditorAddons): NormalizedEditorAddons {
    if (addons === undefined) return {};
    if (!Array.isArray(addons)) throw new Error('addons must be a flat readonly array.');
    const normalized: NormalizedEditorAddons = {};
    const ids = new Set<string>();
    for (const [index, addon] of addons.entries()) {
        if (addon === false || addon === null || addon === undefined) continue;
        const path = `addons[${index}]`;
        if (!isRecord(addon)) throw new Error(`${path} must be an addon descriptor.`);
        if (addon.version !== 1) throw new Error(`${path}.version must be 1.`);
        if (addon.capability !== 'mentions' && addon.capability !== 'code-highlighting') {
            throw new Error(`${path}.capability is unsupported.`);
        }
        if (addon.id !== addon.capability) throw new Error(`${path}.id must match its capability.`);
        if (ids.has(addon.capability))
            throw new Error(`${path}: duplicate addon id '${addon.id}'.`);
        ids.add(addon.capability);
        if (!isRecord(addon.options)) throw new Error(`${path}.options must be an object.`);
        if (addon.capability === 'mentions') {
            normalized.mentions = addon.options;
        } else {
            for (const key of ['provider', 'theme'] as const) {
                if (typeof addon.options[key] !== 'string' || !addon.options[key].trim()) {
                    throw new Error(`${path}.options.${key} must be a nonempty string.`);
                }
            }
            normalized.codeHighlighting = {
                provider: addon.options.provider as string,
                theme: addon.options.theme as string,
            };
        }
    }
    return normalized;
}
