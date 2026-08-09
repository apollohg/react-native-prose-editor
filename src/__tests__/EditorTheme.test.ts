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

    it('carries the addon mention theme into the native theme payload', () => {
        const json = serializeEditorTheme(
            { links: { color: '#445566' } },
            { textColor: '#112233', backgroundColor: '#DDEEFF', borderRadius: undefined }
        );

        expect(json).toBeTruthy();
        expect(JSON.parse(json!)).toEqual({
            links: { color: '#445566' },
            mentions: { textColor: '#112233', backgroundColor: '#DDEEFF' },
        });
    });

    it('serializes the addon mention theme when no editor theme is supplied', () => {
        const json = serializeEditorTheme(undefined, { textColor: '#112233' });

        expect(json).toBeTruthy();
        expect(JSON.parse(json!)).toEqual({ mentions: { textColor: '#112233' } });
    });

    it('omits mentions when the addon supplies no mention theme', () => {
        const json = serializeEditorTheme({ links: { color: '#445566' } });

        expect(JSON.parse(json!)).toEqual({ links: { color: '#445566' } });
    });

    it('serializes list layout spacing', () => {
        const json = serializeEditorTheme({
            list: { indent: 28, markerGap: 12, spacingAfter: 20, markerScale: undefined },
        });

        expect(json).toBeTruthy();
        expect(JSON.parse(json!)).toEqual({
            list: { indent: 28, markerGap: 12, spacingAfter: 20 },
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
