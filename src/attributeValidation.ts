import type { AttrSpec } from './schemas';

function jsonType(value: unknown): string {
    return Array.isArray(value) ? 'array' : value === null ? 'null' : typeof value;
}

function validateJson(value: unknown, ancestors = new Set<object>()): void {
    if (value === null || typeof value === 'string' || typeof value === 'boolean') return;
    if (typeof value === 'number' && Number.isFinite(value)) return;
    if (typeof value !== 'object' || ancestors.has(value) || ancestors.size > 128)
        throw new Error('Attribute values must be finite, acyclic JSON.');
    if (
        !Array.isArray(value) &&
        Object.getPrototypeOf(value) !== Object.prototype &&
        Object.getPrototypeOf(value) !== null
    )
        throw new Error('Attribute values must be plain JSON objects.');
    ancestors.add(value);
    const entries = Array.isArray(value) ? value : Object.values(value);
    for (let index = 0; index < entries.length; index += 1) validateJson(entries[index], ancestors);
    ancestors.delete(value);
}

function equal(left: unknown, right: unknown): boolean {
    if (left === right) return true;
    if (left == null || right == null || typeof left !== 'object' || typeof right !== 'object')
        return false;
    if (Array.isArray(left) !== Array.isArray(right)) return false;
    const a = Object.keys(left),
        b = Object.keys(right);
    return (
        a.length === b.length &&
        a.every(
            (key) =>
                Object.prototype.hasOwnProperty.call(right, key) &&
                equal(
                    (left as Record<string, unknown>)[key],
                    (right as Record<string, unknown>)[key]
                )
        )
    );
}

export function validateAttribute(value: unknown, spec: AttrSpec, name: string): void {
    validateJson(value);
    const type = jsonType(value);
    if ((spec.type != null && type !== spec.type) || (type === 'number' && !Number.isFinite(value)))
        throw new Error(`invalid attribute '${name}': expected ${spec.type ?? 'finite number'}`);
    if (spec.enum != null && !spec.enum.some((entry) => equal(entry, value)))
        throw new Error(`invalid attribute '${name}': outside enum`);
    const size =
        typeof value === 'number'
            ? value
            : typeof value === 'string'
              ? [...value].length
              : Array.isArray(value)
                ? value.length
                : undefined;
    if (
        (spec.min != null && (size == null || size < spec.min)) ||
        (spec.max != null && (size == null || size > spec.max))
    )
        throw new Error(`invalid attribute '${name}': outside bounds`);
}

export function validateAttributeSpec(spec: AttrSpec, name: string): void {
    if (spec == null || typeof spec !== 'object' || Array.isArray(spec))
        throw new Error(`invalid attribute '${name}' declaration`);
    if (Object.keys(spec).some((key) => !['default', 'type', 'enum', 'min', 'max'].includes(key)))
        throw new Error(`invalid attribute '${name}' declaration field`);
    if (
        spec.type !== undefined &&
        !['string', 'number', 'boolean', 'object', 'array'].includes(spec.type)
    )
        throw new Error(`invalid attribute '${name}' type`);
    if (spec.min !== undefined || spec.max !== undefined) {
        if (
            !['number', 'string', 'array'].includes(spec.type ?? '') ||
            (spec.min !== undefined && !Number.isFinite(spec.min)) ||
            (spec.max !== undefined && !Number.isFinite(spec.max)) ||
            (spec.min !== undefined && spec.max !== undefined && spec.min > spec.max)
        )
            throw new Error(`invalid attribute '${name}' bounds`);
        if (
            spec.type !== 'number' &&
            [spec.min, spec.max].some(
                (bound) => bound !== undefined && (!Number.isInteger(bound) || bound < 0)
            )
        )
            throw new Error(`invalid attribute '${name}' length bounds`);
    }
    if (spec.enum !== undefined) {
        if (!Array.isArray(spec.enum) || spec.enum.length === 0)
            throw new Error(`invalid attribute '${name}' enum`);
        if (new Set(spec.enum.map(jsonType)).size !== 1)
            throw new Error(`invalid attribute '${name}': enum values must share one JSON type`);
        for (const entry of spec.enum) validateAttribute(entry, { ...spec, enum: undefined }, name);
    }
    if (spec.default !== undefined) validateAttribute(spec.default, spec, name);
}

export function validateAttributes(
    attrs: Record<string, unknown>,
    specs: Record<string, AttrSpec>
): void {
    for (const name of Object.keys(attrs))
        if (!Object.prototype.hasOwnProperty.call(specs, name))
            throw new Error(`undeclared attribute '${name}'`);
    for (const [name, spec] of Object.entries(specs)) {
        const value = Object.prototype.hasOwnProperty.call(attrs, name)
            ? attrs[name]
            : spec.default;
        if (value === undefined) throw new Error(`required attribute '${name}' missing`);
        validateAttribute(value, spec, name);
    }
}
