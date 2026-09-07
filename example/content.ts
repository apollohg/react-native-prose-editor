import type {
    DocumentJSON,
    EditorToolbarItem,
    MentionSuggestion,
} from 'react-native-rich-text-editor';

import { TASK_ITEM_NODE_NAME, TASK_LIST_NODE_NAME } from './taskList';

export const APP_TITLE = 'React Native Editor';

export const EDITOR_PLACEHOLDER = 'Start writing…';

export const MENTION_TRIGGER = '@';

/** Remote image used by the initial document. */
export const SAMPLE_IMAGE_URL = 'https://picsum.photos/seed/native-editor/1200/800';

const REPOSITORY_URL = 'https://github.com/apollohg/react-native-rich-text-editor';

type MarkJSON = { type: string; attrs?: Record<string, unknown> };

const BOLD: MarkJSON = { type: 'bold' };
const ITALIC: MarkJSON = { type: 'italic' };
const UNDERLINE: MarkJSON = { type: 'underline' };
const STRIKE: MarkJSON = { type: 'strike' };
const REPOSITORY_LINK: MarkJSON = { type: 'link', attrs: { href: REPOSITORY_URL } };

function text(value: string, ...marks: readonly MarkJSON[]): DocumentJSON {
    return marks.length === 0
        ? { type: 'text', text: value }
        : { type: 'text', text: value, marks };
}

function paragraph(...content: readonly DocumentJSON[]): DocumentJSON {
    return content.length === 0 ? { type: 'paragraph' } : { type: 'paragraph', content };
}

function heading(level: number, title: string): DocumentJSON {
    return { type: 'heading', attrs: { level }, content: [text(title)] };
}

function listItem(...content: readonly DocumentJSON[]): DocumentJSON {
    return { type: 'list_item', content };
}

function taskItem(checked: boolean, label: string): DocumentJSON {
    return { type: TASK_ITEM_NODE_NAME, attrs: { checked }, content: [paragraph(text(label))] };
}

/**
 * The initial document is JSON rather than HTML: the HTML importer maps every
 * `<ul>` to a bullet list, so a checklist can only be seeded this way.
 */
export const INITIAL_DOCUMENT: DocumentJSON = {
    type: 'doc',
    content: [
        heading(1, 'Field notes'),
        paragraph(
            text('A native editor with a '),
            text('Rust core', BOLD),
            text('. Everything below is editable: headings, '),
            text('emphasis', ITALIC),
            text(', '),
            text('underline', UNDERLINE),
            text(', '),
            text('strikethrough', STRIKE),
            text(', and '),
            text('links', REPOSITORY_LINK),
            text('.')
        ),
        {
            type: 'blockquote',
            content: [paragraph(text('Type @ anywhere to mention someone on the team.'))],
        },
        {
            type: 'codeBlock',
            attrs: { language: 'typescript' },
            content: [text('const greet = (name: string) => {\n    return `Hello, ${name}!`;\n};')],
        },
        heading(2, 'Today'),
        {
            type: TASK_LIST_NODE_NAME,
            content: [
                taskItem(true, 'Review the toolbar above the keyboard'),
                taskItem(false, 'Tap a checkbox to toggle it'),
                taskItem(false, 'Turn any paragraph into a task from the list menu'),
            ],
        },
        heading(2, 'Lists'),
        {
            type: 'bullet_list',
            content: [
                listItem(paragraph(text('Try nested lists')), {
                    type: 'bullet_list',
                    content: [listItem(paragraph(text('Indent and outdent from the toolbar')))],
                }),
                listItem(paragraph(text('Tap the image to resize it'))),
            ],
        },
        { type: 'image', attrs: { src: SAMPLE_IMAGE_URL, alt: 'Sample' } },
        heading(2, 'Counters'),
        paragraph(text('Custom blocks are React components living inside the document.')),
        { type: 'counterCard', attrs: { title: 'Cups of coffee', count: 2 } },
        {
            type: 'ordered_list',
            content: [
                listItem(paragraph(text('Insert another with the + button'))),
                listItem(
                    paragraph(text('Tap a counter to select it, then delete it like any block'))
                ),
            ],
        },
        { type: 'horizontal_rule' },
        paragraph(),
    ],
};

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

/**
 * Custom toolbar action that wraps the selection in a task list, or unwraps
 * it when already inside one. The built-in `list` item only covers bullet and
 * numbered lists, so active and disabled states come from `onActiveStateChange`.
 */
export const TOGGLE_TASK_LIST_ACTION_KEY = 'toggleTaskList';

export function buildToolbarItems({
    taskListActive,
    taskListAvailable,
}: {
    taskListActive: boolean;
    taskListAvailable: boolean;
}): readonly EditorToolbarItem[] {
    return [
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
                {
                    type: 'heading',
                    level: 1,
                    label: 'Heading 1',
                    icon: { type: 'default', id: 'h1' },
                },
                {
                    type: 'heading',
                    level: 2,
                    label: 'Heading 2',
                    icon: { type: 'default', id: 'h2' },
                },
                {
                    type: 'heading',
                    level: 3,
                    label: 'Heading 3',
                    icon: { type: 'default', id: 'h3' },
                },
                {
                    type: 'heading',
                    level: 4,
                    label: 'Heading 4',
                    icon: { type: 'default', id: 'h4' },
                },
                {
                    type: 'heading',
                    level: 5,
                    label: 'Heading 5',
                    icon: { type: 'default', id: 'h5' },
                },
                {
                    type: 'heading',
                    level: 6,
                    label: 'Heading 6',
                    icon: { type: 'default', id: 'h6' },
                },
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
                    type: 'action',
                    key: TOGGLE_TASK_LIST_ACTION_KEY,
                    label: 'Task list',
                    isActive: taskListActive,
                    isDisabled: !taskListAvailable,
                    icon: {
                        type: 'platform',
                        ios: { type: 'sfSymbol', name: 'checklist' },
                        android: { type: 'material', name: 'checklist' },
                        fallbackText: '☑',
                    },
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
}
