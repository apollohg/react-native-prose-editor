import { EditorStyleSheet } from '../EditorStyleSheet';
import { serializeEditorTheme } from '../EditorTheme';

function styles(theme: Parameters<typeof serializeEditorTheme>[0]) {
    return JSON.parse(serializeEditorTheme(theme)!).styles;
}

describe('EditorStyleSheet', () => {
    it('resolves side and corner overrides after composing styles', () => {
        expect(
            styles({
                blockquote: [
                    { borderLeftWidth: 4, borderTopLeftRadius: 0 },
                    { borderWidth: 1, borderRightWidth: 0, borderRadius: 8 },
                ],
            }).blockquote
        ).toEqual({
            borderTopWidth: 1,
            borderRightWidth: 0,
            borderBottomWidth: 1,
            borderLeftWidth: 4,
            borderTopLeftRadius: 0,
            borderTopRightRadius: 8,
            borderBottomLeftRadius: 8,
            borderBottomRightRadius: 8,
        });
    });

    it('supports nested conditional styles without mutating inputs', () => {
        const base = Object.freeze({ padding: 12, marginVertical: 8, color: 'red' });
        const theme = EditorStyleSheet.create({
            paragraph: [base, false, [null, { paddingLeft: 0 }]],
        });
        expect(styles(theme).paragraph).toEqual({
            paddingTop: 12,
            paddingRight: 12,
            paddingBottom: 12,
            paddingLeft: 0,
            marginTop: 8,
            marginBottom: 8,
            color: '#ff0000ff',
        });
        expect(base).toEqual({ padding: 12, marginVertical: 8, color: 'red' });
    });

    it('lets explicit undefined remove a composed override', () => {
        expect(
            styles({ paragraph: [{ color: 'red', marginBottom: 12 }, { color: undefined }] })
        ).toEqual({ paragraph: { marginBottom: 12 } });
    });

    it('preserves inherited values for native resolution', () => {
        expect(styles({ text: { lineHeight: 24 }, paragraph: { fontSize: 16 } })).toEqual({
            text: { lineHeight: 24 },
            paragraph: { fontSize: 16 },
        });
    });

    it('normalizes alpha colors and numeric font weights', () => {
        expect(
            styles({ codeBlock: { color: 'rgba(10, 20, 30, 0.5)', fontWeight: 600 } }).codeBlock
        ).toEqual({ color: '#0a141e80', fontWeight: '600' });
    });

    it('normalizes marker and checkbox state styles', () => {
        expect(
            styles({
                listMarker: { ordered: { schemes: ['decimal', 'lowerRoman'], suffix: ')' } },
                taskCheckbox: { borderWidth: 1, checked: { backgroundColor: 'blue' } },
            })
        ).toEqual({
            listMarker: { ordered: { schemes: ['decimal', 'lowerRoman'], suffix: ')' } },
            taskCheckbox: {
                borderTopWidth: 1,
                borderRightWidth: 1,
                borderBottomWidth: 1,
                borderLeftWidth: 1,
                checked: { backgroundColor: '#0000ffff' },
            },
        });
    });

    it.each([
        [{ paragraph: { padding: -1 } }, 'paragraph.padding'],
        [{ image: { resizeMode: 'repeat' } }, 'image.resizeMode'],
        [{ link: { marginBottom: 4 } }, 'link.marginBottom'],
        [{ paragraph: { color: 'not-a-color' } }, 'paragraph.color'],
        [{ paragraph: { fontSize: Infinity } }, 'paragraph.fontSize'],
        [{ paragraph: { flex: 1 } }, 'paragraph.flex'],
        [{ paragraphs: {} }, 'paragraphs'],
        [{ paragraph: { fontWeight: 650 } }, 'paragraph.fontWeight'],
    ])('rejects invalid styles with their property path', (theme, path) => {
        expect(() => serializeEditorTheme(theme as never)).toThrow(path);
    });

    it('rejects invalid entries hidden behind later array overrides', () => {
        expect(() =>
            serializeEditorTheme({ paragraph: [{ padding: -1 }, { padding: 1 }] })
        ).toThrow('paragraph.padding');
    });

    it('omits an empty theme and preserves separate toolbar settings', () => {
        expect(serializeEditorTheme(undefined)).toBeUndefined();
        expect(serializeEditorTheme({ paragraph: [false, undefined] })).toBeUndefined();
        expect(JSON.parse(serializeEditorTheme({ toolbar: { height: 44 } })!)).toEqual({
            version: 1,
            toolbar: { height: 44 },
        });
    });
});
