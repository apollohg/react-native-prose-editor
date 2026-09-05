import type { EditorToolbarItem, MentionSuggestion } from 'react-native-rich-text-editor';

export const APP_TITLE = 'React Native Editor';

export const EDITOR_PLACEHOLDER = 'Start writing…';

export const MENTION_TRIGGER = '@';

/** Remote image used by the initial document. */
export const SAMPLE_IMAGE_URL = 'https://picsum.photos/seed/native-editor/1200/800';

export const INITIAL_CONTENT = [
    '<h1>Field notes</h1>',
    '<p>A native editor with a <strong>Rust core</strong>. Everything below is editable: ',
    'headings, <em>emphasis</em>, <u>underline</u>, <s>strikethrough</s>, and ',
    '<a href="https://github.com/apollohg/react-native-rich-text-editor">links</a>.</p>',
    '<blockquote><p>Type @ anywhere to mention someone on the team.</p></blockquote>',
    '<h2>Today</h2>',
    '<ul><li><p>Review the toolbar above the keyboard</p></li>',
    '<li><p>Try nested lists</p><ul><li><p>Indent and outdent from the toolbar</p></li></ul></li>',
    '<li><p>Tap the image to resize it</p></li></ul>',
    `<img src="${SAMPLE_IMAGE_URL}" alt="Sample" />`,
    '<h2>Counters</h2>',
    '<p>Custom blocks are React components living inside the document.</p>',
    '<div data-type="counter-card" data-title="Cups of coffee" data-count="2"></div>',
    '<ol><li><p>Insert another with the + button</p></li>',
    '<li><p>Tap a counter to select it, then delete it like any block</p></li></ol>',
    '<hr />',
    '<p></p>',
].join('');

export const MENTION_SUGGESTIONS: readonly MentionSuggestion[] = [
    {
        key: 'alice',
        title: 'Alice Chen',
        subtitle: 'Design',
        label: 'alice',
        attrs: { id: 'user_alice', entityType: 'user' },
    },
    {
        key: 'ben',
        title: 'Ben Ortiz',
        subtitle: 'Engineering',
        label: 'ben',
        attrs: { id: 'user_ben', entityType: 'user' },
    },
    {
        key: 'chloe',
        title: 'Chloe Park',
        subtitle: 'Product',
        label: 'chloe',
        attrs: { id: 'user_chloe', entityType: 'user' },
    },
    {
        key: 'apollo-team',
        title: 'Apollo Team',
        subtitle: 'Everyone',
        label: 'apollo-team',
        attrs: { id: 'group_apollo', entityType: 'group' },
    },
];

/** Custom toolbar action that inserts a counter card at the caret. */
export const INSERT_COUNTER_ACTION_KEY = 'insertCounter';

export const TOOLBAR_ITEMS: readonly EditorToolbarItem[] = [
    {
        type: 'group',
        key: 'headings',
        label: 'Heading',
        icon: {
            type: 'platform',
            ios: { type: 'sfSymbol', name: 'textformat.size' },
            android: { type: 'material', name: 'format-size' },
            fallbackText: 'H',
        },
        presentation: 'menu',
        items: [
            { type: 'heading', level: 1, label: 'Heading 1', icon: { type: 'default', id: 'h1' } },
            { type: 'heading', level: 2, label: 'Heading 2', icon: { type: 'default', id: 'h2' } },
            { type: 'heading', level: 3, label: 'Heading 3', icon: { type: 'default', id: 'h3' } },
            { type: 'heading', level: 4, label: 'Heading 4', icon: { type: 'default', id: 'h4' } },
            { type: 'heading', level: 5, label: 'Heading 5', icon: { type: 'default', id: 'h5' } },
            { type: 'heading', level: 6, label: 'Heading 6', icon: { type: 'default', id: 'h6' } },
        ],
    },
    { type: 'separator' },
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
    { type: 'separator' },
    { type: 'link', label: 'Link', icon: { type: 'default', id: 'link' } },
    { type: 'image', label: 'Image', icon: { type: 'default', id: 'image' } },
    { type: 'separator' },
    {
        type: 'group',
        key: 'lists',
        label: 'Lists',
        icon: { type: 'default', id: 'bulletList' },
        presentation: 'menu',
        items: [
            {
                type: 'list',
                listType: 'bullet_list',
                label: 'Bullet list',
                icon: { type: 'default', id: 'bulletList' },
            },
            {
                type: 'list',
                listType: 'ordered_list',
                label: 'Numbered list',
                icon: { type: 'default', id: 'orderedList' },
            },
            {
                type: 'command',
                command: 'indentList',
                label: 'Indent',
                icon: { type: 'default', id: 'indentList' },
            },
            {
                type: 'command',
                command: 'outdentList',
                label: 'Outdent',
                icon: { type: 'default', id: 'outdentList' },
            },
        ],
    },
    { type: 'blockquote', label: 'Quote', icon: { type: 'default', id: 'blockquote' } },
    {
        type: 'group',
        key: 'insert',
        label: 'Insert',
        icon: {
            type: 'platform',
            ios: { type: 'sfSymbol', name: 'plus.square' },
            android: { type: 'material', name: 'add-box' },
            fallbackText: '+',
        },
        presentation: 'menu',
        items: [
            {
                type: 'action',
                key: INSERT_COUNTER_ACTION_KEY,
                label: 'Counter',
                icon: {
                    type: 'platform',
                    ios: { type: 'sfSymbol', name: 'number.square' },
                    android: { type: 'material', name: 'pin' },
                    fallbackText: '#',
                },
            },
            {
                type: 'node',
                nodeType: 'horizontal_rule',
                label: 'Divider',
                icon: { type: 'default', id: 'horizontalRule' },
            },
            {
                type: 'node',
                nodeType: 'hard_break',
                label: 'Line break',
                icon: { type: 'default', id: 'lineBreak' },
            },
        ],
    },
    { type: 'separator' },
    { type: 'command', command: 'undo', label: 'Undo', icon: { type: 'default', id: 'undo' } },
    { type: 'command', command: 'redo', label: 'Redo', icon: { type: 'default', id: 'redo' } },
];
