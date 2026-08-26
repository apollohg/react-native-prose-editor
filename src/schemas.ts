import type { DocumentJSON } from './NativeEditorBridge';
import { NativeEditorBoundaryError } from './NativeEditorBoundaryError';
import {
    resolveEditorResourceLimits,
    type EditorResourceLimits,
    type ResolvedEditorResourceLimits,
} from './ResourceLimits';
import {
    CONTENT_EXPRESSION_MAX_DEPTH,
    DEFAULT_CONTENT_MAX_NODES,
    contentExpressionSymbols,
    minimalContentMatch,
} from './contentExpression';

/** Declaration of one node or mark attribute. */
export interface AttrSpec {
    /** Value used when the attribute is absent. Omit to make the attribute required. */
    default?: unknown;
}

/** Static DOM output supported by the native HTML serializer. */
export type DOMOutputSpec = readonly [tag: string] | readonly [tag: string, content: 0];

/** Declarative DOM rule supported by the native HTML parser. */
export interface ParseDOMRule {
    tag: string;
    attrs?: Record<string, unknown>;
}

/** Select a DOM output from one declared node attribute. */
export interface AttributeDOMOutputSpec {
    switchOn: string;
    cases: Readonly<Record<string, DOMOutputSpec>>;
}

/** JSON representation projected from one native node variant. */
export interface NodeJSONProjection {
    type: string;
    attrs?: Record<string, unknown>;
}

/** Declaration of one node type in a {@link SchemaDefinition}. */
export interface NodeSpec {
    /** Native node type name. `json` may expose a different public document type. */
    name: string;
    /** ProseMirror-style content expression, e.g. `'block+'`, `'inline*'`, or `''` for a leaf. */
    content: string;
    /** Content group this node belongs to, e.g. `'block'` or `'inline'`. */
    group?: string;
    /** Attributes this node declares. Undeclared attributes are filtered out on
     *  ingestion unless `allowUndeclaredAttrs` is set. */
    attrs?: Record<string, AttrSpec>;
    /**
     * How the engine treats this node: `'doc'`, `'textBlock'`, `'list'`,
     * `'listItem'`, `'text'`, `'hardBreak'`, `'inline'`, or `'block'`. Any
     * unrecognized value is treated as `'block'`. Exactly one node must use
     * `'doc'`. An ordered list is a `'list'` node whose name contains
     * `ordered`.
     */
    role: string;
    /** Tag used when serializing this node to HTML. */
    htmlTag?: string;
    /** Whether the node holds no content of its own (an image or rule, say). */
    isVoid?: boolean;
    /**
     * Opt-in escape hatch: when `true`, JSON ingestion (`set_json` /
     * `insert_content_json`) admits attrs on this node that are not declared
     * in `attrs`, instead of filtering them out. Default `false`. Intended
     * for node types with an intentional pass-through-metadata contract
     * (e.g. the mention node — see `mentionNodeSpec()` in addons.ts).
     */
    allowUndeclaredAttrs?: boolean;
    /** Public JSON representation when it differs from the native node name. */
    json?: NodeJSONProjection;
}

/** Declaration of one mark type in a {@link SchemaDefinition}. */
export interface MarkSpec {
    /** Mark type name, as it appears in document JSON. */
    name: string;
    /** Attributes this mark declares. */
    attrs?: Record<string, AttrSpec>;
    /** Mark names that cannot coexist with this one on the same text range. */
    excludes?: string;
    /**
     * Serializer tag for a custom mark. Native accepts only the inert allowlist:
     * span, strong, em, u, s, code, a, sub, sup, mark.
     */
    htmlTag?: 'span' | 'strong' | 'em' | 'u' | 's' | 'code' | 'a' | 'sub' | 'sup' | 'mark';
    /**
     * Opt-in escape hatch: when `true`, JSON ingestion (`set_json` /
     * `insert_content_json`) admits attrs on this mark that are not declared
     * in `attrs`, instead of filtering them out. Default `false`. Mirrors
     * `NodeSpec.allowUndeclaredAttrs` for mark types with an intentional
     * pass-through-metadata contract.
     */
    allowUndeclaredAttrs?: boolean;
}

/**
 * The node and mark types a document may contain. Fixed when the document
 * handle is created (`NativeEditorV2CreateConfig.schema`), or per render for
 * `NativeProseViewer`. Start from {@link defaultSchema},
 * {@link prosemirrorSchema}, or {@link tiptapCompatibleSchema} rather than
 * assembling one from scratch.
 */
export interface SchemaDefinition {
    /** Node types. Exactly one must have `role: 'doc'`. */
    nodes: NodeSpec[];
    /** Mark types. */
    marks: MarkSpec[];
}

/** Keyed node declaration accepted by {@link defineSchema}. */
export interface SchemaNodeSpec {
    content?: string;
    group?: string;
    attrs?: Record<string, AttrSpec>;
    /** Native semantic role. Common `doc`, `paragraph`, and `text` shapes are inferred. */
    role?: NodeSpec['role'] | 'heading';
    parseDOM?: readonly ParseDOMRule[];
    toDOM?: DOMOutputSpec | AttributeDOMOutputSpec;
    isVoid?: boolean;
    allowUndeclaredAttrs?: boolean;
}

/** Keyed mark declaration accepted by {@link defineSchema}. */
export interface SchemaMarkSpec {
    attrs?: Record<string, AttrSpec>;
    excludes?: string;
    parseDOM?: readonly ParseDOMRule[];
    toDOM?: DOMOutputSpec;
    allowUndeclaredAttrs?: boolean;
}

/** ProseMirror-shaped authoring schema compiled for the native engine. */
export interface SchemaSpec {
    nodes: Readonly<Record<string, SchemaNodeSpec>>;
    marks?: Readonly<Record<string, SchemaMarkSpec>>;
}

const RESERVED_WIRE_NODE_TYPES = new Set(['__opaque', '__opaque_json', '__skip']);
const ALLOWED_MARK_HTML_TAGS = new Set([
    'span',
    'strong',
    'em',
    'u',
    's',
    'code',
    'a',
    'sub',
    'sup',
    'mark',
]);

function outputTag(spec: DOMOutputSpec): string {
    return spec[0];
}

function appendGroup(group: string | undefined, name: string): string {
    const groups = group?.split(/\s+/).filter(Boolean) ?? [];
    if (!groups.includes(name)) groups.push(name);
    return groups.join(' ');
}

function schemaNodeRole(name: string, node: SchemaNodeSpec): string {
    if (node.role != null) return node.role === 'heading' ? 'textBlock' : node.role;
    if (name === 'doc') return 'doc';
    if (name === 'text') return 'text';
    if (node.content === 'inline*' && node.group?.split(/\s+/).includes('block')) {
        return 'textBlock';
    }
    if (node.group?.split(/\s+/).includes('inline')) return 'inline';
    return 'block';
}

function caseAttributeValue(
    key: string,
    attribute: string,
    tag: string,
    parseDOM: readonly ParseDOMRule[] | undefined,
    defaultValue: unknown
): unknown {
    const rule = parseDOM?.find(
        (candidate) =>
            candidate.tag === tag &&
            candidate.attrs != null &&
            String(candidate.attrs[attribute]) === key
    );
    if (rule?.attrs && Object.prototype.hasOwnProperty.call(rule.attrs, attribute)) {
        return rule.attrs[attribute];
    }
    if (typeof defaultValue === 'number') return Number(key);
    if (typeof defaultValue === 'boolean') return key === 'true';
    return key;
}

function validateAttributeDOMRules(
    name: string,
    switched: AttributeDOMOutputSpec,
    parseDOM: readonly ParseDOMRule[] | undefined,
    defaultValue: unknown
): void {
    const cases = Object.entries(switched.cases);
    let discriminatorType =
        typeof defaultValue === 'string' ||
        typeof defaultValue === 'number' ||
        typeof defaultValue === 'boolean'
            ? typeof defaultValue
            : undefined;
    if (discriminatorType == null && parseDOM != null) {
        const parsedTypes = parseDOM.map((rule) => {
            const value = rule.attrs?.[switched.switchOn];
            return typeof value === 'string' ||
                typeof value === 'number' ||
                typeof value === 'boolean'
                ? typeof value
                : undefined;
        });
        const firstType = parsedTypes[0];
        if (firstType != null && parsedTypes.every((type) => type === firstType)) {
            discriminatorType = firstType;
        }
    }
    if (discriminatorType == null) {
        throw new Error(
            `node '${name}' DOM discriminator '${switched.switchOn}' must have a scalar type`
        );
    }
    if (parseDOM == null) {
        const validCases = cases.every(([caseKey]) => {
            if (discriminatorType === 'number') {
                return /^-?(?:0|[1-9]\d*)(?:\.\d+)?$/.test(caseKey);
            }
            return discriminatorType !== 'boolean' || caseKey === 'true' || caseKey === 'false';
        });
        if (!validCases) {
            throw new Error(
                `node '${name}' DOM discriminator '${switched.switchOn}' must use ${discriminatorType} values`
            );
        }
        return;
    }
    const matchesCase = ([caseKey, output]: [string, DOMOutputSpec]) =>
        parseDOM.filter(
            (rule) =>
                rule.tag === outputTag(output) &&
                rule.attrs != null &&
                Object.prototype.hasOwnProperty.call(rule.attrs, switched.switchOn) &&
                String(rule.attrs[switched.switchOn]) === caseKey
        ).length === 1;
    if (parseDOM.length !== cases.length || !cases.every(matchesCase)) {
        throw new Error(
            `node '${name}' DOM parse rules must map one-to-one with '${switched.switchOn}' output cases`
        );
    }
    if (
        discriminatorType != null &&
        parseDOM.some(
            (rule) =>
                rule.attrs != null && typeof rule.attrs[switched.switchOn] !== discriminatorType
        )
    ) {
        throw new Error(
            `node '${name}' DOM discriminator '${switched.switchOn}' must use ${discriminatorType} values`
        );
    }
}

function staticDOMTag(
    kind: 'node' | 'mark',
    name: string,
    parseDOM: readonly ParseDOMRule[] | undefined,
    toDOM: DOMOutputSpec | undefined
): string | undefined {
    if ((parseDOM?.length ?? 0) > 1) {
        throw new Error(`${kind} '${name}' has multiple static DOM parse rules`);
    }
    const parsed = parseDOM?.[0]?.tag;
    const serialized = toDOM == null ? undefined : outputTag(toDOM);
    if (parsed != null && serialized != null && parsed !== serialized) {
        throw new Error(`${kind} '${name}' parses '${parsed}' but serializes '${serialized}'`);
    }
    return serialized ?? parsed;
}

/** Compile a keyed, ProseMirror-shaped schema into the serializable native form. */
export function defineSchema(spec: SchemaSpec): SchemaDefinition {
    const nodes: NodeSpec[] = [];
    const names = new Set<string>();
    for (const [name, node] of Object.entries(spec.nodes)) {
        const common = {
            content: node.content ?? '',
            ...(node.group == null ? {} : { group: node.group }),
            ...(node.attrs == null ? {} : { attrs: node.attrs }),
            role: schemaNodeRole(name, node),
            ...(node.isVoid == null ? {} : { isVoid: node.isVoid }),
            ...(node.allowUndeclaredAttrs == null
                ? {}
                : { allowUndeclaredAttrs: node.allowUndeclaredAttrs }),
        };
        if (node.toDOM != null && !Array.isArray(node.toDOM)) {
            const switched = node.toDOM as AttributeDOMOutputSpec;
            if (RESERVED_WIRE_NODE_TYPES.has(name)) {
                throw new Error(`node '${name}' uses a reserved wire projection type`);
            }
            if (
                node.attrs == null ||
                !Object.prototype.hasOwnProperty.call(node.attrs, switched.switchOn)
            ) {
                throw new Error(
                    `node '${name}' switches on undeclared attribute '${switched.switchOn}'`
                );
            }
            if (Object.keys(switched.cases).length === 0) {
                throw new Error(`node '${name}' has no DOM output cases`);
            }
            const discriminatorDefault = node.attrs[switched.switchOn]?.default;
            validateAttributeDOMRules(name, switched, node.parseDOM, discriminatorDefault);
            const { attrs: _attrs, ...variantCommon } = common;
            for (const [caseKey, output] of Object.entries(switched.cases)) {
                const tag = outputTag(output);
                if (names.has(tag))
                    throw new Error(`schema produces duplicate native node '${tag}'`);
                names.add(tag);
                const { [switched.switchOn]: _discriminator, ...variantAttrs } = node.attrs ?? {};
                nodes.push({
                    name: tag,
                    ...variantCommon,
                    group: appendGroup(node.group, name),
                    ...(Object.keys(variantAttrs).length === 0 ? {} : { attrs: variantAttrs }),
                    htmlTag: tag,
                    json: {
                        type: name,
                        attrs: {
                            [switched.switchOn]: caseAttributeValue(
                                caseKey,
                                switched.switchOn,
                                tag,
                                node.parseDOM,
                                discriminatorDefault
                            ),
                        },
                    },
                });
            }
            continue;
        }

        if (names.has(name)) throw new Error(`schema produces duplicate native node '${name}'`);
        names.add(name);
        const tag = staticDOMTag(
            'node',
            name,
            node.parseDOM,
            node.toDOM as DOMOutputSpec | undefined
        );
        nodes.push({
            name,
            ...common,
            ...(tag == null ? {} : { htmlTag: tag }),
        });
    }

    const marks: MarkSpec[] = Object.entries(spec.marks ?? {}).map(([name, mark]) => {
        const tag = staticDOMTag('mark', name, mark.parseDOM, mark.toDOM);
        const normalizedTag = tag?.toLowerCase();
        if (normalizedTag != null && !ALLOWED_MARK_HTML_TAGS.has(normalizedTag)) {
            throw new Error(`mark '${name}' has disallowed HTML tag '${tag}'`);
        }
        return {
            name,
            ...(mark.attrs == null ? {} : { attrs: mark.attrs }),
            ...(mark.excludes == null ? {} : { excludes: mark.excludes }),
            ...(normalizedTag == null ? {} : { htmlTag: normalizedTag as MarkSpec['htmlTag'] }),
            ...(mark.allowUndeclaredAttrs == null
                ? {}
                : { allowUndeclaredAttrs: mark.allowUndeclaredAttrs }),
        };
    });
    return { nodes, marks };
}

/** Attributes of the built-in image node. */
export interface ImageNodeAttributes {
    /** Image source: an `https:` URL, or a `data:` URL when base64 images are allowed. */
    src: string;
    alt?: string | null;
    title?: string | null;
    /** Intrinsic width in layout units. Null lets the renderer choose. */
    width?: number | null;
    /** Intrinsic height in layout units. Null lets the renderer choose. */
    height?: number | null;
}

/** A schema together with the facts derived from it. See {@link resolveDocumentDescriptor}. */
export interface ResolvedDocumentSchema {
    /** The schema itself, defaulted to {@link defaultSchema} when none was supplied. */
    schema: SchemaDefinition;
    /** Name of the node with `role: 'doc'` — the `type` of a document's root. */
    documentNodeName: string;
    /** The smallest document this schema admits. Used by `clearContent()`. */
    emptyDocument: DocumentJSON;
}

type DocumentDescriptorLimits = Pick<
    EditorResourceLimits,
    'maxSchemaNodes' | 'maxSchemaExpressionBytes' | 'maxDocumentNodes' | 'maxDocumentDepth'
>;

/** Node name the built-in image node is stored under. */
export const IMAGE_NODE_NAME = 'image';
const HEADING_LEVELS = [1, 2, 3, 4, 5, 6] as const;

/**
 * The built-in image node spec: a void block node carrying
 * {@link ImageNodeAttributes}. Add it through {@link withImagesSchema}.
 *
 * @param name Node name to declare it under. Defaults to {@link IMAGE_NODE_NAME}.
 */
function imageSchemaNodeSpec(): SchemaNodeSpec {
    return {
        group: 'block',
        attrs: {
            src: {},
            alt: { default: null },
            title: { default: null },
            width: { default: null },
            height: { default: null },
        },
        role: 'block',
        parseDOM: [{ tag: 'img' }],
        toDOM: ['img'],
        isVoid: true,
    };
}

export function imageNodeSpec(name: string = IMAGE_NODE_NAME): NodeSpec {
    return defineSchema({ nodes: { [name]: imageSchemaNodeSpec() } }).nodes[0]!;
}

/**
 * Return `schema` with the image node added, or `schema` unchanged when it
 * already declares one. {@link tiptapCompatibleSchema} already includes it.
 */
export function withImagesSchema(schema: SchemaDefinition): SchemaDefinition {
    const hasImageNode = schema.nodes.some((node) => node.name === IMAGE_NODE_NAME);
    if (hasImageNode) {
        return schema;
    }

    return {
        ...schema,
        nodes: [...schema.nodes, imageNodeSpec()],
    };
}

/**
 * Wrap inline or block nodes in a document root, producing a fragment ready
 * for `insertContentJson`.
 *
 * @param descriptor Document node name to wrap in. Defaults to `doc`; pass a
 * {@link ResolvedDocumentSchema} when the schema names its root differently.
 */
export function buildDocumentFragmentJson(
    content: DocumentJSON[],
    descriptor: Pick<ResolvedDocumentSchema, 'documentNodeName'> = {
        documentNodeName: 'doc',
    }
): DocumentJSON {
    return { type: descriptor.documentNodeName, content };
}

/**
 * Build a document fragment holding one image node, ready for
 * `insertContentJson`. `NativeRichTextEditorRef.insertImage` does this for
 * you; use this when assembling a larger insertion by hand.
 */
export function buildImageFragmentJson(
    attrs: ImageNodeAttributes,
    descriptor?: Pick<ResolvedDocumentSchema, 'documentNodeName'>
): DocumentJSON {
    return buildDocumentFragmentJson(
        [
            {
                type: IMAGE_NODE_NAME,
                attrs,
            },
        ],
        descriptor
    );
}

/**
 * Keyed authoring definition for the Tiptap-compatible camelCase schema.
 */
export const tiptapCompatibleSchemaSpec: SchemaSpec = {
    nodes: {
        doc: { content: 'block+', role: 'doc' },
        paragraph: {
            content: 'inline*',
            group: 'block',
            role: 'textBlock',
            parseDOM: [{ tag: 'p' }],
            toDOM: ['p', 0],
        },
        heading: {
            content: 'inline*',
            group: 'block',
            role: 'heading',
            attrs: { level: { default: 1 } },
            parseDOM: HEADING_LEVELS.map((level) => ({
                tag: `h${level}`,
                attrs: { level },
            })),
            toDOM: {
                switchOn: 'level',
                cases: Object.fromEntries(
                    HEADING_LEVELS.map((level) => [level, [`h${level}`, 0] as const])
                ),
            },
        },
        blockquote: {
            content: 'block+',
            group: 'block',
            role: 'block',
            parseDOM: [{ tag: 'blockquote' }],
            toDOM: ['blockquote', 0],
        },
        bulletList: {
            content: 'listItem+',
            group: 'block',
            role: 'list',
            parseDOM: [{ tag: 'ul' }],
            toDOM: ['ul', 0],
        },
        orderedList: {
            content: 'listItem+',
            group: 'block',
            attrs: { start: { default: 1 } },
            role: 'list',
            parseDOM: [{ tag: 'ol' }],
            toDOM: ['ol', 0],
        },
        listItem: {
            content: 'paragraph block*',
            role: 'listItem',
            parseDOM: [{ tag: 'li' }],
            toDOM: ['li', 0],
        },
        hardBreak: {
            group: 'inline',
            role: 'hardBreak',
            parseDOM: [{ tag: 'br' }],
            toDOM: ['br'],
            isVoid: true,
        },
        horizontalRule: {
            group: 'block',
            role: 'block',
            parseDOM: [{ tag: 'hr' }],
            toDOM: ['hr'],
            isVoid: true,
        },
        [IMAGE_NODE_NAME]: imageSchemaNodeSpec(),
        text: { group: 'inline', role: 'text' },
    },
    marks: {
        bold: { parseDOM: [{ tag: 'strong' }], toDOM: ['strong', 0] },
        italic: { parseDOM: [{ tag: 'em' }], toDOM: ['em', 0] },
        underline: { parseDOM: [{ tag: 'u' }], toDOM: ['u', 0] },
        strike: { parseDOM: [{ tag: 's' }], toDOM: ['s', 0] },
        link: { attrs: { href: {} }, parseDOM: [{ tag: 'a' }], toDOM: ['a', 0] },
    },
};

/** Tiptap-compatible serializable schema using camelCase node names. */
export const tiptapCompatibleSchema: SchemaDefinition = defineSchema(tiptapCompatibleSchemaSpec);

/** Keyed authoring definition using ProseMirror snake_case node names. */
export const prosemirrorSchemaSpec: SchemaSpec = {
    nodes: {
        doc: tiptapCompatibleSchemaSpec.nodes.doc,
        paragraph: tiptapCompatibleSchemaSpec.nodes.paragraph,
        heading: tiptapCompatibleSchemaSpec.nodes.heading,
        blockquote: tiptapCompatibleSchemaSpec.nodes.blockquote,
        bullet_list: {
            ...tiptapCompatibleSchemaSpec.nodes.bulletList,
            content: 'list_item+',
        },
        ordered_list: {
            ...tiptapCompatibleSchemaSpec.nodes.orderedList,
            content: 'list_item+',
        },
        list_item: tiptapCompatibleSchemaSpec.nodes.listItem,
        hard_break: tiptapCompatibleSchemaSpec.nodes.hardBreak,
        horizontal_rule: tiptapCompatibleSchemaSpec.nodes.horizontalRule,
        [IMAGE_NODE_NAME]: tiptapCompatibleSchemaSpec.nodes[IMAGE_NODE_NAME],
        text: tiptapCompatibleSchemaSpec.nodes.text,
    },
    marks: tiptapCompatibleSchemaSpec.marks,
};

/** ProseMirror-compatible serializable schema using snake_case node names. */
export const prosemirrorSchema: SchemaDefinition = defineSchema(prosemirrorSchemaSpec);

/** Keyed authoring definition used when callers do not provide a schema. */
export const defaultSchemaSpec: SchemaSpec = prosemirrorSchemaSpec;

/** Schema used when callers do not provide one. */
export const defaultSchema: SchemaDefinition = prosemirrorSchema;

/** Mirror native's invalid-schema fallback before constructing an empty doc. */
function utf8ByteLengthUpTo(value: string, maximum: number): number {
    let bytes = 0;
    for (const char of value) {
        const codePoint = char.codePointAt(0) ?? 0;
        bytes += codePoint <= 0x7f ? 1 : codePoint <= 0x7ff ? 2 : codePoint <= 0xffff ? 3 : 4;
        if (bytes > maximum) return bytes;
    }
    return bytes;
}

function forEachGroupToken(
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

interface SchemaWorkBudget {
    readonly limit: number;
    work: number;
    exhausted: boolean;
}

interface AdmittedSchemaCollections {
    readonly groupsByNode: ReadonlyMap<NodeSpec, readonly string[]>;
    readonly attrsByNode: ReadonlyMap<NodeSpec, ReadonlyArray<[string, AttrSpec]>>;
}

function consumeSchemaWork(budget: SchemaWorkBudget): boolean {
    return consumeSchemaWorkAmount(budget, 1);
}

function consumeSchemaWorkAmount(budget: SchemaWorkBudget, amount: number): boolean {
    if (amount > budget.limit - budget.work) {
        budget.exhausted = true;
        return false;
    }
    budget.work += amount;
    return true;
}

function consumeSchemaStringWork(budget: SchemaWorkBudget, value: string): boolean {
    return consumeSchemaWorkAmount(budget, utf8ByteLengthUpTo(value, budget.limit) + 1);
}

function collectOwnAttrs(
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

function isSafeHtmlTag(tag: string): boolean {
    if (tag.length === 0 || tag[0] < 'a' || tag[0] > 'z') return false;
    for (let index = 1; index < tag.length; index += 1) {
        const char = tag[index];
        if (!((char >= 'a' && char <= 'z') || (char >= '0' && char <= '9') || char === '-')) {
            return false;
        }
    }
    return true;
}

function isSafeHtmlAttr(name: string): boolean {
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

function normalizeAttrs(value: unknown): Record<string, AttrSpec> {
    if (value == null || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(
        Object.entries(value).map(([name, rawSpec]) => {
            if (rawSpec == null || typeof rawSpec !== 'object' || Array.isArray(rawSpec)) {
                return [name, {}];
            }
            return Object.prototype.hasOwnProperty.call(rawSpec, 'default') &&
                (rawSpec as AttrSpec).default !== undefined
                ? [name, { default: (rawSpec as AttrSpec).default }]
                : [name, {}];
        })
    );
}

function normalizeNodeJSONProjection(value: unknown): NodeJSONProjection | null | undefined {
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

function projectionAttrsOverlap(
    left: Record<string, unknown>,
    right: Record<string, unknown>
): boolean {
    return Object.entries(left).every(
        ([name, value]) =>
            !Object.prototype.hasOwnProperty.call(right, name) || right[name] === value
    );
}

function legacyHeadingProjectionName(projection: NodeJSONProjection): string | undefined {
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

function normalizeSchemaDefinition(
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
        const attrs = normalizeAttrs(raw.attrs);
        const json = normalizeNodeJSONProjection(raw.json);
        if (
            (htmlTag != null && !isSafeHtmlTag(htmlTag)) ||
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
            ...(json == null ? {} : { json }),
            isVoid: typeof raw.isVoid === 'boolean' ? raw.isVoid : false,
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

function schemaBoundaryError(limit: number, actual: number): NativeEditorBoundaryError {
    return new NativeEditorBoundaryError(
        'SCHEMA_INVALID',
        `schema work exceeds configured limit ${limit}`,
        limit,
        actual
    );
}

function resolveDescriptorLimits(limits?: DocumentDescriptorLimits): ResolvedEditorResourceLimits {
    return resolveEditorResourceLimits(limits);
}

function createSchemaWorkBudget(limits: ResolvedEditorResourceLimits): SchemaWorkBudget {
    return {
        limit: limits.maxSchemaNodes * 64 + limits.maxSchemaExpressionBytes * 32,
        work: 0,
        exhausted: false,
    };
}

function admitSchemaCollections(
    schema: SchemaDefinition,
    budget: SchemaWorkBudget
): AdmittedSchemaCollections | null {
    const groupsByNode = new Map<NodeSpec, readonly string[]>();
    const attrsByNode = new Map<NodeSpec, ReadonlyArray<[string, AttrSpec]>>();

    for (let nodeIndex = 0; nodeIndex < schema.nodes.length; nodeIndex += 1) {
        const node = schema.nodes[nodeIndex];
        if (node == null || typeof node !== 'object') return null;
        const groups: string[] = [];
        if (typeof node.group === 'string') {
            if (
                !forEachGroupToken(
                    node.group,
                    (group) => groups.push(group),
                    () => consumeSchemaWork(budget),
                    () => consumeSchemaWork(budget)
                )
            ) {
                throw schemaBoundaryError(budget.limit, budget.limit + 1);
            }
        }
        const attrs = collectOwnAttrs(node.attrs, budget);
        if (attrs == null) return null;
        const projectionAttrs = collectOwnAttrs(node.json?.attrs, budget);
        if (projectionAttrs == null) return null;
        if (
            node.json != null &&
            ((typeof node.json.type === 'string' &&
                !consumeSchemaStringWork(budget, node.json.type)) ||
                projectionAttrs.some(([name, rawValue]) => {
                    const value: unknown = rawValue;
                    return (
                        !consumeSchemaStringWork(budget, name) ||
                        (typeof value === 'string'
                            ? !consumeSchemaStringWork(budget, value)
                            : !consumeSchemaWork(budget))
                    );
                }))
        ) {
            throw schemaBoundaryError(budget.limit, budget.limit + 1);
        }
        if (budget.exhausted) {
            throw schemaBoundaryError(budget.limit, budget.limit + 1);
        }
        groupsByNode.set(node, groups);
        attrsByNode.set(node, attrs);
    }

    for (let markIndex = 0; markIndex < schema.marks.length; markIndex += 1) {
        const mark = schema.marks[markIndex];
        if (!consumeSchemaWork(budget)) {
            throw schemaBoundaryError(budget.limit, budget.limit + 1);
        }
        if (mark == null || typeof mark !== 'object') return null;
        const attrs = collectOwnAttrs(mark.attrs, budget);
        if (attrs == null) return null;
        if (budget.exhausted) {
            throw schemaBoundaryError(budget.limit, budget.limit + 1);
        }
    }

    return { groupsByNode, attrsByNode };
}

/** Mirror native's invalid-schema fallback after bounded admission succeeds. */
export function resolveDocumentSchema(
    schema?: SchemaDefinition,
    limits?: DocumentDescriptorLimits
): SchemaDefinition {
    if (schema == null) return defaultSchema;
    if (!Array.isArray(schema.nodes)) return defaultSchema;
    if (!Array.isArray(schema.marks)) {
        schema = { ...schema, marks: [] };
    }

    const resolvedLimits = resolveDescriptorLimits(limits);
    if (schema.nodes.length > resolvedLimits.maxSchemaNodes) {
        throw schemaBoundaryError(resolvedLimits.maxSchemaNodes, schema.nodes.length);
    }
    let expressionBytes = 0;
    for (let nodeIndex = 0; nodeIndex < schema.nodes.length; nodeIndex += 1) {
        const node = schema.nodes[nodeIndex];
        if (node != null && typeof node.content === 'string') {
            expressionBytes += utf8ByteLengthUpTo(
                node.content,
                resolvedLimits.maxSchemaExpressionBytes - expressionBytes
            );
            if (expressionBytes > resolvedLimits.maxSchemaExpressionBytes) {
                throw schemaBoundaryError(resolvedLimits.maxSchemaExpressionBytes, expressionBytes);
            }
        }
    }
    const schemaBudget = createSchemaWorkBudget(resolvedLimits);
    if (admitSchemaCollections(schema, schemaBudget) == null) return defaultSchema;
    const normalizedSchema = normalizeSchemaDefinition(schema, schemaBudget);
    if (normalizedSchema == null) return defaultSchema;
    schema = normalizedSchema;
    const admittedCollections = admitSchemaCollections(schema, {
        limit: Number.MAX_SAFE_INTEGER,
        work: 0,
        exhausted: false,
    });
    if (admittedCollections == null) return defaultSchema;

    const nodeNames = new Set<string>();
    const markNames = new Set<string>();
    let docRoles = 0;
    let textRoles = 0;
    for (const node of schema.nodes) {
        if (
            node == null ||
            typeof node.name !== 'string' ||
            node.name.length === 0 ||
            typeof node.content !== 'string' ||
            typeof node.role !== 'string' ||
            nodeNames.has(node.name)
        ) {
            return defaultSchema;
        }
        nodeNames.add(node.name);
        if (node.role === 'doc') docRoles += 1;
        if (node.role === 'text') textRoles += 1;
    }
    for (const mark of schema.marks) {
        if (
            mark == null ||
            typeof mark.name !== 'string' ||
            mark.name.length === 0 ||
            markNames.has(mark.name)
        ) {
            return defaultSchema;
        }
        markNames.add(mark.name);
    }
    if (docRoles !== 1 || textRoles !== 1) return defaultSchema;

    const groups = new Set<string>();
    const nodesBySymbol = new Map<string, NodeSpec[]>();
    const addCandidate = (symbol: string, node: NodeSpec): void => {
        const candidates = nodesBySymbol.get(symbol);
        if (candidates) candidates.push(node);
        else nodesBySymbol.set(symbol, [node]);
    };
    for (const node of schema.nodes) {
        addCandidate(node.name, node);
        for (const group of admittedCollections.groupsByNode.get(node) ?? []) {
            groups.add(group);
            addCandidate(group, node);
        }
    }
    for (const node of schema.nodes) {
        const symbols = contentExpressionSymbols(node.content);
        if (
            symbols == null ||
            symbols.some((symbol) => !nodeNames.has(symbol) && !groups.has(symbol))
        ) {
            return defaultSchema;
        }
    }

    const generatable = new Set<string>();
    const consumeWork = (): boolean => consumeSchemaWork(schemaBudget);
    const contentIsConstructible = (node: NodeSpec): boolean => {
        if (!consumeWork()) return false;
        return (
            minimalContentMatch(
                node.content,
                (symbol) => {
                    const candidates = nodesBySymbol.get(symbol) ?? [];
                    for (const candidate of candidates) {
                        if (!consumeWork()) return undefined;
                        if (generatable.has(candidate.name)) return { type: candidate.name };
                    }
                    return undefined;
                },
                consumeWork
            ) != null
        );
    };
    while (true) {
        const before = generatable.size;
        for (const node of schema.nodes) {
            const hasRequiredAttrs = (admittedCollections.attrsByNode.get(node) ?? []).some(
                ([, attr]) => attr.default === undefined
            );
            if (node.role !== 'text' && !hasRequiredAttrs && contentIsConstructible(node)) {
                generatable.add(node.name);
            }
        }
        if (generatable.size === before) break;
    }
    if (schemaBudget.exhausted) {
        throw schemaBoundaryError(schemaBudget.limit, schemaBudget.limit + 1);
    }
    const hasUnconstructibleNode = schema.nodes.some((node) => !contentIsConstructible(node));
    if (schemaBudget.exhausted) {
        throw schemaBoundaryError(schemaBudget.limit, schemaBudget.limit + 1);
    }
    if (hasUnconstructibleNode) return defaultSchema;

    try {
        constructDefaultEmptyDocument(schema, resolvedLimits, admittedCollections);
        return schema;
    } catch (error) {
        if (error instanceof NativeEditorBoundaryError) throw error;
        return defaultSchema;
    }
}

function constructDefaultEmptyDocument(
    schema: SchemaDefinition,
    resolvedLimits: ResolvedEditorResourceLimits,
    admittedCollections: AdmittedSchemaCollections
): DocumentJSON {
    const docNode = schema.nodes.find((node) => node.role === 'doc');
    if (!docNode) throw new Error('schema cannot construct a default document: missing doc role');

    const budget = { nodes: 0, work: 0, exhausted: false };
    const workLimit = Math.min(
        DEFAULT_CONTENT_MAX_NODES,
        resolvedLimits.maxSchemaNodes * 64 + resolvedLimits.maxSchemaExpressionBytes * 32
    );
    const consumeWork = (): boolean => {
        if (budget.work >= workLimit) {
            budget.exhausted = true;
            return false;
        }
        budget.work += 1;
        return true;
    };
    const candidatePriority = (candidate: NodeSpec): number => {
        if (
            candidate.role === 'textBlock' &&
            (candidate.htmlTag === 'p' || candidate.name === 'paragraph')
        ) {
            return 0;
        }
        return candidate.role === 'textBlock' ? 1 : 2;
    };
    const candidatesBySymbol = new Map<string, NodeSpec[]>();
    let candidateIndexAdmitted = true;
    const addCandidate = (symbol: string, candidate: NodeSpec): boolean => {
        if (!consumeWork()) return false;
        const candidates = candidatesBySymbol.get(symbol);
        if (candidates) candidates.push(candidate);
        else candidatesBySymbol.set(symbol, [candidate]);
        return true;
    };
    for (const candidate of schema.nodes) {
        if (!addCandidate(candidate.name, candidate)) {
            candidateIndexAdmitted = false;
            break;
        }
        for (const group of admittedCollections.groupsByNode.get(candidate) ?? []) {
            if (group !== candidate.name && !addCandidate(group, candidate)) {
                candidateIndexAdmitted = false;
                break;
            }
        }
        if (!candidateIndexAdmitted) break;
    }
    for (const candidates of candidatesBySymbol.values()) {
        const sortingWork = Math.max(
            candidates.length,
            Math.ceil(candidates.length * Math.log2(Math.max(2, candidates.length)))
        );
        for (let index = 0; index < sortingWork; index += 1) {
            if (!consumeWork()) {
                candidateIndexAdmitted = false;
                break;
            }
        }
        if (!candidateIndexAdmitted) break;
        candidates.sort(
            (left, right) =>
                candidatePriority(left) - candidatePriority(right) ||
                left.name.localeCompare(right.name)
        );
    }
    if (!candidateIndexAdmitted) {
        throw schemaBoundaryError(workLimit, workLimit + 1);
    }
    const constructNode = (
        node: NodeSpec,
        visiting: Set<string>,
        depth: number
    ): DocumentJSON | undefined => {
        if (
            depth > Math.min(CONTENT_EXPRESSION_MAX_DEPTH, resolvedLimits.maxDocumentDepth) ||
            !consumeWork()
        ) {
            return undefined;
        }
        if (node.role === 'text' || visiting.has(node.name)) return undefined;
        const attrs = admittedCollections.attrsByNode.get(node) ?? [];
        for (const _attr of attrs) {
            if (!consumeWork()) {
                budget.exhausted = true;
                return undefined;
            }
        }
        if (attrs.some(([, spec]) => spec.default === undefined)) {
            return undefined;
        }

        visiting.add(node.name);
        const children = minimalContentMatch(
            node.content ?? '',
            (symbol) => {
                for (const candidate of candidatesBySymbol.get(symbol) ?? []) {
                    if (!consumeWork()) return undefined;
                    const constructed = constructNode(candidate, visiting, depth + 1);
                    if (constructed) return constructed;
                }
                return undefined;
            },
            consumeWork
        );
        visiting.delete(node.name);
        if (!children) return undefined;
        if (budget.nodes >= resolvedLimits.maxDocumentNodes) {
            budget.exhausted = true;
            return undefined;
        }
        budget.nodes += 1;

        const result: DocumentJSON = { type: node.json?.type ?? node.name };
        if (children.length > 0) result.content = children;
        const projectedAttrs = Object.entries(node.json?.attrs ?? {});
        if (attrs.length > 0 || projectedAttrs.length > 0) {
            result.attrs = Object.fromEntries([
                ...attrs.map(([name, spec]) => [name, spec.default] as const),
                ...projectedAttrs,
            ]);
        }
        return result;
    };

    const document = constructNode(docNode, new Set(), 0);
    if (!document) {
        if (budget.exhausted || budget.work >= workLimit) {
            throw schemaBoundaryError(workLimit, workLimit + 1);
        }
        throw new Error(`schema cannot construct a default document for '${docNode.name}'`);
    }
    return document;
}

export function defaultEmptyDocument(
    schema: SchemaDefinition = defaultSchema,
    limits?: DocumentDescriptorLimits
): DocumentJSON {
    const resolvedLimits = resolveDescriptorLimits(limits);
    if (schema.nodes.length > resolvedLimits.maxSchemaNodes) {
        throw schemaBoundaryError(resolvedLimits.maxSchemaNodes, schema.nodes.length);
    }
    let expressionBytes = 0;
    for (const node of schema.nodes) {
        if (node != null && typeof node.content === 'string') {
            expressionBytes += utf8ByteLengthUpTo(
                node.content,
                resolvedLimits.maxSchemaExpressionBytes - expressionBytes
            );
            if (expressionBytes > resolvedLimits.maxSchemaExpressionBytes) {
                throw schemaBoundaryError(resolvedLimits.maxSchemaExpressionBytes, expressionBytes);
            }
        }
    }
    const admissionBudget = createSchemaWorkBudget(resolvedLimits);
    const admittedCollections = admitSchemaCollections(schema, admissionBudget);
    if (admittedCollections == null) {
        throw new Error('schema cannot construct a default document: invalid schema collections');
    }
    return constructDefaultEmptyDocument(schema, resolvedLimits, admittedCollections);
}

/**
 * Validate a schema and derive its document node name and empty document,
 * mirroring what the Rust core does when a handle is created. Use it to build
 * fragments or empty documents for a custom schema.
 *
 * @param schema Defaults to {@link defaultSchema}.
 * @param limits Schema and document bounds. Defaults to
 * {@link DEFAULT_EDITOR_RESOURCE_LIMITS}.
 * @throws NativeEditorBoundaryError `SCHEMA_INVALID` when the schema is
 * malformed, exceeds its limits, or declares no `role: 'doc'` node.
 */
export function resolveDocumentDescriptor(
    schema?: SchemaDefinition,
    limits?: DocumentDescriptorLimits
): ResolvedDocumentSchema {
    const resolvedLimits = resolveDescriptorLimits(limits);
    const resolvedSchema = resolveDocumentSchema(schema, resolvedLimits);
    const documentNode = resolvedSchema.nodes.find((node) => node.role === 'doc');
    if (!documentNode) {
        throw new NativeEditorBoundaryError('SCHEMA_INVALID', 'schema has no document-role node');
    }
    return {
        schema: resolvedSchema,
        documentNodeName: documentNode.name,
        emptyDocument: defaultEmptyDocument(resolvedSchema, resolvedLimits),
    };
}

export function normalizeDocumentJson(
    doc: DocumentJSON,
    schemaOrDescriptor: SchemaDefinition | ResolvedDocumentSchema = defaultSchema,
    limits?: DocumentDescriptorLimits
): DocumentJSON {
    const descriptor =
        'documentNodeName' in schemaOrDescriptor
            ? schemaOrDescriptor
            : resolveDocumentDescriptor(schemaOrDescriptor, limits);
    const root = doc as { type?: unknown; content?: unknown } | null;
    if (root?.type !== descriptor.documentNodeName) {
        return doc;
    }
    const content = root?.content;
    if (Array.isArray(content) && content.length > 0) {
        return doc;
    }
    return descriptor.emptyDocument;
}
