import type { EditorMentionTheme } from './EditorTheme';
import type { DocumentJSON } from './NativeEditorBridge';
import type { SchemaDefinition, NodeSpec } from './schemas';

export interface MentionSuggestion {
    key: string;
    title: string;
    subtitle?: string;
    label?: string;
    attrs?: Record<string, unknown>;
}

export interface MentionQueryChangeEvent {
    query: string;
    trigger: string;
    range: {
        anchor: number;
        head: number;
    };
    isActive: boolean;
}

export interface MentionSelectEvent {
    trigger: string;
    suggestion: MentionSuggestion;
    attrs: Record<string, unknown>;
}

export interface MentionsAddonConfig {
    trigger?: string;
    suggestions?: readonly MentionSuggestion[];
    theme?: EditorMentionTheme;
    onQueryChange?: (event: MentionQueryChangeEvent) => void;
    onSelect?: (event: MentionSelectEvent) => void;
}

export interface EditorAddons {
    mentions?: MentionsAddonConfig;
}

export interface SerializedMentionSuggestion {
    key: string;
    title: string;
    subtitle?: string;
    label: string;
    attrs: Record<string, unknown>;
}

export interface SerializedMentionsAddonConfig {
    trigger: string;
    theme?: EditorMentionTheme;
    suggestions: SerializedMentionSuggestion[];
}

export interface SerializedEditorAddons {
    mentions?: SerializedMentionsAddonConfig;
}

export type EditorAddonEvent =
    | {
          type: 'mentionsQueryChange';
          query: string;
          trigger: string;
          range: {
              anchor: number;
              head: number;
          };
          isActive: boolean;
      }
    | {
          type: 'mentionsSelect';
          trigger: string;
          suggestionKey: string;
          attrs: Record<string, unknown>;
      };

export const MENTION_NODE_NAME = 'mention';
const DEFAULT_MENTION_TRIGGER = '@';

export function mentionNodeSpec(): NodeSpec {
    return {
        name: MENTION_NODE_NAME,
        content: '',
        group: 'inline',
        role: 'inline',
        isVoid: true,
        attrs: {
            label: { default: null },
        },
    };
}

export function withMentionsSchema(schema: SchemaDefinition): SchemaDefinition {
    const hasMentionNode = schema.nodes.some((node) => node.name === MENTION_NODE_NAME);
    if (hasMentionNode) {
        return schema;
    }

    return {
        ...schema,
        nodes: [...schema.nodes, mentionNodeSpec()],
    };
}

export function normalizeEditorAddons(
    addons?: EditorAddons
): SerializedEditorAddons | undefined {
    if (!addons?.mentions) {
        return undefined;
    }

    const trigger =
        addons.mentions.trigger?.trim() || DEFAULT_MENTION_TRIGGER;
    const suggestions = (addons.mentions.suggestions ?? []).map((suggestion) => {
        const label = suggestion.label?.trim() || `${trigger}${suggestion.title}`;
        const attrs = {
            label,
            ...(suggestion.attrs ?? {}),
        };

        return {
            key: suggestion.key,
            title: suggestion.title,
            subtitle: suggestion.subtitle,
            label,
            attrs,
        };
    });

    return {
        mentions: {
            trigger,
            theme: addons.mentions.theme,
            suggestions,
        },
    };
}

export function serializeEditorAddons(addons?: EditorAddons): string | undefined {
    const normalized = normalizeEditorAddons(addons);
    if (!normalized?.mentions) {
        return undefined;
    }

    return JSON.stringify(normalized);
}

export function buildMentionFragmentJson(attrs: Record<string, unknown>): DocumentJSON {
    return {
        type: 'doc',
        content: [
            {
                type: MENTION_NODE_NAME,
                attrs,
            },
        ],
    };
}
