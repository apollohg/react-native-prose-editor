import type { EditorMentionTheme } from './EditorTheme';
import type { DocumentJSON, ReadonlyActiveState } from './NativeEditorBridge';
import {
    buildDocumentFragmentJson,
    type ResolvedDocumentSchema,
    type SchemaDefinition,
    type NodeSpec,
} from './schemas';

/** One row offered in the mention suggestion list. */
export interface MentionSuggestion {
    /** Identity of this suggestion. Must be unique within the offered list. */
    key: string;
    /** Primary text shown in the suggestion row. */
    title: string;
    /** Secondary text shown in the suggestion row. */
    subtitle?: string;
    /** Text rendered in the document once inserted. Defaults to `title`. */
    label?: string;
    /** Extra attributes stored on the inserted mention node, e.g. a user id. */
    attrs?: Record<string, unknown>;
}

/** The mention query under the caret changed. */
export interface MentionQueryChangeEvent {
    /** Text typed after the trigger, without the trigger itself. */
    query: string;
    /** Trigger character that opened this query. */
    trigger: string;
    /** Engine document positions the mention would replace. */
    range: {
        anchor: number;
        head: number;
    };
    /** False once the query closed — supply an empty suggestion list. */
    isActive: boolean;
    /** Engine document revision the query was computed against. */
    documentVersion?: string;
}

/** A suggestion was inserted into the document. */
export interface MentionSelectEvent {
    trigger: string;
    suggestion: MentionSuggestion;
    /** The attributes actually written onto the mention node. */
    attrs: Record<string, unknown>;
    documentVersion?: string;
}

/**
 * A suggestion is about to be inserted. Passed to
 * {@link MentionsAddonConfig.resolveSelectionAttrs} and
 * {@link MentionsAddonConfig.resolveTheme} so the host can decide the node's
 * attributes and styling from live document context.
 */
export interface MentionSelectionAttrsEvent {
    trigger: string;
    suggestion: MentionSuggestion;
    /** Attributes resolved so far — the suggestion's own, plus `label` and `mentionSuggestionChar`. */
    attrs: Record<string, unknown>;
    /** Mark attributes active at the insertion point. */
    markAttrs: ReadonlyActiveState['markAttrs'];
    /** Engine document positions the mention will replace. */
    range: {
        anchor: number;
        head: number;
    };
    documentVersion?: string;
}

/** Payload of {@link MentionsAddonConfig.resolveTheme}, carrying the attributes
 *  `resolveSelectionAttrs` already resolved. */
export type MentionThemeResolveEvent = MentionSelectionAttrsEvent;

/**
 * Mentions for `RichTextEditor`. The schema belongs to the document
 * handle, so create the handle with {@link withMentionsSchema} applied to
 * your schema — this config alone does not add the `mention` node.
 * (`RichTextViewer` adds it for you.)
 *
 * The host owns the suggestion list: react to `onQueryChange` by filtering
 * your own data, then pass the result back through `suggestions`.
 */
export interface MentionsAddonConfig {
    /** Character that opens a mention query. Defaults to `'@'`. */
    trigger?: string;
    /** Rows currently offered for the open query. */
    suggestions?: readonly MentionSuggestion[];
    /** Default mention styling. See {@link EditorMentionTheme}. */
    theme?: EditorMentionTheme;
    /**
     * Extra attributes merged over the suggestion's own, just before
     * insertion. Return `null`/`undefined` to keep them unchanged. Throwing
     * is caught and treated the same way.
     */
    resolveSelectionAttrs?: (
        event: MentionSelectionAttrsEvent
    ) => Record<string, unknown> | null | undefined;
    /**
     * Per-mention styling, stored on the inserted node and used in place of
     * `theme`. Runs after `resolveSelectionAttrs`. A value that is not a
     * valid {@link EditorMentionTheme} is dropped rather than written into
     * the document.
     */
    resolveTheme?: (event: MentionThemeResolveEvent) => EditorMentionTheme | null | undefined;
    /** Called whenever the open query changes, including when it closes. */
    onQueryChange?: (event: MentionQueryChangeEvent) => void;
    /** Called after a suggestion has been inserted. */
    onSelect?: (event: MentionSelectEvent) => void;
}

/** Optional editor features, passed to `RichTextEditor.addons`. */
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

/**
 * The raw addon event the native view emits, before the editor turns it into
 * the typed {@link MentionsAddonConfig} callbacks. Exported for hosts that
 * inspect the native stream directly; ordinary use needs `onQueryChange` and
 * `onSelect` instead.
 */
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

/** Node name mentions are stored under in the document. */
export const MENTION_NODE_NAME = 'mention';
const DEFAULT_MENTION_TRIGGER = '@';

/**
 * The mention node spec: a void inline node that round-trips arbitrary
 * app-defined attributes. Add it through {@link withMentionsSchema} rather
 * than assembling it by hand.
 */
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

/**
 * Return `schema` with the mention node added, or `schema` unchanged when it
 * already declares one. Apply it when creating the document handle for an
 * editor that uses mentions.
 *
 * @example
 * ```ts
 * createNativeEditorDocumentHandle({
 *     initialization: { type: 'localEmpty' },
 *     schema: withMentionsSchema(defaultSchema),
 * });
 * ```
 */
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

/** Options for {@link buildMentionFragmentJson}. */
export interface MentionFragmentOptions {
    /** Append a plain, unmarked space so typing continues after the mention. */
    trailingSpace?: boolean;
}

/**
 * Build a document fragment holding one mention node, ready for
 * `insertContentJson`. Use it to insert a mention outside the suggestion
 * flow — from a picker, say.
 *
 * @param attrs Attributes for the mention node, e.g. `{ label, id }`.
 * @param descriptor Document node name to wrap the fragment in. Defaults to `doc`.
 */
export function buildMentionFragmentJson(
    attrs: Record<string, unknown>,
    descriptor?: Pick<ResolvedDocumentSchema, 'documentNodeName'>,
    options?: MentionFragmentOptions
): DocumentJSON {
    return buildDocumentFragmentJson(
        [
            {
                type: MENTION_NODE_NAME,
                attrs,
            },
            ...(options?.trailingSpace ? [{ type: 'text', text: ' ' }] : []),
        ],
        descriptor
    );
}
