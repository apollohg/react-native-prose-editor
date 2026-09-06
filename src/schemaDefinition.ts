import { withAtomsSchema, type AtomNodeDefinition } from './atoms';
import { RESERVED_WIRE_NODE_TYPES } from './atomPolicy';

/** Declaration of one node or mark attribute. */
export interface AttrSpec {
    /** Value used when the attribute is absent. Omit to make the attribute required. */
    default?: unknown;
    type?: 'string' | 'number' | 'boolean' | 'object' | 'array';
    enum?: readonly unknown[];
    min?: number;
    max?: number;
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

/** Declarative HTML parsing and serialization rules for a void node. */
export interface NodeHtmlRules {
    tag: string;
    staticAttrs?: Record<string, string>;
    attrMap?: Record<string, string>;
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
    /** Declarative HTML rules used for lossless void-node round trips. */
    html?: NodeHtmlRules;
    /** Whether the node holds no content of its own (an image or rule, say). */
    isVoid?: boolean;
    /** Whether collapsed backspace may remove this void block from an adjacent caret. */
    deletableOnBackspace?: boolean;
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
 * handle is created (`NativeEditorCreateConfig.schema`), or per render for
 * `RichTextViewer`. Start from {@link defaultSchema},
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
    html?: NodeHtmlRules;
    isVoid?: boolean;
    deletableOnBackspace?: boolean;
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
    atoms?: readonly AtomNodeDefinition<any>[];
}

export const ALLOWED_MARK_HTML_TAGS = new Set([
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

export function outputTag(spec: DOMOutputSpec): string {
    return spec[0];
}

export function appendGroup(group: string | undefined, name: string): string {
    const groups = group?.split(/\s+/).filter(Boolean) ?? [];
    if (!groups.includes(name)) groups.push(name);
    return groups.join(' ');
}

export function schemaNodeRole(name: string, node: SchemaNodeSpec): string {
    if (node.role != null) return node.role === 'heading' ? 'textBlock' : node.role;
    if (name === 'doc') return 'doc';
    if (name === 'text') return 'text';
    if (node.content === 'inline*' && node.group?.split(/\s+/).includes('block')) {
        return 'textBlock';
    }
    if (node.group?.split(/\s+/).includes('inline')) return 'inline';
    return 'block';
}

export function caseAttributeValue(
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

export function validateAttributeDOMRules(
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

export function staticDOMTag(
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
            ...(node.html == null ? {} : { html: node.html }),
            ...(node.isVoid == null ? {} : { isVoid: node.isVoid }),
            ...(node.deletableOnBackspace == null
                ? {}
                : { deletableOnBackspace: node.deletableOnBackspace }),
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
    const schema = { nodes, marks };
    return spec.atoms == null || spec.atoms.length === 0
        ? schema
        : withAtomsSchema(schema, spec.atoms);
}
