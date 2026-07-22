import type { EditorMentionTheme } from './EditorTheme';
import type { DocumentJSON } from './NativeEditorBridge';
import {
    buildDocumentFragmentJson,
    type ResolvedDocumentSchema,
    type SchemaDefinition,
    type NodeSpec,
} from './schemas';

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
    documentVersion?: string;
}

export interface MentionSelectEvent {
    trigger: string;
    suggestion: MentionSuggestion;
    attrs: Record<string, unknown>;
    documentVersion?: string;
}

export interface MentionSelectionAttrsEvent {
    trigger: string;
    suggestion: MentionSuggestion;
    attrs: Record<string, unknown>;
    range: {
        anchor: number;
        head: number;
    };
    documentVersion?: string;
}

export type MentionThemeResolveEvent = MentionSelectionAttrsEvent;

export interface MentionsAddonConfig {
    trigger?: string;
    suggestions?: readonly MentionSuggestion[];
    theme?: EditorMentionTheme;
    resolveSelectionAttrs?: (
        event: MentionSelectionAttrsEvent
    ) => Record<string, unknown> | null | undefined;
    resolveTheme?: (event: MentionThemeResolveEvent) => EditorMentionTheme | null | undefined;
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
    resolveSelectionAttrs?: boolean;
    resolveTheme?: boolean;
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
          documentVersion?: string;
      }
    | {
          type: 'mentionsSelectRequest';
          trigger: string;
          suggestionKey: string;
          attrs: Record<string, unknown>;
          range: {
              anchor: number;
              head: number;
          };
          documentVersion?: string;
          updateJson?: string;
      }
    | {
          type: 'mentionsSelect';
          trigger: string;
          suggestionKey: string;
          attrs: Record<string, unknown>;
          documentVersion?: string;
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
        // Mention nodes round-trip arbitrary app-defined metadata (id, kind,
        // mentionSuggestionChar, mentionTheme, and anything supplied via
        // MentionSuggestion.attrs / resolveSelectionAttrs) that this fixed
        // attrs map cannot enumerate. Opt out of the schema-declared-attrs
        // filter that Rust's set_json ingestion otherwise applies.
        allowUndeclaredAttrs: true,
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

export function normalizeEditorAddons(addons?: EditorAddons): SerializedEditorAddons | undefined {
    if (!addons?.mentions) {
        return undefined;
    }

    const trigger = addons.mentions.trigger?.trim() || DEFAULT_MENTION_TRIGGER;
    const suggestions = (addons.mentions.suggestions ?? []).map((suggestion) => {
        const label = suggestion.label?.trim() || suggestion.title;
        const attrs = {
            label,
            mentionSuggestionChar: trigger,
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
            ...(typeof addons.mentions.resolveSelectionAttrs === 'function'
                ? { resolveSelectionAttrs: true }
                : {}),
            ...(typeof addons.mentions.resolveTheme === 'function' ? { resolveTheme: true } : {}),
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

export function buildMentionFragmentJson(
    attrs: Record<string, unknown>,
    descriptor?: Pick<ResolvedDocumentSchema, 'documentNodeName'>
): DocumentJSON {
    return buildDocumentFragmentJson(
        [
            {
                type: MENTION_NODE_NAME,
                attrs,
            },
        ],
        descriptor
    );
}
