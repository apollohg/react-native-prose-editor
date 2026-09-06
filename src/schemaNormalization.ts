import { validateAttributeSpec } from './attributeValidation';
import {
    ATOM_HTML_DENIED_ATTRS,
    ATOM_HTML_DENIED_TAGS,
    ATOM_HTML_IDENTIFIER,
    RESERVED_WIRE_NODE_TYPES,
} from './atomPolicy';
import {
    type NodeSpec,
    type AttrSpec,
    type NodeJSONProjection,
    type NodeHtmlRules,
    type SchemaDefinition,
    type MarkSpec,
    ALLOWED_MARK_HTML_TAGS,
} from './schemaDefinition';
import { NativeEditorBoundaryError } from './NativeEditorBoundaryError';

export function schemaBoundaryError(limit: number, actual: number): NativeEditorBoundaryError {
    return new NativeEditorBoundaryError(
        'SCHEMA_INVALID',
        `schema work exceeds configured limit ${limit}`,
        limit,
        actual
    );
}

/** Mirror native's invalid-schema fallback before constructing an empty doc. */
export function utf8ByteLengthUpTo(value: string, maximum: number): number {
    let bytes = 0;
    for (const char of value) {
        const codePoint = char.codePointAt(0) ?? 0;
        bytes += codePoint <= 0x7f ? 1 : codePoint <= 0x7ff ? 2 : codePoint <= 0xffff ? 3 : 4;
        if (bytes > maximum) return bytes;
    }
    return bytes;
}

export function forEachGroupToken(
    group: string,
    visit: (token: string) => void,
    consumeCharacter?: () => boolean,
    consumeToken?: () => boolean
): boolean {
    let tokenStart = -1;
    for (let index = 0; index <= group.length; index += 1) {
        if (index < group.length && consumeCharacter && !consumeCharacter()) return false;
        const isBoundary = index === group.length || /\s/.test(group[index]);
        if (!isBoundary && tokenStart < 0) tokenStart = index;
        if (isBoundary && tokenStart >= 0) {
            if (consumeToken && !consumeToken()) return false;
            visit(group.slice(tokenStart, index));
            tokenStart = -1;
        }
    }
    return true;
}

export interface SchemaWorkBudget {
    readonly limit: number;
    work: number;
    exhausted: boolean;
}

export interface AdmittedSchemaCollections {
    readonly groupsByNode: ReadonlyMap<NodeSpec, readonly string[]>;
    readonly attrsByNode: ReadonlyMap<NodeSpec, ReadonlyArray<[string, AttrSpec]>>;
}

export function consumeSchemaWork(budget: SchemaWorkBudget): boolean {
    return consumeSchemaWorkAmount(budget, 1);
}

export function consumeSchemaWorkAmount(budget: SchemaWorkBudget, amount: number): boolean {
    if (amount > budget.limit - budget.work) {
        budget.exhausted = true;
        return false;
    }
    budget.work += amount;
    return true;
}

export function consumeSchemaStringWork(budget: SchemaWorkBudget, value: string): boolean {
    return consumeSchemaWorkAmount(budget, utf8ByteLengthUpTo(value, budget.limit) + 1);
}

export function collectOwnAttrs(
    value: unknown,
    budget: SchemaWorkBudget
): Array<[string, AttrSpec]> | null {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return [];
    const attrs: Array<[string, AttrSpec]> = [];
    for (const name in value) {
        if (!Object.prototype.hasOwnProperty.call(value, name)) continue;
        if (!consumeSchemaWork(budget)) return attrs;
        attrs.push([name, (value as Record<string, AttrSpec>)[name]]);
    }
    return attrs;
}

export function isSafeHtmlTag(tag: string): boolean {
    if (tag.length === 0 || tag[0] < 'a' || tag[0] > 'z') return false;
    for (let index = 1; index < tag.length; index += 1) {
        const char = tag[index];
        if (!((char >= 'a' && char <= 'z') || (char >= '0' && char <= '9') || char === '-')) {
            return false;
        }
    }
    return true;
}

export function isSafeHtmlAttr(name: string): boolean {
    if (name.length === 0) return false;
    const isAlpha = (char: string): boolean =>
        (char >= 'A' && char <= 'Z') || (char >= 'a' && char <= 'z');
    if (!(isAlpha(name[0]) || name[0] === '_' || name[0] === ':')) return false;
    for (let index = 1; index < name.length; index += 1) {
        const char = name[index];
        if (
            !(
                isAlpha(char) ||
                (char >= '0' && char <= '9') ||
                char === '_' ||
                char === ':' ||
                char === '.' ||
                char === '-'
            )
        ) {
            return false;
        }
    }
    return true;
}

export function normalizeAttrs(value: unknown): Record<string, AttrSpec> {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(
        Object.entries(value).map(([name, rawSpec]) => {
            if (rawSpec == null || typeof rawSpec !== 'object' || Array.isArray(rawSpec)) {
                return [name, {}];
            }
            validateAttributeSpec(rawSpec as AttrSpec, name);
            return [
                name,
                Object.fromEntries(
                    Object.entries(rawSpec).filter(([, entry]) => entry !== undefined)
                ),
            ];
        })
    );
}

export function normalizeNodeJSONProjection(value: unknown): NodeJSONProjection | null | undefined {
    if (value === undefined) return undefined;
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return null;
    const raw = value as Record<string, unknown>;
    if (typeof raw.type !== 'string' || raw.type.length === 0) return null;
    if (raw.attrs === undefined) return { type: raw.type };
    if (raw.attrs == null || typeof raw.attrs !== 'object' || Array.isArray(raw.attrs)) return null;
    const attrs: Record<string, unknown> = {};
    for (const [name, attrValue] of Object.entries(raw.attrs)) {
        if (!isSafeHtmlAttr(name)) return null;
        if (
            attrValue !== null &&
            typeof attrValue !== 'string' &&
            typeof attrValue !== 'boolean' &&
            !(typeof attrValue === 'number' && Number.isFinite(attrValue))
        ) {
            return null;
        }
        attrs[name] = attrValue;
    }
    return Object.keys(attrs).length === 0 ? { type: raw.type } : { type: raw.type, attrs };
}

export function isSafeAtomHtmlTag(tag: string): boolean {
    return ATOM_HTML_IDENTIFIER.test(tag) && !ATOM_HTML_DENIED_TAGS.has(tag);
}

export function isSafeAtomHtmlAttr(name: string): boolean {
    return (
        ATOM_HTML_IDENTIFIER.test(name) &&
        !name.startsWith('on') &&
        !ATOM_HTML_DENIED_ATTRS.has(name)
    );
}

export function normalizeNodeHtmlRules(value: unknown): NodeHtmlRules | null | undefined {
    if (value === undefined) return undefined;
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return null;
    const raw = value as Record<string, unknown>;
    if (
        Object.keys(raw).some(
            (key) => key !== 'tag' && key !== 'staticAttrs' && key !== 'attrMap'
        ) ||
        typeof raw.tag !== 'string' ||
        !isSafeAtomHtmlTag(raw.tag)
    ) {
        return null;
    }
    const normalizeMap = (
        candidate: unknown,
        validateName: (name: string) => boolean
    ): Record<string, string> | null | undefined => {
        if (candidate === undefined) return undefined;
        if (candidate == null || typeof candidate !== 'object' || Array.isArray(candidate)) {
            return null;
        }
        const normalized: Record<string, string> = {};
        for (const [name, mapped] of Object.entries(candidate)) {
            if (!validateName(name) || typeof mapped !== 'string') return null;
            normalized[name] = mapped;
        }
        return normalized;
    };
    const staticAttrs = normalizeMap(raw.staticAttrs, isSafeAtomHtmlAttr);
    const attrMap = normalizeMap(raw.attrMap, isSafeHtmlAttr);
    if (
        staticAttrs === null ||
        attrMap === null ||
        (attrMap != null && Object.values(attrMap).some((name) => !isSafeAtomHtmlAttr(name)))
    ) {
        return null;
    }
    return {
        tag: raw.tag,
        ...(staticAttrs == null ? {} : { staticAttrs }),
        ...(attrMap == null ? {} : { attrMap }),
    };
}

export function projectionAttrsOverlap(
    left: Record<string, unknown>,
    right: Record<string, unknown>
): boolean {
    return Object.entries(left).every(
        ([name, value]) =>
            !Object.prototype.hasOwnProperty.call(right, name) || right[name] === value
    );
}

export function legacyHeadingProjectionName(projection: NodeJSONProjection): string | undefined {
    if (projection.type !== 'heading') return undefined;
    const value = projection.attrs?.level;
    let level: number | undefined;
    if (typeof value === 'number' && Number.isInteger(value)) {
        level = value;
    } else if (typeof value === 'string' && value.length <= 3 && /^\+?\d+$/.test(value)) {
        level = Number(value);
    }
    return level != null && level >= 1 && level <= 6 ? `h${level}` : undefined;
}

export function normalizeSchemaDefinition(
    schema: SchemaDefinition,
    budget?: SchemaWorkBudget
): SchemaDefinition | null {
    const normalizeRole = (role: unknown): string => {
        switch (role) {
            case 'doc':
            case 'textBlock':
            case 'list':
            case 'listItem':
            case 'text':
            case 'hardBreak':
            case 'inline':
                return role;
            default:
                return 'block';
        }
    };
    const nodes: NodeSpec[] = [];
    for (let nodeIndex = 0; nodeIndex < schema.nodes.length; nodeIndex += 1) {
        const rawNode = schema.nodes[nodeIndex];
        if (rawNode == null || typeof rawNode !== 'object' || typeof rawNode.name !== 'string') {
            return null;
        }
        const raw = rawNode as unknown as Record<string, unknown>;
        const htmlTag = typeof raw.htmlTag === 'string' ? raw.htmlTag : undefined;
        const html = normalizeNodeHtmlRules(raw.html);
        const attrs = normalizeAttrs(raw.attrs);
        const json = normalizeNodeJSONProjection(raw.json);
        if (
            (htmlTag != null && !isSafeHtmlTag(htmlTag)) ||
            html === null ||
            json === null ||
            Object.keys(attrs).some((name) => !isSafeHtmlAttr(name))
        ) {
            return null;
        }
        nodes.push({
            name: rawNode.name,
            content: typeof raw.content === 'string' ? raw.content : '',
            ...(typeof raw.group === 'string' ? { group: raw.group } : {}),
            ...(Object.keys(attrs).length > 0 ? { attrs } : {}),
            role: normalizeRole(raw.role),
            ...(htmlTag == null ? {} : { htmlTag }),
            ...(html == null ? {} : { html }),
            ...(json == null ? {} : { json }),
            isVoid: typeof raw.isVoid === 'boolean' ? raw.isVoid : false,
            ...(typeof raw.deletableOnBackspace === 'boolean'
                ? { deletableOnBackspace: raw.deletableOnBackspace }
                : {}),
            ...(typeof raw.allowUndeclaredAttrs === 'boolean'
                ? { allowUndeclaredAttrs: raw.allowUndeclaredAttrs }
                : {}),
        });
    }
    const nativeNodeNames = new Set(nodes.map((node) => node.name));
    const projectedNodes = nodes.filter((node) => node.json != null);
    for (let index = 0; index < projectedNodes.length; index += 1) {
        const node = projectedNodes[index];
        const projection = node.json as NodeJSONProjection;
        const legacyHeadingName = legacyHeadingProjectionName(projection);
        if (
            nativeNodeNames.has(projection.type) ||
            RESERVED_WIRE_NODE_TYPES.has(projection.type) ||
            (legacyHeadingName != null &&
                legacyHeadingName !== node.name &&
                nativeNodeNames.has(legacyHeadingName)) ||
            Object.keys(projection.attrs ?? {}).some((name) =>
                Object.prototype.hasOwnProperty.call(node.attrs ?? {}, name)
            )
        ) {
            return null;
        }
        for (let previousIndex = 0; previousIndex < index; previousIndex += 1) {
            if (budget != null && !consumeSchemaWork(budget)) {
                throw schemaBoundaryError(budget.limit, budget.limit + 1);
            }
            const previous = projectedNodes[previousIndex];
            if (
                previous.json?.type === projection.type &&
                projectionAttrsOverlap(projection.attrs ?? {}, previous.json.attrs ?? {})
            ) {
                return null;
            }
        }
    }

    const marks: MarkSpec[] = [];
    for (let markIndex = 0; markIndex < schema.marks.length; markIndex += 1) {
        const rawMark = schema.marks[markIndex];
        if (rawMark == null || typeof rawMark !== 'object' || typeof rawMark.name !== 'string') {
            return null;
        }
        const raw = rawMark as unknown as Record<string, unknown>;
        const htmlTag = typeof raw.htmlTag === 'string' ? raw.htmlTag.toLowerCase() : undefined;
        const attrs = normalizeAttrs(raw.attrs);
        if (
            (htmlTag != null && !ALLOWED_MARK_HTML_TAGS.has(htmlTag)) ||
            Object.keys(attrs).some((name) => !isSafeHtmlAttr(name))
        ) {
            return null;
        }
        marks.push({
            name: rawMark.name,
            ...(Object.keys(attrs).length > 0 ? { attrs } : {}),
            ...(typeof raw.excludes === 'string' ? { excludes: raw.excludes } : {}),
            ...(htmlTag == null ? {} : { htmlTag: htmlTag as MarkSpec['htmlTag'] }),
            ...(typeof raw.allowUndeclaredAttrs === 'boolean'
                ? { allowUndeclaredAttrs: raw.allowUndeclaredAttrs }
                : {}),
        });
    }
    return { nodes, marks };
}
