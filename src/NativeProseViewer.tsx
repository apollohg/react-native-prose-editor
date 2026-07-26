import React, { useCallback, useMemo } from 'react';
import type { NativeSyntheticEvent, ViewProps } from 'react-native';

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
import NativePreparedProseViewer from './specs/NativePreparedProseViewer';
import {
    serializePreparedProseViewerConfiguration,
    type PreparedProseViewerConfiguration,
} from './ViewerConfiguration';
import { tiptapSchema, type SchemaDefinition } from './schemas';

interface NativeProseViewerLinkPressNativeEvent {
    href: string;
    text: string;
}

interface NativeProseViewerMentionPressNativeEvent {
    docPos: number;
    label: string;
}

export interface NativeProseViewerErrorEvent {
    domain: string;
    code: string;
    message: string;
    fatal: boolean;
}

export interface NativeProseViewerMentionPressEvent {
    docPos: number;
    label: string;
}

export interface NativeProseViewerLinkPressEvent {
    href: string;
    text: string;
}

export interface NativeProseViewerMentionsConfig {
    trigger?: string;
    prefix?: string;
    theme?: EditorMentionTheme;
    onPress?: (event: NativeProseViewerMentionPressEvent) => void;
}

export interface NativeProseViewerAddons {
    mentions?: NativeProseViewerMentionsConfig;
}

export interface NativeProseViewerBaseProps extends ViewProps {
    schema?: SchemaDefinition;
    theme?: EditorTheme;
    allowBase64Images?: boolean;
    imageLoadingPolicy?: EditorImageLoadingPolicy;
    resourceLimits?: EditorResourceLimits;
    collapseTrailingEmptyParagraphs?: boolean;
    enableLinkTaps?: boolean;
    renderImages?: boolean;
    fontEnvironmentRevision?: number;
    addons?: NativeProseViewerAddons;
    onPressLink?: (event: NativeProseViewerLinkPressEvent) => void;
    onError?: (error: NativeProseViewerErrorEvent) => void;
}

interface NativeProseViewerJsonProps extends NativeProseViewerBaseProps {
    contentJSON: DocumentJSON | string;
    contentHTML?: never;
}

interface NativeProseViewerHtmlProps extends NativeProseViewerBaseProps {
    contentHTML: string;
    contentJSON?: never;
}

export type NativeProseViewerProps = NativeProseViewerJsonProps | NativeProseViewerHtmlProps;

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
    mentions: NativeProseViewerMentionsConfig | undefined
): PreparedProseViewerConfiguration {
    return {
        initialization: { type: 'localEmpty' },
        schema: withMentionsSchema(schema ?? tiptapSchema),
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

export function NativeProseViewer(props: NativeProseViewerProps) {
    const {
        contentJSON,
        contentHTML,
        schema,
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
                    mentions
                )
            ),
        [allowBase64Images, mentions, resolvedResourceLimits, schema]
    );
    const themeJson = useMemo(
        () =>
            serializeEditorTheme(
                mentions?.theme
                    ? { ...theme, mentions: { ...theme?.mentions, ...mentions.theme } }
                    : theme
            ),
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
    const handlePressLink = useCallback(
        (event: NativeSyntheticEvent<NativeProseViewerLinkPressNativeEvent>) => {
            onPressLink?.(event.nativeEvent);
        },
        [onPressLink]
    );
    const handlePressMention = useCallback(
        (event: NativeSyntheticEvent<NativeProseViewerMentionPressNativeEvent>) => {
            mentions?.onPress?.(event.nativeEvent);
        },
        [mentions]
    );
    const handleError = useCallback(
        (event: NativeSyntheticEvent<NativeProseViewerErrorEvent>) => {
            onError?.(event.nativeEvent);
        },
        [onError]
    );

    return (
        <NativePreparedProseViewer
            {...viewProps}
            sourceKind={sourceKind}
            source={source}
            configJson={configJson}
            themeJson={themeJson}
            imagePolicyJson={imagePolicyJson}
            imagesEnabled={renderImages}
            collapsesWhenEmpty={collapseTrailingEmptyParagraphs}
            enableLinkTaps={enableLinkTaps}
            fontEnvironmentRevision={fontEnvironmentRevision}
            onPressLink={onPressLink ? handlePressLink : undefined}
            onPressMention={mentions?.onPress ? handlePressMention : undefined}
            onError={onError ? handleError : undefined}
        />
    );
}
