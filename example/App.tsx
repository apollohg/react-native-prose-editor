import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { StatusBar } from 'expo-status-bar';
import { SafeAreaProvider, useSafeAreaInsets } from 'react-native-safe-area-context';

import {
  DEFAULT_EDITOR_TOOLBAR_ITEMS,
  NativeRichTextEditor,
  type DocumentJSON,
  type EditorAddons,
  type EditorToolbarItem,
  type EditorToolbarTheme,
  type MentionQueryChangeEvent,
  type MentionSelectEvent,
  type NativeRichTextEditorHeightBehavior,
  type NativeRichTextEditorRef,
  type NativeRichTextEditorToolbarPlacement,
} from '@apollohg/react-native-prose-editor';
import {
  buildExampleEditorTheme,
  DEFAULT_EXAMPLE_THEME_PRESET_ID,
  EXAMPLE_THEME_PRESETS,
  getExampleThemePreset,
} from './themePresets';
import { sharedStyles } from './sharedStyles';
import { EXAMPLE_MENTION_SUGGESTIONS, INITIAL_CONTENT, type ToolbarColorKey } from './constants';
import { ThemePresetPicker } from './components/ThemePresetPicker';
import { EditorSettingsPanel } from './components/EditorSettingsPanel';
import { ToolbarSettingsPanel } from './components/ToolbarSettingsPanel';
import { OutputCard } from './components/OutputCard';
import { CollapsibleSection } from './components/CollapsibleSection';

const ANDROID_KEYBOARD_TOOLBAR_OFFSET = 60;

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
  const [heightBehavior, setHeightBehavior] = useState<NativeRichTextEditorHeightBehavior>('autoGrow');
  const [toolbarPlacement, setToolbarPlacement] =
    useState<NativeRichTextEditorToolbarPlacement>('keyboard');

  const [mentionsEnabled, setMentionsEnabled] = useState(false);
  const [mentionQueryEvent, setMentionQueryEvent] = useState<MentionQueryChangeEvent | null>(
    null
  );
  const [mentionSelectEvent, setMentionSelectEvent] = useState<MentionSelectEvent | null>(null);

  const [expandedToolbarColor, setExpandedToolbarColor] = useState<ToolbarColorKey | null>(
    null
  );

  const [toolbarItems, setToolbarItems] = useState<EditorToolbarItem[]>(
    () => [...DEFAULT_EDITOR_TOOLBAR_ITEMS]
  );

  const activeThemePreset = useMemo(
    () => getExampleThemePreset(selectedThemePresetId),
    [selectedThemePresetId]
  );

  const appChrome = activeThemePreset.appChrome;

  const [toolbarTheme, setToolbarTheme] = useState<Required<EditorToolbarTheme>>(
    () => activeThemePreset.toolbar
  );

  useEffect(() => {
    setToolbarTheme(activeThemePreset.toolbar);
    setExpandedToolbarColor(null);
  }, [activeThemePreset]);

  useEffect(() => {
    if (!mentionsEnabled) {
      setMentionQueryEvent(null);
      setMentionSelectEvent(null);
    }
  }, [mentionsEnabled]);

  const theme = useMemo(() => {
    const fontSize = baseFontSize || 17;
    return buildExampleEditorTheme(activeThemePreset, fontSize, toolbarTheme);
  }, [activeThemePreset, baseFontSize, toolbarTheme]);

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

  return (
    <View
      style={[
        styles.safeArea,
        { backgroundColor: appChrome.screenBackgroundColor },
      ]}
    >
      <StatusBar style={activeThemePreset.statusBarStyle} />

      <KeyboardAvoidingView
        style={styles.keyboardAvoider}
        behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
        keyboardVerticalOffset={
          Platform.OS === 'android' && toolbarPlacement === 'keyboard'
            ? ANDROID_KEYBOARD_TOOLBAR_OFFSET
            : 0
        }
      >
        <ScrollView
          style={[
            styles.screen,
            { backgroundColor: appChrome.screenBackgroundColor },
          ]}
          contentContainerStyle={[
            styles.content,
            {
              paddingTop: 20 + insets.top,
              paddingBottom: 32 + insets.bottom,
            },
          ]}
          keyboardShouldPersistTaps="handled"
        >
          <View style={styles.header}>
            <Text style={[styles.eyebrow, { color: appChrome.eyebrowColor }]}>
              Demo
            </Text>

            <Text style={[styles.title, { color: appChrome.titleColor }]}>
              React Native Prose Editor
            </Text>

            <Text style={[styles.subtitle, { color: appChrome.subtitleColor }]}>
              Live playground for manual testing of native behavior, focus, keyboard dismissal, and theme changes.
            </Text>
          </View>

            <CollapsibleSection
              title="Theme Preset"
              appChrome={appChrome}
              style={[
                styles.collapsibleCard,
                { backgroundColor: appChrome.cardBackgroundColor },
              ]}
            >
              <ThemePresetPicker
                presets={EXAMPLE_THEME_PRESETS}
                selectedId={selectedThemePresetId}
                onSelect={setSelectedThemePresetId}
                appChrome={appChrome}
              />
            </CollapsibleSection>

            <CollapsibleSection
              title="Theme Settings"
              appChrome={appChrome}
              style={[
                styles.collapsibleCard,
                { backgroundColor: appChrome.cardBackgroundColor },
              ]}
            >
              <View style={styles.tabRow}>
                <Pressable
                  style={[
                    styles.tabButton,
                    {
                      borderColor: appChrome.tabBorderColor,
                      backgroundColor: appChrome.tabBackgroundColor,
                    },
                    settingsTab === 'editor' && {
                      borderColor: appChrome.tabActiveBorderColor,
                      backgroundColor: appChrome.tabActiveBackgroundColor,
                    },
                  ]}
                  onPress={() => setSettingsTab('editor')}
                >
                  <Text
                    style={[
                      styles.tabButtonText,
                      { color: appChrome.tabTextColor },
                      settingsTab === 'editor' && {
                        color: appChrome.tabActiveTextColor,
                      },
                    ]}
                  >
                    Editor
                  </Text>
                </Pressable>

                <Pressable
                  style={[
                    styles.tabButton,
                    {
                      borderColor: appChrome.tabBorderColor,
                      backgroundColor: appChrome.tabBackgroundColor,
                    },
                    settingsTab === 'toolbar' && {
                      borderColor: appChrome.tabActiveBorderColor,
                      backgroundColor: appChrome.tabActiveBackgroundColor,
                    },
                  ]}
                  onPress={() => setSettingsTab('toolbar')}
                >
                  <Text
                    style={[
                      styles.tabButtonText,
                      { color: appChrome.tabTextColor },
                      settingsTab === 'toolbar' && {
                        color: appChrome.tabActiveTextColor,
                      },
                    ]}
                  >
                    Toolbar
                  </Text>
                </Pressable>
              </View>

              {settingsTab === 'editor' ? (
                <EditorSettingsPanel
                  baseFontSize={baseFontSize}
                  onBaseFontSizeChange={setBaseFontSize}
                  autoGrow={heightBehavior === 'autoGrow'}
                  onAutoGrowChange={(on) => setHeightBehavior(on ? 'autoGrow' : 'fixed')}
                  toolbarPlacement={toolbarPlacement}
                  onToolbarPlacementChange={setToolbarPlacement}
                  mentionsEnabled={mentionsEnabled}
                  onMentionsEnabledChange={setMentionsEnabled}
                  sliderTheme={activeThemePreset.slider}
                  appChrome={appChrome}
                />
              ) : (
                <ToolbarSettingsPanel
                  toolbarItems={toolbarItems}
                  onToolbarItemsChange={setToolbarItems}
                  toolbarTheme={toolbarTheme}
                  onToolbarThemeChange={setToolbarTheme}
                  expandedColor={expandedToolbarColor}
                  onExpandedColorChange={setExpandedToolbarColor}
                  sliderTheme={activeThemePreset.slider}
                  appChrome={appChrome}
                />
              )}

              <View style={styles.buttonRow}>
                <Pressable
                  style={[
                    styles.actionButton,
                    { backgroundColor: appChrome.actionButtonBackgroundColor },
                  ]}
                  onPress={() => editorRef.current?.focus()}
                >
                  <Text
                    style={[
                      styles.actionButtonText,
                      { color: appChrome.actionButtonTextColor },
                    ]}
                  >
                    Focus
                  </Text>
                </Pressable>

                <Pressable
                  style={[
                    styles.actionButton,
                    { backgroundColor: appChrome.actionButtonBackgroundColor },
                  ]}
                  onPress={() => editorRef.current?.blur()}
                >
                  <Text
                    style={[
                      styles.actionButtonText,
                      { color: appChrome.actionButtonTextColor },
                    ]}
                  >
                    Blur
                  </Text>
                </Pressable>

                <Pressable
                  style={[
                    styles.actionButton,
                    { backgroundColor: appChrome.actionButtonBackgroundColor },
                  ]}
                  onPress={() => editorRef.current?.setContent(INITIAL_CONTENT)}
                >
                  <Text
                    style={[
                      styles.actionButtonText,
                      { color: appChrome.actionButtonTextColor },
                    ]}
                  >
                    Reset Content
                  </Text>
                </Pressable>
              </View>
            </CollapsibleSection>

            <View
              style={[
                styles.editorCard,
                { backgroundColor: appChrome.cardSecondaryBackgroundColor },
              ]}
            >
              <Text style={[sharedStyles.sectionLabel, { color: appChrome.sectionLabelColor }]}>
                Editor
              </Text>

              <NativeRichTextEditor
                ref={editorRef}
                initialContent={INITIAL_CONTENT}
                theme={theme}
                addons={addons}
                toolbarItems={toolbarItems}
                autoFocus
                heightBehavior={heightBehavior}
                toolbarPlacement={toolbarPlacement}
                onContentChange={setHtml}
                onContentChangeJSON={setContentJson}
                style={[
                  styles.editor,
                  heightBehavior === 'fixed' && styles.editorFixed,
                ]}
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
            {'\u00A9'} {new Date().getFullYear()} Apollo Health Group Pty Ltd. All rights reserved.
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
  tabRow: {
    flexDirection: 'row',
    gap: 10,
  },
  tabButton: {
    flex: 1,
    paddingVertical: 10,
    paddingHorizontal: 14,
    borderRadius: 12,
    borderWidth: 1,
    alignItems: 'center',
  },
  tabButtonText: {
    fontSize: 13,
    fontWeight: '700',
    textTransform: 'uppercase',
    letterSpacing: 0.8,
  },
  buttonRow: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 10,
  },
  actionButton: {
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderRadius: 999,
  },
  actionButtonText: {
    fontWeight: '700',
  },
  editorCard: {
    borderRadius: 24,
    padding: 14,
    gap: 10,
  },
  editor: {
    borderRadius: 16,
  },
  editorFixed: {
    minHeight: 200,
    maxHeight: 300,
  },
});
