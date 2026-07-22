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

export interface AttrSpec {
    default?: unknown;
}

export interface NodeSpec {
    name: string;
    content: string;
    group?: string;
    attrs?: Record<string, AttrSpec>;
    role: string;
    htmlTag?: string;
    isVoid?: boolean;
    /**
     * Opt-in escape hatch: when `true`, JSON ingestion (`set_json` /
     * `insert_content_json`) admits attrs on this node that are not declared
     * in `attrs`, instead of filtering them out. Default `false`. Intended
     * for node types with an intentional pass-through-metadata contract
     * (e.g. the mention node — see `mentionNodeSpec()` in addons.ts).
     */
    allowUndeclaredAttrs?: boolean;
}

export interface MarkSpec {
    name: string;
    attrs?: Record<string, AttrSpec>;
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

export interface SchemaDefinition {
    nodes: NodeSpec[];
    marks: MarkSpec[];
}

export interface ImageNodeAttributes {
    src: string;
    alt?: string | null;
    title?: string | null;
    width?: number | null;
    height?: number | null;
}

export interface ResolvedDocumentSchema {
    schema: SchemaDefinition;
    documentNodeName: string;
    emptyDocument: DocumentJSON;
}

type DocumentDescriptorLimits = Pick<
    EditorResourceLimits,
    'maxSchemaNodes' | 'maxSchemaExpressionBytes' | 'maxDocumentNodes' | 'maxDocumentDepth'
>;

export const IMAGE_NODE_NAME = 'image';
const HEADING_LEVELS = [1, 2, 3, 4, 5, 6] as const;
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

export function imageNodeSpec(name: string = IMAGE_NODE_NAME): NodeSpec {
    return {
        name,
        content: '',
        group: 'block',
        attrs: {
            src: {},
            alt: { default: null },
            title: { default: null },
            width: { default: null },
            height: { default: null },
        },
        role: 'block',
        htmlTag: 'img',
        isVoid: true,
    };
}

function headingNodeSpec(level: (typeof HEADING_LEVELS)[number]): NodeSpec {
    return {
        name: `h${level}`,
        content: 'inline*',
        group: 'block',
        role: 'textBlock',
        htmlTag: `h${level}`,
    };
}

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

export function buildDocumentFragmentJson(
    content: DocumentJSON[],
    descriptor: Pick<ResolvedDocumentSchema, 'documentNodeName'> = {
        documentNodeName: 'doc',
    }
): DocumentJSON {
    return { type: descriptor.documentNodeName, content };
}

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

const MARKS: MarkSpec[] = [
    { name: 'bold' },
    { name: 'italic' },
    { name: 'underline' },
    { name: 'strike' },
    { name: 'link', attrs: { href: {} } },
];

export const tiptapSchema: SchemaDefinition = {
    nodes: [
        {
            name: 'doc',
            content: 'block+',
            role: 'doc',
        },
        {
            name: 'paragraph',
            content: 'inline*',
            group: 'block',
            role: 'textBlock',
            htmlTag: 'p',
        },
        ...HEADING_LEVELS.map((level) => headingNodeSpec(level)),
        {
            name: 'blockquote',
            content: 'block+',
            group: 'block',
            role: 'block',
            htmlTag: 'blockquote',
        },
        {
            name: 'bulletList',
            content: 'listItem+',
            group: 'block',
            role: 'list',
            htmlTag: 'ul',
        },
        {
            name: 'orderedList',
            content: 'listItem+',
            group: 'block',
            attrs: { start: { default: 1 } },
            role: 'list',
            htmlTag: 'ol',
        },
        {
            name: 'listItem',
            content: 'paragraph block*',
            role: 'listItem',
            htmlTag: 'li',
        },
        {
            name: 'hardBreak',
            content: '',
            group: 'inline',
            role: 'hardBreak',
            htmlTag: 'br',
            isVoid: true,
        },
        {
            name: 'horizontalRule',
            content: '',
            group: 'block',
            role: 'block',
            htmlTag: 'hr',
            isVoid: true,
        },
        imageNodeSpec(),
        {
            name: 'text',
            content: '',
            group: 'inline',
            role: 'text',
        },
    ],
    marks: MARKS,
};

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
    if (budget.work >= budget.limit) {
        budget.exhausted = true;
        return false;
    }
    budget.work += 1;
    return true;
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

function normalizeSchemaDefinition(schema: SchemaDefinition): SchemaDefinition | null {
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
        if (
            (htmlTag != null && !isSafeHtmlTag(htmlTag)) ||
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
            isVoid: typeof raw.isVoid === 'boolean' ? raw.isVoid : false,
            ...(typeof raw.allowUndeclaredAttrs === 'boolean'
                ? { allowUndeclaredAttrs: raw.allowUndeclaredAttrs }
                : {}),
        });
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
        limit:
            limits.maxSchemaNodes * 64 +
            limits.maxSchemaExpressionBytes * 32,
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
    if (schema == null) return tiptapSchema;
    if (!Array.isArray(schema.nodes)) return tiptapSchema;
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
    if (admitSchemaCollections(schema, schemaBudget) == null) return tiptapSchema;
    const normalizedSchema = normalizeSchemaDefinition(schema);
    if (normalizedSchema == null) return tiptapSchema;
    schema = normalizedSchema;
    const admittedCollections = admitSchemaCollections(schema, {
        limit: Number.MAX_SAFE_INTEGER,
        work: 0,
        exhausted: false,
    });
    if (admittedCollections == null) return tiptapSchema;

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
            return tiptapSchema;
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
            return tiptapSchema;
        }
        markNames.add(mark.name);
    }
    if (docRoles !== 1 || textRoles !== 1) return tiptapSchema;

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
            return tiptapSchema;
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
            const hasRequiredAttrs = (
                admittedCollections.attrsByNode.get(node) ?? []
            ).some(([, attr]) => attr.default === undefined);
            if (node.role !== 'text' && !hasRequiredAttrs && contentIsConstructible(node)) {
                generatable.add(node.name);
            }
        }
        if (generatable.size === before) break;
    }
    if (schemaBudget.exhausted) {
        throw schemaBoundaryError(schemaBudget.limit, schemaBudget.limit + 1);
    }
    const hasUnconstructibleNode = schema.nodes.some(
        (node) => !contentIsConstructible(node)
    );
    if (schemaBudget.exhausted) {
        throw schemaBoundaryError(schemaBudget.limit, schemaBudget.limit + 1);
    }
    if (hasUnconstructibleNode) return tiptapSchema;

    try {
        constructDefaultEmptyDocument(schema, resolvedLimits, admittedCollections);
        return schema;
    } catch (error) {
        if (error instanceof NativeEditorBoundaryError) throw error;
        return tiptapSchema;
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

        const result: DocumentJSON = { type: node.name };
        if (children.length > 0) result.content = children;
        if (attrs.length > 0) {
            result.attrs = Object.fromEntries(attrs.map(([name, spec]) => [name, spec.default]));
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
    schema: SchemaDefinition = tiptapSchema,
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
                throw schemaBoundaryError(
                    resolvedLimits.maxSchemaExpressionBytes,
                    expressionBytes
                );
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
    schemaOrDescriptor: SchemaDefinition | ResolvedDocumentSchema = tiptapSchema,
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

export const prosemirrorSchema: SchemaDefinition = {
    nodes: [
        {
            name: 'doc',
            content: 'block+',
            role: 'doc',
        },
        {
            name: 'paragraph',
            content: 'inline*',
            group: 'block',
            role: 'textBlock',
            htmlTag: 'p',
        },
        ...HEADING_LEVELS.map((level) => headingNodeSpec(level)),
        {
            name: 'blockquote',
            content: 'block+',
            group: 'block',
            role: 'block',
            htmlTag: 'blockquote',
        },
        {
            name: 'bullet_list',
            content: 'list_item+',
            group: 'block',
            role: 'list',
            htmlTag: 'ul',
        },
        {
            name: 'ordered_list',
            content: 'list_item+',
            group: 'block',
            attrs: { start: { default: 1 } },
            role: 'list',
            htmlTag: 'ol',
        },
        {
            name: 'list_item',
            content: 'paragraph block*',
            role: 'listItem',
            htmlTag: 'li',
        },
        {
            name: 'hard_break',
            content: '',
            group: 'inline',
            role: 'hardBreak',
            htmlTag: 'br',
            isVoid: true,
        },
        {
            name: 'horizontal_rule',
            content: '',
            group: 'block',
            role: 'block',
            htmlTag: 'hr',
            isVoid: true,
        },
        imageNodeSpec('image'),
        {
            name: 'text',
            content: '',
            group: 'inline',
            role: 'text',
        },
    ],
    marks: MARKS,
};
