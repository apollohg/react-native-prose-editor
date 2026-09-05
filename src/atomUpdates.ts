import type { AtomAttrsUpdate } from './atoms';
import { AtomUpdateAttrsError } from './atomInstances';

function copyJson(value: unknown, ancestors = new Set<object>()): unknown {
    if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
    if (typeof value === 'number' && Number.isFinite(value)) return value;
    if (typeof value !== 'object' || ancestors.has(value) || ancestors.size > 128) {
        throw new AtomUpdateAttrsError(
            'not-applicable',
            'Atom attributes must contain finite JSON values.'
        );
    }
    if (
        !Array.isArray(value) &&
        Object.getPrototypeOf(value) !== Object.prototype &&
        Object.getPrototypeOf(value) !== null
    ) {
        throw new AtomUpdateAttrsError(
            'not-applicable',
            'Atom attributes must contain plain JSON objects.'
        );
    }
    ancestors.add(value);
    const result = Array.isArray(value)
        ? Array.from(value, (entry) => copyJson(entry, ancestors))
        : Object.fromEntries(
              Object.entries(value).map(([key, entry]) => [key, copyJson(entry, ancestors)])
          );
    ancestors.delete(value);
    return Object.freeze(result);
}

export function resolveAtomAttrsUpdate(
    attrs: Readonly<Record<string, unknown>>,
    update: AtomAttrsUpdate
): Record<string, unknown> {
    let current = copyJson(attrs) as Record<string, unknown>;
    let patch: Record<string, unknown> = {};
    for (const entry of Array.isArray(update) ? update : [update]) {
        const value: unknown = typeof entry === 'function' ? entry(current) : entry;
        if (value == null || typeof value !== 'object' || Array.isArray(value)) {
            throw new AtomUpdateAttrsError(
                'not-applicable',
                'Atom updates must return an attribute object.'
            );
        }
        const partial = copyJson(value) as Record<string, unknown>;
        patch = { ...patch, ...partial };
        current = Object.freeze({ ...current, ...partial });
    }
    return patch;
}
