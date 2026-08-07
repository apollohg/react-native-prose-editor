import type {
    EditorImageLoadingPolicy,
    EditorToolbarTheme,
    MentionSuggestion,
    NativeRichTextEditorAutoCapitalize,
    NativeRichTextEditorHeightBehavior,
    NativeRichTextEditorKeyboardType,
    NativeRichTextEditorToolbarPlacement,
    NativeRichTextEditorValueJSONUpdateMode,
} from '@apollohg/react-native-prose-editor';

export const INITIAL_CONTENT = [
    '<p><strong>Native Editor</strong> example app.</p>',
    '<p>Use this screen to test focus, theme updates, lists, line breaks, toolbar behavior, and optional addons.</p>',
    '<p>Enable mentions above, then type @ after a space, on a blank line, or after punctuation to show native mention suggestions in the toolbar.</p>',
    '<blockquote><p>Blockquotes can wrap one or more blocks and inherit theme styling.</p></blockquote>',
    '<ul><li><p>Try typing</p></li><li><p>Try list indenting</p><ul><li>Multiple levels are supported</li></ul></li></ul>',
    '<p></p>',
].join('');

/** Second document for the controlled path, structurally unlike INITIAL_CONTENT. */
export const CONTROLLED_CONTENT = [
    '<h2>Controlled content</h2>',
    '<p>This document was pushed in through the controlled <code>value</code> prop.</p>',
    '<ol><li><p>External writes are diffed against the engine document</p></li>',
    '<li><p>Undo history survives a <em>replace</em> and is dropped by a <em>reset</em></p></li></ol>',
    '<hr />',
    '<p>Switch back to uncontrolled to resume free editing.</p>',
].join('');

export const EXAMPLE_MENTION_SUGGESTIONS: readonly MentionSuggestion[] = [
    {
        key: 'alice',
        title: 'Alice Chen',
        label: 'alice',
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
        label: 'ben',
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
        label: 'chloe',
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
        label: 'apollo-team',
        attrs: {
            id: 'group_apollo',
            entityType: 'group',
            slug: 'apollo-team',
        },
    },
];

export type ToolbarColorKey = Exclude<
    keyof Required<EditorToolbarTheme>,
    | 'appearance'
    | 'height'
    | 'borderRadius'
    | 'borderWidth'
    | 'marginTop'
    | 'showTopBorder'
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

/** A labelled choice list rendered as a chip row. */
export type ChoiceOption<TValue> = {
    value: TValue;
    label: string;
};

export const HEIGHT_BEHAVIOR_OPTIONS: readonly ChoiceOption<NativeRichTextEditorHeightBehavior>[] =
    [
        { value: 'fixed', label: 'Fixed' },
        { value: 'autoGrow', label: 'Auto grow' },
    ];

export const TOOLBAR_PLACEMENT_OPTIONS: readonly ChoiceOption<NativeRichTextEditorToolbarPlacement>[] =
    [
        { value: 'keyboard', label: 'Keyboard' },
        { value: 'inline', label: 'Inline' },
    ];

export const AUTO_CAPITALIZE_OPTIONS: readonly ChoiceOption<NativeRichTextEditorAutoCapitalize>[] =
    [
        { value: 'none', label: 'None' },
        { value: 'sentences', label: 'Sentences' },
        { value: 'words', label: 'Words' },
        { value: 'characters', label: 'Characters' },
    ];

/** The distinct native layouts; the rest differ only in punctuation. */
export const KEYBOARD_TYPE_OPTIONS: readonly ChoiceOption<NativeRichTextEditorKeyboardType>[] = [
    { value: 'default', label: 'Default' },
    { value: 'email-address', label: 'Email' },
    { value: 'url', label: 'URL' },
    { value: 'numeric', label: 'Numeric' },
    { value: 'phone-pad', label: 'Phone' },
    { value: 'visible-password', label: 'Visible password' },
];

export const VALUE_JSON_UPDATE_MODE_OPTIONS: readonly ChoiceOption<NativeRichTextEditorValueJSONUpdateMode>[] =
    [
        { value: 'replace', label: 'Replace' },
        { value: 'reset', label: 'Reset' },
    ];

export type ControlledSourceMode = 'uncontrolled' | 'html' | 'json';

export const CONTROLLED_SOURCE_OPTIONS: readonly ChoiceOption<ControlledSourceMode>[] = [
    { value: 'uncontrolled', label: 'Uncontrolled' },
    { value: 'html', label: 'value (HTML)' },
    { value: 'json', label: 'valueJSON' },
];

/** Ranges straddle the package defaults, so both sides of each bound are reachable. */
export const IMAGE_POLICY_FIELDS: readonly {
    key: keyof EditorImageLoadingPolicy;
    label: string;
    min: number;
    max: number;
    step: number;
    unit: string;
}[] = [
    {
        key: 'maxSourceBytes',
        label: 'Max source',
        min: 64 * 1024,
        max: 32 * 1024 * 1024,
        step: 64 * 1024,
        unit: 'bytes',
    },
    {
        key: 'maxDecodeDimensionPx',
        label: 'Max decode',
        min: 128,
        max: 8192,
        step: 128,
        unit: 'px',
    },
    { key: 'maxConcurrentRequests', label: 'Max concurrent', min: 1, max: 16, step: 1, unit: '' },
    { key: 'maxPendingRequests', label: 'Max pending', min: 1, max: 512, step: 1, unit: '' },
    {
        key: 'connectTimeoutMs',
        label: 'Connect timeout',
        min: 1_000,
        max: 60_000,
        step: 1_000,
        unit: 'ms',
    },
    {
        key: 'readTimeoutMs',
        label: 'Read timeout',
        min: 1_000,
        max: 120_000,
        step: 1_000,
        unit: 'ms',
    },
    {
        key: 'requestTimeoutMs',
        label: 'Request timeout',
        min: 1_000,
        max: 300_000,
        step: 1_000,
        unit: 'ms',
    },
];

/** Longest event history the log keeps. Bounded so the harness cannot leak. */
export const EVENT_LOG_LIMIT = 50;

/** One tab per API area, so a regression hunt goes straight to the area. */
export type SettingsTab =
    | 'editor'
    | 'toolbar'
    | 'items'
    | 'content'
    | 'commands'
    | 'input'
    | 'images';

export const SETTINGS_TABS: readonly ChoiceOption<SettingsTab>[] = [
    { value: 'editor', label: 'Editor' },
    { value: 'toolbar', label: 'Toolbar' },
    { value: 'items', label: 'Items' },
    { value: 'content', label: 'Content' },
    { value: 'commands', label: 'Commands' },
    { value: 'input', label: 'Input' },
    { value: 'images', label: 'Images' },
];

/** Every imperative ref method, addressable by id, so the panel stays dumb. */
export type EditorCommandId =
    | 'block:blockquote'
    | 'block:bulletList'
    | 'block:orderedList'
    | 'block:indent'
    | 'block:outdent'
    | 'heading:1'
    | 'heading:2'
    | 'heading:3'
    | 'heading:4'
    | 'heading:5'
    | 'heading:6'
    | 'insert:hardBreak'
    | 'insert:horizontalRule'
    | 'insert:text'
    | 'insert:html'
    | 'insert:json'
    | 'insert:image'
    | 'doc:setContent'
    | 'doc:setContentJson'
    | 'doc:clear'
    | 'read:content'
    | 'read:contentJson'
    | 'read:text'
    | 'read:caretRect'
    | 'history:undo'
    | 'history:redo'
    | 'history:state'
    | 'focus'
    | 'blur';

export const EDITOR_COMMAND_GROUPS: readonly {
    title: string;
    hint: string;
    commands: readonly { id: EditorCommandId; label: string }[];
}[] = [
    {
        title: 'Blocks',
        hint: 'Block toggles and list indentation.',
        commands: [
            { id: 'block:blockquote', label: 'Blockquote' },
            { id: 'block:bulletList', label: 'Bullets' },
            { id: 'block:orderedList', label: 'Numbers' },
            { id: 'block:indent', label: 'Indent' },
            { id: 'block:outdent', label: 'Outdent' },
        ],
    },
    {
        title: 'Headings',
        hint: 'toggleHeading at each level the schema allows.',
        commands: [
            { id: 'heading:1', label: 'H1' },
            { id: 'heading:2', label: 'H2' },
            { id: 'heading:3', label: 'H3' },
            { id: 'heading:4', label: 'H4' },
            { id: 'heading:5', label: 'H5' },
            { id: 'heading:6', label: 'H6' },
        ],
    },
    {
        title: 'Insert at caret',
        hint: 'Node and fragment insertion.',
        commands: [
            { id: 'insert:hardBreak', label: 'Line break' },
            { id: 'insert:horizontalRule', label: 'Rule' },
            { id: 'insert:text', label: 'Text' },
            { id: 'insert:html', label: 'HTML fragment' },
            { id: 'insert:json', label: 'JSON fragment' },
            { id: 'insert:image', label: 'Image' },
        ],
    },
    {
        title: 'Whole document',
        hint: 'Replaces or clears all content.',
        commands: [
            { id: 'doc:setContent', label: 'setContent' },
            { id: 'doc:setContentJson', label: 'setContentJson' },
            { id: 'doc:clear', label: 'clearContent' },
        ],
    },
    {
        title: 'Reads',
        hint: 'Results land in the Events readout.',
        commands: [
            { id: 'read:content', label: 'getContent' },
            { id: 'read:contentJson', label: 'getContentJson' },
            { id: 'read:text', label: 'getTextContent' },
            { id: 'read:caretRect', label: 'getCaretRect' },
            { id: 'history:state', label: 'canUndo / canRedo' },
        ],
    },
    {
        title: 'History and focus',
        hint: 'Undo stack and focus control.',
        commands: [
            { id: 'history:undo', label: 'Undo' },
            { id: 'history:redo', label: 'Redo' },
            { id: 'focus', label: 'Focus' },
            { id: 'blur', label: 'Blur' },
        ],
    },
];

/** Needs a URL, so the toolbar link button and `onRequestLink` drive it. */
export const LINK_MARK_NAME = 'link';

/** Fragments used by the insert commands. Short so the effect is obvious. */
export const INSERT_TEXT_SAMPLE = ' inserted text ';
export const INSERT_HTML_SAMPLE = '<p>Inserted <strong>HTML</strong> fragment.</p>';

/** Remote image used by the image insert command and the loading-policy tests. */
export const SAMPLE_IMAGE_URL = 'https://picsum.photos/seed/native-editor/1200/800';
