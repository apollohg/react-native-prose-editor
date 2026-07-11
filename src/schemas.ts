import type { DocumentJSON } from './NativeEditorBridge';
import {
    CONTENT_EXPRESSION_MAX_DEPTH,
    DEFAULT_CONTENT_MAX_NODES,
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

export const IMAGE_NODE_NAME = 'image';
const HEADING_LEVELS = [1, 2, 3, 4, 5, 6] as const;

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

export function buildImageFragmentJson(attrs: ImageNodeAttributes): DocumentJSON {
    return {
        type: 'doc',
        content: [
            {
                type: IMAGE_NODE_NAME,
                attrs,
            },
        ],
    };
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

export function defaultEmptyDocument(schema: SchemaDefinition = tiptapSchema): DocumentJSON {
    const docNode = schema.nodes.find((node) => node.role === 'doc' || node.name === 'doc');
    if (!docNode) throw new Error('schema cannot construct a default document: missing doc role');

    const budget = { nodes: 0, work: 0 };
    const consumeWork = (): boolean => {
        if (budget.work >= DEFAULT_CONTENT_MAX_NODES) return false;
        budget.work += 1;
        return true;
    };
    const constructNode = (
        node: NodeSpec,
        visiting: Set<string>,
        depth: number
    ): DocumentJSON | undefined => {
        if (depth > CONTENT_EXPRESSION_MAX_DEPTH || !consumeWork()) {
            return undefined;
        }
        if (node.role === 'text' || visiting.has(node.name)) return undefined;
        const attrs = Object.entries(node.attrs ?? {});
        if (attrs.some(([, spec]) => spec.default === undefined)) {
            return undefined;
        }

        visiting.add(node.name);
        const children = minimalContentMatch(
            node.content ?? '',
            (symbol) => {
                const candidates = schema.nodes
                    .filter(
                        (candidate) =>
                            candidate.name === symbol ||
                            candidate.group?.split(/\s+/).some((group) => group === symbol)
                    )
                    .sort((left, right) => {
                        const priority = (candidate: NodeSpec): number => {
                            if (
                                candidate.role === 'textBlock' &&
                                (candidate.htmlTag === 'p' || candidate.name === 'paragraph')
                            ) {
                                return 0;
                            }
                            return candidate.role === 'textBlock' ? 1 : 2;
                        };
                        return (
                            priority(left) - priority(right) || left.name.localeCompare(right.name)
                        );
                    });
                for (const candidate of candidates) {
                    const constructed = constructNode(candidate, visiting, depth + 1);
                    if (constructed) return constructed;
                }
                return undefined;
            },
            consumeWork
        );
        visiting.delete(node.name);
        if (!children) return undefined;
        if (budget.nodes >= DEFAULT_CONTENT_MAX_NODES) return undefined;
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
        throw new Error(`schema cannot construct a default document for '${docNode.name}'`);
    }
    return document;
}

export function normalizeDocumentJson(
    doc: DocumentJSON,
    schema: SchemaDefinition = tiptapSchema
): DocumentJSON {
    const root = doc as { type?: unknown; content?: unknown } | null;
    if (root?.type !== 'doc') {
        return doc;
    }
    if (Array.isArray(root.content) && root.content.length > 0) {
        return doc;
    }
    return defaultEmptyDocument(schema);
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
