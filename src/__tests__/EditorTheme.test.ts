import { serializeEditorTheme } from '../EditorTheme';

describe('EditorTheme', () => {
    it('serializes per-level heading theme overrides', () => {
        const json = serializeEditorTheme({
            text: { color: '#112233', fontSize: 16 },
            headings: {
                h1: { fontSize: 32, fontWeight: '700', spacingAfter: 14 },
                h3: { color: '#445566', lineHeight: 28 },
                h5: undefined,
            },
        });

        expect(json).toBeTruthy();
        expect(JSON.parse(json!)).toEqual({
            text: { color: '#112233', fontSize: 16 },
            headings: {
                h1: { fontSize: 32, fontWeight: '700', spacingAfter: 14 },
                h3: { color: '#445566', lineHeight: 28 },
            },
        });
    });

    it('serializes hyperlink theme overrides', () => {
        const json = serializeEditorTheme({
            links: {
                color: '#445566',
                backgroundColor: '#eef6ff',
                fontWeight: '700',
                fontStyle: 'italic',
                underline: false,
            },
        });

        expect(json).toBeTruthy();
        expect(JSON.parse(json!)).toEqual({
            links: {
                color: '#445566',
                backgroundColor: '#eef6ff',
                fontWeight: '700',
                fontStyle: 'italic',
                underline: false,
            },
        });
    });

    it('serializes toolbar height overrides', () => {
        const json = serializeEditorTheme({
            toolbar: {
                appearance: 'native',
                height: 44,
            },
        });

        expect(json).toBeTruthy();
        expect(JSON.parse(json!)).toEqual({
            toolbar: {
                appearance: 'native',
                height: 44,
            },
        });
    });
});
