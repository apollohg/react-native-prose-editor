import React, { useEffect, useMemo, useRef, useState } from 'react';
import { KeyboardAvoidingView, Platform, ScrollView, StyleSheet, Text, View } from 'react-native';
import { StatusBar } from 'expo-status-bar';
import { SafeAreaProvider, useSafeAreaInsets } from 'react-native-safe-area-context';

import {
    createNativeEditorDocumentHandle,
    DEFAULT_EDITOR_RESOURCE_LIMITS,
    NativeRichTextEditor,
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
    const createCollaborationWebSocket = React.useCallback(
        () => new WebSocket(collaborationSocketUrl),
        [collaborationSocketUrl]
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
        connect: collaborationEnabled,
        createWebSocket: createCollaborationWebSocket,
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
                            onLocalDocumentCommit={
                                collaboration.editorBindings.onLocalDocumentCommit
                            }
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
});
