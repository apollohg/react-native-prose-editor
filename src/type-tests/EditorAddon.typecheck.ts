import { createMentionsAddon, type EditorAddons, type CodeHighlightingAddon } from '../EditorAddon';
import type { RichTextEditorProps } from '../RichTextEditorTypes';
import type { RichTextViewerProps } from '../NativeProseViewer';

const mentions = createMentionsAddon({
    suggestions: [{ key: 'alice', title: 'Alice' }],
    onQueryChange: (event) => void event.query,
    onPress: (event) => void event.docPos,
});
const highlighting: CodeHighlightingAddon = {
    id: 'code-highlighting',
    version: 1,
    capability: 'code-highlighting',
    options: { provider: 'syntect', theme: 'base16-ocean.dark' },
};
const addons: EditorAddons = [false, null, undefined, mentions, highlighting] as const;
const editorAddons: RichTextEditorProps['addons'] = addons;
const viewerAddons: RichTextViewerProps['addons'] = addons;
void editorAddons;
void viewerAddons;

// @ts-expect-error Addon arrays are readonly.
addons.push(mentions);
// @ts-expect-error Nested addon arrays are unsupported.
const nested: EditorAddons = [[mentions]];
// @ts-expect-error The former keyed addon object is unsupported.
const keyed: EditorAddons = { mentions: {} };
// @ts-expect-error Descriptor versions are explicit.
const incompatible: CodeHighlightingAddon = { ...highlighting, version: 2 };
void nested;
void keyed;
void incompatible;
