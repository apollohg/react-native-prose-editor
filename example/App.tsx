import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
    Alert,
    KeyboardAvoidingView,
    Platform,
    ScrollView,
    StyleSheet,
    Text,
    View,
} from 'react-native';
import { StatusBar } from 'expo-status-bar';
import * as ImagePicker from 'expo-image-picker';
import { manipulateAsync, SaveFormat } from 'expo-image-manipulator';
import { SafeAreaProvider, useSafeAreaInsets } from 'react-native-safe-area-context';

import {
    useYjsCollaboration,
    type DocumentJSON,
    type EditorAddons,
    type EditorToolbarItem,
    type EditorToolbarTheme,
    type ImageRequestContext,
    type LinkRequestContext,
    type MentionQueryChangeEvent,
    type MentionSelectEvent,
    type NativeRichTextEditorHeightBehavior,
    type NativeRichTextEditorRef,
    type NativeRichTextEditorToolbarPlacement,
    type Selection,
} from '@apollohg/react-native-prose-editor';

import {
    buildExampleEditorTheme,
    DEFAULT_EXAMPLE_THEME_PRESET_ID,
    EXAMPLE_THEME_PRESETS,
    type ExampleEditorThemeOverrides,
    getExampleThemePreset,
} from './themePresets';

import {
    EXAMPLE_DEFAULT_TOOLBAR_ITEMS,
    EXAMPLE_MENTION_SUGGESTIONS,
    INITIAL_CONTENT,
    type ToolbarColorKey,
} from './constants';

import { ThemePresetPicker } from './components/ThemePresetPicker';
import { OutputCard } from './components/OutputCard';
import { CollapsibleSection } from './components/CollapsibleSection';
import { ThemeSettingsCard } from './components/ThemeSettingsCard';
import { CollaborationPanel } from './components/CollaborationPanel';
import { EditorDemoCard } from './components/EditorDemoCard';
import { LinkEditorModal } from './components/LinkEditorModal';

const ANDROID_KEYBOARD_TOOLBAR_OFFSET = 60;
const DEFAULT_COLLABORATION_ENDPOINT = 'ws://localhost:1234/collaboration';
const DEFAULT_COLLABORATION_ROOM_ID = 'example-room';
const MAX_INSERT_IMAGE_DIMENSION = 1024;
const MAX_INSERT_IMAGE_BASE64_LENGTH = 350_000;
const INSERT_IMAGE_COMPRESSION_STEPS = [0.72, 0.58, 0.45] as const;

function buildCollaborationSocketUrl(endpoint: string, documentId: string): string {
    const trimmedEndpoint = endpoint.trim();
    if (!trimmedEndpoint) {
        return trimmedEndpoint;
    }
    const separator = trimmedEndpoint.includes('?') ? '&' : '?';
    return `${trimmedEndpoint}${separator}documentId=${encodeURIComponent(documentId)}`;
}

async function normalizePickedImageForInsertion(uri: string, width?: number, height?: number) {
    const resizeAction =
        width != null && height != null
            ? width >= height
                ? width > MAX_INSERT_IMAGE_DIMENSION
                    ? [{ resize: { width: MAX_INSERT_IMAGE_DIMENSION } }]
                    : []
                : height > MAX_INSERT_IMAGE_DIMENSION
                    ? [{ resize: { height: MAX_INSERT_IMAGE_DIMENSION } }]
                    : []
            : [];

    let workingUri = uri;
    let lastResult:
        | Awaited<ReturnType<typeof manipulateAsync>>
        | null = null;

    for (const compress of INSERT_IMAGE_COMPRESSION_STEPS) {
        const result = await manipulateAsync(workingUri, lastResult == null ? resizeAction : [], {
            compress,
            format: SaveFormat.JPEG,
            base64: true,
        });
        lastResult = result;

        if (result.base64 && result.base64.length <= MAX_INSERT_IMAGE_BASE64_LENGTH) {
            return result;
        }

        workingUri = result.uri;
    }

    return lastResult;
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
    const [heightBehavior, setHeightBehavior] =
        useState<NativeRichTextEditorHeightBehavior>('autoGrow');
    const [toolbarPlacement, setToolbarPlacement] =
        useState<NativeRichTextEditorToolbarPlacement>('keyboard');
    const shouldUseKeyboardAvoidingView =
        Platform.OS === 'android' || toolbarPlacement !== 'keyboard';

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
    const [collaborationSeedDocument, setCollaborationSeedDocument] = useState<
        DocumentJSON | undefined
    >(undefined);
    const [collaborationRevision, setCollaborationRevision] = useState(0);

    const [toolbarItems, setToolbarItems] = useState<EditorToolbarItem[]>(() => [
        ...EXAMPLE_DEFAULT_TOOLBAR_ITEMS,
    ]);
    const [pendingLinkRequest, setPendingLinkRequest] = useState<LinkRequestContext | null>(null);
    const [linkDraft, setLinkDraft] = useState('');
    const [isPickingImage, setIsPickingImage] = useState(false);

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

    const collaborationDocumentId = useMemo(
        () =>
            `${collaborationRoomId.trim() || DEFAULT_COLLABORATION_ROOM_ID}|${collaborationEndpoint.trim()}|${collaborationRevision}`,
        [collaborationEndpoint, collaborationRevision, collaborationRoomId]
    );
    const collaborationSocketUrl = useMemo(
        () =>
            buildCollaborationSocketUrl(
                collaborationEndpoint,
                collaborationRoomId.trim() || DEFAULT_COLLABORATION_ROOM_ID
            ),
        [collaborationEndpoint, collaborationRoomId]
    );
    const createCollaborationWebSocket = React.useCallback(
        () => new WebSocket(collaborationSocketUrl),
        [collaborationSocketUrl]
    );
    const collaboration = useYjsCollaboration({
        documentId: collaborationDocumentId,
        connect: false,
        createWebSocket: createCollaborationWebSocket,
        initialDocumentJson: collaborationSeedDocument ?? contentJson ?? undefined,
        localAwareness: {
            userId: `${Platform.OS}-demo-user`,
            name: collaborationDisplayName,
            color: collaborationColor,
        },
    });
    const remotePeers = useMemo(
        () => collaboration.peers.filter((peer) => !peer.isLocal),
        [collaboration.peers]
    );

    const handleCollaborationEnabledChange = (nextValue: boolean) => {
        if (nextValue) {
            setCollaborationSeedDocument(
                editorRef.current?.getContentJson() ?? contentJson ?? undefined
            );
            setCollaborationRevision((value) => value + 1);
            setCollaborationEnabled(true);
            return;
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

    const handleContentChangeJSON = (json: DocumentJSON) => {
        setContentJson(json);
        if (collaborationEnabled) {
            collaboration.editorBindings.onContentChangeJSON(json);
        }
    };

    const handleSelectionChange = (selection: Selection) => {
        if (collaborationEnabled) {
            collaboration.editorBindings.onSelectionChange(selection);
        }
    };

    const handleEditorFocus = () => {
        if (collaborationEnabled) {
            collaboration.editorBindings.onFocus();
        }
    };

    const handleEditorBlur = () => {
        if (collaborationEnabled) {
            collaboration.editorBindings.onBlur();
        }
    };

    const collaborationStatusText = useMemo(() => {
        const peerLabel =
            remotePeers.length === 1 ? '1 remote peer' : `${remotePeers.length} remote peers`;
        return `${collaboration.state.status} · ${peerLabel}`;
    }, [collaboration.state.status, remotePeers.length]);

    const openLinkRequest = (context: LinkRequestContext) => {
        setPendingLinkRequest(context);
        setLinkDraft(context.href ?? 'https://');
    };

    const closeLinkRequest = () => {
        setPendingLinkRequest(null);
        setLinkDraft('');
    };

    const refocusEditorSoon = React.useCallback(() => {
        requestAnimationFrame(() => {
            editorRef.current?.focus();
        });
    }, []);

    const openImageRequest = React.useCallback(
        async (context: ImageRequestContext) => {
            if (isPickingImage) {
                return;
            }

            if (!context.allowBase64) {
                Alert.alert(
                    'Base64 images disabled',
                    'This editor instance does not currently allow base64 image insertion.'
                );
                refocusEditorSoon();
                return;
            }

            setIsPickingImage(true);
            try {
                const permission = await ImagePicker.requestMediaLibraryPermissionsAsync();
                if (!permission.granted) {
                    Alert.alert(
                        'Permission required',
                        'Photo library access is required to choose an image from your device.'
                    );
                    return;
                }

                const result = await ImagePicker.launchImageLibraryAsync({
                    mediaTypes: ['images'],
                    allowsEditing: false,
                    base64: true,
                    quality: 0.8,
                });

                if (result.canceled) {
                    refocusEditorSoon();
                    return;
                }

                const asset = result.assets?.[0];
                if (!asset?.uri) {
                    Alert.alert(
                        'Image unavailable',
                        'The selected image could not be loaded for insertion.'
                    );
                    refocusEditorSoon();
                    return;
                }

                const normalizedImage = await normalizePickedImageForInsertion(
                    asset.uri,
                    asset.width,
                    asset.height
                );

                if (!normalizedImage?.base64) {
                    Alert.alert(
                        'Image unavailable',
                        'The selected image could not be converted into base64 for insertion.'
                    );
                    refocusEditorSoon();
                    return;
                }

                context.insertImage(`data:image/jpeg;base64,${normalizedImage.base64}`, {
                    alt: asset.fileName ?? null,
                    title: asset.fileName ?? null,
                    width: normalizedImage.width,
                    height: normalizedImage.height,
                });
                refocusEditorSoon();
            } catch {
                Alert.alert(
                    'Image unavailable',
                    'The selected image could not be prepared for insertion.'
                );
                refocusEditorSoon();
            } finally {
                setIsPickingImage(false);
            }
        },
        [isPickingImage, refocusEditorSoon]
    );

    const applyLinkRequest = () => {
        if (!pendingLinkRequest) {
            return;
        }

        const nextHref = linkDraft.trim();
        if (nextHref.length === 0) {
            pendingLinkRequest.unsetLink();
        } else {
            pendingLinkRequest.setLink(nextHref);
        }

        closeLinkRequest();
        refocusEditorSoon();
    };

    const removeLink = () => {
        if (!pendingLinkRequest) {
            return;
        }

        pendingLinkRequest.unsetLink();
        closeLinkRequest();
        refocusEditorSoon();
    };

    return (
        <View style={[styles.safeArea, { backgroundColor: appChrome.screenBackgroundColor }]}>
            <StatusBar style={activeThemePreset.statusBarStyle} />

            <KeyboardAvoidingView
                style={styles.keyboardAvoider}
                enabled={shouldUseKeyboardAvoidingView}
                behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
                keyboardVerticalOffset={
                    Platform.OS === 'android' && toolbarPlacement === 'keyboard'
                        ? ANDROID_KEYBOARD_TOOLBAR_OFFSET
                        : 0
                }>
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
                            Live playground for manual testing of native behavior, focus, keyboard
                            dismissal, and theme changes.
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
                        heightBehavior={heightBehavior}
                        onHeightBehaviorChange={setHeightBehavior}
                        toolbarPlacement={toolbarPlacement}
                        onToolbarPlacementChange={setToolbarPlacement}
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
                        toolbarItems={toolbarItems}
                        onToolbarItemsChange={setToolbarItems}
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

                    <EditorDemoCard
                        editorRef={editorRef}
                        initialContent={INITIAL_CONTENT}
                        valueJSON={
                            collaborationEnabled
                                ? collaboration.editorBindings.valueJSON
                                : undefined
                        }
                        theme={theme}
                        addons={addons}
                        toolbarItems={toolbarItems}
                        onRequestLink={openLinkRequest}
                        onRequestImage={openImageRequest}
                        heightBehavior={heightBehavior}
                        toolbarPlacement={toolbarPlacement}
                        onContentChange={setHtml}
                        onContentChangeJSON={handleContentChangeJSON}
                        onSelectionChange={handleSelectionChange}
                        onFocus={handleEditorFocus}
                        onBlur={handleEditorBlur}
                        remoteSelections={
                            collaborationEnabled
                                ? collaboration.editorBindings.remoteSelections
                                : undefined
                        }
                        appChrome={appChrome}
                    />

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

            <LinkEditorModal
                visible={pendingLinkRequest != null}
                isActive={pendingLinkRequest?.isActive ?? false}
                linkDraft={linkDraft}
                onLinkDraftChange={setLinkDraft}
                onClose={closeLinkRequest}
                onRemove={removeLink}
                onApply={applyLinkRequest}
                appChrome={appChrome}
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
});
