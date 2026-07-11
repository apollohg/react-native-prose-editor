import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { requireNativeModule, requireNativeViewManager } from 'expo-modules-core';
import {
    serializeEditorImageLoadingPolicy,
    type EditorImageLoadingPolicy,
} from './ImageLoadingPolicy';
import {
    type NativeSyntheticEvent,
    PixelRatio,
    Platform,
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
    getHeightCache,
    setHeightCache,
    computeRenderJsonHash,
    computeLayoutContextKey,
} from './heightCache';
import {
    normalizeDocumentJson,
    tiptapSchema,
    type SchemaDefinition,
} from './schemas';

interface NativeProseViewerModule {
    renderDocumentJson(configJson: string, json: string): string;
    renderDocumentHtml(configJson: string, html: string): string;
    measureContentHeight(renderJson: string, themeJson: string | undefined, width: number): number;
}

interface NativeProseViewerViewProps {
    style?: StyleProp<ViewStyle>;
    renderJson: string;
    themeJson?: string;
    imageLoadingPolicyJson?: string;
    collapsesWhenEmpty?: boolean;
    enableLinkTaps?: boolean;
    interceptLinkTaps?: boolean;
    onContentHeightChange?: (
        event: NativeSyntheticEvent<NativeProseViewerContentHeightEvent>
    ) => void;
    onPressLink?: (
        event: NativeSyntheticEvent<NativeProseViewerLinkPressNativeEvent>
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

interface NativeProseViewerLinkPressNativeEvent {
    href: string;
    text: string;
}

export interface NativeProseViewerMentionRenderContext {
    docPos: number;
    label: string;
    attrs: Record<string, unknown>;
}

export interface NativeProseViewerMentionPressEvent
    extends NativeProseViewerMentionRenderContext {}

export interface NativeProseViewerLinkPressEvent {
    href: string;
    text: string;
}

type NativeProseViewerContent = DocumentJSON | string;
export type NativeProseViewerMentionPrefix =
    | string
    | ((mention: NativeProseViewerMentionRenderContext) => string | null | undefined);

export interface NativeProseViewerMentionsAddonConfig {
    trigger?: string;
    prefix?: NativeProseViewerMentionPrefix;
    theme?: EditorMentionTheme;
    resolveTheme?: (
        mention: NativeProseViewerMentionRenderContext
    ) => EditorMentionTheme | null | undefined;
    onPress?: (event: NativeProseViewerMentionPressEvent) => void;
}

export interface NativeProseViewerAddons {
    mentions?: NativeProseViewerMentionsAddonConfig;
}

interface NativeProseViewerBaseProps {
    contentRevision?: string | number;
    contentJSONRevision?: string | number;
    schema?: SchemaDefinition;
    theme?: EditorTheme;
    style?: StyleProp<ViewStyle>;
    allowBase64Images?: boolean;
    imageLoadingPolicy?: EditorImageLoadingPolicy;
    collapseTrailingEmptyParagraphs?: boolean;
    enableLinkTaps?: boolean;
    addons?: NativeProseViewerAddons;
    onPressLink?: (event: NativeProseViewerLinkPressEvent) => void;
    contentId?: string;
    containerWidth?: number;
}

interface NativeProseViewerJsonProps extends NativeProseViewerBaseProps {
    contentJSON: NativeProseViewerContent;
    contentHTML?: never;
}

interface NativeProseViewerHtmlProps extends NativeProseViewerBaseProps {
    contentHTML: string;
    contentJSON?: never;
}

export type NativeProseViewerProps =
    | NativeProseViewerJsonProps
    | NativeProseViewerHtmlProps;

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
const EMPTY_TEXT_BLOCK_PLACEHOLDER = '\u200B';

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

function resolveConfiguredMentionPrefix(
    prefix: NativeProseViewerMentionPrefix | undefined,
    mention: NativeProseViewerMentionRenderContext
): string | undefined {
    const rawPrefix =
        typeof prefix === 'function' ? prefix(mention) : prefix;
    return typeof rawPrefix === 'string' && rawPrefix.length > 0 ? rawPrefix : undefined;
}

function mentionTriggerFromAttrs(attrs: Record<string, unknown>): string | undefined {
    const trigger = attrs.mentionSuggestionChar;
    return typeof trigger === 'string' && trigger.length > 0 ? trigger : undefined;
}

function applyMentionPrefix(label: string, prefix: string | undefined): string {
    if (!prefix || label.startsWith(prefix)) {
        return label;
    }
    return `${prefix}${label}`;
}

function resolveMentionRenderedLabel(
    mentionContext: NativeProseViewerMentionRenderContext,
    prefix: NativeProseViewerMentionPrefix | undefined,
    trigger: string | undefined
): string {
    if (prefix !== undefined) {
        return applyMentionPrefix(
            mentionContext.label,
            resolveConfiguredMentionPrefix(prefix, mentionContext)
        );
    }

    return applyMentionPrefix(
        mentionContext.label,
        trigger ?? mentionTriggerFromAttrs(mentionContext.attrs)
    );
}

interface ResolvedMentionPayload extends NativeProseViewerMentionRenderContext {
    renderedLabel: string;
    mentionTheme?: EditorMentionTheme;
}

function collectMentionPayloadsByDocPos(
    document: DocumentJSON,
    mentionsAddon: NativeProseViewerMentionsAddonConfig | undefined
): Map<number, ResolvedMentionPayload> {
    const mentions = new Map<number, ResolvedMentionPayload>();
    const effectiveMentionPrefix = mentionsAddon?.prefix;
    const effectiveResolveMentionTheme = mentionsAddon?.resolveTheme;
    const defaultMentionTheme = mentionsAddon?.theme;
    const trigger = mentionsAddon?.trigger?.trim() || undefined;

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
            const renderedLabel = resolveMentionRenderedLabel(
                mentionContext,
                effectiveMentionPrefix,
                trigger
            );
            const resolvedMentionTheme =
                effectiveResolveMentionTheme?.(mentionContext) ?? undefined;
            const mentionTheme =
                defaultMentionTheme || resolvedMentionTheme
                    ? {
                          ...(defaultMentionTheme ?? {}),
                          ...(resolvedMentionTheme ?? {}),
                      }
                    : undefined;
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

function isTopLevelSingleElementBlock(element: RenderElement): boolean {
    return element.type === 'voidBlock' || element.type === 'opaqueBlockAtom';
}

function isEmptyParagraphPlaceholderText(text: string): boolean {
    if (text.length === 0) {
        return false;
    }
    return Array.from(text).every((char) => char === EMPTY_TEXT_BLOCK_PLACEHOLDER);
}

function isCollapsibleEmptyParagraphText(text: string): boolean {
    return Array.from(text).every((char) => char === EMPTY_TEXT_BLOCK_PLACEHOLDER);
}

function renderElementsJsonContainsOnlyEmptyParagraphs(renderJson: string): boolean {
    let parsedElements: unknown;
    try {
        parsedElements = JSON.parse(renderJson);
    } catch {
        return false;
    }
    if (!Array.isArray(parsedElements)) {
        return false;
    }
    if (parsedElements.length === 0) {
        return true;
    }

    let hasParagraph = false;
    let paragraphIsOpen = false;

    for (const element of parsedElements) {
        if (element == null || typeof element !== 'object' || Array.isArray(element)) {
            return false;
        }

        const renderElement = element as RenderElement;
        switch (renderElement.type) {
            case 'blockStart':
                if (
                    paragraphIsOpen ||
                    renderElement.nodeType !== 'paragraph' ||
                    renderElement.depth !== 0
                ) {
                    return false;
                }
                paragraphIsOpen = true;
                hasParagraph = true;
                break;
            case 'textRun':
                if (
                    !paragraphIsOpen ||
                    typeof renderElement.text !== 'string' ||
                    !isCollapsibleEmptyParagraphText(renderElement.text)
                ) {
                    return false;
                }
                break;
            case 'blockEnd':
                if (!paragraphIsOpen) {
                    return false;
                }
                paragraphIsOpen = false;
                break;
            default:
                return false;
        }
    }

    return hasParagraph && !paragraphIsOpen;
}

function isTrailingEmptyParagraphRange(
    elements: RenderElement[],
    start: number,
    endExclusive: number
): boolean {
    const startElement = elements[start];
    const endElement = elements[endExclusive - 1];
    if (
        startElement?.type !== 'blockStart' ||
        startElement.nodeType !== 'paragraph' ||
        startElement.depth !== 0 ||
        endElement?.type !== 'blockEnd'
    ) {
        return false;
    }

    const innerElements = elements.slice(start + 1, endExclusive - 1);
    return (
        innerElements.length > 0 &&
        innerElements.every(
            (element) =>
                element.type === 'textRun' &&
                typeof element.text === 'string' &&
                isEmptyParagraphPlaceholderText(element.text)
        )
    );
}

function collapseTrailingEmptyParagraphRenderElements(renderJson: string): string {
    let parsedElements: unknown;
    try {
        parsedElements = JSON.parse(renderJson);
    } catch {
        return renderJson;
    }
    if (!Array.isArray(parsedElements)) {
        return renderJson;
    }

    const elements = parsedElements as RenderElement[];
    const topLevelRanges: Array<{ start: number; endExclusive: number }> = [];

    for (let index = 0; index < elements.length; index += 1) {
        const element = elements[index];
        if (!element || typeof element !== 'object' || Array.isArray(element)) {
            continue;
        }

        if (element.type === 'blockStart' && element.depth === 0) {
            let nestingDepth = 1;
            let cursor = index + 1;
            while (cursor < elements.length && nestingDepth > 0) {
                const current = elements[cursor];
                if (current?.type === 'blockStart') {
                    nestingDepth += 1;
                } else if (current?.type === 'blockEnd') {
                    nestingDepth -= 1;
                }
                cursor += 1;
            }
            if (nestingDepth !== 0) {
                return renderJson;
            }
            topLevelRanges.push({ start: index, endExclusive: cursor });
            index = cursor - 1;
            continue;
        }

        if (isTopLevelSingleElementBlock(element)) {
            topLevelRanges.push({ start: index, endExclusive: index + 1 });
        }
    }

    if (topLevelRanges.length <= 1) {
        return renderJson;
    }

    let trimStart: number | null = null;
    for (let rangeIndex = topLevelRanges.length - 1; rangeIndex >= 1; rangeIndex -= 1) {
        const range = topLevelRanges[rangeIndex];
        if (!isTrailingEmptyParagraphRange(elements, range.start, range.endExclusive)) {
            break;
        }
        trimStart = range.start;
    }

    if (trimStart == null) {
        return renderJson;
    }

    return JSON.stringify(elements.slice(0, trimStart));
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
    ...props
}: NativeProseViewerProps) {
    const {
        contentRevision,
        contentJSONRevision,
        schema,
        theme,
        style,
        allowBase64Images = false,
        imageLoadingPolicy,
        collapseTrailingEmptyParagraphs = true,
        enableLinkTaps = true,
        addons,
        onPressLink,
        contentId,
        containerWidth,
    } = props;
    const mentionPressHandler = addons?.mentions?.onPress;
    const contentJSON = 'contentJSON' in props ? props.contentJSON : undefined;
    const contentHTML = 'contentHTML' in props ? props.contentHTML : undefined;
    const resolvedContentRevision = contentRevision ?? contentJSONRevision;
    const documentSchema = useMemo(
        () => withMentionsSchema(schema ?? tiptapSchema),
        [schema]
    );
    const { normalizedDocument, serializedContentJson } = useMemo(() => {
        if (contentJSON === undefined) {
            return {
                normalizedDocument: null,
                serializedContentJson: null,
            };
        }
        return serializeDocumentInput(contentJSON, documentSchema);
    }, [contentJSON, resolvedContentRevision, documentSchema]);
    const themeJson = useMemo(() => serializeEditorTheme(theme), [theme]);
    const imageLoadingPolicyJson = useMemo(
        () => serializeEditorImageLoadingPolicy(imageLoadingPolicy),
        [imageLoadingPolicy]
    );
    const mentionPayloadsByDocPos = useMemo(
        () =>
            normalizedDocument == null
                ? new Map<number, ResolvedMentionPayload>()
                : collectMentionPayloadsByDocPos(
                      normalizedDocument,
                      addons?.mentions
                  ),
        [addons?.mentions, normalizedDocument]
    );
    const renderJson = useMemo(() => {
        const configJson = JSON.stringify({
            schema: documentSchema,
            ...(allowBase64Images ? { allowBase64Images } : {}),
        });
        const nextRenderJson =
            serializedContentJson != null
                ? getNativeProseViewerModule().renderDocumentJson(
                      configJson,
                      serializedContentJson
                  )
                : getNativeProseViewerModule().renderDocumentHtml(
                      configJson,
                      contentHTML ?? ''
                  );
        const renderError = extractRenderError(nextRenderJson);
        if (renderError != null) {
            console.error(`NativeProseViewer: ${renderError}`);
            return '[]';
        }
        if (looksLikeRenderElementsJson(nextRenderJson)) {
            const collapsedRenderJson = collapseTrailingEmptyParagraphs
                ? collapseTrailingEmptyParagraphRenderElements(nextRenderJson)
                : nextRenderJson;
            return applyResolvedMentionRendering(
                collapsedRenderJson,
                mentionPayloadsByDocPos
            );
        }
        console.error(
            'NativeProseViewer: native renderDocumentJson returned an invalid payload.'
        );
        return '[]';
    }, [
        allowBase64Images,
        collapseTrailingEmptyParagraphs,
        contentHTML,
        documentSchema,
        mentionPayloadsByDocPos,
        serializedContentJson,
    ]);
    const renderJsonIsCollapsedEmpty = useMemo(
        () =>
            collapseTrailingEmptyParagraphs &&
            renderElementsJsonContainsOnlyEmptyParagraphs(renderJson),
        [collapseTrailingEmptyParagraphs, renderJson]
    );
    const [contentHeight, setContentHeight] = useState<number | null>(null);

    useEffect(() => {
        setContentHeight(null);
    }, [contentId]);

    const renderJsonHash = useMemo(
        () => computeRenderJsonHash(renderJson),
        [renderJson]
    );

    const layoutContextKey = useMemo(
        () => containerWidth != null ? computeLayoutContextKey(themeJson, containerWidth) : null,
        [themeJson, containerWidth]
    );

    const preMeasuredHeight = useMemo(() => {
        if (!contentId || layoutContextKey == null || containerWidth == null) {
            return null;
        }

        const cached = getHeightCache(contentId, layoutContextKey, renderJsonHash);
        if (cached != null) return cached;

        const measured = getNativeProseViewerModule().measureContentHeight(
            renderJson,
            themeJson,
            containerWidth
        );
        if (measured > 0) {
            setHeightCache(contentId, layoutContextKey, renderJsonHash, measured);
        }
        return measured > 0 ? measured : null;
    }, [contentId, containerWidth, renderJson, themeJson, layoutContextKey, renderJsonHash]);

    const handleContentHeightChange = useCallback(
        (
            event: NativeSyntheticEvent<NativeProseViewerContentHeightEvent>
        ) => {
            const density = Platform.OS === 'android' ? PixelRatio.get() : 1;
            const nextHeight = Math.ceil(event.nativeEvent.contentHeight / density);
            if (nextHeight < 0) return;
            if (nextHeight === 0 && !renderJsonIsCollapsedEmpty) return;
            if (nextHeight === 0) {
                setContentHeight((currentHeight) =>
                    currentHeight === 0 ? currentHeight : 0
                );
                return;
            }
            setContentHeight((currentHeight) =>
                currentHeight === nextHeight ? currentHeight : nextHeight
            );
            if (contentId && layoutContextKey != null) {
                setHeightCache(contentId, layoutContextKey, renderJsonHash, nextHeight);
            }
        },
        [renderJsonIsCollapsedEmpty, contentId, layoutContextKey, renderJsonHash]
    );

    const handlePressMention = useCallback(
        (
            event: NativeSyntheticEvent<NativeProseViewerMentionPressNativeEvent>
        ) => {
            if (!mentionPressHandler) return;

            const { docPos, label } = event.nativeEvent;
            const resolvedMention = mentionPayloadsByDocPos.get(docPos);
            mentionPressHandler({
                docPos,
                label: resolvedMention?.renderedLabel ?? label,
                attrs: resolvedMention?.attrs ?? {},
            });
        },
        [mentionPayloadsByDocPos, mentionPressHandler]
    );
    const handlePressLink = useCallback(
        (event: NativeSyntheticEvent<NativeProseViewerLinkPressNativeEvent>) => {
            if (!onPressLink) return;

            onPressLink({
                href: event.nativeEvent.href,
                text: event.nativeEvent.text,
            });
        },
        [onPressLink]
    );

    const nativeStyle = useMemo(() => {
        let measuredStyle: ViewStyle | null = null;
        if (renderJsonIsCollapsedEmpty) {
            measuredStyle = { height: 0, minHeight: 0 };
        } else if (contentHeight != null && contentHeight > 0) {
            measuredStyle = { minHeight: contentHeight };
        } else if (preMeasuredHeight != null && preMeasuredHeight > 0) {
            measuredStyle = { minHeight: preMeasuredHeight };
        }
        return [
            { minHeight: renderJsonIsCollapsedEmpty ? 0 : 1 },
            style,
            measuredStyle,
        ];
    }, [contentHeight, preMeasuredHeight, renderJsonIsCollapsedEmpty, style]);

    return (
        <NativeProseViewerView
            style={nativeStyle}
            renderJson={renderJson}
            themeJson={themeJson}
            imageLoadingPolicyJson={imageLoadingPolicyJson}
            collapsesWhenEmpty={collapseTrailingEmptyParagraphs}
            enableLinkTaps={enableLinkTaps}
            interceptLinkTaps={typeof onPressLink === 'function'}
            onContentHeightChange={handleContentHeightChange}
            onPressLink={typeof onPressLink === 'function' ? handlePressLink : undefined}
            onPressMention={
                typeof mentionPressHandler === 'function'
                    ? handlePressMention
                    : undefined
            }
        />
    );
}
