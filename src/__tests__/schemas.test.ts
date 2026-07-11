import { acceptingContentSymbols } from '../contentExpression';
import { defaultEmptyDocument, type SchemaDefinition } from '../schemas';

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
});
