import type {
    EditorToolbarItem,
    EditorToolbarTheme,
    MentionSuggestion,
} from '@apollohg/react-native-prose-editor';

export const INITIAL_CONTENT = [
    '<p><strong>Native Editor</strong> example app.</p>',
    '<p>Use this screen to test focus, theme updates, lists, line breaks, toolbar behavior, and optional addons.</p>',
    '<p>Enable mentions above, then type @ after a space, on a blank line, or after punctuation to show native mention suggestions in the toolbar.</p>',
    '<blockquote><p>Blockquotes can wrap one or more blocks and inherit theme styling.</p></blockquote>',
    '<ul><li><p>Try typing</p></li><li><p>Try list indenting</p><ul><li>Multiple levels are supported</li></ul></li></ul>',
    '<p></p>',
].join('');

export const EXAMPLE_MENTION_SUGGESTIONS: readonly MentionSuggestion[] = [
    {
        key: 'alice',
        title: 'Alice Chen',
        label: '@alice',
        attrs: {
            id: 'user_alice',
            entityType: 'user',
            username: 'alice',
            team: 'design',
        },
    },
    {
        key: 'ben',
        title: 'Ben Ortiz',
        label: '@ben',
        attrs: {
            id: 'user_ben',
            entityType: 'user',
            username: 'ben',
            team: 'engineering',
        },
    },
    {
        key: 'chloe',
        title: 'Chloe Park',
        label: '@chloe',
        attrs: {
            id: 'user_chloe',
            entityType: 'user',
            username: 'chloe',
            team: 'product',
        },
    },
    {
        key: 'apollo-team',
        title: 'Apollo Team',
        label: '@apollo-team',
        attrs: {
            id: 'group_apollo',
            entityType: 'group',
            slug: 'apollo-team',
        },
    },
];

export const EXAMPLE_DEFAULT_TOOLBAR_ITEMS: readonly EditorToolbarItem[] = [
    { type: 'mark', mark: 'bold', label: 'Bold', icon: { type: 'default', id: 'bold' } },
    { type: 'mark', mark: 'italic', label: 'Italic', icon: { type: 'default', id: 'italic' } },
    {
        type: 'mark',
        mark: 'underline',
        label: 'Underline',
        icon: { type: 'default', id: 'underline' },
    },
    {
        type: 'mark',
        mark: 'strike',
        label: 'Strikethrough',
        icon: { type: 'default', id: 'strike' },
    },
    { type: 'link', label: 'Link', icon: { type: 'default', id: 'link' } },
    { type: 'blockquote', label: 'Blockquote', icon: { type: 'default', id: 'blockquote' } },
    { type: 'separator' },
    {
        type: 'list',
        listType: 'bulletList',
        label: 'Bullet List',
        icon: { type: 'default', id: 'bulletList' },
    },
    {
        type: 'list',
        listType: 'orderedList',
        label: 'Ordered List',
        icon: { type: 'default', id: 'orderedList' },
    },
    {
        type: 'command',
        command: 'indentList',
        label: 'Indent List',
        icon: { type: 'default', id: 'indentList' },
    },
    {
        type: 'command',
        command: 'outdentList',
        label: 'Outdent List',
        icon: { type: 'default', id: 'outdentList' },
    },
    {
        type: 'node',
        nodeType: 'hardBreak',
        label: 'Line Break',
        icon: { type: 'default', id: 'lineBreak' },
    },
    {
        type: 'node',
        nodeType: 'horizontalRule',
        label: 'Horizontal Rule',
        icon: { type: 'default', id: 'horizontalRule' },
    },
    { type: 'separator' },
    { type: 'command', command: 'undo', label: 'Undo', icon: { type: 'default', id: 'undo' } },
    { type: 'command', command: 'redo', label: 'Redo', icon: { type: 'default', id: 'redo' } },
];

export type ToolbarColorKey = Exclude<
    keyof Required<EditorToolbarTheme>,
    | 'appearance'
    | 'borderRadius'
    | 'borderWidth'
    | 'buttonBorderRadius'
    | 'keyboardOffset'
    | 'horizontalInset'
>;

export const TOOLBAR_COLOR_FIELDS: Array<{ key: ToolbarColorKey; label: string }> = [
    { key: 'backgroundColor', label: 'Background' },
    { key: 'borderColor', label: 'Border' },
    { key: 'separatorColor', label: 'Separator' },
    { key: 'buttonColor', label: 'Button' },
    { key: 'buttonActiveColor', label: 'Button Active' },
    { key: 'buttonDisabledColor', label: 'Button Disabled' },
    { key: 'buttonActiveBackgroundColor', label: 'Active Fill' },
];
