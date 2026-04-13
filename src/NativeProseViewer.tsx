import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { requireNativeModule, requireNativeViewManager } from 'expo-modules-core';
import {
    type NativeSyntheticEvent,
    type StyleProp,
    type ViewStyle,
} from 'react-native';

import { withMentionsSchema } from './addons';
import {
    serializeEditorTheme,
    type EditorMentionTheme,
    type EditorTheme,
} from './EditorTheme';
import type { DocumentJSON, RenderElement } from './NativeEditorBridge';
import {
    normalizeDocumentJson,
    tiptapSchema,
    type SchemaDefinition,
} from './schemas';

interface NativeProseViewerModule {
    renderDocumentJson(configJson: string, json: string): string;
}

interface NativeProseViewerViewProps {
    style?: StyleProp<ViewStyle>;
    renderJson: string;
    themeJson?: string;
    onContentHeightChange?: (
        event: NativeSyntheticEvent<NativeProseViewerContentHeightEvent>
    ) => void;
    onPressMention?: (
        event: NativeSyntheticEvent<NativeProseViewerMentionPressNativeEvent>
    ) => void;
}

interface NativeProseViewerContentHeightEvent {
    contentHeight: number;
}

interface NativeProseViewerMentionPressNativeEvent {
    docPos: number;
    label: string;
}

export interface NativeProseViewerMentionRenderContext {
    docPos: number;
    label: string;
    attrs: Record<string, unknown>;
}

export interface NativeProseViewerMentionPressEvent
    extends NativeProseViewerMentionRenderContext {}

type NativeProseViewerContent = DocumentJSON | string;

export interface NativeProseViewerProps {
    contentJSON: NativeProseViewerContent;
    contentJSONRevision?: string | number;
    schema?: SchemaDefinition;
    theme?: EditorTheme;
    style?: StyleProp<ViewStyle>;
    allowBase64Images?: boolean;
    mentionPrefix?:
        | string
        | ((mention: NativeProseViewerMentionRenderContext) => string | null | undefined);
    resolveMentionTheme?: (
        mention: NativeProseViewerMentionRenderContext
    ) => EditorMentionTheme | null | undefined;
    onPressMention?: (event: NativeProseViewerMentionPressEvent) => void;
}

const NativeProseViewerView = requireNativeViewManager(
    'NativeEditor',
    'NativeProseViewer'
) as React.ComponentType<NativeProseViewerViewProps>;

let nativeProseViewerModule: NativeProseViewerModule | null = null;

function getNativeProseViewerModule(): NativeProseViewerModule {
    if (!nativeProseViewerModule) {
        nativeProseViewerModule =
            requireNativeModule<NativeProseViewerModule>('NativeEditor');
    }
    return nativeProseViewerModule;
}

const serializedJsonCache = new WeakMap<object, string>();

function stringifyCachedJson(value: unknown): string {
    if (value != null && typeof value === 'object') {
        const cached = serializedJsonCache.get(value);
        if (cached != null) {
            return cached;
        }
        const serialized = JSON.stringify(value);
        serializedJsonCache.set(value, serialized);
        return serialized;
    }
    return JSON.stringify(value);
}

function looksLikeRenderElementsJson(json: string): boolean {
    for (let index = 0; index < json.length; index += 1) {
        const char = json[index];
        if (char === ' ' || char === '\n' || char === '\r' || char === '\t') {
            continue;
        }
        return char === '[';
    }
    return false;
}

function unicodeScalarLength(text: string): number {
    let length = 0;
    for (const _char of text) {
        length += 1;
    }
    return length;
}

function normalizeMentionAttrs(node: unknown): Record<string, unknown> {
    if (node == null || typeof node !== 'object') {
        return {};
    }
    const attrs = (node as Record<string, unknown>).attrs;
    if (attrs == null || typeof attrs !== 'object' || Array.isArray(attrs)) {
        return {};
    }
    return attrs as Record<string, unknown>;
}

function baseMentionLabelFromAttrs(attrs: Record<string, unknown>): string {
    const label = attrs.label;
    return typeof label === 'string' && label.length > 0 ? label : 'mention';
}

function resolveMentionPrefix(
    mentionPrefix: NativeProseViewerProps['mentionPrefix'],
    mention: NativeProseViewerMentionRenderContext
): string | undefined {
    const rawPrefix =
        typeof mentionPrefix === 'function' ? mentionPrefix(mention) : mentionPrefix;
    return typeof rawPrefix === 'string' && rawPrefix.length > 0 ? rawPrefix : undefined;
}

function applyMentionPrefix(label: string, prefix: string | undefined): string {
    if (!prefix || label.startsWith(prefix)) {
        return label;
    }
    return `${prefix}${label}`;
}

interface ResolvedMentionPayload extends NativeProseViewerMentionRenderContext {
    renderedLabel: string;
    mentionTheme?: EditorMentionTheme;
}

function collectMentionPayloadsByDocPos(
    document: DocumentJSON,
    mentionPrefix: NativeProseViewerProps['mentionPrefix'],
    resolveMentionTheme: NativeProseViewerProps['resolveMentionTheme']
): Map<number, ResolvedMentionPayload> {
    const mentions = new Map<number, ResolvedMentionPayload>();

    const visit = (node: unknown, pos: number, isRoot = false): number => {
        if (node == null || typeof node !== 'object') {
            return pos;
        }

        const nodeRecord = node as Record<string, unknown>;
        const nodeType = typeof nodeRecord.type === 'string' ? nodeRecord.type : '';
        const content = Array.isArray(nodeRecord.content) ? nodeRecord.content : [];

        if (nodeType === 'text') {
            const text = typeof nodeRecord.text === 'string' ? nodeRecord.text : '';
            return pos + unicodeScalarLength(text);
        }

        if (nodeType === 'mention') {
            const attrs = normalizeMentionAttrs(nodeRecord);
            const label = baseMentionLabelFromAttrs(attrs);
            const mentionContext = { docPos: pos, label, attrs };
            const renderedLabel = applyMentionPrefix(
                label,
                resolveMentionPrefix(mentionPrefix, mentionContext)
            );
            const mentionTheme = resolveMentionTheme?.(mentionContext) ?? undefined;
            mentions.set(pos, {
                ...mentionContext,
                renderedLabel,
                mentionTheme,
            });
        }

        if (isRoot && nodeType === 'doc') {
            let nextPos = pos;
            for (const child of content) {
                nextPos = visit(child, nextPos);
            }
            return nextPos;
        }

        if (content.length === 0) {
            return pos + 1;
        }

        let nextPos = pos + 1;
        for (const child of content) {
            nextPos = visit(child, nextPos);
        }
        return nextPos + 1;
    };

    visit(document, 0, true);
    return mentions;
}

function applyResolvedMentionRendering(
    renderJson: string,
    mentionPayloadsByDocPos: Map<number, ResolvedMentionPayload>
): string {
    if (mentionPayloadsByDocPos.size === 0) {
        return renderJson;
    }

    let parsedElements: unknown;
    try {
        parsedElements = JSON.parse(renderJson);
    } catch {
        return renderJson;
    }
    if (!Array.isArray(parsedElements)) {
        return renderJson;
    }

    let didChange = false;
    const nextElements = parsedElements.map((element) => {
        if (element == null || typeof element !== 'object' || Array.isArray(element)) {
            return element;
        }

        const renderElement = element as RenderElement;
        if (
            renderElement.type !== 'opaqueInlineAtom' ||
            renderElement.nodeType !== 'mention' ||
            typeof renderElement.docPos !== 'number'
        ) {
            return element;
        }

        const mention = mentionPayloadsByDocPos.get(renderElement.docPos);
        if (!mention) {
            return element;
        }

        let nextElement = renderElement;
        if (renderElement.label !== mention.renderedLabel) {
            nextElement = { ...nextElement, label: mention.renderedLabel };
            didChange = true;
        }

        if (mention.mentionTheme && Object.keys(mention.mentionTheme).length > 0) {
            nextElement =
                nextElement === renderElement ? { ...nextElement } : nextElement;
            nextElement.mentionTheme = mention.mentionTheme;
            didChange = true;
        }

        return nextElement;
    });

    return didChange ? JSON.stringify(nextElements) : renderJson;
}

function serializeDocumentInput(
    document: NativeProseViewerContent,
    schema: SchemaDefinition
): {
    normalizedDocument: DocumentJSON | null;
    serializedContentJson: string;
} {
    if (typeof document === 'string') {
        try {
            const parsed = JSON.parse(document) as DocumentJSON;
            const normalizedDocument = normalizeDocumentJson(parsed, schema);
            return {
                normalizedDocument,
                serializedContentJson: stringifyCachedJson(normalizedDocument),
            };
        } catch {
            return {
                normalizedDocument: null,
                serializedContentJson: document,
            };
        }
    }

    const normalizedDocument = normalizeDocumentJson(document, schema);
    return {
        normalizedDocument,
        serializedContentJson: stringifyCachedJson(normalizedDocument),
    };
}

function extractRenderError(json: string): string | null {
    try {
        const parsed = JSON.parse(json) as unknown;
        if (parsed == null || typeof parsed !== 'object' || Array.isArray(parsed)) {
            return null;
        }
        const error = (parsed as Record<string, unknown>).error;
        return typeof error === 'string' ? error : null;
    } catch {
        return null;
    }
}

export function NativeProseViewer({
    contentJSON,
    contentJSONRevision,
    schema,
    theme,
    style,
    allowBase64Images = false,
    mentionPrefix,
    resolveMentionTheme,
    onPressMention,
}: NativeProseViewerProps) {
    const documentSchema = useMemo(
        () => withMentionsSchema(schema ?? tiptapSchema),
        [schema]
    );
    const { normalizedDocument, serializedContentJson } = useMemo(
        () => serializeDocumentInput(contentJSON, documentSchema),
        [contentJSON, contentJSONRevision, documentSchema]
    );
    const themeJson = useMemo(() => serializeEditorTheme(theme), [theme]);
    const mentionPayloadsByDocPos = useMemo(
        () =>
            normalizedDocument == null
                ? new Map<number, ResolvedMentionPayload>()
                : collectMentionPayloadsByDocPos(
                      normalizedDocument,
                      mentionPrefix,
                      resolveMentionTheme
                  ),
        [mentionPrefix, normalizedDocument, resolveMentionTheme]
    );
    const renderJson = useMemo(() => {
        const configJson = JSON.stringify({
            schema: documentSchema,
            ...(allowBase64Images ? { allowBase64Images } : {}),
        });
        const nextRenderJson = getNativeProseViewerModule().renderDocumentJson(
            configJson,
            serializedContentJson
        );
        const renderError = extractRenderError(nextRenderJson);
        if (renderError != null) {
            console.error(`NativeProseViewer: ${renderError}`);
            return '[]';
        }
        if (looksLikeRenderElementsJson(nextRenderJson)) {
            return applyResolvedMentionRendering(
                nextRenderJson,
                mentionPayloadsByDocPos
            );
        }
        console.error(
            'NativeProseViewer: native renderDocumentJson returned an invalid payload.'
        );
        return '[]';
    }, [
        allowBase64Images,
        documentSchema,
        mentionPayloadsByDocPos,
        serializedContentJson,
    ]);
    const [contentHeight, setContentHeight] = useState<number | null>(null);
    const allowContentHeightShrinkRef = useRef(true);

    useEffect(() => {
        allowContentHeightShrinkRef.current = true;
    }, [contentJSONRevision, renderJson, themeJson]);

    const handleContentHeightChange = useCallback(
        (
            event: NativeSyntheticEvent<NativeProseViewerContentHeightEvent>
        ) => {
            const nextHeight = event.nativeEvent.contentHeight;
            if (nextHeight <= 0) return;
            setContentHeight((currentHeight) =>
                currentHeight == null ||
                nextHeight >= currentHeight ||
                allowContentHeightShrinkRef.current
                    ? (() => {
                          allowContentHeightShrinkRef.current = false;
                          return currentHeight === nextHeight
                              ? currentHeight
                              : nextHeight;
                      })()
                    : currentHeight
            );
        },
        []
    );

    const handlePressMention = useCallback(
        (
            event: NativeSyntheticEvent<NativeProseViewerMentionPressNativeEvent>
        ) => {
            if (!onPressMention) return;

            const { docPos, label } = event.nativeEvent;
            const resolvedMention = mentionPayloadsByDocPos.get(docPos);
            onPressMention({
                docPos,
                label: resolvedMention?.renderedLabel ?? label,
                attrs: resolvedMention?.attrs ?? {},
            });
        },
        [mentionPayloadsByDocPos, onPressMention]
    );

    const nativeStyle = useMemo(
        () => [
            { minHeight: 1 },
            style,
            contentHeight != null ? { minHeight: contentHeight } : null,
        ],
        [contentHeight, style]
    );

    return (
        <NativeProseViewerView
            style={nativeStyle}
            renderJson={renderJson}
            themeJson={themeJson}
            onContentHeightChange={handleContentHeightChange}
            onPressMention={
                typeof onPressMention === 'function' ? handlePressMention : undefined
            }
        />
    );
}
