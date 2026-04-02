import React from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import type {
    EditorToolbarItem,
    EditorToolbarTheme,
    NativeRichTextEditorHeightBehavior,
    NativeRichTextEditorToolbarPlacement,
} from '@apollohg/react-native-prose-editor';
import type { ToolbarColorKey } from '../constants';
import type { ExampleThemePreset } from '../themePresets';
import { CollapsibleSection } from './CollapsibleSection';
import { EditorSettingsPanel } from './EditorSettingsPanel';
import { ToolbarSettingsPanel } from './ToolbarSettingsPanel';

type ThemeSettingsCardProps = {
    settingsTab: 'editor' | 'toolbar';
    onSettingsTabChange: (tab: 'editor' | 'toolbar') => void;
    baseFontSize: number;
    onBaseFontSizeChange: (size: number) => void;
    heightBehavior: NativeRichTextEditorHeightBehavior;
    onHeightBehaviorChange: (behavior: NativeRichTextEditorHeightBehavior) => void;
    toolbarPlacement: NativeRichTextEditorToolbarPlacement;
    onToolbarPlacementChange: (value: NativeRichTextEditorToolbarPlacement) => void;
    mentionsEnabled: boolean;
    onMentionsEnabledChange: (value: boolean) => void;
    blockquoteBorderColor: string;
    onBlockquoteBorderColorChange: (value: string) => void;
    expandedEditorColor: 'blockquoteBorderColor' | null;
    onExpandedEditorColorChange: (key: 'blockquoteBorderColor' | null) => void;
    toolbarItems: readonly EditorToolbarItem[];
    onToolbarItemsChange: (items: EditorToolbarItem[]) => void;
    toolbarTheme: Required<EditorToolbarTheme>;
    onToolbarThemeChange: (
        updater: (current: Required<EditorToolbarTheme>) => Required<EditorToolbarTheme>
    ) => void;
    expandedColor: ToolbarColorKey | null;
    onExpandedColorChange: (key: ToolbarColorKey | null) => void;
    sliderTheme: ExampleThemePreset['slider'];
    appChrome: ExampleThemePreset['appChrome'];
    onFocusPress: () => void;
    onBlurPress: () => void;
    onResetContentPress: () => void;
};

export function ThemeSettingsCard({
    settingsTab,
    onSettingsTabChange,
    baseFontSize,
    onBaseFontSizeChange,
    heightBehavior,
    onHeightBehaviorChange,
    toolbarPlacement,
    onToolbarPlacementChange,
    mentionsEnabled,
    onMentionsEnabledChange,
    blockquoteBorderColor,
    onBlockquoteBorderColorChange,
    expandedEditorColor,
    onExpandedEditorColorChange,
    toolbarItems,
    onToolbarItemsChange,
    toolbarTheme,
    onToolbarThemeChange,
    expandedColor,
    onExpandedColorChange,
    sliderTheme,
    appChrome,
    onFocusPress,
    onBlurPress,
    onResetContentPress,
}: ThemeSettingsCardProps) {
    return (
        <CollapsibleSection
            title='Theme Settings'
            appChrome={appChrome}
            style={[styles.card, { backgroundColor: appChrome.cardBackgroundColor }]}>
            <View style={styles.tabRow}>
                {(['editor', 'toolbar'] as const).map((tab) => {
                    const selected = settingsTab === tab;
                    return (
                        <Pressable
                            key={tab}
                            style={[
                                styles.tabButton,
                                {
                                    borderColor: appChrome.tabBorderColor,
                                    backgroundColor: appChrome.tabBackgroundColor,
                                },
                                selected && {
                                    borderColor: appChrome.tabActiveBorderColor,
                                    backgroundColor: appChrome.tabActiveBackgroundColor,
                                },
                            ]}
                            onPress={() => onSettingsTabChange(tab)}>
                            <Text
                                style={[
                                    styles.tabButtonText,
                                    { color: appChrome.tabTextColor },
                                    selected && { color: appChrome.tabActiveTextColor },
                                ]}>
                                {tab === 'editor' ? 'Editor' : 'Toolbar'}
                            </Text>
                        </Pressable>
                    );
                })}
            </View>

            {settingsTab === 'editor' ? (
                <EditorSettingsPanel
                    baseFontSize={baseFontSize}
                    onBaseFontSizeChange={onBaseFontSizeChange}
                    autoGrow={heightBehavior === 'autoGrow'}
                    onAutoGrowChange={(on) => onHeightBehaviorChange(on ? 'autoGrow' : 'fixed')}
                    toolbarPlacement={toolbarPlacement}
                    onToolbarPlacementChange={onToolbarPlacementChange}
                    mentionsEnabled={mentionsEnabled}
                    onMentionsEnabledChange={onMentionsEnabledChange}
                    blockquoteBorderColor={blockquoteBorderColor}
                    onBlockquoteBorderColorChange={onBlockquoteBorderColorChange}
                    expandedColor={expandedEditorColor}
                    onExpandedColorChange={onExpandedEditorColorChange}
                    sliderTheme={sliderTheme}
                    appChrome={appChrome}
                />
            ) : (
                <ToolbarSettingsPanel
                    toolbarItems={toolbarItems}
                    onToolbarItemsChange={onToolbarItemsChange}
                    toolbarTheme={toolbarTheme}
                    onToolbarThemeChange={onToolbarThemeChange}
                    expandedColor={expandedColor}
                    onExpandedColorChange={onExpandedColorChange}
                    sliderTheme={sliderTheme}
                    appChrome={appChrome}
                />
            )}

            <View style={styles.buttonRow}>
                <Pressable
                    style={[
                        styles.actionButton,
                        { backgroundColor: appChrome.actionButtonBackgroundColor },
                    ]}
                    onPress={onFocusPress}>
                    <Text
                        style={[
                            styles.actionButtonText,
                            { color: appChrome.actionButtonTextColor },
                        ]}>
                        Focus
                    </Text>
                </Pressable>

                <Pressable
                    style={[
                        styles.actionButton,
                        { backgroundColor: appChrome.actionButtonBackgroundColor },
                    ]}
                    onPress={onBlurPress}>
                    <Text
                        style={[
                            styles.actionButtonText,
                            { color: appChrome.actionButtonTextColor },
                        ]}>
                        Blur
                    </Text>
                </Pressable>

                <Pressable
                    style={[
                        styles.actionButton,
                        { backgroundColor: appChrome.actionButtonBackgroundColor },
                    ]}
                    onPress={onResetContentPress}>
                    <Text
                        style={[
                            styles.actionButtonText,
                            { color: appChrome.actionButtonTextColor },
                        ]}>
                        Reset Content
                    </Text>
                </Pressable>
            </View>
        </CollapsibleSection>
    );
}

const styles = StyleSheet.create({
    card: {
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
});
