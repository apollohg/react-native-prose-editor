import type { DocumentJSON } from './NativeEditorBridge';
import { type EditorResourceLimits } from './ResourceLimits';
import {
    type SchemaDefinition,
    type SchemaNodeSpec,
    type NodeSpec,
    defineSchema,
    type SchemaSpec,
} from './schemaDefinition';

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

export type DocumentDescriptorLimits = Pick<
    EditorResourceLimits,
    'maxSchemaNodes' | 'maxSchemaExpressionBytes' | 'maxDocumentNodes' | 'maxDocumentDepth'
>;

/** Node name the built-in image node is stored under. */
export const IMAGE_NODE_NAME = 'image';

export const HEADING_LEVELS = [1, 2, 3, 4, 5, 6] as const;

/**
 * The built-in image node spec: a void block node carrying
 * {@link ImageNodeAttributes}. Add it through {@link withImagesSchema}.
 *
 * @param name Node name to declare it under. Defaults to {@link IMAGE_NODE_NAME}.
 */
export function imageSchemaNodeSpec(): SchemaNodeSpec {
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
        deletableOnBackspace: false,
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
 * `insertContentJson`. `RichTextEditorRef.insertImage` does this for
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
        codeBlock: {
            attrs: { language: { default: null } },
            content: 'text*',
            group: 'block',
            role: 'textBlock',
            parseDOM: [{ tag: 'pre' }],
            toDOM: ['pre', 0],
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
        code: { parseDOM: [{ tag: 'code' }], toDOM: ['code', 0] },
        link: { attrs: { href: {} }, parseDOM: [{ tag: 'a' }], toDOM: ['a', 0] },
    },
};

/** Tiptap-compatible serializable schema using camelCase node names. */
export const tiptapCompatibleSchema: SchemaDefinition = defineSchema(tiptapCompatibleSchemaSpec);

/** Keyed authoring definition using ProseMirror names, including upstream `codeBlock`. */
export const prosemirrorSchemaSpec: SchemaSpec = {
    nodes: {
        doc: tiptapCompatibleSchemaSpec.nodes.doc,
        paragraph: tiptapCompatibleSchemaSpec.nodes.paragraph,
        heading: tiptapCompatibleSchemaSpec.nodes.heading,
        blockquote: tiptapCompatibleSchemaSpec.nodes.blockquote,
        codeBlock: tiptapCompatibleSchemaSpec.nodes.codeBlock,
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

/** ProseMirror-compatible serializable schema, including upstream `codeBlock`. */
export const prosemirrorSchema: SchemaDefinition = defineSchema(prosemirrorSchemaSpec);

/** Keyed authoring definition used when callers do not provide a schema. */
export const defaultSchemaSpec: SchemaSpec = prosemirrorSchemaSpec;

/** Schema used when callers do not provide one. */
export const defaultSchema: SchemaDefinition = prosemirrorSchema;
