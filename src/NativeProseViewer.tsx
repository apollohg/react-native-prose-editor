import React, { useCallback, useMemo } from 'react';
import { View, type NativeSyntheticEvent, type ViewProps } from 'react-native';

import {
    serializeEditorImageLoadingPolicy,
    type EditorImageLoadingPolicy,
} from './ImageLoadingPolicy';
import {
    resolveEditorResourceLimits,
    type EditorResourceLimits,
    type ResolvedEditorResourceLimits,
} from './ResourceLimits';
import { serializeEditorTheme, type EditorMentionTheme, type EditorTheme } from './EditorTheme';
import type { DocumentJSON } from './NativeEditorBridge';
import { withMentionsSchema } from './addons';
import { withAtomsSchema, type AtomNodeDefinition } from './atoms';
import { useViewerAtoms, type RichTextViewerAtomAttrsUpdateEvent } from './useViewerAtoms';
import NativePreparedProseViewer from './specs/PreparedProseViewerNativeComponent';
import {
    serializePreparedProseViewerConfiguration,
    type PreparedProseViewerConfiguration,
} from './ViewerConfiguration';
import { defaultSchema, type SchemaDefinition } from './schemas';

export type {
    RichTextViewerAtomAttrsUpdateEvent,
    NativeProseViewerAtomAttrsUpdateEvent,
} from './useViewerAtoms';

interface RichTextViewerLinkPressNativeEvent {
    href: string;
    text: string;
}

interface RichTextViewerMentionPressNativeEvent {
    docPos: number;
    label: string;
    attrsJson: string;
}

/** A parse, layout, or rendering failure reported by the native viewer. */
export interface RichTextViewerErrorEvent {
    /** Native subsystem that raised the failure, e.g. `'viewer'`. */
    domain: string;
    /** Stable failure code, e.g. `'INVALID_MENTION_ATTRIBUTES'`. */
    code: string;
    message: string;
    /** Whether the viewer gave up on this content, as opposed to skipping one element. */
    fatal: boolean;
}

/** A tap on a mention node. Requires `addons.mentions.onPress`. */
export interface RichTextViewerMentionPressEvent {
    /** Engine document position of the pressed mention node. */
    docPos: number;
    /** Rendered mention label. */
    label: string;
    /** The mention node's attributes, as stored in the document. */
    attrs: Record<string, unknown>;
}

/** A tap on link-marked text. Requires `enableLinkTaps` and `onPressLink`. */
export interface RichTextViewerLinkPressEvent {
    /** The link mark's `href` attribute. */
    href: string;
    /** The text the link mark covers. */
    text: string;
}

/** Mention rendering and press handling for the viewer. */
export interface RichTextViewerMentionsConfig {
    /** The trigger character this content was authored with. Recorded in the
     *  viewer configuration; what gets drawn comes from `prefix`. */
    trigger?: string;
    /** Prepended to a mention's label when the label does not already start
     *  with it. Unset renders the stored label unchanged. */
    prefix?: string;
    /** Mention styling. See {@link EditorMentionTheme}. */
    theme?: EditorMentionTheme;
    /** Called when a mention is pressed. Mentions are inert until this is set. */
    onPress?: (event: RichTextViewerMentionPressEvent) => void;
}

/** Optional viewer features, mirroring the editor's `EditorAddons`. */
export interface RichTextViewerAddons {
    mentions?: RichTextViewerMentionsConfig;
}

/** Props shared by both content forms of {@link RichTextViewerProps}. */
export interface RichTextViewerBaseProps extends ViewProps {
    /** Schema the content is parsed against. Defaults to {@link defaultSchema}; the mention node is always added. */
    schema?: SchemaDefinition;
    /** React renderers for custom block atoms; their node specs are added to the schema. */
    atoms?: readonly AtomNodeDefinition[];
    /** Whether custom atoms are inert. Defaults to true; prose is always read-only. */
    readOnly?: boolean;
    /** Handles an atom update request. Persist it in the app and supply updated content. */
    onUpdateAtomAttrs?: (event: RichTextViewerAtomAttrsUpdateEvent) => void | Promise<void>;
    /** Native content theme. See {@link EditorTheme}. */
    theme?: EditorTheme;
    /** Whether `data:` image sources are admitted. Defaults to false. */
    allowBase64Images?: boolean;
    /** Bounds image fetching and decoding. See {@link EditorImageLoadingPolicy}. */
    imageLoadingPolicy?: EditorImageLoadingPolicy;
    /** Bounds the content the viewer will parse. See {@link EditorResourceLimits}. */
    resourceLimits?: EditorResourceLimits;
    /** Whether content with no text measures to zero height. Defaults to true. */
    collapseTrailingEmptyParagraphs?: boolean;
    /** Whether link-marked text is tappable. Defaults to true. */
    enableLinkTaps?: boolean;
    /** Whether image nodes are fetched and drawn. Defaults to true. */
    renderImages?: boolean;
    /**
     * Bump this to discard the prepared layout after the app's font
     * environment changes (a newly registered font, a font-scale change).
     * Defaults to 0.
     */
    fontEnvironmentRevision?: number;
    addons?: RichTextViewerAddons;
    /** Called when link-marked text is tapped. */
    onPressLink?: (event: RichTextViewerLinkPressEvent) => void;
    /** Called when the viewer cannot parse, lay out, or render the content. */
    onError?: (error: RichTextViewerErrorEvent) => void;
}

interface RichTextViewerJsonProps extends RichTextViewerBaseProps {
    /** ProseMirror JSON document, or a JSON string holding one. */
    contentJSON: DocumentJSON | string;
    contentHTML?: never;
}

interface RichTextViewerHtmlProps extends RichTextViewerBaseProps {
    /** HTML document. Sanitize untrusted HTML before passing it here. */
    contentHTML: string;
    contentJSON?: never;
}

/**
 * Props for {@link RichTextViewer}. Supply exactly one content source:
 * `contentJSON` or `contentHTML`.
 */
export type RichTextViewerProps = RichTextViewerJsonProps | RichTextViewerHtmlProps;

const serializedJsonCache = new WeakMap<object, string>();

function stringifyCachedJson(value: DocumentJSON): string {
    const cached = serializedJsonCache.get(value);
    if (cached != null) {
        return cached;
    }

    const serialized = JSON.stringify(value);
    serializedJsonCache.set(value, serialized);
    return serialized;
}

function resolveViewerConfiguration(
    schema: SchemaDefinition | undefined,
    allowBase64Images: boolean,
    resourceLimits: ResolvedEditorResourceLimits | undefined,
    mentions: RichTextViewerMentionsConfig | undefined,
    atoms: readonly AtomNodeDefinition[] | undefined
): PreparedProseViewerConfiguration {
    return {
        initialization: { type: 'localEmpty' },
        schema: withAtomsSchema(withMentionsSchema(schema ?? defaultSchema), atoms ?? []),
        ...(allowBase64Images ? { policy: { allowBase64Images: true } } : {}),
        ...(resourceLimits ? { limits: { resource: resourceLimits } } : {}),
        ...(mentions?.trigger || mentions?.prefix
            ? {
                  mentions: {
                      ...(mentions.trigger ? { trigger: mentions.trigger } : {}),
                      ...(mentions.prefix ? { prefix: mentions.prefix } : {}),
                  },
              }
            : {}),
    };
}

/**
 * Read-only renderer for HTML or ProseMirror JSON. It prepares a native
 * layout and measures to that layout's exact size, so the host must give it a
 * finite width; no editor session is created and no document handle is
 * needed.
 *
 * Requires the New Architecture.
 *
 * @example
 * ```tsx
 * <RichTextViewer
 *     contentHTML='<p>Read-only content</p>'
 *     theme={{ text: { fontSize: 16 } }}
 *     onPressLink={({ href }) => openLink(href)}
 * />
 * ```
 */
export function RichTextViewer(props: RichTextViewerProps) {
    const {
        contentJSON,
        contentHTML,
        schema,
        atoms,
        readOnly = true,
        onUpdateAtomAttrs,
        theme,
        allowBase64Images = false,
        imageLoadingPolicy,
        resourceLimits,
        collapseTrailingEmptyParagraphs = true,
        enableLinkTaps = true,
        renderImages = true,
        fontEnvironmentRevision = 0,
        addons,
        onPressLink,
        onError,
        ...viewProps
    } = props;
    const mentions = addons?.mentions;
    const resolvedResourceLimits = useMemo(
        () => (resourceLimits ? resolveEditorResourceLimits(resourceLimits) : undefined),
        [resourceLimits]
    );
    const configJson = useMemo(
        () =>
            serializePreparedProseViewerConfiguration(
                resolveViewerConfiguration(
                    schema,
                    allowBase64Images,
                    resolvedResourceLimits,
                    mentions,
                    atoms
                )
            ),
        [allowBase64Images, mentions, resolvedResourceLimits, schema, atoms]
    );
    const themeJson = useMemo(
        () => serializeEditorTheme(theme, mentions?.theme),
        [mentions?.theme, theme]
    );
    const imagePolicyJson = useMemo(
        () => serializeEditorImageLoadingPolicy(imageLoadingPolicy),
        [imageLoadingPolicy]
    );
    const sourceKind = contentJSON === undefined ? 'html' : 'json';
    const source = useMemo(() => {
        if (contentJSON === undefined) {
            return contentHTML ?? '';
        }
        return typeof contentJSON === 'string' ? contentJSON : stringifyCachedJson(contentJSON);
    }, [contentHTML, contentJSON]);
    const atomIdentity = useMemo(
        () => ({}),
        [
            sourceKind,
            source,
            configJson,
            themeJson,
            imagePolicyJson,
            renderImages,
            collapseTrailingEmptyParagraphs,
            fontEnvironmentRevision,
        ]
    );
    const viewerAtoms = useViewerAtoms({
        atoms,
        identity: atomIdentity,
        themeJson,
        readOnly,
        onUpdateAtomAttrs,
        onError,
    });
    const handlePressLink = useCallback(
        (event: NativeSyntheticEvent<RichTextViewerLinkPressNativeEvent>) => {
            onPressLink?.(event.nativeEvent);
        },
        [onPressLink]
    );
    const handlePressMention = useCallback(
        (event: NativeSyntheticEvent<RichTextViewerMentionPressNativeEvent>) => {
            const { docPos, label, attrsJson } = event.nativeEvent;
            let attrs: unknown;
            try {
                attrs = JSON.parse(attrsJson);
            } catch {
                attrs = null;
            }
            if (attrs === null || typeof attrs !== 'object' || Array.isArray(attrs)) {
                onError?.({
                    domain: 'viewer',
                    code: 'INVALID_MENTION_ATTRIBUTES',
                    message: 'The prepared mention attributes are not a JSON object.',
                    fatal: false,
                });
                return;
            }
            mentions?.onPress?.({ docPos, label, attrs: attrs as Record<string, unknown> });
        },
        [mentions, onError]
    );
    const handleError = useCallback(
        (event: NativeSyntheticEvent<RichTextViewerErrorEvent>) => {
            onError?.(event.nativeEvent);
        },
        [onError]
    );

    const nativeViewer = (
        <NativePreparedProseViewer
            {...(viewerAtoms.enabled ? { style: { alignSelf: 'stretch' as const } } : viewProps)}
            sourceKind={sourceKind}
            source={source}
            configJson={configJson}
            themeJson={viewerAtoms.themeJson}
            onAtomLayout={viewerAtoms.enabled ? viewerAtoms.onAtomLayout : undefined}
            imagePolicyJson={imagePolicyJson}
            imagesEnabled={renderImages}
            collapsesWhenEmpty={collapseTrailingEmptyParagraphs}
            enableLinkTaps={enableLinkTaps && onPressLink != null}
            mentionInteractionsEnabled={mentions?.onPress != null}
            fontEnvironmentRevision={fontEnvironmentRevision}
            onPressLink={onPressLink ? handlePressLink : undefined}
            onPressMention={mentions?.onPress ? handlePressMention : undefined}
            onError={onError ? handleError : undefined}
        />
    );
    if (!viewerAtoms.enabled) return nativeViewer;
    return (
        <View {...viewProps}>
            <View style={{ alignSelf: 'stretch' }} onLayout={viewerAtoms.onContainerLayout}>
                {nativeViewer}
                {viewerAtoms.children}
            </View>
        </View>
    );
}

/** @deprecated Use RichTextViewerErrorEvent instead. */
export type NativeProseViewerErrorEvent = RichTextViewerErrorEvent;

/** @deprecated Use RichTextViewerMentionPressEvent instead. */
export type NativeProseViewerMentionPressEvent = RichTextViewerMentionPressEvent;

/** @deprecated Use RichTextViewerLinkPressEvent instead. */
export type NativeProseViewerLinkPressEvent = RichTextViewerLinkPressEvent;

/** @deprecated Use RichTextViewerMentionsConfig instead. */
export type NativeProseViewerMentionsConfig = RichTextViewerMentionsConfig;

/** @deprecated Use RichTextViewerAddons instead. */
export type NativeProseViewerAddons = RichTextViewerAddons;

/** @deprecated Use RichTextViewerBaseProps instead. */
export type NativeProseViewerBaseProps = RichTextViewerBaseProps;

/** @deprecated Use RichTextViewerProps instead. */
export type NativeProseViewerProps = RichTextViewerProps;

/** @deprecated Use RichTextViewer instead. */
export const NativeProseViewer = RichTextViewer;
