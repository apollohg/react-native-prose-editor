import type {
  EditorToolbarTheme,
  MentionSuggestion,
} from '@apollohg/react-native-prose-editor';

export const INITIAL_CONTENT = [
  '<p><strong>Native Editor</strong> example app.</p>',
  '<p>Use this screen to test focus, theme updates, lists, line breaks, toolbar behavior, and optional addons.</p>',
  '<p>Enable mentions above, then type @ after a space, on a blank line, or after punctuation to show native mention suggestions in the toolbar.</p>',
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

export type ToolbarColorKey = Exclude<
  keyof Required<EditorToolbarTheme>,
  'borderRadius' | 'borderWidth' | 'buttonBorderRadius' | 'keyboardOffset' | 'horizontalInset'
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
