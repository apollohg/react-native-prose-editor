import { EditorStyleSheet } from '../EditorStyleSheet';
import type { EditorTheme } from '../EditorTheme';

const styles = EditorStyleSheet.create({
    paragraph: [{ fontWeight: 600, padding: 12 }, false, [undefined, { marginBottom: 0 }]],
    image: { resizeMode: 'contain', borderTopLeftRadius: 8 },
    link: { color: 'rebeccapurple', textDecorationLine: 'underline' },
});

const theme: EditorTheme = styles;
void theme;

// @ts-expect-error Unknown element.
EditorStyleSheet.create({ paragraphs: { color: 'red' } });
// @ts-expect-error Inline links have no box margins.
EditorStyleSheet.create({ link: { color: 'red', marginBottom: 12 } });
// @ts-expect-error Unsupported image fit.
EditorStyleSheet.create({ image: { resizeMode: 'repeat' } });
// @ts-expect-error Unsupported layout property, even alongside valid properties.
EditorStyleSheet.create({ paragraph: [{ fontSize: 16, flex: 1 }] });
// @ts-expect-error Legacy link spelling.
EditorStyleSheet.create({ links: { color: 'red' } });
// @ts-expect-error Unsupported nested checkbox property.
EditorStyleSheet.create({ taskCheckbox: { checked: { backgroundColor: 'red', flex: 1 } } });
// @ts-expect-error Unsupported nested ordered marker property.
EditorStyleSheet.create({ listMarker: { ordered: { suffix: '.', extra: true } } });
// @ts-expect-error Use a supported numeric or string weight.
EditorStyleSheet.create({ paragraph: { fontWeight: 'semibold' } });
