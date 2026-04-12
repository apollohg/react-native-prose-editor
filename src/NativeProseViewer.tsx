import React, { useCallback, useMemo, useState } from 'react';
import { requireNativeModule, requireNativeViewManager } from 'expo-modules-core';
import {
    type NativeSyntheticEvent,
    type StyleProp,
    type ViewStyle,
} from 'react-native';

import { withMentionsSchema } from './addons';
import { serializeEditorTheme, type EditorTheme } from './EditorTheme';
import type { DocumentJSON } from './NativeEditorBridge';
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

export interface NativeProseViewerMentionPressEvent {
    docPos: number;
    label: string;
    attrs: Record<string, unknown>;
}

type NativeProseViewerContent = DocumentJSON | string;

export interface NativeProseViewerProps {
    contentJSON: NativeProseViewerContent;
    contentJSONRevision?: string | number;
    schema?: SchemaDefinition;
    theme?: EditorTheme;
    style?: StyleProp<ViewStyle>;
    allowBase64Images?: boolean;
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

function collectMentionPayloadsByDocPos(
    document: DocumentJSON
): Map<number, { label?: string; attrs: Record<string, unknown> }> {
    const mentions = new Map<number, { label?: string; attrs: Record<string, unknown> }>();

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
            const label = typeof attrs.label === 'string' ? attrs.label : undefined;
            mentions.set(pos, { label, attrs });
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
                ? new Map<number, { label?: string; attrs: Record<string, unknown> }>()
                : collectMentionPayloadsByDocPos(normalizedDocument),
        [normalizedDocument]
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
            return nextRenderJson;
        }
        console.error(
            'NativeProseViewer: native renderDocumentJson returned an invalid payload.'
        );
        return '[]';
    }, [allowBase64Images, documentSchema, serializedContentJson]);
    const [contentHeight, setContentHeight] = useState<number | null>(null);

    const handleContentHeightChange = useCallback(
        (
            event: NativeSyntheticEvent<NativeProseViewerContentHeightEvent>
        ) => {
            const nextHeight = event.nativeEvent.contentHeight;
            setContentHeight((currentHeight) =>
                currentHeight === nextHeight ? currentHeight : nextHeight
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
                label: resolvedMention?.label ?? label,
                attrs: resolvedMention?.attrs ?? {},
            });
        },
        [mentionPayloadsByDocPos, onPressMention]
    );

    const nativeStyle = useMemo(
        () => [{ minHeight: 1 }, style, contentHeight != null ? { height: contentHeight } : null],
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
