import { serializeEditorTheme } from '../EditorTheme';

describe('EditorTheme serialization', () => {
    it('serializes flat per-level heading styles', () => {
        expect(
            JSON.parse(
                serializeEditorTheme({
                    text: { color: '#112233', fontSize: 16 },
                    h1: { fontSize: 32, fontWeight: '700', marginBottom: 14 },
                    h3: { color: '#445566', lineHeight: 28 },
                    h5: undefined,
                })!
            )
        ).toEqual({
            version: 1,
            styles: {
                text: { color: '#112233ff', fontSize: 16 },
                h1: { fontSize: 32, fontWeight: '700', marginBottom: 14 },
                h3: { color: '#445566ff', lineHeight: 28 },
            },
        });
    });

    it('serializes link decoration resets', () => {
        expect(
            JSON.parse(
                serializeEditorTheme({
                    link: {
                        color: '#445566',
                        backgroundColor: '#eef6ff',
                        textDecorationLine: 'none',
                    },
                })!
            )
        ).toEqual({
            version: 1,
            styles: {
                link: {
                    color: '#445566ff',
                    backgroundColor: '#eef6ffff',
                    textDecorationLine: 'none',
                },
            },
        });
    });

    it('carries the addon mention wire theme alongside element styles', () => {
        expect(
            JSON.parse(
                serializeEditorTheme(
                    { link: { color: '#445566' } },
                    { node: { textColor: '#112233', borderRadius: undefined } }
                )!
            )
        ).toEqual({
            version: 1,
            styles: { link: { color: '#445566ff' } },
            mentions: { node: { style: { color: '#112233ff' } } },
        });
    });

    it('serializes mention-only themes', () => {
        expect(
            JSON.parse(serializeEditorTheme(undefined, { node: { textColor: '#112233' } })!)
        ).toEqual({ version: 1, mentions: { node: { style: { color: '#112233ff' } } } });
    });

    it('omits missing mention themes', () => {
        expect(JSON.parse(serializeEditorTheme({ text: { fontSize: 16 } })!)).toEqual({
            version: 1,
            styles: { text: { fontSize: 16 } },
        });
    });

    it('retains toolbar configuration separately from content styles', () => {
        expect(
            JSON.parse(serializeEditorTheme({ toolbar: { appearance: 'native', height: 44 } })!)
        ).toEqual({ version: 1, toolbar: { appearance: 'native', height: 44 } });
    });
});

it('normalizes rich mention overrides using the same style rules', () => {
    const result = JSON.parse(
        serializeEditorTheme(undefined, {
            node: {
                color: 'red',
                fontSize: 18,
                borderWidth: 2,
                borderLeftWidth: 5,
                borderTopRightRadius: 9,
            },
        })!
    );
    expect(result.mentions.node.style).toEqual({
        color: '#ff0000ff',
        fontSize: 18,
        borderTopWidth: 2,
        borderRightWidth: 2,
        borderBottomWidth: 2,
        borderLeftWidth: 5,
        borderTopRightRadius: 9,
    });
});

it('normalizes legacy mention color aliases consistently', () => {
    const result = JSON.parse(
        serializeEditorTheme(undefined, {
            node: { textColor: 'rebeccapurple', backgroundColor: 'red' },
        })!
    );
    expect(result.mentions.node.style).toMatchObject({
        color: '#663399ff',
        backgroundColor: '#ff0000ff',
    });
});
