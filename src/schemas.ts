import type { DocumentJSON } from './NativeEditorBridge';

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
}

export interface MarkSpec {
    name: string;
    attrs?: Record<string, AttrSpec>;
    excludes?: string;
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

function acceptingGroupsForChildCount(content: string, existingChildCount: number): string[] {
    const tokens = content
        .trim()
        .split(/\s+/)
        .filter(Boolean)
        .map((token) => {
            const quantifier = token[token.length - 1];
            if (quantifier === '+' || quantifier === '*' || quantifier === '?') {
                return {
                    group: token.slice(0, -1),
                    min: quantifier === '+' ? 1 : 0,
                    max: quantifier === '?' ? 1 : null,
                };
            }
            return {
                group: token,
                min: 1,
                max: 1,
            };
        });

    let remaining = existingChildCount;
    const acceptingGroups: string[] = [];
    for (const token of tokens) {
        if (remaining >= token.min) {
            const consumed = token.max == null ? remaining : Math.min(remaining, token.max);
            remaining = Math.max(0, remaining - consumed);
            const atMax = token.max != null && consumed >= token.max;
            if (!atMax) {
                acceptingGroups.push(token.group);
            }
            continue;
        }

        acceptingGroups.push(token.group);
        break;
    }

    return acceptingGroups;
}

export function defaultEmptyDocument(schema: SchemaDefinition = tiptapSchema): DocumentJSON {
    const docNode = schema.nodes.find((node) => node.role === 'doc' || node.name === 'doc');
    const acceptingGroups =
        docNode == null ? [] : acceptingGroupsForChildCount(docNode.content ?? '', 0);
    const matchingTextBlocks = schema.nodes.filter(
        (node) =>
            node.role === 'textBlock' &&
            acceptingGroups.some((group) => node.name === group || node.group === group)
    );
    const preferredTextBlock =
        matchingTextBlocks.find((node) => node.htmlTag === 'p' || node.name === 'paragraph') ??
        matchingTextBlocks[0] ??
        schema.nodes.find((node) => node.htmlTag === 'p' || node.name === 'paragraph') ??
        schema.nodes.find((node) => node.role === 'textBlock');

    return {
        type: 'doc',
        content: [{ type: preferredTextBlock?.name ?? 'paragraph' }],
    };
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
