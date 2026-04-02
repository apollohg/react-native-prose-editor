[Back to docs index](../README.md)

# Mentions

The editor includes an optional mentions addon that provides a native `@`-mention flow with native suggestion UI shown in the editor toolbar area.

## Basic Setup

Pass an `addons` prop with a `mentions` configuration:

```tsx
import {
  NativeRichTextEditor,
  type MentionSuggestion,
} from '@apollohg/react-native-prose-editor';

const suggestions: MentionSuggestion[] = [
  { key: 'alice', title: 'Alice Chen', label: '@alice', attrs: { id: 'user_1' } },
  { key: 'ben', title: 'Ben Ortiz', label: '@ben', attrs: { id: 'user_2' } },
];

<NativeRichTextEditor
  initialContent="<p>Hello</p>"
  addons={{
    mentions: {
      trigger: '@',
      suggestions,
    },
  }}
/>;
```

When the user types `@` (or your chosen trigger) at a valid position, the native toolbar switches into mention-suggestion mode with the configured suggestions.

## Mention Suggestions

```ts
interface MentionSuggestion {
  key: string;
  title: string;
  subtitle?: string;
  label?: string;
  attrs?: Record<string, unknown>;
}
```

| Field | Type | Description |
| --- | --- | --- |
| `key` | `string` | Unique identifier for the suggestion. |
| `title` | `string` | Primary display text in the suggestion UI. |
| `subtitle` | `string` | Optional secondary text shown below the title. |
| `label` | `string` | Text displayed in the inline mention chip after insertion. Defaults to `trigger + title` if omitted. |
| `attrs` | `Record<string, unknown>` | Arbitrary attributes stored on the inserted mention node. Use this for IDs, entity types, or any metadata your app needs. |

## Listening to Events

### Query Changes

Fires as the user types after the trigger character. Use this to filter suggestions or fetch results from an API.

```tsx
<NativeRichTextEditor
  addons={{
    mentions: {
      trigger: '@',
      suggestions,
      onQueryChange: (event) => {
        console.log(event.query);    // what the user typed after @
        console.log(event.isActive); // false when mention suggestion mode closes
      },
    },
  }}
/>;
```

```ts
interface MentionQueryChangeEvent {
  query: string;
  trigger: string;
  range: { anchor: number; head: number };
  isActive: boolean;
}
```

### Selection

Fires when the user picks a suggestion from the native mention suggestions.

```tsx
<NativeRichTextEditor
  addons={{
    mentions: {
      trigger: '@',
      suggestions,
      onSelect: (event) => {
        console.log(event.suggestion.key); // which suggestion was picked
        console.log(event.attrs);          // attrs stored on the node
      },
    },
  }}
/>;
```

```ts
interface MentionSelectEvent {
  trigger: string;
  suggestion: MentionSuggestion;
  attrs: Record<string, unknown>;
}
```

## Styling Mentions

Style both the inline mention chip and the native mention suggestions through `theme.mentions`:

```tsx
<NativeRichTextEditor
  theme={{
    mentions: {
      textColor: '#1a5c4f',
      backgroundColor: '#daf0eb',
      borderColor: '#a8d5cb',
      borderWidth: 1,
      borderRadius: 8,
      fontWeight: '700',
      optionTextColor: '#24292f',
      optionHighlightedBackgroundColor: '#daf0eb',
      optionHighlightedTextColor: '#1a5c4f',
    },
  }}
  addons={{
    mentions: {
      trigger: '@',
      suggestions,
    },
  }}
/>;
```

The `popover*` mention theme tokens are still accepted as legacy suggestion-surface aliases, but the current mention UI is rendered in the toolbar area rather than a floating popover.

See [EditorMentionTheme](../reference/editor-theme.md#editormentiontheme) for the full list of tokens.

## Schema Integration

The mentions addon automatically adds a `mention` node to your schema. If you need to add it manually (for example, to customize its attributes), use the provided helpers:

```tsx
import {
  tiptapSchema,
  withMentionsSchema,
  mentionNodeSpec,
  MENTION_NODE_NAME,
} from '@apollohg/react-native-prose-editor';

// Automatic — adds the default mention node if not already present
const schema = withMentionsSchema(tiptapSchema);

// Manual — use mentionNodeSpec() to get the default spec and customize it
const customSpec = { ...mentionNodeSpec(), attrs: { label: { default: null }, id: { default: null } } };
```

### Schema Helpers

| Export | Type | Description |
| --- | --- | --- |
| `MENTION_NODE_NAME` | `string` | The node name used for mentions (`'mention'`). |
| `mentionNodeSpec()` | `() => NodeSpec` | Returns the default mention `NodeSpec` (inline, void, with a `label` attribute). |
| `withMentionsSchema(schema)` | `(schema: SchemaDefinition) => SchemaDefinition` | Returns a new schema with the mention node added. No-op if a node named `mention` already exists. |
| `buildMentionFragmentJson(attrs)` | `(attrs: Record<string, unknown>) => DocumentJSON` | Builds a `DocumentJSON` fragment containing a single mention node with the given attributes. Useful for programmatic mention insertion via `insertContentJson`. |

## Full Configuration

```ts
interface MentionsAddonConfig {
  trigger?: string;
  suggestions?: readonly MentionSuggestion[];
  theme?: EditorMentionTheme;
  onQueryChange?: (event: MentionQueryChangeEvent) => void;
  onSelect?: (event: MentionSelectEvent) => void;
}

interface EditorAddons {
  mentions?: MentionsAddonConfig;
}
```

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `trigger` | `string` | `'@'` | Character that activates mention suggestion mode. |
| `suggestions` | `readonly MentionSuggestion[]` | `[]` | List of available suggestions shown in the native mention suggestions. |
| `theme` | `EditorMentionTheme` | — | Mention styling. Can also be set via `theme.mentions` on the editor. |
| `onQueryChange` | `(event) => void` | — | Fires as the user types after the trigger character. |
| `onSelect` | `(event) => void` | — | Fires when a suggestion is selected. |

## `EditorAddonEvent`

The internal event type used by the native bridge for addon callbacks. You do not typically use this type directly — the addon config callbacks (`onQueryChange`, `onSelect`) are the intended public API. Exported for advanced use cases that need to handle raw bridge events.

```ts
type EditorAddonEvent =
  | {
      type: 'mentionsQueryChange';
      query: string;
      trigger: string;
      range: { anchor: number; head: number };
      isActive: boolean;
    }
  | {
      type: 'mentionsSelect';
      trigger: string;
      suggestionKey: string;
      attrs: Record<string, unknown>;
    };
```

## Related Docs

- [Getting Started](./getting-started.md)
- [Styling Guide](./styling.md)
- [EditorTheme Reference](../reference/editor-theme.md)
- [NativeRichTextEditor Reference](../reference/native-rich-text-editor.md)
