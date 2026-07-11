import { acceptingContentSymbols } from '../contentExpression';
import {
    defaultEmptyDocument,
    normalizeDocumentJson,
    resolveDocumentSchema,
    tiptapSchema,
    type SchemaDefinition,
} from '../schemas';

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

describe('defaultEmptyDocument', () => {
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

    it('falls back to Tiptap for schemas native would reject', () => {
        const empty = { nodes: [], marks: [] } as SchemaDefinition;
        const unconstructible: SchemaDefinition = {
            nodes: [
                { name: 'article', content: 'image', role: 'doc' },
                { name: 'image', content: '', attrs: { src: {} }, role: 'block' },
                { name: 'text', content: '', role: 'text' },
            ],
            marks: [],
        };
        expect(resolveDocumentSchema(empty)).toBe(tiptapSchema);
        expect(resolveDocumentSchema(unconstructible)).toBe(tiptapSchema);
        expect(
            resolveDocumentSchema({
                nodes: [
                    { name: 'doc', content: 'paragraph missing?', role: 'doc' },
                    { name: 'paragraph', content: '', role: 'textBlock' },
                    { name: 'text', content: '', role: 'text' },
                ],
                marks: [],
            })
        ).toBe(tiptapSchema);
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
        ).toBe(tiptapSchema);
        expect(normalizeDocumentJson({ type: 'doc', content: [] }, empty)).toEqual({
            type: 'doc',
            content: [{ type: 'paragraph' }],
        });
    });

    it('retains a valid custom schema', () => {
        expect(resolveDocumentSchema(articleSchema)).toBe(articleSchema);
    });
});
