import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
    FlatList,
    KeyboardAvoidingView,
    Platform,
    ScrollView,
    StyleSheet,
    Text,
    View,
    type NativeScrollEvent,
    type NativeSyntheticEvent,
    type ViewToken,
} from 'react-native';
import { requireNativeModule } from 'expo';
import { StatusBar } from 'expo-status-bar';
import * as ImagePicker from 'expo-image-picker';
import { ImageManipulator, SaveFormat } from 'expo-image-manipulator';
import { SafeAreaProvider, useSafeAreaInsets } from 'react-native-safe-area-context';
import { useHeaderHeight } from '@react-navigation/elements';
import { DefaultTheme, NavigationContainer, type Theme } from '@react-navigation/native';
import {
    createNativeStackNavigator,
    type NativeStackNavigationOptions,
} from '@react-navigation/native-stack';

import {
    createNativeEditorDocumentHandle,
    DEFAULT_EDITOR_IMAGE_LOADING_POLICY,
    DEFAULT_EDITOR_TOOLBAR_ITEMS,
    DEFAULT_EDITOR_RESOURCE_LIMITS,
    NativeRichTextEditor,
    NativeProseViewer,
    tiptapSchema,
    withMentionsSchema,
    type DocumentJSON,
    type EditorAddons,
    type EditorToolbarHeadingLevel,
    type EditorToolbarItem,
    type EditorToolbarTheme,
    type HistoryState,
    type ImageRequestContext,
    type LinkRequestContext,
    type MentionQueryChangeEvent,
    type MentionSelectEvent,
    type NativeRichTextEditorRef,
    type ReadonlyActiveState,
    type Selection,
} from '@apollohg/react-native-prose-editor';
import performanceCorpus from '../scripts/tests/viewer-performance-corpus.json';
import preparedProseBenchmarkConfiguration from '../scripts/tests/prepared-prose-benchmark-config.json';

import {
    buildExampleEditorTheme,
    DEFAULT_EXAMPLE_THEME_PRESET_ID,
    EXAMPLE_THEME_PRESETS,
    type ExampleEditorThemeOverrides,
    type ExampleThemePreset,
    getExampleThemePreset,
} from './themePresets';

import {
    CONTROLLED_CONTENT,
    EVENT_LOG_LIMIT,
    EXAMPLE_MENTION_SUGGESTIONS,
    INITIAL_CONTENT,
    INSERT_HTML_SAMPLE,
    INSERT_TEXT_SAMPLE,
    LINK_MARK_NAME,
    SAMPLE_IMAGE_URL,
    SETTINGS_TABS,
    type EditorCommandId,
    type SettingsTab,
    type ToolbarColorKey,
} from './constants';
import { FONT_SIZE, LINE_HEIGHT, MONO_FONT_FAMILY, RADIUS, SPACE } from './designTokens';
import { sharedStyles } from './sharedStyles';
import type {
    ControlledSettings,
    EditorBehaviorSettings,
    EditorEventEntry,
    ImageSettings,
} from './types';
import {
    BENCHMARK_ROUTE,
    HARNESS_ROUTE,
    ROUTE_TITLES,
    type RootStackParamList,
} from './navigation';

import { ActionButton } from './components/ActionButton';
import { CollapsibleSection } from './components/CollapsibleSection';
import { CommandsPanel } from './components/CommandsPanel';
import { ContentSettingsPanel } from './components/ContentSettingsPanel';
import { EditorSettingsPanel } from './components/EditorSettingsPanel';
import { HeaderActionButton } from './components/HeaderActionButton';
import { ImageSettingsPanel } from './components/ImageSettingsPanel';
import { InputSettingsPanel } from './components/InputSettingsPanel';
import { LinkEditorModal } from './components/LinkEditorModal';
import { ReadoutPanel, type ReadoutPane } from './components/ReadoutPanel';
import { SettingsCard } from './components/SettingsCard';
import { ThemePresetPicker } from './components/ThemePresetPicker';
import { ToolbarItemsEditor } from './components/ToolbarItemsEditor';
import { ToolbarSettingsPanel } from './components/ToolbarSettingsPanel';

const OUTPUT_PANEL_UPDATE_DEBOUNCE_MS = 120;
const SCROLL_COMMAND_NO_MOTION_TIMEOUT_MS = 1_500;
const SCROLL_COMMAND_MIN_OFFSET_DELTA = 4;
const DEFAULT_BASE_FONT_SIZE = 17;
const EVENT_LOG_DETAIL_LIMIT = 240;
const MILLISECONDS_PER_SECOND = 1_000;
const PICKED_IMAGE_COMPRESSION = 0.8;
const EDITOR_MIN_HEIGHT = 200;
/** Only applies under heightBehavior 'fixed'; autoGrow drops the ceiling. */
const EDITOR_MAX_HEIGHT = 300;

const DEFAULT_BEHAVIOR_SETTINGS: EditorBehaviorSettings = {
    editable: true,
    autoFocus: false,
    autoCorrect: true,
    autoCapitalize: 'sentences',
    keyboardType: 'default',
    heightBehavior: 'autoGrow',
    showToolbar: true,
    toolbarPlacement: 'keyboard',
};

const DEFAULT_IMAGE_SETTINGS: ImageSettings = {
    allowImageResizing: true,
    policy: {},
};

const DEFAULT_CONTROLLED_SETTINGS: ControlledSettings = {
    mode: 'uncontrolled',
    updateMode: 'replace',
};

type PreparedProseBenchmarkBridge = {
    preparedProseBenchmarkBegin(): void;
    preparedProseBenchmarkBeginPhase(phase: 'cold' | 'warm' | 'imagesDisabled'): void;
    preparedProseBenchmarkEndPhase(): void;
    preparedProseBenchmarkReset(): void;
    preparedProseBenchmarkExport(): string;
};

function truncateDetail(value: string): string {
    return value.length <= EVENT_LOG_DETAIL_LIMIT
        ? value
        : `${value.slice(0, EVENT_LOG_DETAIL_LIMIT)}…`;
}

function describeSelection(selection: Selection): string {
    const parts = [`type=${selection.type}`];
    if (selection.anchor != null) parts.push(`anchor=${selection.anchor}`);
    if (selection.head != null) parts.push(`head=${selection.head}`);
    if (selection.pos != null) parts.push(`pos=${selection.pos}`);
    return parts.join(' ');
}

function describeActiveState(state: ReadonlyActiveState): string {
    const activeMarks = Object.entries(state.marks)
        .filter(([, isActive]) => isActive)
        .map(([mark]) => mark);
    const activeNodes = Object.entries(state.nodes)
        .filter(([, isActive]) => isActive)
        .map(([node]) => node);
    return `marks=[${activeMarks.join(', ')}] nodes=[${activeNodes.join(', ')}]`;
}

const RootStack = createNativeStackNavigator<RootStackParamList>();

export default function App() {
    return (
        <SafeAreaProvider>
            <RootNavigator />
        </SafeAreaProvider>
    );
}

/** Owns the theme preset: the navigation bar re-themes with the screens. */
function RootNavigator() {
    const [selectedThemePresetId, setSelectedThemePresetId] = useState(
        DEFAULT_EXAMPLE_THEME_PRESET_ID
    );
    const activeThemePreset = useMemo(
        () => getExampleThemePreset(selectedThemePresetId),
        [selectedThemePresetId]
    );
    const chrome = activeThemePreset.appChrome;

    const navigationTheme = useMemo<Theme>(
        () => ({
            dark: activeThemePreset.statusBarStyle === 'light',
            colors: {
                primary: chrome.accentColor,
                background: chrome.screenBackgroundColor,
                // React Navigation paints bar surfaces with `card`.
                card: chrome.screenBackgroundColor,
                text: chrome.titleColor,
                border: chrome.separatorColor,
                notification: chrome.destructiveColor,
            },
            fonts: DefaultTheme.fonts,
        }),
        [activeThemePreset.statusBarStyle, chrome]
    );

    const screenOptions = useMemo<NativeStackNavigationOptions>(
        () => ({
            // Screen background, not card: title and accent clear 4.5:1 only against the screen.
            headerStyle: { backgroundColor: chrome.screenBackgroundColor },
            headerTintColor: chrome.accentColor,
            headerTitleStyle: { color: chrome.titleColor, fontWeight: '700' },
            // Otherwise the back button reads the full harness title.
            headerBackButtonDisplayMode: 'minimal',
            contentStyle: { backgroundColor: chrome.screenBackgroundColor },
        }),
        [chrome]
    );

    return (
        <NavigationContainer theme={navigationTheme}>
            {/* One StatusBar for the stack: expo-status-bar does not restore on unmount. */}
            <StatusBar style={activeThemePreset.statusBarStyle} />

            <RootStack.Navigator screenOptions={screenOptions}>
                <RootStack.Screen
                    name={HARNESS_ROUTE}
                    options={({ navigation }) => ({
                        title: ROUTE_TITLES[HARNESS_ROUTE],
                        headerRight: () => (
                            <HeaderActionButton
                                label='Benchmark'
                                chrome={chrome}
                                accessibilityHint='Opens the FlatList benchmark harness for the prose viewer.'
                                onPress={() => navigation.navigate(BENCHMARK_ROUTE)}
                            />
                        ),
                    })}>
                    {() => (
                        <HarnessScreen
                            activeThemePreset={activeThemePreset}
                            onSelectThemePreset={setSelectedThemePresetId}
                        />
                    )}
                </RootStack.Screen>

                <RootStack.Screen
                    name={BENCHMARK_ROUTE}
                    options={{ title: ROUTE_TITLES[BENCHMARK_ROUTE] }}>
                    {() => <PreparedViewerBenchmarkScreen preset={activeThemePreset} />}
                </RootStack.Screen>
            </RootStack.Navigator>
        </NavigationContainer>
    );
}

type HarnessScreenProps = {
    activeThemePreset: ExampleThemePreset;
    onSelectThemePreset: (id: string) => void;
};

function HarnessScreen({ activeThemePreset, onSelectThemePreset }: HarnessScreenProps) {
    const insets = useSafeAreaInsets();
    const headerHeight = useHeaderHeight();
    const editorRef = useRef<NativeRichTextEditorRef>(null);
    const [settingsTab, setSettingsTab] = useState<SettingsTab>('editor');
    const [readoutPane, setReadoutPane] = useState<ReadoutPane>('html');
    const [baseFontSize, setBaseFontSize] = useState(DEFAULT_BASE_FONT_SIZE);
    const [html, setHtml] = useState(INITIAL_CONTENT);
    const [contentJson, setContentJson] = useState<DocumentJSON | null>(null);
    const [events, setEvents] = useState<readonly EditorEventEntry[]>([]);

    const [mentionsEnabled, setMentionsEnabled] = useState(false);
    const [mentionQueryEvent, setMentionQueryEvent] = useState<MentionQueryChangeEvent | null>(
        null
    );
    const [mentionSelectEvent, setMentionSelectEvent] = useState<MentionSelectEvent | null>(null);

    const [expandedToolbarColor, setExpandedToolbarColor] = useState<ToolbarColorKey | null>(null);
    const [expandedEditorColor, setExpandedEditorColor] = useState<'blockquoteBorderColor' | null>(
        null
    );

    const [toolbarItems, setToolbarItems] = useState<readonly EditorToolbarItem[]>(
        DEFAULT_EDITOR_TOOLBAR_ITEMS
    );
    const [behavior, setBehavior] = useState<EditorBehaviorSettings>(DEFAULT_BEHAVIOR_SETTINGS);
    const [imageSettings, setImageSettings] = useState<ImageSettings>(DEFAULT_IMAGE_SETTINGS);
    const [controlled, setControlled] = useState<ControlledSettings>(DEFAULT_CONTROLLED_SETTINGS);
    const [controlledHtml, setControlledHtml] = useState(INITIAL_CONTENT);
    const [controlledJson, setControlledJson] = useState<DocumentJSON | null>(null);
    const [valueRevisionCounter, setValueRevisionCounter] = useState(0);
    const [documentRevisionCounter, setDocumentRevisionCounter] = useState<number | null>(null);

    const [linkRequest, setLinkRequest] = useState<LinkRequestContext | null>(null);
    const [linkDraft, setLinkDraft] = useState('');

    const chrome = activeThemePreset.appChrome;

    const [toolbarTheme, setToolbarTheme] = useState<Required<EditorToolbarTheme>>(
        () => activeThemePreset.toolbar
    );
    const [editorThemeOverrides, setEditorThemeOverrides] = useState<ExampleEditorThemeOverrides>(
        () => ({ blockquoteBorderColor: activeThemePreset.blockquote.borderColor })
    );

    useEffect(() => {
        setToolbarTheme(activeThemePreset.toolbar);
        setEditorThemeOverrides({
            blockquoteBorderColor: activeThemePreset.blockquote.borderColor,
        });
        setExpandedToolbarColor(null);
        setExpandedEditorColor(null);
    }, [activeThemePreset]);

    useEffect(() => {
        if (!mentionsEnabled) {
            setMentionQueryEvent(null);
            setMentionSelectEvent(null);
        }
    }, [mentionsEnabled]);

    // Selection and active state fire per caret move, so one debounced flush.

    const mountedAtRef = useRef(Date.now());
    const eventIdRef = useRef(0);
    const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const pendingRef = useRef<{
        html?: string;
        contentJson?: DocumentJSON | null;
        events: EditorEventEntry[];
    }>({ events: [] });

    const flushPending = useCallback(() => {
        flushTimerRef.current = null;
        const pending = pendingRef.current;
        pendingRef.current = { events: [] };

        if (Object.prototype.hasOwnProperty.call(pending, 'html')) {
            setHtml(pending.html ?? INITIAL_CONTENT);
        }
        if (Object.prototype.hasOwnProperty.call(pending, 'contentJson')) {
            setContentJson(pending.contentJson ?? null);
        }
        if (pending.events.length > 0) {
            const newestFirst = [...pending.events].reverse();
            setEvents((current) => [...newestFirst, ...current].slice(0, EVENT_LOG_LIMIT));
        }
    }, []);

    const scheduleFlush = useCallback(() => {
        if (flushTimerRef.current != null) return;
        flushTimerRef.current = setTimeout(flushPending, OUTPUT_PANEL_UPDATE_DEBOUNCE_MS);
    }, [flushPending]);

    useEffect(
        () => () => {
            if (flushTimerRef.current != null) {
                clearTimeout(flushTimerRef.current);
                flushTimerRef.current = null;
            }
        },
        []
    );

    const appendEvent = useCallback(
        (kind: string, detail: string) => {
            eventIdRef.current += 1;
            pendingRef.current.events.push({
                id: eventIdRef.current,
                kind,
                detail: truncateDetail(detail),
                atSeconds: (Date.now() - mountedAtRef.current) / MILLISECONDS_PER_SECOND,
            });
            scheduleFlush();
        },
        [scheduleFlush]
    );

    const documentSchema = useMemo(
        () => (mentionsEnabled ? withMentionsSchema(tiptapSchema) : tiptapSchema),
        [mentionsEnabled]
    );

    /** Marks come from the schema, so the panel cannot offer one that does not exist. */
    const toggleableMarks = useMemo(
        () =>
            documentSchema.marks.map((mark) => mark.name).filter((name) => name !== LINK_MARK_NAME),
        [documentSchema]
    );

    const localContentRef = useRef<DocumentJSON | null>(null);
    const documentConfig = useMemo(
        () => ({
            schema: documentSchema,
            policy: { allowBase64Images: false },
            limits: { resource: DEFAULT_EDITOR_RESOURCE_LIMITS },
        }),
        [documentSchema]
    );

    const documentHandle = useMemo(() => {
        const localJson = localContentRef.current;
        return createNativeEditorDocumentHandle({
            ...documentConfig,
            initialization: localJson
                ? { type: 'localJson', json: localJson }
                : { type: 'localHtml', html: INITIAL_CONTENT },
        });
    }, [documentConfig]);

    useEffect(() => () => documentHandle.destroy(), [documentHandle]);

    const theme = useMemo(() => {
        const fontSize = baseFontSize || DEFAULT_BASE_FONT_SIZE;
        return buildExampleEditorTheme(
            activeThemePreset,
            fontSize,
            toolbarTheme,
            editorThemeOverrides
        );
    }, [activeThemePreset, baseFontSize, editorThemeOverrides, toolbarTheme]);

    const addons = useMemo<EditorAddons | undefined>(() => {
        if (!mentionsEnabled) {
            return undefined;
        }

        return {
            mentions: {
                trigger: '@',
                suggestions: EXAMPLE_MENTION_SUGGESTIONS,
                theme: activeThemePreset.mentions,
                onQueryChange: setMentionQueryEvent,
                onSelect: setMentionSelectEvent,
            },
        };
    }, [activeThemePreset.mentions, mentionsEnabled]);

    const handleContentChange = useCallback(
        (nextHtml: string) => {
            pendingRef.current.html = nextHtml;
            scheduleFlush();
        },
        [scheduleFlush]
    );

    const handleContentChangeJSON = useCallback(
        (json: DocumentJSON) => {
            localContentRef.current = json;
            pendingRef.current.contentJson = json;
            scheduleFlush();
        },
        [scheduleFlush]
    );

    const handleFocus = useCallback(
        () => appendEvent('onFocus', 'editor gained focus'),
        [appendEvent]
    );
    const handleBlur = useCallback(() => appendEvent('onBlur', 'editor lost focus'), [appendEvent]);
    const handleSelectionChange = useCallback(
        (selection: Selection) => appendEvent('onSelectionChange', describeSelection(selection)),
        [appendEvent]
    );
    const handleActiveStateChange = useCallback(
        (state: ReadonlyActiveState) =>
            appendEvent('onActiveStateChange', describeActiveState(state)),
        [appendEvent]
    );
    const handleHistoryStateChange = useCallback(
        (state: HistoryState) =>
            appendEvent(
                'onHistoryStateChange',
                `canUndo=${state.canUndo} canRedo=${state.canRedo}`
            ),
        [appendEvent]
    );
    const handleLocalCommit = useCallback(
        () => appendEvent('onLocalCommit', 'local mutation committed to the engine'),
        [appendEvent]
    );
    const handleToolbarAction = useCallback(
        (key: string) => appendEvent('onToolbarAction', `key=${key}`),
        [appendEvent]
    );

    const handleRequestLink = useCallback(
        (context: LinkRequestContext) => {
            appendEvent(
                'onRequestLink',
                `isActive=${context.isActive} href=${context.href ?? '(none)'}`
            );
            setLinkDraft(context.href ?? '');
            setLinkRequest(context);
        },
        [appendEvent]
    );

    const closeLinkRequest = useCallback(() => {
        setLinkRequest(null);
        setLinkDraft('');
    }, []);

    const applyLinkRequest = useCallback(() => {
        const trimmed = linkDraft.trim();
        if (trimmed.length === 0) {
            linkRequest?.unsetLink();
        } else {
            linkRequest?.setLink(trimmed);
        }
        closeLinkRequest();
    }, [closeLinkRequest, linkDraft, linkRequest]);

    const removeLinkRequest = useCallback(() => {
        linkRequest?.unsetLink();
        closeLinkRequest();
    }, [closeLinkRequest, linkRequest]);

    const resolvedDecodeLimit =
        imageSettings.policy.maxDecodeDimensionPx ??
        DEFAULT_EDITOR_IMAGE_LOADING_POLICY.maxDecodeDimensionPx;

    /** Picks an image and downsizes it to the active decode limit. */
    const pickImageUri = useCallback(async (): Promise<string | null> => {
        const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
        if (!permission.granted) {
            appendEvent('image', 'photo library permission denied');
            return null;
        }

        const result = await ImagePicker.launchImageLibraryAsync({ quality: 1 });
        if (result.canceled || result.assets.length === 0) {
            appendEvent('image', 'picker cancelled');
            return null;
        }

        const asset = result.assets[0];
        const context = ImageManipulator.manipulate(asset.uri);
        // Downscale only: an unconditional resize enlarged anything already smaller.
        if (asset.width > resolvedDecodeLimit) {
            context.resize({ width: resolvedDecodeLimit });
        }
        const rendered = await context.renderAsync();
        const saved = await rendered.saveAsync({
            format: SaveFormat.JPEG,
            compress: PICKED_IMAGE_COMPRESSION,
        });
        appendEvent('image', `picked ${saved.width}x${saved.height}`);
        return saved.uri;
    }, [appendEvent, resolvedDecodeLimit]);

    const handleRequestImage = useCallback(
        (context: ImageRequestContext) => {
            appendEvent('onRequestImage', describeSelection(context.selection));
            void pickImageUri().then((uri) => {
                if (uri != null) {
                    context.insertImage(uri);
                }
            });
        },
        [appendEvent, pickImageUri]
    );

    const handlePickImageFromPanel = useCallback(() => {
        void pickImageUri().then((uri) => {
            if (uri != null) {
                editorRef.current?.insertImage(uri);
            }
        });
    }, [pickImageUri]);

    const handleInsertSampleImage = useCallback(() => {
        editorRef.current?.insertImage(SAMPLE_IMAGE_URL);
        appendEvent('insertImage', SAMPLE_IMAGE_URL);
    }, [appendEvent]);

    const handleToggleMark = useCallback((mark: string) => {
        editorRef.current?.toggleMark(mark);
    }, []);

    const handleCommand = useCallback(
        (id: EditorCommandId) => {
            const editor = editorRef.current;
            if (editor == null) return;

            switch (id) {
                case 'block:blockquote':
                    editor.toggleBlockquote();
                    return;
                case 'block:bulletList':
                    editor.toggleList('bulletList');
                    return;
                case 'block:orderedList':
                    editor.toggleList('orderedList');
                    return;
                case 'block:indent':
                    editor.indentListItem();
                    return;
                case 'block:outdent':
                    editor.outdentListItem();
                    return;
                case 'heading:1':
                case 'heading:2':
                case 'heading:3':
                case 'heading:4':
                case 'heading:5':
                case 'heading:6':
                    editor.toggleHeading(
                        Number(id.slice('heading:'.length)) as EditorToolbarHeadingLevel
                    );
                    return;
                case 'insert:hardBreak':
                    editor.insertNode('hardBreak');
                    return;
                case 'insert:horizontalRule':
                    editor.insertNode('horizontalRule');
                    return;
                case 'insert:text':
                    editor.insertText(INSERT_TEXT_SAMPLE);
                    return;
                case 'insert:html':
                    editor.insertContentHtml(INSERT_HTML_SAMPLE);
                    return;
                case 'insert:json':
                    editor.insertContentJson({
                        type: 'doc',
                        content: [
                            {
                                type: 'paragraph',
                                content: [{ type: 'text', text: 'Inserted JSON fragment.' }],
                            },
                        ],
                    });
                    return;
                case 'insert:image':
                    handleInsertSampleImage();
                    return;
                case 'doc:setContent':
                    editor.setContent(INITIAL_CONTENT);
                    return;
                case 'doc:setContentJson':
                    editor.setContentJson(editor.getContentJson());
                    appendEvent('setContentJson', 'reapplied the current document');
                    return;
                case 'doc:clear':
                    editor.clearContent();
                    return;
                case 'read:content':
                    appendEvent('getContent', editor.getContent());
                    return;
                case 'read:contentJson':
                    appendEvent('getContentJson', JSON.stringify(editor.getContentJson()));
                    return;
                case 'read:text':
                    appendEvent('getTextContent', editor.getTextContent());
                    return;
                case 'read:caretRect':
                    void editor
                        .getCaretRect()
                        .then((rect) => appendEvent('getCaretRect', JSON.stringify(rect)));
                    return;
                case 'history:undo':
                    editor.undo();
                    return;
                case 'history:redo':
                    editor.redo();
                    return;
                case 'history:state':
                    appendEvent(
                        'history',
                        `canUndo=${editor.canUndo()} canRedo=${editor.canRedo()}`
                    );
                    return;
                case 'focus':
                    editor.focus();
                    return;
                case 'blur':
                    editor.blur();
                    return;
            }
        },
        [appendEvent, handleInsertSampleImage]
    );

    const handleBehaviorChange = useCallback((patch: Partial<EditorBehaviorSettings>) => {
        setBehavior((current) => ({ ...current, ...patch }));
    }, []);

    const handleImageSettingsChange = useCallback((patch: Partial<ImageSettings>) => {
        setImageSettings((current) => ({ ...current, ...patch }));
    }, []);

    /** Seeding runs beside the state update; a setState updater has to stay pure. */
    const controlledModeRef = useRef(DEFAULT_CONTROLLED_SETTINGS.mode);

    const handleControlledChange = useCallback((patch: Partial<ControlledSettings>) => {
        const nextMode = patch.mode;
        if (nextMode != null && nextMode !== controlledModeRef.current) {
            // Seed from what is on screen so switching modes never blanks the editor.
            const editor = editorRef.current;
            if (editor != null) {
                if (nextMode === 'html') {
                    setControlledHtml(editor.getContent());
                }
                if (nextMode === 'json') {
                    setControlledJson(editor.getContentJson());
                    setValueRevisionCounter((revision) => revision + 1);
                }
            }
            controlledModeRef.current = nextMode;
        }

        setControlled((current) => ({ ...current, ...patch }));
    }, []);

    const handleBumpValueRevision = useCallback(() => {
        setValueRevisionCounter((revision) => revision + 1);
    }, []);

    const handleBumpDocumentRevision = useCallback(() => {
        setDocumentRevisionCounter((revision) => (revision == null ? 0 : revision + 1));
    }, []);

    const handleLoadControlledDocument = useCallback(() => {
        setControlledHtml(CONTROLLED_CONTENT);
        setControlledJson(null);
        setValueRevisionCounter((revision) => revision + 1);
        appendEvent('controlled', 'pushed the sample document');
    }, [appendEvent]);

    const handleLoadInitialDocument = useCallback(() => {
        setControlledHtml(INITIAL_CONTENT);
        setControlledJson(null);
        setValueRevisionCounter((revision) => revision + 1);
        appendEvent('controlled', 'pushed the initial document');
    }, [appendEvent]);

    const handleResetToolbarItems = useCallback(() => {
        setToolbarItems(DEFAULT_EDITOR_TOOLBAR_ITEMS);
    }, []);

    const handleToolbarItemsChange = useCallback((items: EditorToolbarItem[]) => {
        setToolbarItems(items);
    }, []);

    const jsonSnapshot = useMemo(() => {
        if (!contentJson) {
            return 'Edit the document to capture the current ProseMirror JSON.';
        }
        return JSON.stringify(contentJson, null, 2);
    }, [contentJson]);

    const mentionQuerySummary = useMemo(() => {
        if (!mentionsEnabled) return 'Mentions are disabled.';
        if (!mentionQueryEvent) return 'Type @ to show native mention suggestions in the toolbar.';
        return JSON.stringify(mentionQueryEvent, null, 2);
    }, [mentionQueryEvent, mentionsEnabled]);

    const mentionSelectionSummary = useMemo(() => {
        if (!mentionsEnabled)
            return 'Enable mentions to see selection callbacks and mention attrs.';
        if (!mentionSelectEvent) return 'Pick a suggestion to inspect the inserted attrs payload.';
        return JSON.stringify(mentionSelectEvent, null, 2);
    }, [mentionSelectEvent, mentionsEnabled]);

    const settingsBadge = useMemo(
        () => SETTINGS_TABS.find((tab) => tab.value === settingsTab)?.label,
        [settingsTab]
    );

    const editorStyle = useMemo(
        () => (behavior.heightBehavior === 'autoGrow' ? styles.editorAutoGrow : styles.editorFixed),
        [behavior.heightBehavior]
    );

    const contentContainerStyle = useMemo(
        // The navigation bar consumes the top inset.
        () => [styles.content, { paddingTop: SPACE.xl, paddingBottom: SPACE.xxl + insets.bottom }],
        [insets.bottom]
    );

    const valueJSONRevision = `r${valueRevisionCounter}`;
    const documentRevision =
        documentRevisionCounter == null ? null : `doc-${documentRevisionCounter}`;

    return (
        <View style={[styles.safeArea, { backgroundColor: chrome.screenBackgroundColor }]}>
            <KeyboardAvoidingView
                style={styles.keyboardAvoider}
                enabled={Platform.OS === 'ios'}
                behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
                // The bar is outside this view but inside the window the keyboard is measured against.
                keyboardVerticalOffset={headerHeight}>
                <ScrollView
                    style={[styles.screen, { backgroundColor: chrome.screenBackgroundColor }]}
                    contentContainerStyle={contentContainerStyle}
                    automaticallyAdjustKeyboardInsets={Platform.OS === 'ios'}
                    keyboardDismissMode={Platform.OS === 'ios' ? 'interactive' : 'on-drag'}
                    keyboardShouldPersistTaps='always'>
                    <View style={styles.header}>
                        <Text style={[styles.subtitle, { color: chrome.subtitleColor }]}>
                            Manual test harness. Every editor prop, ref method, and callback the
                            package exposes is reachable from this screen.
                        </Text>
                    </View>

                    <CollapsibleSection
                        title='Theme preset'
                        badge={activeThemePreset.label}
                        chrome={chrome}
                        style={[styles.card, { backgroundColor: chrome.cardBackgroundColor }]}>
                        <ThemePresetPicker
                            presets={EXAMPLE_THEME_PRESETS}
                            selectedId={activeThemePreset.id}
                            onSelect={onSelectThemePreset}
                            chrome={chrome}
                        />
                    </CollapsibleSection>

                    <SettingsCard
                        tab={settingsTab}
                        onTabChange={setSettingsTab}
                        badge={settingsBadge}
                        chrome={chrome}>
                        {settingsTab === 'editor' ? (
                            <EditorSettingsPanel
                                baseFontSize={baseFontSize}
                                onBaseFontSizeChange={setBaseFontSize}
                                mentionsEnabled={mentionsEnabled}
                                onMentionsEnabledChange={setMentionsEnabled}
                                blockquoteBorderColor={
                                    editorThemeOverrides.blockquoteBorderColor ??
                                    activeThemePreset.blockquote.borderColor
                                }
                                onBlockquoteBorderColorChange={(value) =>
                                    setEditorThemeOverrides((current) => ({
                                        ...current,
                                        blockquoteBorderColor: value,
                                    }))
                                }
                                expandedColor={expandedEditorColor}
                                onExpandedColorChange={setExpandedEditorColor}
                                sliderTheme={activeThemePreset.slider}
                                chrome={chrome}
                            />
                        ) : null}

                        {settingsTab === 'toolbar' ? (
                            <ToolbarSettingsPanel
                                toolbarTheme={toolbarTheme}
                                onToolbarThemeChange={setToolbarTheme}
                                expandedColor={expandedToolbarColor}
                                onExpandedColorChange={setExpandedToolbarColor}
                                sliderTheme={activeThemePreset.slider}
                                chrome={chrome}
                            />
                        ) : null}

                        {settingsTab === 'items' ? (
                            <ToolbarItemsEditor
                                items={toolbarItems}
                                onItemsChange={handleToolbarItemsChange}
                                onReset={handleResetToolbarItems}
                                chrome={chrome}
                            />
                        ) : null}

                        {settingsTab === 'content' ? (
                            <ContentSettingsPanel
                                settings={controlled}
                                onChange={handleControlledChange}
                                valueJSONRevision={valueJSONRevision}
                                documentRevision={documentRevision}
                                onBumpValueRevision={handleBumpValueRevision}
                                onBumpDocumentRevision={handleBumpDocumentRevision}
                                onLoadControlledDocument={handleLoadControlledDocument}
                                onLoadInitialDocument={handleLoadInitialDocument}
                                chrome={chrome}
                            />
                        ) : null}

                        {settingsTab === 'commands' ? (
                            <CommandsPanel
                                onCommand={handleCommand}
                                toggleableMarks={toggleableMarks}
                                onToggleMark={handleToggleMark}
                                editable={behavior.editable}
                                chrome={chrome}
                            />
                        ) : null}

                        {settingsTab === 'input' ? (
                            <InputSettingsPanel
                                settings={behavior}
                                onChange={handleBehaviorChange}
                                chrome={chrome}
                            />
                        ) : null}

                        {settingsTab === 'images' ? (
                            <ImageSettingsPanel
                                settings={imageSettings}
                                onChange={handleImageSettingsChange}
                                onPickImage={handlePickImageFromPanel}
                                onInsertSampleImage={handleInsertSampleImage}
                                sliderTheme={activeThemePreset.slider}
                                chrome={chrome}
                            />
                        ) : null}
                    </SettingsCard>

                    <View
                        style={[
                            styles.editorCard,
                            { backgroundColor: chrome.cardSecondaryBackgroundColor },
                        ]}>
                        <Text
                            style={[
                                sharedStyles.sectionLabel,
                                { color: chrome.sectionLabelColor },
                            ]}>
                            Editor
                        </Text>
                        <NativeRichTextEditor
                            ref={editorRef}
                            documentHandle={documentHandle}
                            documentRevision={documentRevision}
                            theme={theme}
                            addons={addons}
                            placeholder='Start typing...'
                            accessibilityLabel='Rich text editor under test'
                            accessibilityHint='Formatting is available from the toolbar above the keyboard.'
                            value={controlled.mode === 'html' ? controlledHtml : undefined}
                            valueJSON={
                                controlled.mode === 'json'
                                    ? (controlledJson ?? undefined)
                                    : undefined
                            }
                            valueJSONRevision={
                                controlled.mode === 'json' ? valueJSONRevision : undefined
                            }
                            valueJSONUpdateMode={controlled.updateMode}
                            editable={behavior.editable}
                            autoFocus={behavior.autoFocus}
                            autoCorrect={behavior.autoCorrect}
                            autoCapitalize={behavior.autoCapitalize}
                            keyboardType={behavior.keyboardType}
                            heightBehavior={behavior.heightBehavior}
                            showToolbar={behavior.showToolbar}
                            toolbarPlacement={behavior.toolbarPlacement}
                            toolbarItems={toolbarItems}
                            allowImageResizing={imageSettings.allowImageResizing}
                            imageLoadingPolicy={imageSettings.policy}
                            onContentChange={handleContentChange}
                            onContentChangeJSON={handleContentChangeJSON}
                            onSelectionChange={handleSelectionChange}
                            onActiveStateChange={handleActiveStateChange}
                            onHistoryStateChange={handleHistoryStateChange}
                            onLocalCommit={handleLocalCommit}
                            onToolbarAction={handleToolbarAction}
                            onRequestLink={handleRequestLink}
                            onRequestImage={handleRequestImage}
                            onFocus={handleFocus}
                            onBlur={handleBlur}
                            style={editorStyle}
                            containerStyle={styles.editorContainer}
                        />
                    </View>

                    <ReadoutPanel
                        pane={readoutPane}
                        onPaneChange={setReadoutPane}
                        html={html}
                        jsonSnapshot={jsonSnapshot}
                        mentionQuerySummary={mentionQuerySummary}
                        mentionSelectionSummary={mentionSelectionSummary}
                        events={events}
                        chrome={chrome}
                    />

                    <Text style={[styles.copyright, { color: chrome.subtitleColor }]}>
                        {'©'} {new Date().getFullYear()} Apollo Health Group Pty Ltd. All rights
                        reserved.
                    </Text>
                </ScrollView>
            </KeyboardAvoidingView>

            <LinkEditorModal
                visible={linkRequest != null}
                isActive={linkRequest?.isActive ?? false}
                linkDraft={linkDraft}
                onLinkDraftChange={setLinkDraft}
                onClose={closeLinkRequest}
                onRemove={removeLinkRequest}
                onApply={applyLinkRequest}
                chrome={chrome}
            />
        </View>
    );
}

type CorpusEntry = {
    id: string;
    category: string;
    contentJSON: DocumentJSON;
};

type WarmWindow = {
    id: string;
    primeIds: string[];
    warmIds: string[];
};

type PerformanceCorpus = {
    documents: CorpusEntry[];
    warmWindows: WarmWindow[];
};

type ScrollCommandToken = {
    runId: number;
    windowIndex: number;
    direction: 'prime' | 'warm';
    expectedDirection: 'forward' | 'reverse';
    expectedTerminalEntryId: string;
    dispatched: boolean;
    momentumBegan: boolean;
    consumed: boolean;
    dispatchOffsetY: number;
};

const preparedViewerCorpus = performanceCorpus as PerformanceCorpus;
const preparedViewerConfiguration = preparedProseBenchmarkConfiguration as {
    configuration: { schema: Parameters<typeof NativeProseViewer>[0]['schema'] };
    imageLoadingPolicy: Parameters<typeof NativeProseViewer>[0]['imageLoadingPolicy'];
};

/** Deterministic FlatList harness; it consumes the checked-in corpus verbatim. */
function PreparedViewerBenchmarkScreen({ preset }: { preset: ExampleThemePreset }) {
    // Resolved here so normal editor use does not depend on the benchmark bridge.
    const benchmarkBridge = useMemo(
        () => requireNativeModule<PreparedProseBenchmarkBridge>('NativeEditor'),
        []
    );
    const insets = useSafeAreaInsets();
    const chrome = preset.appChrome;
    const [windowIndex, setWindowIndex] = useState(0);
    const [phase, setPhase] = useState<'cold' | 'warm' | 'imagesDisabled'>('cold');
    const [direction, setDirection] = useState<'prime' | 'warm'>('prime');
    const [imagesEnabled, setImagesEnabled] = useState(true);
    const [exportedCounters, setExportedCounters] = useState('Counters not exported yet.');
    const [benchmarkError, setBenchmarkError] = useState<string | null>(null);
    const [isTraversing, setIsTraversing] = useState(false);
    const listRef = useRef<FlatList<CorpusEntry>>(null);
    const traversalInFlightRef = useRef(false);
    const activeBridgePhaseRef = useRef<'cold' | 'warm' | 'imagesDisabled' | null>(null);
    const activeRunIdRef = useRef<number | null>(null);
    const nextRunIdRef = useRef(0);
    const activeScrollCommandRef = useRef<ScrollCommandToken | null>(null);
    const scrollCommandWatchdogRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const visibleEntryIdsRef = useRef<Set<string>>(new Set());
    const latestNativeContentOffsetYRef = useRef(0);
    useEffect(() => {
        benchmarkBridge.preparedProseBenchmarkBegin();
    }, [benchmarkBridge]);
    const byId = useMemo(
        () => new Map(preparedViewerCorpus.documents.map((entry) => [entry.id, entry])),
        []
    );
    const window = preparedViewerCorpus.warmWindows[windowIndex];
    const entries = useMemo(
        () =>
            (direction === 'prime' ? window.primeIds : window.warmIds)
                .map((id) => byId.get(id))
                .filter((entry): entry is CorpusEntry => entry != null),
        [byId, direction, window]
    );
    const endActiveBridgePhase = useCallback(() => {
        if (activeBridgePhaseRef.current == null) return;
        benchmarkBridge.preparedProseBenchmarkEndPhase();
        activeBridgePhaseRef.current = null;
    }, [benchmarkBridge]);

    const clearScrollCommandWatchdog = useCallback(() => {
        if (scrollCommandWatchdogRef.current == null) return;
        clearTimeout(scrollCommandWatchdogRef.current);
        scrollCommandWatchdogRef.current = null;
    }, []);

    const releaseActiveTraversal = useCallback(() => {
        clearScrollCommandWatchdog();
        activeScrollCommandRef.current = null;
        activeRunIdRef.current = null;
        traversalInFlightRef.current = false;
        endActiveBridgePhase();
    }, [clearScrollCommandWatchdog, endActiveBridgePhase]);

    const cancelActiveTraversal = useCallback(
        (errorMessage?: string) => {
            releaseActiveTraversal();
            setIsTraversing(false);
            if (errorMessage != null) {
                console.warn(errorMessage);
                setBenchmarkError(errorMessage);
            }
        },
        [releaseActiveTraversal]
    );

    const beginWindowRun = useCallback(
        (nextImagesEnabled: boolean) => {
            if (traversalInFlightRef.current) return;
            clearScrollCommandWatchdog();
            activeScrollCommandRef.current = null;
            traversalInFlightRef.current = true;
            const runId = nextRunIdRef.current + 1;
            nextRunIdRef.current = runId;
            activeRunIdRef.current = runId;
            activeBridgePhaseRef.current = null;
            visibleEntryIdsRef.current = new Set();
            setBenchmarkError(null);
            setImagesEnabled(nextImagesEnabled);
            setWindowIndex(0);
            setPhase(nextImagesEnabled ? 'cold' : 'imagesDisabled');
            setDirection('prime');
            setIsTraversing(true);
        },
        [clearScrollCommandWatchdog]
    );

    const advanceDirectionOrWindow = useCallback(
        (completedCommand: ScrollCommandToken) => {
            if (
                !traversalInFlightRef.current ||
                activeRunIdRef.current !== completedCommand.runId ||
                completedCommand.windowIndex !== windowIndex ||
                completedCommand.direction !== direction
            ) {
                return;
            }
            if (direction === 'prime') {
                if (phase !== 'imagesDisabled') {
                    endActiveBridgePhase();
                    setPhase('warm');
                }
                setDirection('warm');
                return;
            }

            const nextWindowIndex = windowIndex + 1;
            if (nextWindowIndex < preparedViewerCorpus.warmWindows.length) {
                if (phase !== 'imagesDisabled') {
                    endActiveBridgePhase();
                    setPhase('cold');
                }
                visibleEntryIdsRef.current = new Set();
                setWindowIndex(nextWindowIndex);
                setDirection('prime');
                return;
            }

            cancelActiveTraversal();
        },
        [cancelActiveTraversal, direction, endActiveBridgePhase, phase, windowIndex]
    );

    const dispatchScrollCommand = useCallback(
        (commandWindowIndex: number, commandDirection: 'prime' | 'warm') => {
            const runId = activeRunIdRef.current;
            if (!traversalInFlightRef.current || runId == null) return;

            const commandWindow = preparedViewerCorpus.warmWindows[commandWindowIndex];
            const commandIds =
                commandDirection === 'prime' ? commandWindow.primeIds : commandWindow.warmIds;
            const expectedTerminalEntryId =
                commandDirection === 'prime' ? commandIds[commandIds.length - 1] : commandIds[0];
            if (expectedTerminalEntryId == null) {
                cancelActiveTraversal(
                    `Prepared viewer benchmark aborted: ${commandDirection} window ${commandWindowIndex + 1} has no terminal entry.`
                );
                return;
            }

            clearScrollCommandWatchdog();
            const dispatchOffsetY = latestNativeContentOffsetYRef.current;
            const command: ScrollCommandToken = {
                runId,
                windowIndex: commandWindowIndex,
                direction: commandDirection,
                expectedDirection: commandDirection === 'prime' ? 'forward' : 'reverse',
                expectedTerminalEntryId,
                dispatched: true,
                momentumBegan: false,
                consumed: false,
                dispatchOffsetY,
            };
            activeScrollCommandRef.current = command;
            scrollCommandWatchdogRef.current = setTimeout(() => {
                const activeCommand = activeScrollCommandRef.current;
                if (
                    activeCommand?.runId !== command.runId ||
                    activeCommand.windowIndex !== command.windowIndex ||
                    activeCommand.direction !== command.direction ||
                    !activeCommand.dispatched ||
                    activeCommand.consumed
                ) {
                    return;
                }
                cancelActiveTraversal(
                    `Prepared viewer benchmark aborted: ${command.direction} scroll for window ${command.windowIndex + 1} did not complete valid motion.`
                );
            }, SCROLL_COMMAND_NO_MOTION_TIMEOUT_MS);

            if (commandDirection === 'prime') {
                listRef.current?.scrollToEnd({ animated: true });
            } else {
                listRef.current?.scrollToIndex({ index: 0, animated: true });
            }
        },
        [cancelActiveTraversal, clearScrollCommandWatchdog]
    );

    useEffect(() => {
        if (!isTraversing) return;
        if (activeBridgePhaseRef.current !== phase) {
            benchmarkBridge.preparedProseBenchmarkBeginPhase(phase);
            activeBridgePhaseRef.current = phase;
        }
        const frame = requestAnimationFrame(() => {
            dispatchScrollCommand(windowIndex, direction);
        });
        return () => cancelAnimationFrame(frame);
    }, [benchmarkBridge, direction, dispatchScrollCommand, isTraversing, phase, windowIndex]);

    const handleViewableItemsChanged = useCallback(
        ({ viewableItems }: { viewableItems: Array<ViewToken> }) => {
            const visibleEntryIds = new Set<string>();
            for (const viewableItem of viewableItems) {
                const entry = viewableItem.item as CorpusEntry | null;
                if (entry?.id != null) {
                    visibleEntryIds.add(entry.id);
                }
            }
            visibleEntryIdsRef.current = visibleEntryIds;
        },
        []
    );

    const handleScroll = useCallback((event: NativeSyntheticEvent<NativeScrollEvent>) => {
        latestNativeContentOffsetYRef.current = event.nativeEvent.contentOffset.y;
    }, []);

    const handleMomentumScrollBegin = useCallback(() => {
        const command = activeScrollCommandRef.current;
        if (
            !traversalInFlightRef.current ||
            command == null ||
            activeRunIdRef.current !== command.runId ||
            !command.dispatched ||
            command.momentumBegan ||
            command.consumed
        ) {
            return;
        }
        command.momentumBegan = true;
    }, []);

    const handleMomentumScrollEnd = useCallback(
        (event: NativeSyntheticEvent<NativeScrollEvent>) => {
            latestNativeContentOffsetYRef.current = event.nativeEvent.contentOffset.y;
            const command = activeScrollCommandRef.current;
            if (
                !traversalInFlightRef.current ||
                command == null ||
                activeRunIdRef.current !== command.runId ||
                !command.dispatched ||
                !command.momentumBegan ||
                command.consumed
            ) {
                return;
            }
            const offsetDelta = event.nativeEvent.contentOffset.y - command.dispatchOffsetY;
            const movedInExpectedDirection =
                command.expectedDirection === 'forward'
                    ? offsetDelta > SCROLL_COMMAND_MIN_OFFSET_DELTA
                    : offsetDelta < -SCROLL_COMMAND_MIN_OFFSET_DELTA;
            if (
                !movedInExpectedDirection ||
                !visibleEntryIdsRef.current.has(command.expectedTerminalEntryId)
            ) {
                return;
            }
            command.consumed = true;
            activeScrollCommandRef.current = null;
            clearScrollCommandWatchdog();
            advanceDirectionOrWindow(command);
        },
        [advanceDirectionOrWindow, clearScrollCommandWatchdog]
    );

    useEffect(
        () => () => {
            releaseActiveTraversal();
        },
        [releaseActiveTraversal]
    );

    const handleResetCache = useCallback(() => {
        // Reset is intentionally not a traversal phase.
        benchmarkBridge.preparedProseBenchmarkReset();
    }, [benchmarkBridge]);

    const handleExportCounters = useCallback(() => {
        setExportedCounters(benchmarkBridge.preparedProseBenchmarkExport());
    }, [benchmarkBridge]);

    const handleRunWarmWindows = useCallback(() => beginWindowRun(true), [beginWindowRun]);
    const handleRunImagesDisabled = useCallback(() => beginWindowRun(false), [beginWindowRun]);

    const viewerRowStyle = useMemo(
        () => [styles.benchmarkViewer, { backgroundColor: chrome.cardSecondaryBackgroundColor }],
        [chrome.cardSecondaryBackgroundColor]
    );

    /** Stable identity: an inline arrow would re-render inside the measured window. */
    const renderItem = useCallback(
        ({ item }: { item: CorpusEntry }) => (
            <NativeProseViewer
                contentJSON={item.contentJSON}
                schema={preparedViewerConfiguration.configuration.schema}
                imageLoadingPolicy={preparedViewerConfiguration.imageLoadingPolicy}
                renderImages={imagesEnabled}
                style={viewerRowStyle}
            />
        ),
        [imagesEnabled, viewerRowStyle]
    );

    const extraData = useMemo(
        () => `${phase}:${direction}:${imagesEnabled}`,
        [direction, imagesEnabled, phase]
    );

    const keyExtractor = useCallback((item: CorpusEntry) => item.id, []);

    return (
        <View
            style={[
                styles.benchmarkScreen,
                {
                    backgroundColor: chrome.screenBackgroundColor,
                    // The navigation bar owns the top inset.
                    paddingTop: SPACE.lg,
                    paddingBottom: insets.bottom,
                },
            ]}>
            <Text style={[styles.benchmarkSubtitle, { color: chrome.subtitleColor }]}>
                {entries.length} messages · {window.id} · {phase} {direction} · images{' '}
                {imagesEnabled ? 'enabled' : 'disabled'}
            </Text>

            <View style={styles.benchmarkControls}>
                <ActionButton
                    label='Run warm windows'
                    disabled={isTraversing}
                    onPress={handleRunWarmWindows}
                    chrome={chrome}
                />
                <ActionButton
                    label='Reset cache'
                    tone='secondary'
                    disabled={isTraversing}
                    onPress={handleResetCache}
                    chrome={chrome}
                />
                <ActionButton
                    label='Images disabled'
                    tone='secondary'
                    disabled={isTraversing}
                    onPress={handleRunImagesDisabled}
                    chrome={chrome}
                />
                <ActionButton
                    label='Export counters'
                    tone='secondary'
                    disabled={isTraversing}
                    onPress={handleExportCounters}
                    chrome={chrome}
                />
            </View>

            {benchmarkError != null ? (
                <Text style={[styles.benchmarkCounters, { color: chrome.destructiveColor }]}>
                    {benchmarkError}
                </Text>
            ) : null}

            <Text
                numberOfLines={3}
                style={[styles.benchmarkCounters, { color: chrome.subtitleColor }]}>
                {exportedCounters}
            </Text>

            <FlatList
                ref={listRef}
                data={entries}
                extraData={extraData}
                keyExtractor={keyExtractor}
                initialNumToRender={12}
                maxToRenderPerBatch={12}
                windowSize={9}
                onScroll={handleScroll}
                onViewableItemsChanged={handleViewableItemsChanged}
                onMomentumScrollBegin={handleMomentumScrollBegin}
                onMomentumScrollEnd={handleMomentumScrollEnd}
                renderItem={renderItem}
            />
        </View>
    );
}

const styles = StyleSheet.create({
    safeArea: {
        flex: 1,
    },
    keyboardAvoider: {
        flex: 1,
    },
    screen: {
        flex: 1,
    },
    content: {
        flexGrow: 1,
        paddingHorizontal: SPACE.xl,
        gap: SPACE.lg,
    },
    header: {
        gap: SPACE.sm,
    },
    subtitle: {
        fontSize: FONT_SIZE.label,
        lineHeight: LINE_HEIGHT.label,
    },
    copyright: {
        fontSize: FONT_SIZE.section,
        lineHeight: LINE_HEIGHT.hint,
        textAlign: 'center',
    },
    card: {
        padding: SPACE.lg,
        borderRadius: RADIUS,
    },
    editorCard: {
        borderRadius: RADIUS,
        padding: SPACE.lg,
        gap: SPACE.md,
    },
    editorFixed: {
        borderRadius: RADIUS,
        minHeight: EDITOR_MIN_HEIGHT,
        maxHeight: EDITOR_MAX_HEIGHT,
    },
    editorAutoGrow: {
        borderRadius: RADIUS,
        minHeight: EDITOR_MIN_HEIGHT,
    },
    /** Exercises `containerStyle`, the only way to space an inline toolbar. */
    editorContainer: {
        gap: SPACE.sm,
    },
    benchmarkScreen: {
        flex: 1,
        paddingHorizontal: SPACE.lg,
        gap: SPACE.sm,
    },
    benchmarkSubtitle: {
        fontSize: FONT_SIZE.hint,
        lineHeight: LINE_HEIGHT.hint,
    },
    benchmarkControls: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: SPACE.sm,
    },
    benchmarkViewer: {
        borderRadius: RADIUS,
        marginBottom: SPACE.sm,
        padding: SPACE.md,
    },
    benchmarkCounters: {
        fontFamily: MONO_FONT_FAMILY,
        fontSize: FONT_SIZE.monoMicro,
        lineHeight: LINE_HEIGHT.mono,
    },
});
