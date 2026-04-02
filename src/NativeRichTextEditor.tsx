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
    StyleSheet,
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
    parseEditorUpdateJson,
} from './NativeEditorBridge';
import {
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    EditorToolbar,
    type EditorToolbarCommand,
    type EditorToolbarIcon,
    type EditorToolbarItem,
    type EditorToolbarListType,
} from './EditorToolbar';
import { serializeEditorTheme, type EditorTheme } from './EditorTheme';
import {
    serializeEditorAddons,
    type EditorAddonEvent,
    type EditorAddons,
    type MentionSuggestion,
    withMentionsSchema,
} from './addons';
import { tiptapSchema, type SchemaDefinition } from './schemas';

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

function isPromiseLike(value: unknown): value is Promise<unknown> {
    return (
        value != null &&
        typeof value === 'object' &&
        'then' in value &&
        typeof (value as Promise<unknown>).then === 'function'
    );
}

interface NativeUpdateEvent {
    updateJson: string;
}

interface NativeSelectionEvent {
    anchor: number;
    head: number;
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
    return JSON.stringify(remoteSelections);
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

export interface NativeRichTextEditorProps {
    /** Initial content as HTML (uncontrolled mode). */
    initialContent?: string;
    /** Initial content as ProseMirror JSON (uncontrolled mode). */
    initialJSON?: DocumentJSON;
    /** Controlled HTML content. External changes are diffed and applied. */
    value?: string;
    /** Controlled ProseMirror JSON content. Ignored if value is set. */
    valueJSON?: DocumentJSON;
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
    /** Called when content changes with the current HTML. */
    onContentChange?: (html: string) => void;
    /** Called when content changes with the current ProseMirror JSON. */
    onContentChangeJSON?: (json: DocumentJSON) => void;
    /** Called when selection changes. */
    onSelectionChange?: (selection: Selection) => void;
    /** Called when active formatting state changes. */
    onActiveStateChange?: (state: ActiveState) => void;
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
    /** Toggle a list type (bulletList or orderedList). */
    toggleList(listType: 'bulletList' | 'orderedList'): void;
    /** Indent the current list item. */
    indentListItem(): void;
    /** Outdent the current list item. */
    outdentListItem(): void;
    /** Insert a void node (e.g. 'horizontalRule'). */
    insertNode(nodeType: string): void;
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
}

export const NativeRichTextEditor = forwardRef<NativeRichTextEditorRef, NativeRichTextEditorProps>(
    function NativeRichTextEditor(
        {
            initialContent,
            initialJSON,
            value,
            valueJSON,
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
            onContentChange,
            onContentChangeJSON,
            onSelectionChange,
            onActiveStateChange,
            onFocus,
            onBlur,
            style,
            containerStyle,
            theme,
            addons,
            remoteSelections,
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

        // Selection and rendered text length refs (non-rendering state)
        const selectionRef = useRef<Selection>({ type: 'text', anchor: 0, head: 0 });
        const renderedTextLengthRef = useRef(0);
        const toolbarRef = useRef<View | null>(null);

        // Stable callback refs to avoid re-renders
        const onContentChangeRef = useRef(onContentChange);
        onContentChangeRef.current = onContentChange;
        const onContentChangeJSONRef = useRef(onContentChangeJSON);
        onContentChangeJSONRef.current = onContentChangeJSON;
        const onSelectionChangeRef = useRef(onSelectionChange);
        onSelectionChangeRef.current = onSelectionChange;
        const onActiveStateChangeRef = useRef(onActiveStateChange);
        onActiveStateChangeRef.current = onActiveStateChange;
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

        const syncStateFromUpdate = useCallback((update: EditorUpdate | null) => {
            if (!update) return;
            setActiveState(update.activeState);
            setHistoryState(update.historyState);
            selectionRef.current = update.selection;
            renderedTextLengthRef.current = computeRenderedTextLength(update.renderElements);
        }, []);

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
                const preservedSelection =
                    options?.preserveLiveTextSelection === true ? selectionRef.current : null;
                const shouldCheckForNoopNativeApply =
                    options?.skipNativeApplyIfContentUnchanged === true &&
                    bridgeRef.current != null &&
                    !bridgeRef.current.isDestroyed;
                const htmlBefore = shouldCheckForNoopNativeApply
                    ? bridgeRef.current!.getHtml()
                    : null;
                const update = mutate();
                if (!update) return null;

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

                const htmlAfter = shouldCheckForNoopNativeApply
                    ? bridgeRef.current!.getHtml()
                    : null;
                if (!shouldCheckForNoopNativeApply || htmlBefore !== htmlAfter) {
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

                syncStateFromUpdate(update);

                onActiveStateChangeRef.current?.(update.activeState);

                if (!options?.suppressContentCallbacks) {
                    if (onContentChangeRef.current && bridgeRef.current) {
                        onContentChangeRef.current(bridgeRef.current.getHtml());
                    }
                    if (onContentChangeJSONRef.current && bridgeRef.current) {
                        onContentChangeJSONRef.current(bridgeRef.current.getJson());
                    }
                }

                onSelectionChangeRef.current?.(update.selection);

                return update;
            },
            [syncStateFromUpdate]
        );

        useEffect(() => {
            const effectiveSchema =
                addonsRef.current?.mentions != null
                    ? withMentionsSchema(schema ?? tiptapSchema)
                    : schema;
            const schemaJson = effectiveSchema ? JSON.stringify(effectiveSchema) : undefined;
            const bridge = NativeEditorBridge.create(
                maxLength != null || schemaJson ? { maxLength, schemaJson } : undefined
            );
            bridgeRef.current = bridge;
            setEditorInstanceId(bridge.editorId);

            // Four-way content initialization: value > valueJSON > initialJSON > initialContent
            if (value != null) {
                bridge.setHtml(value);
            } else if (valueJSON != null) {
                bridge.setJson(valueJSON);
            } else if (initialJSON) {
                bridge.setJson(initialJSON);
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
        }, [schema, maxLength, syncStateFromUpdate, Boolean(addons?.mentions)]);

        useEffect(() => {
            if (value == null) return;
            if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;

            const currentHtml = bridgeRef.current.getHtml();
            if (currentHtml === value) return;

            runAndApply(() => bridgeRef.current!.replaceHtml(value), {
                suppressContentCallbacks: true,
                preserveLiveTextSelection: true,
            });
        }, [value, runAndApply]);

        useEffect(() => {
            if (valueJSON == null || value != null) return;
            if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;

            // No-op if JSON content is identical (avoids churning undo history)
            const currentJson = bridgeRef.current.getJson();
            if (JSON.stringify(currentJson) === JSON.stringify(valueJSON)) return;

            runAndApply(() => bridgeRef.current!.replaceJson(valueJSON), {
                suppressContentCallbacks: true,
                preserveLiveTextSelection: true,
            });
        }, [valueJSON, value, runAndApply]);

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
                    const update = parseEditorUpdateJson(event.nativeEvent.updateJson);
                    if (!update) return;
                    syncStateFromUpdate(update);

                    onActiveStateChangeRef.current?.(update.activeState);

                    if (onContentChangeRef.current) {
                        onContentChangeRef.current(bridgeRef.current.getHtml());
                    }
                    if (onContentChangeJSONRef.current) {
                        onContentChangeJSONRef.current(bridgeRef.current.getJson());
                    }

                    onSelectionChangeRef.current?.(update.selection);
                } catch {
                    // Invalid JSON from native — skip
                }
            },
            [syncStateFromUpdate]
        );

        const handleSelectionChange = useCallback(
            (event: NativeSyntheticEvent<NativeSelectionEvent>) => {
                if (!bridgeRef.current || bridgeRef.current.isDestroyed) return;

                const { anchor, head } = event.nativeEvent;
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
                const currentState = bridgeRef.current.getCurrentState();
                syncStateFromUpdate(currentState);
                const nextSelection =
                    selection.type === 'all' ? selection : (currentState?.selection ?? selection);
                selectionRef.current = nextSelection;
                if (currentState) {
                    onActiveStateChangeRef.current?.(currentState.activeState);
                }
                onSelectionChangeRef.current?.(nextSelection);
            },
            [syncStateFromUpdate]
        );

        const handleFocusChange = useCallback((event: NativeSyntheticEvent<NativeFocusEvent>) => {
            const { isFocused: focused } = event.nativeEvent;
            setIsFocused(focused);
            if (focused) {
                onFocusRef.current?.();
            } else {
                onBlurRef.current?.();
            }
        }, []);

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

        const openLinkRequest = useCallback(() => {
            const requestSelection = selectionRef.current;
            const restoreCapturedSelection = () => {
                if (requestSelection.type === 'text') {
                    const { anchor, head } = requestSelection;
                    if (anchor == null || head == null) {
                        return;
                    }
                    bridgeRef.current?.setSelection(anchor, head);
                    return;
                }

                if (requestSelection.type === 'node') {
                    const { pos } = requestSelection;
                    if (pos == null) {
                        return;
                    }
                    bridgeRef.current?.setSelection(pos, pos);
                }
            };

            onRequestLink?.({
                href: currentLinkHref,
                isActive: activeState.marks.link === true,
                selection: requestSelection,
                setLink: (href: string) => {
                    const trimmedHref = href.trim();
                    if (!trimmedHref) return;
                    runAndApply(
                        () => {
                            restoreCapturedSelection();
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
                            restoreCapturedSelection();
                            return bridgeRef.current?.unsetMark('link') ?? null;
                        },
                        { skipNativeApplyIfContentUnchanged: true }
                    );
                },
            });
        }, [activeState.marks.link, currentLinkHref, onRequestLink, runAndApply]);

        const handleToolbarAction = useCallback(
            (event: NativeSyntheticEvent<NativeToolbarActionEvent>) => {
                if (event.nativeEvent.key === LINK_TOOLBAR_ACTION_KEY) {
                    openLinkRequest();
                    return;
                }
                onToolbarAction?.(event.nativeEvent.key);
            },
            [onToolbarAction, openLinkRequest]
        );

        const handleAddonEvent = useCallback((event: NativeSyntheticEvent<NativeAddonEvent>) => {
            let parsed: EditorAddonEvent | null = null;
            try {
                parsed = JSON.parse(event.nativeEvent.eventJson) as EditorAddonEvent;
            } catch {
                return;
            }
            if (!parsed) return;

            if (parsed.type === 'mentionsQueryChange') {
                addonsRef.current?.mentions?.onQueryChange?.({
                    query: parsed.query,
                    trigger: parsed.trigger,
                    range: parsed.range,
                    isActive: parsed.isActive,
                });
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
        }, []);

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
            [runAndApply]
        );

        if (!isReady) return null;

        const toolbarItemsForNative = toolbarItems.map((item) => {
            if (item.type !== 'link') {
                return item;
            }
            return {
                type: 'action' as const,
                key: LINK_TOOLBAR_ACTION_KEY,
                label: item.label,
                icon: item.icon as EditorToolbarIcon,
                isActive: activeState.marks.link === true,
                isDisabled:
                    !editable || !onRequestLink || !activeState.allowedMarks.includes('link'),
            };
        });
        const themeJson = serializeEditorTheme(theme);
        const addonsJson = serializeEditorAddons(addons);
        const toolbarItemsJson = JSON.stringify(toolbarItemsForNative);
        const remoteSelectionsJson = serializeRemoteSelections(remoteSelections);
        const usesNativeKeyboardToolbar =
            toolbarPlacement === 'keyboard' && (Platform.OS === 'ios' || Platform.OS === 'android');
        const shouldRenderJsToolbar = !usesNativeKeyboardToolbar && showToolbar && editable;
        const inlineToolbarChrome = {
            backgroundColor: theme?.toolbar?.backgroundColor,
            borderColor: theme?.toolbar?.borderColor,
            borderWidth: theme?.toolbar?.borderWidth,
            borderRadius: theme?.toolbar?.borderRadius,
        };
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
                <EditorToolbar
                    activeState={activeState}
                    historyState={historyState}
                    toolbarItems={toolbarItems}
                    theme={theme?.toolbar}
                    showTopBorder={false}
                    onToggleMark={(mark) =>
                        runAndApply(() => bridgeRef.current?.toggleMark(mark) ?? null, {
                            skipNativeApplyIfContentUnchanged: true,
                        })
                    }
                    onToggleListType={(listType: EditorToolbarListType) =>
                        runAndApply(() => bridgeRef.current?.toggleList(listType) ?? null)
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
                        runAndApply(() => bridgeRef.current?.insertNode('horizontalRule') ?? null)
                    }
                    onInsertLineBreak={() =>
                        runAndApply(() => bridgeRef.current?.insertNode('hardBreak') ?? null)
                    }
                    onUndo={() => runAndApply(() => bridgeRef.current?.undo() ?? null)}
                    onRedo={() => runAndApply(() => bridgeRef.current?.redo() ?? null)}
                />
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
        marginTop: 8,
        borderWidth: StyleSheet.hairlineWidth,
        borderColor: '#E5E5EA',
        overflow: 'hidden',
    },
});
