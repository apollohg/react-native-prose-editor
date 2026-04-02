import React from 'react';
import { StyleSheet, Text, View } from 'react-native';
import {
    NativeRichTextEditor,
    type DocumentJSON,
    type EditorAddons,
    type EditorToolbarItem,
    type LinkRequestContext,
    type NativeRichTextEditorHeightBehavior,
    type NativeRichTextEditorRef,
    type NativeRichTextEditorToolbarPlacement,
    type Selection,
} from '@apollohg/react-native-prose-editor';
import type { ExampleThemePreset } from '../themePresets';
import { sharedStyles } from '../sharedStyles';

type EditorDemoCardProps = {
    editorRef: React.RefObject<NativeRichTextEditorRef | null>;
    initialContent: string;
    valueJSON?: DocumentJSON;
    theme: React.ComponentProps<typeof NativeRichTextEditor>['theme'];
    addons?: EditorAddons;
    toolbarItems: readonly EditorToolbarItem[];
    onRequestLink: (context: LinkRequestContext) => void;
    heightBehavior: NativeRichTextEditorHeightBehavior;
    toolbarPlacement: NativeRichTextEditorToolbarPlacement;
    onContentChange: (html: string) => void;
    onContentChangeJSON: (json: DocumentJSON) => void;
    onSelectionChange: (selection: Selection) => void;
    onFocus: () => void;
    onBlur: () => void;
    remoteSelections?: React.ComponentProps<typeof NativeRichTextEditor>['remoteSelections'];
    appChrome: ExampleThemePreset['appChrome'];
};

export function EditorDemoCard({
    editorRef,
    initialContent,
    valueJSON,
    theme,
    addons,
    toolbarItems,
    onRequestLink,
    heightBehavior,
    toolbarPlacement,
    onContentChange,
    onContentChangeJSON,
    onSelectionChange,
    onFocus,
    onBlur,
    remoteSelections,
    appChrome,
}: EditorDemoCardProps) {
    return (
        <View style={[styles.card, { backgroundColor: appChrome.cardSecondaryBackgroundColor }]}>
            <Text style={[sharedStyles.sectionLabel, { color: appChrome.sectionLabelColor }]}>
                Editor
            </Text>

            <NativeRichTextEditor
                ref={editorRef}
                initialContent={initialContent}
                valueJSON={valueJSON}
                theme={theme}
                addons={addons}
                toolbarItems={toolbarItems}
                onRequestLink={onRequestLink}
                autoFocus
                heightBehavior={heightBehavior}
                toolbarPlacement={toolbarPlacement}
                onContentChange={onContentChange}
                onContentChangeJSON={onContentChangeJSON}
                onSelectionChange={onSelectionChange}
                onFocus={onFocus}
                onBlur={onBlur}
                remoteSelections={remoteSelections}
                style={[styles.editor, heightBehavior === 'fixed' && styles.editorFixed]}
            />
        </View>
    );
}

const styles = StyleSheet.create({
    card: {
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
