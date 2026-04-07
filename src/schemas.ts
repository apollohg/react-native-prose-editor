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
