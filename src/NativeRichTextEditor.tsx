import React, {
    forwardRef,
    useEffect,
    useCallback,
    useImperativeHandle,
    useMemo,
    useRef,
    useState,
} from 'react';
import {
    PixelRatio,
    Platform,
    Pressable,
    ScrollView,
    StyleSheet,
    Text,
    View,
    type NativeSyntheticEvent,
    type StyleProp,
    type ViewStyle,
} from 'react-native';
import { requireNativeViewManager } from 'expo-modules-core';

import {
    NativeEditorBridge,
    type ActiveState,
    type CommandBlockedInfo,
    type DocumentJSON,
    type EditorUpdate,
    type HistoryState,
    type RenderElement,
    type Selection,
} from './NativeEditorBridge';
import {
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    EditorToolbar,
    setActiveEditorToolbarFrameOwnerForEditor,
    setEditorToolbarMentionState,
    useEditorToolbarFrames,
    type EditorToolbarCommand,
    type EditorToolbarFrame,
    type EditorToolbarGroupChildItem,
    type EditorToolbarHeadingLevel,
    type EditorToolbarIcon,
    type EditorToolbarItem,
    type EditorToolbarListType,
} from './EditorToolbar';
import { serializeEditorTheme, type EditorMentionTheme, type EditorTheme } from './EditorTheme';
import {
    buildMentionFragmentJson,
    serializeEditorAddons,
    type EditorAddonEvent,
    type EditorAddons,
    type MentionQueryChangeEvent,
    type MentionSelectionAttrsEvent,
    type MentionSuggestion,
    withMentionsSchema,
} from './addons';
import {
    IMAGE_NODE_NAME,
    buildImageFragmentJson,
    normalizeDocumentJson,
    tiptapSchema,
    type ImageNodeAttributes,
    type SchemaDefinition,
} from './schemas';

interface NativeEditorViewHandle {
    focus?: () => void;
    blur?: () => void;
    getCaretRect?: () => Promise<string | null> | string | null;
    applyEditorUpdate: (updateJson: string) => boolean | void | Promise<boolean | void>;
    applyEditorResetUpdate?: (updateJson: string) => boolean | void | Promise<boolean | void>;
}

interface NativeEditorViewProps {
    style?: StyleProp<ViewStyle>;
    editorId: number;
    placeholder?: string;
    editable: boolean;
    autoFocus: boolean;
    autoCapitalize?: NativeRichTextEditorAutoCapitalize;
    autoCorrect?: boolean;
    keyboardType?: NativeRichTextEditorKeyboardType;
    showToolbar: boolean;
    toolbarPlacement: NativeRichTextEditorToolbarPlacement;
    heightBehavior: NativeRichTextEditorHeightBehavior;
    allowImageResizing: boolean;
    themeJson?: string;
    addonsJson?: string;
    toolbarItemsJson?: string;
    toolbarFrameJson?: string;
    remoteSelectionsJson?: string;
    editorUpdateJson?: string;
    editorUpdateEditorId?: number;
    editorUpdateRevision?: number;
    editorResetUpdateJson?: string;
    editorResetUpdateEditorId?: number;
    editorResetUpdateRevision?: number;
    onEditorUpdate: (event: NativeSyntheticEvent<NativeUpdateEvent>) => void;
    onSelectionChange: (event: NativeSyntheticEvent<NativeSelectionEvent>) => void;
    onFocusChange: (event: NativeSyntheticEvent<NativeFocusEvent>) => void;
    onContentHeightChange: (event: NativeSyntheticEvent<NativeContentHeightEvent>) => void;
    onEditorReady?: (event: NativeSyntheticEvent<NativeEditorReadyEvent>) => void;
    onToolbarAction: (event: NativeSyntheticEvent<NativeToolbarActionEvent>) => void;
    onAddonEvent: (event: NativeSyntheticEvent<NativeAddonEvent>) => void;
}

const NativeEditorView = requireNativeViewManager('NativeEditor') as React.ComponentType<
    NativeEditorViewProps & React.RefAttributes<NativeEditorViewHandle>
>;

const DEV_NATIVE_VIEW_KEY = __DEV__
    ? `native-editor-dev:${Math.random().toString(36).slice(2)}`
    : 'native-editor';
const LINK_TOOLBAR_ACTION_KEY = '__native-editor-link__';
const IMAGE_TOOLBAR_ACTION_KEY = '__native-editor-image__';
const DEFAULT_MENTION_TRIGGER = '@';
const MAX_INLINE_MENTION_SUGGESTIONS = 8;
const INLINE_TOOLBAR_BORDER_COLOR = '#E5E5EA';

function mapToolbarChildForNative(
    item: EditorToolbarGroupChildItem,
    activeState: ActiveState,
    editable: boolean,
    onRequestLink?: NativeRichTextEditorProps['onRequestLink'],
    onRequestImage?: NativeRichTextEditorProps['onRequestImage']
): EditorToolbarGroupChildItem {
    if (item.type === 'link') {
        return {
            type: 'action',
            key: LINK_TOOLBAR_ACTION_KEY,
            label: item.label,
            icon: item.icon as EditorToolbarIcon,
            placement: item.placement,
            isActive: activeState.marks.link === true,
            isDisabled: !editable || !onRequestLink || !activeState.allowedMarks.includes('link'),
        };
    }
    if (item.type === 'image') {
        return {
            type: 'action',
            key: IMAGE_TOOLBAR_ACTION_KEY,
            label: item.label,
            icon: item.icon as EditorToolbarIcon,
            placement: item.placement,
            isActive: false,
            isDisabled:
                !editable ||
                !onRequestImage ||
                !activeState.insertableNodes.includes(IMAGE_NODE_NAME),
        };
    }
    return item;
}

function mapToolbarItemsForNative(
    items: readonly EditorToolbarItem[],
    activeState: ActiveState,
    editable: boolean,
    onRequestLink?: NativeRichTextEditorProps['onRequestLink'],
    onRequestImage?: NativeRichTextEditorProps['onRequestImage']
): EditorToolbarItem[] {
    return items.map((item) => {
        if (item.type === 'group') {
            return {
                ...item,
                items: item.items.map((child) =>
                    mapToolbarChildForNative(
                        child,
                        activeState,
                        editable,
                        onRequestLink,
                        onRequestImage
                    )
                ),
            };
        }
        if (item.type === 'separator') {
            return item;
        }
        return mapToolbarChildForNative(item, activeState, editable, onRequestLink, onRequestImage);
    });
}

function isImageDataUrl(value: string): boolean {
    return /^data:image\//i.test(value.trim());
}

function isRetryableNativeCommandBlock(reason: CommandBlockedInfo['reason']): boolean {
    return reason === 'composition' || (Platform.OS === 'android' && reason === 'pendingUpdate');
}

function restoreSelectionInBridge(bridge: NativeEditorBridge, selection: Selection): boolean {
    if (selection.type === 'text') {
        const { anchor, head } = selection;
        if (anchor == null || head == null) {
            return false;
        }
        bridge.setSelection(anchor, head);
        return true;
    }

    if (selection.type === 'node') {
        const { pos } = selection;
        if (pos == null) {
            return false;
        }
        bridge.setSelection(pos, pos);
        return true;
    }

    return false;
}

function isPromiseLike(value: unknown): value is Promise<unknown> {
    return (
        value != null &&
        typeof value === 'object' &&
        'then' in value &&
        typeof (value as Promise<unknown>).then === 'function'
    );
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return value != null && typeof value === 'object' && !Array.isArray(value);
}

function resolveMentionTrigger(addons?: EditorAddons): string {
    return addons?.mentions?.trigger?.trim() || DEFAULT_MENTION_TRIGGER;
}

function resolveMentionSuggestionLabel(suggestion: MentionSuggestion): string {
    return suggestion.label?.trim() || suggestion.title;
}

function resolveMentionSuggestionDisplayLabel(
    suggestion: MentionSuggestion,
    trigger: string
): string {
    const label = resolveMentionSuggestionLabel(suggestion);
    return trigger.length > 0 && !label.startsWith(trigger) ? `${trigger}${label}` : label;
}

function filterMentionSuggestions(
    suggestions: readonly MentionSuggestion[],
    query: string,
    trigger: string
): MentionSuggestion[] {
    const normalizedQuery = query.trim().toLowerCase();
    const filtered =
        normalizedQuery.length === 0
            ? suggestions
            : suggestions.filter((suggestion) => {
                  const label = resolveMentionSuggestionLabel(suggestion);
                  const displayLabel = resolveMentionSuggestionDisplayLabel(suggestion, trigger);
                  return (
                      suggestion.title.toLowerCase().includes(normalizedQuery) ||
                      label.toLowerCase().includes(normalizedQuery) ||
                      displayLabel.toLowerCase().includes(normalizedQuery) ||
                      suggestion.subtitle?.toLowerCase().includes(normalizedQuery) === true
                  );
              });
    return filtered.slice(0, MAX_INLINE_MENTION_SUGGESTIONS);
}

function resolveMentionSuggestionAttrs(
    suggestion: MentionSuggestion,
    trigger: string
): Record<string, unknown> {
    const attrs = { ...(suggestion.attrs ?? {}) };
    if (!('label' in attrs)) {
        attrs.label = resolveMentionSuggestionLabel(suggestion);
    }
    if (!('mentionSuggestionChar' in attrs)) {
        attrs.mentionSuggestionChar = trigger;
    }
    return attrs;
}

function mergeMentionSuggestionTheme(
    baseTheme: EditorMentionTheme | undefined,
    resolvedTheme: EditorMentionTheme | undefined
): EditorMentionTheme | undefined {
    if (baseTheme == null && resolvedTheme == null) {
        return undefined;
    }

    const merged: EditorMentionTheme = {
        ...(baseTheme ?? {}),
        ...(resolvedTheme ?? {}),
    };

    if (resolvedTheme?.textColor != null && resolvedTheme.optionTextColor == null) {
        merged.optionTextColor = resolvedTheme.textColor;
    }

    return merged;
}

interface AutoLinkCandidate {
    docFrom: number;
    docTo: number;
    href: string;
}

interface AutoLinkInlineBlock {
    chars: string[];
    docPositions: number[];
    linked: boolean[];
    contentStart: number;
    contentEnd: number;
    nextPos: number;
}

const AUTO_LINK_URL_REGEX = /(?:https?:\/\/|www\.)\S+/giu;
const AUTO_LINK_INLINE_PLACEHOLDER = '\uFFFC';
const AUTO_LINK_LEADING_BOUNDARY_CHARS = new Set(['(', '[', '{', '<', '"', "'"]);
const AUTO_LINK_TRAILING_DELIMITER_CHARS = new Set([
    '.',
    ',',
    '!',
    '?',
    ';',
    ':',
    ')',
    ']',
    '}',
    '>',
    '"',
    "'",
]);
const AUTO_LINK_ALWAYS_TRIM_CHARS = new Set(['.', ',', '!', '?', ';', ':']);
const AUTO_LINK_MATCHED_CLOSERS: Record<string, string> = {
    ')': '(',
    ']': '[',
    '}': '{',
};

function unicodeScalars(text: string): string[] {
    return Array.from(text);
}

function unicodeScalarCount(text: string): number {
    return unicodeScalars(text).length;
}

function hasDocumentLinkMark(marks: unknown): boolean {
    if (!Array.isArray(marks)) {
        return false;
    }
    return marks.some(
        (mark) => isRecord(mark) && typeof mark.type === 'string' && mark.type === 'link'
    );
}

function isInlineDocumentNode(node: unknown): boolean {
    if (!isRecord(node)) {
        return false;
    }
    if (node.type === 'text') {
        return true;
    }
    return !Array.isArray(node.content);
}

function isInlineTextBlockNode(node: Record<string, unknown>): boolean {
    const content = Array.isArray(node.content) ? node.content : [];
    return content.length > 0 && content.every((child) => isInlineDocumentNode(child));
}

function countOccurrences(text: string, target: string): number {
    let count = 0;
    for (const char of text) {
        if (char === target) {
            count += 1;
        }
    }
    return count;
}

function trimAutoLinkTrailingPunctuation(value: string): string {
    let result = value;

    while (result.length > 0) {
        const chars = unicodeScalars(result);
        const lastChar = chars[chars.length - 1];
        if (!lastChar) {
            break;
        }

        if (AUTO_LINK_ALWAYS_TRIM_CHARS.has(lastChar)) {
            chars.pop();
            result = chars.join('');
            continue;
        }

        const matchingOpener = AUTO_LINK_MATCHED_CLOSERS[lastChar];
        if (
            matchingOpener &&
            countOccurrences(result, lastChar) > countOccurrences(result, matchingOpener)
        ) {
            chars.pop();
            result = chars.join('');
            continue;
        }

        if (
            (lastChar === '"' || lastChar === "'") &&
            countOccurrences(result, lastChar) % 2 !== 0
        ) {
            chars.pop();
            result = chars.join('');
            continue;
        }

        break;
    }

    return result;
}

function normalizeAutoDetectedHref(value: string): string | null {
    const trimmed = trimAutoLinkTrailingPunctuation(value);
    if (!trimmed) {
        return null;
    }

    const normalized = /^www\./iu.test(trimmed) ? `https://${trimmed}` : trimmed;
    try {
        new URL(normalized);
        return normalized;
    } catch {
        return null;
    }
}

function isAutoLinkBoundaryChar(char: string | undefined): boolean {
    if (!char) {
        return true;
    }
    return (
        /\s/u.test(char) ||
        char === AUTO_LINK_INLINE_PLACEHOLDER ||
        AUTO_LINK_LEADING_BOUNDARY_CHARS.has(char)
    );
}

function isAutoLinkTrailingDelimiterChar(char: string | undefined): boolean {
    if (!char) {
        return true;
    }
    return (
        /\s/u.test(char) ||
        char === AUTO_LINK_INLINE_PLACEHOLDER ||
        AUTO_LINK_TRAILING_DELIMITER_CHARS.has(char)
    );
}

function codeUnitBoundariesForScalars(chars: readonly string[]): number[] {
    const boundaries = [0];
    let offset = 0;
    for (const char of chars) {
        offset += char.length;
        boundaries.push(offset);
    }
    return boundaries;
}

function codeUnitOffsetToScalarIndex(boundaries: readonly number[], offset: number): number {
    let scalarIndex = 0;
    while (scalarIndex + 1 < boundaries.length && boundaries[scalarIndex + 1] <= offset) {
        scalarIndex += 1;
    }
    return scalarIndex;
}

function buildAutoLinkInlineBlock(node: Record<string, unknown>, pos: number): AutoLinkInlineBlock {
    const chars: string[] = [];
    const docPositions: number[] = [];
    const linked: boolean[] = [];
    const content = Array.isArray(node.content) ? node.content : [];
    let nextPos = pos + 1;

    for (const child of content) {
        if (!isRecord(child)) {
            continue;
        }

        if (child.type === 'text') {
            const childText = typeof child.text === 'string' ? child.text : '';
            const childChars = unicodeScalars(childText);
            const hasLink = hasDocumentLinkMark(child.marks);
            for (const char of childChars) {
                chars.push(char);
                docPositions.push(nextPos);
                linked.push(hasLink);
                nextPos += 1;
            }
            continue;
        }

        chars.push(child.type === 'hardBreak' ? '\n' : AUTO_LINK_INLINE_PLACEHOLDER);
        docPositions.push(nextPos);
        linked.push(false);
        nextPos += 1;
    }

    return {
        chars,
        docPositions,
        linked,
        contentStart: pos + 1,
        contentEnd: nextPos,
        nextPos: nextPos + 1,
    };
}

function findAutoLinkCandidateInInlineBlock(
    block: AutoLinkInlineBlock,
    cursorDocPos: number
): AutoLinkCandidate | null {
    if (cursorDocPos < block.contentStart || cursorDocPos > block.contentEnd) {
        return null;
    }

    let localIndex = 0;
    while (
        localIndex < block.docPositions.length &&
        block.docPositions[localIndex] < cursorDocPos
    ) {
        localIndex += 1;
    }

    if (localIndex === 0) {
        return null;
    }

    if (
        cursorDocPos < block.contentEnd &&
        !isAutoLinkTrailingDelimiterChar(block.chars[localIndex - 1])
    ) {
        return null;
    }

    const prefixChars = block.chars.slice(0, localIndex);
    const prefixText = prefixChars.join('');
    if (!prefixText) {
        return null;
    }

    const boundaries = codeUnitBoundariesForScalars(prefixChars);
    AUTO_LINK_URL_REGEX.lastIndex = 0;
    let lastMatch: RegExpExecArray | null = null;
    for (const match of prefixText.matchAll(AUTO_LINK_URL_REGEX)) {
        lastMatch = match;
    }

    if (!lastMatch || typeof lastMatch.index !== 'number') {
        return null;
    }

    const rawStartScalar = codeUnitOffsetToScalarIndex(boundaries, lastMatch.index);
    const normalizedHref = normalizeAutoDetectedHref(lastMatch[0]);
    const trimmedText = trimAutoLinkTrailingPunctuation(lastMatch[0]);
    if (!normalizedHref || !trimmedText) {
        return null;
    }

    const candidateEndScalar = rawStartScalar + unicodeScalarCount(trimmedText);
    if (candidateEndScalar > prefixChars.length) {
        return null;
    }

    if (!isAutoLinkBoundaryChar(prefixChars[rawStartScalar - 1])) {
        return null;
    }

    for (let index = candidateEndScalar; index < localIndex; index += 1) {
        if (!isAutoLinkTrailingDelimiterChar(prefixChars[index])) {
            return null;
        }
    }

    for (let index = rawStartScalar; index < candidateEndScalar; index += 1) {
        if (block.linked[index]) {
            return null;
        }
    }

    const docFrom = block.docPositions[rawStartScalar];
    const docTo =
        candidateEndScalar < block.docPositions.length
            ? block.docPositions[candidateEndScalar]
            : block.contentEnd;

    if (!(docTo > docFrom)) {
        return null;
    }

    return {
        docFrom,
        docTo,
        href: normalizedHref,
    };
}

function findAutoLinkCandidateInDocument(
    document: DocumentJSON,
    cursorDocPos: number
): AutoLinkCandidate | null {
    const visit = (
        node: unknown,
        pos: number,
        isRoot = false
    ): { candidate: AutoLinkCandidate | null; nextPos: number } => {
        if (!isRecord(node)) {
            return { candidate: null, nextPos: pos };
        }

        const nodeType = typeof node.type === 'string' ? node.type : '';
        const content = Array.isArray(node.content) ? node.content : [];

        if (nodeType === 'text') {
            const text = typeof node.text === 'string' ? node.text : '';
            return { candidate: null, nextPos: pos + unicodeScalarCount(text) };
        }

        if (isRoot && nodeType === 'doc') {
            let nextPos = pos;
            for (const child of content) {
                const result = visit(child, nextPos);
                if (result.candidate) {
                    return result;
                }
                nextPos = result.nextPos;
            }
            return { candidate: null, nextPos };
        }

        if (isInlineTextBlockNode(node)) {
            const block = buildAutoLinkInlineBlock(node, pos);
            return {
                candidate: findAutoLinkCandidateInInlineBlock(block, cursorDocPos),
                nextPos: block.nextPos,
            };
        }

        if (content.length === 0) {
            return { candidate: null, nextPos: pos + 1 };
        }

        let nextPos = pos + 1;
        for (const child of content) {
            const result = visit(child, nextPos);
            if (result.candidate) {
                return result;
            }
            nextPos = result.nextPos;
        }

        return { candidate: null, nextPos: nextPos + 1 };
    };

    return visit(document, 0, true).candidate;
}

function didContentChange(
    previousDocumentVersion: number | null,
    update: EditorUpdate | null
): boolean {
    if (!update) {
        return false;
    }
    return (
        previousDocumentVersion == null ||
        typeof update.documentVersion !== 'number' ||
        update.documentVersion !== previousDocumentVersion
    );
}

function documentVersionFromUpdateJson(json: string): number | null {
    try {
        const parsed = JSON.parse(json) as { documentVersion?: unknown };
        return typeof parsed.documentVersion === 'number' ? parsed.documentVersion : null;
    } catch {
        return null;
    }
}

interface NativeUpdateEvent {
    updateJson: string;
    editorId?: number;
}

interface NativeSelectionEvent {
    anchor: number;
    head: number;
    stateJson?: string;
    editorId?: number;
    documentVersion?: number;
}

interface NativeFocusEvent {
    isFocused: boolean;
    editorId?: number;
}

interface NativeContentHeightEvent {
    contentHeight: number;
    editorId?: number;
}

interface NativeEditorReadyEvent {
    editorId?: number;
    editorUpdateRevision?: number;
}

interface NativeToolbarActionEvent {
    key: string;
    editorId?: number;
    updateJson?: string;
    stateJson?: string;
    documentVersion?: number;
}

interface NativeAddonEvent {
    eventJson: string;
    editorId?: number;
}

function isCurrentNativeEditorEvent(
    event: { editorId?: number },
    bridge: NativeEditorBridge | null
): boolean {
    if (Platform.OS === 'android') {
        return typeof event.editorId === 'number' && bridge?.editorId === event.editorId;
    }
    if (typeof event.editorId !== 'number') return true;
    return event.editorId === 0 || bridge?.editorId === event.editorId;
}

function computeRenderedTextLength(elements: RenderElement[]): number {
    let len = 0;
    let blockCount = 0;
    for (const el of elements) {
        if (el.type === 'blockStart' && el.listContext) {
            len += el.listContext.kind === 'task'
                ? unicodeScalarCount(el.listContext.checked ? '☑ ' : '☐ ')
                : el.listContext.ordered
                  ? unicodeScalarCount(`${el.listContext.index}. `)
                  : unicodeScalarCount('• ');
        } else if (el.type === 'textRun' && el.text) {
            len += unicodeScalarCount(el.text);
        } else if (
            el.type === 'voidInline' ||
            el.type === 'voidBlock' ||
            el.type === 'opaqueInlineAtom' ||
            el.type === 'opaqueBlockAtom'
        ) {
            if (el.type === 'opaqueInlineAtom' || el.type === 'opaqueBlockAtom') {
                const visibleText =
                    el.nodeType === 'mention' ? (el.label ?? '?') : `[${el.label ?? '?'}]`;
                len += unicodeScalarCount(visibleText);
            } else {
                // U+FFFC placeholder / hard break
                len += 1;
            }
        } else if (el.type === 'blockEnd') {
            blockCount++;
        }
    }
    // Block breaks add 1 scalar each, except the last block
    if (blockCount > 1) len += blockCount - 1;
    return len;
}

function serializeRemoteSelections(
    remoteSelections?: readonly RemoteSelectionDecoration[]
): string | undefined {
    if (!remoteSelections || remoteSelections.length === 0) {
        return undefined;
    }
    return stringifyCachedJson(remoteSelections);
}

function areToolbarFramesEqual(
    left: EditorToolbarFrame | null | undefined,
    right: EditorToolbarFrame | null | undefined
): boolean {
    return (
        left?.x === right?.x &&
        left?.y === right?.y &&
        left?.width === right?.width &&
        left?.height === right?.height
    );
}

function serializeToolbarFrames(
    frames: readonly EditorToolbarFrame[] | null | undefined
): string | undefined {
    if (!frames || frames.length === 0) {
        return undefined;
    }
    return JSON.stringify(frames.length === 1 ? frames[0] : { frames });
}

function parseCaretRectJson(raw: string | null | undefined): NativeRichTextEditorCaretRect | null {
    if (!raw) {
        return null;
    }

    try {
        const parsed = JSON.parse(raw) as Record<string, unknown>;
        const x = typeof parsed.x === 'number' ? parsed.x : null;
        const y = typeof parsed.y === 'number' ? parsed.y : null;
        const width = typeof parsed.width === 'number' ? parsed.width : null;
        const height = typeof parsed.height === 'number' ? parsed.height : null;
        const editorWidth = typeof parsed.editorWidth === 'number' ? parsed.editorWidth : null;
        const editorHeight = typeof parsed.editorHeight === 'number' ? parsed.editorHeight : null;
        if (
            x == null ||
            y == null ||
            width == null ||
            height == null ||
            editorWidth == null ||
            editorHeight == null
        ) {
            return null;
        }
        return { x, y, width, height, editorWidth, editorHeight };
    } catch {
        return null;
    }
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

function useSerializedValue<T>(
    value: T | null | undefined,
    serialize: (value: T) => string | undefined,
    revision?: unknown
): string | undefined {
    const cacheRef = useRef<{
        value: T | null | undefined;
        revision: unknown;
        hasRevision: boolean;
        serialized: string | undefined;
    } | null>(null);
    const hasRevision = revision !== undefined;
    const cached = cacheRef.current;

    if (cached) {
        if (hasRevision && cached.hasRevision && Object.is(cached.revision, revision)) {
            return cached.serialized;
        }
        if (Object.is(cached.value, value) && cached.hasRevision === hasRevision) {
            return cached.serialized;
        }
    }

    const serialized = value == null ? undefined : serialize(value);
    cacheRef.current = {
        value,
        revision,
        hasRevision,
        serialized,
    };
    return serialized;
}

export type NativeRichTextEditorHeightBehavior = 'fixed' | 'autoGrow';
export type NativeRichTextEditorToolbarPlacement = 'keyboard' | 'inline';
export type NativeRichTextEditorValueJSONUpdateMode = 'replace' | 'reset';
export type NativeRichTextEditorAutoCapitalize = 'none' | 'sentences' | 'words' | 'characters';
export type NativeRichTextEditorKeyboardType =
    | 'default'
    | 'email-address'
    | 'numeric'
    | 'phone-pad'
    | 'ascii-capable'
    | 'numbers-and-punctuation'
    | 'url'
    | 'number-pad'
    | 'name-phone-pad'
    | 'decimal-pad'
    | 'twitter'
    | 'web-search'
    | 'visible-password'
    | 'ascii-capable-number-pad';

export interface RemoteSelectionDecoration {
    clientId: number;
    anchor: number;
    head: number;
    color: string;
    name?: string;
    avatarUrl?: string;
    isFocused?: boolean;
}

export interface LinkRequestContext {
    href?: string;
    isActive: boolean;
    selection: Selection;
    setLink: (href: string) => void;
    unsetLink: () => void;
}

export interface ImageRequestContext {
    selection: Selection;
    allowBase64: boolean;
    insertImage: (src: string, attrs?: Omit<ImageNodeAttributes, 'src'>) => void;
}

export interface NativeRichTextEditorProps {
    /** Initial content as HTML (uncontrolled mode). */
    initialContent?: string;
    /** Initial content as ProseMirror JSON (uncontrolled mode). */
    initialJSON?: DocumentJSON;
    /** Controlled HTML content. External changes are diffed and applied. */
    value?: string;
    /** Controlled ProseMirror JSON content. Ignored if value is set. */
    valueJSON?: DocumentJSON;
    /** Optional stable revision hint for `valueJSON` to avoid reserializing equal docs on rerender. */
    valueJSONRevision?: string | number;
    /** Controls how external `valueJSON` changes are applied. Defaults to preserving undo history. */
    valueJSONUpdateMode?: NativeRichTextEditorValueJSONUpdateMode;
    /** When using reset-mode `valueJSON`, preserve the current local text selection after applying the reset. */
    preserveSelectionOnValueJSONReset?: boolean;
    /** Preferred selection to restore when applying reset-mode `valueJSON`; falls back to the live selection. */
    selectionOnValueJSONReset?: Selection;
    /** Schema definition. Defaults to tiptapSchema if not provided. */
    schema?: SchemaDefinition;
    /** Placeholder text shown when editor is empty. */
    placeholder?: string;
    /** Whether the editor is editable. */
    editable?: boolean;
    /** Maximum character length. */
    maxLength?: number;
    /** Whether to auto-focus on mount. */
    autoFocus?: boolean;
    /** Controls native keyboard auto-capitalization. Defaults to sentences. */
    autoCapitalize?: NativeRichTextEditorAutoCapitalize;
    /** Controls native keyboard autocorrection. Defaults to the platform-specific editor default. */
    autoCorrect?: boolean;
    /** Controls the native keyboard layout. Defaults to the platform default keyboard. */
    keyboardType?: NativeRichTextEditorKeyboardType;
    /** Controls whether the editor scrolls internally or grows with content. */
    heightBehavior?: NativeRichTextEditorHeightBehavior;
    /** Whether to show the formatting toolbar. Defaults to true. */
    showToolbar?: boolean;
    /** Whether the toolbar is attached to the keyboard natively or rendered inline in React. */
    toolbarPlacement?: NativeRichTextEditorToolbarPlacement;
    /** Displayed toolbar buttons, in order. Supports custom marks/nodes. */
    toolbarItems?: readonly EditorToolbarItem[];
    /** Called when a custom `action` toolbar item is pressed. */
    onToolbarAction?: (key: string) => void;
    /** Called when a toolbar link item is pressed so the host can collect/edit a URL. */
    onRequestLink?: (context: LinkRequestContext) => void;
    /** Called when a toolbar image item is pressed so the host can choose an image source. */
    onRequestImage?: (context: ImageRequestContext) => void;
    /** Whether plain URLs typed or pasted into the editor should be converted into link marks automatically. */
    autoDetectLinks?: boolean;
    /** Whether `data:image/...` sources are accepted for image insertion and HTML parsing. */
    allowBase64Images?: boolean;
    /** Whether selected images show native resize handles. */
    allowImageResizing?: boolean;
    /** Called when content changes with the current HTML. */
    onContentChange?: (html: string) => void;
    /** Called when content changes with the current ProseMirror JSON. */
    onContentChangeJSON?: (json: DocumentJSON) => void;
    /** Called when selection changes. */
    onSelectionChange?: (selection: Selection) => void;
    /** Called when active formatting state changes. */
    onActiveStateChange?: (state: ActiveState) => void;
    /** Called when undo/redo availability changes. */
    onHistoryStateChange?: (state: HistoryState) => void;
    /** Called when the editor gains focus. */
    onFocus?: () => void;
    /** Called when the editor loses focus. */
    onBlur?: () => void;
    /** Style applied to the native editor view. */
    style?: StyleProp<ViewStyle>;
    /** Style applied to the outer React container wrapping the editor and inline toolbar. */
    containerStyle?: StyleProp<ViewStyle>;
    /** Optional native content theme applied to rendered blocks and typing attrs. */
    theme?: EditorTheme;
    /** Optional addon configuration. */
    addons?: EditorAddons;
    /** Remote awareness selections rendered as native overlays. */
    remoteSelections?: readonly RemoteSelectionDecoration[];
}

export interface NativeRichTextEditorRef {
    /** Programmatically focus the editor. */
    focus(): void;
    /** Programmatically blur the editor. */
    blur(): void;
    /** Toggle a formatting mark (e.g. 'bold', 'italic'). */
    toggleMark(markType: string): void;
    /** Apply or update a hyperlink on the current selection. */
    setLink(href: string): void;
    /** Remove a hyperlink from the current selection. */
    unsetLink(): void;
    /** Toggle blockquote wrapping around the current block selection. */
    toggleBlockquote(): void;
    /** Toggle a heading level on the current block selection. */
    toggleHeading(level: EditorToolbarHeadingLevel): void;
    /** Toggle a list type supported by the active schema. */
    toggleList(listType: string): void;
    /** Indent the current list item. */
    indentListItem(): void;
    /** Outdent the current list item. */
    outdentListItem(): void;
    /** Insert a void node (e.g. 'horizontalRule'). */
    insertNode(nodeType: string): void;
    /** Insert a block image node with the given source and optional metadata. */
    insertImage(src: string, attrs?: Omit<ImageNodeAttributes, 'src'>): void;
    /** Insert text at the current cursor position. */
    insertText(text: string): void;
    /** Insert HTML content at the current selection. */
    insertContentHtml(html: string): void;
    /** Insert JSON content at the current selection. */
    insertContentJson(doc: DocumentJSON): void;
    /** Replace entire document with HTML (preserves undo history). */
    setContent(html: string): void;
    /** Replace entire document with JSON (preserves undo history). */
    setContentJson(doc: DocumentJSON): void;
    /** Clear the document to the active schema's empty text block. */
    clearContent(): void;
    /** Get the current HTML content. */
    getContent(): string;
    /** Get the current content as ProseMirror JSON. */
    getContentJson(): DocumentJSON;
    /** Get the plain text content (no markup). */
    getTextContent(): string;
    /** Get the current caret rectangle in editor-local layout coordinates. */
    getCaretRect(): Promise<NativeRichTextEditorCaretRect | null>;
    /** Undo the last operation. */
    undo(): void;
    /** Redo the last undone operation. */
    redo(): void;
    /** Check if undo is available. */
    canUndo(): boolean;
    /** Check if redo is available. */
    canRedo(): boolean;
}

export interface NativeRichTextEditorCaretRect {
    /** Left edge of the caret, relative to the editor root view. */
    x: number;
    /** Top edge of the caret, relative to the editor root view. */
    y: number;
    /** Caret width. */
    width: number;
    /** Caret height. */
    height: number;
    /** Current editor root view width. */
    editorWidth: number;
    /** Current editor root view height. */
    editorHeight: number;
}

interface RunAndApplyOptions {
    /** If true, suppress onContentChange/onContentChangeJSON callbacks. */
    suppressContentCallbacks?: boolean;
    /** If true, skip the native view apply when the Rust HTML is unchanged. */
    skipNativeApplyIfContentUnchanged?: boolean;
    /** If true, preserve the current live text selection instead of the update selection. */
    preserveLiveTextSelection?: boolean;
    /** Internal: skip the autolink pass for this mutation. */
    skipAutoDetectLinks?: boolean;
    /** Internal: retry callback when native preflight is temporarily blocked. */
    retryBlockedCommand?: () => boolean;
    /** Internal: called when a retry callback is queued. */
    onBlockedCommandRetryQueued?: () => void;
}

interface CommandRetryScope {
    editorId: number;
    documentVersion: number | null;
    scalarAnchor?: number;
    scalarHead?: number;
    mentionRange?: {
        anchor: number;
        head: number;
    };
    mentionQuery?: MentionQueryChangeEvent;
    nativeMentionSelectRequest?: NativeMentionSelectRetryScope;
}

interface NativeMentionSelectRetryScope {
    trigger: string;
    suggestionKey: string;
    range: {
        anchor: number;
        head: number;
    };
}

function doesLiveMentionQueryConflictWithNativeSelectRequest(
    request: NativeMentionSelectRetryScope,
    currentQuery: MentionQueryChangeEvent | null,
    requestDocumentVersion: number | null,
    currentDocumentVersion: number | null
): boolean {
    if (currentQuery == null) return false;

    const currentQueryDocumentVersion =
        typeof currentQuery.documentVersion === 'number' ? currentQuery.documentVersion : null;
    const isSameDocument =
        currentQueryDocumentVersion != null
            ? currentQueryDocumentVersion === requestDocumentVersion
            : requestDocumentVersion == null || requestDocumentVersion === currentDocumentVersion;
    if (!isSameDocument) return false;

    return (
        currentQuery.trigger !== request.trigger ||
        currentQuery.range.anchor !== request.range.anchor ||
        currentQuery.range.head !== request.range.head
    );
}

export const NativeRichTextEditor = forwardRef<NativeRichTextEditorRef, NativeRichTextEditorProps>(
    function NativeRichTextEditor(
        {
            initialContent,
            initialJSON,
            value,
            valueJSON,
            valueJSONRevision,
            valueJSONUpdateMode = 'replace',
            preserveSelectionOnValueJSONReset = false,
            selectionOnValueJSONReset,
            schema,
            placeholder,
            editable = true,
            maxLength,
            autoFocus = false,
            autoCapitalize,
            autoCorrect,
            keyboardType,
            heightBehavior = 'autoGrow',
            showToolbar = true,
            toolbarPlacement = 'keyboard',
            toolbarItems = DEFAULT_EDITOR_TOOLBAR_ITEMS,
            onToolbarAction,
            onRequestLink,
            onRequestImage,
            autoDetectLinks = false,
            onContentChange,
            onContentChangeJSON,
            onSelectionChange,
            onActiveStateChange,
            onHistoryStateChange,
            onFocus,
            onBlur,
            style,
            containerStyle,
            theme,
            addons,
            remoteSelections,
            allowBase64Images = false,
            allowImageResizing = true,
        },
        ref
    ) {
        const bridgeRef = useRef<NativeEditorBridge | null>(null);
        const nativeViewRef = useRef<NativeEditorViewHandle | null>(null);
        const [isReady, setIsReady] = useState(false);
        const [editorInstanceId, setEditorInstanceId] = useState(0);
        const editorInstanceIdRef = useRef(0);
        editorInstanceIdRef.current = editorInstanceId;
        const [isFocused, setIsFocused] = useState(false);
        const isFocusedRef = useRef(false);
        const [inlineToolbarFrame, setInlineToolbarFrame] = useState<EditorToolbarFrame | null>(
            null
        );
        const registeredToolbarFrames = useEditorToolbarFrames(editorInstanceId);
        const [pendingNativeUpdate, setPendingNativeUpdate] = useState<{
            json?: string;
            editorId?: number;
            revision: number;
        }>({
            json: undefined,
            editorId: undefined,
            revision: 0,
        });
        const [pendingNativeResetUpdate, setPendingNativeResetUpdate] = useState<{
            json?: string;
            editorId?: number;
            revision: number;
        }>({
            json: undefined,
            editorId: undefined,
            revision: 0,
        });
        const pendingNativeUpdateInFlightRef = useRef<{
            editorId?: number;
            revision: number;
        } | null>(null);
        const pendingNativeResetUpdateInFlightRef = useRef<{
            editorId?: number;
            revision: number;
        } | null>(null);
        const nativeUpdateRevisionRef = useRef(0);
        const nextNativeUpdateRevision = useCallback((): number => {
            const revision = nativeUpdateRevisionRef.current + 1;
            nativeUpdateRevisionRef.current = revision;
            return revision;
        }, []);
        const [blockedNativeCommandRetry, setBlockedNativeCommandRetry] = useState(0);
        const [detachedControlledSyncRetry, setDetachedControlledSyncRetry] = useState(0);
        const [controlledNativeUpdateRetry, setControlledNativeUpdateRetry] = useState(0);
        const pendingDetachedControlledSyncRef = useRef(false);
        const pendingControlledSyncAfterNativeUpdateRef = useRef(false);
        const pendingBlockedNativeCommandRetryRef = useRef(false);
        const pendingNativeCommandRetryRef = useRef<(() => boolean) | null>(null);
        const pendingBridgeRecreationContentRef = useRef<{
            jsonString: string;
            selection: Selection;
        } | null>(null);
        const blockedNativeCommandRetryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
            null
        );
        const [autoGrowHeight, setAutoGrowHeight] = useState<number | null>(null);

        // Toolbar state from EditorUpdate events
        const [activeState, setActiveState] = useState<ActiveState>({
            marks: {},
            markAttrs: {},
            nodes: {},
            commands: {},
            allowedMarks: [],
            insertableNodes: [],
        });
        const activeStateRef = useRef(activeState);
        activeStateRef.current = activeState;
        const [historyState, setHistoryState] = useState<HistoryState>({
            canUndo: false,
            canRedo: false,
        });
        const [mentionQueryEvent, setMentionQueryEvent] = useState<MentionQueryChangeEvent | null>(
            null
        );

        // Selection and rendered text length refs (non-rendering state)
        const selectionRef = useRef<Selection>({ type: 'text', anchor: 0, head: 0 });
        const renderedTextLengthRef = useRef(0);
        const documentVersionRef = useRef<number | null>(null);
        const controlledHtmlSyncRef = useRef<{
            value?: string | null;
            documentVersion: number | null;
        }>({
            value: undefined,
            documentVersion: null,
        });
        const controlledJsonSyncRef = useRef<{
            value?: string | null;
            documentVersion: number | null;
        }>({
            value: undefined,
            documentVersion: null,
        });
        const toolbarRef = useRef<View | null>(null);
        const mentionQueryEventRef = useRef<MentionQueryChangeEvent | null>(null);
        const mentionQueryEditorIdRef = useRef<number | null>(null);
        mentionQueryEventRef.current = mentionQueryEvent;
        const setMentionQueryEventState = useCallback(
            (nextEvent: MentionQueryChangeEvent | null, editorId?: number | null) => {
                mentionQueryEventRef.current = nextEvent;
                mentionQueryEditorIdRef.current =
                    nextEvent == null
                        ? null
                        : typeof editorId === 'number'
                          ? editorId
                          : (bridgeRef.current?.editorId ?? null);
                setMentionQueryEvent(nextEvent);
            },
            []
        );
        const toolbarItemsSerializationCacheRef = useRef<{
            toolbarItems: readonly EditorToolbarItem[];
            editable: boolean;
            isLinkActive: boolean;
            allowsLink: boolean;
            canRequestLink: boolean;
            canRequestImage: boolean;
            canInsertImage: boolean;
            mappedItems: EditorToolbarItem[];
            serialized: string;
        } | null>(null);

        // Stable callback refs to avoid re-renders
        const onContentChangeRef = useRef(onContentChange);
        onContentChangeRef.current = onContentChange;
        const onContentChangeJSONRef = useRef(onContentChangeJSON);
        onContentChangeJSONRef.current = onContentChangeJSON;
        const onSelectionChangeRef = useRef(onSelectionChange);
        onSelectionChangeRef.current = onSelectionChange;
        const onActiveStateChangeRef = useRef(onActiveStateChange);
        onActiveStateChangeRef.current = onActiveStateChange;
        const onHistoryStateChangeRef = useRef(onHistoryStateChange);
        onHistoryStateChangeRef.current = onHistoryStateChange;
        const onFocusRef = useRef(onFocus);
        onFocusRef.current = onFocus;
        const onBlurRef = useRef(onBlur);
        onBlurRef.current = onBlur;
        const addonsRef = useRef(addons);
        addonsRef.current = addons;
        const currentLinkHref =
            typeof activeState.markAttrs?.link?.href === 'string'
                ? (activeState.markAttrs.link.href as string)
                : undefined;

        const mentionSuggestionsByKeyRef = useRef<Map<string, MentionSuggestion>>(new Map());
        mentionSuggestionsByKeyRef.current = new Map(
            (addons?.mentions?.suggestions ?? []).map((suggestion) => [suggestion.key, suggestion])
        );

        const bridgeSchema =
            addons?.mentions != null ? withMentionsSchema(schema ?? tiptapSchema) : schema;
        const documentSchema = bridgeSchema ?? tiptapSchema;
        const serializedSchemaJson = useSerializedValue(bridgeSchema, (nextSchema) =>
            stringifyCachedJson(nextSchema)
        );
        const serializedInitialJson = useSerializedValue(initialJSON, (doc) =>
            stringifyCachedJson(normalizeDocumentJson(doc, documentSchema))
        );
        const serializedValueJson = useSerializedValue(
            valueJSON,
            (doc) => stringifyCachedJson(normalizeDocumentJson(doc, documentSchema)),
            valueJSONRevision
        );
        const themeJson = useSerializedValue(theme, serializeEditorTheme);
        const addonsJson = useSerializedValue(addons, serializeEditorAddons);
        const remoteSelectionsJson = useSerializedValue(remoteSelections, (selections) =>
            serializeRemoteSelections(selections)
        );

        const clearBlockedNativeCommandRetryTimer = useCallback(() => {
            const timer = blockedNativeCommandRetryTimerRef.current;
            if (timer == null) return;
            clearTimeout(timer);
            blockedNativeCommandRetryTimerRef.current = null;
        }, []);

        const flushBlockedNativeCommandRetry = useCallback(() => {
            const hadPendingRetry =
                pendingBlockedNativeCommandRetryRef.current ||
                blockedNativeCommandRetryTimerRef.current != null;
            if (!hadPendingRetry) return;
            pendingBlockedNativeCommandRetryRef.current = false;
            clearBlockedNativeCommandRetryTimer();
            setBlockedNativeCommandRetry((revision) => revision + 1);
        }, [clearBlockedNativeCommandRetryTimer]);

        const scheduleBlockedNativeCommandRetry = useCallback(() => {
            pendingBlockedNativeCommandRetryRef.current = true;
            if (blockedNativeCommandRetryTimerRef.current != null) return;
            blockedNativeCommandRetryTimerRef.current = setTimeout(() => {
                blockedNativeCommandRetryTimerRef.current = null;
                if (!pendingBlockedNativeCommandRetryRef.current) return;
                pendingBlockedNativeCommandRetryRef.current = false;
                setBlockedNativeCommandRetry((revision) => revision + 1);
            }, 50);
        }, []);

        const enqueueBlockedNativeCommandRetry = useCallback(
            (retry: () => boolean) => {
                pendingNativeCommandRetryRef.current = retry;
                scheduleBlockedNativeCommandRetry();
            },
            [scheduleBlockedNativeCommandRetry]
        );

        const clearStaleMentionQueryForDocumentVersion = useCallback(
            (documentVersion: number) => {
                const currentMentionQuery = mentionQueryEventRef.current;
                if (
                    currentMentionQuery != null &&
                    (typeof currentMentionQuery.documentVersion !== 'number' ||
                        currentMentionQuery.documentVersion < documentVersion)
                ) {
                    setMentionQueryEventState(null);
                }
            },
            [setMentionQueryEventState]
        );

        const syncStateFromUpdate = useCallback((update: EditorUpdate | null) => {
            if (!update) return;
            activeStateRef.current = update.activeState;
            setActiveState(update.activeState);
            setHistoryState(update.historyState);
            selectionRef.current = update.selection;
            renderedTextLengthRef.current = computeRenderedTextLength(update.renderElements);
            if (typeof update.documentVersion === 'number') {
                const previousDocumentVersion = documentVersionRef.current;
                documentVersionRef.current = update.documentVersion;
                if (
                    previousDocumentVersion == null ||
                    update.documentVersion > previousDocumentVersion
                ) {
                    clearStaleMentionQueryForDocumentVersion(update.documentVersion);
                }
            }
            flushBlockedNativeCommandRetry();
        }, [clearStaleMentionQueryForDocumentVersion, flushBlockedNativeCommandRetry]);

        const syncSelectionStateFromUpdate = useCallback((update: EditorUpdate | null) => {
            if (!update) return;
            activeStateRef.current = update.activeState;
            setActiveState(update.activeState);
            setHistoryState(update.historyState);
            selectionRef.current = update.selection;
            if (typeof update.documentVersion === 'number') {
                const previousDocumentVersion = documentVersionRef.current;
                documentVersionRef.current = update.documentVersion;
                if (
                    previousDocumentVersion == null ||
                    update.documentVersion > previousDocumentVersion
                ) {
                    clearStaleMentionQueryForDocumentVersion(update.documentVersion);
                }
            }
            flushBlockedNativeCommandRetry();
        }, [clearStaleMentionQueryForDocumentVersion, flushBlockedNativeCommandRetry]);

        const emitContentCallbacksForUpdate = useCallback(
            (update: EditorUpdate | null, previousDocumentVersion: number | null) => {
                if (!update || !bridgeRef.current || bridgeRef.current.isDestroyed) return;
                const wantsHtml = typeof onContentChangeRef.current === 'function';
                const wantsJson = typeof onContentChangeJSONRef.current === 'function';
                if (!wantsHtml && !wantsJson) return;

                if (
                    previousDocumentVersion != null &&
                    typeof update.documentVersion === 'number' &&
                    update.documentVersion === previousDocumentVersion
                ) {
                    return;
                }

                if (wantsHtml && wantsJson) {
                    const snapshot = bridgeRef.current.getContentSnapshot();
                    onContentChangeRef.current?.(snapshot.html);
                    onContentChangeJSONRef.current?.(snapshot.json);
                    return;
                }

                if (wantsHtml) {
                    onContentChangeRef.current?.(bridgeRef.current.getHtml());
                }
                if (wantsJson) {
                    onContentChangeJSONRef.current?.(bridgeRef.current.getJson());
                }
            },
            []
        );

        const clearPendingNativeUpdateForCurrentEditor = useCallback(() => {
            if (Platform.OS !== 'android') return;
            const editorId = bridgeRef.current?.editorId;
            if (typeof editorId !== 'number') return;
            const inFlight = pendingNativeUpdateInFlightRef.current;
            if (inFlight != null && inFlight.editorId === editorId) {
                pendingNativeUpdateInFlightRef.current = null;
            }
            const next = {
                json: undefined,
                editorId,
                revision: nextNativeUpdateRevision(),
            };
            setPendingNativeUpdate(next);
        }, [nextNativeUpdateRevision]);

        const queuePendingNativeResetUpdate = useCallback((updateJson: string) => {
            if (Platform.OS !== 'android') return;
            const editorId = bridgeRef.current?.editorId;
            const revision = nextNativeUpdateRevision();
            const next = {
                json: updateJson,
                editorId,
                revision,
            };
            pendingNativeResetUpdateInFlightRef.current = { editorId, revision };
            setPendingNativeResetUpdate(next);
        }, [nextNativeUpdateRevision]);

        const consumeBlockedCommandInfoForRetry = useCallback(
            (bridge: NativeEditorBridge): CommandBlockedInfo => {
                const blockedInfo = bridge.consumeLastCommandBlockedInfo();
                if (!blockedInfo.blocked) return blockedInfo;
                if (blockedInfo.reason === 'detached') {
                    pendingDetachedControlledSyncRef.current = true;
                } else if (blockedInfo.reason === 'destroyed') {
                    pendingDetachedControlledSyncRef.current = false;
                    pendingBlockedNativeCommandRetryRef.current = false;
                    pendingNativeCommandRetryRef.current = null;
                    clearBlockedNativeCommandRetryTimer();
                    clearPendingNativeUpdateForCurrentEditor();
                } else {
                    scheduleBlockedNativeCommandRetry();
                }
                return blockedInfo;
            },
            [
                clearBlockedNativeCommandRetryTimer,
                clearPendingNativeUpdateForCurrentEditor,
                scheduleBlockedNativeCommandRetry,
            ]
        );

        const consumeBlockedCommandForRetry = useCallback(
            (bridge: NativeEditorBridge): boolean =>
                consumeBlockedCommandInfoForRetry(bridge).blocked,
            [consumeBlockedCommandInfoForRetry]
        );

        useEffect(() => {
            const retry = pendingNativeCommandRetryRef.current;
            if (retry == null) return;
            pendingNativeCommandRetryRef.current = null;
            retry();
        }, [blockedNativeCommandRetry]);

        const hasPendingNativeUpdateInFlightForCurrentEditor = useCallback((): boolean => {
            if (Platform.OS !== 'android') return false;
            const editorId = bridgeRef.current?.editorId;
            if (typeof editorId !== 'number') return false;
            const inFlight = pendingNativeUpdateInFlightRef.current;
            return inFlight != null && inFlight.editorId === editorId;
        }, []);

        const applyUpdateToNativeView = useCallback(
            (
                update: EditorUpdate,
                previousDocumentVersion: number | null,
                skipNativeApplyIfContentUnchanged = false
            ): boolean => {
                const contentChanged = didContentChange(previousDocumentVersion, update);
                const handleApplyResult = (result: unknown): boolean => {
                    if (result === false) {
                        setBlockedNativeCommandRetry((revision) => revision + 1);
                        return false;
                    }
                    return true;
                };

                if (!skipNativeApplyIfContentUnchanged || contentChanged) {
                    const updateJson = JSON.stringify(update);
                    if (Platform.OS === 'android') {
                        const editorId = bridgeRef.current?.editorId;
                        const revision = nextNativeUpdateRevision();
                        const next = {
                            json: updateJson,
                            editorId,
                            revision,
                        };
                        pendingNativeUpdateInFlightRef.current = { editorId, revision };
                        setPendingNativeUpdate(next);
                    } else {
                        try {
                            const applyResult =
                                nativeViewRef.current?.applyEditorUpdate(updateJson);
                            if (isPromiseLike(applyResult)) {
                                void applyResult
                                    .then((result) => {
                                        handleApplyResult(result);
                                    })
                                    .catch(() => {
                                        // The native view may already be torn down during navigation.
                                    });
                            } else {
                                return handleApplyResult(applyResult);
                            }
                        } catch {
                            // The native view may already be torn down during navigation.
                        }
                    }
                }

                return true;
            },
            [nextNativeUpdateRevision]
        );

        const applyResetUpdateToNativeView = useCallback(
            (update: EditorUpdate, previousDocumentVersion: number | null): void => {
                const updateJson = JSON.stringify(update);
                if (Platform.OS === 'android') {
                    clearPendingNativeUpdateForCurrentEditor();
                    queuePendingNativeResetUpdate(updateJson);
                    const applyResetUpdate = nativeViewRef.current?.applyEditorResetUpdate;
                    if (applyResetUpdate) {
                        try {
                            const applyResult = applyResetUpdate(updateJson);
                            if (isPromiseLike(applyResult)) {
                                void applyResult.catch(() => {
                                    // The native view may already be torn down during navigation.
                                });
                            }
                            return;
                        } catch {
                            // Fall through to the regular prop-based apply path.
                        }
                    }
                    return;
                }
                applyUpdateToNativeView(update, previousDocumentVersion);
            },
            [
                applyUpdateToNativeView,
                clearPendingNativeUpdateForCurrentEditor,
                queuePendingNativeResetUpdate,
            ]
        );

        const maybeApplyAutoDetectedLink = useCallback(
            (update: EditorUpdate | null, previousDocumentVersion: number | null) => {
                const applyAutoLink = (
                    candidateUpdate: EditorUpdate | null,
                    allowPreflightRetry: boolean
                ): EditorUpdate | null => {
                    if (
                        !autoDetectLinks ||
                        !candidateUpdate ||
                        !didContentChange(previousDocumentVersion, candidateUpdate) ||
                        !bridgeRef.current ||
                        bridgeRef.current.isDestroyed ||
                        !candidateUpdate.activeState.allowedMarks.includes('link') ||
                        candidateUpdate.selection.type !== 'text' ||
                        candidateUpdate.selection.anchor == null ||
                        candidateUpdate.selection.head == null ||
                        candidateUpdate.selection.anchor !== candidateUpdate.selection.head
                    ) {
                        return candidateUpdate;
                    }

                    const cursorDocPos = candidateUpdate.selection.head;
                    const candidate = findAutoLinkCandidateInDocument(
                        bridgeRef.current.getJson(),
                        cursorDocPos
                    );
                    if (!candidate) {
                        return candidateUpdate;
                    }

                    const scalarFrom = bridgeRef.current.docToScalar(candidate.docFrom);
                    const scalarTo = bridgeRef.current.docToScalar(candidate.docTo);
                    if (!(scalarTo > scalarFrom)) {
                        return candidateUpdate;
                    }

                    const autoLinkUpdate = bridgeRef.current.setMarkAtSelectionScalar(
                        scalarFrom,
                        scalarTo,
                        'link',
                        { href: candidate.href }
                    );
                    if (!autoLinkUpdate) {
                        const preflightUpdate =
                            bridgeRef.current.consumeLastCommandPreflightUpdate();
                        if (preflightUpdate) {
                            return allowPreflightRetry
                                ? applyAutoLink(preflightUpdate, false)
                                : preflightUpdate;
                        }
                        consumeBlockedCommandForRetry(bridgeRef.current);
                        return candidateUpdate;
                    }

                    bridgeRef.current.setSelection(
                        candidateUpdate.selection.anchor,
                        candidateUpdate.selection.head
                    );
                    const selectionState = bridgeRef.current.getSelectionState();
                    if (selectionState) {
                        autoLinkUpdate.selection = selectionState.selection;
                        autoLinkUpdate.activeState = selectionState.activeState;
                        autoLinkUpdate.historyState = selectionState.historyState;
                        if (typeof selectionState.documentVersion === 'number') {
                            autoLinkUpdate.documentVersion = selectionState.documentVersion;
                        }
                    } else {
                        autoLinkUpdate.selection = candidateUpdate.selection;
                    }

                    return autoLinkUpdate;
                };

                return applyAutoLink(update, true);
            },
            [autoDetectLinks, consumeBlockedCommandForRetry]
        );

        const syncNativeUpdateFromBridge = useCallback(
            (
                nativeUpdate: EditorUpdate,
                previousDocumentVersion: number | null,
                options?: Pick<
                    RunAndApplyOptions,
                    'skipNativeApplyIfContentUnchanged' | 'suppressContentCallbacks'
                >
            ): EditorUpdate | null => {
                const update = maybeApplyAutoDetectedLink(nativeUpdate, previousDocumentVersion);
                if (!update) return null;
                if (update !== nativeUpdate) {
                    applyUpdateToNativeView(
                        update,
                        previousDocumentVersion,
                        options?.skipNativeApplyIfContentUnchanged
                    );
                }

                syncStateFromUpdate(update);

                onActiveStateChangeRef.current?.(update.activeState);
                onHistoryStateChangeRef.current?.(update.historyState);

                onSelectionChangeRef.current?.(update.selection);
                if (!options?.suppressContentCallbacks) {
                    emitContentCallbacksForUpdate(update, previousDocumentVersion);
                }
                return update;
            },
            [
                applyUpdateToNativeView,
                emitContentCallbacksForUpdate,
                maybeApplyAutoDetectedLink,
                syncStateFromUpdate,
            ]
        );

        // Warn if both value and valueJSON are set
        if (__DEV__ && value != null && valueJSON != null) {
            console.warn(
                'NativeRichTextEditor: value and valueJSON are mutually exclusive. ' +
                    'Only value will be used.'
            );
        }

        const runAndApply = useCallback(
            (
                mutate: () => EditorUpdate | null,
                options?: RunAndApplyOptions
            ): EditorUpdate | null => {
                if (hasPendingNativeUpdateInFlightForCurrentEditor()) {
                    if (options?.retryBlockedCommand != null) {
                        enqueueBlockedNativeCommandRetry(options.retryBlockedCommand);
                        options.onBlockedCommandRetryQueued?.();
                    }
                    return null;
                }
                const previousDocumentVersion = documentVersionRef.current;
                const preservedSelection =
                    options?.preserveLiveTextSelection === true ? selectionRef.current : null;
                let update = mutate();
                if (!update) {
                    const bridge = bridgeRef.current;
                    if (bridge != null && !bridge.isDestroyed) {
                        const preflightUpdate = bridge.consumeLastCommandPreflightUpdate();
                        if (preflightUpdate) {
                            syncNativeUpdateFromBridge(preflightUpdate, previousDocumentVersion);
                        }
                        const blockedInfo = consumeBlockedCommandInfoForRetry(bridge);
                        if (
                            options?.retryBlockedCommand != null &&
                            isRetryableNativeCommandBlock(blockedInfo.reason)
                        ) {
                            enqueueBlockedNativeCommandRetry(options.retryBlockedCommand);
                            options.onBlockedCommandRetryQueued?.();
                        }
                    }
                    return null;
                }

                if (!options?.skipAutoDetectLinks) {
                    update = maybeApplyAutoDetectedLink(update, previousDocumentVersion);
                    if (!update) {
                        return null;
                    }
                }

                if (
                    preservedSelection?.type === 'text' &&
                    typeof preservedSelection.anchor === 'number' &&
                    typeof preservedSelection.head === 'number' &&
                    !didContentChange(previousDocumentVersion, update) &&
                    bridgeRef.current != null &&
                    !bridgeRef.current.isDestroyed
                ) {
                    bridgeRef.current.setSelection(
                        preservedSelection.anchor,
                        preservedSelection.head
                    );
                    update.selection = {
                        type: 'text',
                        anchor: preservedSelection.anchor,
                        head: preservedSelection.head,
                    };
                }

                applyUpdateToNativeView(
                    update,
                    previousDocumentVersion,
                    options?.skipNativeApplyIfContentUnchanged
                );

                syncStateFromUpdate(update);

                onActiveStateChangeRef.current?.(update.activeState);
                onHistoryStateChangeRef.current?.(update.historyState);

                onSelectionChangeRef.current?.(update.selection);
                if (!options?.suppressContentCallbacks) {
                    emitContentCallbacksForUpdate(update, previousDocumentVersion);
                }

                return update;
            },
            [
                applyUpdateToNativeView,
                consumeBlockedCommandInfoForRetry,
                enqueueBlockedNativeCommandRetry,
                emitContentCallbacksForUpdate,
                hasPendingNativeUpdateInFlightForCurrentEditor,
                maybeApplyAutoDetectedLink,
                syncNativeUpdateFromBridge,
                syncStateFromUpdate,
            ]
        );

        const prepareBridgeForExternalContentRead = useCallback(
            (options?: { skipWhenContentChanged?: boolean }): boolean => {
                const bridge = bridgeRef.current;
                if (!bridge || bridge.isDestroyed) return false;

                const previousDocumentVersion = documentVersionRef.current;
                const ready = bridge.prepareForNativeCommand();
                const preflightUpdate = bridge.consumeLastCommandPreflightUpdate();
                if (preflightUpdate) {
                    syncNativeUpdateFromBridge(preflightUpdate, previousDocumentVersion);
                }
                if (!ready) {
                    consumeBlockedCommandForRetry(bridge);
                    return false;
                }

                return (
                    options?.skipWhenContentChanged !== true ||
                    !didContentChange(previousDocumentVersion, preflightUpdate)
                );
            },
            [consumeBlockedCommandForRetry, syncNativeUpdateFromBridge]
        );

        const resetContentJsonString = useCallback(
            (
                jsonString: string,
                options?: Pick<
                    RunAndApplyOptions,
                    'suppressContentCallbacks' | 'preserveLiveTextSelection'
                > & {
                    selection?: Selection;
                }
            ): EditorUpdate | null => {
                const bridge = bridgeRef.current;
                if (!bridge || bridge.isDestroyed) return null;
                const previousDocumentVersion = documentVersionRef.current;
                const preservedSelection =
                    options?.preserveLiveTextSelection === true
                        ? (options.selection ?? selectionRef.current)
                        : null;
                bridge.setJsonString(jsonString);
                const update = bridge.getCurrentState();
                if (!update) return null;
                if (
                    preservedSelection != null &&
                    restoreSelectionInBridge(bridge, preservedSelection)
                ) {
                    const selectionState = bridge.getSelectionState();
                    if (selectionState) {
                        update.selection = selectionState.selection;
                        update.activeState = selectionState.activeState;
                        update.historyState = selectionState.historyState;
                        if (typeof selectionState.documentVersion === 'number') {
                            update.documentVersion = selectionState.documentVersion;
                        }
                    }
                }

                applyResetUpdateToNativeView(update, previousDocumentVersion);
                syncStateFromUpdate(update);
                onActiveStateChangeRef.current?.(update.activeState);
                onHistoryStateChangeRef.current?.(update.historyState);
                onSelectionChangeRef.current?.(update.selection);
                if (!options?.suppressContentCallbacks) {
                    emitContentCallbacksForUpdate(update, previousDocumentVersion);
                }

                return update;
            },
            [applyResetUpdateToNativeView, emitContentCallbacksForUpdate, syncStateFromUpdate]
        );

        const syncPreflightUpdateFromNativeEvent = useCallback(
            (updateJson?: string): boolean => {
                if (typeof updateJson !== 'string' || updateJson.length === 0) {
                    return true;
                }
                const bridge = bridgeRef.current;
                if (!bridge || bridge.isDestroyed) return false;
                const previousDocumentVersion = documentVersionRef.current;
                try {
                    const parsed = JSON.parse(updateJson) as { documentVersion?: unknown };
                    if (
                        Platform.OS === 'android' &&
                        typeof previousDocumentVersion === 'number' &&
                        typeof parsed.documentVersion !== 'number'
                    ) {
                        return false;
                    }
                    const update = bridge.parseUpdateJson(updateJson);
                    if (!update) return false;
                    syncNativeUpdateFromBridge(update, previousDocumentVersion);
                    return true;
                } catch {
                    return false;
                }
            },
            [syncNativeUpdateFromBridge]
        );

        useEffect(() => {
            const preservedUncontrolledContent =
                value == null && serializedValueJson == null
                    ? pendingBridgeRecreationContentRef.current
                    : null;
            pendingBridgeRecreationContentRef.current = null;
            const bridgeConfig =
                maxLength != null || serializedSchemaJson || allowBase64Images
                    ? {
                          maxLength,
                          schemaJson: serializedSchemaJson,
                          allowBase64Images,
                      }
                    : undefined;
            const bridge = NativeEditorBridge.create(bridgeConfig);
            bridgeRef.current = bridge;
            setEditorInstanceId(bridge.editorId);

            // Four-way content initialization: value > valueJSON > initialJSON > initialContent
            if (value != null) {
                bridge.setHtml(value);
            } else if (serializedValueJson != null) {
                bridge.setJsonString(serializedValueJson);
            } else if (preservedUncontrolledContent != null) {
                bridge.setJsonString(preservedUncontrolledContent.jsonString);
            } else if (serializedInitialJson != null) {
                bridge.setJsonString(serializedInitialJson);
            } else if (initialContent) {
                bridge.setHtml(initialContent);
            }

            if (preservedUncontrolledContent != null) {
                const preservedSelection = preservedUncontrolledContent.selection;
                if (preservedSelection.type === 'text') {
                    const anchor = preservedSelection.anchor ?? 0;
                    const head = preservedSelection.head ?? anchor;
                    bridge.setSelection(anchor, head);
                } else if (
                    preservedSelection.type === 'node' &&
                    typeof preservedSelection.pos === 'number'
                ) {
                    bridge.setSelection(preservedSelection.pos, preservedSelection.pos);
                }
            }

            syncStateFromUpdate(bridge.getCurrentState());
            setIsReady(true);

            return () => {
                if (
                    bridgeRef.current === bridge &&
                    value == null &&
                    serializedValueJson == null &&
                    !bridge.isDestroyed
                ) {
                    try {
                        pendingBridgeRecreationContentRef.current = {
                            jsonString: bridge.getJsonString(),
                            selection: selectionRef.current,
                        };
                    } catch {
                        pendingBridgeRecreationContentRef.current = null;
                    }
                }
                bridge.destroy();
                if (bridgeRef.current === bridge) {
                    bridgeRef.current = null;
                }
                pendingNativeUpdateInFlightRef.current = null;
                pendingNativeResetUpdateInFlightRef.current = null;
                pendingDetachedControlledSyncRef.current = false;
                pendingControlledSyncAfterNativeUpdateRef.current = false;
                pendingBlockedNativeCommandRetryRef.current = false;
                pendingNativeCommandRetryRef.current = null;
                documentVersionRef.current = null;
                mentionQueryEventRef.current = null;
                mentionQueryEditorIdRef.current = null;
                clearBlockedNativeCommandRetryTimer();
                setMentionQueryEvent(null);
                const clearedNativeUpdate = {
                    json: undefined,
                    editorId: undefined,
                    revision: nextNativeUpdateRevision(),
                };
                setPendingNativeUpdate(clearedNativeUpdate);
                const clearedNativeResetUpdate = {
                    json: undefined,
                    editorId: undefined,
                    revision: nextNativeUpdateRevision(),
                };
                setPendingNativeResetUpdate(clearedNativeResetUpdate);
                setEditorInstanceId(0);
                setIsReady(false);
            };
            // eslint-disable-next-line react-hooks/exhaustive-deps
        }, [
            maxLength,
            syncStateFromUpdate,
            allowBase64Images,
            serializedSchemaJson,
            clearBlockedNativeCommandRetryTimer,
            nextNativeUpdateRevision,
        ]);

        useEffect(() => {
            if (value == null) return;
            if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;
            const previousSync = controlledHtmlSyncRef.current;
            const didControlledValueChange = previousSync.value !== value;
            const didDocumentAdvanceForSameValue =
                !didControlledValueChange &&
                previousSync.documentVersion !== documentVersionRef.current;
            if (
                !prepareBridgeForExternalContentRead({
                    skipWhenContentChanged: !didControlledValueChange,
                })
            ) {
                controlledHtmlSyncRef.current = {
                    value,
                    documentVersion: documentVersionRef.current,
                };
                return;
            }
            if (didDocumentAdvanceForSameValue) {
                controlledHtmlSyncRef.current = {
                    value,
                    documentVersion: documentVersionRef.current,
                };
                return;
            }

            const currentHtml = bridgeRef.current.getHtml();
            if (currentHtml === value) {
                clearPendingNativeUpdateForCurrentEditor();
                controlledHtmlSyncRef.current = {
                    value,
                    documentVersion: documentVersionRef.current,
                };
                return;
            }

            if (hasPendingNativeUpdateInFlightForCurrentEditor()) {
                pendingControlledSyncAfterNativeUpdateRef.current = true;
                return;
            }

            const update = runAndApply(() => bridgeRef.current!.replaceHtml(value), {
                suppressContentCallbacks: true,
                preserveLiveTextSelection: true,
                skipAutoDetectLinks: true,
            });
            if (!update && hasPendingNativeUpdateInFlightForCurrentEditor()) {
                pendingControlledSyncAfterNativeUpdateRef.current = true;
                return;
            }
            controlledHtmlSyncRef.current = {
                value,
                documentVersion: documentVersionRef.current,
            };
        }, [
            value,
            runAndApply,
            blockedNativeCommandRetry,
            controlledNativeUpdateRetry,
            detachedControlledSyncRetry,
            prepareBridgeForExternalContentRead,
            clearPendingNativeUpdateForCurrentEditor,
            hasPendingNativeUpdateInFlightForCurrentEditor,
        ]);

        useEffect(() => {
            if (serializedValueJson == null || value != null) return;
            if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;
            const previousSync = controlledJsonSyncRef.current;
            const didControlledValueChange = previousSync.value !== serializedValueJson;
            const didDocumentAdvanceForSameValue =
                !didControlledValueChange &&
                previousSync.documentVersion !== documentVersionRef.current;
            if (valueJSONUpdateMode === 'reset') {
                const currentJson = bridgeRef.current.getJsonString();
                if (currentJson === serializedValueJson) {
                    clearPendingNativeUpdateForCurrentEditor();
                    controlledJsonSyncRef.current = {
                        value: serializedValueJson,
                        documentVersion: documentVersionRef.current,
                    };
                    return;
                }

                resetContentJsonString(serializedValueJson, {
                    suppressContentCallbacks: true,
                    preserveLiveTextSelection: preserveSelectionOnValueJSONReset,
                    selection: selectionOnValueJSONReset,
                });
                controlledJsonSyncRef.current = {
                    value: serializedValueJson,
                    documentVersion: documentVersionRef.current,
                };
                return;
            }
            if (
                !prepareBridgeForExternalContentRead({
                    skipWhenContentChanged: !didControlledValueChange,
                })
            ) {
                controlledJsonSyncRef.current = {
                    value: serializedValueJson,
                    documentVersion: documentVersionRef.current,
                };
                return;
            }
            if (didDocumentAdvanceForSameValue) {
                controlledJsonSyncRef.current = {
                    value: serializedValueJson,
                    documentVersion: documentVersionRef.current,
                };
                return;
            }

            const currentJson = bridgeRef.current.getJsonString();
            if (currentJson === serializedValueJson) {
                clearPendingNativeUpdateForCurrentEditor();
                controlledJsonSyncRef.current = {
                    value: serializedValueJson,
                    documentVersion: documentVersionRef.current,
                };
                return;
            }

            if (hasPendingNativeUpdateInFlightForCurrentEditor()) {
                pendingControlledSyncAfterNativeUpdateRef.current = true;
                return;
            }

            const update = runAndApply(
                () => bridgeRef.current!.replaceJsonString(serializedValueJson),
                {
                    suppressContentCallbacks: true,
                    preserveLiveTextSelection: true,
                    skipAutoDetectLinks: true,
                }
            );
            if (!update && hasPendingNativeUpdateInFlightForCurrentEditor()) {
                pendingControlledSyncAfterNativeUpdateRef.current = true;
                return;
            }
            controlledJsonSyncRef.current = {
                value: serializedValueJson,
                documentVersion: documentVersionRef.current,
            };
        }, [
            serializedValueJson,
            value,
            valueJSONUpdateMode,
            preserveSelectionOnValueJSONReset,
            selectionOnValueJSONReset,
            runAndApply,
            blockedNativeCommandRetry,
            controlledNativeUpdateRetry,
            detachedControlledSyncRetry,
            prepareBridgeForExternalContentRead,
            clearPendingNativeUpdateForCurrentEditor,
            hasPendingNativeUpdateInFlightForCurrentEditor,
            resetContentJsonString,
        ]);

        const updateToolbarFrame = useCallback(() => {
            const toolbar = toolbarRef.current;
            if (!toolbar) {
                setInlineToolbarFrame(null);
                return;
            }

            toolbar.measureInWindow((x, y, width, height) => {
                if (width <= 0 || height <= 0) {
                    setInlineToolbarFrame(null);
                    return;
                }

                const nextFrame = { x, y, width, height };
                setInlineToolbarFrame((prev) =>
                    areToolbarFramesEqual(prev, nextFrame) ? prev : nextFrame
                );
            });
        }, []);

        useEffect(() => {
            if (!(showToolbar && toolbarPlacement === 'inline' && editable)) {
                setInlineToolbarFrame(null);
                return;
            }

            const frame = requestAnimationFrame(() => {
                updateToolbarFrame();
            });
            return () => cancelAnimationFrame(frame);
        }, [editable, showToolbar, toolbarPlacement, updateToolbarFrame]);

        useEffect(() => {
            if (heightBehavior !== 'autoGrow') {
                setAutoGrowHeight(null);
            }
        }, [heightBehavior]);

        const handleUpdate = useCallback(
            (event: NativeSyntheticEvent<NativeUpdateEvent>) => {
                const bridge = bridgeRef.current;
                if (!bridge || bridge.isDestroyed) return;
                if (!isCurrentNativeEditorEvent(event.nativeEvent, bridge)) return;

                try {
                    const previousDocumentVersion = documentVersionRef.current;
                    if (Platform.OS === 'android' && typeof previousDocumentVersion === 'number') {
                        const parsed = JSON.parse(event.nativeEvent.updateJson) as {
                            documentVersion?: unknown;
                        };
                        if (typeof parsed.documentVersion !== 'number') {
                            return;
                        }
                    }
                    const nativeUpdate = bridge.parseUpdateJson(
                        event.nativeEvent.updateJson,
                        { rejectSameDocumentVersion: true }
                    );
                    if (!nativeUpdate) return;
                    syncNativeUpdateFromBridge(nativeUpdate, previousDocumentVersion);
                } catch {
                    // Invalid JSON from native — skip
                }
            },
            [syncNativeUpdateFromBridge]
        );

        const handleSelectionChange = useCallback(
            (event: NativeSyntheticEvent<NativeSelectionEvent>) => {
                const bridge = bridgeRef.current;
                if (!bridge || bridge.isDestroyed) return;
                if (!isCurrentNativeEditorEvent(event.nativeEvent, bridge)) return;

                const { anchor, head, stateJson, documentVersion } = event.nativeEvent;
                const currentDocumentVersion = documentVersionRef.current;
                if (
                    typeof documentVersion === 'number' &&
                    typeof currentDocumentVersion === 'number' &&
                    documentVersion < currentDocumentVersion
                ) {
                    return;
                }
                let currentState: EditorUpdate | null = null;
                if (typeof stateJson === 'string' && stateJson.length > 0) {
                    if (
                        Platform.OS === 'android' &&
                        typeof currentDocumentVersion === 'number'
                    ) {
                        const stateDocumentVersion = documentVersionFromUpdateJson(stateJson);
                        if (
                            typeof stateDocumentVersion !== 'number' ||
                            stateDocumentVersion < currentDocumentVersion
                        ) {
                            return;
                        }
                    }
                    try {
                        currentState = bridge.parseUpdateJson(stateJson);
                        if (!currentState) return;
                    } catch {
                        currentState = bridge.getSelectionState();
                    }
                } else {
                    currentState = bridge.getSelectionState();
                }
                if (
                    currentState != null &&
                    typeof currentState.documentVersion === 'number' &&
                    typeof currentDocumentVersion === 'number' &&
                    currentState.documentVersion < currentDocumentVersion
                ) {
                    return;
                }
                if (
                    currentState != null &&
                    Platform.OS === 'android' &&
                    typeof currentDocumentVersion === 'number' &&
                    typeof currentState.documentVersion !== 'number'
                ) {
                    return;
                }
                if (
                    currentState == null &&
                    Platform.OS === 'android' &&
                    typeof currentDocumentVersion === 'number'
                ) {
                    return;
                }
                let selection: Selection;
                const selectionStart = Math.min(anchor, head);
                const selectionEnd = Math.max(anchor, head);

                if (
                    selectionStart === 0 &&
                    selectionEnd >= renderedTextLengthRef.current &&
                    renderedTextLengthRef.current > 0
                ) {
                    selection = { type: 'all' };
                } else {
                    selection = { type: 'text', anchor, head };
                }

                const nextSelection =
                    selection.type === 'all' ? selection : (currentState?.selection ?? selection);
                if (currentState == null) {
                    bridge.updateSelectionFromNative(anchor, head);
                }
                syncSelectionStateFromUpdate(currentState);
                selectionRef.current = nextSelection;
                if (currentState) {
                    onActiveStateChangeRef.current?.(currentState.activeState);
                    onHistoryStateChangeRef.current?.(currentState.historyState);
                }
                onSelectionChangeRef.current?.(nextSelection);
            },
            [syncSelectionStateFromUpdate]
        );

        const handleFocusChange = useCallback(
            (event: NativeSyntheticEvent<NativeFocusEvent>) => {
                if (!isCurrentNativeEditorEvent(event.nativeEvent, bridgeRef.current)) return;
                const { isFocused: focused } = event.nativeEvent;

                const wasFocused = isFocusedRef.current;
                isFocusedRef.current = focused;
                setActiveEditorToolbarFrameOwnerForEditor(editorInstanceIdRef.current, focused);
                setIsFocused(focused);
                if (!focused) {
                    setMentionQueryEventState(null);
                }
                if (focused) {
                    if (!wasFocused) {
                        onFocusRef.current?.();
                    }
                } else if (wasFocused) {
                    onBlurRef.current?.();
                }
            },
            [setMentionQueryEventState]
        );

        useEffect(
            () => () => {
                setActiveEditorToolbarFrameOwnerForEditor(editorInstanceId, false);
            },
            [editorInstanceId]
        );

        useEffect(() => {
            if (addons?.mentions != null) {
                return;
            }
            setMentionQueryEventState(null);
        }, [addons?.mentions, setMentionQueryEventState]);

        const handleContentHeightChange = useCallback(
            (event: NativeSyntheticEvent<NativeContentHeightEvent>) => {
                if (!isCurrentNativeEditorEvent(event.nativeEvent, bridgeRef.current)) return;
                if (heightBehavior !== 'autoGrow') return;
                const density = Platform.OS === 'android' ? PixelRatio.get() : 1;
                const nextHeight = Math.ceil(event.nativeEvent.contentHeight / density);
                if (!(nextHeight > 0)) return;
                setAutoGrowHeight((prev) => (prev === nextHeight ? prev : nextHeight));
            },
            [autoGrowHeight, heightBehavior]
        );

        const handleEditorReady = useCallback(
            (event: NativeSyntheticEvent<NativeEditorReadyEvent>) => {
                if (!isCurrentNativeEditorEvent(event.nativeEvent, bridgeRef.current)) return;
                const editorId = bridgeRef.current?.editorId;
                const acknowledgedRevision = event.nativeEvent.editorUpdateRevision;
                const inFlight = pendingNativeUpdateInFlightRef.current;
                const resetInFlight = pendingNativeResetUpdateInFlightRef.current;
                let didClearInFlight = false;
                let didMatchInFlight = false;
                if (inFlight != null) {
                    const matchesRevision =
                        typeof acknowledgedRevision === 'number' &&
                        inFlight.editorId === editorId &&
                        inFlight.revision === acknowledgedRevision;
                    if (matchesRevision) {
                        didMatchInFlight = true;
                        pendingNativeUpdateInFlightRef.current = null;
                        didClearInFlight = true;
                        setPendingNativeUpdate((current) => {
                            if (
                                current.editorId !== editorId ||
                                current.revision !== acknowledgedRevision ||
                                current.json == null
                            ) {
                                return current;
                            }
                            const next = {
                                json: undefined,
                                editorId,
                                revision: current.revision,
                            };
                            return next;
                        });
                    }
                }
                if (resetInFlight != null) {
                    const matchesRevision =
                        typeof acknowledgedRevision === 'number' &&
                        resetInFlight.editorId === editorId &&
                        resetInFlight.revision === acknowledgedRevision;
                    if (matchesRevision) {
                        didMatchInFlight = true;
                        pendingNativeResetUpdateInFlightRef.current = null;
                        didClearInFlight = true;
                        setPendingNativeResetUpdate((current) => {
                            if (
                                current.editorId !== editorId ||
                                current.revision !== acknowledgedRevision ||
                                current.json == null
                            ) {
                                return current;
                            }
                            const next = {
                                json: undefined,
                                editorId,
                                revision: current.revision,
                            };
                            return next;
                        });
                    }
                }
                if ((inFlight != null || resetInFlight != null) && !didMatchInFlight) {
                    return;
                }
                flushBlockedNativeCommandRetry();
                if (didClearInFlight && pendingControlledSyncAfterNativeUpdateRef.current) {
                    pendingControlledSyncAfterNativeUpdateRef.current = false;
                    setControlledNativeUpdateRetry((revision) => revision + 1);
                }
                if (!pendingDetachedControlledSyncRef.current) return;
                pendingDetachedControlledSyncRef.current = false;
                setDetachedControlledSyncRetry((revision) => revision + 1);
            },
            [flushBlockedNativeCommandRetry]
        );

        const restoreSelection = useCallback((selection: Selection) => {
            if (selection.type === 'text') {
                const { anchor, head } = selection;
                if (anchor == null || head == null) {
                    return;
                }
                bridgeRef.current?.setSelection(anchor, head);
                return;
            }

            if (selection.type === 'node') {
                const { pos } = selection;
                if (pos == null) {
                    return;
                }
                bridgeRef.current?.setSelection(pos, pos);
            }
        }, []);

        const isAsyncRequestCurrent = useCallback(
            (requestEditorId: number | null, requestDocumentVersion: number | null): boolean => {
                const bridge = bridgeRef.current;
                return (
                    bridge != null &&
                    !bridge.isDestroyed &&
                    bridge.editorId === requestEditorId &&
                    documentVersionRef.current === requestDocumentVersion
                );
            },
            []
        );

        const scalarRangeFromSelection = useCallback(
            (selection: Selection): { anchor: number; head: number } | null => {
                const bridge = bridgeRef.current;
                if (!bridge || bridge.isDestroyed) return null;
                if (selection.type === 'text') {
                    const anchor = selection.anchor ?? 0;
                    const head = selection.head ?? anchor;
                    return {
                        anchor: bridge.docToScalar(anchor),
                        head: bridge.docToScalar(head),
                    };
                }
                if (selection.type === 'node' && typeof selection.pos === 'number') {
                    const scalar = bridge.docToScalar(selection.pos);
                    return { anchor: scalar, head: scalar };
                }
                return null;
            },
            []
        );

        const isCommandRetryScopeCurrent = useCallback(
            (scope: CommandRetryScope): boolean => {
                const bridge = bridgeRef.current;
                if (!bridge || bridge.isDestroyed || bridge.editorId !== scope.editorId) {
                    return false;
                }
                const currentDocumentVersion = documentVersionRef.current;
                const hasMentionScope =
                    scope.mentionQuery != null ||
                    scope.mentionRange != null ||
                    scope.nativeMentionSelectRequest != null;
                if (currentDocumentVersion !== scope.documentVersion) {
                    if (!hasMentionScope) {
                        return false;
                    }
                    if (
                        typeof currentDocumentVersion === 'number' &&
                        typeof scope.documentVersion === 'number' &&
                        currentDocumentVersion > scope.documentVersion
                    ) {
                        return false;
                    }
                }
                if (
                    hasMentionScope &&
                    typeof currentDocumentVersion === 'number' &&
                    scope.documentVersion == null
                ) {
                    return false;
                }
                if (
                    typeof scope.scalarAnchor === 'number' &&
                    typeof scope.scalarHead === 'number'
                ) {
                    const currentRange = scalarRangeFromSelection(selectionRef.current);
                    if (
                        currentRange?.anchor !== scope.scalarAnchor ||
                        currentRange?.head !== scope.scalarHead
                    ) {
                        return false;
                    }
                }
                if (scope.mentionQuery) {
                    const mentionQueryEditorId = mentionQueryEditorIdRef.current;
                    if (
                        mentionQueryEditorId != null &&
                        mentionQueryEditorId !== scope.editorId
                    ) {
                        return false;
                    }
                    const currentQuery = mentionQueryEventRef.current;
                    if (
                        currentQuery == null ||
                        currentQuery.trigger !== scope.mentionQuery.trigger ||
                        currentQuery.query !== scope.mentionQuery.query ||
                        currentQuery.range.anchor !== scope.mentionQuery.range.anchor ||
                        currentQuery.range.head !== scope.mentionQuery.range.head ||
                        currentQuery.documentVersion !== scope.mentionQuery.documentVersion
                    ) {
                        return false;
                    }
                }
                if (scope.mentionRange) {
                    const mentionQueryEditorId = mentionQueryEditorIdRef.current;
                    if (
                        mentionQueryEditorId != null &&
                        mentionQueryEditorId !== scope.editorId
                    ) {
                        return false;
                    }
                    const currentQuery = mentionQueryEventRef.current;
                    if (
                        currentQuery == null ||
                        currentQuery.range.anchor !== scope.mentionRange.anchor ||
                        currentQuery.range.head !== scope.mentionRange.head ||
                        (currentQuery.documentVersion ?? null) !== scope.documentVersion
                    ) {
                        return false;
                    }
                }
                if (scope.nativeMentionSelectRequest) {
                    const mentionQueryEditorId = mentionQueryEditorIdRef.current;
                    if (
                        mentionQueryEditorId != null &&
                        mentionQueryEditorId !== scope.editorId
                    ) {
                        return false;
                    }
                    if (
                        doesLiveMentionQueryConflictWithNativeSelectRequest(
                            scope.nativeMentionSelectRequest,
                            mentionQueryEventRef.current,
                            scope.documentVersion,
                            currentDocumentVersion
                        )
                    ) {
                        return false;
                    }
                }
                return true;
            },
            [scalarRangeFromSelection]
        );

        const runAndApplyWithCommandRetry = useCallback(
            (
                scope: CommandRetryScope,
                mutate: () => EditorUpdate | null,
                options?: RunAndApplyOptions,
                onApplied?: (update: EditorUpdate) => void
            ): { update: EditorUpdate | null; queued: boolean } => {
                let didQueueRetry = false;
                let run: (() => EditorUpdate | null) | null = null;
                const runIfScopeCurrent = (): EditorUpdate | null => {
                    if (!isCommandRetryScopeCurrent(scope)) return null;
                    return run?.() ?? null;
                };
                const retry = (): boolean => {
                    const update = runIfScopeCurrent();
                    if (update) {
                        onApplied?.(update);
                    }
                    return update != null;
                };
                run = () =>
                    runAndApply(mutate, {
                        ...options,
                        retryBlockedCommand: retry,
                        onBlockedCommandRetryQueued: () => {
                            didQueueRetry = true;
                            options?.onBlockedCommandRetryQueued?.();
                        },
                    });
                const update = runIfScopeCurrent();
                if (update) {
                    onApplied?.(update);
                }
                return { update, queued: didQueueRetry };
            },
            [isCommandRetryScopeCurrent, runAndApply]
        );

        const runPersistentContentCommand = useCallback(
            (
                mutate: () => EditorUpdate | null,
                options?: Omit<
                    RunAndApplyOptions,
                    'retryBlockedCommand' | 'onBlockedCommandRetryQueued'
                >
            ): EditorUpdate | null => {
                const requestedEditorId = bridgeRef.current?.editorId;
                let run: (() => EditorUpdate | null) | null = null;
                const retry = (): boolean => {
                    const bridge = bridgeRef.current;
                    if (
                        bridge == null ||
                        bridge.isDestroyed ||
                        bridge.editorId !== requestedEditorId
                    ) {
                        return false;
                    }
                    return run?.() != null;
                };
                run = () =>
                    runAndApply(mutate, {
                        ...options,
                        retryBlockedCommand: retry,
                    });
                return run();
            },
            [runAndApply]
        );

        const insertImage = useCallback(
            (src: string, attrs?: Omit<ImageNodeAttributes, 'src'>, selection?: Selection) => {
                const trimmedSrc = src.trim();
                if (!trimmedSrc) return;
                if (!allowBase64Images && isImageDataUrl(trimmedSrc)) {
                    return;
                }
                runAndApply(() => {
                    if (selection) {
                        restoreSelection(selection);
                    }
                    return (
                        bridgeRef.current?.insertContentJson(
                            buildImageFragmentJson({
                                src: trimmedSrc,
                                ...(attrs ?? {}),
                            })
                        ) ?? null
                    );
                });
            },
            [allowBase64Images, restoreSelection, runAndApply]
        );

        const openLinkRequest = useCallback(() => {
            const requestSelection = selectionRef.current;
            const requestDocumentVersion = documentVersionRef.current;
            const requestEditorId = bridgeRef.current?.editorId ?? null;
            const linkState = activeStateRef.current;
            const requestLinkHref =
                typeof linkState.markAttrs?.link?.href === 'string'
                    ? (linkState.markAttrs.link.href as string)
                    : undefined;

            onRequestLink?.({
                href: requestLinkHref,
                isActive: linkState.marks.link === true,
                selection: requestSelection,
                setLink: (href: string) => {
                    const trimmedHref = href.trim();
                    if (!trimmedHref) return;
                    if (!isAsyncRequestCurrent(requestEditorId, requestDocumentVersion)) return;
                    const bridge = bridgeRef.current;
                    if (!bridge || bridge.isDestroyed) return;
                    const scalarSelection = scalarRangeFromSelection(requestSelection);
                    if (!scalarSelection) return;
                    runAndApplyWithCommandRetry(
                        {
                            editorId: bridge.editorId,
                            documentVersion: requestDocumentVersion,
                            scalarAnchor: scalarSelection.anchor,
                            scalarHead: scalarSelection.head,
                        },
                        () =>
                            bridgeRef.current?.setMarkAtSelectionScalar(
                                scalarSelection.anchor,
                                scalarSelection.head,
                                'link',
                                { href: trimmedHref }
                            ) ?? null,
                        { skipNativeApplyIfContentUnchanged: true }
                    );
                },
                unsetLink: () => {
                    if (!isAsyncRequestCurrent(requestEditorId, requestDocumentVersion)) return;
                    const bridge = bridgeRef.current;
                    if (!bridge || bridge.isDestroyed) return;
                    const scalarSelection = scalarRangeFromSelection(requestSelection);
                    if (!scalarSelection) return;
                    runAndApplyWithCommandRetry(
                        {
                            editorId: bridge.editorId,
                            documentVersion: requestDocumentVersion,
                            scalarAnchor: scalarSelection.anchor,
                            scalarHead: scalarSelection.head,
                        },
                        () =>
                            bridgeRef.current?.unsetMarkAtSelectionScalar(
                                scalarSelection.anchor,
                                scalarSelection.head,
                                'link'
                            ) ?? null,
                        { skipNativeApplyIfContentUnchanged: true }
                    );
                },
            });
        }, [
            isAsyncRequestCurrent,
            onRequestLink,
            runAndApplyWithCommandRetry,
            scalarRangeFromSelection,
        ]);

        const openImageRequest = useCallback(() => {
            const requestSelection = selectionRef.current;
            const requestDocumentVersion = documentVersionRef.current;
            const requestEditorId = bridgeRef.current?.editorId ?? null;

            onRequestImage?.({
                selection: requestSelection,
                allowBase64: allowBase64Images,
                insertImage: (src: string, attrs?: Omit<ImageNodeAttributes, 'src'>) => {
                    const trimmedSrc = src.trim();
                    if (!trimmedSrc) return;
                    if (!allowBase64Images && isImageDataUrl(trimmedSrc)) return;
                    if (!isAsyncRequestCurrent(requestEditorId, requestDocumentVersion)) return;
                    const bridge = bridgeRef.current;
                    if (!bridge || bridge.isDestroyed) return;
                    const scalarSelection = scalarRangeFromSelection(requestSelection);
                    if (!scalarSelection) return;
                    runAndApplyWithCommandRetry(
                        {
                            editorId: bridge.editorId,
                            documentVersion: requestDocumentVersion,
                            scalarAnchor: scalarSelection.anchor,
                            scalarHead: scalarSelection.head,
                        },
                        () =>
                            bridgeRef.current?.insertContentJsonAtSelectionScalar(
                                scalarSelection.anchor,
                                scalarSelection.head,
                                buildImageFragmentJson({
                                    src: trimmedSrc,
                                    ...(attrs ?? {}),
                                })
                            ) ?? null
                    );
                },
            });
        }, [
            allowBase64Images,
            isAsyncRequestCurrent,
            onRequestImage,
            runAndApplyWithCommandRetry,
            scalarRangeFromSelection,
        ]);

        const handleToolbarAction = useCallback(
            (event: NativeSyntheticEvent<NativeToolbarActionEvent>) => {
                if (!isCurrentNativeEditorEvent(event.nativeEvent, bridgeRef.current)) return;
                const currentDocumentVersion = documentVersionRef.current;
                const eventDocumentVersion = event.nativeEvent.documentVersion;
                if (
                    typeof eventDocumentVersion === 'number' &&
                    typeof currentDocumentVersion === 'number' &&
                    eventDocumentVersion < currentDocumentVersion
                ) {
                    return;
                }
                if (
                    Platform.OS === 'android' &&
                    typeof currentDocumentVersion === 'number' &&
                    typeof eventDocumentVersion !== 'number' &&
                    typeof event.nativeEvent.updateJson !== 'string'
                ) {
                    return;
                }
                if (!syncPreflightUpdateFromNativeEvent(event.nativeEvent.updateJson)) {
                    return;
                }
                if (event.nativeEvent.key === LINK_TOOLBAR_ACTION_KEY) {
                    openLinkRequest();
                    return;
                }
                if (event.nativeEvent.key === IMAGE_TOOLBAR_ACTION_KEY) {
                    openImageRequest();
                    return;
                }
                onToolbarAction?.(event.nativeEvent.key);
            },
            [
                onToolbarAction,
                openImageRequest,
                openLinkRequest,
                syncPreflightUpdateFromNativeEvent,
            ]
        );

        const resolveMentionSelectionAttrs = useCallback(
            (selectionEvent: MentionSelectionAttrsEvent): Record<string, unknown> => {
                let resolvedAttrs: Record<string, unknown> | null | undefined;
                try {
                    resolvedAttrs =
                        addonsRef.current?.mentions?.resolveSelectionAttrs?.(selectionEvent);
                } catch (error) {
                    if (__DEV__) {
                        console.error(
                            'NativeRichTextEditor: mentions.resolveSelectionAttrs threw',
                            error
                        );
                    }
                }

                return isRecord(resolvedAttrs)
                    ? { ...selectionEvent.attrs, ...resolvedAttrs }
                    : selectionEvent.attrs;
            },
            []
        );

        const resolveMentionInsertionAttrs = useCallback(
            (selectionEvent: MentionSelectionAttrsEvent): Record<string, unknown> => {
                const attrs = resolveMentionSelectionAttrs(selectionEvent);
                let resolvedTheme: EditorMentionTheme | null | undefined;
                try {
                    resolvedTheme = addonsRef.current?.mentions?.resolveTheme?.({
                        ...selectionEvent,
                        attrs,
                    });
                } catch (error) {
                    if (__DEV__) {
                        console.error('NativeRichTextEditor: mentions.resolveTheme threw', error);
                    }
                }

                return isRecord(resolvedTheme) ? { ...attrs, mentionTheme: resolvedTheme } : attrs;
            },
            [resolveMentionSelectionAttrs]
        );

        const handleInlineMentionSuggestionPress = useCallback(
            (suggestion: MentionSuggestion) => {
                const mentionQuery = mentionQueryEventRef.current;
                const mentions = addonsRef.current?.mentions;
                if (
                    !mentionQuery ||
                    !mentions ||
                    !bridgeRef.current ||
                    bridgeRef.current.isDestroyed
                ) {
                    return;
                }
                const currentDocumentVersion = documentVersionRef.current;
                if (
                    typeof mentionQuery.documentVersion === 'number' &&
                    typeof currentDocumentVersion === 'number' &&
                    mentionQuery.documentVersion < currentDocumentVersion
                ) {
                    setMentionQueryEventState(null);
                    return;
                }

                let attrs: Record<string, unknown> | null = null;

                const requestDocumentVersion =
                    typeof mentionQuery.documentVersion === 'number'
                        ? mentionQuery.documentVersion
                        : currentDocumentVersion;
                const retryScope: CommandRetryScope = {
                    editorId: bridgeRef.current.editorId,
                    documentVersion: requestDocumentVersion,
                    mentionQuery,
                };
                let queuedRetry = false;
                let retry: (() => boolean) | null = null;
                const finishSelection = (selectedAttrs: Record<string, unknown>) => {
                    setMentionQueryEventState(null);
                    mentions.onSelect?.({
                        trigger: mentionQuery.trigger,
                        suggestion,
                        attrs: selectedAttrs,
                        ...(typeof mentionQuery.documentVersion === 'number'
                            ? { documentVersion: mentionQuery.documentVersion }
                            : {}),
                    });
                };
                const attemptInsertion = (): boolean => {
                    if (!isCommandRetryScopeCurrent(retryScope)) return false;
                    attrs = null;
                    const update = runAndApply(
                        () =>
                            bridgeRef.current?.insertContentJsonAtSelectionScalarLazy(
                                mentionQuery.range.anchor,
                                mentionQuery.range.head,
                                () => {
                                    attrs = resolveMentionInsertionAttrs({
                                        trigger: mentionQuery.trigger,
                                        suggestion,
                                        attrs: resolveMentionSuggestionAttrs(
                                            suggestion,
                                            mentionQuery.trigger
                                        ),
                                        range: mentionQuery.range,
                                        ...(typeof mentionQuery.documentVersion === 'number'
                                            ? { documentVersion: mentionQuery.documentVersion }
                                            : {}),
                                    });
                                    return buildMentionFragmentJson(attrs);
                                }
                            ) ?? null,
                        {
                            retryBlockedCommand: () => retry?.() === true,
                            onBlockedCommandRetryQueued: () => {
                                queuedRetry = true;
                            },
                        }
                    );
                    if (update && attrs) {
                        finishSelection(attrs);
                        return true;
                    }
                    return false;
                };
                retry = attemptInsertion;

                if (attemptInsertion()) {
                    return;
                }

                if (queuedRetry) {
                    return;
                }

                const latestMentionQuery = mentionQueryEventRef.current;
                if (
                    latestMentionQuery == null ||
                    (latestMentionQuery.trigger === mentionQuery.trigger &&
                        latestMentionQuery.query === mentionQuery.query &&
                        latestMentionQuery.range.anchor === mentionQuery.range.anchor &&
                        latestMentionQuery.range.head === mentionQuery.range.head &&
                        latestMentionQuery.documentVersion === mentionQuery.documentVersion)
                ) {
                    setMentionQueryEventState(null);
                }
            },
            [
                isCommandRetryScopeCurrent,
                resolveMentionInsertionAttrs,
                runAndApply,
                setMentionQueryEventState,
            ]
        );

        const handleAddonEvent = useCallback(
            (event: NativeSyntheticEvent<NativeAddonEvent>) => {
                const bridge = bridgeRef.current;
                if (!isCurrentNativeEditorEvent(event.nativeEvent, bridge)) return;
                let parsed: EditorAddonEvent | null = null;
                try {
                    parsed = JSON.parse(event.nativeEvent.eventJson) as EditorAddonEvent;
                } catch {
                    return;
                }
                if (!parsed) return;
                let parsedDocumentVersion =
                    typeof parsed.documentVersion === 'number' ? parsed.documentVersion : undefined;
                if (
                    typeof parsedDocumentVersion !== 'number' &&
                    parsed.type === 'mentionsSelectRequest' &&
                    typeof parsed.updateJson === 'string'
                ) {
                    try {
                        const parsedUpdate = JSON.parse(parsed.updateJson) as {
                            documentVersion?: unknown;
                        };
                        if (typeof parsedUpdate.documentVersion === 'number') {
                            parsedDocumentVersion = parsedUpdate.documentVersion;
                        }
                    } catch {
                        return;
                    }
                }
                const currentDocumentVersion = documentVersionRef.current;
                const isStaleAddonEvent =
                    typeof parsedDocumentVersion === 'number' &&
                    typeof currentDocumentVersion === 'number' &&
                    parsedDocumentVersion < currentDocumentVersion;
                const isVersionlessAndroidAddonEvent =
                    Platform.OS === 'android' &&
                    typeof currentDocumentVersion === 'number' &&
                    typeof parsedDocumentVersion !== 'number';

                if (parsed.type === 'mentionsQueryChange') {
                    if (isStaleAddonEvent || isVersionlessAndroidAddonEvent) return;
                    const nextEvent: MentionQueryChangeEvent = {
                        query: parsed.query,
                        trigger: parsed.trigger,
                        range: parsed.range,
                        isActive: parsed.isActive,
                        documentVersion: parsedDocumentVersion,
                    };
                    setMentionQueryEventState(
                        parsed.isActive ? nextEvent : null,
                        event.nativeEvent.editorId ?? bridgeRef.current?.editorId ?? null
                    );
                    addonsRef.current?.mentions?.onQueryChange?.({
                        query: nextEvent.query,
                        trigger: nextEvent.trigger,
                        range: nextEvent.range,
                        isActive: nextEvent.isActive,
                        ...(typeof nextEvent.documentVersion === 'number'
                            ? { documentVersion: nextEvent.documentVersion }
                            : {}),
                    });
                    return;
                }

                if (parsed.type === 'mentionsSelectRequest') {
                    if (isStaleAddonEvent || isVersionlessAndroidAddonEvent) return;
                    const requestDocumentVersion =
                        typeof parsedDocumentVersion === 'number'
                            ? parsedDocumentVersion
                            : currentDocumentVersion;
                    const nativeMentionSelectRequest: NativeMentionSelectRetryScope = {
                        trigger: parsed.trigger,
                        suggestionKey: parsed.suggestionKey,
                        range: parsed.range,
                    };
                    if (
                        doesLiveMentionQueryConflictWithNativeSelectRequest(
                            nativeMentionSelectRequest,
                            mentionQueryEventRef.current,
                            requestDocumentVersion,
                            currentDocumentVersion
                        )
                    ) {
                        return;
                    }
                    if (!syncPreflightUpdateFromNativeEvent(parsed.updateJson)) return;
                    const suggestion = mentionSuggestionsByKeyRef.current.get(parsed.suggestionKey);
                    if (!suggestion || !bridgeRef.current || bridgeRef.current.isDestroyed) return;

                    let finalAttrs: Record<string, unknown> | null = null;
                    runAndApplyWithCommandRetry(
                        {
                            editorId: bridgeRef.current.editorId,
                            documentVersion: requestDocumentVersion,
                            nativeMentionSelectRequest,
                        },
                        () => {
                            return (
                                bridgeRef.current?.insertContentJsonAtSelectionScalarLazy(
                                    parsed.range.anchor,
                                    parsed.range.head,
                                    () => {
                                        const selectionEvent: MentionSelectionAttrsEvent = {
                                            trigger: parsed.trigger,
                                            suggestion,
                                            attrs: parsed.attrs,
                                            range: parsed.range,
                                            ...(typeof parsedDocumentVersion === 'number'
                                                ? { documentVersion: parsedDocumentVersion }
                                                : {}),
                                        };
                                        const attrs = resolveMentionInsertionAttrs(selectionEvent);
                                        finalAttrs = attrs;
                                        return buildMentionFragmentJson(attrs);
                                    }
                                ) ?? null
                            );
                        },
                        undefined,
                        () => {
                            if (!finalAttrs) return;
                            addonsRef.current?.mentions?.onSelect?.({
                                trigger: parsed.trigger,
                                suggestion,
                                attrs: finalAttrs,
                                ...(typeof parsedDocumentVersion === 'number'
                                    ? { documentVersion: parsedDocumentVersion }
                                    : {}),
                            });
                        }
                    );
                    return;
                }

                if (parsed.type === 'mentionsSelect') {
                    if (isStaleAddonEvent || isVersionlessAndroidAddonEvent) return;
                    const suggestion = mentionSuggestionsByKeyRef.current.get(parsed.suggestionKey);
                    if (!suggestion) return;
                    addonsRef.current?.mentions?.onSelect?.({
                        trigger: parsed.trigger,
                        suggestion,
                        attrs: parsed.attrs,
                        ...(typeof parsedDocumentVersion === 'number'
                            ? { documentVersion: parsedDocumentVersion }
                            : {}),
                    });
                }
            },
            [
                resolveMentionInsertionAttrs,
                runAndApplyWithCommandRetry,
                setMentionQueryEventState,
                syncPreflightUpdateFromNativeEvent,
            ]
        );

        useImperativeHandle(
            ref,
            () => ({
                focus() {
                    nativeViewRef.current?.focus?.();
                },
                blur() {
                    nativeViewRef.current?.blur?.();
                },
                toggleMark(markType: string) {
                    runAndApply(() => bridgeRef.current?.toggleMark(markType) ?? null, {
                        skipNativeApplyIfContentUnchanged: true,
                    });
                },
                setLink(href: string) {
                    const trimmedHref = href.trim();
                    if (!trimmedHref) return;
                    runAndApply(
                        () => bridgeRef.current?.setMark('link', { href: trimmedHref }) ?? null,
                        { skipNativeApplyIfContentUnchanged: true }
                    );
                },
                unsetLink() {
                    runAndApply(() => bridgeRef.current?.unsetMark('link') ?? null, {
                        skipNativeApplyIfContentUnchanged: true,
                    });
                },
                toggleBlockquote() {
                    runAndApply(() => bridgeRef.current?.toggleBlockquote() ?? null);
                },
                toggleHeading(level: EditorToolbarHeadingLevel) {
                    runAndApply(() => bridgeRef.current?.toggleHeading(level) ?? null);
                },
                toggleList(listType: string) {
                    runAndApply(() => bridgeRef.current?.toggleList(listType) ?? null);
                },
                indentListItem() {
                    runAndApply(() => bridgeRef.current?.indentListItem() ?? null);
                },
                outdentListItem() {
                    runAndApply(() => bridgeRef.current?.outdentListItem() ?? null);
                },
                insertNode(nodeType: string) {
                    runAndApply(() => bridgeRef.current?.insertNode(nodeType) ?? null);
                },
                insertImage(src: string, attrs?: Omit<ImageNodeAttributes, 'src'>) {
                    insertImage(src, attrs);
                },
                insertText(text: string) {
                    runAndApply(() => bridgeRef.current?.replaceSelectionText(text) ?? null);
                },
                insertContentHtml(html: string) {
                    runAndApply(() => bridgeRef.current?.insertContentHtml(html) ?? null);
                },
                insertContentJson(doc: DocumentJSON) {
                    runAndApply(() => bridgeRef.current?.insertContentJson(doc) ?? null);
                },
                setContent(html: string) {
                    runPersistentContentCommand(
                        () => bridgeRef.current?.replaceHtml(html) ?? null
                    );
                },
                setContentJson(doc: DocumentJSON) {
                    const jsonString = stringifyCachedJson(
                        normalizeDocumentJson(doc, documentSchema)
                    );
                    runPersistentContentCommand(
                        () => bridgeRef.current?.replaceJsonString(jsonString) ?? null
                    );
                },
                clearContent() {
                    const jsonString = stringifyCachedJson(
                        normalizeDocumentJson({ type: 'doc', content: [] }, documentSchema)
                    );
                    resetContentJsonString(jsonString);
                },
                getContent(): string {
                    if (!bridgeRef.current || bridgeRef.current.isDestroyed) return '';
                    if (!prepareBridgeForExternalContentRead()) {
                        return bridgeRef.current.getCachedHtml() ?? '';
                    }
                    return bridgeRef.current.getHtml();
                },
                getContentJson(): DocumentJSON {
                    if (!bridgeRef.current || bridgeRef.current.isDestroyed) return {};
                    if (!prepareBridgeForExternalContentRead()) {
                        return bridgeRef.current.getCachedJson() ?? {};
                    }
                    return bridgeRef.current.getJson();
                },
                getTextContent(): string {
                    if (!bridgeRef.current || bridgeRef.current.isDestroyed) return '';
                    if (!prepareBridgeForExternalContentRead()) {
                        return (bridgeRef.current.getCachedHtml() ?? '').replace(/<[^>]+>/g, '');
                    }
                    return bridgeRef.current.getHtml().replace(/<[^>]+>/g, '');
                },
                async getCaretRect(): Promise<NativeRichTextEditorCaretRect | null> {
                    const nativeView = nativeViewRef.current;
                    if (!nativeView?.getCaretRect) return null;
                    const raw = await Promise.resolve(nativeView.getCaretRect());
                    return parseCaretRectJson(raw);
                },
                undo() {
                    runAndApply(() => bridgeRef.current?.undo() ?? null);
                },
                redo() {
                    runAndApply(() => bridgeRef.current?.redo() ?? null);
                },
                canUndo(): boolean {
                    if (!bridgeRef.current || bridgeRef.current.isDestroyed) return false;
                    return bridgeRef.current.canUndo();
                },
                canRedo(): boolean {
                    if (!bridgeRef.current || bridgeRef.current.isDestroyed) return false;
                    return bridgeRef.current.canRedo();
                },
            }),
            [
                documentSchema,
                insertImage,
                prepareBridgeForExternalContentRead,
                runAndApply,
                runPersistentContentCommand,
                resetContentJsonString,
            ]
        );

        const activeMentionTrigger = mentionQueryEvent?.trigger || resolveMentionTrigger(addons);
        const activeMentionSuggestions = useMemo(
            () =>
                isFocused && mentionQueryEvent != null && addons?.mentions != null
                    ? filterMentionSuggestions(
                          addons.mentions.suggestions ?? [],
                          mentionQueryEvent.query,
                          activeMentionTrigger
                      )
                    : [],
            [activeMentionTrigger, addons?.mentions, isFocused, mentionQueryEvent]
        );
        const inlineToolbarMentionTheme = theme?.mentions ?? addons?.mentions?.theme;
        const activeMentionSuggestionThemes = useMemo(() => {
            if (
                mentionQueryEvent == null ||
                addons?.mentions == null ||
                typeof addons.mentions.resolveTheme !== 'function' ||
                activeMentionSuggestions.length === 0
            ) {
                return undefined;
            }

            const suggestionThemes: Record<string, EditorMentionTheme> = {};

            for (const suggestion of activeMentionSuggestions) {
                const selectionEvent: MentionSelectionAttrsEvent = {
                    trigger: activeMentionTrigger,
                    suggestion,
                    attrs: resolveMentionSuggestionAttrs(suggestion, activeMentionTrigger),
                    range: mentionQueryEvent.range,
                    ...(typeof mentionQueryEvent.documentVersion === 'number'
                        ? { documentVersion: mentionQueryEvent.documentVersion }
                        : {}),
                };
                const attrs = resolveMentionSelectionAttrs(selectionEvent);
                let resolvedTheme: EditorMentionTheme | undefined;
                try {
                    const nextTheme = addons.mentions.resolveTheme({
                        ...selectionEvent,
                        attrs,
                    });
                    resolvedTheme = isRecord(nextTheme)
                        ? (nextTheme as EditorMentionTheme)
                        : undefined;
                } catch (error) {
                    if (__DEV__) {
                        console.error('NativeRichTextEditor: mentions.resolveTheme threw', error);
                    }
                }

                const mergedTheme = mergeMentionSuggestionTheme(
                    inlineToolbarMentionTheme,
                    resolvedTheme
                );
                if (mergedTheme != null) {
                    suggestionThemes[suggestion.key] = mergedTheme;
                }
            }

            return Object.keys(suggestionThemes).length > 0 ? suggestionThemes : undefined;
        }, [
            activeMentionSuggestions,
            activeMentionTrigger,
            addons?.mentions,
            inlineToolbarMentionTheme,
            mentionQueryEvent,
            resolveMentionSelectionAttrs,
        ]);
        const shouldPublishStandaloneMentionSuggestions =
            editable &&
            !showToolbar &&
            registeredToolbarFrames.length > 0 &&
            mentionQueryEvent != null &&
            activeMentionSuggestions.length > 0 &&
            addons?.mentions != null;
        useEffect(() => {
            if (editorInstanceId === 0) {
                return;
            }

            if (!shouldPublishStandaloneMentionSuggestions || mentionQueryEvent == null) {
                setEditorToolbarMentionState(editorInstanceId, null);
                return () => setEditorToolbarMentionState(editorInstanceId, null);
            }

            setEditorToolbarMentionState(editorInstanceId, {
                trigger: activeMentionTrigger,
                suggestions: activeMentionSuggestions,
                theme: inlineToolbarMentionTheme,
                suggestionThemes: activeMentionSuggestionThemes,
                onSelectSuggestion: handleInlineMentionSuggestionPress,
            });
            return () => setEditorToolbarMentionState(editorInstanceId, null);
        }, [
            activeMentionSuggestions,
            activeMentionSuggestionThemes,
            activeMentionTrigger,
            editorInstanceId,
            handleInlineMentionSuggestionPress,
            inlineToolbarMentionTheme,
            mentionQueryEvent,
            shouldPublishStandaloneMentionSuggestions,
        ]);

        if (!isReady) return null;

        const isLinkActive = activeState.marks.link === true;
        const allowsLink = activeState.allowedMarks.includes('link');
        const canInsertImage = activeState.insertableNodes.includes(IMAGE_NODE_NAME);
        const canRequestLink = typeof onRequestLink === 'function';
        const canRequestImage = typeof onRequestImage === 'function';
        const cachedToolbarItems = toolbarItemsSerializationCacheRef.current;
        let toolbarItemsJson: string;
        if (
            cachedToolbarItems?.toolbarItems === toolbarItems &&
            cachedToolbarItems.editable === editable &&
            cachedToolbarItems.isLinkActive === isLinkActive &&
            cachedToolbarItems.allowsLink === allowsLink &&
            cachedToolbarItems.canRequestLink === canRequestLink &&
            cachedToolbarItems.canRequestImage === canRequestImage &&
            cachedToolbarItems.canInsertImage === canInsertImage
        ) {
            toolbarItemsJson = cachedToolbarItems.serialized;
        } else {
            const mappedItems = mapToolbarItemsForNative(
                toolbarItems,
                activeState,
                editable,
                onRequestLink,
                onRequestImage
            );
            toolbarItemsJson = stringifyCachedJson(mappedItems);
            toolbarItemsSerializationCacheRef.current = {
                toolbarItems,
                editable,
                isLinkActive,
                allowsLink,
                canRequestLink,
                canRequestImage,
                canInsertImage,
                mappedItems,
                serialized: toolbarItemsJson,
            };
        }
        const usesNativeKeyboardToolbar =
            toolbarPlacement === 'keyboard' && (Platform.OS === 'ios' || Platform.OS === 'android');
        const shouldRenderJsToolbar = !usesNativeKeyboardToolbar && showToolbar && editable;
        const inlineToolbarChrome = {
            backgroundColor: theme?.toolbar?.backgroundColor,
            borderColor: theme?.toolbar?.borderColor,
            borderWidth: theme?.toolbar?.borderWidth,
            borderRadius: theme?.toolbar?.borderRadius,
        };
        const inlineToolbarMarginTop = theme?.toolbar?.marginTop ?? 8;
        const inlineToolbarShowTopBorder = theme?.toolbar?.showTopBorder ?? false;
        const inlineToolbarContentTopBorderStyle = inlineToolbarShowTopBorder
            ? {
                  borderTopWidth: theme?.toolbar?.borderWidth ?? StyleSheet.hairlineWidth,
                  borderTopColor: theme?.toolbar?.borderColor ?? INLINE_TOOLBAR_BORDER_COLOR,
              }
            : null;
        const inlineMentionSuggestions =
            toolbarPlacement === 'inline' ? activeMentionSuggestions : [];
        const shouldShowInlineMentionSuggestions =
            shouldRenderJsToolbar &&
            toolbarPlacement === 'inline' &&
            isFocused &&
            inlineMentionSuggestions.length > 0;
        const containerMinHeight = StyleSheet.flatten(containerStyle)?.minHeight;
        const nativeViewStyleParts: StyleProp<ViewStyle>[] = [];
        if (containerMinHeight != null) {
            nativeViewStyleParts.push({ minHeight: containerMinHeight });
        }
        if (style != null) {
            nativeViewStyleParts.push(style);
        }
        if (heightBehavior === 'autoGrow' && autoGrowHeight != null) {
            nativeViewStyleParts.push({ height: autoGrowHeight });
        }
        const nativeViewStyle =
            nativeViewStyleParts.length <= 1 ? nativeViewStyleParts[0] : nativeViewStyleParts;
        const toolbarFrameJson = serializeToolbarFrames(
            editable && isFocused
                ? [
                      ...(toolbarPlacement === 'inline' && inlineToolbarFrame != null
                          ? [inlineToolbarFrame]
                          : []),
                      ...registeredToolbarFrames,
                  ]
                : undefined
        );
        const jsToolbar = (
            <View
                ref={toolbarRef}
                testID='native-editor-js-toolbar'
                style={[
                    styles.inlineToolbar,
                    { marginTop: inlineToolbarMarginTop },
                    inlineToolbarChrome.backgroundColor != null
                        ? { backgroundColor: inlineToolbarChrome.backgroundColor }
                        : null,
                    inlineToolbarChrome.borderColor != null
                        ? { borderColor: inlineToolbarChrome.borderColor }
                        : null,
                    inlineToolbarChrome.borderWidth != null
                        ? { borderWidth: inlineToolbarChrome.borderWidth }
                        : null,
                    inlineToolbarChrome.borderRadius != null
                        ? { borderRadius: inlineToolbarChrome.borderRadius }
                        : null,
                ]}
                onLayout={updateToolbarFrame}>
                {shouldShowInlineMentionSuggestions ? (
                    <View
                        testID='native-editor-inline-mention-suggestions'
                        style={[
                            styles.inlineMentionSuggestionsContainer,
                            inlineToolbarContentTopBorderStyle,
                        ]}>
                        <ScrollView
                            horizontal
                            showsHorizontalScrollIndicator={false}
                            contentContainerStyle={styles.inlineMentionSuggestionsContent}
                            keyboardShouldPersistTaps='always'>
                            {inlineMentionSuggestions.map((suggestion) => {
                                const label = resolveMentionSuggestionDisplayLabel(
                                    suggestion,
                                    mentionQueryEvent?.trigger ?? resolveMentionTrigger(addons)
                                );
                                const suggestionTheme =
                                    activeMentionSuggestionThemes?.[suggestion.key] ??
                                    inlineToolbarMentionTheme;

                                return (
                                    <Pressable
                                        key={suggestion.key}
                                        testID={`native-editor-inline-mention-suggestion-${suggestion.key}`}
                                        onPress={() =>
                                            handleInlineMentionSuggestionPress(suggestion)
                                        }
                                        accessibilityRole='button'
                                        accessibilityLabel={label}
                                        style={({ pressed }) => [
                                            styles.inlineMentionSuggestion,
                                            {
                                                backgroundColor: pressed
                                                    ? (suggestionTheme?.optionHighlightedBackgroundColor ??
                                                      'rgba(0, 122, 255, 0.12)')
                                                    : (suggestionTheme?.backgroundColor ??
                                                      '#F2F2F7'),
                                                borderColor:
                                                    suggestionTheme?.borderColor ?? 'transparent',
                                                borderWidth: suggestionTheme?.borderWidth ?? 0,
                                                borderRadius: suggestionTheme?.borderRadius ?? 12,
                                            },
                                        ]}>
                                        {({ pressed }) => (
                                            <>
                                                <Text
                                                    numberOfLines={1}
                                                    style={[
                                                        styles.inlineMentionSuggestionTitle,
                                                        {
                                                            fontWeight:
                                                                suggestionTheme?.fontWeight ??
                                                                '600',
                                                            color: pressed
                                                                ? (suggestionTheme?.optionHighlightedTextColor ??
                                                                  suggestionTheme?.optionTextColor ??
                                                                  '#000000')
                                                                : (suggestionTheme?.optionTextColor ??
                                                                  suggestionTheme?.textColor ??
                                                                  '#000000'),
                                                        },
                                                    ]}>
                                                    {label}
                                                </Text>
                                                {suggestion.subtitle ? (
                                                    <Text
                                                        numberOfLines={1}
                                                        style={[
                                                            styles.inlineMentionSuggestionSubtitle,
                                                            {
                                                                color:
                                                                    suggestionTheme?.optionSecondaryTextColor ??
                                                                    '#8E8E93',
                                                            },
                                                        ]}>
                                                        {suggestion.subtitle}
                                                    </Text>
                                                ) : null}
                                            </>
                                        )}
                                    </Pressable>
                                );
                            })}
                        </ScrollView>
                    </View>
                ) : (
                    <EditorToolbar
                        activeState={activeState}
                        historyState={historyState}
                        toolbarItems={toolbarItems}
                        theme={theme?.toolbar}
                        showTopBorder={inlineToolbarShowTopBorder}
                        preserveEditorFocus={false}
                        onToggleMark={(mark) =>
                            runAndApply(() => bridgeRef.current?.toggleMark(mark) ?? null, {
                                skipNativeApplyIfContentUnchanged: true,
                            })
                        }
                        onToggleListType={(listType: EditorToolbarListType) =>
                            runAndApply(() => bridgeRef.current?.toggleList(listType) ?? null)
                        }
                        onToggleHeading={(level: EditorToolbarHeadingLevel) =>
                            runAndApply(() => bridgeRef.current?.toggleHeading(level) ?? null)
                        }
                        onToggleBlockquote={() =>
                            runAndApply(() => bridgeRef.current?.toggleBlockquote() ?? null)
                        }
                        onInsertNodeType={(nodeType) =>
                            runAndApply(() => bridgeRef.current?.insertNode(nodeType) ?? null)
                        }
                        onRunCommand={(command: EditorToolbarCommand) => {
                            switch (command) {
                                case 'indentList':
                                    runAndApply(() => bridgeRef.current?.indentListItem() ?? null);
                                    break;
                                case 'outdentList':
                                    runAndApply(() => bridgeRef.current?.outdentListItem() ?? null);
                                    break;
                                case 'undo':
                                    runAndApply(() => bridgeRef.current?.undo() ?? null);
                                    break;
                                case 'redo':
                                    runAndApply(() => bridgeRef.current?.redo() ?? null);
                                    break;
                            }
                        }}
                        onRequestLink={openLinkRequest}
                        onRequestImage={openImageRequest}
                        onToolbarAction={onToolbarAction}
                        onToggleBold={() =>
                            runAndApply(() => bridgeRef.current?.toggleMark('bold') ?? null, {
                                skipNativeApplyIfContentUnchanged: true,
                            })
                        }
                        onToggleItalic={() =>
                            runAndApply(() => bridgeRef.current?.toggleMark('italic') ?? null, {
                                skipNativeApplyIfContentUnchanged: true,
                            })
                        }
                        onToggleUnderline={() =>
                            runAndApply(() => bridgeRef.current?.toggleMark('underline') ?? null, {
                                skipNativeApplyIfContentUnchanged: true,
                            })
                        }
                        onToggleStrike={() =>
                            runAndApply(() => bridgeRef.current?.toggleMark('strike') ?? null, {
                                skipNativeApplyIfContentUnchanged: true,
                            })
                        }
                        onToggleBulletList={() =>
                            runAndApply(() => bridgeRef.current?.toggleList('bulletList') ?? null)
                        }
                        onToggleOrderedList={() =>
                            runAndApply(() => bridgeRef.current?.toggleList('orderedList') ?? null)
                        }
                        onIndentList={() =>
                            runAndApply(() => bridgeRef.current?.indentListItem() ?? null)
                        }
                        onOutdentList={() =>
                            runAndApply(() => bridgeRef.current?.outdentListItem() ?? null)
                        }
                        onInsertHorizontalRule={() =>
                            runAndApply(
                                () => bridgeRef.current?.insertNode('horizontalRule') ?? null
                            )
                        }
                        onInsertLineBreak={() =>
                            runAndApply(() => bridgeRef.current?.insertNode('hardBreak') ?? null)
                        }
                        onUndo={() => runAndApply(() => bridgeRef.current?.undo() ?? null)}
                        onRedo={() => runAndApply(() => bridgeRef.current?.redo() ?? null)}
                    />
                )}
            </View>
        );

        return (
            <View style={[styles.container, containerStyle]}>
                <NativeEditorView
                    key={DEV_NATIVE_VIEW_KEY}
                    ref={nativeViewRef}
                    style={nativeViewStyle}
                    editorId={editorInstanceId}
                    placeholder={placeholder}
                    editable={editable}
                    autoFocus={autoFocus}
                    autoCapitalize={autoCapitalize}
                    autoCorrect={autoCorrect}
                    keyboardType={keyboardType}
                    showToolbar={showToolbar}
                    toolbarPlacement={toolbarPlacement}
                    heightBehavior={heightBehavior}
                    allowImageResizing={allowImageResizing}
                    themeJson={themeJson}
                    addonsJson={addonsJson}
                    toolbarItemsJson={toolbarItemsJson}
                    remoteSelectionsJson={remoteSelectionsJson}
                    toolbarFrameJson={toolbarFrameJson}
                    editorUpdateJson={pendingNativeUpdate.json}
                    editorUpdateEditorId={pendingNativeUpdate.editorId}
                    editorUpdateRevision={pendingNativeUpdate.revision}
                    editorResetUpdateJson={pendingNativeResetUpdate.json}
                    editorResetUpdateEditorId={pendingNativeResetUpdate.editorId}
                    editorResetUpdateRevision={pendingNativeResetUpdate.revision}
                    onEditorUpdate={handleUpdate}
                    onSelectionChange={handleSelectionChange}
                    onFocusChange={handleFocusChange}
                    onContentHeightChange={handleContentHeightChange}
                    {...(Platform.OS === 'android' ? { onEditorReady: handleEditorReady } : {})}
                    onToolbarAction={handleToolbarAction}
                    onAddonEvent={handleAddonEvent}
                />
                {shouldRenderJsToolbar && jsToolbar}
            </View>
        );
    }
);

const styles = StyleSheet.create({
    container: {
        position: 'relative',
    },
    inlineToolbar: {
        borderWidth: StyleSheet.hairlineWidth,
        borderColor: INLINE_TOOLBAR_BORDER_COLOR,
        overflow: 'hidden',
    },
    inlineMentionSuggestionsContainer: {
        overflow: 'hidden',
    },
    inlineMentionSuggestionsContent: {
        paddingHorizontal: 12,
        paddingVertical: 8,
        alignItems: 'center',
    },
    inlineMentionSuggestion: {
        minWidth: 88,
        minHeight: 40,
        marginRight: 8,
        paddingHorizontal: 12,
        paddingVertical: 8,
        justifyContent: 'center',
    },
    inlineMentionSuggestionTitle: {
        fontSize: 14,
        fontWeight: '600',
    },
    inlineMentionSuggestionSubtitle: {
        marginTop: 1,
        fontSize: 12,
    },
});
