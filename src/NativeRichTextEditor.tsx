import React, {
    forwardRef,
    useEffect,
    useCallback,
    useImperativeHandle,
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
    type DocumentJSON,
    type EditorUpdate,
    type HistoryState,
    type RenderElement,
    type Selection,
} from './NativeEditorBridge';
import {
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    EditorToolbar,
    type EditorToolbarCommand,
    type EditorToolbarGroupChildItem,
    type EditorToolbarHeadingLevel,
    type EditorToolbarIcon,
    type EditorToolbarItem,
    type EditorToolbarListType,
} from './EditorToolbar';
import { serializeEditorTheme, type EditorTheme } from './EditorTheme';
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
    applyEditorUpdate: (updateJson: string) => void | Promise<void>;
}

interface NativeEditorViewProps {
    style?: StyleProp<ViewStyle>;
    editorId: number;
    placeholder?: string;
    editable: boolean;
    autoFocus: boolean;
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
    editorUpdateRevision?: number;
    onEditorUpdate: (event: NativeSyntheticEvent<NativeUpdateEvent>) => void;
    onSelectionChange: (event: NativeSyntheticEvent<NativeSelectionEvent>) => void;
    onFocusChange: (event: NativeSyntheticEvent<NativeFocusEvent>) => void;
    onContentHeightChange: (event: NativeSyntheticEvent<NativeContentHeightEvent>) => void;
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

function resolveMentionSuggestionLabel(suggestion: MentionSuggestion, trigger: string): string {
    return suggestion.label?.trim() || `${trigger}${suggestion.title}`;
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
                  const label = resolveMentionSuggestionLabel(suggestion, trigger);
                  return (
                      suggestion.title.toLowerCase().includes(normalizedQuery) ||
                      label.toLowerCase().includes(normalizedQuery) ||
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
        attrs.label = resolveMentionSuggestionLabel(suggestion, trigger);
    }
    if (!('mentionSuggestionChar' in attrs)) {
        attrs.mentionSuggestionChar = trigger;
    }
    return attrs;
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

interface NativeUpdateEvent {
    updateJson: string;
}

interface NativeSelectionEvent {
    anchor: number;
    head: number;
    stateJson?: string;
}

interface NativeFocusEvent {
    isFocused: boolean;
}

interface NativeContentHeightEvent {
    contentHeight: number;
}

interface NativeToolbarActionEvent {
    key: string;
}

interface NativeAddonEvent {
    eventJson: string;
}

function computeRenderedTextLength(elements: RenderElement[]): number {
    let len = 0;
    let blockCount = 0;
    for (const el of elements) {
        if (el.type === 'blockStart' && el.listContext) {
            len += el.listContext.ordered ? `${el.listContext.index}. `.length : '• '.length;
        } else if (el.type === 'textRun' && el.text) {
            len += el.text.length;
        } else if (
            el.type === 'voidInline' ||
            el.type === 'voidBlock' ||
            el.type === 'opaqueInlineAtom' ||
            el.type === 'opaqueBlockAtom'
        ) {
            if (el.type === 'opaqueInlineAtom' || el.type === 'opaqueBlockAtom') {
                const visibleText =
                    el.nodeType === 'mention' ? (el.label ?? '?') : `[${el.label ?? '?'}]`;
                len += visibleText.length;
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
    /** Toggle a list type (bulletList or orderedList). */
    toggleList(listType: 'bulletList' | 'orderedList'): void;
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
    /** Get the current HTML content. */
    getContent(): string;
    /** Get the current content as ProseMirror JSON. */
    getContentJson(): DocumentJSON;
    /** Get the plain text content (no markup). */
    getTextContent(): string;
    /** Undo the last operation. */
    undo(): void;
    /** Redo the last undone operation. */
    redo(): void;
    /** Check if undo is available. */
    canUndo(): boolean;
    /** Check if redo is available. */
    canRedo(): boolean;
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
}

export const NativeRichTextEditor = forwardRef<NativeRichTextEditorRef, NativeRichTextEditorProps>(
    function NativeRichTextEditor(
        {
            initialContent,
            initialJSON,
            value,
            valueJSON,
            valueJSONRevision,
            schema,
            placeholder,
            editable = true,
            maxLength,
            autoFocus = false,
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
        const [isFocused, setIsFocused] = useState(false);
        const [toolbarFrameJson, setToolbarFrameJson] = useState<string | undefined>(undefined);
        const [pendingNativeUpdate, setPendingNativeUpdate] = useState<{
            json?: string;
            revision: number;
        }>({
            json: undefined,
            revision: 0,
        });
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
        const toolbarRef = useRef<View | null>(null);
        const mentionQueryEventRef = useRef<MentionQueryChangeEvent | null>(null);
        mentionQueryEventRef.current = mentionQueryEvent;
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

        const syncStateFromUpdate = useCallback((update: EditorUpdate | null) => {
            if (!update) return;
            setActiveState(update.activeState);
            setHistoryState(update.historyState);
            selectionRef.current = update.selection;
            renderedTextLengthRef.current = computeRenderedTextLength(update.renderElements);
            if (typeof update.documentVersion === 'number') {
                documentVersionRef.current = update.documentVersion;
            }
        }, []);

        const syncSelectionStateFromUpdate = useCallback((update: EditorUpdate | null) => {
            if (!update) return;
            setActiveState(update.activeState);
            setHistoryState(update.historyState);
            selectionRef.current = update.selection;
            if (typeof update.documentVersion === 'number') {
                documentVersionRef.current = update.documentVersion;
            }
        }, []);

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

        const applyUpdateToNativeView = useCallback(
            (
                update: EditorUpdate,
                previousDocumentVersion: number | null,
                skipNativeApplyIfContentUnchanged = false
            ) => {
                const contentChanged = didContentChange(previousDocumentVersion, update);

                if (!skipNativeApplyIfContentUnchanged || contentChanged) {
                    const updateJson = JSON.stringify(update);
                    if (Platform.OS === 'android') {
                        setPendingNativeUpdate((current) => ({
                            json: updateJson,
                            revision: current.revision + 1,
                        }));
                    } else {
                        try {
                            const applyResult =
                                nativeViewRef.current?.applyEditorUpdate(updateJson);
                            if (isPromiseLike(applyResult)) {
                                void applyResult.catch(() => {
                                    // The native view may already be torn down during navigation.
                                });
                            }
                        } catch {
                            // The native view may already be torn down during navigation.
                        }
                    }
                }

                return contentChanged;
            },
            []
        );

        const maybeApplyAutoDetectedLink = useCallback(
            (update: EditorUpdate | null, previousDocumentVersion: number | null) => {
                if (
                    !autoDetectLinks ||
                    !update ||
                    !didContentChange(previousDocumentVersion, update) ||
                    !bridgeRef.current ||
                    bridgeRef.current.isDestroyed ||
                    !update.activeState.allowedMarks.includes('link') ||
                    update.selection.type !== 'text' ||
                    update.selection.anchor == null ||
                    update.selection.head == null ||
                    update.selection.anchor !== update.selection.head
                ) {
                    return update;
                }

                const cursorDocPos = update.selection.head;
                const candidate = findAutoLinkCandidateInDocument(
                    bridgeRef.current.getJson(),
                    cursorDocPos
                );
                if (!candidate) {
                    return update;
                }

                const scalarFrom = bridgeRef.current.docToScalar(candidate.docFrom);
                const scalarTo = bridgeRef.current.docToScalar(candidate.docTo);
                if (!(scalarTo > scalarFrom)) {
                    return update;
                }

                const autoLinkUpdate = bridgeRef.current.setMarkAtSelectionScalar(
                    scalarFrom,
                    scalarTo,
                    'link',
                    { href: candidate.href }
                );
                if (!autoLinkUpdate) {
                    return update;
                }

                bridgeRef.current.setSelection(update.selection.anchor, update.selection.head);
                const selectionState = bridgeRef.current.getSelectionState();
                if (selectionState) {
                    autoLinkUpdate.selection = selectionState.selection;
                    autoLinkUpdate.activeState = selectionState.activeState;
                    autoLinkUpdate.historyState = selectionState.historyState;
                    if (typeof selectionState.documentVersion === 'number') {
                        autoLinkUpdate.documentVersion = selectionState.documentVersion;
                    }
                } else {
                    autoLinkUpdate.selection = update.selection;
                }

                return autoLinkUpdate;
            },
            [autoDetectLinks]
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
                const previousDocumentVersion = documentVersionRef.current;
                const preservedSelection =
                    options?.preserveLiveTextSelection === true ? selectionRef.current : null;
                let update = mutate();
                if (!update) return null;

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

                if (!options?.suppressContentCallbacks) {
                    emitContentCallbacksForUpdate(update, previousDocumentVersion);
                }

                onSelectionChangeRef.current?.(update.selection);

                return update;
            },
            [
                applyUpdateToNativeView,
                emitContentCallbacksForUpdate,
                maybeApplyAutoDetectedLink,
                syncStateFromUpdate,
            ]
        );

        useEffect(() => {
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
            } else if (serializedInitialJson != null) {
                bridge.setJsonString(serializedInitialJson);
            } else if (initialContent) {
                bridge.setHtml(initialContent);
            }

            syncStateFromUpdate(bridge.getCurrentState());
            setIsReady(true);

            return () => {
                bridge.destroy();
                bridgeRef.current = null;
                nativeViewRef.current = null;
                setEditorInstanceId(0);
                setIsReady(false);
            };
            // eslint-disable-next-line react-hooks/exhaustive-deps
        }, [maxLength, syncStateFromUpdate, allowBase64Images, serializedSchemaJson]);

        useEffect(() => {
            if (value == null) return;
            if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;

            const currentHtml = bridgeRef.current.getHtml();
            if (currentHtml === value) return;

            runAndApply(() => bridgeRef.current!.replaceHtml(value), {
                suppressContentCallbacks: true,
                preserveLiveTextSelection: true,
                skipAutoDetectLinks: true,
            });
        }, [value, runAndApply]);

        useEffect(() => {
            if (serializedValueJson == null || value != null) return;
            if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;

            const currentJson = bridgeRef.current.getJsonString();
            if (currentJson === serializedValueJson) return;

            runAndApply(() => bridgeRef.current!.replaceJsonString(serializedValueJson), {
                suppressContentCallbacks: true,
                preserveLiveTextSelection: true,
                skipAutoDetectLinks: true,
            });
        }, [serializedValueJson, value, runAndApply]);

        const updateToolbarFrame = useCallback(() => {
            const toolbar = toolbarRef.current;
            if (!toolbar) {
                setToolbarFrameJson(undefined);
                return;
            }

            toolbar.measureInWindow((x, y, width, height) => {
                if (width <= 0 || height <= 0) {
                    setToolbarFrameJson(undefined);
                    return;
                }

                const nextJson = JSON.stringify({ x, y, width, height });
                setToolbarFrameJson((prev) => (prev === nextJson ? prev : nextJson));
            });
        }, []);

        useEffect(() => {
            if (!(showToolbar && toolbarPlacement === 'inline' && isFocused && editable)) {
                setToolbarFrameJson(undefined);
                return;
            }

            const frame = requestAnimationFrame(() => {
                updateToolbarFrame();
            });
            return () => cancelAnimationFrame(frame);
        }, [editable, isFocused, showToolbar, toolbarPlacement, updateToolbarFrame]);

        useEffect(() => {
            if (heightBehavior !== 'autoGrow') {
                setAutoGrowHeight(null);
            }
        }, [heightBehavior]);

        const handleUpdate = useCallback(
            (event: NativeSyntheticEvent<NativeUpdateEvent>) => {
                if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;

                try {
                    const previousDocumentVersion = documentVersionRef.current;
                    const nativeUpdate = bridgeRef.current.parseUpdateJson(
                        event.nativeEvent.updateJson
                    );
                    if (!nativeUpdate) return;
                    const update = maybeApplyAutoDetectedLink(
                        nativeUpdate,
                        previousDocumentVersion
                    );
                    if (!update) return;
                    if (update !== nativeUpdate) {
                        applyUpdateToNativeView(update, previousDocumentVersion);
                    }
                    syncStateFromUpdate(update);

                    onActiveStateChangeRef.current?.(update.activeState);
                    onHistoryStateChangeRef.current?.(update.historyState);

                    emitContentCallbacksForUpdate(update, previousDocumentVersion);

                    onSelectionChangeRef.current?.(update.selection);
                } catch {
                    // Invalid JSON from native — skip
                }
            },
            [
                applyUpdateToNativeView,
                emitContentCallbacksForUpdate,
                maybeApplyAutoDetectedLink,
                syncStateFromUpdate,
            ]
        );

        const handleSelectionChange = useCallback(
            (event: NativeSyntheticEvent<NativeSelectionEvent>) => {
                if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;

                const { anchor, head, stateJson } = event.nativeEvent;
                let selection: Selection;

                if (
                    anchor === 0 &&
                    head >= renderedTextLengthRef.current &&
                    renderedTextLengthRef.current > 0
                ) {
                    selection = { type: 'all' };
                } else {
                    selection = { type: 'text', anchor, head };
                }

                bridgeRef.current.updateSelectionFromNative(anchor, head);
                let currentState: EditorUpdate | null = null;
                if (typeof stateJson === 'string' && stateJson.length > 0) {
                    try {
                        currentState = bridgeRef.current.parseUpdateJson(stateJson);
                    } catch {
                        currentState = bridgeRef.current.getSelectionState();
                    }
                } else {
                    currentState = bridgeRef.current.getSelectionState();
                }
                syncSelectionStateFromUpdate(currentState);
                const nextSelection =
                    selection.type === 'all' ? selection : (currentState?.selection ?? selection);
                selectionRef.current = nextSelection;
                if (currentState) {
                    onActiveStateChangeRef.current?.(currentState.activeState);
                    onHistoryStateChangeRef.current?.(currentState.historyState);
                }
                onSelectionChangeRef.current?.(nextSelection);
            },
            [syncSelectionStateFromUpdate]
        );

        const handleFocusChange = useCallback((event: NativeSyntheticEvent<NativeFocusEvent>) => {
            const { isFocused: focused } = event.nativeEvent;
            setIsFocused(focused);
            if (!focused) {
                setMentionQueryEvent(null);
            }
            if (focused) {
                onFocusRef.current?.();
            } else {
                onBlurRef.current?.();
            }
        }, []);

        useEffect(() => {
            if (addons?.mentions != null) {
                return;
            }
            setMentionQueryEvent(null);
        }, [addons?.mentions]);

        const handleContentHeightChange = useCallback(
            (event: NativeSyntheticEvent<NativeContentHeightEvent>) => {
                if (heightBehavior !== 'autoGrow') return;
                const density = Platform.OS === 'android' ? PixelRatio.get() : 1;
                const nextHeight = Math.ceil(event.nativeEvent.contentHeight / density);
                if (!(nextHeight > 0)) return;
                setAutoGrowHeight((prev) => (prev === nextHeight ? prev : nextHeight));
            },
            [autoGrowHeight, heightBehavior]
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

            onRequestLink?.({
                href: currentLinkHref,
                isActive: activeState.marks.link === true,
                selection: requestSelection,
                setLink: (href: string) => {
                    const trimmedHref = href.trim();
                    if (!trimmedHref) return;
                    runAndApply(
                        () => {
                            restoreSelection(requestSelection);
                            return (
                                bridgeRef.current?.setMark('link', {
                                    href: trimmedHref,
                                }) ?? null
                            );
                        },
                        { skipNativeApplyIfContentUnchanged: true }
                    );
                },
                unsetLink: () => {
                    runAndApply(
                        () => {
                            restoreSelection(requestSelection);
                            return bridgeRef.current?.unsetMark('link') ?? null;
                        },
                        { skipNativeApplyIfContentUnchanged: true }
                    );
                },
            });
        }, [activeState.marks.link, currentLinkHref, onRequestLink, restoreSelection, runAndApply]);

        const openImageRequest = useCallback(() => {
            const requestSelection = selectionRef.current;

            onRequestImage?.({
                selection: requestSelection,
                allowBase64: allowBase64Images,
                insertImage: (src: string, attrs?: Omit<ImageNodeAttributes, 'src'>) =>
                    insertImage(src, attrs, requestSelection),
            });
        }, [allowBase64Images, insertImage, onRequestImage]);

        const handleToolbarAction = useCallback(
            (event: NativeSyntheticEvent<NativeToolbarActionEvent>) => {
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
            [onToolbarAction, openImageRequest, openLinkRequest]
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

                const attrs = resolveMentionSelectionAttrs({
                    trigger: mentionQuery.trigger,
                    suggestion,
                    attrs: resolveMentionSuggestionAttrs(suggestion, mentionQuery.trigger),
                    range: mentionQuery.range,
                });

                const update = runAndApply(
                    () =>
                        bridgeRef.current?.insertContentJsonAtSelectionScalar(
                            mentionQuery.range.anchor,
                            mentionQuery.range.head,
                            buildMentionFragmentJson(attrs)
                        ) ?? null
                );

                if (update) {
                    setMentionQueryEvent(null);
                    mentions.onSelect?.({
                        trigger: mentionQuery.trigger,
                        suggestion,
                        attrs,
                    });
                }
            },
            [resolveMentionSelectionAttrs, runAndApply]
        );

        const handleAddonEvent = useCallback(
            (event: NativeSyntheticEvent<NativeAddonEvent>) => {
                let parsed: EditorAddonEvent | null = null;
                try {
                    parsed = JSON.parse(event.nativeEvent.eventJson) as EditorAddonEvent;
                } catch {
                    return;
                }
                if (!parsed) return;

                if (parsed.type === 'mentionsQueryChange') {
                    const nextEvent: MentionQueryChangeEvent = {
                        query: parsed.query,
                        trigger: parsed.trigger,
                        range: parsed.range,
                        isActive: parsed.isActive,
                    };
                    setMentionQueryEvent(parsed.isActive ? nextEvent : null);
                    addonsRef.current?.mentions?.onQueryChange?.({
                        query: nextEvent.query,
                        trigger: nextEvent.trigger,
                        range: nextEvent.range,
                        isActive: nextEvent.isActive,
                    });
                    return;
                }

                if (parsed.type === 'mentionsSelectRequest') {
                    const suggestion = mentionSuggestionsByKeyRef.current.get(parsed.suggestionKey);
                    if (!suggestion || !bridgeRef.current || bridgeRef.current.isDestroyed) return;

                    const selectionEvent: MentionSelectionAttrsEvent = {
                        trigger: parsed.trigger,
                        suggestion,
                        attrs: parsed.attrs,
                        range: parsed.range,
                    };
                    const finalAttrs = resolveMentionSelectionAttrs(selectionEvent);

                    const update = runAndApply(
                        () =>
                            bridgeRef.current?.insertContentJsonAtSelectionScalar(
                                parsed.range.anchor,
                                parsed.range.head,
                                buildMentionFragmentJson(finalAttrs)
                            ) ?? null
                    );

                    if (update) {
                        addonsRef.current?.mentions?.onSelect?.({
                            trigger: parsed.trigger,
                            suggestion,
                            attrs: finalAttrs,
                        });
                    }
                    return;
                }

                if (parsed.type === 'mentionsSelect') {
                    const suggestion = mentionSuggestionsByKeyRef.current.get(parsed.suggestionKey);
                    if (!suggestion) return;
                    addonsRef.current?.mentions?.onSelect?.({
                        trigger: parsed.trigger,
                        suggestion,
                        attrs: parsed.attrs,
                    });
                }
            },
            [resolveMentionSelectionAttrs, runAndApply]
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
                toggleList(listType: 'bulletList' | 'orderedList') {
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
                    runAndApply(() => bridgeRef.current?.replaceHtml(html) ?? null);
                },
                setContentJson(doc: DocumentJSON) {
                    runAndApply(() => bridgeRef.current?.replaceJson(doc) ?? null);
                },
                getContent(): string {
                    if (!bridgeRef.current || bridgeRef.current.isDestroyed) return '';
                    return bridgeRef.current.getHtml();
                },
                getContentJson(): DocumentJSON {
                    if (!bridgeRef.current || bridgeRef.current.isDestroyed) return {};
                    return bridgeRef.current.getJson();
                },
                getTextContent(): string {
                    if (!bridgeRef.current || bridgeRef.current.isDestroyed) return '';
                    return bridgeRef.current.getHtml().replace(/<[^>]+>/g, '');
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
            [insertImage, runAndApply]
        );

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
        const inlineToolbarMentionTheme = theme?.mentions ?? addons?.mentions?.theme;
        const inlineToolbarContentTopBorderStyle = inlineToolbarShowTopBorder
            ? {
                  borderTopWidth: theme?.toolbar?.borderWidth ?? StyleSheet.hairlineWidth,
                  borderTopColor: theme?.toolbar?.borderColor ?? INLINE_TOOLBAR_BORDER_COLOR,
              }
            : null;
        const inlineMentionSuggestions =
            toolbarPlacement === 'inline' &&
            isFocused &&
            mentionQueryEvent != null &&
            addons?.mentions != null
                ? filterMentionSuggestions(
                      addons.mentions.suggestions ?? [],
                      mentionQueryEvent.query,
                      mentionQueryEvent.trigger || resolveMentionTrigger(addons)
                  )
                : [];
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
                                const label = resolveMentionSuggestionLabel(
                                    suggestion,
                                    mentionQueryEvent?.trigger ?? resolveMentionTrigger(addons)
                                );

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
                                                    ? (inlineToolbarMentionTheme?.optionHighlightedBackgroundColor ??
                                                      'rgba(0, 122, 255, 0.12)')
                                                    : (inlineToolbarMentionTheme?.backgroundColor ??
                                                      '#F2F2F7'),
                                                borderColor:
                                                    inlineToolbarMentionTheme?.borderColor ??
                                                    'transparent',
                                                borderWidth:
                                                    inlineToolbarMentionTheme?.borderWidth ?? 0,
                                                borderRadius:
                                                    inlineToolbarMentionTheme?.borderRadius ?? 12,
                                            },
                                        ]}>
                                        {({ pressed }) => (
                                            <>
                                                <Text
                                                    numberOfLines={1}
                                                    style={[
                                                        styles.inlineMentionSuggestionTitle,
                                                        {
                                                            color: pressed
                                                                ? (inlineToolbarMentionTheme?.optionHighlightedTextColor ??
                                                                  inlineToolbarMentionTheme?.optionTextColor ??
                                                                  '#000000')
                                                                : (inlineToolbarMentionTheme?.optionTextColor ??
                                                                  inlineToolbarMentionTheme?.textColor ??
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
                                                                    inlineToolbarMentionTheme?.optionSecondaryTextColor ??
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
                    showToolbar={showToolbar}
                    toolbarPlacement={toolbarPlacement}
                    heightBehavior={heightBehavior}
                    allowImageResizing={allowImageResizing}
                    themeJson={themeJson}
                    addonsJson={addonsJson}
                    toolbarItemsJson={toolbarItemsJson}
                    remoteSelectionsJson={remoteSelectionsJson}
                    toolbarFrameJson={
                        toolbarPlacement === 'inline' && isFocused ? toolbarFrameJson : undefined
                    }
                    editorUpdateJson={pendingNativeUpdate.json}
                    editorUpdateRevision={pendingNativeUpdate.revision}
                    onEditorUpdate={handleUpdate}
                    onSelectionChange={handleSelectionChange}
                    onFocusChange={handleFocusChange}
                    onContentHeightChange={handleContentHeightChange}
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
