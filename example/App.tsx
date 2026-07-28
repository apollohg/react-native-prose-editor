import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { FlatList, KeyboardAvoidingView, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';
import { StatusBar } from 'expo-status-bar';
import { requireNativeModule } from 'expo-modules-core';
import { SafeAreaProvider, useSafeAreaInsets } from 'react-native-safe-area-context';

import {
    createNativeEditorDocumentHandle,
    DEFAULT_EDITOR_RESOURCE_LIMITS,
    NativeRichTextEditor,
    NativeProseViewer,
    tiptapSchema,
    useYjsCollaboration,
    withMentionsSchema,
    type DocumentJSON,
    type EditorAddons,
    type EditorToolbarTheme,
    type MentionQueryChangeEvent,
    type MentionSelectEvent,
    type NativeEditorV2RoomSnapshot,
    type NativeRichTextEditorRef,
} from '@apollohg/react-native-prose-editor';
import performanceCorpus from '../scripts/tests/viewer-performance-corpus.json';
import preparedProseBenchmarkConfiguration from '../scripts/tests/prepared-prose-benchmark-config.json';

import {
    buildExampleEditorTheme,
    DEFAULT_EXAMPLE_THEME_PRESET_ID,
    EXAMPLE_THEME_PRESETS,
    type ExampleEditorThemeOverrides,
    getExampleThemePreset,
} from './themePresets';

import { EXAMPLE_MENTION_SUGGESTIONS, INITIAL_CONTENT, type ToolbarColorKey } from './constants';
import { sharedStyles } from './sharedStyles';

import { ThemePresetPicker } from './components/ThemePresetPicker';
import { OutputCard } from './components/OutputCard';
import { CollapsibleSection } from './components/CollapsibleSection';
import { ThemeSettingsCard } from './components/ThemeSettingsCard';
import { CollaborationPanel } from './components/CollaborationPanel';

const DEFAULT_COLLABORATION_ENDPOINT = 'ws://localhost:1234/collaboration';
const DEFAULT_COLLABORATION_ROOM_ID = 'example-room';
const OUTPUT_PANEL_UPDATE_DEBOUNCE_MS = 120;
const SCROLL_COMMAND_NO_MOTION_TIMEOUT_MS = 1_500;

type PreparedProseBenchmarkBridge = {
    preparedProseBenchmarkBegin(): void;
    preparedProseBenchmarkBeginPhase(phase: 'cold' | 'warm' | 'imagesDisabled'): void;
    preparedProseBenchmarkEndPhase(): void;
    preparedProseBenchmarkReset(): void;
    preparedProseBenchmarkExport(): string;
};

function buildCollaborationSocketUrl(endpoint: string, documentId: string): string {
    const trimmedEndpoint = endpoint.trim();
    if (!trimmedEndpoint) {
        return trimmedEndpoint;
    }
    const separator = trimmedEndpoint.includes('?') ? '&' : '?';
    return `${trimmedEndpoint}${separator}documentId=${encodeURIComponent(documentId)}`;
}

export default function App() {
    return (
        <SafeAreaProvider>
            <AppScreen />
        </SafeAreaProvider>
    );
}

function AppScreen() {
    const insets = useSafeAreaInsets();
    const editorRef = useRef<NativeRichTextEditorRef>(null);
    const [settingsTab, setSettingsTab] = useState<'editor' | 'toolbar'>('editor');
    const [showPreparedViewerBenchmark, setShowPreparedViewerBenchmark] = useState(false);
    const [selectedThemePresetId, setSelectedThemePresetId] = useState(
        DEFAULT_EXAMPLE_THEME_PRESET_ID
    );
    const [baseFontSize, setBaseFontSize] = useState(17);
    const [html, setHtml] = useState(INITIAL_CONTENT);
    const [contentJson, setContentJson] = useState<DocumentJSON | null>(null);
    const outputPanelUpdateTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
    const pendingOutputPanelUpdateRef = useRef<{
        html?: string;
        contentJson?: DocumentJSON | null;
    }>({});

    const [mentionsEnabled, setMentionsEnabled] = useState(false);
    const [mentionQueryEvent, setMentionQueryEvent] = useState<MentionQueryChangeEvent | null>(
        null
    );
    const [mentionSelectEvent, setMentionSelectEvent] = useState<MentionSelectEvent | null>(null);

    const [expandedToolbarColor, setExpandedToolbarColor] = useState<ToolbarColorKey | null>(null);
    const [collaborationEnabled, setCollaborationEnabled] = useState(false);
    const [collaborationEndpoint, setCollaborationEndpoint] = useState(
        DEFAULT_COLLABORATION_ENDPOINT
    );
    const [collaborationRoomId, setCollaborationRoomId] = useState(DEFAULT_COLLABORATION_ROOM_ID);
    const [collaborationDisplayName, setCollaborationDisplayName] = useState(
        Platform.OS === 'ios' ? 'iOS Demo' : 'Android Demo'
    );

    const activeThemePreset = useMemo(
        () => getExampleThemePreset(selectedThemePresetId),
        [selectedThemePresetId]
    );

    const appChrome = activeThemePreset.appChrome;

    const [toolbarTheme, setToolbarTheme] = useState<Required<EditorToolbarTheme>>(
        () => activeThemePreset.toolbar
    );
    const [editorThemeOverrides, setEditorThemeOverrides] = useState<ExampleEditorThemeOverrides>(
        () => ({
            blockquoteBorderColor: activeThemePreset.blockquote.borderColor,
        })
    );
    const [expandedEditorColor, setExpandedEditorColor] = useState<'blockquoteBorderColor' | null>(
        null
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

    useEffect(
        () => () => {
            if (outputPanelUpdateTimerRef.current != null) {
                clearTimeout(outputPanelUpdateTimerRef.current);
                outputPanelUpdateTimerRef.current = null;
            }
        },
        []
    );

    const theme = useMemo(() => {
        const fontSize = baseFontSize || 17;
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

    const documentSchema = useMemo(
        () => (mentionsEnabled ? withMentionsSchema(tiptapSchema) : tiptapSchema),
        [mentionsEnabled]
    );

    const jsonSnapshot = useMemo(() => {
        if (!contentJson) {
            return 'Edit the document to capture the current ProseMirror JSON.';
        }

        return JSON.stringify(contentJson, null, 2);
    }, [contentJson]);

    const mentionQuerySummary = useMemo(() => {
        if (!mentionsEnabled) {
            return 'Mentions are disabled.';
        }

        if (!mentionQueryEvent) {
            return 'Type @ to show native mention suggestions in the toolbar.';
        }

        return JSON.stringify(mentionQueryEvent, null, 2);
    }, [mentionQueryEvent, mentionsEnabled]);

    const mentionSelectionSummary = useMemo(() => {
        if (!mentionsEnabled) {
            return 'Enable mentions to see selection callbacks and mention attrs.';
        }

        if (!mentionSelectEvent) {
            return 'Pick a suggestion to inspect the inserted attrs payload.';
        }

        return JSON.stringify(mentionSelectEvent, null, 2);
    }, [mentionSelectEvent, mentionsEnabled]);

    const collaborationColor = useMemo(() => (Platform.OS === 'ios' ? '#0A84FF' : '#34A853'), []);

    const collaborationRoomName = collaborationRoomId.trim() || DEFAULT_COLLABORATION_ROOM_ID;
    const collaborationDocumentId = useMemo(
        () => `${collaborationRoomName}|${collaborationEndpoint.trim()}`,
        [collaborationEndpoint, collaborationRoomName]
    );
    const collaborationSocketUrl = useMemo(
        () => buildCollaborationSocketUrl(collaborationEndpoint, collaborationRoomName),
        [collaborationEndpoint, collaborationRoomName]
    );
    // ── Document session ─────────────────────────────────────────
    // One NativeEditorDocumentHandle per session: a local handle while
    // offline, a room handle while collaborating. The editor and the
    // collaboration controller share the same handle; initialization
    // (including initial content) lives in the handle's creation config.
    const localContentRef = useRef<DocumentJSON | null>(null);
    const roomSnapshotRef = useRef<{
        roomKey: string;
        snapshot: NativeEditorV2RoomSnapshot;
    } | null>(null);
    // Engine schema, policy, and limits are fixed when the handle is created.
    // NativeRichTextEditor receives only this handle and per-view options.
    const documentConfig = useMemo(
        () => ({
            schema: documentSchema,
            policy: { allowBase64Images: false },
            limits: { resource: DEFAULT_EDITOR_RESOURCE_LIMITS },
        }),
        [documentSchema]
    );

    const documentHandle = useMemo(() => {
        if (!collaborationEnabled) {
            const localJson = localContentRef.current;
            return createNativeEditorDocumentHandle({
                ...documentConfig,
                initialization: localJson
                    ? { type: 'localJson', json: localJson }
                    : { type: 'localHtml', html: INITIAL_CONTENT },
            });
        }
        const stored = roomSnapshotRef.current;
        return createNativeEditorDocumentHandle({
            ...documentConfig,
            initialization: {
                type: 'room',
                documentId: collaborationDocumentId,
                lineageId: `example|${collaborationDocumentId}`,
                // Offline restore: re-enter a known room from its exported
                // snapshot; otherwise the server seeds the room (the editor
                // renders nothing until the server document is promoted).
                snapshot:
                    stored != null && stored.roomKey === collaborationDocumentId
                        ? stored.snapshot
                        : undefined,
            },
        });
    }, [collaborationEnabled, collaborationDocumentId, documentConfig]);

    useEffect(() => () => documentHandle.destroy(), [documentHandle]);

    const collaboration = useYjsCollaboration({
        documentId: collaborationEnabled ? collaborationDocumentId : 'local',
        handle: documentHandle,
        transport: {
            url: collaborationSocketUrl,
            connect: collaborationEnabled,
        },
        localAwareness: collaborationEnabled
            ? {
                  userId: `${Platform.OS}-demo-user`,
                  name: collaborationDisplayName,
                  color: collaborationColor,
              }
            : undefined,
    });
    const remotePeers = useMemo(
        () => collaboration.peers.filter((peer) => !peer.isLocal),
        [collaboration.peers]
    );

    const handleCollaborationEnabledChange = (nextValue: boolean) => {
        if (nextValue) {
            setCollaborationEnabled(true);
            return;
        }

        // Export a document-scoped snapshot of the room before leaving it so
        // the next session for the same room can restore offline.
        try {
            const exported = documentHandle.bridge.snapshotExport();
            roomSnapshotRef.current = {
                roomKey: collaborationDocumentId,
                snapshot: {
                    metadata: JSON.parse(exported.metadataJson) as NativeEditorV2RoomSnapshot[
                        'metadata'
                    ],
                    encodedState: exported.encodedState,
                },
            };
        } catch {
            roomSnapshotRef.current = null;
        }

        collaboration.disconnect();
        setCollaborationEnabled(false);
    };

    const handleCollaborationDisplayNameChange = (nextValue: string) => {
        setCollaborationDisplayName(nextValue);
        if (collaborationEnabled) {
            collaboration.updateLocalAwareness({
                user: {
                    userId: `${Platform.OS}-demo-user`,
                    name: nextValue,
                    color: collaborationColor,
                },
            });
        }
    };

    const flushOutputPanelUpdate = React.useCallback(() => {
        outputPanelUpdateTimerRef.current = null;
        const pendingUpdate = pendingOutputPanelUpdateRef.current;
        pendingOutputPanelUpdateRef.current = {};
        if (Object.prototype.hasOwnProperty.call(pendingUpdate, 'html')) {
            setHtml(pendingUpdate.html ?? INITIAL_CONTENT);
        }
        if (Object.prototype.hasOwnProperty.call(pendingUpdate, 'contentJson')) {
            setContentJson(pendingUpdate.contentJson ?? null);
        }
    }, []);

    const scheduleOutputPanelUpdate = React.useCallback(() => {
        if (outputPanelUpdateTimerRef.current != null) return;
        outputPanelUpdateTimerRef.current = setTimeout(
            flushOutputPanelUpdate,
            OUTPUT_PANEL_UPDATE_DEBOUNCE_MS
        );
    }, [flushOutputPanelUpdate]);

    const handleContentChange = React.useCallback(
        (nextHtml: string) => {
            pendingOutputPanelUpdateRef.current.html = nextHtml;
            scheduleOutputPanelUpdate();
        },
        [scheduleOutputPanelUpdate]
    );

    const handleContentChangeJSON = React.useCallback(
        (json: DocumentJSON) => {
            localContentRef.current = json;
            pendingOutputPanelUpdateRef.current.contentJson = json;
            scheduleOutputPanelUpdate();
        },
        [scheduleOutputPanelUpdate]
    );

    const collaborationStatusText = useMemo(() => {
        const peerLabel =
            remotePeers.length === 1 ? '1 remote peer' : `${remotePeers.length} remote peers`;
        return `${collaboration.state.status} · ${peerLabel}`;
    }, [collaboration.state.status, remotePeers.length]);

    if (showPreparedViewerBenchmark) {
        return <PreparedViewerBenchmarkScreen onBack={() => setShowPreparedViewerBenchmark(false)} />;
    }

    return (
        <View style={[styles.safeArea, { backgroundColor: appChrome.screenBackgroundColor }]}>
            <StatusBar style={activeThemePreset.statusBarStyle} />

            <KeyboardAvoidingView
                style={styles.keyboardAvoider}
                enabled={Platform.OS === 'ios'}
                behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
                keyboardVerticalOffset={0}>
                <ScrollView
                    style={[styles.screen, { backgroundColor: appChrome.screenBackgroundColor }]}
                    contentContainerStyle={[
                        styles.content,
                        {
                            paddingTop: 20 + insets.top,
                            paddingBottom: 32 + insets.bottom,
                        },
                    ]}
                    automaticallyAdjustKeyboardInsets={Platform.OS === 'ios'}
                    keyboardDismissMode={Platform.OS === 'ios' ? 'interactive' : 'on-drag'}
                    keyboardShouldPersistTaps='always'>
                    <View style={styles.header}>
                        <Text style={[styles.eyebrow, { color: appChrome.eyebrowColor }]}>
                            Demo
                        </Text>

                        <Text style={[styles.title, { color: appChrome.titleColor }]}>
                            React Native Prose Editor
                        </Text>

                        <Text style={[styles.subtitle, { color: appChrome.subtitleColor }]}>
                            Live playground for manual testing of document sessions, collaboration
                            presence, and theme changes.
                        </Text>
                        <Pressable
                            accessibilityRole='button'
                            onPress={() => setShowPreparedViewerBenchmark(true)}
                            style={styles.benchmarkButton}>
                            <Text style={styles.benchmarkButtonLabel}>Prepared viewer benchmark</Text>
                        </Pressable>
                    </View>

                    <CollapsibleSection
                        title='Theme Preset'
                        appChrome={appChrome}
                        style={[
                            styles.collapsibleCard,
                            { backgroundColor: appChrome.cardBackgroundColor },
                        ]}>
                        <ThemePresetPicker
                            presets={EXAMPLE_THEME_PRESETS}
                            selectedId={selectedThemePresetId}
                            onSelect={setSelectedThemePresetId}
                            appChrome={appChrome}
                        />
                    </CollapsibleSection>

                    <ThemeSettingsCard
                        settingsTab={settingsTab}
                        onSettingsTabChange={setSettingsTab}
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
                        expandedEditorColor={expandedEditorColor}
                        onExpandedEditorColorChange={setExpandedEditorColor}
                        toolbarTheme={toolbarTheme}
                        onToolbarThemeChange={setToolbarTheme}
                        expandedColor={expandedToolbarColor}
                        onExpandedColorChange={setExpandedToolbarColor}
                        sliderTheme={activeThemePreset.slider}
                        appChrome={appChrome}
                        onFocusPress={() => editorRef.current?.focus()}
                        onBlurPress={() => editorRef.current?.blur()}
                        onResetContentPress={() => editorRef.current?.setContent(INITIAL_CONTENT)}
                    />

                    <CollaborationPanel
                        collaborationEnabled={collaborationEnabled}
                        onCollaborationEnabledChange={handleCollaborationEnabledChange}
                        collaborationEndpoint={collaborationEndpoint}
                        onCollaborationEndpointChange={setCollaborationEndpoint}
                        collaborationRoomId={collaborationRoomId}
                        onCollaborationRoomIdChange={setCollaborationRoomId}
                        collaborationDisplayName={collaborationDisplayName}
                        onCollaborationDisplayNameChange={handleCollaborationDisplayNameChange}
                        collaborationStatusText={collaborationStatusText}
                        collaborationLastErrorMessage={collaboration.state.lastError?.message}
                        collaborationIsConnected={collaboration.state.isConnected}
                        remotePeers={remotePeers}
                        onConnect={() => collaboration.connect()}
                        onDisconnect={() => collaboration.disconnect()}
                        appChrome={appChrome}
                    />

                    <View
                        style={[
                            styles.editorCard,
                            { backgroundColor: appChrome.cardSecondaryBackgroundColor },
                        ]}>
                        <Text style={[sharedStyles.sectionLabel, { color: appChrome.sectionLabelColor }]}>
                            Editor
                        </Text>
                        <NativeRichTextEditor
                            ref={editorRef}
                            documentHandle={collaboration.editorBindings.documentHandle}
                            documentRevision={collaboration.editorBindings.documentRevision}
                            remoteSelections={collaboration.editorBindings.remoteSelections}
                            onSelectionChange={collaboration.editorBindings.onSelectionChange}
                            onFocus={collaboration.editorBindings.onFocus}
                            onBlur={collaboration.editorBindings.onBlur}
                            theme={theme}
                            addons={addons}
                            placeholder='Start typing...'
                            onContentChange={handleContentChange}
                            onContentChangeJSON={handleContentChangeJSON}
                            style={styles.editor}
                        />
                    </View>

                    <OutputCard
                        html={html}
                        jsonSnapshot={jsonSnapshot}
                        mentionQuerySummary={mentionQuerySummary}
                        mentionSelectionSummary={mentionSelectionSummary}
                        appChrome={appChrome}
                    />

                    <Text style={[styles.copyright, { color: appChrome.subtitleColor }]}>
                        {'\u00A9'} {new Date().getFullYear()} Apollo Health Group Pty Ltd. All
                        rights reserved.
                    </Text>
                </ScrollView>
            </KeyboardAvoidingView>
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
    dispatched: boolean;
    momentumBegan: boolean;
    consumed: boolean;
};

const preparedViewerCorpus = performanceCorpus as PerformanceCorpus;
const preparedViewerConfiguration = preparedProseBenchmarkConfiguration as {
    configuration: { schema: Parameters<typeof NativeProseViewer>[0]['schema'] };
    imageLoadingPolicy: Parameters<typeof NativeProseViewer>[0]['imageLoadingPolicy'];
};

/** Deterministic FlatList harness; it consumes the checked-in corpus verbatim. */
function PreparedViewerBenchmarkScreen({ onBack }: { onBack: () => void }) {
    // This screen produces benchmark evidence. Resolving the Expo module here
    // leaves normal editor/viewer use independent of the benchmark harness,
    // while a missing required bridge fails visibly when the harness opens.
    const benchmarkBridge = useMemo(
        () => requireNativeModule<PreparedProseBenchmarkBridge>('NativeEditor'),
        []
    );
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

    const cancelActiveTraversal = useCallback(
        (errorMessage?: string) => {
            clearScrollCommandWatchdog();
            activeScrollCommandRef.current = null;
            activeRunIdRef.current = null;
            traversalInFlightRef.current = false;
            endActiveBridgePhase();
            setIsTraversing(false);
            if (errorMessage != null) {
                console.warn(errorMessage);
                setBenchmarkError(errorMessage);
            }
        },
        [clearScrollCommandWatchdog, endActiveBridgePhase]
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

            clearScrollCommandWatchdog();
            const command: ScrollCommandToken = {
                runId,
                windowIndex: commandWindowIndex,
                direction: commandDirection,
                dispatched: true,
                momentumBegan: false,
                consumed: false,
            };
            activeScrollCommandRef.current = command;
            scrollCommandWatchdogRef.current = setTimeout(() => {
                const activeCommand = activeScrollCommandRef.current;
                if (
                    activeCommand?.runId !== command.runId ||
                    activeCommand.windowIndex !== command.windowIndex ||
                    activeCommand.direction !== command.direction ||
                    !activeCommand.dispatched ||
                    activeCommand.momentumBegan ||
                    activeCommand.consumed
                ) {
                    return;
                }
                cancelActiveTraversal(
                    `Prepared viewer benchmark aborted: ${command.direction} scroll for window ${command.windowIndex + 1} did not begin.`
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

    const handleMomentumScrollBegin = useCallback(() => {
        const command = activeScrollCommandRef.current;
        if (
            !traversalInFlightRef.current ||
            command == null ||
            activeRunIdRef.current !== command.runId ||
            !command.dispatched ||
            command.consumed
        ) {
            return;
        }
        command.momentumBegan = true;
        clearScrollCommandWatchdog();
    }, [clearScrollCommandWatchdog]);

    const handleMomentumScrollEnd = useCallback(() => {
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
        command.consumed = true;
        activeScrollCommandRef.current = null;
        clearScrollCommandWatchdog();
        advanceDirectionOrWindow(command);
    }, [advanceDirectionOrWindow, clearScrollCommandWatchdog]);

    useEffect(
        () => () => {
            cancelActiveTraversal();
        },
        [cancelActiveTraversal]
    );

    const handleBack = useCallback(() => {
        cancelActiveTraversal();
        onBack();
    }, [cancelActiveTraversal, onBack]);

    return (
        <View style={styles.benchmarkScreen}>
            <Text style={styles.benchmarkTitle}>Prepared prose benchmark</Text>
            <Text style={styles.benchmarkSubtitle}>
                {entries.length} messages · {window.id} · {phase} {direction} · images{' '}
                {imagesEnabled ? 'enabled' : 'disabled'}
            </Text>
            <View style={styles.benchmarkControls}>
                <Pressable
                    disabled={isTraversing}
                    onPress={() => beginWindowRun(true)}
                    style={styles.benchmarkButton}>
                    <Text style={styles.benchmarkButtonLabel}>Run warm windows</Text>
                </Pressable>
                <Pressable
                    disabled={isTraversing}
                    onPress={() => {
                        // Reset is intentionally not a traversal phase.
                        benchmarkBridge.preparedProseBenchmarkReset();
                    }}
                    style={styles.benchmarkButton}>
                    <Text style={styles.benchmarkButtonLabel}>Reset cache</Text>
                </Pressable>
                <Pressable
                    disabled={isTraversing}
                    onPress={() => beginWindowRun(false)}
                    style={styles.benchmarkButton}>
                    <Text style={styles.benchmarkButtonLabel}>Images disabled windows</Text>
                </Pressable>
                <Pressable
                    disabled={isTraversing}
                    onPress={() => setExportedCounters(benchmarkBridge.preparedProseBenchmarkExport())}
                    style={styles.benchmarkButton}>
                    <Text style={styles.benchmarkButtonLabel}>Export counters</Text>
                </Pressable>
                <Pressable onPress={handleBack} style={styles.benchmarkButton}>
                    <Text style={styles.benchmarkButtonLabel}>Back</Text>
                </Pressable>
            </View>
            {benchmarkError != null ? <Text style={styles.benchmarkCounters}>{benchmarkError}</Text> : null}
            <Text numberOfLines={3} style={styles.benchmarkCounters}>{exportedCounters}</Text>
            <FlatList
                ref={listRef}
                data={entries}
                extraData={`${phase}:${direction}:${imagesEnabled}`}
                keyExtractor={(item) => item.id}
                initialNumToRender={12}
                maxToRenderPerBatch={12}
                windowSize={9}
                onMomentumScrollBegin={handleMomentumScrollBegin}
                onMomentumScrollEnd={handleMomentumScrollEnd}
                renderItem={({ item }) => (
                    <NativeProseViewer
                        contentJSON={item.contentJSON}
                        schema={preparedViewerConfiguration.configuration.schema}
                        imageLoadingPolicy={preparedViewerConfiguration.imageLoadingPolicy}
                        renderImages={imagesEnabled}
                        style={styles.benchmarkViewer}
                    />
                )}
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
        paddingHorizontal: 20,
        gap: 18,
    },
    header: {
        gap: 8,
    },
    eyebrow: {
        fontSize: 12,
        fontWeight: '700',
        letterSpacing: 1.2,
        textTransform: 'uppercase',
        color: '#8d5b3d',
    },
    title: {
        fontSize: 30,
        lineHeight: 36,
        fontWeight: '800',
    },
    subtitle: {
        fontSize: 15,
        lineHeight: 22,
    },
    copyright: {
        fontSize: 12,
        lineHeight: 18,
        textAlign: 'center',
    },
    collapsibleCard: {
        padding: 16,
        borderRadius: 18,
    },
    editorCard: {
        borderRadius: 24,
        padding: 14,
        gap: 10,
    },
    editor: {
        borderRadius: 16,
        minHeight: 200,
        maxHeight: 300,
    },
    benchmarkButton: {
        alignSelf: 'flex-start',
        borderRadius: 10,
        backgroundColor: '#165DFF',
        paddingHorizontal: 12,
        paddingVertical: 9,
    },
    benchmarkButtonLabel: {
        color: '#FFFFFF',
        fontSize: 13,
        fontWeight: '700',
    },
    benchmarkScreen: {
        flex: 1,
        backgroundColor: '#F4F6FA',
        paddingTop: 56,
        paddingHorizontal: 16,
    },
    benchmarkTitle: {
        color: '#172033',
        fontSize: 24,
        fontWeight: '800',
    },
    benchmarkSubtitle: {
        color: '#596579',
        marginTop: 6,
        marginBottom: 12,
    },
    benchmarkControls: {
        flexDirection: 'row',
        flexWrap: 'wrap',
        gap: 8,
        marginBottom: 12,
    },
    benchmarkViewer: {
        backgroundColor: '#FFFFFF',
        borderRadius: 12,
        marginBottom: 8,
        padding: 12,
    },
    benchmarkCounters: {
        color: '#344055',
        fontFamily: Platform.select({ ios: 'Menlo', android: 'monospace' }),
        fontSize: 10,
        marginBottom: 8,
    },
});
