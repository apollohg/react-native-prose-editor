import { acceptingContentSymbols } from '../contentExpression';
import * as schemaExports from '../schemas';
import {
    buildDocumentFragmentJson,
    defaultSchema,
    defineSchema,
    defaultEmptyDocument,
    normalizeDocumentJson,
    prosemirrorSchema,
    resolveDocumentDescriptor,
    resolveDocumentSchema,
    tiptapCompatibleSchema,
    tiptapCompatibleSchemaSpec,
    type NodeSpec,
    type SchemaDefinition,
} from '../schemas';
import { buildImageFragmentJson } from '../schemas';
import { buildMentionFragmentJson } from '../addons';

const GROUPED_RANGE_SCHEMA: SchemaDefinition = {
    nodes: [
        { name: 'doc', content: '(title | image){1}', role: 'doc' },
        { name: 'title', content: 'inline*', group: 'block', role: 'textBlock' },
        { name: 'paragraph', content: 'inline*', group: 'block', role: 'textBlock' },
        { name: 'image', content: '', group: 'block', role: 'block', isVoid: true },
        { name: 'text', content: '', group: 'inline', role: 'text' },
    ],
    marks: [],
};

const COUNTER_NODE: NodeSpec = {
    name: 'counterCard',
    content: '',
    group: 'block',
    role: 'block',
    isVoid: true,
    attrs: { title: { default: '' } },
    html: {
        tag: 'div',
        staticAttrs: { 'data-type': 'counter-card' },
        attrMap: { title: 'data-title' },
    },
};

describe('defineSchema', () => {
    it('includes declarative HTML rules in compiled node specs', () => {
        const schema = defineSchema({
            nodes: {
                doc: { content: 'block+' },
                counterCard: {
                    content: '',
                    group: 'block',
                    role: 'block',
                    isVoid: true,
                    attrs: COUNTER_NODE.attrs,
                    html: COUNTER_NODE.html,
                },
                text: { role: 'text' },
            },
        });

        expect(schema.nodes.find((node) => node.name === 'counterCard')?.html).toEqual(
            COUNTER_NODE.html
        );
    });

    it('uses ProseMirror naming by default and retains a Tiptap-compatible preset', () => {
        const exports = schemaExports as unknown as Record<string, unknown>;
        const defaultSchema = exports.defaultSchema as SchemaDefinition | undefined;
        const tiptapCompatibleSchema = exports.tiptapCompatibleSchema as
            | SchemaDefinition
            | undefined;
        const defaultSchemaSpec = exports.defaultSchemaSpec as SchemaSpec | undefined;
        const prosemirrorSchemaSpec = exports.prosemirrorSchemaSpec as SchemaSpec | undefined;

        expect(defaultSchema).toBe(prosemirrorSchema);
        expect(defaultSchemaSpec).toBe(prosemirrorSchemaSpec);
        expect(defaultSchemaSpec?.nodes.bullet_list.content).toBe('list_item+');
        expect(defaultSchema?.nodes.some((node) => node.name === 'bullet_list')).toBe(true);
        expect(tiptapCompatibleSchema?.nodes.some((node) => node.name === 'bulletList')).toBe(true);
        expect(exports.tiptapSchema).toBeUndefined();
    });

    it('exposes the Tiptap-compatible schema as a keyed authoring definition', () => {
        expect(tiptapCompatibleSchemaSpec.nodes.heading).toMatchObject({
            content: 'inline*',
            group: 'block',
            role: 'heading',
            attrs: { level: { default: 1 } },
        });
        expect(tiptapCompatibleSchema.nodes.find((node) => node.name === 'h2')?.json).toEqual({
            type: 'heading',
            attrs: { level: 2 },
        });
        expect(prosemirrorSchema.nodes.find((node) => node.name === 'h3')?.json).toEqual({
            type: 'heading',
            attrs: { level: 3 },
        });
    });

    it('compiles keyed static DOM specs into the native schema representation', () => {
        expect(
            defineSchema({
                nodes: {
                    doc: { content: 'block+' },
                    paragraph: {
                        content: 'inline*',
                        group: 'block',
                        parseDOM: [{ tag: 'p' }],
                        toDOM: ['p', 0],
                    },
                    text: { group: 'inline' },
                },
            })
        ).toEqual({
            nodes: [
                { name: 'doc', content: 'block+', role: 'doc' },
                {
                    name: 'paragraph',
                    content: 'inline*',
                    group: 'block',
                    role: 'textBlock',
                    htmlTag: 'p',
                },
                { name: 'text', content: '', group: 'inline', role: 'text' },
            ],
            marks: [],
        });
    });

    it('compiles an attribute-driven heading into native variants with JSON projections', () => {
        const schema = defineSchema({
            nodes: {
                doc: { content: 'block+', role: 'doc' },
                heading: {
                    content: 'inline*',
                    group: 'block',
                    role: 'heading',
                    attrs: { level: { default: 1 } },
                    parseDOM: [
                        { tag: 'h1', attrs: { level: 1 } },
                        { tag: 'h2', attrs: { level: 2 } },
                    ],
                    toDOM: {
                        switchOn: 'level',
                        cases: {
                            1: ['h1', 0],
                            2: ['h2', 0],
                        },
                    },
                },
                text: { group: 'inline', role: 'text' },
            },
        });
        expect(schema.nodes).toEqual([
            { name: 'doc', content: 'block+', role: 'doc' },
            {
                name: 'h1',
                content: 'inline*',
                group: 'block heading',
                role: 'textBlock',
                htmlTag: 'h1',
                json: { type: 'heading', attrs: { level: 1 } },
            },
            {
                name: 'h2',
                content: 'inline*',
                group: 'block heading',
                role: 'textBlock',
                htmlTag: 'h2',
                json: { type: 'heading', attrs: { level: 2 } },
            },
            { name: 'text', content: '', group: 'inline', role: 'text' },
        ]);
        expect(resolveDocumentSchema(schema).nodes[1].json).toEqual({
            type: 'heading',
            attrs: { level: 1 },
        });
    });

    it('rejects an attribute-driven DOM spec without the discriminating attribute', () => {
        expect(() =>
            defineSchema({
                nodes: {
                    doc: { content: 'block+', role: 'doc' },
                    callout: {
                        group: 'block',
                        role: 'block',
                        toDOM: {
                            switchOn: 'tone',
                            cases: { info: ['aside'] },
                        },
                    },
                    text: { group: 'inline', role: 'text' },
                },
            })
        ).toThrow("node 'callout' switches on undeclared attribute 'tone'");
    });

    it('rejects static DOM rules the native schema cannot represent', () => {
        expect(() =>
            defineSchema({
                nodes: {
                    doc: { content: 'block+', role: 'doc' },
                    paragraph: {
                        content: 'inline*',
                        group: 'block',
                        role: 'textBlock',
                        parseDOM: [{ tag: 'p' }, { tag: 'div' }],
                        toDOM: ['p', 0],
                    },
                    text: { group: 'inline', role: 'text' },
                },
            })
        ).toThrow("node 'paragraph' has multiple static DOM parse rules");

        expect(() =>
            defineSchema({
                nodes: {
                    doc: { content: 'text*', role: 'doc' },
                    text: { group: 'inline', role: 'text' },
                },
                marks: {
                    bold: { parseDOM: [{ tag: 'strong' }], toDOM: ['b', 0] },
                },
            })
        ).toThrow("mark 'bold' parses 'strong' but serializes 'b'");
    });

    it('rejects unsafe mark DOM tags at definition time', () => {
        expect(() =>
            defineSchema({
                nodes: {
                    doc: { content: 'text*', role: 'doc' },
                    text: { group: 'inline', role: 'text' },
                },
                marks: {
                    unsafe: { toDOM: ['script'] },
                },
            })
        ).toThrow("mark 'unsafe' has disallowed HTML tag 'script'");
    });

    it('requires attribute-driven parse rules to map one-to-one with output cases', () => {
        expect(() =>
            defineSchema({
                nodes: {
                    doc: { content: 'block+', role: 'doc' },
                    heading: {
                        content: 'inline*',
                        group: 'block',
                        role: 'heading',
                        attrs: { level: { default: 1 } },
                        parseDOM: [
                            { tag: 'h1', attrs: { level: 1 } },
                            { tag: 'h2', attrs: { level: 2 } },
                        ],
                        toDOM: {
                            switchOn: 'level',
                            cases: { 1: ['h1', 0] },
                        },
                    },
                    text: { group: 'inline', role: 'text' },
                },
            })
        ).toThrow("node 'heading' DOM parse rules must map one-to-one with 'level' output cases");
    });

    it('derives discriminator types from the declared attribute default', () => {
        const schema = defineSchema({
            nodes: {
                doc: { content: 'badge', role: 'doc' },
                badge: {
                    group: 'block',
                    attrs: { value: { default: '1' } },
                    toDOM: { switchOn: 'value', cases: { 1: ['badge-one'] } },
                },
                text: { group: 'inline', role: 'text' },
            },
        });
        expect(schema.nodes[1]?.json).toEqual({
            type: 'badge',
            attrs: { value: '1' },
        });
    });

    it('rejects parse rules whose discriminator type conflicts with its default', () => {
        expect(() =>
            defineSchema({
                nodes: {
                    doc: { content: 'heading', role: 'doc' },
                    heading: {
                        content: 'inline*',
                        group: 'block',
                        attrs: { level: { default: 1 } },
                        parseDOM: [{ tag: 'h1', attrs: { level: '1' } }],
                        toDOM: { switchOn: 'level', cases: { 1: ['h1', 0] } },
                    },
                    text: { group: 'inline', role: 'text' },
                },
            })
        ).toThrow("node 'heading' DOM discriminator 'level' must use number values");
    });

    it('rejects an attribute switch whose discriminator type cannot be inferred', () => {
        expect(() =>
            defineSchema({
                nodes: {
                    doc: { content: 'callout', role: 'doc' },
                    callout: {
                        attrs: { tone: {} },
                        toDOM: {
                            switchOn: 'tone',
                            cases: { info: ['aside-info'], warning: ['aside-warning'] },
                        },
                    },
                    text: { group: 'inline', role: 'text' },
                },
            })
        ).toThrow("node 'callout' DOM discriminator 'tone' must have a scalar type");
    });
});

describe('defaultEmptyDocument', () => {
    it('uses the public JSON projection for constructed nodes', () => {
        const schema = defineSchema({
            nodes: {
                doc: { content: 'heading', role: 'doc' },
                heading: {
                    content: 'inline*',
                    group: 'block',
                    role: 'heading',
                    attrs: { level: { default: 1 } },
                    toDOM: { switchOn: 'level', cases: { 1: ['h1', 0] } },
                },
                text: { group: 'inline', role: 'text' },
            },
        });
        expect(defaultEmptyDocument(schema)).toEqual({
            type: 'doc',
            content: [{ type: 'heading', attrs: { level: 1 } }],
        });
    });

    it('uses grouped alternatives and ranges when selecting the first text block', () => {
        expect(defaultEmptyDocument(GROUPED_RANGE_SCHEMA)).toEqual({
            type: 'doc',
            content: [{ type: 'title' }],
        });
    });

    it('matches sequence branches and every supported quantifier', () => {
        const expression = '(title paragraph | image caption) note? tag* end+ item{2} tail{1,2}';
        expect(acceptingContentSymbols(expression, ['title'])).toEqual(['paragraph']);
        expect(acceptingContentSymbols(expression, ['image'])).toEqual(['caption']);
        expect(acceptingContentSymbols('a{2,}', ['a', 'a'])).toEqual(['a']);
    });

    it('fails closed for excessive nesting and compiled state counts', () => {
        const deeplyNested = `${'('.repeat(129)}a${')'.repeat(129)}`;
        expect(acceptingContentSymbols(deeplyNested)).toEqual([]);
        expect(acceptingContentSymbols('(a{101}){101}')).toEqual([]);
    });

    it('constructs every node in the minimal accepted document sequence', () => {
        const schema: SchemaDefinition = {
            nodes: [
                { name: 'doc', content: 'title image', role: 'doc' },
                { name: 'title', content: 'inline*', group: 'block', role: 'textBlock' },
                { name: 'image', content: '', group: 'block', role: 'block', isVoid: true },
                { name: 'text', content: '', group: 'inline', role: 'text' },
            ],
            marks: [],
        };
        expect(defaultEmptyDocument(schema)).toEqual({
            type: 'doc',
            content: [{ type: 'title' }, { type: 'image' }],
        });
    });

    it('uses a valid non-text default instead of an unaccepted paragraph fallback', () => {
        const schema: SchemaDefinition = {
            nodes: [
                { name: 'doc', content: 'image', role: 'doc' },
                { name: 'image', content: '', group: 'block', role: 'block', isVoid: true },
                { name: 'text', content: '', group: 'inline', role: 'text' },
            ],
            marks: [],
        };
        expect(defaultEmptyDocument(schema)).toEqual({
            type: 'doc',
            content: [{ type: 'image' }],
        });
    });

    it('throws a clear error when required content cannot be default-constructed', () => {
        const schema: SchemaDefinition = {
            nodes: [
                { name: 'doc', content: 'image', role: 'doc' },
                { name: 'image', content: '', attrs: { src: {} }, role: 'block', isVoid: true },
                { name: 'text', content: '', role: 'text' },
            ],
            marks: [],
        };
        expect(() => defaultEmptyDocument(schema)).toThrow(
            "schema cannot construct a default document for 'doc'"
        );
    });

    it('rejects default construction deeper than the shared limit', () => {
        const nodes: SchemaDefinition['nodes'] = [
            { name: 'doc', content: 'n0', role: 'doc' },
            ...Array.from({ length: 129 }, (_, index) => ({
                name: `n${index}`,
                content: index === 128 ? '' : `n${index + 1}`,
                role: 'block',
            })),
            { name: 'text', content: '', role: 'text' },
        ];
        expect(() => defaultEmptyDocument({ nodes, marks: [] })).toThrow(
            "schema cannot construct a default document for 'doc'"
        );
    });

    it('treats an explicit undefined default as missing like serialized JSON', () => {
        const schema: SchemaDefinition = {
            nodes: [
                { name: 'doc', content: 'image', role: 'doc' },
                {
                    name: 'image',
                    content: '',
                    attrs: { src: { default: undefined } },
                    role: 'block',
                },
                { name: 'text', content: '', role: 'text' },
            ],
            marks: [],
        };
        expect(() => defaultEmptyDocument(schema)).toThrow();
    });
});

describe('schema-aware document normalization', () => {
    const articleSchema: SchemaDefinition = {
        nodes: [
            { name: 'article', content: 'title+', role: 'doc' },
            { name: 'title', content: 'inline*', group: 'block', role: 'textBlock' },
            { name: 'words', content: '', group: 'inline', role: 'text' },
        ],
        marks: [],
    };

    it('normalizes an empty custom doc-role root without changing its type', () => {
        expect(normalizeDocumentJson({ type: 'article', content: [] }, articleSchema)).toEqual({
            type: 'article',
            content: [{ type: 'title' }],
        });
    });

    it('does not treat a literal doc node as the root of a custom schema', () => {
        const foreign = { type: 'doc', content: [] };
        expect(normalizeDocumentJson(foreign, articleSchema)).toBe(foreign);
    });

    it('never selects a non-root node named doc before the document-role node', () => {
        const schema: SchemaDefinition = {
            nodes: [
                { name: 'doc', content: '', role: 'block' },
                { name: 'article', content: 'paragraph', role: 'doc' },
                { name: 'paragraph', content: '', role: 'textBlock' },
                { name: 'text', content: '', role: 'text' },
            ],
            marks: [],
        };

        expect(defaultEmptyDocument(resolveDocumentSchema(schema))).toEqual({
            type: 'article',
            content: [{ type: 'paragraph' }],
        });
    });

    it('falls back to the default schema for schemas native would reject', () => {
        const empty = { nodes: [], marks: [] } as SchemaDefinition;
        const unconstructible: SchemaDefinition = {
            nodes: [
                { name: 'article', content: 'image', role: 'doc' },
                { name: 'image', content: '', attrs: { src: {} }, role: 'block' },
                { name: 'text', content: '', role: 'text' },
            ],
            marks: [],
        };
        expect(resolveDocumentSchema(empty)).toBe(defaultSchema);
        expect(resolveDocumentSchema(unconstructible)).toBe(defaultSchema);
        expect(
            resolveDocumentSchema({
                nodes: [
                    { name: 'doc', content: 'paragraph missing?', role: 'doc' },
                    { name: 'paragraph', content: '', role: 'textBlock' },
                    { name: 'text', content: '', role: 'text' },
                ],
                marks: [],
            })
        ).toBe(defaultSchema);
        expect(
            resolveDocumentSchema({
                nodes: [
                    { name: 'doc', content: 'paragraph', role: 'doc' },
                    { name: 'paragraph', content: '', role: 'textBlock' },
                    { name: 'caption', content: 'text+', role: 'block' },
                    { name: 'text', content: '', role: 'text' },
                ],
                marks: [],
            })
        ).toBe(defaultSchema);
        expect(normalizeDocumentJson({ type: 'doc', content: [] }, empty)).toEqual({
            type: 'doc',
            content: [{ type: 'paragraph' }],
        });
    });

    it('retains a valid custom schema', () => {
        expect(resolveDocumentSchema(articleSchema)).toMatchObject(articleSchema);
    });

    it('retains declarative HTML rules during schema resolution', () => {
        const resolved = resolveDocumentSchema({
            ...defaultSchema,
            nodes: [...defaultSchema.nodes, COUNTER_NODE],
        });

        expect(resolved.nodes.find((node) => node.name === 'counterCard')?.html).toEqual(
            COUNTER_NODE.html
        );
    });

    it('does not fall back when a custom node has valid declarative HTML rules', () => {
        const resolved = resolveDocumentSchema({
            ...defaultSchema,
            nodes: [...defaultSchema.nodes, COUNTER_NODE],
        });

        expect(resolved).not.toBe(defaultSchema);
        expect(resolved.nodes.some((node) => node.name === 'counterCard')).toBe(true);
    });

    it('applies the atom identifier policy to declarative HTML rules', () => {
        const unsafe = {
            ...COUNTER_NODE,
            html: { ...COUNTER_NODE.html!, tag: 'img' },
        };
        const resolved = resolveDocumentSchema({
            ...defaultSchema,
            nodes: [...defaultSchema.nodes, unsafe],
        });

        expect(resolved).toBe(defaultSchema);
        expect(resolved.nodes.some((node) => node.name === 'counterCard')).toBe(false);
    });

    it('matches native projection ambiguity validation', () => {
        const invalid: SchemaDefinition = {
            nodes: [
                { name: 'doc', content: 'block+', role: 'doc' },
                {
                    name: 'note',
                    content: '',
                    group: 'block',
                    role: 'block',
                    json: { type: 'callout', attrs: { tone: 'info' } },
                },
                {
                    name: 'compact-note',
                    content: '',
                    group: 'block',
                    role: 'block',
                    json: { type: 'callout', attrs: { tone: 'info', size: 'compact' } },
                },
                { name: 'text', content: '', role: 'text' },
            ],
            marks: [],
        };

        expect(resolveDocumentSchema(invalid)).toBe(defaultSchema);
    });

    it('rejects reserved wire sentinel names as public projection types', () => {
        for (const type of ['__opaque', '__opaque_json', '__skip']) {
            const invalid: SchemaDefinition = {
                nodes: [
                    { name: 'doc', content: 'block+', role: 'doc' },
                    {
                        name: 'note',
                        content: '',
                        group: 'block',
                        role: 'block',
                        json: { type, attrs: { tone: 'info' } },
                    },
                    { name: 'text', content: '', role: 'text' },
                ],
                marks: [],
            };

            expect(resolveDocumentSchema(invalid)).toBe(defaultSchema);
        }
    });

    it('rejects projections shadowed by legacy heading normalization', () => {
        const projectedHeading = (level: number | string): NodeSpec => ({
            name: 'infoBox',
            content: 'inline*',
            group: 'block',
            role: 'textBlock',
            json: { type: 'heading', attrs: { level } },
        });
        const valid: SchemaDefinition = {
            nodes: [
                { name: 'doc', content: 'block+', role: 'doc' },
                projectedHeading(2),
                { name: 'text', content: '', group: 'inline', role: 'text' },
            ],
            marks: [],
        };

        expect(resolveDocumentSchema(valid)).not.toBe(defaultSchema);
        for (const level of [2, 2.0, '2', '+2']) {
            const invalid: SchemaDefinition = {
                nodes: [
                    { name: 'doc', content: 'block+', role: 'doc' },
                    {
                        name: 'h2',
                        content: 'inline*',
                        group: 'block',
                        role: 'textBlock',
                    },
                    projectedHeading(level),
                    { name: 'text', content: '', group: 'inline', role: 'text' },
                ],
                marks: [],
            };
            expect(resolveDocumentSchema(invalid)).toBe(defaultSchema);
        }
    });

    it('matches native by treating a non-array marks field as an empty mark list', () => {
        const malformedMarks = { ...articleSchema, marks: {} } as unknown as SchemaDefinition;
        const resolved = resolveDocumentSchema(malformedMarks);

        expect(resolved.nodes[0].name).toBe('article');
        expect(resolved.marks).toEqual([]);
        expect(resolveDocumentDescriptor(malformedMarks).documentNodeName).toBe('article');
    });

    it('exposes approved custom mark HTML tags and rejects executable tags', () => {
        const approved: SchemaDefinition = {
            ...articleSchema,
            marks: [{ name: 'highlight', htmlTag: 'mark' }],
        };
        expect(resolveDocumentSchema(approved)).toMatchObject(approved);

        expect(
            resolveDocumentSchema({
                ...articleSchema,
                marks: [{ name: 'danger', htmlTag: 'script' }],
            } as unknown as SchemaDefinition)
        ).toBe(defaultSchema);
    });

    it('matches native schema defaults, tag normalization, and identifier validation', () => {
        const schema = {
            nodes: [
                { name: 'article', content: 'paragraph', role: 'doc' },
                { name: 'paragraph', group: 'block', role: 'textBlock', htmlTag: 'p' },
                { name: 'text', role: 'text' },
            ],
            marks: [{ name: 'highlight', htmlTag: 'MARK' }],
        } as unknown as SchemaDefinition;
        const resolved = resolveDocumentSchema(schema);

        expect(resolved).not.toBe(schema);
        expect(resolved.nodes[1]).toMatchObject({ content: '', isVoid: false });
        expect(resolved.marks[0].htmlTag).toBe('mark');

        for (const invalid of [
            { ...schema, nodes: [{ ...schema.nodes[0], htmlTag: 'P' }, ...schema.nodes.slice(1)] },
            {
                ...schema,
                nodes: [
                    schema.nodes[0],
                    { ...schema.nodes[1], attrs: { 'on load': {} } },
                    schema.nodes[2],
                ],
            },
            {
                ...schema,
                marks: [{ name: 'highlight', attrs: { 'href\" onclick': {} } }],
            },
        ] as SchemaDefinition[]) {
            expect(resolveDocumentSchema(invalid)).toBe(defaultSchema);
        }
    });

    it('matches native by treating non-object attrs and non-string groups as absent', () => {
        const schema = {
            nodes: [
                { name: 'article', content: 'paragraph', role: 'doc', group: 7 },
                { name: 'paragraph', role: 'textBlock', attrs: [] },
                { name: 'text', role: 'text' },
            ],
            marks: [{ name: 'highlight', attrs: 'ignored' }],
        } as unknown as SchemaDefinition;

        const resolved = resolveDocumentSchema(schema);
        expect(resolved.nodes[0].name).toBe('article');
        expect(resolved.nodes[0].group).toBeUndefined();
        expect(resolved.nodes[1].attrs).toBeUndefined();
        expect(resolved.marks[0].attrs).toBeUndefined();
    });

    it('resolves one custom-root descriptor for empty documents and fragments', () => {
        const descriptor = resolveDocumentDescriptor(articleSchema);

        expect(descriptor).toMatchObject({
            schema: articleSchema,
            documentNodeName: 'article',
            emptyDocument: { type: 'article', content: [{ type: 'title' }] },
        });
        expect(buildDocumentFragmentJson([{ type: 'title' }], descriptor)).toEqual({
            type: 'article',
            content: [{ type: 'title' }],
        });
        expect(buildImageFragmentJson({ src: 'https://example.test/a.png' }, descriptor).type).toBe(
            'article'
        );
        expect(buildMentionFragmentJson({ label: '@a' }, descriptor).type).toBe('article');
    });

    it('rejects schema node and expression work before resolving a descriptor', () => {
        const tooManyNodes: SchemaDefinition = {
            nodes: [
                { name: 'article', content: 'title', role: 'doc' },
                { name: 'title', content: '', role: 'textBlock' },
                { name: 'text', content: '', role: 'text' },
            ],
            marks: [],
        };

        expect(() =>
            resolveDocumentDescriptor(tooManyNodes, {
                maxSchemaNodes: 2,
                maxSchemaExpressionBytes: 1024,
            })
        ).toThrow(expect.objectContaining({ code: 'SCHEMA_INVALID', limit: 2, actual: 3 }));

        expect(() =>
            resolveDocumentDescriptor(articleSchema, {
                maxSchemaNodes: 10,
                maxSchemaExpressionBytes: 5,
            })
        ).toThrow(expect.objectContaining({ code: 'SCHEMA_INVALID', limit: 5 }));
    });

    it('does not reinterpret node and expression limits as mark, group, or attr caps', () => {
        const minimalNodes: SchemaDefinition['nodes'] = [
            { name: 'doc', content: '', role: 'doc' },
            { name: 'text', content: '', role: 'text' },
        ];
        const limits = { maxSchemaNodes: 2, maxSchemaExpressionBytes: 16 };

        expect(() =>
            resolveDocumentDescriptor(
                {
                    nodes: [
                        {
                            ...minimalNodes[0],
                            content: ' '.repeat(16),
                            group: 'a b c',
                            attrs: {
                                a: { default: null },
                                b: { default: null },
                                c: { default: null },
                            },
                        },
                        minimalNodes[1],
                    ],
                    marks: [{ name: 'a' }, { name: 'b' }, { name: 'c' }],
                },
                limits
            )
        ).not.toThrow();
    });

    it.each([
        [
            'marks',
            {
                marks: Array.from({ length: 700 }, (_, index) => ({ name: `m${index}` })),
            },
        ],
        [
            'group work',
            {
                group: 'a '.repeat(700),
            },
        ],
        [
            'attrs',
            {
                attrs: Object.fromEntries(
                    Array.from({ length: 700 }, (_, index) => [`a${index}`, { default: null }])
                ),
            },
        ],
    ])('rejects excessive %s through the derived schema work budget', (_name, hostile) => {
        const nodes: SchemaDefinition['nodes'] = [
            {
                name: 'doc',
                content: '',
                role: 'doc',
                ...('group' in hostile ? { group: hostile.group } : {}),
                ...('attrs' in hostile ? { attrs: hostile.attrs } : {}),
            },
            { name: 'text', content: '', role: 'text' },
        ];

        expect(() =>
            resolveDocumentDescriptor(
                {
                    nodes,
                    marks: 'marks' in hostile ? hostile.marks : [],
                },
                { maxSchemaNodes: 2, maxSchemaExpressionBytes: 16 }
            )
        ).toThrow(expect.objectContaining({ code: 'SCHEMA_INVALID', limit: 640 }));
    });

    it('bounds hostile group scanning for direct defaultEmptyDocument callers', () => {
        const schema: SchemaDefinition = {
            nodes: [
                { name: 'doc', content: '', group: 'a'.repeat(700), role: 'doc' },
                { name: 'text', content: '', role: 'text' },
            ],
            marks: [],
        };
        expect(() =>
            defaultEmptyDocument(schema, {
                maxSchemaNodes: 2,
                maxSchemaExpressionBytes: 16,
            })
        ).toThrow(expect.objectContaining({ code: 'SCHEMA_INVALID', limit: 640 }));

        if (false) {
            // @ts-expect-error the public API must not expose an admission-bypass argument
            defaultEmptyDocument(schema, undefined, true);
        }
    });

    it('charges projection scalar payloads to the schema work budget', () => {
        const schema: SchemaDefinition = {
            nodes: [
                {
                    name: 'article',
                    content: '',
                    role: 'doc',
                    json: { type: 'x'.repeat(700) },
                },
                { name: 'text', content: '', role: 'text' },
            ],
            marks: [],
        };

        expect(() =>
            resolveDocumentDescriptor(schema, {
                maxSchemaNodes: 2,
                maxSchemaExpressionBytes: 16,
            })
        ).toThrow(expect.objectContaining({ code: 'SCHEMA_INVALID' }));
    });

    it('accepts schema collections at their exact public limits', () => {
        expect(() =>
            resolveDocumentDescriptor(
                {
                    nodes: [
                        {
                            name: 'doc',
                            content: ' '.repeat(16),
                            group: 'a b',
                            role: 'doc',
                            attrs: { a: { default: null }, b: { default: null } },
                        },
                        { name: 'text', content: '', role: 'text' },
                    ],
                    marks: [{ name: 'a' }, { name: 'b' }, { name: 'c' }],
                },
                { maxSchemaNodes: 2, maxSchemaExpressionBytes: 16 }
            )
        ).not.toThrow();
    });
});
