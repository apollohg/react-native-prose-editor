import { processColor } from 'react-native';
import type {
    EditorStyleMap,
    NormalizedEditorStyle,
    NormalizedEditorTheme,
} from './EditorStyleSheetTypes';

const typography = [
    'fontFamily',
    'fontSize',
    'fontWeight',
    'fontStyle',
    'color',
    'lineHeight',
    'letterSpacing',
    'textDecorationLine',
    'textDecorationColor',
    'textDecorationStyle',
];
const sides = ['Top', 'Right', 'Bottom', 'Left'];
const corners = ['TopLeft', 'TopRight', 'BottomRight', 'BottomLeft'];
const border = [
    'borderWidth',
    'borderColor',
    'borderStyle',
    'borderRadius',
    ...sides.flatMap((side) => [`border${side}Width`, `border${side}Color`]),
    ...corners.map((corner) => `border${corner}Radius`),
];
const spacing = (prefix: string) => [
    prefix,
    `${prefix}Horizontal`,
    `${prefix}Vertical`,
    ...sides.map((side) => `${prefix}${side}`),
];
const surface = ['backgroundColor', ...border, ...spacing('padding')];
const box = [...surface, ...spacing('margin')];
const block = [...typography, ...box, 'textAlign'];
const inline = [...typography, 'backgroundColor'];
const checkbox = [...border, 'backgroundColor', 'checkColor'];
const fields: Record<keyof EditorStyleMap, readonly string[]> = {
    content: surface,
    text: typography,
    paragraph: block,
    h1: block,
    h2: block,
    h3: block,
    h4: block,
    h5: block,
    h6: block,
    blockquote: block,
    codeBlock: block,
    bulletList: [...block, 'indent', 'baseIndentMultiplier'],
    orderedList: [...block, 'indent', 'baseIndentMultiplier'],
    taskList: [...block, 'indent', 'baseIndentMultiplier'],
    listItem: block,
    taskItem: block,
    listMarker: ['color', 'scale', 'gap', 'ordered'],
    taskCheckbox: [...checkbox, 'size', 'gap', 'checked'],
    horizontalRule: [...border, ...spacing('margin'), 'backgroundColor', 'height'],
    image: [...box, 'resizeMode'],
    link: inline,
    inlineCode: inline,
    bold: inline,
    italic: inline,
    underline: inline,
    strike: inline,
    mention: [...inline, ...border],
    placeholder: typography,
};
const enums: Record<string, readonly string[]> = {
    fontStyle: ['normal', 'italic'],
    fontWeight: ['normal', 'bold', '100', '200', '300', '400', '500', '600', '700', '800', '900'],
    textAlign: ['auto', 'left', 'right', 'center', 'justify'],
    textDecorationLine: ['none', 'underline', 'line-through', 'underline line-through'],
    textDecorationStyle: ['solid', 'double', 'dotted', 'dashed'],
    borderStyle: ['solid', 'dashed', 'dotted'],
    resizeMode: ['contain', 'cover', 'stretch'],
};

function invalid(path: string, reason: string): never {
    throw new TypeError(`Invalid editor style ${path}: ${reason}`);
}

function record(value: unknown): value is Record<string, unknown> {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return false;
    const prototype = Object.getPrototypeOf(value);
    return prototype === Object.prototype || prototype === null;
}

export function normalizeEditorColor(value: unknown, path: string): string {
    if (typeof value !== 'string') return invalid(path, 'expected a color string');
    const argb = processColor(value);
    if (typeof argb !== 'number') return invalid(path, 'unrecognized color');
    const rgba = ((argb << 8) | (argb >>> 24)) >>> 0;
    return `#${rgba.toString(16).padStart(8, '0')}`;
}

function orderedMarker(value: unknown, path: string): Record<string, unknown> {
    if (!record(value)) return invalid(path, 'expected an ordered marker object');
    const result: Record<string, unknown> = {};
    for (const [key, entry] of Object.entries(value)) {
        if (key !== 'schemes' && key !== 'suffix')
            invalid(`${path}.${key}`, 'unsupported property');
        if (entry === undefined) continue;
        if (key === 'suffix') {
            if (entry !== '.' && entry !== ')') invalid(`${path}.${key}`, 'expected . or )');
            result[key] = entry;
        } else {
            const schemes = ['decimal', 'lowerAlpha', 'upperAlpha', 'lowerRoman', 'upperRoman'];
            if (
                !Array.isArray(entry) ||
                entry.length === 0 ||
                !entry.every((item) => typeof item === 'string' && schemes.includes(item))
            ) {
                invalid(`${path}.${key}`, 'expected a nonempty array of numbering schemes');
            }
            result[key] = [...entry];
        }
    }
    return result;
}

function property(key: string, value: unknown, path: string): unknown {
    if (value === undefined) return undefined;
    if (key === 'ordered') return orderedMarker(value, path);
    if (key === 'checked') return normalizeStyle(value, checkbox, path);
    if (key === 'color' || key.endsWith('Color')) return normalizeEditorColor(value, path);
    if (key === 'fontFamily') {
        if (typeof value !== 'string' || value.length === 0)
            invalid(path, 'expected a nonempty font family');
        return value;
    }
    if (enums[key]) {
        const candidate = key === 'fontWeight' && typeof value === 'number' ? String(value) : value;
        if (typeof candidate !== 'string' || !enums[key].includes(candidate))
            invalid(path, 'unsupported value');
        return candidate;
    }
    if (typeof value !== 'number' || !Number.isFinite(value))
        invalid(path, 'expected a finite number');
    if (['fontSize', 'lineHeight', 'size', 'scale'].includes(key) && value <= 0)
        invalid(path, 'expected a positive number');
    if (!key.startsWith('margin') && key !== 'letterSpacing' && value < 0)
        invalid(path, 'expected a nonnegative number');
    return value;
}

function normalizeStyle(
    value: unknown,
    allowed: readonly string[],
    path: string
): Record<string, unknown> {
    const raw: Record<string, unknown> = {};
    const ancestors = new Set<object>();
    function append(entry: unknown, depth: number): void {
        if (entry === false || entry == null) return;
        if (depth > 64) invalid(path, 'style arrays are nested too deeply');
        if (Array.isArray(entry)) {
            if (ancestors.has(entry)) invalid(path, 'cyclic style array');
            ancestors.add(entry);
            entry.forEach((item) => append(item, depth + 1));
            ancestors.delete(entry);
            return;
        }
        if (!record(entry)) invalid(path, 'expected a style object or array');
        for (const [key, field] of Object.entries(entry)) {
            if (!allowed.includes(key)) invalid(`${path}.${key}`, 'unsupported property');
            raw[key] = property(key, field, `${path}.${key}`);
        }
    }
    append(value, 0);
    const result = Object.fromEntries(
        Object.entries(raw).filter(([, value]) => value !== undefined)
    );
    for (const prefix of ['margin', 'padding']) {
        for (const side of sides) {
            const axis = side === 'Top' || side === 'Bottom' ? 'Vertical' : 'Horizontal';
            const resolved = raw[`${prefix}${side}`] ?? raw[`${prefix}${axis}`] ?? raw[prefix];
            if (resolved !== undefined) result[`${prefix}${side}`] = resolved;
        }
        delete result[prefix];
        delete result[`${prefix}Horizontal`];
        delete result[`${prefix}Vertical`];
    }
    for (const suffix of ['Width', 'Color']) {
        for (const side of sides) {
            const resolved = raw[`border${side}${suffix}`] ?? raw[`border${suffix}`];
            if (resolved !== undefined) result[`border${side}${suffix}`] = resolved;
        }
        delete result[`border${suffix}`];
    }
    for (const corner of corners) {
        const resolved = raw[`border${corner}Radius`] ?? raw.borderRadius;
        if (resolved !== undefined) result[`border${corner}Radius`] = resolved;
    }
    delete result.borderRadius;
    return result;
}

export function normalizeEditorTheme(theme: unknown): NormalizedEditorTheme {
    if (!record(theme)) invalid('theme', 'expected a named style map');
    const styles: Partial<Record<keyof EditorStyleMap, NormalizedEditorStyle>> = {};
    const result: NormalizedEditorTheme = { version: 1 };
    for (const [key, value] of Object.entries(theme)) {
        if (key === 'toolbar') {
            if (value !== undefined) {
                if (!record(value)) invalid('toolbar', 'expected toolbar configuration');
                result.toolbar = { ...value };
            }
            continue;
        }
        if (!Object.prototype.hasOwnProperty.call(fields, key)) invalid(key, 'unsupported element');
        const style = normalizeStyle(value, fields[key as keyof EditorStyleMap], key);
        if (Object.keys(style).length) styles[key as keyof EditorStyleMap] = style;
    }
    if (Object.keys(styles).length) result.styles = styles;
    return result;
}
